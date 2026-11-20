//! A Verilog subset to an `hdl` module (task C5.3).
//!
//! The subset is what `neuron emit verilog` writes plus hand-written modules of the same
//! kind: `input`/`output`/`output reg` ports (ANSI or old style) with widths, `parameter` and
//! `localparam`, `reg` and `wire` (also `wire x = e;`), `assign`, `always @(posedge clk)` with
//! non-blocking assignments (optionally with a synchronous `if (rst)` reset), complete
//! combinational `always @*` blocks, `if`/`else`, `case` and sized literals. Each module becomes
//! a `circuit`; the clock is the signal of the `posedge` (or `negedge`) event.
//!
//! Not imported (`NE1002`, the rest of the file is): latch-inferring combinational blocks,
//! blocking assignments in clocked blocks, non-blocking assignments in combinational blocks,
//! several clocks, asynchronous resets, `initial`, tasks, functions, generate blocks, module
//! instances, memories, `integer`, partial assignments and arithmetic whose carry is lost.

use crate::st::{finish, Out};
use crate::util::{ident, module_name, syntax};
use neuron_emit::{Class, Diag, Fidelity};
use std::collections::{BTreeMap, BTreeSet};

type Sp = [usize; 2];

// ---------------------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Num {
    /// Neuron text: a sized literal (`8'hff`) or a decimal.
    text: String,
    val: Option<i128>,
    w: Option<u32>,
    bad: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
enum K {
    Id(String),
    Num(Num),
    Str,
    Sys(String),
    P(&'static str),
    Directive(String),
    Escaped,
}

#[derive(Debug, Clone)]
struct Tok {
    k: K,
    pos: usize,
    end: usize,
}

const PUNCT: &[&str] = &[
    "<<<", ">>>", "===", "!==", "~^", "^~", "~&", "~|", "<<", ">>", "<=", ">=", "==", "!=", "&&",
    "||", "**", "+:", "-:", "->", "+", "-", "*", "/", "%", "&", "|", "^", "~", "!", "<", ">", "=",
    "?", ":", ";", ",", ".", "(", ")", "[", "]", "{", "}", "@", "#",
];

fn lex(text: &str) -> Result<Vec<Tok>, (usize, usize, String)> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut toks: Vec<Tok> = Vec::new();
    let base_at = |j: usize| -> bool {
        let mut k = j + 1;
        if matches!(b.get(k), Some(b's' | b'S')) {
            k += 1;
        }
        matches!(
            b.get(k),
            Some(b'b' | b'B' | b'o' | b'O' | b'd' | b'D' | b'h' | b'H')
        )
    };
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
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
            continue;
        }
        let start = i;
        if c == b'`' {
            i += 1;
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let name = text[s..i].to_string();
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            if !matches!(
                name.as_str(),
                "timescale" | "default_nettype" | "resetall" | "celldefine" | "endcelldefine"
            ) {
                toks.push(Tok {
                    k: K::Directive(name),
                    pos: start,
                    end: i,
                });
            }
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
        if c == b'\\' {
            while i < b.len() && !b[i].is_ascii_whitespace() {
                i += 1;
            }
            toks.push(Tok {
                k: K::Escaped,
                pos: start,
                end: i,
            });
            continue;
        }
        if c == b'$' {
            i += 1;
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            toks.push(Tok {
                k: K::Sys(text[s..i].to_string()),
                pos: start,
                end: i,
            });
            continue;
        }
        if c.is_ascii_digit() || (c == b'\'' && base_at(i)) {
            let mut size = String::new();
            if c.is_ascii_digit() {
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
                    if b[i] != b'_' {
                        size.push(b[i] as char);
                    }
                    i += 1;
                }
            }
            let mut j = i;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            let num = if j < b.len() && b[j] == b'\'' && base_at(j) {
                i = j + 1;
                let signed = matches!(b[i], b's' | b'S');
                if signed {
                    i += 1;
                }
                let base = (b[i] | 0x20) as char;
                i += 1;
                let ds = i;
                while i < b.len()
                    && (b[i].is_ascii_hexdigit()
                        || matches!(b[i], b'_' | b'x' | b'X' | b'z' | b'Z' | b'?'))
                {
                    i += 1;
                }
                let digits = &text[ds..i];
                let clean: String = digits.chars().filter(|c| *c != '_').collect();
                let w = if size.is_empty() {
                    None
                } else {
                    size.parse::<u32>().ok()
                };
                let radix = match base {
                    'b' => 2,
                    'o' => 8,
                    'd' => 10,
                    _ => 16,
                };
                let mut bad = None;
                let mut val = None;
                if signed {
                    bad = Some("signed literals");
                } else if clean.is_empty()
                    || clean
                        .chars()
                        .any(|c| matches!(c, 'x' | 'X' | 'z' | 'Z' | '?'))
                {
                    bad = Some("literals with x or z digits");
                } else {
                    val = i128::from_str_radix(&clean, radix).ok();
                    if val.is_none() {
                        bad = Some("literals this large");
                    }
                }
                if !size.is_empty() && w.is_none_or(|w| w == 0) {
                    bad = Some("a literal of width 0");
                }
                let text = match (w, val) {
                    (Some(w), Some(_)) => format!("{w}'{base}{digits}"),
                    (None, Some(v)) => v.to_string(),
                    _ => digits.to_string(),
                };
                Num { text, val, w, bad }
            } else {
                let mut bad = None;
                if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
                    bad = Some("real numbers");
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let val = size.parse::<i128>().ok();
                Num {
                    text: size,
                    val,
                    w: None,
                    bad,
                }
            };
            toks.push(Tok {
                k: K::Num(num),
                pos: start,
                end: i,
            });
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$') {
                i += 1;
            }
            toks.push(Tok {
                k: K::Id(text[start..i].to_string()),
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
            None => return Err((start, 1, format!("unexpected character `{}`", c as char))),
        }
    }
    Ok(toks)
}

