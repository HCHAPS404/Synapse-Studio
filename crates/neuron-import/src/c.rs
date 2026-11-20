//! A C subset to a `core` module (task C5.3).
//!
//! The subset: functions over the integer types (`int`, `unsigned`, `char`, `short`, `long`,
//! `bool`/`_Bool` and the `stdint.h` names), `const` globals and `#define` integer constants,
//! local variables, `if`/`else`, `while` and `for` with a bound (`/* max N */` after the
//! condition, or constant bounds `for (int i = 0; i < 8; i++)`), `break`, `continue`,
//! `return`, the arithmetic, bit, comparison and logical operators, `?:`, widening casts and
//! calls between the imported functions.
//!
//! Integer semantics: `+`, `-` and `*` become the wrapping operators (`+%`, `-%`, `*%`), the
//! behavior of C unsigned arithmetic; `<<` on an unsigned type multiplies with wrap-around.
//! Conversions between integer types are written only when they lose nothing (widening); a
//! narrowing or sign-changing conversion is `NE1002`, because Neuron has no wrapping cast.
//!
//! Not imported (`NE1002`, the rest of the file is): pointers, arrays, structs, unions, enums,
//! floating point, strings, `goto`, `switch`, `do`, an unbounded loop, recursion, mutable
//! globals, `void` functions, function-like macros and conditional compilation.

use crate::st::{finish, Out};
use crate::util::{ident, module_name, syntax};
use neuron_emit::{Class, Diag, Fidelity};
use std::collections::{BTreeMap, BTreeSet};

/// A byte range of the input.
type Sp = [usize; 2];

// ---------------------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum K {
    Id(String),
    Int(i128),
    Float,
    Str,
    P(&'static str),
    /// `#` that starts a preprocessor line.
    Hash,
    /// End of a preprocessor line.
    EndDir,
}

#[derive(Debug, Clone)]
struct Tok {
    k: K,
    pos: usize,
    end: usize,
}

struct Lexed {
    toks: Vec<Tok>,
    /// Bound comments (`/* max 100 */`): position and bound.
    bounds: Vec<(usize, u64)>,
}

const PUNCT: &[&str] = &[
    "<<=", ">>=", "...", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>",
    "<=", ">=", "==", "!=", "&&", "||", "->", "+", "-", "*", "/", "%", "&", "|", "^", "~", "!",
    "<", ">", "=", "?", ":", ";", ",", "(", ")", "{", "}", "[", "]", ".", "#",
];

/// `max 100`, `max: 100` or `MAX=100` inside a comment.
fn max_bound(comment: &str) -> Option<u64> {
    let lower = comment.to_ascii_lowercase();
    let at = lower.find("max")?;
    let rest = lower[at + 3..].trim_start_matches([' ', '\t', ':', '=']);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn escape(b: &[u8], i: &mut usize) -> Option<i128> {
    // `b[*i]` is the character after the backslash.
    let c = *b.get(*i)?;
    *i += 1;
    Some(match c {
        b'n' => 10,
        b't' => 9,
        b'r' => 13,
        b'0' => 0,
        b'a' => 7,
        b'b' => 8,
        b'f' => 12,
        b'v' => 11,
        b'x' => {
            let s = *i;
            while *i < b.len() && b[*i].is_ascii_hexdigit() {
                *i += 1;
            }
            let t = std::str::from_utf8(&b[s..*i]).ok()?;
            i128::from_str_radix(t, 16).ok()?
        }
        other => i128::from(other),
    })
}

fn lex(text: &str) -> Result<Lexed, (usize, usize, String)> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut toks: Vec<Tok> = Vec::new();
    let mut bounds = Vec::new();
    let mut line_start = true;
    let mut in_dir = false;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            if in_dir && !(i > 0 && b[i - 1] == b'\\') {
                toks.push(Tok {
                    k: K::EndDir,
                    pos: i,
                    end: i,
                });
                in_dir = false;
            }
            line_start = true;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() || (c == b'\\' && in_dir) {
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            let s = i;
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            if let Some(n) = max_bound(&text[s..i]) {
                bounds.push((s, n));
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let s = i;
            i += 2;
            loop {
                if i + 1 >= b.len() {
                    return Err((s, 2, "unterminated comment".into()));
                }
                if b[i] == b'*' && b[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            if let Some(n) = max_bound(&text[s..i]) {
                bounds.push((s, n));
            }
            continue;
        }
        let start = i;
        if c == b'#' && line_start && !in_dir {
            in_dir = true;
            line_start = false;
            i += 1;
            toks.push(Tok {
                k: K::Hash,
                pos: start,
                end: i,
            });
            continue;
        }
        line_start = false;
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let bad = |m: &str| (start, 1, m.to_string());
            let mut float = false;
            let mut value: i128 = 0;
            if c == b'0' && b.get(i + 1).is_some_and(|x| x | 0x20 == b'x') {
                i += 2;
                let d = i;
                while i < b.len() && b[i].is_ascii_hexdigit() {
                    i += 1;
                }
                value = i128::from_str_radix(&text[d..i], 16).map_err(|_| bad("bad number"))?;
            } else if c == b'0' && b.get(i + 1).is_some_and(|x| x | 0x20 == b'b') {
                i += 2;
                let d = i;
                while i < b.len() && (b[i] == b'0' || b[i] == b'1') {
                    i += 1;
                }
                value = i128::from_str_radix(&text[d..i], 2).map_err(|_| bad("bad number"))?;
            } else {
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i < b.len() && b[i] == b'.' {
                    float = true;
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < b.len() && b[i] | 0x20 == b'e' {
                    let mut j = i + 1;
                    if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                        j += 1;
                    }
                    if j < b.len() && b[j].is_ascii_digit() {
                        float = true;
                        i = j;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                if !float {
                    let t = &text[start..i];
                    value = if t.len() > 1 && t.starts_with('0') {
                        i128::from_str_radix(t, 8).map_err(|_| bad("bad octal number"))?
                    } else {
                        t.parse().map_err(|_| bad("bad number"))?
                    };
                }
            }
            while i < b.len() && matches!(b[i] | 0x20, b'u' | b'l' | b'f') {
                i += 1;
            }
            if i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                return Err((start, i - start + 1, "bad number suffix".into()));
            }
            toks.push(Tok {
                k: if float { K::Float } else { K::Int(value) },
                pos: start,
                end: i,
            });
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            toks.push(Tok {
                k: K::Id(text[start..i].to_string()),
                pos: start,
                end: i,
            });
            continue;
        }
        if c == b'\'' {
            i += 1;
            let v = match b.get(i) {
                Some(b'\\') => {
                    i += 1;
                    escape(b, &mut i)
                }
                Some(&x) if x != b'\'' && x != b'\n' => {
                    i += 1;
                    Some(i128::from(x))
                }
                _ => None,
            };
            if v.is_none() || b.get(i) != Some(&b'\'') {
                return Err((start, 1, "bad character constant".into()));
            }
            i += 1;
            toks.push(Tok {
                k: K::Int(v.unwrap_or(0)),
                pos: start,
                end: i,
            });
            continue;
        }
        if c == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            if i >= b.len() || b[i] != b'"' {
                return Err((start, 1, "unterminated string".into()));
            }
            i += 1;
            toks.push(Tok {
                k: K::Str,
                pos: start,
                end: i,
            });
            continue;
        }
        match PUNCT.iter().find(|p| b[i..].starts_with(p.as_bytes())) {
            Some(&p) => {
                i += p.len();
                toks.push(Tok {
                    k: K::P(p),
                    pos: start,
                    end: i,
                });
            }
            None => {
                return Err((start, 1, format!("unexpected character `{}`", c as char)));
            }
        }
    }
    if in_dir {
        toks.push(Tok {
            k: K::EndDir,
            pos: b.len(),
            end: b.len(),
        });
    }
    Ok(Lexed { toks, bounds })
}

// ---------------------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    Void,
    Bool,
    Int(u8, bool),
}

fn ty_name(t: Ty) -> String {
    match t {
        Ty::Void => "()".into(),
        Ty::Bool => "bool".into(),
        Ty::Int(b, true) => format!("i{b}"),
        Ty::Int(b, false) => format!("u{b}"),
    }
}

