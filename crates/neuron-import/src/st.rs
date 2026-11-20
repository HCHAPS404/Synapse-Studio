//! Structured Text to a `plc` module.
//!
//! The importer reads what `neuron emit st` and `neuron emit scl` write, and common
//! hand-written programs. It recognizes the patterns of the emitter (shadow input
//! variables, end-of-scan block calls, the safe-state block, the `CASE` machine of a
//! sequence) and turns them back into Neuron; everything else that has a Neuron
//! equivalent is translated statement by statement, and the rest is `NE1002`.

use crate::util::{dur, ident, module_name, real, syntax};
use neuron_emit::{Class, Diag, Fidelity};
use neuron_st::ast::{Arg, Expr, Init, IntTy, Pou, PouKind, Section, Stmt, Ty};
use std::collections::{BTreeMap, BTreeSet};

// Precedence levels of Neuron expressions (spec 04, 4.6).
const OR: u8 = 4;
const AND: u8 = 5;
const NOT: u8 = 6;
const CMP: u8 = 7;
const BXOR: u8 = 10;
const ADD: u8 = 13;
const MUL: u8 = 14;
const NEG: u8 = 15;
const POW: u8 = 16;
const POSTFIX: u8 = 17;
const ATOM: u8 = 20;

fn par(t: String, p: u8, min: u8) -> String {
    if p < min {
        format!("({t})")
    } else {
        t
    }
}

/// One step of a Sequential Function Chart, already read from PLCopen XML.
#[derive(Debug, Clone)]
pub(crate) struct SfcStep {
    pub name: String,
    pub actions: String,
    pub exit: SfcExit,
}

/// How a step is left.
#[derive(Debug, Clone)]
pub(crate) enum SfcExit {
    /// Nothing follows.
    None,
    /// A transition: condition, then a jump or the next step.
    Transition { cond: SfcCond, jump: Option<String> },
    /// A divergence with a condition and a time-out branch.
    Timeout {
        cond: String,
        ns: i64,
        target: Option<String>,
    },
}

/// Condition of a transition.
#[derive(Debug, Clone)]
pub(crate) enum SfcCond {
    True,
    Elapsed(i64),
    Text(String),
}

/// What the importer reads.
pub(crate) struct Input<'a> {
    pub file: &'a str,
    pub name: &'a str,
    pub text: &'a str,
    /// Offset of `text` inside `file` (for diagnostics).
    pub base: usize,
    pub scan_ns: Option<i64>,
    pub tool: &'static str,
    pub sfc: Option<Vec<SfcStep>>,
}

/// What the importer returns.
pub(crate) struct Out {
    pub source: String,
    pub fid: Fidelity,
    pub diags: Vec<Diag>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Ton,
    Tof,
    Tp,
    Ctu,
    Ctd,
    Ctud,
    RTrig,
    FTrig,
    Sr,
    Rs,
    Hysteresis,
    /// A helper block of `NeuronLib` that the importer does not read back.
    Helper,
}

type Field = (&'static str, &'static str, bool);

impl Kind {
    fn from_type(t: &str) -> Option<Kind> {
        let u = t.to_ascii_uppercase();
        let base = ["_TIME", "_INT", "_DINT"]
            .iter()
            .find_map(|s| u.strip_suffix(s))
            .unwrap_or(&u);
        Some(match base {
            "TON" => Kind::Ton,
            "TOF" => Kind::Tof,
            "TP" => Kind::Tp,
            "CTU" => Kind::Ctu,
            "CTD" => Kind::Ctd,
            "CTUD" => Kind::Ctud,
            "R_TRIG" => Kind::RTrig,
            "F_TRIG" => Kind::FTrig,
            "SR" => Kind::Sr,
            "RS" => Kind::Rs,
            "NEURON_HYSTERESIS" => Kind::Hysteresis,
            b if b.starts_with("NEURON_") => Kind::Helper,
            _ => return None,
        })
    }

    fn neuron(self) -> &'static str {
        match self {
            Kind::Ton => "ton",
            Kind::Tof => "tof",
            Kind::Tp => "tp",
            Kind::Ctu => "ctu",
            Kind::Ctd => "ctd",
            Kind::Ctud => "ctud",
            Kind::RTrig => "r_trig",
            Kind::FTrig => "f_trig",
            Kind::Sr => "sr",
            Kind::Rs => "rs",
            Kind::Hysteresis => "hysteresis",
            Kind::Helper => "helper",
        }
    }

    fn fields(self) -> &'static [Field] {
        match self {
            Kind::Ton | Kind::Tof | Kind::Tp => &[
                ("input", "IN", true),
                ("q", "Q", false),
                ("et", "ET", false),
            ],
            Kind::Ctu => &[
                ("input", "CU", true),
                ("reset", "R", true),
                ("q", "Q", false),
                ("cv", "CV", false),
            ],
            Kind::Ctd => &[
                ("input", "CD", true),
                ("load", "LD", true),
                ("q", "Q", false),
                ("cv", "CV", false),
            ],
            Kind::Ctud => &[
                ("input", "CU", true),
                ("down", "CD", true),
                ("reset", "R", true),
                ("load", "LD", true),
                ("qu", "QU", false),
                ("qd", "QD", false),
                ("cv", "CV", false),
            ],
            Kind::RTrig | Kind::FTrig => &[("input", "CLK", true), ("q", "Q", false)],
            Kind::Sr => &[
                ("set", "S1", true),
                ("reset", "R", true),
                ("q", "Q1", false),
            ],
            Kind::Rs => &[
                ("set", "S", true),
                ("reset", "R1", true),
                ("q", "Q1", false),
            ],
            Kind::Hysteresis => &[("q", "Q", false)],
            Kind::Helper => &[],
        }
    }

    fn timer(self) -> bool {
        matches!(self, Kind::Ton | Kind::Tof | Kind::Tp)
    }
}

