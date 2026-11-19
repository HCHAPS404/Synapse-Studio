//! Lexer and recursive-descent parser for the Structured Text subset.
//!
//! Keywords and identifiers are case-insensitive, as in IEC 61131-3. The lexer also
//! accepts the Siemens-style `#local` and `"global"` identifiers, so SCL parses with the
//! same code.

use crate::ast::{
    Arg, CaseLabel, Expr, Init, IntTy, Pou, PouKind, Section, Stmt, Ty, Unit, VarDecl,
};
use std::fmt;

/// A syntax error with its byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset.
    pub pos: usize,
    /// What was expected.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.pos, self.message)
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Kw(String),
    Int(i64),
    Real(f64),
    Time(i64),
    Str(String),
    Addr(String),
    Sym(&'static str),
    Eof,
}

const KEYWORDS: &[&str] = &[
    "PROGRAM",
    "END_PROGRAM",
    "FUNCTION_BLOCK",
    "END_FUNCTION_BLOCK",
    "FUNCTION",
    "END_FUNCTION",
    "ORGANIZATION_BLOCK",
    "END_ORGANIZATION_BLOCK",
    "VAR",
    "VAR_INPUT",
    "VAR_OUTPUT",
    "VAR_IN_OUT",
    "VAR_TEMP",
    "VAR_GLOBAL",
    "VAR_EXTERNAL",
    "END_VAR",
    "RETAIN",
    "CONSTANT",
    "NON_RETAIN",
    "AT",
    "IF",
    "THEN",
    "ELSIF",
    "ELSE",
    "END_IF",
    "CASE",
    "OF",
    "END_CASE",
    "FOR",
    "TO",
    "BY",
    "DO",
    "END_FOR",
    "WHILE",
    "END_WHILE",
    "REPEAT",
    "UNTIL",
    "END_REPEAT",
    "EXIT",
    "RETURN",
    "AND",
    "OR",
    "XOR",
    "NOT",
    "MOD",
    "TRUE",
    "FALSE",
    "ARRAY",
    "CONFIGURATION",
    "END_CONFIGURATION",
    "BEGIN",
    "VERSION",
    "TYPE",
    "END_TYPE",
    "STRUCT",
    "END_STRUCT",
];

const SYMS: &[&str] = &[
    ":=", "=>", "<=", ">=", "<>", "**", "..", "=", "<", ">", "+", "-", "*", "/", "(", ")", "[",
    "]", ",", ";", ":", ".", "&",
];