#[derive(Debug, Clone)]
enum Ex {
    Int(i128, Sp),
    Var(String, Sp),
    Un(&'static str, Box<Ex>, Sp),
    Bin(&'static str, Box<Ex>, Box<Ex>, Sp),
    Tern(Box<Ex>, Box<Ex>, Box<Ex>, Sp),
    Call(String, Vec<Ex>, Sp),
    Cast(Ty, Box<Ex>, Sp),
}

impl Ex {
    fn sp(&self) -> Sp {
        match self {
            Ex::Int(_, s)
            | Ex::Var(_, s)
            | Ex::Un(_, _, s)
            | Ex::Bin(_, _, _, s)
            | Ex::Tern(_, _, _, s)
            | Ex::Call(_, _, s)
            | Ex::Cast(_, _, s) => *s,
        }
    }
}

#[derive(Debug, Clone)]
enum St {
    Decl {
        ty: Ty,
        name: String,
        init: Option<Ex>,
        sp: Sp,
    },
    Assign {
        name: String,
        op: &'static str,
        rhs: Ex,
        sp: Sp,
    },
    IncDec {
        name: String,
        plus: bool,
        sp: Sp,
    },
    If {
        cond: Ex,
        then: Vec<St>,
        els: Vec<St>,
    },
    While {
        cond: Ex,
        body: Vec<St>,
        bound: Option<u64>,
        sp: Sp,
    },
    For {
        init: Option<Box<St>>,
        cond: Option<Ex>,
        step: Option<Box<St>>,
        body: Vec<St>,
        bound: Option<u64>,
        sp: Sp,
    },
    Return(Option<Ex>, Sp),
    Break,
    Continue,
}

type Env = BTreeMap<String, (i128, Option<Ty>)>;

#[derive(Debug, Clone)]
struct Func {
    name: String,
    ret: Ty,
    params: Vec<(Ty, String)>,
    /// `None` for a prototype; `Err` when the body has an unsupported construct.
    body: Option<Result<Vec<St>, (Sp, String)>>,
    is_static: bool,
    sp: Sp,
}

#[derive(Debug, Clone)]
struct Konst {
    name: String,
    ty: Option<Ty>,
    value: i128,
}

#[derive(Debug, Clone)]
enum Item {
    Fn(Func),
    Const(Konst),
}

fn wrap_to(v: i128, bits: u8, signed: bool) -> i128 {
    let m = 1i128 << bits;
    let u = v.rem_euclid(m);
    if signed && u >= m >> 1 {
        u - m
    } else {
        u
    }
}

fn fits(v: i128, bits: u8, signed: bool) -> bool {
    if signed {
        (-(1i128 << (bits - 1))..(1i128 << (bits - 1))).contains(&v)
    } else {
        (0..(1i128 << bits)).contains(&v)
    }
}

/// The value of a constant expression.
fn fold(e: &Ex, env: &Env) -> Option<i128> {
    match e {
        Ex::Int(v, _) => Some(*v),
        Ex::Var(n, _) => env.get(n).map(|c| c.0),
        Ex::Un(op, x, _) => {
            let v = fold(x, env)?;
            match *op {
                "-" => v.checked_neg(),
                "+" => Some(v),
                "~" => Some(!v),
                "!" => Some(i128::from(v == 0)),
                _ => None,
            }
        }
        Ex::Bin(op, a, b, _) => {
            let (x, y) = (fold(a, env)?, fold(b, env)?);
            match *op {
                "+" => x.checked_add(y),
                "-" => x.checked_sub(y),
                "*" => x.checked_mul(y),
                "/" => x.checked_div(y),
                "%" => x.checked_rem(y),
                "<<" => u32::try_from(y)
                    .ok()
                    .filter(|s| *s < 100)
                    .and_then(|s| x.checked_shl(s)),
                ">>" => u32::try_from(y).ok().filter(|s| *s < 127).map(|s| x >> s),
                "&" => Some(x & y),
                "|" => Some(x | y),
                "^" => Some(x ^ y),
                "==" => Some(i128::from(x == y)),
                "!=" => Some(i128::from(x != y)),
                "<" => Some(i128::from(x < y)),
                "<=" => Some(i128::from(x <= y)),
                ">" => Some(i128::from(x > y)),
                ">=" => Some(i128::from(x >= y)),
                "&&" => Some(i128::from(x != 0 && y != 0)),
                "||" => Some(i128::from(x != 0 || y != 0)),
                _ => None,
            }
        }
        Ex::Tern(c, a, b, _) => {
            if fold(c, env)? != 0 {
                fold(a, env)
            } else {
                fold(b, env)
            }
        }
        Ex::Cast(t, x, _) => {
            let v = fold(x, env)?;
            match t {
                Ty::Int(bits, s) => Some(wrap_to(v, *bits, *s)),
                Ty::Bool => Some(i128::from(v != 0)),
                Ty::Void => None,
            }
        }
        Ex::Call(..) => None,
    }
}

// ---------------------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------------------

enum PErr {
    Syntax(Sp, String),
    Unsup(Sp, String),
}

type PRes<T> = Result<T, PErr>;

struct TyInfo {
    ty: Ty,
    konst: bool,
    stat: bool,
}

const TYPE_WORDS: &[&str] = &[
    "const",
    "static",
    "inline",
    "extern",
    "volatile",
    "register",
    "unsigned",
    "signed",
    "char",
    "short",
    "int",
    "long",
    "_Bool",
    "bool",
    "void",
    "float",
    "double",
    "struct",
    "union",
    "enum",
    "size_t",
    "ssize_t",
    "intptr_t",
    "uintptr_t",
    "int8_t",
    "int16_t",
    "int32_t",
    "int64_t",
    "uint8_t",
    "uint16_t",
    "uint32_t",
    "uint64_t",
];

fn bin_prec(op: &str) -> Option<u8> {
    Some(match op {
        "||" => 1,
        "&&" => 2,
        "|" => 3,
        "^" => 4,
        "&" => 5,
        "==" | "!=" => 6,
        "<" | "<=" | ">" | ">=" => 7,
        "<<" | ">>" => 8,
        "+" | "-" => 9,
        "*" | "/" | "%" => 10,
        _ => return None,
    })
}

const ASSIGN_OPS: &[(&str, &str)] = &[
    ("=", "="),
    ("+=", "+"),
    ("-=", "-"),
    ("*=", "*"),
    ("/=", "/"),
    ("%=", "%"),
    ("&=", "&"),
    ("|=", "|"),
    ("^=", "^"),
    ("<<=", "<<"),
    (">>=", ">>"),
];

struct P<'a> {
    t: &'a [Tok],
    i: usize,
    end: usize,
    bounds: &'a [(usize, u64)],
    aliases: BTreeMap<String, Ty>,
    consts: Env,
    items: Vec<Item>,
    unsup: Vec<(Sp, String)>,
    notes: Vec<(Sp, &'static str, String)>,
}

impl<'a> P<'a> {
    fn tok(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }

    fn sp(&self) -> Sp {
        self.t
            .get(self.i)
            .map_or([self.end, self.end + 1], |t| [t.pos, t.end.max(t.pos + 1)])
    }

    fn at_p(&self, s: &str) -> bool {
        self.nth_p(0, s)
    }

    fn nth_p(&self, n: usize, s: &str) -> bool {
        matches!(self.t.get(self.i + n), Some(Tok { k: K::P(x), .. }) if *x == s)
    }

    fn at_id(&self, s: &str) -> bool {
        matches!(self.tok(), Some(Tok { k: K::Id(x), .. }) if x == s)
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.at_p(s) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str) -> PRes<()> {
        if self.eat(s) {
            Ok(())
        } else {
            Err(PErr::Syntax(self.sp(), format!("expected `{s}`")))
        }
    }

    fn id(&mut self) -> PRes<String> {
        match self.tok() {
            Some(Tok { k: K::Id(s), .. }) => {
                let s = s.clone();
                self.i += 1;
                Ok(s)
            }
            _ => Err(PErr::Syntax(self.sp(), "expected a name".into())),
        }
    }

    fn starts_type_at(&self, idx: usize) -> bool {
        match self.t.get(idx) {
            Some(Tok { k: K::Id(s), .. }) => {
                TYPE_WORDS.contains(&s.as_str()) || self.aliases.contains_key(s)
            }
            _ => false,
        }
    }

    fn starts_type(&self) -> bool {
        self.starts_type_at(self.i)
    }

    fn parse_type(&mut self) -> PRes<TyInfo> {
        let (mut uns, mut sgn, mut short, mut longs, mut chr) = (false, false, false, 0u8, false);
        let mut saw = false;
        let mut base: Option<Ty> = None;
        let (mut konst, mut stat) = (false, false);
        let first = self.sp();
        while let Some(Tok { k: K::Id(w), .. }) = self.tok() {
            let w = w.clone();
            let sp = self.sp();
            match w.as_str() {
                "const" => konst = true,
                "static" => stat = true,
                "inline" | "register" => {}
                "int" => saw = true,
                "extern" | "volatile" => return Err(PErr::Unsup(sp, format!("`{w}`"))),
                "unsigned" => {
                    uns = true;
                    saw = true;
                }
                "signed" => {
                    sgn = true;
                    saw = true;
                }
                "short" => {
                    short = true;
                    saw = true;
                }
                "long" => {
                    longs += 1;
                    saw = true;
                }
                "char" => {
                    chr = true;
                    saw = true;
                }
                "_Bool" | "bool" => {
                    base = Some(Ty::Bool);
                    saw = true;
                }
                "void" => {
                    base = Some(Ty::Void);
                    saw = true;
                }
                "float" | "double" => {
                    return Err(PErr::Unsup(sp, "floating-point types".into()));
                }
                "struct" | "union" | "enum" => {
                    return Err(PErr::Unsup(sp, format!("`{w}` types")));
                }
                "size_t" | "ssize_t" | "intptr_t" | "uintptr_t" => {
                    return Err(PErr::Unsup(
                        sp,
                        format!("`{w}`, whose width depends on the platform"),
                    ));
                }
                "int8_t" | "int16_t" | "int32_t" | "int64_t" | "uint8_t" | "uint16_t"
                | "uint32_t" | "uint64_t" => {
                    let bits: u8 = w
                        .trim_start_matches('u')
                        .trim_start_matches("int")
                        .trim_end_matches("_t")
                        .parse()
                        .unwrap_or(32);
                    base = Some(Ty::Int(bits, !w.starts_with('u')));
                    saw = true;
                }
                other => match self.aliases.get(other) {
                    Some(t) => {
                        base = Some(*t);
                        saw = true;
                    }
                    None => break,
                },
            }
            self.i += 1;
        }
        if !saw {
            let what = match self.tok() {
                Some(Tok { k: K::Id(n), .. }) => {
                    format!("the type `{n}`, which is not a C integer type")
                }
                _ => "a declaration without a type".to_string(),
            };
            return Err(PErr::Unsup(self.sp(), what));
        }
        let ty = match base {
            Some(t) => t,
            None => {
                let bits = if chr {
                    8
                } else if short {
                    16
                } else if longs > 0 {
                    64
                } else {
                    32
                };
                if longs > 0 {
                    self.notes.push((
                        first,
                        "long",
                        "`long` is imported as a 64-bit integer (LP64); on Windows it has 32 bits"
                            .into(),
                    ));
                }
                if chr && !uns && !sgn {
                    self.notes.push((
                        first,
                        "char",
                        "plain `char` is imported as `i8` (it is signed on most targets)".into(),
                    ));
                }
                Ty::Int(bits, !uns)
            }
        };
        if self.at_p("*") {
            return Err(PErr::Unsup(self.sp(), "pointers".into()));
        }
        Ok(TyInfo { ty, konst, stat })
    }

    fn match_brace(&self, open: usize) -> Option<usize> {
        let mut depth = 0i32;
        for (j, t) in self.t.iter().enumerate().skip(open) {
            if let K::P(p) = &t.k {
                if *p == "{" {
                    depth += 1;
                } else if *p == "}" {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
            }
        }
        None
    }

    /// Moves past the item that starts at token `start` (after an `NE1002`).
    fn skip_item(&mut self, start: usize) {
        let mut i = start;
        let mut depth = 0i32;
        let mut init = false;
        while i < self.t.len() {
            if let K::P(p) = &self.t[i].k {
                match *p {
                    "(" | "[" => depth += 1,
                    ")" | "]" => depth -= 1,
                    "=" if depth == 0 => init = true,
                    "{" if depth == 0 => {
                        let Some(c) = self.match_brace(i) else {
                            i = self.t.len();
                            break;
                        };
                        i = c + 1;
                        if !init {
                            if matches!(self.t.get(i), Some(Tok { k: K::P(";"), .. })) {
                                i += 1;
                            }
                            break;
                        }
                        continue;
                    }
                    ";" if depth == 0 => {
                        i += 1;
                        break;
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        self.i = i.max(start + 1);
    }

    fn top(&mut self) -> PRes<()> {
        while self.i < self.t.len() {
            let start = self.i;
            match self.item() {
                Ok(()) => {}
                Err(PErr::Unsup(sp, m)) => {
                    self.unsup.push((sp, m));
                    self.skip_item(start);
                }
                Err(e) => return Err(e),
            }
            if self.i == start {
                self.i += 1;
            }
        }
        Ok(())
    }

    fn dir_end(&self) -> usize {
        (self.i..self.t.len())
            .find(|&j| self.t[j].k == K::EndDir)
            .unwrap_or(self.t.len())
    }

    fn directive(&mut self) -> PRes<()> {
        self.i += 1;
        let dsp = self.sp();
        let name = match self.tok() {
            Some(Tok { k: K::Id(n), .. }) => n.clone(),
            _ => String::new(),
        };
        if !name.is_empty() {
            self.i += 1;
        }
        let end = self.dir_end();
        match name.as_str() {
            "include" | "pragma" | "" => {}
            "define" => self.define(end, dsp),
            other => self.unsup.push((
                dsp,
                format!("the preprocessor directive `#{other}` (conditional compilation)"),
            )),
        }
        self.i = end + 1;
        Ok(())
    }

    fn define(&mut self, end: usize, dsp: Sp) {
        let Some(Tok {
            k: K::Id(name),
            end: name_end,
            ..
        }) = self.t.get(self.i)
        else {
            return;
        };
        let (name, name_end) = (name.clone(), *name_end);
        if self.nth_p(1, "(") && self.t.get(self.i + 1).is_some_and(|t| t.pos == name_end) {
            self.unsup
                .push((dsp, format!("the function-like macro `{name}`")));
            return;
        }
        let from = self.i + 1;
        if from >= end {
            return;
        }
        let toks: &'a [Tok] = self.t;
        let mut sub = P {
            t: &toks[from..end],
            i: 0,
            end: self.end,
            bounds: &[],
            aliases: self.aliases.clone(),
            consts: self.consts.clone(),
            items: Vec::new(),
            unsup: Vec::new(),
            notes: Vec::new(),
        };
        let value = match sub.expr() {
            Ok(e) if sub.i == sub.t.len() => fold(&e, &self.consts),
            _ => None,
        };
        match value {
            Some(v) => {
                self.consts.insert(name.clone(), (v, None));
                self.items.push(Item::Const(Konst {
                    name,
                    ty: None,
                    value: v,
                }));
            }
            None => self.unsup.push((
                dsp,
                format!("the macro `{name}`, which is not an integer constant"),
            )),
        }
    }

    fn typedef(&mut self) -> PRes<()> {
        self.i += 1;
        if !self.starts_type() {
            return Err(PErr::Unsup(self.sp(), "this `typedef`".into()));
        }
        let info = self.parse_type()?;
        let name = self.id()?;
        if self.at_p("[") || self.at_p("(") {
            return Err(PErr::Unsup(self.sp(), "this `typedef`".into()));
        }
        self.expect(";")?;
        self.aliases.insert(name, info.ty);
        Ok(())
    }

    fn item(&mut self) -> PRes<()> {
        match self.tok().map(|t| t.k.clone()) {
            Some(K::Hash) => return self.directive(),
            Some(K::P(";")) => {
                self.i += 1;
                return Ok(());
            }
            Some(K::Id(w)) if w == "typedef" => return self.typedef(),
            _ => {}
        }
        if !self.starts_type() {
            return Err(PErr::Syntax(self.sp(), "expected a declaration".into()));
        }
        let info = self.parse_type()?;
        let name_sp = self.sp();
        let name = self.id()?;
        if self.at_p("(") {
            return self.function(&info, name, name_sp);
        }
        if self.at_p("[") {
            return Err(PErr::Unsup(self.sp(), "arrays".into()));
        }
        if !info.konst || info.ty == Ty::Void {
            return Err(PErr::Unsup(
                name_sp,
                format!(
                    "the global variable `{name}` (only `const` globals and `#define` constants are imported)"
                ),
            ));
        }
        self.expect("=")?;
        let e = self.expr()?;
        let Some(v) = fold(&e, &self.consts) else {
            return Err(PErr::Unsup(
                e.sp(),
                "an initializer that is not an integer constant".into(),
            ));
        };
        if let Ty::Int(b, s) = info.ty {
            if !fits(v, b, s) {
                return Err(PErr::Unsup(
                    e.sp(),
                    format!(
                        "the constant {v}, which does not fit `{}`",
                        ty_name(info.ty)
                    ),
                ));
            }
        }
        if !self.at_p(";") {
            return Err(PErr::Unsup(self.sp(), "several declarators".into()));
        }
        self.i += 1;
        self.consts.insert(name.clone(), (v, Some(info.ty)));
        self.items.push(Item::Const(Konst {
            name,
            ty: Some(info.ty),
            value: v,
        }));
        Ok(())
    }

    fn function(&mut self, info: &TyInfo, name: String, sp: Sp) -> PRes<()> {
        self.expect("(")?;
        let mut params: Vec<(Ty, String)> = Vec::new();
        if self.at_id("void") && self.nth_p(1, ")") {
            self.i += 1;
        } else if !self.at_p(")") {
            loop {
                if self.at_p("...") {
                    return Err(PErr::Unsup(self.sp(), "variadic functions".into()));
                }
                if !self.starts_type() {
                    return Err(PErr::Syntax(self.sp(), "expected a parameter type".into()));
                }
                let pi = self.parse_type()?;
                if pi.ty == Ty::Void {
                    return Err(PErr::Unsup(self.sp(), "a `void` parameter".into()));
                }
                let pname = if self.at_p(",") || self.at_p(")") {
                    format!("arg{}", params.len())
                } else {
                    self.id()?
                };
                if self.at_p("[") {
                    return Err(PErr::Unsup(self.sp(), "array parameters".into()));
                }
                params.push((pi.ty, pname));
                if !self.eat(",") {
                    break;
                }
            }
        }
        self.expect(")")?;
        if self.eat(";") {
            self.items.push(Item::Fn(Func {
                name,
                ret: info.ty,
                params,
                body: None,
                is_static: info.stat,
                sp,
            }));
            return Ok(());
        }
        if !self.at_p("{") {
            return Err(PErr::Syntax(self.sp(), "expected `;` or `{`".into()));
        }
        let Some(close) = self.match_brace(self.i) else {
            return Err(PErr::Syntax(self.sp(), "this `{` is never closed".into()));
        };
        let body = match self.block() {
            Ok(b) => Ok(b),
            Err(PErr::Unsup(sp, m)) => Err((sp, m)),
            Err(e) => return Err(e),
        };
        self.i = close + 1;
        self.items.push(Item::Fn(Func {
            name,
            ret: info.ty,
            params,
            body: Some(body),
            is_static: info.stat,
            sp,
        }));
        Ok(())
    }

    fn block(&mut self) -> PRes<Vec<St>> {
        self.expect("{")?;
        let mut v = Vec::new();
        while !self.at_p("}") {
            if self.tok().is_none() {
                return Err(PErr::Syntax(self.sp(), "unexpected end of input".into()));
            }
            v.extend(self.stmt()?);
        }
        self.i += 1;
        Ok(v)
    }

    fn body(&mut self) -> PRes<Vec<St>> {
        if self.at_p("{") {
            self.block()
        } else {
            self.stmt()
        }
    }

    /// Position of the first token of the body that starts here.
    fn body_start(&self) -> usize {
        let at = if self.at_p("{") { self.i + 1 } else { self.i };
        self.t.get(at).map_or(self.end, |t| t.pos)
    }

    fn bound_in(&self, lo: usize, hi: usize) -> Option<u64> {
        self.bounds
            .iter()
            .find(|(p, _)| *p >= lo && *p < hi)
            .map(|(_, n)| *n)
    }

    fn stmt(&mut self) -> PRes<Vec<St>> {
        let sp = self.sp();
        let Some(tok) = self.tok() else {
            return Err(PErr::Syntax(sp, "unexpected end of input".into()));
        };
        match tok.k.clone() {
            K::P(";") => {
                self.i += 1;
                Ok(Vec::new())
            }
            K::P("{") => Err(PErr::Unsup(sp, "a nested block".into())),
            K::Hash => Err(PErr::Unsup(
                sp,
                "a preprocessor directive inside a function".into(),
            )),
            K::Id(w) => match w.as_str() {
                "if" => self.if_stmt(),
                "while" => self.while_stmt(),
                "for" => self.for_stmt(),
                "return" => {
                    self.i += 1;
                    let e = if self.at_p(";") {
                        None
                    } else {
                        Some(self.expr()?)
                    };
                    self.expect(";")?;
                    Ok(vec![St::Return(e, sp)])
                }
                "break" => {
                    self.i += 1;
                    self.expect(";")?;
                    Ok(vec![St::Break])
                }
                "continue" => {
                    self.i += 1;
                    self.expect(";")?;
                    Ok(vec![St::Continue])
                }
                "do" | "switch" | "goto" | "case" | "default" => {
                    Err(PErr::Unsup(sp, format!("`{w}`")))
                }
                _ => self.simple(true),
            },
            _ => self.simple(true),
        }
    }

    fn if_stmt(&mut self) -> PRes<Vec<St>> {
        self.i += 1;
        self.expect("(")?;
        let cond = self.expr()?;
        self.expect(")")?;
        let then = self.body()?;
        let els = if self.at_id("else") {
            self.i += 1;
            self.body()?
        } else {
            Vec::new()
        };
        Ok(vec![St::If { cond, then, els }])
    }

    fn while_stmt(&mut self) -> PRes<Vec<St>> {
        let kw = self.i;
        let sp = self.sp();
        self.i += 1;
        self.expect("(")?;
        let cond = self.expr()?;
        self.expect(")")?;
        let lo = if kw > 0 { self.t[kw - 1].end } else { 0 };
        let hi = self.body_start();
        let body = self.body()?;
        Ok(vec![St::While {
            cond,
            body,
            bound: self.bound_in(lo, hi),
            sp,
        }])
    }

    fn for_stmt(&mut self) -> PRes<Vec<St>> {
        let kw = self.i;
        let sp = self.sp();
        self.i += 1;
        self.expect("(")?;
        let init = if self.at_p(";") {
            None
        } else {
            Some(Box::new(self.single()?))
        };
        self.expect(";")?;
        let cond = if self.at_p(";") {
            None
        } else {
            Some(self.expr()?)
        };
        self.expect(";")?;
        let step = if self.at_p(")") {
            None
        } else {
            Some(Box::new(self.single()?))
        };
        if self.at_p(",") {
            return Err(PErr::Unsup(self.sp(), "the comma operator".into()));
        }
        self.expect(")")?;
        let lo = if kw > 0 { self.t[kw - 1].end } else { 0 };
        let hi = self.body_start();
        let body = self.body()?;
        Ok(vec![St::For {
            init,
            cond,
            step,
            body,
            bound: self.bound_in(lo, hi),
            sp,
        }])
    }

    /// One declaration or assignment without its `;` (a `for` header).
    fn single(&mut self) -> PRes<St> {
        let sp = self.sp();
        let mut v = self.simple(false)?;
        if v.len() != 1 {
            return Err(PErr::Unsup(
                sp,
                "several declarators in a `for` header".into(),
            ));
        }
        Ok(v.remove(0))
    }

    fn simple(&mut self, semi: bool) -> PRes<Vec<St>> {
        let sp = self.sp();
        let mut out = Vec::new();
        if self.starts_type() {
            let info = self.parse_type()?;
            if info.ty == Ty::Void {
                return Err(PErr::Unsup(sp, "a `void` variable".into()));
            }
            loop {
                let nsp = self.sp();
                let name = self.id()?;
                if self.at_p("[") {
                    return Err(PErr::Unsup(self.sp(), "arrays".into()));
                }
                if info.stat {
                    return Err(PErr::Unsup(nsp, "a `static` local variable".into()));
                }
                let init = if self.eat("=") {
                    Some(self.expr()?)
                } else {
                    None
                };
                out.push(St::Decl {
                    ty: info.ty,
                    name,
                    init,
                    sp: nsp,
                });
                if !self.eat(",") {
                    break;
                }
            }
        } else if self.at_p("++") || self.at_p("--") {
            let plus = self.at_p("++");
            self.i += 1;
            let name = self.id()?;
            out.push(St::IncDec { name, plus, sp });
        } else if self.at_p("*") {
            return Err(PErr::Unsup(sp, "pointers".into()));
        } else if matches!(
            self.tok(),
            Some(Tok {
                k: K::Int(_) | K::P("(" | "-" | "+" | "!" | "~"),
                ..
            })
        ) {
            return Err(PErr::Unsup(
                sp,
                "an expression statement (its value is dropped)".into(),
            ));
        } else if matches!(self.tok(), Some(Tok { k: K::Id(_), .. })) {
            let name = self.id()?;
            match self.tok().map(|t| t.k.clone()) {
                Some(K::P("++")) => {
                    self.i += 1;
                    out.push(St::IncDec {
                        name,
                        plus: true,
                        sp,
                    });
                }
                Some(K::P("--")) => {
                    self.i += 1;
                    out.push(St::IncDec {
                        name,
                        plus: false,
                        sp,
                    });
                }
                Some(K::P(op)) if ASSIGN_OPS.iter().any(|(a, _)| *a == op) => {
                    self.i += 1;
                    let bop = ASSIGN_OPS
                        .iter()
                        .find(|(a, _)| *a == op)
                        .map_or("=", |(_, b)| *b);
                    let rhs = self.expr()?;
                    out.push(St::Assign {
                        name,
                        op: bop,
                        rhs,
                        sp,
                    });
                }
                _ => {
                    return Err(PErr::Unsup(
                        sp,
                        "an expression statement (a call whose value is dropped, or an unknown type)"
                            .into(),
                    ));
                }
            }
        } else {
            return Err(PErr::Syntax(sp, "expected a statement".into()));
        }
        if semi {
            self.expect(";")?;
        }
        Ok(out)
    }

    fn expr(&mut self) -> PRes<Ex> {
        let c = self.bin(1)?;
        if self.at_p("?") {
            let sp = self.sp();
            self.i += 1;
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr()?;
            return Ok(Ex::Tern(Box::new(c), Box::new(a), Box::new(b), sp));
        }
        Ok(c)
    }

    fn bin(&mut self, min: u8) -> PRes<Ex> {
        let mut l = self.unary()?;
        while let Some(&Tok { k: K::P(op), .. }) = self.tok() {
            let Some(p) = bin_prec(op) else {
                break;
            };
            if p < min {
                break;
            }
            let sp = self.sp();
            self.i += 1;
            let r = self.bin(p + 1)?;
            l = Ex::Bin(op, Box::new(l), Box::new(r), sp);
        }
        Ok(l)
    }

    fn unary(&mut self) -> PRes<Ex> {
        let sp = self.sp();
        match self.tok().map(|t| t.k.clone()) {
            Some(K::P(op)) if matches!(op, "-" | "+" | "!" | "~") => {
                self.i += 1;
                let x = self.unary()?;
                Ok(Ex::Un(op, Box::new(x), sp))
            }
            Some(K::P("++" | "--")) => Err(PErr::Unsup(
                sp,
                "an increment or decrement inside an expression".into(),
            )),
            Some(K::P("*" | "&")) => Err(PErr::Unsup(sp, "pointers".into())),
            Some(K::P("(")) => {
                if self.starts_type_at(self.i + 1) {
                    self.i += 1;
                    let info = self.parse_type()?;
                    self.expect(")")?;
                    let x = self.unary()?;
                    Ok(Ex::Cast(info.ty, Box::new(x), sp))
                } else {
                    self.i += 1;
                    let e = self.expr()?;
                    self.expect(")")?;
                    self.postfix_check()?;
                    Ok(e)
                }
            }
            _ => self.primary(),
        }
    }

    fn postfix_check(&mut self) -> PRes<()> {
        let sp = self.sp();
        if self.at_p("[") {
            return Err(PErr::Unsup(sp, "arrays".into()));
        }
        if self.at_p(".") || self.at_p("->") {
            return Err(PErr::Unsup(sp, "structures".into()));
        }
        if self.at_p("++") || self.at_p("--") {
            return Err(PErr::Unsup(
                sp,
                "an increment or decrement inside an expression".into(),
            ));
        }
        Ok(())
    }

    fn primary(&mut self) -> PRes<Ex> {
        let sp = self.sp();
        let Some(tok) = self.tok() else {
            return Err(PErr::Syntax(sp, "expected an expression".into()));
        };
        match tok.k.clone() {
            K::Int(v) => {
                self.i += 1;
                self.postfix_check()?;
                Ok(Ex::Int(v, sp))
            }
            K::Float => Err(PErr::Unsup(sp, "floating-point constants".into())),
            K::Str => Err(PErr::Unsup(sp, "string literals".into())),
            K::Id(name) => {
                if name == "sizeof" {
                    return Err(PErr::Unsup(sp, "`sizeof`".into()));
                }
                self.i += 1;
                if name == "true" || name == "false" {
                    return Ok(Ex::Int(i128::from(name == "true"), sp));
                }
                if self.at_p("(") {
                    self.i += 1;
                    let mut args = Vec::new();
                    if !self.at_p(")") {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat(",") {
                                break;
                            }
                        }
                    }
                    self.expect(")")?;
                    self.postfix_check()?;
                    return Ok(Ex::Call(name, args, sp));
                }
                self.postfix_check()?;
                Ok(Ex::Var(name, sp))
            }
            _ => Err(PErr::Syntax(sp, "expected an expression".into())),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Conversion to Neuron
// ---------------------------------------------------------------------------------------

const COND: u8 = 0;
const OR: u8 = 1;
const AND: u8 = 2;
const NOT: u8 = 3;
const CMP: u8 = 4;
const BOR: u8 = 5;
const BXOR: u8 = 6;
const BAND: u8 = 7;
const SHIFT: u8 = 8;
const ADD: u8 = 9;
const MUL: u8 = 10;
const UNARY: u8 = 11;
const ATOM: u8 = 13;

/// The type of a converted expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum V {
    Bool,
    Int(u8, bool),
    /// An integer constant that takes the type of its context.
    Lit,
}

fn vty(t: Ty) -> V {
    match t {
        Ty::Bool => V::Bool,
        Ty::Int(b, s) => V::Int(b, s),
        Ty::Void => V::Lit,
    }
}

#[derive(Debug, Clone)]
struct Tx {
    s: String,
    prec: u8,
    ty: V,
    val: Option<i128>,
}

fn lit(v: i128) -> Tx {
    Tx {
        s: v.to_string(),
        prec: if v < 0 { UNARY } else { ATOM },
        ty: V::Lit,
        val: Some(v),
    }
}

fn par(t: &Tx, min: u8) -> String {
    if t.prec < min {
        format!("({})", t.s)
    } else {
        t.s.clone()
    }
}

/// A name that is a valid Neuron identifier and not a keyword.
fn safe_name(s: &str) -> String {
    let mut out = ident(s);
    if neuron_syntax::Keyword::lookup(&out).is_some() || matches!(out.as_str(), "result" | "old") {
        out.push('_');
    }
    out
}

struct Sigs {
    fns: BTreeMap<String, (Vec<Ty>, Ty)>,
    consts: Env,
    reserved: BTreeSet<String>,
}

struct Var {
    c: String,
    n: String,
    ty: Ty,
}

type CRes<T> = Result<T, (Sp, String)>;

struct Fx<'a> {
    env: &'a Sigs,
    name: String,
    ret: Ty,
    scopes: Vec<Vec<Var>>,
    assigned: BTreeSet<String>,
    out: Vec<String>,
    calls: BTreeSet<String>,
}

fn assigned_in(body: &[St], out: &mut BTreeSet<String>) {
    for s in body {
        match s {
            St::Assign { name, .. } | St::IncDec { name, .. } => {
                out.insert(name.clone());
            }
            St::If { then, els, .. } => {
                assigned_in(then, out);
                assigned_in(els, out);
            }
            St::While { body, .. } => assigned_in(body, out),
            St::For {
                init, step, body, ..
            } => {
                if let Some(i) = init {
                    assigned_in(std::slice::from_ref(&**i), out);
                }
                if let Some(i) = step {
                    assigned_in(std::slice::from_ref(&**i), out);
                }
                assigned_in(body, out);
            }
            _ => {}
        }
    }
}

fn has_continue(body: &[St]) -> bool {
    body.iter().any(|s| match s {
        St::Continue => true,
        St::If { then, els, .. } => has_continue(then) || has_continue(els),
        _ => false,
    })
}

impl Fx<'_> {
    fn put(&mut self, depth: usize, text: impl AsRef<str>) {
        self.out
            .push(format!("{}{}", "  ".repeat(depth), text.as_ref()));
    }

    fn lookup(&self, c: &str) -> Option<&Var> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|s| s.iter().rev())
            .find(|v| v.c == c)
    }

    fn visible(&self, n: &str) -> bool {
        self.env.reserved.contains(n) || self.scopes.iter().flatten().any(|v| v.n == n)
    }

    fn fresh(&self, base: &str) -> String {
        let b = safe_name(base);
        let mut n = b.clone();
        let mut k = 2;
        while self.visible(&n) {
            n = format!("{b}_{k}");
            k += 1;
        }
        n
    }

    fn coerce(&self, t: Tx, to: Ty, sp: Sp) -> CRes<Tx> {
        match (t.ty, to) {
            (_, Ty::Void) => Err((sp, "a `void` value".into())),
            (V::Lit, Ty::Int(b, s)) => match t.val {
                Some(v) if !fits(v, b, s) => Err((
                    sp,
                    format!("the constant {v} does not fit `{}`", ty_name(to)),
                )),
                _ => Ok(Tx {
                    ty: V::Int(b, s),
                    ..t
                }),
            },
            (V::Lit, Ty::Bool) => match t.val {
                Some(v) => Ok(Tx {
                    s: if v != 0 { "true" } else { "false" }.into(),
                    prec: ATOM,
                    ty: V::Bool,
                    val: Some(v),
                }),
                None => Ok(self.truth(t)),
            },
            (V::Bool, Ty::Bool) => Ok(t),
            (V::Bool, Ty::Int(b, s)) => Ok(Tx {
                s: format!("(if {} then 1 else 0)", t.s),
                prec: ATOM,
                ty: V::Int(b, s),
                val: None,
            }),
            (V::Int(..), Ty::Bool) => Ok(self.truth(t)),
            (V::Int(a, sa), Ty::Int(b, sb)) => {
                if (a, sa) == (b, sb) {
                    Ok(t)
                } else if (sa == sb && b > a) || (!sa && sb && b > a) {
                    Ok(Tx {
                        s: format!("({} as {})", par(&t, ATOM), ty_name(to)),
                        prec: ATOM,
                        ty: V::Int(b, sb),
                        val: None,
                    })
                } else {
                    Err((
                        sp,
                        format!(
                            "a conversion from `{}` to `{}` can lose information, and Neuron has no wrapping conversion",
                            ty_name(Ty::Int(a, sa)),
                            ty_name(to)
                        ),
                    ))
                }
            }
        }
    }

    fn truth(&self, t: Tx) -> Tx {
        match t.ty {
            V::Bool => t,
            _ => match t.val {
                Some(v) => Tx {
                    s: if v != 0 { "true" } else { "false" }.into(),
                    prec: ATOM,
                    ty: V::Bool,
                    val: Some(v),
                },
                None => Tx {
                    s: format!("{} != 0", par(&t, BOR)),
                    prec: CMP,
                    ty: V::Bool,
                    val: None,
                },
            },
        }
    }

    fn int_of(&self, t: Tx, sp: Sp) -> CRes<Tx> {
        if t.ty == V::Bool {
            self.coerce(t, Ty::Int(32, true), sp)
        } else {
            Ok(t)
        }
    }

    fn unify(&self, a: Tx, b: Tx, sp: Sp) -> CRes<(Tx, Tx)> {
        match (a.ty, b.ty) {
            (V::Lit, V::Lit) | (V::Bool, _) | (_, V::Bool) => Ok((a, b)),
            (V::Lit, V::Int(bits, s)) => {
                let a = self.coerce(a, Ty::Int(bits, s), sp)?;
                Ok((a, b))
            }
            (V::Int(bits, s), V::Lit) => {
                let b = self.coerce(b, Ty::Int(bits, s), sp)?;
                Ok((a, b))
            }
            (V::Int(ab, asg), V::Int(bb, bsg)) => {
                if (ab, asg) == (bb, bsg) {
                    return Ok((a, b));
                }
                let target = if asg == bsg {
                    Ty::Int(ab.max(bb), asg)
                } else {
                    let (ub, sb) = if asg { (bb, ab) } else { (ab, bb) };
                    if ub < sb {
                        Ty::Int(sb, true)
                    } else {
                        return Err((
                            sp,
                            format!(
                                "the operands mix signed and unsigned integers of {ab} and {bb} bits; use one type"
                            ),
                        ));
                    }
                };
                Ok((self.coerce(a, target, sp)?, self.coerce(b, target, sp)?))
            }
        }
    }

    fn cond(&mut self, e: &Ex) -> CRes<Tx> {
        let t = self.expr(e)?;
        Ok(self.truth(t))
    }

    fn expr(&mut self, e: &Ex) -> CRes<Tx> {
        match e {
            Ex::Int(v, _) => Ok(lit(*v)),
            Ex::Var(n, sp) => {
                if let Some(v) = self.lookup(n) {
                    return Ok(Tx {
                        s: v.n.clone(),
                        prec: ATOM,
                        ty: vty(v.ty),
                        val: None,
                    });
                }
                if let Some((val, ty)) = self.env.consts.get(n) {
                    return Ok(Tx {
                        s: safe_name(n),
                        prec: ATOM,
                        ty: ty.map_or(V::Lit, vty),
                        val: Some(*val),
                    });
                }
                Err((*sp, format!("unknown identifier `{n}`")))
            }
            Ex::Un(op, x, sp) => self.unary(op, x, *sp),
            Ex::Bin(op, a, b, sp) => self.binary(op, a, b, *sp),
            Ex::Tern(c, a, b, sp) => {
                let c = self.cond(c)?;
                let x = self.expr(a)?;
                let y = self.expr(b)?;
                let (x, y) = if x.ty == V::Bool && y.ty == V::Bool {
                    (x, y)
                } else {
                    let x = self.int_of(x, *sp)?;
                    let y = self.int_of(y, *sp)?;
                    self.unify(x, y, *sp)?
                };
                Ok(Tx {
                    s: format!("if {} then {} else {}", c.s, x.s, y.s),
                    prec: COND,
                    ty: x.ty,
                    val: None,
                })
            }
            Ex::Call(name, args, sp) => {
                if *name == self.name {
                    return Err((*sp, "recursion is not allowed in `core` code".into()));
                }
                let Some((ptys, ret)) = self.env.fns.get(name).cloned() else {
                    return Err((
                        *sp,
                        format!("the call to `{name}`, which is not an imported function"),
                    ));
                };
                if args.len() != ptys.len() {
                    return Err((*sp, format!("`{name}` takes {} arguments", ptys.len())));
                }
                let mut v = Vec::new();
                for (a, p) in args.iter().zip(&ptys) {
                    let t = self.expr(a)?;
                    v.push(self.coerce(t, *p, a.sp())?.s);
                }
                self.calls.insert(name.clone());
                Ok(Tx {
                    s: format!("{}({})", safe_name(name), v.join(", ")),
                    prec: ATOM,
                    ty: vty(ret),
                    val: None,
                })
            }
            Ex::Cast(ty, x, sp) => {
                let t = self.expr(x)?;
                self.coerce(t, *ty, *sp)
            }
        }
    }

    fn unary(&mut self, op: &'static str, x: &Ex, sp: Sp) -> CRes<Tx> {
        let t = self.expr(x)?;
        match op {
            "+" => self.int_of(t, sp),
            "!" => {
                let t = self.truth(t);
                Ok(Tx {
                    s: format!("not {}", par(&t, NOT)),
                    prec: NOT,
                    ty: V::Bool,
                    val: None,
                })
            }
            "-" | "~" => {
                let t = self.int_of(t, sp)?;
                if let (V::Lit, Some(v)) = (t.ty, t.val) {
                    return Ok(lit(if op == "-" { -v } else { !v }));
                }
                match (op, t.ty) {
                    ("-", V::Int(_, false)) => Ok(Tx {
                        s: format!("0 -% {}", par(&t, ADD + 1)),
                        prec: ADD,
                        ty: t.ty,
                        val: None,
                    }),
                    _ => Ok(Tx {
                        s: format!("{op}{}", par(&t, UNARY)),
                        prec: UNARY,
                        ty: t.ty,
                        val: None,
                    }),
                }
            }
            _ => Err((sp, format!("the operator `{op}`"))),
        }
    }

    fn binary(&mut self, op: &'static str, a: &Ex, b: &Ex, sp: Sp) -> CRes<Tx> {
        match op {
            "&&" | "||" => {
                let l = self.cond(a)?;
                let r = self.cond(b)?;
                let (word, p) = if op == "&&" { ("and", AND) } else { ("or", OR) };
                Ok(Tx {
                    s: format!("{} {word} {}", par(&l, p), par(&r, p + 1)),
                    prec: p,
                    ty: V::Bool,
                    val: None,
                })
            }
            "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                let l = self.expr(a)?;
                let r = self.expr(b)?;
                let (l, r) = if l.ty == V::Bool && r.ty == V::Bool && (op == "==" || op == "!=") {
                    (l, r)
                } else {
                    let l = self.int_of(l, sp)?;
                    let r = self.int_of(r, sp)?;
                    self.unify(l, r, sp)?
                };
                Ok(Tx {
                    s: format!("{} {op} {}", par(&l, BOR), par(&r, BOR)),
                    prec: CMP,
                    ty: V::Bool,
                    val: None,
                })
            }
            "<<" | ">>" => self.shift(op, a, b, sp),
            _ => {
                let l = self.expr(a)?;
                let l = self.int_of(l, sp)?;
                let r = self.expr(b)?;
                let r = self.int_of(r, sp)?;
                let (l, r) = self.unify(l, r, sp)?;
                if let (V::Lit, Some(x), Some(y)) = (l.ty, l.val, r.val) {
                    let v = match op {
                        "+" => x.checked_add(y),
                        "-" => x.checked_sub(y),
                        "*" => x.checked_mul(y),
                        "/" => x.checked_div(y),
                        "%" => x.checked_rem(y),
                        "&" => Some(x & y),
                        "|" => Some(x | y),
                        _ => Some(x ^ y),
                    };
                    return v
                        .map(lit)
                        .ok_or((sp, "a constant division by zero or overflow".into()));
                }
                let (nop, p) = match op {
                    "+" => ("+%", ADD),
                    "-" => ("-%", ADD),
                    "*" => ("*%", MUL),
                    "/" => ("/", MUL),
                    "%" => ("%", MUL),
                    "&" => ("&", BAND),
                    "|" => ("|", BOR),
                    _ => ("^", BXOR),
                };
                Ok(Tx {
                    s: format!("{} {nop} {}", par(&l, p), par(&r, p + 1)),
                    prec: p,
                    ty: l.ty,
                    val: None,
                })
            }
        }
    }

    fn shift(&mut self, op: &'static str, a: &Ex, b: &Ex, sp: Sp) -> CRes<Tx> {
        let l = self.expr(a)?;
        let l = self.int_of(l, sp)?;
        let r = self.expr(b)?;
        let r = self.int_of(r, sp)?;
        if let (V::Lit, Some(x), Some(y)) = (l.ty, l.val, r.val) {
            if !(0..100).contains(&y) {
                return Err((sp, "a shift count out of range".into()));
            }
            let v = if op == "<<" {
                x.checked_shl(y as u32)
            } else {
                Some(x >> y)
            };
            return v.map(lit).ok_or((sp, "a constant overflow".into()));
        }
        if let Some(y) = r.val {
            if y < 0 {
                return Err((sp, "a negative shift count".into()));
            }
            if let V::Int(bits, _) = l.ty {
                if y >= i128::from(bits) {
                    return Err((sp, "a shift count as wide as the operand".into()));
                }
            }
        }
        if op == "<<" {
            if let V::Int(_, false) = l.ty {
                let m = match r.val {
                    Some(y) => (1i128 << y).to_string(),
                    None => format!("(1 << {})", par(&r, SHIFT + 1)),
                };
                return Ok(Tx {
                    s: format!("{} *% {m}", par(&l, MUL)),
                    prec: MUL,
                    ty: l.ty,
                    val: None,
                });
            }
        }
        Ok(Tx {
            s: format!("{} {op} {}", par(&l, SHIFT), par(&r, SHIFT + 1)),
            prec: SHIFT,
            ty: l.ty,
            val: None,
        })
    }

    fn block(&mut self, body: &[St], depth: usize) -> CRes<()> {
        self.scopes.push(Vec::new());
        let before = self.out.len();
        for s in body {
            self.stmt(s, depth)?;
        }
        if self.out.len() == before {
            self.put(depth, "pass");
        }
        self.scopes.pop();
        Ok(())
    }

    /// `for v in a..b` when the loop has constant bounds and its body leaves `v` alone.
    fn const_range(
        &self,
        init: &Option<Box<St>>,
        cond: &Option<Ex>,
        step: &Option<Box<St>>,
        body: &[St],
    ) -> Option<(String, Ty, i128, i128, bool)> {
        let Some(St::Decl {
            ty: ty @ Ty::Int(..),
            name,
            init: Some(e),
            ..
        }) = init.as_deref()
        else {
            return None;
        };
        let a = fold(e, &self.env.consts)?;
        let Some(Ex::Bin(op, l, r, _)) = cond else {
            return None;
        };
        if !matches!(&**l, Ex::Var(n, _) if n == name) || !matches!(*op, "<" | "<=") {
            return None;
        }
        let b = fold(r, &self.env.consts)?;
        let stepped = match step.as_deref() {
            Some(St::IncDec {
                name: n,
                plus: true,
                ..
            }) => n == name,
            Some(St::Assign {
                name: n,
                op: "+",
                rhs: Ex::Int(1, _),
                ..
            }) => n == name,
            _ => false,
        };
        if !stepped {
            return None;
        }
        let mut assigned = BTreeSet::new();
        assigned_in(body, &mut assigned);
        if assigned.contains(name) {
            return None;
        }
        if let Ty::Int(bits, s) = ty {
            if !fits(a, *bits, *s) || !fits(b, *bits, *s) {
                return None;
            }
        }
        Some((name.clone(), *ty, a, b, *op == "<="))
    }

    fn stmt(&mut self, s: &St, depth: usize) -> CRes<()> {
        match s {
            St::Decl { ty, name, init, sp } => {
                let mutable = self.assigned.contains(name);
                let val = match init {
                    Some(e) => {
                        let t = self.expr(e)?;
                        self.coerce(t, *ty, *sp)?.s
                    }
                    None if *ty == Ty::Bool => "false".to_string(),
                    None => "0".to_string(),
                };
                let n = self.fresh(name);
                self.put(
                    depth,
                    format!(
                        "{} {n}: {} = {val}",
                        if mutable { "var" } else { "let" },
                        ty_name(*ty)
                    ),
                );
                if let Some(frame) = self.scopes.last_mut() {
                    frame.push(Var {
                        c: name.clone(),
                        n,
                        ty: *ty,
                    });
                }
                Ok(())
            }
            St::Assign { name, op, rhs, sp } => {
                let Some(v) = self.lookup(name) else {
                    return Err((
                        *sp,
                        format!("the assignment to `{name}`, which is not a local variable"),
                    ));
                };
                let (n, ty) = (v.n.clone(), v.ty);
                let t = if *op == "=" {
                    self.expr(rhs)?
                } else {
                    let e = Ex::Bin(
                        op,
                        Box::new(Ex::Var(name.clone(), *sp)),
                        Box::new(rhs.clone()),
                        *sp,
                    );
                    self.expr(&e)?
                };
                let t = self.coerce(t, ty, *sp)?;
                self.put(depth, format!("{n} = {}", t.s));
                Ok(())
            }
            St::IncDec { name, plus, sp } => {
                let rhs = Ex::Int(1, *sp);
                let st = St::Assign {
                    name: name.clone(),
                    op: if *plus { "+" } else { "-" },
                    rhs,
                    sp: *sp,
                };
                self.stmt(&st, depth)
            }
            St::If { cond, then, els } => {
                let c = self.cond(cond)?;
                self.put(depth, format!("if {}:", c.s));
                self.block(then, depth + 1)?;
                let mut rest: &[St] = els;
                loop {
                    match rest {
                        [] => break,
                        [St::If { cond, then, els }] => {
                            let c = self.cond(cond)?;
                            self.put(depth, format!("elif {}:", c.s));
                            self.block(then, depth + 1)?;
                            rest = els;
                        }
                        _ => {
                            self.put(depth, "else:");
                            self.block(rest, depth + 1)?;
                            break;
                        }
                    }
                }
                Ok(())
            }
            St::While {
                cond,
                body,
                bound,
                sp,
            } => {
                let Some(n) = bound else {
                    return Err((
                        *sp,
                        "an unbounded `while`: add a bound comment such as `/* max 100 */`".into(),
                    ));
                };
                let c = self.cond(cond)?;
                self.put(depth, format!("while {} max {n}:", c.s));
                self.block(body, depth + 1)
            }
            St::For {
                init,
                cond,
                step,
                body,
                bound,
                sp,
            } => {
                // A `for` becomes a `while` with its own counter: Neuron's `for v in a..b`
                // gives `v` the type `index`, which does not mix with the C integer types.
                // A constant range has its trip count as the bound.
                let range = self.const_range(init, cond, step, body);
                let (trips, ranged) = match (&range, bound) {
                    (Some((name, _, a, b, incl)), _) => {
                        let n = (b - a + i128::from(*incl)).max(1);
                        (u64::try_from(n).unwrap_or(u64::MAX), Some(name.clone()))
                    }
                    (None, Some(n)) => (*n, None),
                    (None, None) => {
                        return Err((
                            *sp,
                            "a `for` loop without constant bounds: add a bound comment such as `/* max 100 */`"
                                .into(),
                        ));
                    }
                };
                let Some(cond) = cond else {
                    return Err((*sp, "a `for` loop without a condition".into()));
                };
                let continues = has_continue(body);
                if continues && ranged.is_none() {
                    return Err((
                        *sp,
                        "`continue` in a `for` loop that is not a constant range".into(),
                    ));
                }
                if let Some(i) = init {
                    self.stmt(i, depth)?;
                }
                let c = self.cond(cond)?;
                self.put(depth, format!("while {} max {trips}:", c.s));
                self.scopes.push(Vec::new());
                let before = self.out.len();
                // With a `continue` in the body the step runs first and the body reads a copy
                // of the loop variable (which the body never assigns).
                let mut step_last = true;
                if let (true, Some(name)) = (continues, &ranged) {
                    let Some((n, ty)) = self.lookup(name).map(|v| (v.n.clone(), v.ty)) else {
                        return Err((*sp, "the loop variable is not declared".into()));
                    };
                    let copy = self.fresh(&format!("{name}_c"));
                    self.put(depth + 1, format!("let {copy}: {} = {n}", ty_name(ty)));
                    if let Some(st) = step {
                        self.stmt(st, depth + 1)?;
                    }
                    step_last = false;
                    if let Some(frame) = self.scopes.last_mut() {
                        frame.push(Var {
                            c: name.clone(),
                            n: copy,
                            ty,
                        });
                    }
                }
                for st in body {
                    self.stmt(st, depth + 1)?;
                }
                if step_last {
                    if let Some(st) = step {
                        self.stmt(st, depth + 1)?;
                    }
                }
                if self.out.len() == before {
                    self.put(depth + 1, "pass");
                }
                self.scopes.pop();
                Ok(())
            }
            St::Return(e, sp) => {
                let Some(e) = e else {
                    return Err((*sp, "a `return` without a value".into()));
                };
                let t = self.expr(e)?;
                let t = self.coerce(t, self.ret, *sp)?;
                self.put(depth, format!("return {}", t.s));
                Ok(())
            }
            St::Break => {
                self.put(depth, "break");
                Ok(())
            }
            St::Continue => {
                self.put(depth, "continue");
                Ok(())
            }
        }
    }
}

type Converted = (String, BTreeSet<String>);

fn convert_fn(env: &Sigs, f: &Func, body: &[St]) -> CRes<Converted> {
    if f.ret == Ty::Void {
        return Err((
            f.sp,
            "a `void` function has no observable effect without pointers or globals".into(),
        ));
    }
    let mut fx = Fx {
        env,
        name: f.name.clone(),
        ret: f.ret,
        scopes: vec![Vec::new()],
        assigned: BTreeSet::new(),
        out: Vec::new(),
        calls: BTreeSet::new(),
    };
    assigned_in(body, &mut fx.assigned);
    let mut params = Vec::new();
    let mut prologue = Vec::new();
    for (ty, c) in &f.params {
        let pn = safe_name(c);
        params.push(format!("{pn}: {}", ty_name(*ty)));
        let n = if fx.assigned.contains(c) {
            let n = fx.fresh(&format!("{c}_v"));
            prologue.push(format!("  var {n}: {} = {pn}", ty_name(*ty)));
            n
        } else {
            pn
        };
        fx.scopes[0].push(Var {
            c: c.clone(),
            n,
            ty: *ty,
        });
    }
    let before = fx.out.len();
    for s in body {
        fx.stmt(s, 1)?;
    }
    if fx.out.len() == before && prologue.is_empty() {
        fx.put(1, "pass");
    }
    let mut text = format!(
        "{}fn {}({}) -> {}:\n",
        if f.is_static { "" } else { "pub " },
        safe_name(&f.name),
        params.join(", "),
        ty_name(f.ret)
    );
    for l in prologue.iter().chain(fx.out.iter()) {
        text.push_str(l);
        text.push('\n');
    }
    Ok((text, fx.calls))
}

fn reaches(start: &str, graph: &BTreeMap<&String, &BTreeSet<String>>) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut stack: Vec<&str> = graph
        .get(&start.to_string())
        .map(|c| c.iter().map(String::as_str).collect())
        .unwrap_or_default();
    while let Some(n) = stack.pop() {
        if n == start {
            return true;
        }
        if seen.insert(n) {
            if let Some(c) = graph.get(&n.to_string()) {
                stack.extend(c.iter().map(String::as_str));
            }
        }
    }
    false
}

fn unsup(fid: &mut Fidelity, diags: &mut Vec<Diag>, file: &str, sp: Sp, what: &str) {
    diags.push(Diag {
        code: "NE1002",
        message: format!("not imported: {what}"),
        help: Some(
            "write that part in Neuron by hand; the rest of the file was imported".to_string(),
        ),
        file: file.to_string(),
        span: sp,
    });
    fid.record(
        file,
        sp,
        Class::Unsupported,
        what,
        "not imported: it has no Neuron equivalent in the C subset",
    );
}

/// Imports C source as a `core` module called `name`.
pub(crate) fn import(file: &str, name: &str, text: &str) -> Result<Out, Vec<Diag>> {
    let lexed = lex(text).map_err(|(p, l, m)| vec![syntax(file, p, l, &m)])?;
    let mut p = P {
        t: &lexed.toks,
        i: 0,
        end: text.len(),
        bounds: &lexed.bounds,
        aliases: BTreeMap::new(),
        consts: BTreeMap::new(),
        items: Vec::new(),
        unsup: Vec::new(),
        notes: Vec::new(),
    };
    if let Err(e) = p.top() {
        let (PErr::Syntax(sp, m) | PErr::Unsup(sp, m)) = e;
        return Err(vec![syntax(file, sp[0], sp[1] - sp[0], &m)]);
    }
    let mut fid = Fidelity::new("import-c", "neuron.core");
    let mut diags: Vec<Diag> = Vec::new();
    for (sp, m) in &p.unsup {
        unsup(&mut fid, &mut diags, file, *sp, m);
    }
    for (sp, c, n) in &p.notes {
        fid.record(file, *sp, Class::Equivalent, c, n);
    }
    let module = module_name(name, &mut fid, file, [0, 0]);

    let defs: Vec<&Func> = p
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Fn(f) if matches!(f.body, Some(Ok(_))) => Some(f),
            _ => None,
        })
        .collect();
    let mut failed: BTreeMap<String, (Sp, String)> = BTreeMap::new();
    for i in &p.items {
        if let Item::Fn(f) = i {
            if let Some(Err((sp, m))) = &f.body {
                failed.insert(f.name.clone(), (*sp, m.clone()));
            }
        }
    }
    let reserved: BTreeSet<String> = p
        .items
        .iter()
        .map(|i| match i {
            Item::Fn(f) => safe_name(&f.name),
            Item::Const(c) => safe_name(&c.name),
        })
        .collect();
    let results = loop {
        let sigs = Sigs {
            fns: defs
                .iter()
                .filter(|f| f.ret != Ty::Void && !failed.contains_key(&f.name))
                .map(|f| {
                    (
                        f.name.clone(),
                        (f.params.iter().map(|x| x.0).collect(), f.ret),
                    )
                })
                .collect(),
            consts: p.consts.clone(),
            reserved: reserved.clone(),
        };
        let mut res: BTreeMap<String, CRes<Converted>> = BTreeMap::new();
        for f in &defs {
            if let Some(Ok(body)) = &f.body {
                res.insert(f.name.clone(), convert_fn(&sigs, f, body));
            }
        }
        let mut changed = false;
        for (n, r) in &res {
            if let Err(e) = r {
                if !failed.contains_key(n) {
                    failed.insert(n.clone(), e.clone());
                    changed = true;
                }
            }
        }
        let graph: BTreeMap<&String, &BTreeSet<String>> = res
            .iter()
            .filter(|(n, _)| !failed.contains_key(*n))
            .filter_map(|(n, r)| r.as_ref().ok().map(|(_, c)| (n, c)))
            .collect();
        let cyclic: Vec<String> = graph
            .keys()
            .filter(|n| reaches(n, &graph))
            .map(|n| (*n).clone())
            .collect();
        for n in cyclic {
            let sp = defs.iter().find(|f| f.name == n).map_or([0, 1], |f| f.sp);
            failed.insert(
                n,
                (sp, "recursion is not allowed in `core` code".to_string()),
            );
            changed = true;
        }
        if !changed {
            break res;
        }
    };

