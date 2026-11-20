//! Lexer and parser of the subset: one statement per line.

use crate::{Ex, Func, Ins, Op, PRef, PointDef, Program};
use std::fmt;

/// A line the parser could not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Line number, from 1.
    pub line: usize,
    /// What was wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Id(String),
    Num(f64),
    Str(String),
    Sym(&'static str),
}

fn lex(line: &str) -> Result<Vec<Tok>, String> {
    let b: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '\'' {
            break;
        } else if c.is_ascii_digit() || (c == '.' && b.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                i += 1;
            }
            let text: String = b[start..i].iter().collect();
            out.push(Tok::Num(
                text.parse().map_err(|_| format!("bad number `{text}`"))?,
            ));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_' || b[i] == '$') {
                i += 1;
            }
            let text: String = b[start..i].iter().collect();
            out.push(Tok::Id(text.to_lowercase()));
        } else if c == '"' {
            let start = i + 1;
            i += 1;
            while i < b.len() && b[i] != '"' {
                i += 1;
            }
            if i >= b.len() {
                return Err("unterminated string".into());
            }
            out.push(Tok::Str(b[start..i].iter().collect()));
            i += 1;
        } else {
            let two: String = b[i..(i + 2).min(b.len())].iter().collect();
            let sym = match two.as_str() {
                "<>" => Some("<>"),
                "<=" => Some("<="),
                ">=" => Some(">="),
                _ => None,
            };
            if let Some(s) = sym {
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
                ';' => ";",
                '#' => "#",
                other => return Err(format!("unexpected `{other}`")),
            };
            out.push(Tok::Sym(s));
            i += 1;
        }
    }
    Ok(out)
}

struct Ep<'a> {
    t: &'a [Tok],
    i: usize,
}

