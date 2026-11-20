//! SPEL+-style robot programs to a `robot` module.
//!
//! The subset is what `neuron emit spel` writes plus hand-written programs with the same
//! statements (see `neuron-spel`). `main` holds the set-up (frames, tools, points and
//! their labels, the calls of the sequences); every other function except the
//! `Neuron*` helpers becomes a `sequence`; labels reached by `If TW Then GoTo` become
//! `step`s; `NeuronSafe` becomes the safe state.

use crate::st::{finish, Out};
use crate::util::{dur, ident, module_name, num, syntax};
use neuron_emit::{Class, Diag, Fidelity};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Id(String),
    Num(f64),
    Str(String),
    Sym(&'static str),
}

/// A line without its comment, and the comment.
fn split_comment(line: &str) -> (&str, &str) {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '\'' if !in_str => return (&line[..i], line[i + 1..].trim()),
            _ => {}
        }
    }
    (line, "")
}

fn lex(line: &str) -> Result<Vec<Tok>, String> {
    let b: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && b.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let s = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                i += 1;
            }
            let t: String = b[s..i].iter().collect();
            out.push(Tok::Num(
                t.parse().map_err(|_| format!("bad number `{t}`"))?,
            ));
        } else if c.is_alphabetic() || c == '_' {
            let s = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_' || b[i] == '$') {
                i += 1;
            }
            let t: String = b[s..i].iter().collect();
            out.push(Tok::Id(t.to_lowercase()));
        } else if c == '"' {
            let s = i + 1;
            i += 1;
            while i < b.len() && b[i] != '"' {
                i += 1;
            }
            if i >= b.len() {
                return Err("unterminated string".into());
            }
            out.push(Tok::Str(b[s..i].iter().collect()));
            i += 1;
        } else {
            let two: String = b[i..(i + 2).min(b.len())].iter().collect();
            let s2 = match two.as_str() {
                "<>" => Some("<>"),
                "<=" => Some("<="),
                ">=" => Some(">="),
                _ => None,
            };
            if let Some(s) = s2 {
                out.push(Tok::Sym(s));
                i += 2;
                continue;
            }
            let s = match c {
                '(' => "(",
                ')' => ")",
                ',' => ",",
                '=' => "=",
                '<' => "<",
                '>' => ">",
                '+' => "+",
                '-' => "-",
                '*' => "*",
                '/' => "/",
                ':' => ":",
                other => return Err(format!("unexpected `{other}`")),
            };
            out.push(Tok::Sym(s));
            i += 1;
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
enum E {
    Num(f64),
    Var(String),
    Neg(Box<E>),
    Not(Box<E>),
    Bin(&'static str, Box<E>, Box<E>),
    Call(String, Vec<E>),
}

struct P<'t> {
    t: &'t [Tok],
    i: usize,
}

impl P<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn sym(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(x)) if *x == s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn kw(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Id(x)) if x == s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<(), String> {
        if self.sym(s) {
            Ok(())
        } else {
            Err(format!("expected `{s}`"))
        }
    }
    fn expr(&mut self) -> Result<E, String> {
        let mut l = self.and()?;
        while self.kw("or") {
            let r = self.and()?;
            l = E::Bin("or", Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<E, String> {
        let mut l = self.not()?;
        while self.kw("and") {
            let r = self.not()?;
            l = E::Bin("and", Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn not(&mut self) -> Result<E, String> {
        if self.kw("not") {
            return Ok(E::Not(Box::new(self.not()?)));
        }
        self.cmp()
    }
    fn cmp(&mut self) -> Result<E, String> {
        let l = self.add()?;
        let op = match self.peek() {
            Some(Tok::Sym("=")) => "==",
            Some(Tok::Sym("<>")) => "!=",
            Some(Tok::Sym("<")) => "<",
            Some(Tok::Sym(">")) => ">",
            Some(Tok::Sym("<=")) => "<=",
            Some(Tok::Sym(">=")) => ">=",
            _ => return Ok(l),
        };
        self.i += 1;
        let r = self.add()?;
        Ok(E::Bin(op, Box::new(l), Box::new(r)))
    }
    fn add(&mut self) -> Result<E, String> {
        let mut l = self.mul()?;
        loop {
            let op = if self.sym("+") {
                "+"
            } else if self.sym("-") {
                "-"
            } else {
                return Ok(l);
            };
            let r = self.mul()?;
            l = E::Bin(op, Box::new(l), Box::new(r));
        }
    }
    fn mul(&mut self) -> Result<E, String> {
        let mut l = self.unary()?;
        loop {
            let op = if self.sym("*") {
                "*"
            } else if self.sym("/") {
                "/"
            } else if self.kw("mod") {
                "%"
            } else {
                return Ok(l);
            };
            let r = self.unary()?;
            l = E::Bin(op, Box::new(l), Box::new(r));
        }
    }
    fn unary(&mut self) -> Result<E, String> {
        if self.sym("-") {
            return Ok(E::Neg(Box::new(self.unary()?)));
        }
        self.prim()
    }
    fn prim(&mut self) -> Result<E, String> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(E::Num(n))
            }
            Some(Tok::Sym("(")) => {
                self.i += 1;
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Some(Tok::Id(n)) => {
                self.i += 1;
                if self.sym("(") {
                    Ok(E::Call(n, self.args()?))
                } else {
                    Ok(E::Var(n))
                }
            }
            other => Err(format!("expected a value, found {other:?}")),
        }
    }
    fn args(&mut self) -> Result<Vec<E>, String> {
        let mut v = Vec::new();
        if self.sym(")") {
            return Ok(v);
        }
        loop {
            v.push(self.expr()?);
            if self.sym(")") {
                return Ok(v);
            }
            self.expect(",")?;
        }
    }
    fn done(&self) -> Result<(), String> {
        match self.peek() {
            None => Ok(()),
            Some(t) => Err(format!("unexpected {t:?} at the end of the statement")),
        }
    }
    /// `P3` or `P3 +Z(d)`.
    fn pref(&mut self) -> Result<(u32, Option<E>), String> {
        let Some(Tok::Id(name)) = self.peek().cloned() else {
            return Err("expected a point such as `P3`".into());
        };
        let n = point_number(&name).ok_or_else(|| format!("`{name}` is not a point"))?;
        self.i += 1;
        let mut dz = None;
        if self.sym("+") {
            if !self.kw("z") {
                return Err("only `+Z(d)` offsets are supported".into());
            }
            self.expect("(")?;
            dz = Some(self.expr()?);
            self.expect(")")?;
        }
        Ok((n, dz))
    }
}

fn point_number(name: &str) -> Option<u32> {
    let rest = name.strip_prefix('p')?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

#[derive(Debug, Clone)]
enum Def {
    Ja(Vec<E>),
    Xy(Vec<E>, Option<u32>),
}

#[derive(Debug, Clone)]
enum S {
    Ignore(&'static str),
    Local(u32, Vec<E>),
    TlSet(u32, Vec<E>),
    Tool(u32),
    Weight(f64),
    PLabel(u32, String),
    SetPoint(u32, Def),
    Speed(E),
    SpeedLin(E),
    Cp(bool),
    Go(u32, Option<E>),
    Move(u32, Option<E>),
    Arc((u32, Option<E>), (u32, Option<E>)),
    On(E),
    Off(E),
    Wait(E),
    WaitCond(E, Option<E>),
    IfTw(String),
    IfOther,
    EndIf,
    For(String, E, E, bool),
    Next,
    Error(f64),
    Call(String),
    ExitFunction,
    Label(String),
    Unknown(String),
}

#[derive(Debug, Clone)]
struct L {
    s: S,
    span: [usize; 2],
    comment: String,
}

struct Func {
    name: String,
    body: Vec<L>,
}

fn parse_stmt(toks: &[Tok]) -> Result<S, String> {
    let mut p = P { t: toks, i: 0 };
    let Some(Tok::Id(head)) = p.peek().cloned() else {
        return Err("a statement must start with a word".into());
    };
    // `label:`
    if toks.len() == 2 && toks[1] == Tok::Sym(":") {
        return Ok(S::Label(head));
    }
    p.i += 1;
    let s = match head.as_str() {
        "motor" => S::Ignore("motor"),
        "integer" | "real" | "global" | "long" | "boolean" | "string" | "byte" | "double" => {
            S::Ignore("declaration")
        }
        "onerr" | "fend" | "quit" | "halt" | "fend:" => S::Ignore("error handling"),
        "local" | "tlset" => {
            let Some(Tok::Num(k)) = p.peek().cloned() else {
                return Err(format!("`{head}` needs a number"));
            };
            p.i += 1;
            p.expect(",")?;
            if !p.kw("xy") {
                return Err("expected `XY(...)`".into());
            }
            p.expect("(")?;
            let a = p.args()?;
            p.done()?;
            if head == "local" {
                S::Local(k as u32, a)
            } else {
                S::TlSet(k as u32, a)
            }
        }
        "tool" => {
            let Some(Tok::Num(k)) = p.peek().cloned() else {
                return Err("`Tool` needs a number".into());
            };
            S::Tool(k as u32)
        }
        "weight" => {
            let Some(Tok::Num(w)) = p.peek().cloned() else {
                return Err("`Weight` needs a number".into());
            };
            S::Weight(w)
        }
        "plabel" => {
            let Some(Tok::Num(k)) = p.peek().cloned() else {
                return Err("`PLabel` needs a point number".into());
            };
            p.i += 1;
            p.expect(",")?;
            let Some(Tok::Str(s)) = p.peek().cloned() else {
                return Err("`PLabel` needs a name in quotes".into());
            };
            S::PLabel(k as u32, s)
        }
        "speed" => S::Speed(p.expr()?),
        "speeds" => S::SpeedLin(p.expr()?),
        "cp" => {
            if p.kw("on") {
                S::Cp(true)
            } else if p.kw("off") {
                S::Cp(false)
            } else {
                return Err("`CP` needs `On` or `Off`".into());
            }
        }
        "go" | "move" => {
            let (n, dz) = p.pref()?;
            p.done()?;
            if head == "go" {
                S::Go(n, dz)
            } else {
                S::Move(n, dz)
            }
        }
        "arc" => {
            let a = p.pref()?;
            p.expect(",")?;
            let b = p.pref()?;
            p.done()?;
            S::Arc(a, b)
        }
        "on" => S::On(p.expr()?),
        "off" => S::Off(p.expr()?),
        "wait" => {
            let e = p.expr()?;
            if p.sym(",") {
                S::WaitCond(e, Some(p.expr()?))
            } else if is_cond(&e) {
                S::WaitCond(e, None)
            } else {
                S::Wait(e)
            }
        }
        "if" => {
            if p.kw("tw") && p.kw("then") && p.kw("goto") {
                let Some(Tok::Id(l)) = p.peek().cloned() else {
                    return Err("`GoTo` needs a label".into());
                };
                S::IfTw(l)
            } else {
                S::IfOther
            }
        }
        "endif" => S::EndIf,
        "for" => {
            let Some(Tok::Id(v)) = p.peek().cloned() else {
                return Err("`For` needs a variable".into());
            };
            p.i += 1;
            p.expect("=")?;
            let a = p.expr()?;
            if !p.kw("to") {
                return Err("expected `To`".into());
            }
            let b = p.expr()?;
            let step = p.kw("step");
            S::For(v, a, b, step)
        }
        "next" => S::Next,
        "error" => {
            let Some(Tok::Num(n)) = p.peek().cloned() else {
                return Err("`Error` needs a number".into());
            };
            S::Error(n)
        }
        "call" => {
            let Some(Tok::Id(n)) = p.peek().cloned() else {
                return Err("`Call` needs a function".into());
            };
            S::Call(n)
        }
        "exit" => S::ExitFunction,
        _ => {
            // `P3 = JA(...)` or `P3 = XY(...) /L1`.
            if let Some(n) = point_number(&head) {
                if p.sym("=") {
                    let Some(Tok::Id(kind)) = p.peek().cloned() else {
                        return Ok(S::Unknown(head));
                    };
                    if kind == "ja" || kind == "xy" {
                        p.i += 1;
                        p.expect("(")?;
                        let a = p.args()?;
                        let mut local = None;
                        if p.sym("/") {
                            match p.peek().cloned() {
                                Some(Tok::Id(l)) if l.starts_with('l') => {
                                    local = l[1..].parse().ok();
                                    p.i += 1;
                                }
                                _ => return Err("expected a local such as `/L1`".into()),
                            }
                        }
                        p.done()?;
                        return Ok(S::SetPoint(
                            n,
                            if kind == "ja" {
                                Def::Ja(a)
                            } else {
                                Def::Xy(a, local)
                            },
                        ));
                    }
                }
            }
            S::Unknown(head)
        }
    };
    Ok(s)
}

fn is_cond(e: &E) -> bool {
    match e {
        E::Not(_) => true,
        E::Bin(op, ..) => matches!(*op, "==" | "!=" | "<" | ">" | "<=" | ">=" | "and" | "or"),
        E::Call(n, _) => n == "sw" || n == "out",
        E::Var(n) => n == "tw",
        _ => false,
    }
}

fn parse(file: &str, text: &str) -> Result<Vec<Func>, Vec<Diag>> {
    let mut funcs: Vec<Func> = Vec::new();
    let mut cur: Option<Func> = None;
    let mut off = 0;
    for line in text.split('\n') {
        let start = off;
        off += line.len() + 1;
        let (code, comment) = split_comment(line);
        if code.trim().is_empty() {
            continue;
        }
        let span = [start, start + line.len()];
        let toks = lex(code)
            .map_err(|m| vec![syntax(file, span[0], line.len(), &format!("SPEL+: {m}"))])?;
        if let [Tok::Id(h), Tok::Id(n)] = toks.as_slice() {
            if h == "function" {
                if let Some(f) = cur.take() {
                    funcs.push(f);
                }
                cur = Some(Func {
                    name: n.clone(),
                    body: Vec::new(),
                });
                continue;
            }
        }
        if let Some(Tok::Id(h)) = toks.first() {
            if h == "function" {
                // `Function NeuronCheck(n As Integer)`
                if let Some(Tok::Id(n)) = toks.get(1) {
                    if let Some(f) = cur.take() {
                        funcs.push(f);
                    }
                    cur = Some(Func {
                        name: n.clone(),
                        body: Vec::new(),
                    });
                    continue;
                }
            }
            if h == "fend" {
                if let Some(f) = cur.take() {
                    funcs.push(f);
                }
                continue;
            }
        }
        let Some(f) = cur.as_mut() else {
            return Err(vec![syntax(
                file,
                span[0],
                line.len(),
                "SPEL+: a statement outside a function",
            )]);
        };
        let mut s = parse_stmt(&toks)
            .map_err(|m| vec![syntax(file, span[0], line.len(), &format!("SPEL+: {m}"))])?;
        // Labels keep their case: they become the names of steps.
        match &mut s {
            S::Label(n) => *n = code.trim().trim_end_matches(':').trim().to_string(),
            S::IfTw(n) => {
                if let Some(w) = code.split_whitespace().last() {
                    *n = w.to_string();
                }
            }
            _ => {}
        }
        f.body.push(L {
            s,
            span,
            comment: comment.to_string(),
        });
    }
    if let Some(f) = cur.take() {
        funcs.push(f);
    }
    if funcs.is_empty() {
        return Err(vec![syntax(
            file,
            0,
            1,
            "SPEL+: the program has no function",
        )]);
    }
    Ok(funcs)
}

// Precedence of Neuron expressions.
const OR: u8 = 4;
const AND: u8 = 5;
const NOT: u8 = 6;
const CMP: u8 = 7;
const ADD: u8 = 13;
const MUL: u8 = 14;
const NEG: u8 = 15;
const ATOM: u8 = 20;

fn par(t: String, p: u8, min: u8) -> String {
    if p < min {
        format!("({t})")
    } else {
        t
    }
}

type ForHead = (String, E, E, bool, [usize; 2]);

enum N {
    S(L),
    For {
        var: String,
        from: E,
        to: E,
        step: bool,
        body: Vec<N>,
        span: [usize; 2],
    },
}

fn nest(file: &str, body: &[L]) -> Result<Vec<N>, Vec<Diag>> {
    let mut stack: Vec<(Vec<N>, Option<ForHead>)> = vec![(Vec::new(), None)];
    for l in body {
        match &l.s {
            S::For(v, a, b, st) => stack.push((
                Vec::new(),
                Some((v.clone(), a.clone(), b.clone(), *st, l.span)),
            )),
            S::Next => {
                if stack.len() < 2 {
                    return Err(vec![syntax(
                        file,
                        l.span[0],
                        l.span[1] - l.span[0],
                        "SPEL+: `Next` without `For`",
                    )]);
                }
                let (nodes, head) = stack.pop().unwrap_or_default();
                if let Some((var, from, to, step, span)) = head {
                    if let Some(top) = stack.last_mut() {
                        top.0.push(N::For {
                            var,
                            from,
                            to,
                            step,
                            body: nodes,
                            span,
                        });
                    }
                }
            }
            _ => {
                if let Some(top) = stack.last_mut() {
                    top.0.push(N::S(l.clone()));
                }
            }
        }
    }
    if stack.len() != 1 {
        let l = body.last().map_or([0, 1], |l| l.span);
        return Err(vec![syntax(file, l[0], 1, "SPEL+: `For` without `Next`")]);
    }
    Ok(stack.pop().map(|s| s.0).unwrap_or_default())
}

struct Frame {
    k: u32,
    name: String,
    origin: [String; 3],
    rpy: [String; 3],
}

struct Tool {
    k: u32,
    name: String,
    offset: [String; 3],
    payload: Option<f64>,
}

struct G<'a> {
    file: &'a str,
    diags: Vec<Diag>,
    fid: Fidelity,
    frames: Vec<Frame>,
    tools: Vec<Tool>,
    selected_tool: Option<u32>,
    weight: Option<f64>,
    labels: BTreeMap<u32, String>,
    names: BTreeMap<u32, String>,
    module_points: Vec<String>,
    outputs: BTreeSet<i64>,
    inputs: BTreeSet<i64>,
    speed: Option<String>,
    speeds: Option<String>,
    cp: bool,
    cur: [usize; 2],
    pending_fault: Option<String>,
}

impl G<'_> {
    fn unsup(&mut self, what: &str) {
        let sp = self.cur;
        self.diags.push(Diag {
            code: "NE1002",
            message: format!("{what} has no Neuron equivalent yet"),
            help: Some(
                "write that part in Neuron by hand; the rest of the program was imported".into(),
            ),
            file: self.file.to_string(),
            span: sp,
        });
        self.fid.record(
            self.file,
            sp,
            Class::Unsupported,
            what,
            "not imported: it has no Neuron equivalent yet",
        );
    }

    fn fail<T>(&mut self, what: &str) -> Option<T> {
        self.unsup(what);
        None
    }

    fn skipped(&self, what: &str) -> Vec<String> {
        vec![format!("# not imported (NE1002): {what}")]
    }

    fn rex(&mut self, e: &E) -> Option<(String, u8)> {
        Some(match e {
            E::Num(v) => (num(*v), ATOM),
            E::Var(n) => match n.as_str() {
                "true" | "on" => ("true".into(), ATOM),
                "false" | "off" => ("false".into(), ATOM),
                "tw" => return self.fail("the timeout flag `TW` outside `If TW Then GoTo`"),
                _ => (ident(n), ATOM),
            },
            E::Neg(x) => {
                let (t, p) = self.rex(x)?;
                (format!("-{}", par(t, p, NEG)), NEG)
            }
            E::Not(x) => {
                let (t, p) = self.rex(x)?;
                (format!("not {}", par(t, p, NOT)), NOT)
            }
            E::Bin(op, a, b) => {
                // `Sw(5) = On` is the input itself.
                if matches!(*op, "==" | "!=") {
                    let flag = |x: &E| match x {
                        E::Var(n) if n == "on" => Some(true),
                        E::Var(n) if n == "off" => Some(false),
                        _ => None,
                    };
                    if let (Some(v), E::Call(..)) = (flag(b), &**a) {
                        let (t, p) = self.rex(a)?;
                        return Some(if v == (*op == "==") {
                            (t, p)
                        } else {
                            (format!("not {}", par(t, p, NOT)), NOT)
                        });
                    }
                }
                let (l, lp) = self.rex(a)?;
                let (r, rp) = self.rex(b)?;
                match *op {
                    "or" => (format!("{} or {}", par(l, lp, OR), par(r, rp, AND)), OR),
                    "and" => (format!("{} and {}", par(l, lp, AND), par(r, rp, NOT)), AND),
                    "==" | "!=" | "<" | ">" | "<=" | ">=" => (
                        format!("{} {op} {}", par(l, lp, CMP + 1), par(r, rp, CMP + 1)),
                        CMP,
                    ),
                    "+" | "-" => (
                        format!("{} {op} {}", par(l, lp, ADD), par(r, rp, ADD + 1)),
                        ADD,
                    ),
                    _ => (
                        format!("{} {op} {}", par(l, lp, MUL), par(r, rp, MUL + 1)),
                        MUL,
                    ),
                }
            }
            E::Call(n, args) => match (n.as_str(), args.as_slice()) {
                ("sw", [E::Num(b)]) => {
                    self.inputs.insert(*b as i64);
                    (format!("din{}", *b as i64), ATOM)
                }
                _ => return self.fail(&format!("the call of `{n}` in an expression")),
            },
        })
    }

    fn rx(&mut self, e: &E) -> Option<String> {
        self.rex(e).map(|x| x.0)
    }

    fn consts(&mut self, args: &[E], n: usize, what: &str) -> Option<Vec<String>> {
        if args.len() != n {
            return self.fail(&format!("{what} with {} values instead of {n}", args.len()));
        }
        let mut out = Vec::new();
        for a in args {
            out.push(self.rx(a)?);
        }
        Some(out)
    }

    fn zero(e: &E) -> bool {
        matches!(e, E::Num(v) if *v == 0.0)
    }

    fn point_def(&mut self, d: &Def) -> Option<String> {
        match d {
            Def::Ja(a) => {
                let v = self.consts(a, a.len(), "a joint point")?;
                Some(format!("joints({}) deg", v.join(", ")))
            }
            Def::Xy(a, local) => {
                if a.len() != 6 {
                    return self.fail("a point that does not have six values");
                }
                if !(Self::zero(&a[3]) && Self::zero(&a[4]) && Self::zero(&a[5])) {
                    return self
                        .fail("a point with an orientation (Neuron points carry a position only)");
                }
                let Some(k) = local else {
                    return self.fail("a point in the base frame (a point needs a local, `/L1`)");
                };
                let Some(f) = self
                    .frames
                    .iter()
                    .find(|f| f.k == *k)
                    .map(|f| f.name.clone())
                else {
                    return self.fail(&format!("a point in the undefined local {k}"));
                };
                let v = self.consts(&a[..3], 3, "a point")?;
                Some(format!("{f}.xyz({}) mm", v.join(", ")))
            }
        }
    }

    fn pref(&mut self, n: u32, dz: &Option<E>) -> Option<String> {
        let Some(name) = self.names.get(&n).cloned() else {
            return self.fail(&format!(
                "a motion to the point P{n}, which the program has not defined"
            ));
        };
        match dz {
            None => Some(name),
            Some(e) => {
                let d = self.rx(e)?;
                Some(format!("{name}.above({d} mm)"))
            }
        }
    }

    fn bit(&mut self, e: &E, output: bool) -> Option<String> {
        let E::Num(b) = e else {
            return self.fail("an output whose bit number is not a constant");
        };
        let b = *b as i64;
        if output {
            self.outputs.insert(b);
            Some(format!("dout{b}"))
        } else {
            self.inputs.insert(b);
            Some(format!("din{b}"))
        }
    }

    fn secs(&mut self, e: &E) -> Option<String> {
        match e {
            E::Num(v) => Some(dur((v * 1e9).round() as i64)),
            _ => self.fail("a time that is not a constant"),
        }
    }

    fn indent(l: Vec<String>, by: &str) -> Vec<String> {
        l.into_iter().map(|x| format!("{by}{x}")).collect()
    }

    fn nodes(&mut self, nodes: &[N], top: bool) -> Vec<String> {
        let mut out = Vec::new();
        let mut ind = "";
        let mut i = 0;
        while i < nodes.len() {
            match &nodes[i] {
                N::For {
                    var,
                    from,
                    to,
                    step,
                    body,
                    span,
                } => {
                    self.cur = *span;
                    if *step {
                        self.unsup("a `For` loop with `Step`");
                        out.push("# not imported (NE1002): For loop with Step".into());
                    } else {
                        let a = self.rx(from);
                        let hi = match to {
                            E::Num(v) => Some(num(v + 1.0)),
                            e => self.rex(e).map(|(t, p)| format!("{} + 1", par(t, p, ADD))),
                        };
                        match (a, hi) {
                            (Some(a), Some(hi)) => {
                                out.push(format!("{ind}for {} in {a}..{hi}:", ident(var)));
                                let mut b = self.nodes(body, false);
                                if !b.iter().any(|l| !l.trim_start().starts_with('#')) {
                                    b.push("pass".into());
                                }
                                out.extend(Self::indent(b, "  "));
                                self.fid.exact();
                            }
                            _ => out.push("# not imported (NE1002): For loop".into()),
                        }
                    }
                }
                N::S(l) => {
                    self.cur = l.span;
                    if top {
                        if let S::Label(name) = &l.s {
                            if name.eq_ignore_ascii_case("neuronfault") {
                                // The error handler: the rest of the function.
                                return out;
                            }
                            out.push(format!("step {}:", ident(name)));
                            ind = "  ";
                            i += 1;
                            continue;
                        }
                    }
                    // `Wait cond, d` and the `If TW Then GoTo L` that follows it.
                    let mut consumed = 1;
                    let lines = match &l.s {
                        S::WaitCond(c, t) => {
                            let cond = self.rx(c);
                            let timeout = match t {
                                Some(t) => self.secs(t),
                                None => Some(String::new()),
                            };
                            let target = match nodes.get(i + 1) {
                                Some(N::S(L {
                                    s: S::IfTw(label), ..
                                })) if t.is_some() => {
                                    consumed = 2;
                                    Some(label.clone())
                                }
                                _ => None,
                            };
                            match (cond, timeout) {
                                (Some(c), Some(t)) => {
                                    self.fid.exact();
                                    vec![format!(
                                        "until {c}{}{}",
                                        if t.is_empty() {
                                            String::new()
                                        } else {
                                            format!(" timeout {t}")
                                        },
                                        target.map_or(String::new(), |l| format!(
                                            " -> {}",
                                            ident(&l)
                                        ))
                                    )]
                                }
                                _ => self.skipped("wait"),
                            }
                        }
                        other => {
                            if let S::Error(_) = other {
                                self.pending_fault =
                                    l.comment.strip_prefix("fault ").map(|n| ident(n.trim()));
                            }
                            self.stmt(other)
                        }
                    };
                    out.extend(Self::indent(lines, ind));
                    i += consumed;
                    continue;
                }
            }
            i += 1;
        }
        out
    }

    fn stmt(&mut self, s: &S) -> Vec<String> {
        match s {
            S::Ignore(what) => {
                if *what == "motor" {
                    self.fid.record(
                        self.file,
                        self.cur,
                        Class::Equivalent,
                        "motor power",
                        "the power-up of the arm is the controller's business; Neuron does not write it",
                    );
                }
                vec![]
            }
            S::Speed(e) => {
                self.speed = self.rx(e);
                vec![]
            }
            S::SpeedLin(e) => {
                self.speeds = self.rx(e);
                vec![]
            }
            S::Cp(on) => {
                self.cp = *on;
                vec![]
            }
            S::Go(n, dz) => {
                let Some(p) = self.pref(*n, dz) else {
                    return self.skipped("motion");
                };
                let speed = match self.speed.clone() {
                    Some(s) => s,
                    None => {
                        self.default_speed("Speed", "100");
                        "100".into()
                    }
                };
                self.fid.exact();
                vec![format!("move ptp to {p} speed {speed} %")]
            }
            S::Move(n, dz) => {
                let Some(p) = self.pref(*n, dz) else {
                    return self.skipped("motion");
                };
                let speed = match self.speeds.clone() {
                    Some(s) => s,
                    None => {
                        self.default_speed("SpeedS", "100");
                        "100".into()
                    }
                };
                let blend = if self.cp {
                    self.fid.record(
                        self.file,
                        self.cur,
                        Class::Approximate,
                        "blend",
                        "`CP On` does not say the blend radius; 10 mm is assumed",
                    );
                    " blend 10 mm"
                } else {
                    self.fid.exact();
                    ""
                };
                vec![format!("move linear to {p} speed {speed} mm/s{blend}")]
            }
            S::Arc((m, mdz), (e, edz)) => {
                let (Some(via), Some(end)) = (self.pref(*m, mdz), self.pref(*e, edz)) else {
                    return self.skipped("arc");
                };
                let speed = match self.speeds.clone() {
                    Some(s) => s,
                    None => {
                        self.default_speed("SpeedS", "100");
                        "100".into()
                    }
                };
                self.fid.exact();
                vec![format!("move arc to {end} via {via} speed {speed} mm/s")]
            }
            S::On(e) | S::Off(e) => {
                let Some(n) = self.bit(e, true) else {
                    return self.skipped("output");
                };
                self.fid.exact();
                vec![format!("{n}.write({})", matches!(s, S::On(_)))]
            }
            S::Wait(e) => {
                let Some(d) = self.secs(e) else {
                    return self.skipped("wait");
                };
                self.fid.exact();
                vec![format!("wait {d}")]
            }
            S::IfTw(_) => {
                self.unsup("`If TW Then GoTo` without a `Wait` with a time-out before it");
                self.skipped("If TW")
            }
            S::Error(n) => {
                let name = self.fault_name(*n);
                self.fid.exact();
                vec![format!("fault {name}")]
            }
            S::SetPoint(n, def) => {
                let Some(d) = self.point_def(def) else {
                    return self.skipped("point");
                };
                let name = format!("p{n}");
                self.names.insert(*n, name.clone());
                self.fid.exact();
                vec![format!("let {name} = {d}")]
            }
            S::Call(name) => {
                if name == "neuroncheck" {
                    self.fid.record(
                        self.file,
                        self.cur,
                        Class::Equivalent,
                        "workspace check",
                        "`NeuronCheck` raises error 8100 at run time; Neuron checks constant targets when it compiles (NE0804)",
                    );
                    vec![]
                } else if name == "neuronsafe" {
                    vec![]
                } else {
                    self.unsup(&format!("the call of `{name}` inside a sequence"));
                    self.skipped("call")
                }
            }
            S::ExitFunction | S::Label(_) => vec![],
            S::Local(..) | S::TlSet(..) | S::Tool(_) | S::Weight(_) | S::PLabel(..) => {
                self.unsup("a frame, tool or point declaration outside `main`");
                self.skipped("declaration")
            }
            S::IfOther => {
                self.unsup("an `If` statement (only `If TW Then GoTo` is read)");
                self.skipped("If")
            }
            S::EndIf => vec![],
            S::Unknown(w) => {
                self.unsup(&format!("the statement `{w}`"));
                self.skipped(&format!("statement {w}"))
            }
            S::WaitCond(..) | S::For(..) | S::Next => vec![],
        }
    }

    fn default_speed(&mut self, which: &str, v: &str) {
        self.fid.record(
            self.file,
            self.cur,
            Class::Approximate,
            "speed",
            &format!("no `{which}` before this motion; {v} is assumed"),
        );
    }

    fn fault_name(&mut self, n: f64) -> String {
        self.pending_fault
            .take()
            .unwrap_or_else(|| format!("Error{}", n as i64))
    }
}

/// The word after `kind ` in a comment such as `frame pallet; rpy(...) ...`.
fn comment_name(comment: &str, kind: &str) -> Option<String> {
    let rest = comment.strip_prefix(kind)?.trim_start();
    let w: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if w.is_empty() {
        None
    } else {
        Some(ident(&w))
    }
}

fn reserved(name: &str) -> bool {
    name.starts_with("neuron")
}

/// Imports a SPEL+-style program.
pub(crate) fn import(file: &str, name: &str, text: &str) -> Result<Out, Vec<Diag>> {
    let funcs = parse(file, text)?;
    let mut g = G {
        file,
        diags: Vec::new(),
        fid: Fidelity::new("import-spel", "neuron.robot"),
        frames: Vec::new(),
        tools: Vec::new(),
        selected_tool: None,
        weight: None,
        labels: BTreeMap::new(),
        names: BTreeMap::new(),
        module_points: Vec::new(),
        outputs: BTreeSet::new(),
        inputs: BTreeSet::new(),
        speed: None,
        speeds: None,
        cp: false,
        cur: [0, 0],
        pending_fault: None,
    };
    let main = funcs.iter().find(|f| f.name == "main");
    let mut called: Vec<String> = Vec::new();
    let mut main_actions: Vec<L> = Vec::new();
    if let Some(m) = main {
        for l in &m.body {
            if let S::PLabel(n, s) = &l.s {
                g.labels.insert(*n, ident(s));
            }
        }
        for l in &m.body {
            g.cur = l.span;
            match &l.s {
                S::Local(k, a) => {
                    if a.len() != 6 {
                        g.unsup("a local with a number of values other than six");
                        continue;
                    }
                    if let Some(v) = g.consts(a, 6, "a local") {
                        let name = comment_name(&l.comment, "frame ")
                            .unwrap_or_else(|| format!("local{k}"));
                        g.frames.push(Frame {
                            k: *k,
                            name,
                            origin: [v[0].clone(), v[1].clone(), v[2].clone()],
                            rpy: [v[5].clone(), v[4].clone(), v[3].clone()],
                        });
                        g.fid.exact();
                    }
                }
                S::TlSet(k, a) => {
                    if let Some(v) = g.consts(a, 6, "a tool") {
                        if v[3..].iter().any(|x| x != "0") {
                            g.unsup(
                                "a tool with an orientation (Neuron tools carry an offset only)",
                            );
                            continue;
                        }
                        let name =
                            comment_name(&l.comment, "tool ").unwrap_or_else(|| format!("tool{k}"));
                        g.tools.push(Tool {
                            k: *k,
                            name,
                            offset: [v[0].clone(), v[1].clone(), v[2].clone()],
                            payload: None,
                        });
                        g.fid.exact();
                    }
                }
                S::Tool(k) => g.selected_tool = Some(*k),
                S::Weight(w) => g.weight = Some(*w),
                S::PLabel(..) | S::ExitFunction | S::Label(_) | S::EndIf => {}
                S::SetPoint(n, d) => {
                    let pname = g.labels.get(n).cloned().unwrap_or_else(|| format!("p{n}"));
                    if let Some(def) = g.point_def(d) {
                        g.module_points.push(format!("point {pname} = {def}"));
                        g.names.insert(*n, pname);
                        g.fid.exact();
                    }
                }
                S::Call(c) => {
                    if !reserved(c) {
                        called.push(c.clone());
                    }
                }
                S::Ignore(_) => {
                    g.stmt(&l.s);
                }
                _ => main_actions.push(l.clone()),
            }
        }
    }
    if let Some(w) = g.weight {
        let sel = g.selected_tool;
        let idx = g.tools.iter().position(|t| Some(t.k) == sel).unwrap_or(0);
        if let Some(t) = g.tools.get_mut(idx) {
            t.payload = Some(w);
        }
    }
    // Which functions are sequences.
    let mut seqs: Vec<&Func> = Vec::new();
    if main.is_some() {
        for c in &called {
            if let Some(f) = funcs.iter().find(|f| &f.name == c) {
                if !seqs.iter().any(|s| s.name == f.name) {
                    seqs.push(f);
                }
            }
        }
    } else {
        seqs.extend(funcs.iter().filter(|f| !reserved(&f.name)));
    }
    let mut seq_lines: Vec<Vec<String>> = Vec::new();
    if !main_actions.is_empty() {
        let nodes = nest(file, &main_actions)?;
        let mut lines = g.nodes(&nodes, true);
        if !lines.iter().any(|l| !l.trim_start().starts_with('#')) {
            lines.push("pass".into());
        }
        let mut out = vec!["sequence main():".to_string()];
        out.extend(G::indent(lines, "  "));
        seq_lines.push(out);
    }
    for f in &seqs {
        let nodes = nest(file, &f.body)?;
        let mut lines = g.nodes(&nodes, true);
        if !lines.iter().any(|l| !l.trim_start().starts_with('#')) {
            lines.push("pass".into());
        }
        let mut out = vec![format!("sequence {}():", ident(&f.name))];
        out.extend(G::indent(lines, "  "));
        seq_lines.push(out);
    }
    // The safe state.
    let mut safe: Vec<String> = Vec::new();
    if let Some(f) = funcs.iter().find(|f| f.name == "neuronsafe") {
        for l in &f.body {
            g.cur = l.span;
            if matches!(l.s, S::On(_) | S::Off(_)) {
                safe.extend(g.stmt(&l.s));
            }
        }
    }

    // ---- the Neuron source ----------------------------------------------------------
    let mname = module_name(
        if name.is_empty() { "robot" } else { name },
        &mut g.fid,
        file,
        [0, 0],
    );
    let mut src = format!(
        "## Imported from a SPEL+-style program by neuron-import.\nmodule {mname} for robot\n"
    );
    let mut decls: Vec<String> = Vec::new();
    for f in &g.frames {
        decls.push(format!(
            "frame {} = frame(origin: xyz({}, {}, {}) mm, rot: rpy({}, {}, {}) deg)",
            f.name, f.origin[0], f.origin[1], f.origin[2], f.rpy[0], f.rpy[1], f.rpy[2]
        ));
    }
    for t in &g.tools {
        decls.push(format!(
            "tool {} = tool(offset: xyz({}, {}, {}) mm, payload: {} kg)",
            t.name,
            t.offset[0],
            t.offset[1],
            t.offset[2],
            num(t.payload.unwrap_or(0.0))
        ));
    }
    if !decls.is_empty() {
        src.push('\n');
        for d in &decls {
            src.push_str(d);
            src.push('\n');
        }
    }
    if !g.outputs.is_empty() || !g.inputs.is_empty() {
        src.push('\n');
        for b in &g.outputs {
            src.push_str(&format!("io dout{b}: gpio.output @ dout({b})\n"));
        }
        for b in &g.inputs {
            src.push_str(&format!("io din{b}: gpio.input @ din({b})\n"));
        }
    }
    if !g.module_points.is_empty() {
        src.push('\n');
        for p in &g.module_points {
            src.push_str(p);
            src.push('\n');
        }
    }
    for s in &seq_lines {
        src.push('\n');
        for l in s {
            src.push_str(l);
            src.push('\n');
        }
    }
    if safe.iter().any(|l| !l.trim_start().starts_with('#')) {
        src.push_str("\nsafety:\n  safe_state:\n");
        for l in &safe {
            src.push_str(&format!("    {l}\n"));
        }
    }
    finish(src, g.fid, g.diags, file)
}