fn time_literal(text: &str) -> Option<i64> {
    let t = text.replace('_', "").to_ascii_lowercase();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r.to_string()),
        None => (false, t),
    };
    let b = t.as_bytes();
    let mut i = 0;
    let mut total = 0f64;
    let mut any = false;
    while i < b.len() {
        let start = i;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
            i += 1;
        }
        let num: f64 = t[start..i].parse().ok()?;
        let ustart = i;
        while i < b.len() && b[i].is_ascii_alphabetic() {
            i += 1;
        }
        let scale = match &t[ustart..i] {
            "d" => 86_400e9,
            "h" => 3_600e9,
            "m" => 60e9,
            "s" => 1e9,
            "ms" => 1e6,
            "us" => 1e3,
            "ns" => 1.0,
            _ => return None,
        };
        total += num * scale;
        any = true;
    }
    if !any {
        return None;
    }
    let v = total.round() as i64;
    Some(if neg { -v } else { v })
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, ParseError> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let err = |pos: usize, m: &str| ParseError {
        pos,
        message: m.to_string(),
    };
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if src[i..].starts_with("(*") {
            let end = src[i + 2..]
                .find("*)")
                .ok_or_else(|| err(i, "unterminated comment"))?;
            i += end + 4;
            continue;
        }
        if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'{' {
            let end = src[i..]
                .find('}')
                .ok_or_else(|| err(i, "unterminated pragma"))?;
            i += end + 1;
            continue;
        }
        let start = i;
        if c == b'\'' {
            let mut s = String::new();
            i += 1;
            loop {
                if i >= b.len() {
                    return Err(err(start, "unterminated string"));
                }
                if b[i] == b'\'' {
                    if i + 1 < b.len() && b[i + 1] == b'\'' {
                        s.push('\'');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                if b[i] == b'$' && i + 1 < b.len() {
                    s.push(match b[i + 1] {
                        b'N' | b'n' | b'L' | b'l' => '\n',
                        b'T' | b't' => '\t',
                        other => other as char,
                    });
                    i += 2;
                    continue;
                }
                let ch = src[i..].chars().next().unwrap_or(' ');
                s.push(ch);
                i += ch.len_utf8();
            }
            out.push((Tok::Str(s), start));
            continue;
        }
        if c == b'"' {
            let end = src[i + 1..]
                .find('"')
                .ok_or_else(|| err(i, "unterminated name"))?;
            out.push((Tok::Ident(src[i + 1..i + 1 + end].to_string()), start));
            i += end + 2;
            continue;
        }
        if c == b'%' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') {
                i += 1;
            }
            out.push((Tok::Addr(src[start..i].to_ascii_uppercase()), start));
            continue;
        }
        if c == b'#' && i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') {
            i += 1;
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push((Tok::Ident(src[s..i].to_string()), start));
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &src[start..i];
            let up = word.to_ascii_uppercase();
            if i < b.len() && b[i] == b'#' {
                // Typed literal: T#1s, TIME#..., BOOL#TRUE, INT#5, REAL#1.5.
                let vs = i + 1;
                let mut j = vs;
                while j < b.len()
                    && (b[j].is_ascii_alphanumeric()
                        || b[j] == b'_'
                        || b[j] == b'.'
                        || b[j] == b'-'
                        || b[j] == b'#')
                {
                    j += 1;
                }
                let val = &src[vs..j];
                let tok = match up.as_str() {
                    "T" | "TIME" | "LT" | "LTIME" => Tok::Time(
                        time_literal(val).ok_or_else(|| err(start, "invalid time literal"))?,
                    ),
                    "BOOL" => Tok::Kw(match val.to_ascii_uppercase().as_str() {
                        "TRUE" | "1" => "TRUE".to_string(),
                        _ => "FALSE".to_string(),
                    }),
                    _ => match number(val) {
                        Some(t) => t,
                        None => return Err(err(start, "invalid typed literal")),
                    },
                };
                out.push((tok, start));
                i = j;
                continue;
            }
            if KEYWORDS.contains(&up.as_str()) {
                out.push((Tok::Kw(up), start));
            } else {
                out.push((Tok::Ident(word.to_string()), start));
            }
            continue;
        }
        if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'#') {
                i += 1;
            }
            // A fraction, but not the start of a range `1..5`.
            if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                if i < b.len() && (b[i] == b'+' || b[i] == b'-') && matches!(b[i - 1], b'e' | b'E')
                {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let t = number(&src[start..i]).ok_or_else(|| err(start, "invalid number"))?;
            out.push((t, start));
            continue;
        }
        match SYMS.iter().find(|s| src[i..].starts_with(**s)) {
            Some(s) => {
                out.push((Tok::Sym(s), start));
                i += s.len();
            }
            None => return Err(err(i, &format!("unexpected character `{}`", c as char))),
        }
    }
    out.push((Tok::Eof, src.len()));
    Ok(out)
}

fn number(text: &str) -> Option<Tok> {
    let t = text.replace('_', "");
    if let Some((base, digits)) = t.split_once('#') {
        let radix: u32 = base.parse().ok()?;
        return i64::from_str_radix(digits, radix).ok().map(Tok::Int);
    }
    if t.contains('.') || t.contains('e') || t.contains('E') {
        return t.parse().ok().map(Tok::Real);
    }
    t.parse().ok().map(Tok::Int)
}

/// Parses a source file.
///
/// # Errors
/// The first syntax error.
///
/// ```
/// let u = neuron_st::parse("PROGRAM p\nVAR x : BOOL; END_VAR\nx := NOT x;\nEND_PROGRAM\n").unwrap();
/// assert_eq!(u.pous[0].name, "p");
/// ```
pub fn parse(src: &str) -> Result<Unit, ParseError> {
    let toks = lex(src)?;
    let mut p = P { t: toks, i: 0 };
    p.unit()
}

struct P {
    t: Vec<(Tok, usize)>,
    i: usize,
}

type R<T> = Result<T, ParseError>;