impl Ep<'_> {
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

    fn expr(&mut self) -> Result<Ex, String> {
        let mut l = self.and()?;
        while self.kw("or") {
            let r = self.and()?;
            l = Ex::Bin(Op::Or, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn and(&mut self) -> Result<Ex, String> {
        let mut l = self.not()?;
        while self.kw("and") {
            let r = self.not()?;
            l = Ex::Bin(Op::And, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn not(&mut self) -> Result<Ex, String> {
        if self.kw("not") {
            return Ok(Ex::Not(Box::new(self.not()?)));
        }
        self.cmp()
    }

    fn cmp(&mut self) -> Result<Ex, String> {
        let l = self.add()?;
        let op = match self.peek() {
            Some(Tok::Sym("=")) => Op::Eq,
            Some(Tok::Sym("<>")) => Op::Ne,
            Some(Tok::Sym("<")) => Op::Lt,
            Some(Tok::Sym(">")) => Op::Gt,
            Some(Tok::Sym("<=")) => Op::Le,
            Some(Tok::Sym(">=")) => Op::Ge,
            _ => return Ok(l),
        };
        self.i += 1;
        let r = self.add()?;
        Ok(Ex::Bin(op, Box::new(l), Box::new(r)))
    }

    fn add(&mut self) -> Result<Ex, String> {
        let mut l = self.mul()?;
        loop {
            let op = if self.sym("+") {
                Op::Add
            } else if self.sym("-") {
                Op::Sub
            } else {
                return Ok(l);
            };
            let r = self.mul()?;
            l = Ex::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn mul(&mut self) -> Result<Ex, String> {
        let mut l = self.unary()?;
        loop {
            let op = if self.sym("*") {
                Op::Mul
            } else if self.sym("/") {
                Op::Div
            } else if self.kw("mod") {
                Op::Mod
            } else {
                return Ok(l);
            };
            let r = self.unary()?;
            l = Ex::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn unary(&mut self) -> Result<Ex, String> {
        if self.sym("-") {
            return Ok(Ex::Neg(Box::new(self.unary()?)));
        }
        self.prim()
    }

    fn prim(&mut self) -> Result<Ex, String> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Ex::Num(n))
            }
            Some(Tok::Sym("(")) => {
                self.i += 1;
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Some(Tok::Id(name)) => {
                self.i += 1;
                if self.sym("(") {
                    let args = self.args()?;
                    Ok(Ex::Call(name, args))
                } else {
                    Ok(Ex::Var(name))
                }
            }
            other => Err(format!("expected a value, found {other:?}")),
        }
    }

    /// Arguments after `(`, up to and including `)`.
    fn args(&mut self) -> Result<Vec<Ex>, String> {
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
    fn pref(&mut self) -> Result<PRef, String> {
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
        Ok(PRef { n, dz })
    }
}

fn point_number(name: &str) -> Option<u32> {
    let rest = name.strip_prefix('p')?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

enum Block {
    If {
        jf: Option<usize>,
        ends: Vec<usize>,
    },
    For {
        head: usize,
        test: usize,
        var: String,
        step: Ex,
    },
}

struct Builder {
    funcs: Vec<Func>,
    globals: Vec<String>,
    cur: Option<Func>,
    blocks: Vec<Block>,
}

fn patch(code: &mut [Ins], at: usize, to: usize) {
    match code.get_mut(at) {
        Some(Ins::Jmp(t) | Ins::JmpIfNot(_, t)) => *t = to,
        Some(Ins::ForTest { exit, .. }) => *exit = to,
        _ => {}
    }
}

impl Builder {
    fn code(&mut self) -> Result<&mut Func, String> {
        self.cur
            .as_mut()
            .ok_or_else(|| "a statement outside a function".to_string())
    }

    fn emit(&mut self, i: Ins) -> Result<usize, String> {
        let f = self.code()?;
        f.code.push(i);
        Ok(f.code.len() - 1)
    }

    fn here(&mut self) -> Result<usize, String> {
        Ok(self.code()?.code.len())
    }

    fn statement(&mut self, t: &[Tok]) -> Result<(), String> {
        let Some(first) = t.first() else {
            return Ok(());
        };
        let Tok::Id(kw) = first else {
            return Err("a statement starts with a word".into());
        };
        let rest = &t[1..];
        let mut p = Ep { t: rest, i: 0 };
        match kw.as_str() {
            "function" => {
                let Some(Tok::Id(name)) = rest.first() else {
                    return Err("expected a function name".into());
                };
                if self.cur.is_some() {
                    return Err("`Function` inside a function".into());
                }
                self.cur = Some(Func {
                    name: name.clone(),
                    code: Vec::new(),
                    labels: Vec::new(),
                });
                Ok(())
            }
            "fend" => {
                if !self.blocks.is_empty() {
                    return Err("`Fend` with an open `If` or `For`".into());
                }
                let f = self.cur.take().ok_or("`Fend` without `Function`")?;
                self.funcs.push(f);
                Ok(())
            }
            "global" => {
                if let Some(Tok::Id(name)) = rest.get(1) {
                    self.globals.push(name.clone());
                }
                Ok(())
            }
            "integer" | "real" | "boolean" | "string" | "double" | "long" | "byte" => Ok(()),
            "motor" | "power" | "accel" | "accels" | "fine" | "print" | "eclr" => Ok(()),
            "if" => self.if_stmt(rest),
            "elseif" => {
                let c = Self::cond_before_then(rest)?;
                self.close_branch()?;
                let jf = self.emit(Ins::JmpIfNot(c, 0))?;
                if let Some(Block::If { jf: slot, .. }) = self.blocks.last_mut() {
                    *slot = Some(jf);
                }
                Ok(())
            }
            "else" => self.close_branch(),
            "endif" => {
                let here = self.here()?;
                let Some(Block::If { jf, ends }) = self.blocks.pop() else {
                    return Err("`EndIf` without `If`".into());
                };
                let f = self.code()?;
                if let Some(j) = jf {
                    patch(&mut f.code, j, here);
                }
                for e in ends {
                    patch(&mut f.code, e, here);
                }
                Ok(())
            }
            "for" => {
                let Some(Tok::Id(var)) = rest.first().cloned() else {
                    return Err("expected a loop variable".into());
                };
                let mut p = Ep { t: rest, i: 1 };
                p.expect("=")?;
                let from = p.expr()?;
                if !p.kw("to") {
                    return Err("expected `To`".into());
                }
                let to = p.expr()?;
                let step = if p.kw("step") {
                    p.expr()?
                } else {
                    Ex::Num(1.0)
                };
                p.done()?;
                self.emit(Ins::Set(var.clone(), from))?;
                let head = self.here()?;
                let test = self.emit(Ins::ForTest {
                    var: var.clone(),
                    to,
                    step: step.clone(),
                    exit: 0,
                })?;
                self.blocks.push(Block::For {
                    head,
                    test,
                    var,
                    step,
                });
                Ok(())
            }
            "next" => {
                let Some(Block::For {
                    head,
                    test,
                    var,
                    step,
                }) = self.blocks.pop()
                else {
                    return Err("`Next` without `For`".into());
                };
                self.emit(Ins::Set(
                    var.clone(),
                    Ex::Bin(Op::Add, Box::new(Ex::Var(var)), Box::new(step)),
                ))?;
                self.emit(Ins::Jmp(head))?;
                let here = self.here()?;
                patch(&mut self.code()?.code, test, here);
                Ok(())
            }
            "goto" => match rest.first() {
                Some(Tok::Id(l)) => {
                    self.emit(Ins::GoTo(l.clone()))?;
                    Ok(())
                }
                _ => Err("expected a label".into()),
            },
            "exit" => {
                self.emit(Ins::Exit)?;
                Ok(())
            }
            "call" => {
                let Some(Tok::Id(name)) = rest.first().cloned() else {
                    return Err("expected a function name".into());
                };
                let mut p = Ep { t: rest, i: 1 };
                let args = if p.sym("(") { p.args()? } else { Vec::new() };
                p.done()?;
                self.emit(Ins::Call(name, args))?;
                Ok(())
            }
            "speed" | "speeds" => {
                let e = p.expr()?;
                p.done()?;
                self.emit(if kw == "speed" {
                    Ins::Speed(e)
                } else {
                    Ins::SpeedS(e)
                })?;
                Ok(())
            }
            "cp" => {
                let on = if p.kw("on") {
                    true
                } else if p.kw("off") {
                    false
                } else {
                    return Err("expected `On` or `Off`".into());
                };
                self.emit(Ins::Cp(on))?;
                Ok(())
            }
            "go" | "move" => {
                let r = p.pref()?;
                p.done()?;
                self.emit(if kw == "go" { Ins::Go(r) } else { Ins::Move(r) })?;
                Ok(())
            }
            "arc" => {
                let a = p.pref()?;
                p.expect(",")?;
                let b = p.pref()?;
                p.done()?;
                self.emit(Ins::Arc(a, b))?;
                Ok(())
            }
            "on" | "off" => {
                let e = p.expr()?;
                p.done()?;
                self.emit(if kw == "on" { Ins::On(e) } else { Ins::Off(e) })?;
                Ok(())
            }
            "wait" => {
                let e = p.expr()?;
                let timeout = if p.sym(",") { Some(p.expr()?) } else { None };
                p.done()?;
                if e.is_cond() {
                    self.emit(Ins::WaitCond(e, timeout))?;
                } else if timeout.is_none() {
                    self.emit(Ins::Wait(e))?;
                } else {
                    return Err("`Wait t, d` needs a condition".into());
                }
                Ok(())
            }
            "error" => {
                let e = p.expr()?;
                p.done()?;
                self.emit(Ins::Error(e))?;
                Ok(())
            }
            "onerr" => {
                if !p.kw("goto") {
                    return Err("expected `GoTo`".into());
                }
                let Some(Tok::Id(l)) = p.peek().cloned() else {
                    return Err("expected a label".into());
                };
                self.emit(Ins::OnErr(l))?;
                Ok(())
            }
            "quit" | "halt" => {
                self.emit(Ins::Quit)?;
                Ok(())
            }
            "local" | "tlset" => {
                let k = p.expr()?;
                let Ex::Num(k) = k else {
                    return Err("expected a number".into());
                };
                p.expect(",")?;
                if !p.kw("xy") {
                    return Err("expected `XY(..)`".into());
                }
                p.expect("(")?;
                let args = p.args()?;
                p.done()?;
                let k = k as u32;
                self.emit(if kw == "local" {
                    Ins::Local(k, args)
                } else {
                    Ins::TlSet(k, args)
                })?;
                Ok(())
            }
            "tool" => {
                let Ex::Num(k) = p.expr()? else {
                    return Err("expected a number".into());
                };
                self.emit(Ins::Tool(k as u32))?;
                Ok(())
            }
            "weight" => {
                let e = p.expr()?;
                p.done()?;
                self.emit(Ins::Weight(e))?;
                Ok(())
            }
            "plabel" => {
                let Ex::Num(n) = p.expr()? else {
                    return Err("expected a point number".into());
                };
                p.expect(",")?;
                let Some(Tok::Str(s)) = p.peek().cloned() else {
                    return Err("expected a label text".into());
                };
                self.emit(Ins::PLabel(n as u32, s))?;
                Ok(())
            }
            _ => self.assignment(kw, rest),
        }
    }

    fn assignment(&mut self, name: &str, rest: &[Tok]) -> Result<(), String> {
        let mut p = Ep { t: rest, i: 0 };
        if rest.first() == Some(&Tok::Sym(":")) && rest.len() == 1 {
            let f = self.code()?;
            let at = f.code.len();
            f.labels.push((name.to_string(), at));
            return Ok(());
        }
        p.expect("=")
            .map_err(|_| format!("unknown statement `{name}`"))?;
        if let Some(n) = point_number(name) {
            let def = if p.kw("ja") {
                p.expect("(")?;
                PointDef::Ja(p.args()?)
            } else if p.kw("xy") {
                p.expect("(")?;
                let a = p.args()?;
                let mut local = None;
                if p.sym("/") {
                    let Some(Tok::Id(l)) = p.peek().cloned() else {
                        return Err("expected a local such as `/L1`".into());
                    };
                    let k = l
                        .strip_prefix('l')
                        .and_then(|d| d.parse::<u32>().ok())
                        .ok_or_else(|| format!("`{l}` is not a local"))?;
                    p.i += 1;
                    local = Some(k);
                }
                PointDef::Xy(a, local)
            } else {
                PointDef::Copy(p.pref()?)
            };
            p.done()?;
            self.emit(Ins::SetPoint(n, def))?;
            return Ok(());
        }
        let e = p.expr()?;
        p.done()?;
        self.emit(Ins::Set(name.to_string(), e))?;
        Ok(())
    }

    fn cond_before_then(rest: &[Tok]) -> Result<Ex, String> {
        let then = rest
            .iter()
            .position(|t| matches!(t, Tok::Id(x) if x == "then"))
            .ok_or("expected `Then`")?;
        let mut p = Ep {
            t: &rest[..then],
            i: 0,
        };
        let c = p.expr()?;
        p.done()?;
        Ok(c)
    }

    fn if_stmt(&mut self, rest: &[Tok]) -> Result<(), String> {
        let then = rest
            .iter()
            .position(|t| matches!(t, Tok::Id(x) if x == "then"))
            .ok_or("expected `Then`")?;
        let c = Self::cond_before_then(rest)?;
        let after = &rest[then + 1..];
        let jf = self.emit(Ins::JmpIfNot(c, 0))?;
        if after.is_empty() {
            self.blocks.push(Block::If {
                jf: Some(jf),
                ends: Vec::new(),
            });
            return Ok(());
        }
        self.statement(after)?;
        let here = self.here()?;
        patch(&mut self.code()?.code, jf, here);
        Ok(())
    }

    /// Ends the body of a branch of an `If`: jumps to the end and lets the failed
    /// condition of the branch continue after that jump.
    fn close_branch(&mut self) -> Result<(), String> {
        let j = self.emit(Ins::Jmp(0))?;
        let here = self.here()?;
        let Some(Block::If { jf, ends }) = self.blocks.last_mut() else {
            return Err("`Else` without `If`".into());
        };
        ends.push(j);
        let old = jf.take();
        if let Some(o) = old {
            patch(&mut self.code()?.code, o, here);
        }
        Ok(())
    }
}

/// Parses a program.
///
/// # Errors
/// The first line that is not in the subset.
pub fn parse(src: &str) -> Result<Program, ParseError> {
    let mut b = Builder {
        funcs: Vec::new(),
        globals: Vec::new(),
        cur: None,
        blocks: Vec::new(),
    };
    for (i, raw) in src.lines().enumerate() {
        let err = |message: String| ParseError {
            line: i + 1,
            message,
        };
        let toks = lex(raw).map_err(err)?;
        b.statement(&toks).map_err(err)?;
    }
    if b.cur.is_some() {
        return Err(ParseError {
            line: src.lines().count(),
            message: "missing `Fend`".into(),
        });
    }
    Ok(Program {
        funcs: b.funcs,
        globals: b.globals,
    })
}