    let mut src = format!("## Imported from C by `neuron import`.\nmodule {module} for core\n");
    let mut any_fn = false;
    for item in &p.items {
        match item {
            Item::Const(c) => {
                src.push('\n');
                match c.ty {
                    Some(t) => src.push_str(&format!(
                        "const {}: {} = {}\n",
                        safe_name(&c.name),
                        ty_name(t),
                        c.value
                    )),
                    None => src.push_str(&format!("const {} = {}\n", safe_name(&c.name), c.value)),
                }
                fid.exact();
            }
            Item::Fn(f) => {
                if matches!(f.body, Some(Ok(_))) {
                    if let Some((sp, m)) = failed.get(&f.name) {
                        unsup(
                            &mut fid,
                            &mut diags,
                            file,
                            *sp,
                            &format!("function `{}`: {m}", f.name),
                        );
                    } else if let Some(Ok((text, _))) = results.get(&f.name) {
                        src.push('\n');
                        src.push_str(text);
                        fid.exact();
                        any_fn = true;
                    }
                } else if let (Some(Err(_)), Some((sp, m))) = (&f.body, failed.get(&f.name)) {
                    unsup(
                        &mut fid,
                        &mut diags,
                        file,
                        *sp,
                        &format!("function `{}`: {m}", f.name),
                    );
                }
            }
        }
    }
    if any_fn {
        fid.record(
            file,
            [0, 0],
            Class::Equivalent,
            "integer arithmetic",
            "`+`, `-` and `*` wrap (`+%`, `-%`, `*%`) and `<<` on an unsigned type multiplies with wrap-around, like C unsigned arithmetic; signed overflow is undefined in C. Operands narrower than `int` are not promoted: arithmetic on `u8` or `i16` wraps at its own width, where C computes in `int` and wraps on the store",
        );
    }
    finish(src, fid, diags, file)
}