impl P {
    fn peek(&self) -> &Tok {
        &self.t[self.i.min(self.t.len() - 1)].0
    }
    fn pos(&self) -> usize {
        self.t[self.i.min(self.t.len() - 1)].1
    }
    fn bump(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.i < self.t.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn err<T>(&self, m: &str) -> R<T> {
        Err(ParseError {
            pos: self.pos(),
            message: format!("{m}, found {}", describe(self.peek())),
        })
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Tok::Kw(x) if x == k)
    }
    fn is_sym(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Sym(x) if *x == s)
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        if self.is_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn eat_sym(&mut self, s: &str) -> bool {
        if self.is_sym(s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn kw(&mut self, k: &str) -> R<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            self.err(&format!("expected `{k}`"))
        }
    }
    fn sym(&mut self, s: &str) -> R<()> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            self.err(&format!("expected `{s}`"))
        }
    }
    fn ident(&mut self) -> R<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            _ => self.err("expected a name"),
        }
    }

    fn unit(&mut self) -> R<Unit> {
        let mut u = Unit::default();
        loop {
            match self.peek().clone() {
                Tok::Eof => return Ok(u),
                Tok::Kw(k) => match k.as_str() {
                    "PROGRAM" | "ORGANIZATION_BLOCK" => {
                        let end = if k == "PROGRAM" {
                            "END_PROGRAM"
                        } else {
                            "END_ORGANIZATION_BLOCK"
                        };
                        let pou = self.pou(PouKind::Program, end)?;
                        u.pous.push(pou);
                    }
                    "FUNCTION_BLOCK" => {
                        let pou = self.pou(PouKind::FunctionBlock, "END_FUNCTION_BLOCK")?;
                        u.pous.push(pou);
                    }
                    "FUNCTION" => {
                        let pou = self.pou(PouKind::Function, "END_FUNCTION")?;
                        u.pous.push(pou);
                    }
                    "CONFIGURATION" => self.configuration(&mut u)?,
                    "TYPE" => {
                        while !self.is_kw("END_TYPE") && !matches!(self.peek(), Tok::Eof) {
                            self.bump();
                        }
                        self.kw("END_TYPE")?;
                    }
                    "VAR_GLOBAL" => {
                        // A global variable list file: kept as a program without a body.
                        let pos = self.pos();
                        let vars = self.var_sections()?;
                        u.pous.push(Pou {
                            kind: PouKind::Program,
                            name: "GlobalVars".to_string(),
                            ret: None,
                            vars,
                            body: Vec::new(),
                            pos,
                        });
                    }
                    _ => return self.err("expected PROGRAM, FUNCTION_BLOCK or FUNCTION"),
                },
                _ => return self.err("expected PROGRAM, FUNCTION_BLOCK or FUNCTION"),
            }
        }
    }

    fn configuration(&mut self, u: &mut Unit) -> R<()> {
        self.kw("CONFIGURATION")?;
        let mut tasks: Vec<(String, i64)> = Vec::new();
        let mut last_interval = 0i64;
        let mut by_task: Vec<(String, i64)> = Vec::new();
        while !self.is_kw("END_CONFIGURATION") {
            match self.bump() {
                Tok::Eof => return self.err("expected END_CONFIGURATION"),
                Tok::Ident(name) if name.eq_ignore_ascii_case("TASK") => {
                    let task = self.ident()?;
                    while !self.is_sym(";") && !matches!(self.peek(), Tok::Eof) {
                        if let Tok::Time(ns) = self.bump() {
                            last_interval = ns;
                        }
                    }
                    by_task.push((task.to_ascii_lowercase(), last_interval));
                }
                Tok::Kw(k) if k == "PROGRAM" => {
                    let _inst = self.ident()?;
                    let mut task = String::new();
                    if self.eat_kw("WITH")
                        || matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("WITH"))
                    {
                        if matches!(self.peek(), Tok::Ident(w) if w.eq_ignore_ascii_case("WITH")) {
                            self.bump();
                        }
                        task = self.ident()?.to_ascii_lowercase();
                    }
                    self.sym(":")?;
                    let ty = self.ident()?;
                    let interval = by_task
                        .iter()
                        .find(|(t, _)| *t == task)
                        .map_or(last_interval, |(_, v)| *v);
                    tasks.push((ty, interval));
                }
                _ => {}
            }
        }
        self.kw("END_CONFIGURATION")?;
        u.tasks.extend(tasks);
        Ok(())
    }

    fn pou(&mut self, kind: PouKind, end: &str) -> R<Pou> {
        let pos = self.pos();
        self.bump();
        let name = self.ident()?;
        let ret = if kind == PouKind::Function && self.eat_sym(":") {
            Some(self.ty()?)
        } else {
            None
        };
        // SCL: `VERSION : 0.1`.
        if self.eat_kw("VERSION") {
            self.sym(":")?;
            self.bump();
        }
        let vars = self.var_sections()?;
        self.eat_kw("BEGIN");
        let body = self.stmts(&[end])?;
        self.kw(end)?;
        Ok(Pou {
            kind,
            name,
            ret,
            vars,
            body,
            pos,
        })
    }

    fn var_sections(&mut self) -> R<Vec<VarDecl>> {
        let mut vars = Vec::new();
        loop {
            let section = match self.peek() {
                Tok::Kw(k) => match k.as_str() {
                    "VAR" => Section::Var,
                    "VAR_INPUT" => Section::Input,
                    "VAR_OUTPUT" => Section::Output,
                    "VAR_IN_OUT" => Section::InOut,
                    "VAR_TEMP" => Section::Temp,
                    "VAR_GLOBAL" => Section::Global,
                    "VAR_EXTERNAL" => Section::External,
                    _ => return Ok(vars),
                },
                _ => return Ok(vars),
            };
            self.bump();
            let mut retain = false;
            loop {
                if self.eat_kw("RETAIN") {
                    retain = true;
                } else if self.eat_kw("CONSTANT") || self.eat_kw("NON_RETAIN") {
                } else {
                    break;
                }
            }
            while !self.eat_kw("END_VAR") {
                let pos = self.pos();
                let mut names = vec![self.ident()?];
                while self.eat_sym(",") {
                    names.push(self.ident()?);
                }
                let at = if self.eat_kw("AT") {
                    match self.bump() {
                        Tok::Addr(a) => Some(a),
                        _ => return self.err("expected a direct address after AT"),
                    }
                } else {
                    None
                };
                self.sym(":")?;
                let ty = self.ty()?;
                let init = if self.eat_sym(":=") {
                    if self.is_sym("(")
                        && matches!(self.t.get(self.i + 1), Some((Tok::Ident(_), _)))
                        && matches!(self.t.get(self.i + 2), Some((Tok::Sym(":="), _)))
                    {
                        self.bump();
                        let mut fields = Vec::new();
                        loop {
                            let f = self.ident()?;
                            self.sym(":=")?;
                            fields.push((f, self.expr()?));
                            if !self.eat_sym(",") {
                                break;
                            }
                        }
                        self.sym(")")?;
                        Some(Init::Struct(fields))
                    } else {
                        Some(Init::Expr(self.expr()?))
                    }
                } else {
                    None
                };
                self.sym(";")?;
                for n in names {
                    vars.push(VarDecl {
                        name: n,
                        section,
                        retain,
                        at: at.clone(),
                        ty: ty.clone(),
                        init: init.clone(),
                        pos,
                    });
                }
            }
        }
    }

    fn ty(&mut self) -> R<Ty> {
        if self.eat_kw("ARRAY") {
            self.sym("[")?;
            let lo = self.int_const()?;
            self.sym("..")?;
            let hi = self.int_const()?;
            self.sym("]")?;
            self.kw("OF")?;
            let elem = self.ty()?;
            return Ok(Ty::Array(lo, hi, Box::new(elem)));
        }
        let name = self.ident()?;
        let up = name.to_ascii_uppercase();
        let int = |bits: u8, signed: bool| Ty::Int(IntTy { bits, signed });
        let t = match up.as_str() {
            "BOOL" => Ty::Bool,
            "SINT" => int(8, true),
            "INT" => int(16, true),
            "DINT" => int(32, true),
            "LINT" => int(64, true),
            "USINT" | "BYTE" => int(8, false),
            "UINT" | "WORD" => int(16, false),
            "UDINT" | "DWORD" => int(32, false),
            "ULINT" | "LWORD" => int(64, false),
            "REAL" => Ty::Real(false),
            "LREAL" => Ty::Real(true),
            "TIME" | "LTIME" => Ty::Time,
            "STRING" | "WSTRING" => {
                if self.eat_sym("[") {
                    self.int_const()?;
                    self.sym("]")?;
                }
                Ty::Str
            }
            _ => Ty::Named(name),
        };
        Ok(t)
    }

    fn int_const(&mut self) -> R<i64> {
        let neg = self.eat_sym("-");
        match self.bump() {
            Tok::Int(v) => Ok(if neg { -v } else { v }),
            _ => self.err("expected an integer"),
        }
    }

    fn stmts(&mut self, ends: &[&str]) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        loop {
            if let Tok::Kw(k) = self.peek() {
                if ends.contains(&k.as_str()) {
                    return Ok(out);
                }
            }
            if matches!(self.peek(), Tok::Eof) {
                return self.err(&format!("expected `{}`", ends.join("` or `")));
            }
            if self.eat_sym(";") {
                continue;
            }
            // A CASE label ends the statements of the previous label.
            if ends.contains(&"<label>") && self.at_case_label() {
                return Ok(out);
            }
            out.push(self.stmt()?);
        }
    }

    fn at_case_label(&self) -> bool {
        let mut j = self.i;
        let mut seen = false;
        loop {
            match &self.t[j].0 {
                Tok::Int(_) | Tok::Ident(_) => seen = true,
                Tok::Sym("-" | "," | "..") => {}
                Tok::Sym(":") => return seen,
                _ => return false,
            }
            j += 1;
            if j >= self.t.len() {
                return false;
            }
        }
    }

    fn stmt(&mut self) -> R<Stmt> {
        let pos = self.pos();
        if self.eat_kw("IF") {
            let mut branches = Vec::new();
            let c = self.expr()?;
            self.kw("THEN")?;
            let b = self.stmts(&["ELSIF", "ELSE", "END_IF"])?;
            branches.push((c, b));
            let mut otherwise = None;
            loop {
                if self.eat_kw("ELSIF") {
                    let c = self.expr()?;
                    self.kw("THEN")?;
                    let b = self.stmts(&["ELSIF", "ELSE", "END_IF"])?;
                    branches.push((c, b));
                } else if self.eat_kw("ELSE") {
                    otherwise = Some(self.stmts(&["END_IF"])?);
                } else {
                    break;
                }
            }
            self.kw("END_IF")?;
            self.eat_sym(";");
            return Ok(Stmt::If(branches, otherwise, pos));
        }
        if self.eat_kw("CASE") {
            let e = self.expr()?;
            self.kw("OF")?;
            let mut arms = Vec::new();
            let mut otherwise = None;
            loop {
                if self.eat_kw("ELSE") {
                    otherwise = Some(self.stmts(&["END_CASE"])?);
                    break;
                }
                if self.is_kw("END_CASE") {
                    break;
                }
                let mut labels = Vec::new();
                loop {
                    let a = self.int_const()?;
                    if self.eat_sym("..") {
                        let b = self.int_const()?;
                        labels.push(CaseLabel::Range(a, b));
                    } else {
                        labels.push(CaseLabel::Value(a));
                    }
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.sym(":")?;
                let body = self.stmts(&["ELSE", "END_CASE", "<label>"])?;
                arms.push((labels, body));
            }
            self.kw("END_CASE")?;
            self.eat_sym(";");
            return Ok(Stmt::Case(e, arms, otherwise, pos));
        }
        if self.eat_kw("FOR") {
            let v = self.ident()?;
            self.sym(":=")?;
            let a = self.expr()?;
            self.kw("TO")?;
            let b = self.expr()?;
            let by = if self.eat_kw("BY") {
                Some(self.expr()?)
            } else {
                None
            };
            self.kw("DO")?;
            let body = self.stmts(&["END_FOR"])?;
            self.kw("END_FOR")?;
            self.eat_sym(";");
            return Ok(Stmt::For(v, a, b, by, body, pos));
        }
        if self.eat_kw("WHILE") {
            let c = self.expr()?;
            self.kw("DO")?;
            let body = self.stmts(&["END_WHILE"])?;
            self.kw("END_WHILE")?;
            self.eat_sym(";");
            return Ok(Stmt::While(c, body, pos));
        }
        if self.eat_kw("REPEAT") {
            let body = self.stmts(&["UNTIL"])?;
            self.kw("UNTIL")?;
            let c = self.expr()?;
            self.kw("END_REPEAT")?;
            self.eat_sym(";");
            return Ok(Stmt::Repeat(body, c, pos));
        }
        if self.eat_kw("EXIT") {
            self.eat_sym(";");
            return Ok(Stmt::Exit(pos));
        }
        if self.eat_kw("RETURN") {
            self.eat_sym(";");
            return Ok(Stmt::Return(pos));
        }
        let target = self.postfix()?;
        if self.eat_sym(":=") {
            let v = self.expr()?;
            self.sym(";")?;
            return Ok(Stmt::Assign(target, v, pos));
        }
        if let Expr::Call(name, args) = target {
            self.sym(";")?;
            return Ok(Stmt::Call(Expr::Var(name), args, pos));
        }
        self.err("expected `:=` or a call")
    }

    fn args(&mut self) -> R<Vec<Arg>> {
        let mut args = Vec::new();
        if self.eat_sym(")") {
            return Ok(args);
        }
        loop {
            let named = matches!(self.peek(), Tok::Ident(_))
                && matches!(self.t.get(self.i + 1), Some((Tok::Sym(":=" | "=>"), _)));
            if named {
                let n = self.ident()?;
                let output = matches!(self.bump(), Tok::Sym("=>"));
                let v = self.expr()?;
                args.push(Arg {
                    name: Some(n),
                    output,
                    value: v,
                });
            } else {
                args.push(Arg {
                    name: None,
                    output: false,
                    value: self.expr()?,
                });
            }
            if !self.eat_sym(",") {
                break;
            }
        }
        self.sym(")")?;
        Ok(args)
    }

    fn expr(&mut self) -> R<Expr> {
        self.binary(0)
    }

    fn binary(&mut self, level: usize) -> R<Expr> {
        const LEVELS: &[&[&str]] = &[
            &["OR"],
            &["XOR"],
            &["AND", "&"],
            &["=", "<>"],
            &["<", ">", "<=", ">="],
            &["+", "-"],
            &["*", "/", "MOD"],
            &["**"],
        ];
        if level >= LEVELS.len() {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        loop {
            let op = match self.peek() {
                Tok::Kw(k) if LEVELS[level].contains(&k.as_str()) => k.clone(),
                Tok::Sym(s) if LEVELS[level].contains(s) => {
                    if *s == "&" {
                        "AND".to_string()
                    } else {
                        (*s).to_string()
                    }
                }
                _ => return Ok(lhs),
            };
            self.bump();
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn unary(&mut self) -> R<Expr> {
        if self.eat_kw("NOT") {
            return Ok(Expr::Unary("NOT".into(), Box::new(self.unary()?)));
        }
        if self.eat_sym("-") {
            return Ok(match self.unary()? {
                Expr::Int(v) => Expr::Int(-v),
                Expr::Real(v) => Expr::Real(-v),
                Expr::Time(v) => Expr::Time(-v),
                e => Expr::Unary("-".into(), Box::new(e)),
            });
        }
        if self.eat_sym("+") {
            return self.unary();
        }
        self.postfix()
    }

    fn postfix(&mut self) -> R<Expr> {
        let pos = self.pos();
        let mut e = match self.bump() {
            Tok::Int(v) => return Ok(Expr::Int(v)),
            Tok::Real(v) => return Ok(Expr::Real(v)),
            Tok::Time(v) => return Ok(Expr::Time(v)),
            Tok::Str(s) => return Ok(Expr::Str(s)),
            Tok::Kw(k) if k == "TRUE" => return Ok(Expr::Bool(true)),
            Tok::Kw(k) if k == "FALSE" => return Ok(Expr::Bool(false)),
            Tok::Sym("(") => {
                let e = self.expr()?;
                self.sym(")")?;
                e
            }
            Tok::Ident(n) => {
                if self.eat_sym("(") {
                    let a = self.args()?;
                    Expr::Call(n, a)
                } else {
                    Expr::Var(n)
                }
            }
            other => {
                return Err(ParseError {
                    pos,
                    message: format!("expected an expression, found {}", describe(&other)),
                });
            }
        };
        loop {
            if self.is_sym(".") {
                self.bump();
                let f = self.ident()?;
                e = Expr::Field(Box::new(e), f);
            } else if self.eat_sym("[") {
                let ix = self.expr()?;
                self.sym("]")?;
                e = Expr::Index(Box::new(e), Box::new(ix));
            } else {
                return Ok(e);
            }
        }
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("`{s}`"),
        Tok::Kw(s) => format!("`{s}`"),
        Tok::Int(v) => format!("`{v}`"),
        Tok::Real(v) => format!("`{v}`"),
        Tok::Time(_) => "a time literal".into(),
        Tok::Str(_) => "a string".into(),
        Tok::Addr(a) => format!("`{a}`"),
        Tok::Sym(s) => format!("`{s}`"),
        Tok::Eof => "the end of the file".into(),
    }
}