#[derive(Clone, PartialEq, Debug)]
enum Role {
    Input,
    Output,
    Memory,
    Block(Kind),
    /// Generated by the Neuron emitter (`neuron_fault`, `neuron_seq0_pc`).
    State,
    /// The shadow variable of an input of a block: instance (lower case) and Neuron field.
    Shadow(String, &'static str),
}

#[derive(Clone, Debug)]
struct Var {
    name: String,
    ty: Ty,
    role: Role,
    at: Option<String>,
    retain: bool,
    init: Option<Init>,
}

struct Comment {
    start: usize,
    end: usize,
    body: String,
}

/// `(* ... *)` comments with their offsets; strings and `//` comments are skipped.
fn comments(text: &str) -> Vec<Comment> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"(*") {
            let end = text[i + 2..].find("*)").map_or(b.len(), |e| i + 2 + e + 2);
            let body_end = end.saturating_sub(2).max(i + 2);
            out.push(Comment {
                start: i,
                end,
                body: text[i + 2..body_end].trim().to_string(),
            });
            i = end;
        } else if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i] == b'\'' {
            i += 1;
            while i < b.len() && b[i] != b'\'' {
                i += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Offset just after the statement that starts at `pos`.
fn stmt_end(text: &str, pos: usize) -> usize {
    let b = text.as_bytes();
    let mut i = pos.min(b.len());
    let mut depth: i32 = 0;
    while i < b.len() {
        let c = b[i];
        if b[i..].starts_with(b"(*") {
            i = text[i + 2..].find("*)").map_or(b.len(), |e| i + 2 + e + 2);
            continue;
        }
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != b'\'' {
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == b';' && depth <= 0 {
            return i + 1;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            match text[s..i].to_ascii_uppercase().as_str() {
                "IF" | "CASE" | "WHILE" | "FOR" | "REPEAT" => depth += 1,
                "END_IF" | "END_CASE" | "END_WHILE" | "END_FOR" | "END_REPEAT" => {
                    depth -= 1;
                    if depth <= 0 {
                        let mut j = i;
                        while j < b.len() && b[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        return if j < b.len() && b[j] == b';' {
                            j + 1
                        } else {
                            i
                        };
                    }
                }
                _ => {}
            }
            continue;
        }
        i += 1;
    }
    b.len()
}

fn neuron_ty(t: &Ty) -> Option<&'static str> {
    Some(match t {
        Ty::Bool => "bool",
        Ty::Int(IntTy {
            bits: 8,
            signed: true,
        }) => "i8",
        Ty::Int(IntTy {
            bits: 16,
            signed: true,
        }) => "i16",
        Ty::Int(IntTy {
            bits: 32,
            signed: true,
        }) => "i32",
        Ty::Int(IntTy {
            bits: 64,
            signed: true,
        }) => "i64",
        Ty::Int(IntTy {
            bits: 8,
            signed: false,
        }) => "u8",
        Ty::Int(IntTy {
            bits: 16,
            signed: false,
        }) => "u16",
        Ty::Int(IntTy {
            bits: 32,
            signed: false,
        }) => "u32",
        Ty::Int(IntTy {
            bits: 64,
            signed: false,
        }) => "u64",
        Ty::Real(false) => "f32",
        Ty::Real(true) => "f64",
        _ => return None,
    })
}

/// `%IX0.0` -> `%I0.0`; word addresses are kept.
fn neuron_addr(a: &str) -> String {
    let u = a.to_ascii_uppercase();
    let b = u.as_bytes();
    if b.len() > 3 && b[0] == b'%' && b[2] == b'X' {
        format!("%{}{}", b[1] as char, &u[3..])
    } else {
        u
    }
}

fn is_cmp(op: &str) -> bool {
    matches!(op, "=" | "<>" | "<" | ">" | "<=" | ">=")
}

/// A line of Neuron source, relative to the block it belongs to.
type Lines = Vec<String>;

fn indent(lines: Lines, by: &str) -> Lines {
    lines.into_iter().map(|l| format!("{by}{l}")).collect()
}

fn has_code(lines: &[String]) -> bool {
    lines.iter().any(|l| !l.trim_start().starts_with('#'))
}

enum Item {
    Rung(String, Lines),
    Seq(Lines),
}

enum SeqItem {
    Step(String),
    Lines(Lines),
    Until {
        cond: String,
        timeout: Option<(i64, Option<String>)>,
    },
    Wait(i64),
    Jump(String),
}

struct Conv<'a> {
    file: &'a str,
    text: &'a str,
    base: usize,
    comments: Vec<Comment>,
    vars: Vec<Var>,
    index: BTreeMap<String, usize>,
    params: BTreeMap<String, Vec<(&'static str, String)>>,
    pending: BTreeMap<String, String>,
    loop_vars: BTreeSet<String>,
    diags: Vec<Diag>,
    fid: Fidelity,
    cur: usize,
    safe: Option<Lines>,
    reset: Option<String>,
}

impl<'a> Conv<'a> {
    fn span(&self, pos: usize) -> [usize; 2] {
        [self.base + pos, self.base + stmt_end(self.text, pos)]
    }

    fn unsup(&mut self, what: &str) {
        let sp = self.span(self.cur);
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

    fn not_imported(&self, what: &str) -> Lines {
        vec![format!("# not imported (NE1002): {what}")]
    }

    fn look(&self, name: &str) -> Option<&Var> {
        self.index
            .get(&name.to_ascii_lowercase())
            .map(|&i| &self.vars[i])
    }

    // ---- expressions -----------------------------------------------------------------

    fn is_bool(&self, e: &Expr) -> bool {
        match e {
            Expr::Bool(_) => true,
            Expr::Unary(op, _) => op == "NOT",
            Expr::Binary(op, a, b) => {
                is_cmp(op)
                    || (matches!(op.as_str(), "AND" | "OR" | "XOR")
                        && self.is_bool(a)
                        && self.is_bool(b))
            }
            Expr::Var(n) => self
                .look(n)
                .is_some_and(|v| v.ty == Ty::Bool || matches!(v.role, Role::Shadow(..))),
            Expr::Field(_, f) => {
                matches!(f.to_ascii_uppercase().as_str(), "Q" | "QU" | "QD" | "Q1")
            }
            _ => false,
        }
    }

    fn ex(&mut self, e: &Expr) -> Option<(String, u8)> {
        Some(match e {
            Expr::Bool(b) => (b.to_string(), ATOM),
            Expr::Int(v) => (v.to_string(), if *v < 0 { NEG } else { ATOM }),
            Expr::Real(v) => (real(*v), if *v < 0.0 { NEG } else { ATOM }),
            Expr::Time(ns) => (dur(*ns), if *ns < 0 { NEG } else { ATOM }),
            Expr::Str(_) => return self.fail("a string"),
            Expr::Var(n) => self.var_ref(n)?,
            Expr::Field(base, f) => self.field_ref(base, f)?,
            Expr::Index(..) => return self.fail("an array element"),
            Expr::Unary(op, x) => {
                let (t, p) = self.ex(x)?;
                if op == "NOT" {
                    (format!("not {}", par(t, p, NOT)), NOT)
                } else {
                    (format!("-{}", par(t, p, NEG)), NEG)
                }
            }
            Expr::Binary(op, a, b) => self.binary(op, a, b)?,
            Expr::Call(name, args) => self.call_expr(name, args)?,
        })
    }

    fn var_ref(&mut self, n: &str) -> Option<(String, u8)> {
        match self.look(n).map(|v| (v.role.clone(), v.name.clone())) {
            Some((Role::Shadow(inst, field), _)) => {
                let name = self.look(&inst).map_or(inst.clone(), |v| v.name.clone());
                Some((format!("{name}.{field}"), ATOM))
            }
            Some((Role::State, name)) => {
                self.fail(&format!("the generated variable `{name}` (a state machine or `when` rule of the Neuron emitter)"))
            }
            Some((Role::Block(_), name)) => {
                self.fail(&format!("using the function block `{name}` as a value"))
            }
            Some((_, name)) => Some((ident(&name), ATOM)),
            None => Some((ident(n), ATOM)),
        }
    }

    fn field_ref(&mut self, base: &Expr, f: &str) -> Option<(String, u8)> {
        let Expr::Var(inst) = base else {
            return self.fail(&format!("the field `.{f}` of a structure"));
        };
        let Some((Role::Block(kind), name)) =
            self.look(inst).map(|v| (v.role.clone(), v.name.clone()))
        else {
            return self.fail(&format!("the field `{inst}.{f}`"));
        };
        let up = f.to_ascii_uppercase();
        let Some((nf, _, _)) = kind.fields().iter().find(|x| x.1 == up) else {
            return self.fail(&format!("the field `{inst}.{f}`"));
        };
        let key = inst.to_ascii_lowercase();
        if kind == Kind::Hysteresis {
            if let Some(x) = self.pending.remove(&key) {
                return Some((format!("{name}.step({x})"), ATOM));
            }
        }
        Some((format!("{name}.{nf}"), ATOM))
    }

    fn binary(&mut self, op: &str, a: &Expr, b: &Expr) -> Option<(String, u8)> {
        let (l, lp) = self.ex(a)?;
        let (r, rp) = self.ex(b)?;
        Some(match op {
            "OR" => (format!("{} or {}", par(l, lp, OR), par(r, rp, AND)), OR),
            "AND" => (format!("{} and {}", par(l, lp, AND), par(r, rp, NOT)), AND),
            "XOR" => {
                if self.is_bool(a) && self.is_bool(b) {
                    (
                        format!("{} != {}", par(l, lp, CMP + 1), par(r, rp, CMP + 1)),
                        CMP,
                    )
                } else {
                    (
                        format!("{} ^ {}", par(l, lp, BXOR), par(r, rp, BXOR + 1)),
                        BXOR,
                    )
                }
            }
            "=" | "<>" | "<" | ">" | "<=" | ">=" => {
                let o = match op {
                    "=" => "==",
                    "<>" => "!=",
                    other => other,
                };
                (
                    format!("{} {o} {}", par(l, lp, CMP + 1), par(r, rp, CMP + 1)),
                    CMP,
                )
            }
            "+" | "-" => (
                format!("{} {op} {}", par(l, lp, ADD), par(r, rp, ADD + 1)),
                ADD,
            ),
            "*" | "/" | "MOD" => {
                let o = if op == "MOD" { "%" } else { op };
                (
                    format!("{} {o} {}", par(l, lp, MUL), par(r, rp, MUL + 1)),
                    MUL,
                )
            }
            "**" => (
                format!("{} ** {}", par(l, lp, POW + 1), par(r, rp, POW)),
                POW,
            ),
            other => return self.fail(&format!("the operator `{other}`")),
        })
    }

    fn call_expr(&mut self, name: &str, args: &[Arg]) -> Option<(String, u8)> {
        let up = name.to_ascii_uppercase();
        let mut xs = Vec::new();
        for a in args {
            xs.push(self.ex(&a.value)?.0);
        }
        let plain = |n: &str, xs: &[String]| Some((format!("{n}({})", xs.join(", ")), ATOM));
        match up.as_str() {
            "ABS" | "MIN" | "MAX" | "SQRT" | "SIN" | "COS" | "TAN" | "ASIN" | "ACOS" | "ATAN"
            | "EXP" => plain(&up.to_ascii_lowercase(), &xs),
            "LN" => plain("log", &xs),
            "LOG" => plain("log10", &xs),
            "EXPT" => plain("pow", &xs),
            "LIMIT" if xs.len() == 3 => {
                plain("clamp", &[xs[1].clone(), xs[0].clone(), xs[2].clone()])
            }
            "SEL" if xs.len() == 3 => Some((
                format!("(if {} then {} else {})", xs[0], xs[2], xs[1]),
                ATOM,
            )),
            _ => {
                let target = up
                    .rsplit_once("_TO_")
                    .map(|x| x.1)
                    .or_else(|| up.strip_prefix("TO_"));
                let ty = target.and_then(|t| match t {
                    "BOOL" => Some("bool"),
                    "SINT" => Some("i8"),
                    "INT" => Some("i16"),
                    "DINT" => Some("i32"),
                    "LINT" => Some("i64"),
                    "USINT" | "BYTE" => Some("u8"),
                    "UINT" | "WORD" => Some("u16"),
                    "UDINT" | "DWORD" => Some("u32"),
                    "ULINT" | "LWORD" => Some("u64"),
                    "REAL" => Some("f32"),
                    "LREAL" => Some("f64"),
                    _ => None,
                });
                match (ty, args.len()) {
                    (Some(t), 1) => {
                        let (x, p) = self.ex(&args[0].value)?;
                        Some((format!("{} as {t}", par(x, p, POSTFIX)), POSTFIX))
                    }
                    _ => self.fail(&format!("the call of `{name}`")),
                }
            }
        }
    }

    // ---- statements ------------------------------------------------------------------

    fn place(&mut self, p: &Expr) -> Option<String> {
        match p {
            Expr::Var(n) => match self.look(n).map(|v| (v.role.clone(), v.name.clone())) {
                Some((Role::Shadow(inst, field), _)) => {
                    let name = self.look(&inst).map_or(inst.clone(), |v| v.name.clone());
                    Some(format!("{name}.{field}"))
                }
                Some((Role::State, name)) => {
                    self.fail(&format!("an assignment to the generated variable `{name}`"))
                }
                Some((Role::Block(_), name)) => {
                    self.fail(&format!("an assignment to the function block `{name}`"))
                }
                Some((_, name)) => Some(ident(&name)),
                None => Some(ident(n)),
            },
            Expr::Field(base, f) => {
                if let Expr::Var(inst) = &**base {
                    if let Some((Role::Block(kind), name)) =
                        self.look(inst).map(|v| (v.role.clone(), v.name.clone()))
                    {
                        let up = f.to_ascii_uppercase();
                        if let Some((nf, _, true)) = kind.fields().iter().find(|x| x.1 == up) {
                            return Some(format!("{name}.{nf}"));
                        }
                    }
                }
                self.fail(&format!("an assignment to the field `.{f}`"))
            }
            _ => self.fail("an assignment to an array element"),
        }
    }

    fn assign(&mut self, place: &Expr, value: &Expr, when: Option<&str>) -> Lines {
        let Some(target) = self.place(place) else {
            return self.not_imported("assignment");
        };
        // `x := x + e` is `x += e`.
        let compound = match (place, value) {
            (Expr::Var(a), Expr::Binary(op, l, r))
                if matches!(op.as_str(), "+" | "-" | "*" | "/" | "MOD")
                    && matches!(&**l, Expr::Var(b) if b.eq_ignore_ascii_case(a))
                    && !matches!(self.look(a).map(|v| &v.role), Some(Role::Shadow(..))) =>
            {
                Some((op.as_str(), &**r))
            }
            _ => None,
        };
        let line = if let Some((op, rhs)) = compound {
            let Some((r, _)) = self.ex(rhs) else {
                return self.not_imported("assignment");
            };
            let o = if op == "MOD" { "%" } else { op };
            format!("{target} {o}= {r}")
        } else {
            let Some((r, _)) = self.ex(value) else {
                return self.not_imported("assignment");
            };
            format!("{target} = {r}")
        };
        self.fid.exact();
        vec![match when {
            Some(c) => format!("{line} when {c}"),
            None => line,
        }]
    }

    /// A call of a function block instance.
    fn block_call(&mut self, inst: &str, args: &[Arg]) -> Lines {
        let Some((Role::Block(kind), name)) =
            self.look(inst).map(|v| (v.role.clone(), v.name.clone()))
        else {
            self.unsup(&format!(
                "the call of `{inst}` (a user function block or function)"
            ));
            return self.not_imported(&format!("call of {inst}"));
        };
        let key = inst.to_ascii_lowercase();
        let mut lines = Vec::new();
        if kind == Kind::Helper {
            self.unsup(&format!(
                "the helper block of `{inst}` (only `hysteresis` is read back)"
            ));
            return self.not_imported(&format!("call of {inst}"));
        }
        for a in args {
            let Some(pn) = a.name.as_ref().map(|n| n.to_ascii_uppercase()) else {
                self.unsup(&format!("a call of `{inst}` with positional arguments"));
                return self.not_imported(&format!("call of {inst}"));
            };
            if kind == Kind::Hysteresis {
                match pn.as_str() {
                    "X" => {
                        if let Some((x, _)) = self.ex(&a.value) {
                            self.pending.insert(key.clone(), x);
                        }
                    }
                    "LOW" | "HIGH" => {
                        let k = if pn == "LOW" { "low" } else { "high" };
                        if let Some((v, _)) = self.ex(&a.value) {
                            self.set_param(&key, k, v);
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if a.output {
                // `Q => y` is `y = inst.q`.
                if let Some((nf, _, false)) = kind.fields().iter().find(|x| x.1 == pn) {
                    if let Some(t) = self.place(&a.value) {
                        lines.push(format!("{t} = {name}.{nf}"));
                    }
                }
                continue;
            }
            if let Some((nf, _, true)) = kind.fields().iter().find(|x| x.1 == pn) {
                // The emitter's own shadow variable already carries the value.
                if let Expr::Var(v) = &a.value {
                    if let Some(Role::Shadow(i, f)) = self.look(v).map(|x| x.role.clone()) {
                        if i == key && f == *nf {
                            continue;
                        }
                    }
                }
                if let Some((v, _)) = self.ex(&a.value) {
                    lines.push(format!("{name}.{nf} = {v}"));
                }
            } else if pn == "PT" || pn == "PV" {
                match &a.value {
                    Expr::Time(_) | Expr::Int(_) => {
                        if let Some((v, _)) = self.ex(&a.value) {
                            self.set_param(&key, "preset", v);
                        }
                    }
                    _ => {
                        self.unsup(&format!("the preset of `{inst}` (it must be a constant)"));
                    }
                }
            }
        }
        if !lines.is_empty() {
            let sp = self.span(self.cur);
            self.fid.record(
                self.file,
                sp,
                Class::Equivalent,
                &format!("call of `{inst}`"),
                "a Neuron block is evaluated once at the end of the scan; here it was called in the middle of the program, so its outputs may be one scan later",
            );
        }
        lines
    }

    fn set_param(&mut self, inst: &str, key: &'static str, v: String) {
        let e = self.params.entry(inst.to_string()).or_default();
        if !e.iter().any(|(k, _)| *k == key) {
            e.push((key, v));
        }
    }

    fn if_stmt(&mut self, branches: &[(Expr, Vec<Stmt>)], other: &Option<Vec<Stmt>>) -> Lines {
        let pos = self.cur;
        // `IF c THEN x := TRUE; END_IF;` is `set x when c`; one assignment is `x = e when c`.
        if branches.len() == 1 && other.is_none() && branches[0].1.len() == 1 {
            if let Stmt::Assign(place, value, p) = &branches[0].1[0] {
                let Some((c, _)) = self.ex(&branches[0].0) else {
                    return self.not_imported("condition");
                };
                self.cur = *p;
                let lines = match (place, value) {
                    (Expr::Var(x), Expr::Bool(b))
                        if !matches!(
                            self.look(x).map(|v| &v.role),
                            Some(Role::Shadow(..) | Role::State | Role::Block(_))
                        ) =>
                    {
                        self.fid.exact();
                        vec![format!(
                            "{} {} when {c}",
                            if *b { "set" } else { "reset" },
                            ident(x)
                        )]
                    }
                    _ => self.assign(place, value, Some(&c)),
                };
                self.cur = pos;
                return lines;
            }
        }
        let mut out = Vec::new();
        for (k, (cond, body)) in branches.iter().enumerate() {
            let c = self.ex(cond).map(|x| x.0);
            let head = match &c {
                Some(c) => format!("{} {c}:", if k == 0 { "if" } else { "elif" }),
                None => format!(
                    "{} false:  # condition not imported",
                    if k == 0 { "if" } else { "elif" }
                ),
            };
            out.push(head);
            out.extend(indent(self.block(body), "  "));
        }
        if let Some(body) = other {
            out.push("else:".into());
            out.extend(indent(self.block(body), "  "));
        }
        self.fid.exact();
        out
    }

    fn block(&mut self, body: &[Stmt]) -> Lines {
        let mut lines = self.stmts(body);
        if !has_code(&lines) {
            lines.push("pass".into());
        }
        lines
    }

    fn stmt(&mut self, s: &Stmt) -> Lines {
        self.cur = s.pos();
        match s {
            Stmt::Assign(p, v, _) => self.assign(p, v, None),
            Stmt::Call(Expr::Var(inst), args, _) => self.block_call(inst, args),
            Stmt::Call(..) => {
                self.unsup("this call");
                self.not_imported("call")
            }
            Stmt::If(b, o, _) => self.if_stmt(b, o),
            Stmt::For(v, a, b, by, body, _) => {
                let (Expr::Int(lo), Expr::Int(hi), None) = (a, b, by) else {
                    self.unsup("a `FOR` loop whose bounds are not constants, or with `BY`");
                    return self.not_imported("FOR loop");
                };
                self.loop_vars.insert(v.to_ascii_lowercase());
                let mut out = vec![format!("for {} in {}..{}:", ident(v), lo, hi + 1)];
                out.extend(indent(self.block(body), "  "));
                self.fid.exact();
                out
            }
            Stmt::Case(..) => {
                self.unsup("a `CASE` statement that is not the state machine of a sequence");
                self.not_imported("CASE statement")
            }
            Stmt::While(..) => {
                self.unsup("a `WHILE` loop (Neuron loops have constant bounds)");
                self.not_imported("WHILE loop")
            }
            Stmt::Repeat(..) => {
                self.unsup("a `REPEAT` loop (Neuron loops have constant bounds)");
                self.not_imported("REPEAT loop")
            }
            Stmt::Exit(_) => {
                self.unsup("`EXIT`");
                self.not_imported("EXIT")
            }
            Stmt::Return(_) => {
                self.unsup("`RETURN` outside the safe-state block");
                self.not_imported("RETURN")
            }
        }
    }

    /// The name in a `(* fault Name *)` comment right before `pos`.
    fn fault_comment(&self, pos: usize) -> Option<String> {
        let c = self.comments.iter().rev().find(|c| c.end <= pos)?;
        let gap = self.text.get(c.end..pos)?;
        if !gap.trim().is_empty() {
            return None;
        }
        c.body.strip_prefix("fault ").map(|n| ident(n.trim()))
    }

    fn stmts(&mut self, body: &[Stmt]) -> Lines {
        let mut out = Vec::new();
        let mut i = 0;
        while i < body.len() {
            let s = &body[i];
            // `neuron_fault := TRUE; neuron_fault_code := n; <safe state>; RETURN;` is `fault F`.
            if let Stmt::Assign(Expr::Var(v), Expr::Bool(true), pos) = s {
                if v.eq_ignore_ascii_case("neuron_fault") {
                    let code = match body.get(i + 1) {
                        Some(Stmt::Assign(Expr::Var(c), Expr::Int(n), _))
                            if c.eq_ignore_ascii_case("neuron_fault_code") =>
                        {
                            Some(*n)
                        }
                        _ => None,
                    };
                    let name = self
                        .fault_comment(*pos)
                        .unwrap_or_else(|| format!("Fault{}", code.unwrap_or(0)));
                    out.push(format!("fault {name}"));
                    self.fid.exact();
                    while i < body.len() && !matches!(body[i], Stmt::Return(_)) {
                        i += 1;
                    }
                    i += 1;
                    continue;
                }
            }
            out.extend(self.stmt(s));
            i += 1;
        }
        out
    }

    // ---- generated patterns ----------------------------------------------------------

    /// `IF neuron_fault AND c THEN neuron_fault := FALSE; ... END_IF;` -> the reset condition.
    fn reset_of(&mut self, s: &Stmt) -> Option<String> {
        let Stmt::If(br, None, _) = s else {
            return None;
        };
        let [(Expr::Binary(op, l, r), body)] = br.as_slice() else {
            return None;
        };
        if op != "AND" || !matches!(&**l, Expr::Var(n) if n.eq_ignore_ascii_case("neuron_fault")) {
            return None;
        }
        if !matches!(body.first(), Some(Stmt::Assign(Expr::Var(n), Expr::Bool(false), _)) if n.eq_ignore_ascii_case("neuron_fault"))
        {
            return None;
        }
        self.ex(r).map(|x| x.0)
    }

    /// `IF neuron_fault THEN <safe state>; RETURN; END_IF;` -> the safe-state lines.
    fn safe_of(&mut self, s: &Stmt) -> Option<Lines> {
        let Stmt::If(br, None, _) = s else {
            return None;
        };
        let [(Expr::Var(n), body)] = br.as_slice() else {
            return None;
        };
        if !n.eq_ignore_ascii_case("neuron_fault") || !matches!(body.last(), Some(Stmt::Return(_)))
        {
            return None;
        }
        Some(self.stmts(&body[..body.len() - 1]))
    }

    fn between(&self, a: usize, b: usize) -> Vec<&Comment> {
        self.comments
            .iter()
            .filter(|c| c.start >= a && c.end <= b)
            .collect()
    }

    fn book(&self, k: &str, v: &str) -> bool {
        let n = v.to_ascii_lowercase();
        n == format!("neuron_seq{k}_pc")
            || n == format!("neuron_seq{k}_el")
            || n == format!("neuron_seq{k}_go")
            || n == format!("neuron_seq{k}_guard")
    }

    fn is_book(&self, k: &str, s: &Stmt) -> bool {
        matches!(s, Stmt::Assign(Expr::Var(v), _, _) if self.book(k, v))
    }

    fn pc_of(&self, k: &str, body: &[Stmt]) -> Option<i64> {
        body.iter().find_map(|s| match s {
            Stmt::Assign(Expr::Var(v), Expr::Int(n), _)
                if v.eq_ignore_ascii_case(&format!("neuron_seq{k}_pc")) =>
            {
                Some(*n)
            }
            _ => None,
        })
    }

    fn elapsed(&self, k: &str, e: &Expr) -> Option<i64> {
        match e {
            Expr::Binary(op, l, r) if op == ">=" => match (&**l, &**r) {
                (Expr::Var(v), Expr::Time(ns))
                    if v.eq_ignore_ascii_case(&format!("neuron_seq{k}_el")) =>
                {
                    Some(*ns)
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// The `CASE` machine of one sequence, read back as `sequence name:`.
    fn sequence(&mut self, k: &str, name: &str, case: &Stmt) -> Option<Lines> {
        let Stmt::Case(_, arms, _, case_pos) = case else {
            return None;
        };
        let labels: Vec<i64> = arms
            .iter()
            .filter_map(|(l, _)| match l.as_slice() {
                [neuron_st::ast::CaseLabel::Value(v)] => Some(*v),
                _ => None,
            })
            .collect();
        if labels.len() != arms.len() {
            self.cur = *case_pos;
            return self.fail("the labels of a sequence state machine");
        }
        // Events of every arm: step comments and statements, in source order.
        enum Ev<'s> {
            Step(String),
            St(&'s Stmt),
        }
        let mut region_start = *case_pos;
        let mut events: Vec<Vec<Ev>> = Vec::new();
        for (_, body) in arms {
            let mut evs: Vec<(usize, Ev)> = Vec::new();
            if let Some(last) = body.last() {
                let end = stmt_end(self.text, last.pos());
                for c in self.between(region_start, end) {
                    if let Some(n) = c.body.strip_prefix("step ") {
                        evs.push((c.start, Ev::Step(ident(n.trim()))));
                    }
                }
                region_start = end;
            }
            for s in body {
                evs.push((s.pos(), Ev::St(s)));
            }
            evs.sort_by_key(|e| e.0);
            events.push(evs.into_iter().map(|e| e.1).collect());
        }
        let mut step_of: BTreeMap<i64, String> = BTreeMap::new();
        for (a, evs) in events.iter().enumerate() {
            if let Some(Ev::Step(n)) = evs.first() {
                step_of.insert(labels[a], n.clone());
            }
        }
        let mut items: Vec<SeqItem> = Vec::new();
        for (a, evs) in events.iter().enumerate() {
            let label = labels[a];
            let next = labels.get(a + 1).copied();
            let body = &arms[a].1;
            // A wait: one `IF` whose branches only move the program counter.
            if let [Stmt::If(br, None, p)] = body.as_slice() {
                if br.iter().all(|(_, b)| b.iter().all(|s| self.is_book(k, s))) && !br.is_empty() {
                    self.cur = *p;
                    let target = |this: &Self, b: &[Stmt]| -> Option<Option<String>> {
                        match this.pc_of(k, b) {
                            Some(n) if Some(n) == next => Some(None),
                            Some(n) => step_of.get(&n).cloned().map(Some),
                            None => Some(None),
                        }
                    };
                    if let Some(d) = self.elapsed(k, &br[0].0) {
                        items.push(SeqItem::Wait(d));
                        continue;
                    }
                    let cond = self.ex(&br[0].0).map(|x| x.0);
                    let Some(cond) = cond else {
                        items.push(SeqItem::Lines(self.not_imported("wait condition")));
                        continue;
                    };
                    let timeout = match br.get(1) {
                        Some((c1, b1)) => {
                            match (self.elapsed(k, c1), target(self, b1)) {
                                (Some(d), Some(t)) => Some((d, t)),
                                _ => {
                                    self.unsup("a time-out branch of a sequence that jumps to an unknown step");
                                    None
                                }
                            }
                        }
                        None => None,
                    };
                    items.push(SeqItem::Until { cond, timeout });
                    self.fid.exact();
                    continue;
                }
            }
            let mut last_pc: Option<i64> = None;
            for ev in evs {
                match ev {
                    Ev::Step(n) => items.push(SeqItem::Step(n.clone())),
                    Ev::St(s) => {
                        if self.is_book(k, s) {
                            if let Stmt::Assign(Expr::Var(v), Expr::Int(n), _) = s {
                                if v.eq_ignore_ascii_case(&format!("neuron_seq{k}_pc")) {
                                    last_pc = Some(*n);
                                }
                            }
                            continue;
                        }
                        let lines = self.stmts(std::slice::from_ref(*s));
                        items.push(SeqItem::Lines(lines));
                    }
                }
            }
            if let Some(n) = last_pc {
                if n >= 0 && Some(n) != next && n != label {
                    match step_of.get(&n) {
                        Some(t) => items.push(SeqItem::Jump(t.clone())),
                        None => {
                            self.cur = *case_pos;
                            self.unsup("a jump of a sequence to a label that is not a step");
                        }
                    }
                }
            }
        }
        // Render.
        let mut out = vec![format!("sequence {}:", ident(name))];
        let mut ind = "  ";
        let mut first = true;
        for it in items {
            match it {
                SeqItem::Step(n) => {
                    out.push(format!(
                        "  {}step {n}:",
                        if first { "initial " } else { "" }
                    ));
                    ind = "    ";
                }
                SeqItem::Lines(l) => out.extend(indent(l, ind)),
                SeqItem::Until { cond, timeout } => {
                    let t = match timeout {
                        None => String::new(),
                        Some((d, None)) => format!(" timeout {}", dur(d)),
                        Some((d, Some(s))) => format!(" timeout {} -> {s}", dur(d)),
                    };
                    out.push(format!("{ind}until {cond}{t}"));
                }
                SeqItem::Wait(d) => out.push(format!("{ind}wait {}", dur(d))),
                SeqItem::Jump(t) => out.push(format!("{ind}-> {t}")),
            }
            first = false;
        }
        let sp = self.span(*case_pos);
        self.fid.record(
            self.file,
            sp,
            Class::Equivalent,
            &format!("sequence `{name}`"),
            "the state machine ran once per scan; the sequence runs on the base tick of the interpreter, so a step may end up to two scans earlier or later than on the PLC",
        );
        Some(out)
    }

    /// A sequence written as an SFC.
    fn sfc(&mut self, name: &str, steps: &[SfcStep]) -> Lines {
        let mut out = vec![format!("sequence {}:", ident(name))];
        let mut base = String::new();
        let mut first = true;
        for (i, s) in steps.iter().enumerate() {
            let cont = !base.is_empty()
                && s.name
                    .strip_prefix(&format!("{base}_"))
                    .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()));
            if !cont {
                base = s.name.clone();
                out.push(format!(
                    "  {}step {}:",
                    if first { "initial " } else { "" },
                    ident(&s.name)
                ));
            }
            first = false;
            if !s.actions.trim().is_empty() {
                let lines = self.sfc_actions(&s.actions);
                out.extend(indent(lines, "    "));
            }
            let next = steps.get(i + 1).map(|n| n.name.clone());
            match &s.exit {
                SfcExit::None => {}
                SfcExit::Transition { cond, jump } => {
                    match cond {
                        SfcCond::True => {}
                        SfcCond::Elapsed(d) => out.push(format!("    wait {}", dur(*d))),
                        SfcCond::Text(t) => {
                            if let Some(c) = self.sfc_expr(t) {
                                out.push(format!("    until {c}"));
                            }
                        }
                    }
                    if let Some(j) = jump {
                        if Some(j) != next.as_ref() {
                            out.push(format!("    -> {}", ident(j)));
                        }
                    }
                }
                SfcExit::Timeout { cond, ns, target } => {
                    if let Some(c) = self.sfc_expr(cond) {
                        let t = match target {
                            Some(t) if Some(t) != next.as_ref() => format!(" -> {}", ident(t)),
                            _ => String::new(),
                        };
                        out.push(format!("    until {c} timeout {}{t}", dur(*ns)));
                    }
                }
            }
            self.fid.exact();
        }
        self.fid.record(
            self.file,
            [0, 0],
            Class::Equivalent,
            &format!("sequence `{name}`"),
            "steps advanced once per scan on the PLC; the sequence runs on the base tick of the interpreter, so a step may end up to two scans earlier or later",
        );
        out
    }

    fn sfc_actions(&mut self, code: &str) -> Lines {
        let src = format!("PROGRAM a\n{code}\nEND_PROGRAM\n");
        match neuron_st::parse(&src) {
            Ok(u) => {
                let body = u.pous.first().map(|p| p.body.clone()).unwrap_or_default();
                self.stmts(&body)
            }
            Err(e) => {
                self.diags.push(syntax(
                    self.file,
                    self.base,
                    1,
                    &format!("an action of the SFC: {e}"),
                ));
                Vec::new()
            }
        }
    }

    fn sfc_expr(&mut self, code: &str) -> Option<String> {
        let src = format!("PROGRAM a\nx := {code};\nEND_PROGRAM\n");
        match neuron_st::parse(&src) {
            Ok(u) => match u.pous.first().and_then(|p| p.body.first()) {
                Some(Stmt::Assign(_, e, _)) => {
                    self.cur = 0;
                    self.ex(e).map(|x| x.0)
                }
                _ => None,
            },
            Err(e) => {
                self.diags.push(syntax(
                    self.file,
                    self.base,
                    1,
                    &format!("a transition of the SFC: {e}"),
                ));
                None
            }
        }
    }
}

fn find_programs(unit: &neuron_st::ast::Unit) -> Vec<&Pou> {
    unit.pous
        .iter()
        .filter(|p| {
            matches!(p.kind, PouKind::Program | PouKind::FunctionBlock)
                && !p.name.eq_ignore_ascii_case("GlobalVars")
                && !(p.kind == PouKind::FunctionBlock
                    && p.name.to_ascii_uppercase().starts_with("NEURON_"))
                && !p.name.to_ascii_uppercase().starts_with("NEURON_CYCLIC_")
        })
        .collect()
}

/// Imports Structured Text (or SCL) text.
pub(crate) fn import(inp: Input) -> Result<Out, Vec<Diag>> {
    let unit = neuron_st::parse(inp.text).map_err(|e| {
        vec![syntax(
            inp.file,
            inp.base + e.pos,
            1,
            &format!("Structured Text: {e}"),
        )]
    })?;
    let progs = find_programs(&unit);
    let Some(pou) = progs.first().copied() else {
        return Err(vec![syntax(
            inp.file,
            inp.base,
            1,
            "Structured Text: the file has no PROGRAM or FUNCTION_BLOCK to import",
        )]);
    };
    let mut c = Conv {
        file: inp.file,
        text: inp.text,
        base: inp.base,
        comments: comments(inp.text),
        vars: Vec::new(),
        index: BTreeMap::new(),
        params: BTreeMap::new(),
        pending: BTreeMap::new(),
        loop_vars: BTreeSet::new(),
        diags: Vec::new(),
        fid: Fidelity::new(inp.tool, "neuron.plc"),
        cur: pou.pos,
        safe: None,
        reset: None,
    };
    if progs.len() > 1 {
        c.cur = progs[1].pos;
        c.unsup(&format!(
            "the {} other program units of the file",
            progs.len() - 1
        ));
    }
    // Variables.
    let globals: BTreeMap<String, &neuron_st::ast::VarDecl> = unit
        .pous
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case("GlobalVars"))
        .flat_map(|p| p.vars.iter())
        .map(|v| (v.name.to_ascii_lowercase(), v))
        .collect();
    for d in &pou.vars {
        let (ty, at, retain, init) = if d.section == Section::External {
            match globals.get(&d.name.to_ascii_lowercase()) {
                Some(g) => (g.ty.clone(), g.at.clone(), g.retain, g.init.clone()),
                None => (d.ty.clone(), None, false, None),
            }
        } else {
            (d.ty.clone(), d.at.clone(), d.retain, d.init.clone())
        };
        let lower = d.name.to_ascii_lowercase();
        let role = if let Some(k) = match &ty {
            Ty::Named(n) => Kind::from_type(n),
            _ => None,
        } {
            Role::Block(k)
        } else if lower.starts_with("neuron_") {
            Role::State
        } else if let Some(a) = &at {
            match a.as_bytes().get(1) {
                Some(b'I') => Role::Input,
                Some(b'Q') => Role::Output,
                _ => Role::Memory,
            }
        } else {
            match d.section {
                Section::Input => Role::Input,
                Section::Output => Role::Output,
                _ => Role::Memory,
            }
        };
        c.index.insert(lower, c.vars.len());
        c.vars.push(Var {
            name: d.name.clone(),
            ty,
            role,
            at,
            retain,
            init,
        });
    }
    // Shadow variables: `<instance>_<IEC input>`.
    let blocks: Vec<(String, Kind)> = c
        .vars
        .iter()
        .filter_map(|v| match v.role {
            Role::Block(k) => Some((v.name.to_ascii_lowercase(), k)),
            _ => None,
        })
        .collect();
    for v in &mut c.vars {
        if v.ty != Ty::Bool || matches!(v.role, Role::Block(_)) {
            continue;
        }
        let l = v.name.to_ascii_lowercase();
        for (inst, kind) in &blocks {
            for (nf, iec, input) in kind.fields() {
                if *input && l == format!("{inst}_{}", iec.to_ascii_lowercase()) {
                    v.role = Role::Shadow(inst.clone(), nf);
                }
            }
        }
    }
    // Presets written as initial values: `t : TON := (PT := T#3S);`.
    let inits: Vec<(String, Vec<(String, Expr)>)> = c
        .vars
        .iter()
        .filter_map(|v| match (&v.role, &v.init) {
            (Role::Block(_), Some(Init::Struct(f))) => {
                Some((v.name.to_ascii_lowercase(), f.clone()))
            }
            _ => None,
        })
        .collect();
    for (inst, fields) in inits {
        for (f, e) in fields {
            if matches!(f.to_ascii_uppercase().as_str(), "PT" | "PV") {
                if let Some((t, _)) = c.ex(&e) {
                    c.set_param(&inst, "preset", t);
                }
            }
        }
    }

    // The body.
    let body = &pou.body;
    let mut at = 0;
    let mut reset = None;
    while at < body.len() {
        c.cur = body[at].pos();
        if let Some(r) = c.reset_of(&body[at]) {
            reset = Some(r);
            at += 1;
        } else {
            break;
        }
    }
    let mut safe_found = false;
    if at < body.len() {
        c.cur = body[at].pos();
        if let Some(safe) = c.safe_of(&body[at]) {
            c.safe = Some(safe);
            safe_found = true;
            at += 1;
            c.fid.exact();
        }
    }
    if safe_found {
        c.reset = reset;
    } else {
        // Not the block of the emitter: every statement is ordinary.
        at = 0;
    }
    let mut items: Vec<Item> = Vec::new();
    let mut cur: Option<(String, Lines)> = None;
    let mut used: BTreeSet<String> = BTreeSet::new();
    let flush = |cur: &mut Option<(String, Lines)>, items: &mut Vec<Item>| {
        if let Some((n, l)) = cur.take() {
            if !l.is_empty() {
                items.push(Item::Rung(n, l));
            }
        }
    };
    let mut prev_end = pou.pos;
    while at < body.len() {
        let s = &body[at];
        let p = s.pos();
        c.cur = p;
        let cs: Vec<String> = c
            .between(prev_end, p)
            .iter()
            .map(|x| x.body.clone())
            .collect();
        for text in &cs {
            if let Some(n) = text.strip_prefix("rung ") {
                flush(&mut cur, &mut items);
                let mut n = ident(n.trim());
                while !used.insert(n.to_ascii_lowercase()) {
                    n.push('_');
                }
                cur = Some((n, Vec::new()));
            } else if text.starts_with("end of scan") {
                flush(&mut cur, &mut items);
                cur = Some(("end_of_scan".into(), Vec::new()));
            }
        }
        // The state machine of a sequence.
        if let Stmt::Assign(Expr::Var(v), Expr::Bool(true), _) = s {
            let l = v.to_ascii_lowercase();
            if let Some(k) = l
                .strip_prefix("neuron_seq")
                .and_then(|r| r.strip_suffix("_go"))
                .filter(|k| !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()))
            {
                if let (Some(Stmt::Assign(..)), Some(Stmt::While(_, wb, _))) =
                    (body.get(at + 1), body.get(at + 2))
                {
                    let name = c
                        .between(prev_end, p)
                        .iter()
                        .find_map(|x| {
                            x.body
                                .strip_prefix("sequence ")
                                .map(|n| n.trim().to_string())
                        })
                        .unwrap_or_else(|| format!("seq{k}"));
                    if let Some(case) = wb.iter().find(|s| matches!(s, Stmt::Case(..))) {
                        if let Some(lines) = c.sequence(k, &name, case) {
                            flush(&mut cur, &mut items);
                            items.push(Item::Seq(lines));
                            prev_end = stmt_end(inp.text, body[at + 2].pos());
                            at += 3;
                            continue;
                        }
                    }
                }
            }
            // The clock of a sequence state is not statement.
        }
        // `neuron_seqN_el := neuron_seqN_el + T#10MS;`
        if let Stmt::Assign(Expr::Var(v), Expr::Binary(op, l, r), _) = s {
            if v.to_ascii_lowercase().starts_with("neuron_")
                && op == "+"
                && matches!(&**l, Expr::Var(w) if w.eq_ignore_ascii_case(v))
                && matches!(&**r, Expr::Time(_))
            {
                prev_end = stmt_end(inp.text, p);
                at += 1;
                continue;
            }
        }
        let lines = c.stmts(std::slice::from_ref(s));
        if !lines.is_empty() {
            cur.get_or_insert_with(|| ("main".to_string(), Vec::new()))
                .1
                .extend(lines);
        }
        prev_end = stmt_end(inp.text, p);
        at += 1;
    }
    flush(&mut cur, &mut items);
    if let Some(steps) = &inp.sfc {
        let lines = c.sfc("main", steps);
        items.push(Item::Seq(lines));
    }

    // ---- the Neuron source ----------------------------------------------------------
    let name = module_name(
        if inp.name.is_empty() {
            &pou.name
        } else {
            inp.name
        },
        &mut c.fid,
        inp.file,
        [inp.base, inp.base],
    );
    let mut src = String::new();
    src.push_str(&format!(
        "## Imported from {} program `{}` by neuron-import.\n",
        if inp.tool == "import-scl" {
            "SCL"
        } else {
            "IEC 61131-3"
        },
        pou.name
    ));
    src.push_str(&format!("module {name} for plc\n"));
    let task = unit
        .tasks
        .iter()
        .find(|(t, _)| t.eq_ignore_ascii_case(&pou.name))
        .map(|(_, ns)| *ns)
        .filter(|ns| *ns > 0);
    let scan = match (task, inp.scan_ns) {
        (Some(ns), _) => ns,
        (None, Some(ns)) => ns,
        (None, None) => {
            c.fid.record(
                inp.file,
                [inp.base, inp.base],
                Class::Equivalent,
                "scan period",
                "the source does not say how often the program runs (no TASK interval); 10 ms is assumed, change `scan` to the real cycle",
            );
            10_000_000
        }
    };
    src.push_str(&format!("scan {}\n", dur(scan)));
    let mut sections: [(&str, Vec<String>); 3] = [
        ("inputs", Vec::new()),
        ("outputs", Vec::new()),
        ("memory", Vec::new()),
    ];
    let mut lets: Vec<String> = Vec::new();
    let mut outputs: Vec<(String, Ty)> = Vec::new();
    let vars = c.vars.clone();
    for v in &vars {
        match &v.role {
            Role::Block(k) => {
                if *k == Kind::Helper {
                    continue;
                }
                let key = v.name.to_ascii_lowercase();
                let ps = c.params.get(&key).cloned().unwrap_or_default();
                let get = |k: &str| ps.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
                let args = match k {
                    Kind::Ton | Kind::Tof | Kind::Tp => {
                        format!("preset: {}", get("preset").unwrap_or_else(|| "0 ms".into()))
                    }
                    Kind::Ctu | Kind::Ctd | Kind::Ctud => {
                        format!("preset: {}", get("preset").unwrap_or_else(|| "0".into()))
                    }
                    Kind::Hysteresis => format!(
                        "low: {}, high: {}",
                        get("low").unwrap_or_else(|| "0.0".into()),
                        get("high").unwrap_or_else(|| "0.0".into())
                    ),
                    _ => String::new(),
                };
                lets.push(format!("let {} = {}({args})", ident(&v.name), k.neuron()));
                if k.timer() {
                    c.fid.record(
                        inp.file,
                        [inp.base, inp.base],
                        Class::Equivalent,
                        &format!("timer `{}`", v.name),
                        "Neuron counts scans from the first scan in which the input is true; an IEC timer that measures wall-clock time raises Q up to one scan later",
                    );
                } else {
                    c.fid.exact();
                }
            }
            Role::State | Role::Shadow(..) => {}
            role => {
                if c.loop_vars.contains(&v.name.to_ascii_lowercase()) {
                    continue;
                }
                let Some(ty) = neuron_ty(&v.ty) else {
                    c.cur = pou.pos;
                    c.unsup(&format!(
                        "the variable `{}` (its type is not supported)",
                        v.name
                    ));
                    continue;
                };
                let init = match &v.init {
                    Some(Init::Expr(e)) => {
                        c.ex(e).map(|x| format!(" = {}", x.0)).unwrap_or_default()
                    }
                    _ => String::new(),
                };
                let at =
                    v.at.as_ref()
                        .map(|a| format!(" @ {}", neuron_addr(a)))
                        .unwrap_or_default();
                let line = format!(
                    "  {}{}: {ty}{init}{at}",
                    if v.retain { "retain " } else { "" },
                    ident(&v.name)
                );
                let k = match role {
                    Role::Input => 0,
                    Role::Output => {
                        outputs.push((ident(&v.name), v.ty.clone()));
                        1
                    }
                    _ => 2,
                };
                sections[k].1.push(line);
                c.fid.exact();
            }
        }
    }
    for (title, lines) in &sections {
        if lines.is_empty() {
            continue;
        }
        src.push_str(&format!("\n{title}:\n"));
        for l in lines {
            src.push_str(l);
            src.push('\n');
        }
    }
    if !lets.is_empty() {
        src.push('\n');
        for l in &lets {
            src.push_str(l);
            src.push('\n');
        }
    }
    for it in &items {
        src.push('\n');
        match it {
            Item::Rung(n, lines) => {
                if has_code(lines) {
                    src.push_str(&format!("rung {n}:\n"));
                    for l in lines {
                        src.push_str(&format!("  {l}\n"));
                    }
                } else {
                    for l in lines {
                        src.push_str(&format!("{l}\n"));
                    }
                }
            }
            Item::Seq(lines) => {
                for l in lines {
                    src.push_str(l);
                    src.push('\n');
                }
            }
        }
    }
    // The safe state: every output needs one.
    if !outputs.is_empty() || c.safe.is_some() {
        src.push_str("\nsafety:\n");
        if let Some(r) = &c.reset {
            src.push_str(&format!("  reset when {r}\n"));
        }
        src.push_str("  safe_state:\n");
        match &c.safe {
            Some(lines) if has_code(lines) => {
                for l in lines {
                    src.push_str(&format!("    {l}\n"));
                }
            }
            _ => {
                for (n, ty) in &outputs {
                    let zero = if *ty == Ty::Bool { "false" } else { "0" };
                    src.push_str(&format!("    {n} = {zero}\n"));
                }
                c.fid.record(
                    inp.file,
                    [inp.base, inp.base],
                    Class::Approximate,
                    "safe state",
                    "the source has no safe state; every output is set to false (or 0) when a fault is raised",
                );
            }
        }
    }
    finish(src, c.fid, c.diags, inp.file)
}

/// Formats the source and returns the result.
pub(crate) fn finish(
    src: String,
    fid: Fidelity,
    diags: Vec<Diag>,
    file: &str,
) -> Result<Out, Vec<Diag>> {
    match neuron_syntax::format::format_source(&src) {
        Ok(f) => Ok(Out {
            source: f.text,
            fid,
            diags,
        }),
        Err(errs) => {
            let first = errs
                .first()
                .map(|d| d.message.clone())
                .unwrap_or_else(|| "unknown error".into());
            Err(vec![syntax(
                file,
                0,
                1,
                &format!("the imported Neuron source does not parse ({first}):\n{src}"),
            )])
        }
    }
}