// ---------------------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum VEx {
    Num(Num, Sp),
    Id(String, Sp),
    Un(&'static str, Box<VEx>, Sp),
    Bin(&'static str, Box<VEx>, Box<VEx>, Sp),
    Tern(Box<VEx>, Box<VEx>, Box<VEx>, Sp),
    Concat(Vec<VEx>, Sp),
    Repl(Box<VEx>, Vec<VEx>, Sp),
    Index(Box<VEx>, Box<VEx>, Sp),
    Slice(Box<VEx>, Box<VEx>, Box<VEx>, Sp),
    Sys(String, Vec<VEx>, Sp),
}

impl VEx {
    fn sp(&self) -> Sp {
        match self {
            VEx::Num(_, s)
            | VEx::Id(_, s)
            | VEx::Un(_, _, s)
            | VEx::Bin(_, _, _, s)
            | VEx::Tern(_, _, _, s)
            | VEx::Concat(_, s)
            | VEx::Repl(_, _, s)
            | VEx::Index(_, _, s)
            | VEx::Slice(_, _, _, s)
            | VEx::Sys(_, _, s) => *s,
        }
    }
}

type Params = BTreeMap<String, i128>;

fn clog2(v: i128) -> i128 {
    let mut n = 0;
    while (1i128 << n) < v {
        n += 1;
    }
    n
}

/// The value of a constant expression over the parameters.
fn vfold(e: &VEx, env: &Params) -> Option<i128> {
    match e {
        VEx::Num(n, _) => n.val,
        VEx::Id(n, _) => env.get(n).copied(),
        VEx::Un(op, x, _) => {
            let v = vfold(x, env)?;
            match *op {
                "-" => v.checked_neg(),
                "+" => Some(v),
                "!" => Some(i128::from(v == 0)),
                _ => None,
            }
        }
        VEx::Bin(op, a, b, _) => {
            let (x, y) = (vfold(a, env)?, vfold(b, env)?);
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
                "**" => u32::try_from(y).ok().and_then(|s| x.checked_pow(s)),
                "&" => Some(x & y),
                "|" => Some(x | y),
                "^" => Some(x ^ y),
                _ => None,
            }
        }
        VEx::Tern(c, a, b, _) => {
            if vfold(c, env)? != 0 {
                vfold(a, env)
            } else {
                vfold(b, env)
            }
        }
        VEx::Sys(n, args, _) if n == "clog2" && args.len() == 1 => vfold(&args[0], env).map(clog2),
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum VSt {
    Assign {
        lhs: String,
        rhs: VEx,
        sp: Sp,
    },
    If {
        cond: VEx,
        then: Vec<VSt>,
        els: Vec<VSt>,
    },
    Case {
        sel: VEx,
        arms: Vec<(Vec<VEx>, Vec<VSt>)>,
        default: Option<Vec<VSt>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    In,
    Out,
    Inout,
}

#[derive(Debug, Clone)]
struct Port {
    name: String,
    dir: Dir,
    w: u32,
    signed: bool,
    reg: bool,
    init: Option<VEx>,
    sp: Sp,
}

#[derive(Debug, Clone)]
struct Net {
    name: String,
    w: u32,
    signed: bool,
    is_reg: bool,
    init: Option<VEx>,
    sp: Sp,
}

#[derive(Debug, Clone)]
enum Blk {
    Clocked {
        clk: String,
        rising: bool,
        body: Vec<VSt>,
        sp: Sp,
    },
    Comb {
        body: Vec<VSt>,
        assign: bool,
        sp: Sp,
    },
}

#[derive(Debug, Default)]
struct Module {
    name: String,
    name_sp: Sp,
    ports: Vec<Port>,
    port_order: Vec<String>,
    ansi: bool,
    nets: Vec<Net>,
    params: Vec<(String, i128)>,
    blocks: Vec<Blk>,
    unsup: Vec<(Sp, String)>,
    notes: Vec<(Sp, &'static str, String)>,
}

// ---------------------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------------------

enum PErr {
    Syntax(Sp, String),
    Unsup(Sp, String),
}

type PRes<T> = Result<T, PErr>;

struct P<'a> {
    t: &'a [Tok],
    i: usize,
    end: usize,
    env: Params,
}

fn bin_prec(op: &str) -> Option<u8> {
    Some(match op {
        "||" => 1,
        "&&" => 2,
        "|" => 3,
        "^" | "~^" | "^~" => 4,
        "&" => 5,
        "==" | "!=" | "===" | "!==" => 6,
        "<" | "<=" | ">" | ">=" => 7,
        "<<" | ">>" | "<<<" | ">>>" => 8,
        "+" | "-" => 9,
        "*" | "/" | "%" => 10,
        "**" => 11,
        _ => return None,
    })
}

impl P<'_> {
    fn tok(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }

    fn sp(&self) -> Sp {
        self.t
            .get(self.i)
            .map_or([self.end, self.end + 1], |t| [t.pos, t.end.max(t.pos + 1)])
    }

    fn nth_p(&self, n: usize, s: &str) -> bool {
        matches!(self.t.get(self.i + n), Some(Tok { k: K::P(x), .. }) if *x == s)
    }

    fn at_p(&self, s: &str) -> bool {
        self.nth_p(0, s)
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

    fn eat_id(&mut self, s: &str) -> bool {
        if self.at_id(s) {
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
            Some(Tok { k: K::Escaped, .. }) => {
                Err(PErr::Unsup(self.sp(), "escaped identifiers".into()))
            }
            _ => Err(PErr::Syntax(self.sp(), "expected a name".into())),
        }
    }

    /// `[hi:lo]` as a width (1 when there is no range).
    fn range(&mut self) -> PRes<u32> {
        if !self.at_p("[") {
            return Ok(1);
        }
        let sp = self.sp();
        self.i += 1;
        let hi = self.expr()?;
        self.expect(":")?;
        let lo = self.expr()?;
        self.expect("]")?;
        let (Some(h), Some(l)) = (vfold(&hi, &self.env), vfold(&lo, &self.env)) else {
            return Err(PErr::Unsup(sp, "a range that is not constant".into()));
        };
        if l != 0 || h < l {
            return Err(PErr::Unsup(
                sp,
                "a range that does not run from a bit index down to 0".into(),
            ));
        }
        if h + 1 > 64 {
            return Err(PErr::Unsup(sp, "a vector wider than 64 bits".into()));
        }
        Ok((h + 1) as u32)
    }

    fn skip_semi(&mut self) {
        while let Some(t) = self.tok() {
            let semi = matches!(t.k, K::P(";"));
            self.i += 1;
            if semi {
                break;
            }
        }
    }

    fn skip_until_id(&mut self, word: &str) {
        while let Some(t) = self.tok() {
            let hit = matches!(&t.k, K::Id(x) if x == word);
            self.i += 1;
            if hit {
                break;
            }
        }
    }

    /// Skips a parenthesized group when one starts here.
    fn skip_parens(&mut self) {
        if !self.at_p("(") {
            return;
        }
        let mut depth = 0i32;
        while let Some(t) = self.tok() {
            if let K::P(p) = &t.k {
                if *p == "(" {
                    depth += 1;
                } else if *p == ")" {
                    depth -= 1;
                }
            }
            self.i += 1;
            if depth == 0 {
                break;
            }
        }
    }

    /// Skips one statement.
    fn skip_stmt(&mut self) {
        match self.tok().map(|t| t.k.clone()) {
            Some(K::Id(w)) if w == "begin" => {
                let mut depth = 0i32;
                while let Some(t) = self.tok() {
                    if let K::Id(x) = &t.k {
                        if x == "begin" {
                            depth += 1;
                        } else if x == "end" {
                            depth -= 1;
                        }
                    }
                    self.i += 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            Some(K::Id(w)) if w == "if" => {
                self.i += 1;
                self.skip_parens();
                self.skip_stmt();
                if self.eat_id("else") {
                    self.skip_stmt();
                }
            }
            Some(K::Id(w)) if matches!(w.as_str(), "case" | "casex" | "casez") => {
                self.skip_until_id("endcase");
            }
            Some(K::Id(w)) if matches!(w.as_str(), "for" | "while" | "repeat" | "forever") => {
                self.i += 1;
                self.skip_parens();
                self.skip_stmt();
            }
            Some(K::P("#")) => {
                self.i += 2;
                self.skip_stmt();
            }
            Some(_) => self.skip_semi(),
            None => {}
        }
    }

    fn skip_item(&mut self, start: usize) {
        self.i = start;
        match self.tok().map(|t| t.k.clone()) {
            Some(K::Id(w)) => match w.as_str() {
                "always" | "initial" => {
                    self.i += 1;
                    if self.eat("@") && !self.eat("*") {
                        self.skip_parens();
                    }
                    self.skip_stmt();
                }
                "task" => self.skip_until_id("endtask"),
                "function" => self.skip_until_id("endfunction"),
                "generate" => self.skip_until_id("endgenerate"),
                "specify" => self.skip_until_id("endspecify"),
                "primitive" => self.skip_until_id("endprimitive"),
                _ => self.skip_semi(),
            },
            Some(K::P("(")) => {
                // An attribute `(* ... *)`.
                while let Some(t) = self.tok() {
                    let close = matches!(t.k, K::P("*")) && self.nth_p(1, ")");
                    self.i += 1;
                    if close {
                        self.i += 1;
                        break;
                    }
                }
            }
            _ => self.skip_semi(),
        }
        if self.i == start {
            self.i += 1;
        }
    }

    fn module(&mut self) -> PRes<Module> {
        self.i += 1;
        let name_sp = self.sp();
        let name = self.id()?;
        let mut m = Module {
            name,
            name_sp,
            ..Module::default()
        };
        if self.eat("#") {
            self.expect("(")?;
            loop {
                if self.at_p(")") {
                    break;
                }
                self.eat_id("parameter");
                self.eat_id("integer");
                self.eat_id("signed");
                if self.at_p("[") {
                    self.range()?;
                }
                let pn = self.id()?;
                self.expect("=")?;
                let e = self.expr()?;
                match vfold(&e, &self.env) {
                    Some(v) => {
                        self.env.insert(pn.clone(), v);
                        m.params.push((pn, v));
                    }
                    None => m.unsup.push((
                        e.sp(),
                        format!("the parameter `{pn}`, which is not constant"),
                    )),
                }
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(")")?;
        }
        if self.eat("(") {
            if !self.at_p(")") {
                if matches!(self.tok(), Some(Tok { k: K::Id(w), .. }) if matches!(w.as_str(), "input" | "output" | "inout"))
                {
                    m.ansi = true;
                    self.ansi_ports(&mut m)?;
                } else {
                    loop {
                        let n = self.id()?;
                        m.port_order.push(n);
                        if !self.eat(",") {
                            break;
                        }
                    }
                }
            }
            self.expect(")")?;
        }
        self.expect(";")?;
        loop {
            let Some(tok) = self.tok() else {
                return Err(PErr::Syntax(
                    self.sp(),
                    format!("the module `{}` has no `endmodule`", m.name),
                ));
            };
            if matches!(&tok.k, K::Id(w) if w == "endmodule") {
                self.i += 1;
                break;
            }
            let start = self.i;
            match self.item(&mut m) {
                Ok(()) => {}
                Err(PErr::Unsup(sp, msg)) => {
                    m.unsup.push((sp, msg));
                    self.skip_item(start);
                }
                Err(e) => return Err(e),
            }
            if self.i == start {
                self.i += 1;
            }
        }
        self.finish_module(&mut m)?;
        Ok(m)
    }

    fn finish_module(&mut self, m: &mut Module) -> PRes<()> {
        // A `reg` that redeclares a port makes the port a register.
        let nets = std::mem::take(&mut m.nets);
        for n in nets {
            if let Some(p) = m.ports.iter_mut().find(|p| p.name == n.name) {
                p.reg |= n.is_reg;
                if p.w == 1 {
                    p.w = n.w;
                }
                p.signed |= n.signed;
                if n.init.is_some() {
                    p.init = n.init;
                }
            } else {
                m.nets.push(n);
            }
        }
        if !m.ansi {
            let mut ordered = Vec::new();
            for n in &m.port_order {
                match m.ports.iter().find(|p| &p.name == n) {
                    Some(p) => ordered.push(p.clone()),
                    None => {
                        return Err(PErr::Syntax(
                            m.name_sp,
                            format!(
                                "the port `{n}` has no `input`, `output` or `inout` declaration"
                            ),
                        ))
                    }
                }
            }
            m.ports = ordered;
        }
        Ok(())
    }

    fn ansi_ports(&mut self, m: &mut Module) -> PRes<()> {
        let mut cur: Option<(Dir, bool, bool, u32)> = None;
        loop {
            if let Some(Tok { k: K::Id(w), .. }) = self.tok() {
                let dir = match w.as_str() {
                    "input" => Some(Dir::In),
                    "output" => Some(Dir::Out),
                    "inout" => Some(Dir::Inout),
                    _ => None,
                };
                if let Some(dir) = dir {
                    self.i += 1;
                    let (mut reg, mut signed) = (false, false);
                    loop {
                        if self.eat_id("reg") {
                            reg = true;
                        } else if self.eat_id("signed") {
                            signed = true;
                        } else if self.at_id("logic") || self.at_id("var") {
                            return Err(PErr::Unsup(self.sp(), "SystemVerilog types".into()));
                        } else if !self.eat_id("wire") {
                            break;
                        }
                    }
                    let w = self.range()?;
                    cur = Some((dir, reg, signed, w));
                }
            }
            let Some((dir, reg, signed, w)) = cur else {
                return Err(PErr::Syntax(self.sp(), "expected a port direction".into()));
            };
            let sp = self.sp();
            let name = self.id()?;
            let init = if reg && self.eat("=") {
                Some(self.expr()?)
            } else {
                None
            };
            m.ports.push(Port {
                name,
                dir,
                w,
                signed,
                reg,
                init,
                sp,
            });
            if !self.eat(",") {
                break;
            }
        }
        Ok(())
    }

    fn item(&mut self, m: &mut Module) -> PRes<()> {
        let sp = self.sp();
        let Some(tok) = self.tok() else {
            return Ok(());
        };
        match tok.k.clone() {
            K::Directive(d) => Err(PErr::Unsup(sp, format!("the compiler directive `` `{d}``"))),
            K::P(";") => {
                self.i += 1;
                Ok(())
            }
            K::P("(") if self.nth_p(1, "*") => {
                // An attribute `(* ... *)` on the next declaration is ignored.
                while let Some(t) = self.tok() {
                    let close = matches!(t.k, K::P("*")) && self.nth_p(1, ")");
                    self.i += 1;
                    if close {
                        self.i += 1;
                        break;
                    }
                }
                Ok(())
            }
            K::Id(w) => match w.as_str() {
                "input" | "output" | "inout" => self.port_decl(m),
                "wire" | "reg" => self.net_decl(m),
                "parameter" | "localparam" => self.param_decl(m),
                "assign" => self.assign(m),
                "always" => self.always(m),
                "initial" => Err(PErr::Unsup(sp, "`initial` blocks".into())),
                "task" | "function" => Err(PErr::Unsup(sp, format!("`{w}`s"))),
                "generate" | "genvar" => Err(PErr::Unsup(sp, "generate blocks".into())),
                "integer" | "real" | "time" | "realtime" => {
                    Err(PErr::Unsup(sp, format!("`{w}` variables")))
                }
                "defparam" | "specify" | "primitive" => Err(PErr::Unsup(sp, format!("`{w}`"))),
                _ => Err(PErr::Unsup(sp, "module instances and primitives".into())),
            },
            _ => Err(PErr::Syntax(sp, "expected a module item".into())),
        }
    }

    fn port_decl(&mut self, m: &mut Module) -> PRes<()> {
        let dir = match self.id()?.as_str() {
            "input" => Dir::In,
            "output" => Dir::Out,
            _ => Dir::Inout,
        };
        let (mut reg, mut signed) = (false, false);
        loop {
            if self.eat_id("reg") {
                reg = true;
            } else if self.eat_id("signed") {
                signed = true;
            } else if !self.eat_id("wire") {
                break;
            }
        }
        let w = self.range()?;
        loop {
            let sp = self.sp();
            let name = self.id()?;
            let init = if reg && self.eat("=") {
                Some(self.expr()?)
            } else {
                None
            };
            m.ports.push(Port {
                name,
                dir,
                w,
                signed,
                reg,
                init,
                sp,
            });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(";")
    }

    fn net_decl(&mut self, m: &mut Module) -> PRes<()> {
        let is_reg = self.id()? == "reg";
        let signed = self.eat_id("signed");
        let w = self.range()?;
        loop {
            let sp = self.sp();
            let name = self.id()?;
            if self.at_p("[") {
                return Err(PErr::Unsup(self.sp(), format!("the memory `{name}`")));
            }
            let mut init = None;
            if self.eat("=") {
                let e = self.expr()?;
                if is_reg {
                    init = Some(e);
                } else {
                    m.blocks.push(Blk::Comb {
                        body: vec![VSt::Assign {
                            lhs: name.clone(),
                            rhs: e,
                            sp,
                        }],
                        assign: true,
                        sp,
                    });
                }
            }
            m.nets.push(Net {
                name,
                w,
                signed,
                is_reg,
                init,
                sp,
            });
            if !self.eat(",") {
                break;
            }
        }
        self.expect(";")
    }

    fn param_decl(&mut self, m: &mut Module) -> PRes<()> {
        self.i += 1;
        self.eat_id("integer");
        self.eat_id("signed");
        if self.at_p("[") {
            self.range()?;
        }
        loop {
            let sp = self.sp();
            let name = self.id()?;
            self.expect("=")?;
            let e = self.expr()?;
            let Some(v) = vfold(&e, &self.env) else {
                return Err(PErr::Unsup(
                    sp,
                    format!("the parameter `{name}`, which is not constant"),
                ));
            };
            self.env.insert(name.clone(), v);
            m.params.push((name, v));
            if !self.eat(",") {
                break;
            }
        }
        self.expect(";")
    }

    fn lhs(&mut self) -> PRes<String> {
        if self.at_p("{") {
            return Err(PErr::Unsup(
                self.sp(),
                "an assignment to a concatenation".into(),
            ));
        }
        let name = self.id()?;
        if self.at_p("[") {
            return Err(PErr::Unsup(
                self.sp(),
                format!("a partial assignment to `{name}` (a bit or part select)"),
            ));
        }
        Ok(name)
    }

    fn assign(&mut self, m: &mut Module) -> PRes<()> {
        let sp = self.sp();
        self.i += 1;
        if self.at_p("#") || self.at_p("(") {
            return Err(PErr::Unsup(self.sp(), "delays and drive strengths".into()));
        }
        let lhs = self.lhs()?;
        self.expect("=")?;
        let rhs = self.expr()?;
        self.expect(";")?;
        m.blocks.push(Blk::Comb {
            body: vec![VSt::Assign { lhs, rhs, sp }],
            assign: true,
            sp,
        });
        Ok(())
    }

    fn always(&mut self, m: &mut Module) -> PRes<()> {
        let sp = self.sp();
        self.i += 1;
        self.expect("@")?;
        let mut edges: Vec<(bool, String)> = Vec::new();
        let mut levels = 0;
        if !self.eat("*") {
            self.expect("(")?;
            if !self.eat("*") {
                loop {
                    if self.eat_id("posedge") {
                        edges.push((true, self.id()?));
                    } else if self.eat_id("negedge") {
                        edges.push((false, self.id()?));
                    } else {
                        self.id()?;
                        levels += 1;
                    }
                    if !(self.eat(",") || self.eat_id("or")) {
                        break;
                    }
                }
            }
            self.expect(")")?;
        }
        if edges.is_empty() {
            if levels > 0 {
                m.notes.push((
                    sp,
                    "sensitivity list",
                    "the sensitivity list is ignored: the block is combinational and re-evaluated when any input changes, like `always @*`".into(),
                ));
            }
            let body = self.stmt(false)?;
            m.blocks.push(Blk::Comb {
                body,
                assign: false,
                sp,
            });
            return Ok(());
        }
        if edges.len() > 1 || levels > 0 {
            return Err(PErr::Unsup(
                sp,
                "an `always` block with an asynchronous reset or several clocks".into(),
            ));
        }
        let (rising, clk) = edges.remove(0);
        let body = self.stmt(true)?;
        m.blocks.push(Blk::Clocked {
            clk,
            rising,
            body,
            sp,
        });
        Ok(())
    }

    fn stmt(&mut self, clocked: bool) -> PRes<Vec<VSt>> {
        let sp = self.sp();
        let Some(tok) = self.tok() else {
            return Err(PErr::Syntax(sp, "unexpected end of input".into()));
        };
        match tok.k.clone() {
            K::P(";") => {
                self.i += 1;
                Ok(Vec::new())
            }
            K::P("#") => Err(PErr::Unsup(sp, "delays".into())),
            K::Sys(n) => Err(PErr::Unsup(sp, format!("the system task `${n}`"))),
            K::Id(w) => match w.as_str() {
                "begin" => {
                    self.i += 1;
                    if self.eat(":") {
                        self.id()?;
                    }
                    let mut out = Vec::new();
                    while !self.at_id("end") {
                        if self.tok().is_none() {
                            return Err(PErr::Syntax(self.sp(), "`begin` without `end`".into()));
                        }
                        if matches!(self.tok(), Some(Tok { k: K::Id(d), .. }) if matches!(d.as_str(), "reg" | "integer" | "wire"))
                        {
                            return Err(PErr::Unsup(
                                self.sp(),
                                "declarations inside a block".into(),
                            ));
                        }
                        out.extend(self.stmt(clocked)?);
                    }
                    self.i += 1;
                    Ok(out)
                }
                "if" => {
                    self.i += 1;
                    self.expect("(")?;
                    let cond = self.expr()?;
                    self.expect(")")?;
                    let then = self.stmt(clocked)?;
                    let els = if self.eat_id("else") {
                        self.stmt(clocked)?
                    } else {
                        Vec::new()
                    };
                    Ok(vec![VSt::If { cond, then, els }])
                }
                "case" => self.case(clocked),
                "casex" | "casez" => Err(PErr::Unsup(sp, format!("`{w}`"))),
                "for" | "while" | "repeat" | "forever" | "wait" | "disable" | "fork" => {
                    Err(PErr::Unsup(sp, format!("`{w}` statements")))
                }
                _ => self.assignment(clocked),
            },
            _ => self.assignment(clocked),
        }
    }

    fn assignment(&mut self, clocked: bool) -> PRes<Vec<VSt>> {
        let sp = self.sp();
        let lhs = self.lhs()?;
        let nb = if self.eat("<=") {
            true
        } else if self.eat("=") {
            false
        } else {
            return Err(PErr::Syntax(self.sp(), "expected `<=` or `=`".into()));
        };
        if clocked && !nb {
            return Err(PErr::Unsup(
                sp,
                format!("a blocking assignment to `{lhs}` in a clocked block"),
            ));
        }
        if !clocked && nb {
            return Err(PErr::Unsup(
                sp,
                format!("a non-blocking assignment to `{lhs}` in a combinational block"),
            ));
        }
        let rhs = self.expr()?;
        self.expect(";")?;
        Ok(vec![VSt::Assign { lhs, rhs, sp }])
    }

    fn case(&mut self, clocked: bool) -> PRes<Vec<VSt>> {
        self.i += 1;
        self.expect("(")?;
        let sel = self.expr()?;
        self.expect(")")?;
        let mut arms: Vec<(Vec<VEx>, Vec<VSt>)> = Vec::new();
        let mut default = None;
        while !self.at_id("endcase") {
            if self.tok().is_none() {
                return Err(PErr::Syntax(self.sp(), "`case` without `endcase`".into()));
            }
            if self.eat_id("default") {
                self.eat(":");
                default = Some(self.stmt(clocked)?);
                continue;
            }
            let mut labels = Vec::new();
            loop {
                labels.push(self.expr()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(":")?;
            arms.push((labels, self.stmt(clocked)?));
        }
        self.i += 1;
        Ok(vec![VSt::Case { sel, arms, default }])
    }

    fn expr(&mut self) -> PRes<VEx> {
        let c = self.bin(1)?;
        if self.at_p("?") {
            let sp = self.sp();
            self.i += 1;
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr()?;
            return Ok(VEx::Tern(Box::new(c), Box::new(a), Box::new(b), sp));
        }
        Ok(c)
    }

    fn bin(&mut self, min: u8) -> PRes<VEx> {
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
            l = VEx::Bin(op, Box::new(l), Box::new(r), sp);
        }
        Ok(l)
    }

    fn unary(&mut self) -> PRes<VEx> {
        let sp = self.sp();
        if let Some(&Tok { k: K::P(op), .. }) = self.tok() {
            if matches!(
                op,
                "-" | "+" | "!" | "~" | "&" | "|" | "^" | "~&" | "~|" | "~^" | "^~"
            ) {
                self.i += 1;
                let x = self.unary()?;
                return Ok(VEx::Un(op, Box::new(x), sp));
            }
        }
        self.primary()
    }

    fn primary(&mut self) -> PRes<VEx> {
        let sp = self.sp();
        let Some(tok) = self.tok() else {
            return Err(PErr::Syntax(sp, "expected an expression".into()));
        };
        match tok.k.clone() {
            K::Num(n) => {
                self.i += 1;
                Ok(VEx::Num(n, sp))
            }
            K::Str => Err(PErr::Unsup(sp, "strings".into())),
            K::P("#") => Err(PErr::Unsup(sp, "delays".into())),
            K::Directive(d) => Err(PErr::Unsup(sp, format!("the macro `` `{d}``"))),
            K::Sys(n) => {
                self.i += 1;
                let mut args = Vec::new();
                if self.eat("(") {
                    if !self.at_p(")") {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat(",") {
                                break;
                            }
                        }
                    }
                    self.expect(")")?;
                }
                Ok(VEx::Sys(n, args, sp))
            }
            K::P("(") => {
                self.i += 1;
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            K::P("{") => {
                self.i += 1;
                let first = self.expr()?;
                if self.at_p("{") {
                    // Replication: {n{a, b}}
                    self.i += 1;
                    let mut inner = Vec::new();
                    loop {
                        inner.push(self.expr()?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect("}")?;
                    self.expect("}")?;
                    return Ok(VEx::Repl(Box::new(first), inner, sp));
                }
                let mut parts = vec![first];
                while self.eat(",") {
                    parts.push(self.expr()?);
                }
                self.expect("}")?;
                Ok(VEx::Concat(parts, sp))
            }
            K::Id(name) => {
                self.i += 1;
                let base = VEx::Id(name, sp);
                if self.at_p("[") {
                    self.i += 1;
                    let a = self.expr()?;
                    if self.eat(":") {
                        let b = self.expr()?;
                        self.expect("]")?;
                        return Ok(VEx::Slice(Box::new(base), Box::new(a), Box::new(b), sp));
                    }
                    if self.at_p("+:") || self.at_p("-:") {
                        return Err(PErr::Unsup(self.sp(), "indexed part selects".into()));
                    }
                    self.expect("]")?;
                    return Ok(VEx::Index(Box::new(base), Box::new(a), sp));
                }
                Ok(base)
            }
            K::Escaped => Err(PErr::Unsup(sp, "escaped identifiers".into())),
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

/// A converted expression: Neuron text, its precedence and its width when known.
#[derive(Debug, Clone)]
struct Vx {
    s: String,
    prec: u8,
    w: Option<u32>,
    /// The root is `+`, `-`, `*` or `<<`, whose carry Neuron would lose.
    arith: bool,
    val: Option<i128>,
}

fn par(t: &Vx, min: u8) -> String {
    if t.prec < min {
        format!("({})", t.s)
    } else {
        t.s.clone()
    }
}

fn atom(s: String, w: Option<u32>) -> Vx {
    Vx {
        s,
        prec: ATOM,
        w,
        arith: false,
        val: None,
    }
}

/// A Neuron name for a Verilog name: not a keyword, not an `hdl` word.
fn nm(s: &str) -> String {
    let mut out = ident(s);
    if neuron_syntax::Keyword::lookup(&out).is_some()
        || matches!(
            out.as_str(),
            "clock"
                | "circuit"
                | "reg"
                | "wire"
                | "comb"
                | "rising"
                | "falling"
                | "sync"
                | "via"
                | "instance"
                | "out"
                | "inout"
                | "bit"
                | "sbit"
                | "result"
                | "old"
        )
    {
        out.push('_');
    }
    out
}

fn ty_text(w: u32, signed: bool) -> String {
    let base = if signed { "sbit" } else { "bit" };
    if w == 1 {
        base.to_string()
    } else {
        format!("{base}<{w}>")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SigKind {
    In,
    Out,
    Reg,
    Wire,
    Clock,
    Param,
}

#[derive(Debug, Clone)]
struct Sig {
    n: String,
    w: u32,
    kind: SigKind,
    val: Option<i128>,
}

type CRes<T> = Result<T, (Sp, String)>;

fn written(body: &[VSt], out: &mut BTreeSet<String>) {
    for s in body {
        match s {
            VSt::Assign { lhs, .. } => {
                out.insert(lhs.clone());
            }
            VSt::If { then, els, .. } => {
                written(then, out);
                written(els, out);
            }
            VSt::Case { arms, default, .. } => {
                for (_, b) in arms {
                    written(b, out);
                }
                if let Some(d) = default {
                    written(d, out);
                }
            }
        }
    }
}

/// Names assigned on every path through `body`.
fn always_assigned(body: &[VSt]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for s in body {
        match s {
            VSt::Assign { lhs, .. } => {
                out.insert(lhs.clone());
            }
            VSt::If { then, els, .. } => {
                let a = always_assigned(then);
                let b = always_assigned(els);
                out.extend(a.intersection(&b).cloned());
            }
            VSt::Case { arms, default, .. } => {
                if let Some(d) = default {
                    let mut common = always_assigned(d);
                    for (_, b) in arms {
                        let a = always_assigned(b);
                        common = common.intersection(&a).cloned().collect();
                    }
                    out.extend(common);
                }
            }
        }
    }
    out
}

fn reads_ex(e: &VEx, out: &mut BTreeSet<String>) {
    match e {
        VEx::Id(n, _) => {
            out.insert(n.clone());
        }
        VEx::Un(_, x, _) => reads_ex(x, out),
        VEx::Bin(_, a, b, _) => {
            reads_ex(a, out);
            reads_ex(b, out);
        }
        VEx::Tern(c, a, b, _) => {
            reads_ex(c, out);
            reads_ex(a, out);
            reads_ex(b, out);
        }
        VEx::Concat(v, _) | VEx::Sys(_, v, _) => v.iter().for_each(|x| reads_ex(x, out)),
        VEx::Repl(n, v, _) => {
            reads_ex(n, out);
            v.iter().for_each(|x| reads_ex(x, out));
        }
        VEx::Index(a, b, _) => {
            reads_ex(a, out);
            reads_ex(b, out);
        }
        VEx::Slice(a, b, c, _) => {
            reads_ex(a, out);
            reads_ex(b, out);
            reads_ex(c, out);
        }
        VEx::Num(..) => {}
    }
}

fn reads_body(body: &[VSt], out: &mut BTreeSet<String>) {
    for s in body {
        match s {
            VSt::Assign { rhs, .. } => reads_ex(rhs, out),
            VSt::If { cond, then, els } => {
                reads_ex(cond, out);
                reads_body(then, out);
                reads_body(els, out);
            }
            VSt::Case { sel, arms, default } => {
                reads_ex(sel, out);
                for (l, b) in arms {
                    l.iter().for_each(|x| reads_ex(x, out));
                    reads_body(b, out);
                }
                if let Some(d) = default {
                    reads_body(d, out);
                }
            }
        }
    }
}

struct Gen<'a> {
    file: &'a str,
    fid: &'a mut Fidelity,
    diags: &'a mut Vec<Diag>,
}

impl Gen<'_> {
    fn unsup(&mut self, sp: Sp, what: &str) {
        self.diags.push(Diag {
            code: "NE1002",
            message: format!("not imported: {what}"),
            help: Some(
                "write that part in Neuron by hand; the rest of the file was imported".to_string(),
            ),
            file: self.file.to_string(),
            span: sp,
        });
        self.fid.record(
            self.file,
            sp,
            Class::Unsupported,
            what,
            "not imported: it has no Neuron equivalent in the Verilog subset",
        );
    }
}

struct Mx<'a> {
    sigs: &'a BTreeMap<String, Sig>,
}

impl Mx<'_> {
    fn truth(&self, t: Vx) -> Vx {
        if t.w == Some(1) {
            t
        } else {
            Vx {
                s: format!("{} != 0", par(&t, BOR)),
                prec: CMP,
                w: Some(1),
                arith: false,
                val: None,
            }
        }
    }

    fn ex(&self, e: &VEx) -> CRes<Vx> {
        match e {
            VEx::Num(n, sp) => {
                if let Some(b) = n.bad {
                    return Err((*sp, b.to_string()));
                }
                Ok(Vx {
                    s: n.text.clone(),
                    prec: ATOM,
                    w: n.w,
                    arith: false,
                    val: n.val,
                })
            }
            VEx::Id(name, sp) => match self.sigs.get(name) {
                Some(s) if s.kind == SigKind::Clock => {
                    Err((*sp, format!("the clock `{name}` used as data")))
                }
                Some(s) if s.kind == SigKind::Param => Ok(Vx {
                    s: s.n.clone(),
                    prec: ATOM,
                    w: None,
                    arith: false,
                    val: s.val,
                }),
                Some(s) => Ok(atom(s.n.clone(), Some(s.w))),
                None => Err((*sp, format!("the undeclared name `{name}`"))),
            },
            VEx::Un(op, x, sp) => {
                let t = self.ex(x)?;
                match *op {
                    "!" => {
                        if t.w == Some(1) {
                            Ok(Vx {
                                s: format!("not {}", par(&t, NOT)),
                                prec: NOT,
                                w: Some(1),
                                arith: false,
                                val: None,
                            })
                        } else {
                            Ok(Vx {
                                s: format!("{} == 0", par(&t, BOR)),
                                prec: CMP,
                                w: Some(1),
                                arith: false,
                                val: None,
                            })
                        }
                    }
                    "~" => Ok(Vx {
                        s: format!("~{}", par(&t, UNARY)),
                        prec: UNARY,
                        w: t.w,
                        arith: false,
                        val: None,
                    }),
                    "+" => Ok(t),
                    "-" => Ok(Vx {
                        s: format!("0 -% {}", par(&t, ADD + 1)),
                        prec: ADD,
                        w: t.w,
                        arith: false,
                        val: None,
                    }),
                    "|" | "~|" => Ok(Vx {
                        s: format!(
                            "{} {} 0",
                            par(&t, BOR),
                            if *op == "|" { "!=" } else { "==" }
                        ),
                        prec: CMP,
                        w: Some(1),
                        arith: false,
                        val: None,
                    }),
                    "&" | "~&" => {
                        let Some(w) = t.w else {
                            return Err((*sp, "a reduction of a value of unknown width".into()));
                        };
                        let mask = (1i128 << w) - 1;
                        Ok(Vx {
                            s: format!(
                                "{} {} {mask}",
                                par(&t, BOR),
                                if *op == "&" { "==" } else { "!=" }
                            ),
                            prec: CMP,
                            w: Some(1),
                            arith: false,
                            val: None,
                        })
                    }
                    _ => Err((*sp, format!("the reduction operator `{op}` (parity)"))),
                }
            }
            VEx::Bin(op, a, b, sp) => self.binary(op, a, b, *sp),
            VEx::Tern(c, a, b, _) => {
                let c = self.ex(c)?;
                let c = self.truth(c);
                let x = self.ex(a)?;
                let y = self.ex(b)?;
                Ok(Vx {
                    s: format!("if {} then {} else {}", c.s, x.s, y.s),
                    prec: COND,
                    w: x.w.max(y.w),
                    arith: false,
                    val: None,
                })
            }
            VEx::Concat(parts, sp) => self.concat(parts, *sp),
            VEx::Repl(n, parts, sp) => {
                let Some(count) = vfold_sigs(n, self.sigs) else {
                    return Err((*sp, "a replication count that is not constant".into()));
                };
                if !(1..=64).contains(&count) {
                    return Err((*sp, "a replication count outside 1 to 64".into()));
                }
                let inner = self.concat(parts, *sp)?;
                let Some(w) = inner.w else {
                    return Err((*sp, "a replication of a value of unknown width".into()));
                };
                let one = par(&inner, ADD + 1);
                let text = vec![one; count as usize].join(" ++ ");
                Ok(Vx {
                    s: text,
                    prec: ADD,
                    w: Some(w * count as u32),
                    arith: false,
                    val: None,
                })
            }
            VEx::Index(base, idx, sp) => {
                let b = self.ex(base)?;
                if b.w.is_none() {
                    return Err((*sp, "a bit select of a constant".into()));
                }
                let i = self.ex(idx)?;
                Ok(atom(format!("{}[{}]", par(&b, ATOM), i.s), Some(1)))
            }
            VEx::Slice(base, hi, lo, sp) => {
                let b = self.ex(base)?;
                let (Some(h), Some(l)) = (vfold_sigs(hi, self.sigs), vfold_sigs(lo, self.sigs))
                else {
                    return Err((*sp, "a part select that is not constant".into()));
                };
                if h < l || l < 0 {
                    return Err((*sp, "a part select with its bounds reversed".into()));
                }
                Ok(atom(
                    format!("{}[{h}:{l}]", par(&b, ATOM)),
                    Some((h - l + 1) as u32),
                ))
            }
            VEx::Sys(n, _, sp) => Err((*sp, format!("the system function `${n}`"))),
        }
    }

    fn concat(&self, parts: &[VEx], sp: Sp) -> CRes<Vx> {
        let mut items = Vec::new();
        let mut total = 0u32;
        for p in parts {
            let t = self.ex(p)?;
            let Some(w) = t.w else {
                return Err((sp, "a concatenation with a part of unknown width".into()));
            };
            total += w;
            items.push(t);
        }
        if items.len() == 1 {
            return Ok(items.remove(0));
        }
        Ok(Vx {
            s: items
                .iter()
                .map(|t| par(t, ADD + 1))
                .collect::<Vec<_>>()
                .join(" ++ "),
            prec: ADD,
            w: Some(total),
            arith: false,
            val: None,
        })
    }

    fn binary(&self, op: &str, a: &VEx, b: &VEx, sp: Sp) -> CRes<Vx> {
        let l = self.ex(a)?;
        let r = self.ex(b)?;
        let w = match (l.w, r.w) {
            (Some(x), Some(y)) => Some(x.max(y)),
            (x, y) => x.or(y),
        };
        let plain = |s: String, prec: u8, w: Option<u32>, arith: bool| Vx {
            s,
            prec,
            w,
            arith,
            val: None,
        };
        match op {
            "&&" | "||" => {
                let (l, r) = (self.truth(l), self.truth(r));
                let (word, p) = if op == "&&" { ("and", AND) } else { ("or", OR) };
                Ok(plain(
                    format!("{} {word} {}", par(&l, p), par(&r, p + 1)),
                    p,
                    Some(1),
                    false,
                ))
            }
            "==" | "!=" | "<" | "<=" | ">" | ">=" => Ok(plain(
                format!("{} {op} {}", par(&l, BOR), par(&r, BOR)),
                CMP,
                Some(1),
                false,
            )),
            "&" | "|" | "^" => {
                let p = match op {
                    "&" => BAND,
                    "|" => BOR,
                    _ => BXOR,
                };
                Ok(plain(
                    format!("{} {op} {}", par(&l, p), par(&r, p + 1)),
                    p,
                    w,
                    false,
                ))
            }
            "~^" | "^~" => Ok(plain(
                format!("~({} ^ {})", par(&l, BXOR), par(&r, BXOR + 1)),
                UNARY,
                w,
                false,
            )),
            "+" | "-" | "*" => {
                let (nop, p) = match op {
                    "+" => ("+%", ADD),
                    "-" => ("-%", ADD),
                    _ => ("*%", MUL),
                };
                Ok(plain(
                    format!("{} {nop} {}", par(&l, p), par(&r, p + 1)),
                    p,
                    w,
                    true,
                ))
            }
            "/" | "%" => Ok(plain(
                format!("{} {op} {}", par(&l, MUL), par(&r, MUL + 1)),
                MUL,
                w,
                false,
            )),
            "<<" | ">>" => Ok(plain(
                format!("{} {op} {}", par(&l, SHIFT), par(&r, SHIFT + 1)),
                SHIFT,
                l.w,
                op == "<<",
            )),
            _ => Err((sp, format!("the operator `{op}`"))),
        }
    }
}

fn vfold_sigs(e: &VEx, sigs: &BTreeMap<String, Sig>) -> Option<i128> {
    let env: Params = sigs
        .iter()
        .filter(|(_, s)| s.kind == SigKind::Param)
        .filter_map(|(k, s)| s.val.map(|v| (k.clone(), v)))
        .collect();
    vfold(e, &env)
}

struct Lines {
    out: Vec<String>,
}

impl Lines {
    fn put(&mut self, depth: usize, text: impl AsRef<str>) {
        self.out
            .push(format!("{}{}", "  ".repeat(depth), text.as_ref()));
    }
}

struct Cv<'a> {
    mx: Mx<'a>,
    clocked: bool,
    lines: Lines,
}

impl Cv<'_> {
    fn cond(&self, e: &VEx) -> CRes<Vx> {
        Ok(self.mx.truth(self.mx.ex(e)?))
    }

    fn body(&mut self, body: &[VSt], depth: usize) -> CRes<()> {
        let before = self.lines.out.len();
        for s in body {
            self.stmt(s, depth)?;
        }
        if self.lines.out.len() == before {
            self.lines.put(depth, "pass");
        }
        Ok(())
    }

    fn stmt(&mut self, s: &VSt, depth: usize) -> CRes<()> {
        match s {
            VSt::Assign { lhs, rhs, sp, .. } => {
                let Some(sig) = self.mx.sigs.get(lhs) else {
                    return Err((
                        *sp,
                        format!("the assignment to the undeclared name `{lhs}`"),
                    ));
                };
                let ok = if self.clocked {
                    sig.kind == SigKind::Reg
                } else {
                    matches!(sig.kind, SigKind::Wire | SigKind::Out)
                };
                if !ok {
                    return Err((
                        *sp,
                        format!(
                            "the assignment to `{lhs}`, which is not {} in this kind of block",
                            if self.clocked {
                                "a register"
                            } else {
                                "a wire or an output"
                            }
                        ),
                    ));
                }
                let (n, lw) = (sig.n.clone(), sig.w);
                let t = self.mx.ex(rhs)?;
                if let Some(v) = t.val {
                    if !(0..(1i128 << lw)).contains(&v) {
                        return Err((
                            *sp,
                            format!(
                                "the constant {v}, which does not fit the {lw} bits of `{lhs}`"
                            ),
                        ));
                    }
                }
                if let Some(rw) = t.w {
                    if rw > lw {
                        return Err((
                            *sp,
                            format!("the assignment of {rw} bits to `{lhs}`, which has {lw} (Verilog truncates, Neuron does not)"),
                        ));
                    }
                    if t.arith && rw < lw {
                        return Err((
                            *sp,
                            format!("an arithmetic result for `{lhs}` that is wider than its operands (Neuron would drop the carry)"),
                        ));
                    }
                }
                self.lines.put(depth, format!("{n} = {}", t.s));
                Ok(())
            }
            VSt::If { cond, then, els } => {
                let c = self.cond(cond)?;
                self.lines.put(depth, format!("if {}:", c.s));
                self.body(then, depth + 1)?;
                let mut rest: &[VSt] = els;
                loop {
                    match rest {
                        [] => break,
                        [VSt::If { cond, then, els }] => {
                            let c = self.cond(cond)?;
                            self.lines.put(depth, format!("elif {}:", c.s));
                            self.body(then, depth + 1)?;
                            rest = els;
                        }
                        _ => {
                            self.lines.put(depth, "else:");
                            self.body(rest, depth + 1)?;
                            break;
                        }
                    }
                }
                Ok(())
            }
            VSt::Case { sel, arms, default } => {
                let sel = self.mx.ex(sel)?;
                if arms.is_empty() {
                    if let Some(d) = default {
                        for st in d {
                            self.stmt(st, depth)?;
                        }
                    }
                    return Ok(());
                }
                for (k, (labels, body)) in arms.iter().enumerate() {
                    let mut tests = Vec::new();
                    for l in labels {
                        let lt = self.mx.ex(l)?;
                        tests.push(format!("{} == {}", par(&sel, BOR), par(&lt, BOR)));
                    }
                    let test = if tests.len() == 1 {
                        tests.remove(0)
                    } else {
                        tests
                            .iter()
                            .map(|t| format!("({t})"))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    };
                    self.lines.put(
                        depth,
                        format!("{} {test}:", if k == 0 { "if" } else { "elif" }),
                    );
                    self.body(body, depth + 1)?;
                }
                if let Some(d) = default {
                    self.lines.put(depth, "else:");
                    self.body(d, depth + 1)?;
                }
                Ok(())
            }
        }
    }
}

/// A converted block waiting to be placed.
struct Piece {
    lines: Vec<String>,
    reads: BTreeSet<String>,
    writes: BTreeSet<String>,
    assign: bool,
}

struct Circuit {
    text: String,
    clock: Option<String>,
}

fn alloc(used: &mut BTreeSet<String>, base: &str) -> String {
    let b = nm(base);
    let mut n = b.clone();
    let mut k = 2;
    while used.contains(&n) {
        n = format!("{b}_{k}");
        k += 1;
    }
    used.insert(n.clone());
    n
}

/// The reset values of a clocked block and the rest of its body.
type Resets = (Vec<(String, i128)>, Vec<VSt>);

/// `if (rst) <constant resets> else <body>`: the reset values and the rest of the body.
fn split_reset(body: &[VSt], m: &Module, params: &Params) -> Option<Resets> {
    let [VSt::If {
        cond: VEx::Id(r, _),
        then,
        els,
    }] = body
    else {
        return None;
    };
    if !matches!(r.as_str(), "rst" | "reset") {
        return None;
    }
    if !m
        .ports
        .iter()
        .any(|p| &p.name == r && p.dir == Dir::In && p.w == 1)
    {
        return None;
    }
    let mut resets = Vec::new();
    for s in then {
        let VSt::Assign { lhs, rhs, .. } = s else {
            return None;
        };
        resets.push((lhs.clone(), vfold(rhs, params)?));
    }
    let mut rest = BTreeSet::new();
    written(els, &mut rest);
    let named: BTreeSet<&String> = resets.iter().map(|(n, _)| n).collect();
    if !rest.iter().all(|n| named.contains(n)) {
        return None;
    }
    Some((resets, els.clone()))
}

fn circuit(g: &mut Gen<'_>, m: &Module) -> Option<Circuit> {
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut sigs: BTreeMap<String, Sig> = BTreeMap::new();
    let params: Params = m.params.iter().cloned().collect();
    for (sp, c, n) in &m.notes {
        g.fid.record(g.file, *sp, Class::Equivalent, c, n);
    }
    for (name, v) in &m.params {
        let n = alloc(&mut used, name);
        sigs.insert(
            name.clone(),
            Sig {
                n,
                w: 0,
                kind: SigKind::Param,
                val: Some(*v),
            },
        );
    }

    // The clock and the blocks that are kept.
    let mut clock: Option<String> = None;
    let mut kept: Vec<&Blk> = Vec::new();
    for b in &m.blocks {
        match b {
            Blk::Clocked { clk, sp, .. } => {
                if !m
                    .ports
                    .iter()
                    .any(|p| &p.name == clk && p.dir == Dir::In && p.w == 1)
                {
                    g.unsup(
                        *sp,
                        &format!("a clocked block whose clock `{clk}` is not a 1-bit input port"),
                    );
                    continue;
                }
                match &clock {
                    None => clock = Some(clk.clone()),
                    Some(c) if c == clk => {}
                    Some(_) => {
                        g.unsup(*sp, "a block with a second clock (one clock per circuit)");
                        continue;
                    }
                }
                kept.push(b);
            }
            Blk::Comb { .. } => kept.push(b),
        }
    }

    // Reset folding: `if (rst) ... else ...` as the whole body of a clocked block.
    let mut bodies: Vec<Vec<VSt>> = Vec::new();
    let mut reset_vals: BTreeMap<String, i128> = BTreeMap::new();
    let mut unfolded_clocked = false;
    let mut conflict: BTreeSet<String> = BTreeSet::new();
    for b in &kept {
        match b {
            Blk::Clocked { body, .. } => match split_reset(body, m, &params) {
                Some((resets, rest)) => {
                    for (n, v) in resets {
                        if let Some(old) = reset_vals.insert(n.clone(), v) {
                            if old != v {
                                conflict.insert(n);
                            }
                        }
                    }
                    bodies.push(rest);
                }
                None => {
                    unfolded_clocked = true;
                    bodies.push(body.clone());
                }
            },
            Blk::Comb { body, .. } => bodies.push(body.clone()),
        }
    }
    // A declared initial value that differs from the reset value cannot be folded.
    let mut declared_init: BTreeMap<String, Option<i128>> = BTreeMap::new();
    for p in &m.ports {
        if let Some(e) = &p.init {
            declared_init.insert(p.name.clone(), vfold(e, &params));
        }
    }
    for n in &m.nets {
        if let Some(e) = &n.init {
            declared_init.insert(n.name.clone(), vfold(e, &params));
        }
    }
    for (n, v) in &reset_vals {
        if let Some(Some(d)) = declared_init.get(n) {
            if d != v {
                conflict.insert(n.clone());
            }
        }
    }
    if !conflict.is_empty() {
        // Fall back to the literal bodies when the reset values cannot become initial values.
        bodies = kept
            .iter()
            .map(|b| match b {
                Blk::Clocked { body, .. } | Blk::Comb { body, .. } => body.clone(),
            })
            .collect();
        reset_vals.clear();
        unfolded_clocked = clock.is_some();
    }

    let mut clocked_w = BTreeSet::new();
    let mut comb_w = BTreeSet::new();
    for (b, body) in kept.iter().zip(&bodies) {
        match b {
            Blk::Clocked { .. } => written(body, &mut clocked_w),
            Blk::Comb { .. } => written(body, &mut comb_w),
        }
    }
    // Names written by clocked blocks before folding (reset assignments count).
    for b in &kept {
        if let Blk::Clocked { body, .. } = b {
            written(body, &mut clocked_w);
        }
    }

    // Signals.
    let mut port_lines: Vec<String> = Vec::new();
    let mut out_copies: Vec<(String, String)> = Vec::new();
    let mut reg_decls: Vec<(String, u32, bool, Option<i128>)> = Vec::new();
    let mut wire_decls: Vec<(String, u32, bool)> = Vec::new();
    let mut default_init = false;
    let rename_reset = unfolded_clocked;
    for p in &m.ports {
        if Some(&p.name) == clock.as_ref() {
            sigs.insert(
                p.name.clone(),
                Sig {
                    n: nm(&p.name),
                    w: 1,
                    kind: SigKind::Clock,
                    val: None,
                },
            );
            used.insert(nm(&p.name));
            continue;
        }
        if p.w > 64 {
            g.unsup(p.sp, &format!("the port `{}`, wider than 64 bits", p.name));
            return None;
        }
        let mut base = p.name.clone();
        if rename_reset
            && p.dir == Dir::In
            && p.w == 1
            && matches!(p.name.as_str(), "rst" | "reset")
        {
            base = format!("{}_in", p.name);
            g.fid.record(
                g.file,
                p.sp,
                Class::Equivalent,
                "port renamed",
                &format!("the Verilog backend adds a synchronous reset to a circuit with a port named `{}`; the port is called `{base}` so that it does not", p.name),
            );
        }
        let n = alloc(&mut used, &base);
        let dir = match p.dir {
            Dir::In => "in",
            Dir::Out => "out",
            Dir::Inout => "inout",
        };
        port_lines.push(format!("{n}: {dir} {}", ty_text(p.w, p.signed)));
        if p.dir == Dir::Out && clocked_w.contains(&p.name) {
            let r = alloc(&mut used, &format!("{}_r", p.name));
            let init = reset_vals
                .get(&p.name)
                .copied()
                .or_else(|| p.init.as_ref().and_then(|e| vfold(e, &params)));
            if init.is_none() {
                default_init = true;
            }
            reg_decls.push((r.clone(), p.w, p.signed, init));
            out_copies.push((n, r.clone()));
            sigs.insert(
                p.name.clone(),
                Sig {
                    n: r,
                    w: p.w,
                    kind: SigKind::Reg,
                    val: None,
                },
            );
        } else {
            let kind = match p.dir {
                Dir::In => SigKind::In,
                _ => SigKind::Out,
            };
            sigs.insert(
                p.name.clone(),
                Sig {
                    n,
                    w: p.w,
                    kind,
                    val: None,
                },
            );
        }
    }
    for net in &m.nets {
        if net.w > 64 {
            g.unsup(net.sp, &format!("`{}`, wider than 64 bits", net.name));
            return None;
        }
        let n = alloc(&mut used, &net.name);
        let as_wire = !net.is_reg || comb_w.contains(&net.name);
        if as_wire {
            wire_decls.push((n.clone(), net.w, net.signed));
            sigs.insert(
                net.name.clone(),
                Sig {
                    n,
                    w: net.w,
                    kind: SigKind::Wire,
                    val: None,
                },
            );
        } else {
            let init = reset_vals
                .get(&net.name)
                .copied()
                .or_else(|| net.init.as_ref().and_then(|e| vfold(e, &params)));
            if init.is_none() {
                default_init = true;
            }
            reg_decls.push((n.clone(), net.w, net.signed, init));
            sigs.insert(
                net.name.clone(),
                Sig {
                    n,
                    w: net.w,
                    kind: SigKind::Reg,
                    val: None,
                },
            );
        }
    }
    if default_init {
        g.fid.record(
            g.file,
            m.name_sp,
            Class::Equivalent,
            "initial value",
            "registers without an initial value or reset value start at 0 (Verilog starts them at x)",
        );
    }

    // Blocks.
    let mut edge_pieces: Vec<Vec<String>> = Vec::new();
    let mut comb_pieces: Vec<Piece> = Vec::new();
    for (b, body) in kept.iter().zip(&bodies) {
        let (sp, clocked, rising) = match b {
            Blk::Clocked { sp, rising, .. } => (*sp, true, *rising),
            Blk::Comb { sp, .. } => (*sp, false, true),
        };
        let mut cv = Cv {
            mx: Mx { sigs: &sigs },
            clocked,
            lines: Lines { out: Vec::new() },
        };
        let depth = 2;
        let mut res = cv.body(body, depth);
        if res.is_ok() && !clocked {
            let mut w = BTreeSet::new();
            written(body, &mut w);
            let da = always_assigned(body);
            if let Some(x) = w.difference(&da).next() {
                res = Err((
                    sp,
                    format!("a combinational block that infers a latch (`{x}` is not assigned on every path)"),
                ));
            }
        }
        match res {
            Err((esp, msg)) => g.unsup(esp, &msg),
            Ok(()) => {
                if clocked {
                    let clk = nm(clock.as_deref().unwrap_or("clk"));
                    let edge = if rising { "rising" } else { "falling" };
                    let mut v = vec![format!("  on {edge}({clk}):")];
                    v.extend(cv.lines.out);
                    edge_pieces.push(v);
                } else {
                    let mut reads = BTreeSet::new();
                    reads_body(body, &mut reads);
                    let mut writes = BTreeSet::new();
                    written(body, &mut writes);
                    let assign = matches!(b, Blk::Comb { assign: true, .. });
                    comb_pieces.push(Piece {
                        lines: cv.lines.out,
                        reads,
                        writes,
                        assign,
                    });
                }
            }
        }
    }
    if !out_copies.is_empty() {
        let lines = out_copies
            .iter()
            .map(|(p, r)| format!("    {p} = {r}"))
            .collect();
        comb_pieces.push(Piece {
            lines,
            reads: BTreeSet::new(),
            writes: BTreeSet::new(),
            assign: false,
        });
    }
    // Combinational blocks in dependency order.
    let mut ordered: Vec<Piece> = Vec::new();
    let mut pending = comb_pieces;
    while !pending.is_empty() {
        let pick = (0..pending.len())
            .find(|&k| {
                pending
                    .iter()
                    .enumerate()
                    .all(|(j, o)| j == k || !pending[k].reads.iter().any(|r| o.writes.contains(r)))
            })
            .unwrap_or(0);
        ordered.push(pending.remove(pick));
    }

    // Text.
    let mut out: Vec<String> = Vec::new();
    for (name, v) in &m.params {
        if let Some(s) = sigs.get(name) {
            out.push(format!("  const {} = {v}", s.n));
        }
    }
    for (n, w, signed, init) in &reg_decls {
        out.push(format!(
            "  reg {n}: {} = {}",
            ty_text(*w, *signed),
            init.unwrap_or(0)
        ));
    }
    for (n, w, signed) in &wire_decls {
        out.push(format!("  wire {n}: {}", ty_text(*w, *signed)));
    }
    for e in edge_pieces {
        out.extend(e);
    }
    let mut k = 0;
    while k < ordered.len() {
        out.push("  comb:".to_string());
        out.extend(ordered[k].lines.iter().cloned());
        if ordered[k].assign {
            while k + 1 < ordered.len() && ordered[k + 1].assign {
                k += 1;
                out.extend(ordered[k].lines.iter().cloned());
            }
        }
        k += 1;
    }
    if out.is_empty() {
        out.push("  comb:".to_string());
        out.push("    pass".to_string());
    }
    let on = clock
        .as_ref()
        .map_or(String::new(), |c| format!(" on {}", nm(c)));
    let mut text = format!("circuit {}({}){on}:\n", nm(&m.name), port_lines.join(", "));
    for l in out {
        text.push_str(&l);
        text.push('\n');
    }
    g.fid.exact();
    Some(Circuit { text, clock })
}

/// Imports Verilog source as an `hdl` module called `name`.
pub(crate) fn import(file: &str, name: &str, text: &str) -> Result<Out, Vec<Diag>> {
    let toks = lex(text).map_err(|(p, l, m)| vec![syntax(file, p, l, &m)])?;
    let mut p = P {
        t: &toks,
        i: 0,
        end: text.len(),
        env: Params::new(),
    };
    let mut mods: Vec<Module> = Vec::new();
    let mut top_unsup: Vec<(Sp, String)> = Vec::new();
    while p.i < p.t.len() {
        if p.at_id("module") || p.at_id("macromodule") {
            p.env.clear();
            match p.module() {
                Ok(m) => mods.push(m),
                Err(PErr::Syntax(sp, m) | PErr::Unsup(sp, m)) => {
                    return Err(vec![syntax(file, sp[0], sp[1] - sp[0], &m)]);
                }
            }
        } else if let Some(Tok {
            k: K::Directive(d),
            pos,
            end,
        }) = p.tok()
        {
            top_unsup.push(([*pos, *end], format!("the compiler directive `` `{d}``")));
            p.i += 1;
        } else {
            let sp = p.sp();
            return Err(vec![syntax(
                file,
                sp[0],
                sp[1] - sp[0],
                "expected `module`",
            )]);
        }
    }
    if mods.is_empty() {
        return Err(vec![syntax(file, 0, 1, "no `module` in the input")]);
    }
    let mut fid = Fidelity::new("import-verilog", "neuron.hdl");
    let mut diags: Vec<Diag> = Vec::new();
    let mn = module_name(name, &mut fid, file, [0, 0]);
    let mut g = Gen {
        file,
        fid: &mut fid,
        diags: &mut diags,
    };
    for (sp, m) in &top_unsup {
        g.unsup(*sp, m);
    }
    let mut clocks: Vec<String> = Vec::new();
    let mut circuits: Vec<String> = Vec::new();
    for m in &mods {
        for (sp, msg) in &m.unsup {
            g.unsup(*sp, msg);
        }
        if let Some(c) = circuit(&mut g, m) {
            if let Some(clk) = c.clock {
                if !clocks.contains(&clk) {
                    clocks.push(clk);
                }
            }
            circuits.push(c.text);
        }
    }
    let mut src = format!("## Imported from Verilog by `neuron import`.\nmodule {mn} for hdl\n");
    for c in &clocks {
        src.push_str(&format!("clock {}: 100 MHz\n", nm(c)));
        fid.record(
            file,
            [0, 0],
            Class::Equivalent,
            "clock frequency",
            &format!("Verilog does not give the frequency of `{c}`; the clock is declared at 100 MHz, set the real frequency in the `clock` item"),
        );
    }
    for c in &circuits {
        src.push('\n');
        src.push_str(c);
    }
    finish(src, fid, diags, file)
}
