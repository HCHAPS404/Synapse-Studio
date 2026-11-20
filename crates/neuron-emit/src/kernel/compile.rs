//! From NIR to state tables: the subset of `fsm` and `sequence` the micro-kernel runs.
//!
//! Supported: states with `entry`, `exit` and `do` blocks; transitions with `when`,
//! `after` and an action; sequences of steps with assignments, `set`/`reset`, `io` writes,
//! `wait`, `until` with a timeout and `->` jumps. Expressions are integer and boolean
//! (`+ - *`, comparisons, `and or not`). Everything else is `NE0610` with its location.
//! A table that exceeds a limit of the kernel is `NE1004`.

use super::{Insn, Machine, Port, StateCode, Table, Trans, MAX_IO, MAX_TIMER, MAX_VARS};
use crate::{nirx, Class, Diag};
use neuron_nir::{
    Block, CallTarget, Expr, ExprKind, Fsm, ItemDef, Lit, Nir, SeqItem, Stmt, StmtKind, Trigger,
    Type,
};
use std::collections::BTreeMap;

/// Scan period of a machine that does not declare one (the interpreter's default rate).
const DEFAULT_SCAN_NS: i64 = 10_000_000;

/// A difference from the reference interpreter, for the fidelity report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// Class.
    pub class: Class,
    /// What it is about.
    pub construct: String,
    /// The difference and its bound.
    pub note: String,
}

/// One compiled machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// The table.
    pub table: Table,
    /// Source file.
    pub file: String,
    /// Span of the `fsm` or `sequence` in that file.
    pub span: [usize; 2],
    /// Differences from the reference interpreter.
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sym {
    In(usize),
    Out(usize),
    Var(usize),
}

#[derive(Debug, Clone)]
enum Lazy {
    Var { init: Option<Box<Expr>> },
    Io,
}

struct Cx {
    file: String,
    plc: bool,
    scan_ns: i64,
    inputs: Vec<Port>,
    outputs: Vec<Port>,
    vars: Vec<(String, i16)>,
    by_id: BTreeMap<String, Sym>,
    named: BTreeMap<String, Sym>,
    lazy_id: BTreeMap<String, (String, Lazy)>,
    lazy_name: BTreeMap<String, Lazy>,
    diags: Vec<Diag>,
    arith: bool,
    stores: BTreeMap<usize, usize>,
}

fn is_real(t: &Type) -> bool {
    matches!(t, Type::Num { kind, .. }
        if kind.starts_with('f') || kind == "real" || kind.contains("fixed"))
}

impl Cx {
    fn new(nir: &Nir, m: usize) -> Cx {
        let module = &nir.modules[m];
        let mut cx = Cx {
            file: nirx::file(nir, m),
            plc: module.dialect == "plc",
            scan_ns: DEFAULT_SCAN_NS,
            inputs: Vec::new(),
            outputs: Vec::new(),
            vars: Vec::new(),
            by_id: BTreeMap::new(),
            named: BTreeMap::new(),
            lazy_id: BTreeMap::new(),
            lazy_name: BTreeMap::new(),
            diags: Vec::new(),
            arith: false,
            stores: BTreeMap::new(),
        };
        for item in &module.items {
            match &item.def {
                ItemDef::Scan { period } => {
                    if let Some(p) = nirx::dur_ns(period).filter(|p| *p > 0) {
                        cx.scan_ns = p;
                    }
                }
                ItemDef::PlcSection { section, vars } => {
                    for v in vars {
                        let boolean = matches!(v.vty, Type::Bool);
                        match section.as_str() {
                            "inputs" => {
                                if let Some(i) = cx.add_port(true, &v.name, boolean, v.span) {
                                    cx.by_id.insert(v.id.clone(), Sym::In(i));
                                    cx.named.insert(v.name.clone(), Sym::In(i));
                                }
                            }
                            "outputs" => {
                                if let Some(i) = cx.add_port(false, &v.name, boolean, v.span) {
                                    cx.by_id.insert(v.id.clone(), Sym::Out(i));
                                    cx.named.insert(v.name.clone(), Sym::Out(i));
                                }
                            }
                            _ => cx.lazy_var(&v.id, &v.name, v.init.clone()),
                        }
                    }
                }
                ItemDef::Io { name, .. } => {
                    cx.lazy_id.insert(item.id.clone(), (name.clone(), Lazy::Io));
                    cx.lazy_name.insert(name.clone(), Lazy::Io);
                }
                ItemDef::Var { var } => cx.lazy_var(&var.id, &var.name, var.init.clone()),
                _ => {}
            }
        }
        cx
    }

    fn lazy_var(&mut self, id: &str, name: &str, init: Option<Expr>) {
        let init = init.map(Box::new);
        self.lazy_id.insert(
            id.to_string(),
            (name.to_string(), Lazy::Var { init: init.clone() }),
        );
        self.lazy_name.insert(name.to_string(), Lazy::Var { init });
    }

    fn diag(&mut self, code: &'static str, span: [usize; 2], message: String, help: &str) {
        self.diags.push(Diag {
            code,
            message,
            help: Some(help.to_string()),
            file: self.file.clone(),
            span,
        });
    }

    fn unsupported(&mut self, span: [usize; 2], what: &str) {
        self.diag(
            "NE0610",
            span,
            format!("{what} cannot be expressed by the micro-kernel"),
            "the kernel runs assignments to outputs and variables, `io` writes, integer and boolean expressions, waits and `until` with a timeout; emit this program as regular code instead",
        );
    }

    fn limit(&mut self, span: [usize; 2], what: &str) {
        self.diag(
            "NE1004",
            span,
            format!("{what}: beyond the limits of the micro-kernel"),
            "split the machine into smaller ones, or emit it as regular code for the device",
        );
    }

    fn add_port(
        &mut self,
        input: bool,
        name: &str,
        boolean: bool,
        span: [usize; 2],
    ) -> Option<usize> {
        let list = if input {
            &mut self.inputs
        } else {
            &mut self.outputs
        };
        if list.len() >= MAX_IO {
            let what = if input {
                "more than 32 inputs"
            } else {
                "more than 32 outputs"
            };
            self.limit(span, what);
            return None;
        }
        list.push(Port {
            name: name.to_string(),
            boolean,
        });
        Some(list.len() - 1)
    }

    fn const_init(&mut self, init: &Option<Box<Expr>>) -> i16 {
        let Some(e) = init.as_deref() else { return 0 };
        let v = nirx::int(e).or_else(|| nirx::boolean(e).map(i64::from));
        match v.and_then(|v| i16::try_from(v).ok()) {
            Some(v) => v,
            None => {
                self.unsupported(e.span, "an initial value that is not a 16-bit constant");
                0
            }
        }
    }

    fn lookup(
        &mut self,
        id: Option<&str>,
        name: &str,
        write: bool,
        boolean: bool,
        span: [usize; 2],
    ) -> Option<Sym> {
        if let Some(s) = id.and_then(|i| self.by_id.get(i)) {
            return Some(*s);
        }
        if let Some(s) = self.named.get(name) {
            return Some(*s);
        }
        let hit = id.and_then(|i| self.lazy_id.get(i).cloned()).or_else(|| {
            self.lazy_name
                .get(name)
                .cloned()
                .map(|l| (name.to_string(), l))
        });
        let Some((n, lazy)) = hit else {
            self.unsupported(span, &format!("the reference to `{name}`"));
            return None;
        };
        let sym = match lazy {
            Lazy::Var { init } => {
                if self.vars.len() >= MAX_VARS {
                    self.limit(span, "more than 32 variables");
                    return None;
                }
                let v = self.const_init(&init);
                self.vars.push((n.clone(), v));
                Sym::Var(self.vars.len() - 1)
            }
            Lazy::Io => {
                let i = self.add_port(!write, &n, boolean, span)?;
                if write {
                    Sym::Out(i)
                } else {
                    Sym::In(i)
                }
            }
        };
        if let Some(i) = id {
            self.by_id.insert(i.to_string(), sym);
        }
        self.named.insert(name.to_string(), sym);
        self.named.insert(n, sym);
        Some(sym)
    }

    fn is_io(&self, name: &str) -> bool {
        matches!(self.lazy_name.get(name), Some(Lazy::Io))
    }

    // ---- expressions ------------------------------------------------------------

    fn load_sym(&mut self, s: Sym, out: &mut Vec<Insn>) {
        out.push(match s {
            Sym::In(i) => Insn::Ldi(i as u8),
            Sym::Out(i) => Insn::Ldo(i as u8),
            Sym::Var(i) => Insn::Ldv(i as u8),
        });
    }

    fn io_call(&self, callee: &Expr, target: &Option<CallTarget>) -> Option<(String, String)> {
        if let Some(CallTarget::Io { binding, method }) = target {
            return Some((binding.clone(), method.clone()));
        }
        if let ExprKind::Field { base, name, .. } = &callee.e {
            if let ExprKind::Ref { name: b, .. }
            | ExprKind::Global { name: b }
            | ExprKind::Local { name: b, .. } = &base.e
            {
                if self.is_io(b) {
                    return Some((b.clone(), name.clone()));
                }
            }
        }
        None
    }

    fn ex(&mut self, e: &Expr, out: &mut Vec<Insn>) {
        if is_real(&e.ty) {
            self.unsupported(e.span, "a real number");
            return;
        }
        let boolean = matches!(e.ty, Type::Bool);
        match &e.e {
            ExprKind::Lit {
                lit: Lit::Bool { v },
            } => out.push(Insn::Push(i16::from(*v))),
            ExprKind::Lit {
                lit: Lit::Int { v, .. },
            } => match v.parse::<i64>().ok().and_then(|x| i16::try_from(x).ok()) {
                Some(x) => out.push(Insn::Push(x)),
                None => self.limit(e.span, "a constant that does not fit in 16 bits"),
            },
            ExprKind::Ref { id, name } | ExprKind::Local { id, name } => {
                if let Some(s) = self.lookup(Some(id), name, false, boolean, e.span) {
                    self.load_sym(s, out);
                }
            }
            ExprKind::Global { name } => {
                if let Some(s) = self.lookup(None, name, false, boolean, e.span) {
                    self.load_sym(s, out);
                }
            }
            ExprKind::Unary { op, operand } => match op.as_str() {
                "not" | "!" => {
                    self.ex(operand, out);
                    out.push(Insn::Not);
                }
                "-" => {
                    self.ex(operand, out);
                    out.push(Insn::Neg);
                    self.arith = true;
                }
                other => self.unsupported(e.span, &format!("the operator `{other}`")),
            },
            ExprKind::Binary { op, lhs, rhs } => {
                let insn = match (op.as_str(), boolean) {
                    ("and" | "&&", _) | ("&", true) => Some(Insn::And),
                    ("or" | "||", _) | ("|", true) => Some(Insn::Or),
                    ("+", _) => Some(Insn::Add),
                    ("-", _) => Some(Insn::Sub),
                    ("*", _) => Some(Insn::Mul),
                    ("==", _) => Some(Insn::Eq),
                    ("!=", _) => Some(Insn::Ne),
                    ("<", _) => Some(Insn::Lt),
                    ("<=", _) => Some(Insn::Le),
                    (">", _) => Some(Insn::Gt),
                    (">=", _) => Some(Insn::Ge),
                    _ => None,
                };
                match insn {
                    Some(i) => {
                        self.ex(lhs, out);
                        self.ex(rhs, out);
                        out.push(i);
                        if matches!(i, Insn::Add | Insn::Sub | Insn::Mul) {
                            self.arith = true;
                        }
                    }
                    None => self.unsupported(e.span, &format!("the operator `{op}`")),
                }
            }
            ExprKind::Compare { first, rest } => {
                for (j, c) in rest.iter().enumerate() {
                    let left = if j == 0 { first } else { &rest[j - 1].rhs };
                    let insn = match c.op.as_str() {
                        "==" => Insn::Eq,
                        "!=" => Insn::Ne,
                        "<" => Insn::Lt,
                        "<=" => Insn::Le,
                        ">" => Insn::Gt,
                        ">=" => Insn::Ge,
                        other => {
                            self.unsupported(e.span, &format!("the comparison `{other}`"));
                            return;
                        }
                    };
                    self.ex(left, out);
                    self.ex(&c.rhs, out);
                    out.push(insn);
                    if j > 0 {
                        out.push(Insn::And);
                    }
                }
            }
            ExprKind::Call {
                callee,
                args,
                target,
            } => match self.io_call(callee, target) {
                Some((b, m)) if matches!(m.as_str(), "read" | "get") && args.is_empty() => {
                    if let Some(s) = self.lookup(None, &b, false, boolean, e.span) {
                        self.load_sym(s, out);
                    }
                }
                _ => self.unsupported(e.span, "this call"),
            },
            _ => self.unsupported(e.span, "this expression"),
        }
    }

    // ---- statements -------------------------------------------------------------

    fn place(&mut self, p: &Expr) -> Option<Sym> {
        let boolean = matches!(p.ty, Type::Bool);
        match &p.e {
            ExprKind::Ref { id, name } | ExprKind::Local { id, name } => {
                self.lookup(Some(id), name, true, boolean, p.span)
            }
            ExprKind::Global { name } => self.lookup(None, name, true, boolean, p.span),
            _ => {
                self.unsupported(p.span, "this assignment target");
                None
            }
        }
    }

    fn store(&mut self, s: Sym, span: [usize; 2], out: &mut Vec<Insn>) {
        match s {
            Sym::Out(i) => {
                *self.stores.entry(i).or_insert(0) += 1;
                out.push(Insn::Sto(i as u8));
            }
            Sym::Var(i) => out.push(Insn::Stv(i as u8)),
            Sym::In(_) => self.unsupported(span, "an assignment to an input"),
        }
    }

    /// `c` as 0 or 1.
    fn flag(&mut self, c: &Expr, out: &mut Vec<Insn>) {
        self.ex(c, out);
        out.push(Insn::Push(0));
        out.push(Insn::Ne);
    }

    fn block(&mut self, b: &Block, out: &mut Vec<Insn>) {
        for s in &b.stmts {
            self.stmt(s, out);
        }
    }

    fn stmt(&mut self, s: &Stmt, out: &mut Vec<Insn>) {
        match &s.s {
            StmtKind::Pass => {}
            StmtKind::Assign {
                place,
                op,
                value,
                when,
            } => {
                let Some(sym) = self.place(place) else { return };
                let combine = match op.as_str() {
                    "=" => None,
                    "+=" => Some(Insn::Add),
                    "-=" => Some(Insn::Sub),
                    "*=" => Some(Insn::Mul),
                    other => {
                        self.unsupported(s.span, &format!("the assignment `{other}`"));
                        return;
                    }
                };
                let mut v = Vec::new();
                if let Some(c) = combine {
                    self.load_sym(sym, &mut v);
                    self.ex(value, &mut v);
                    v.push(c);
                    self.arith = true;
                } else {
                    self.ex(value, &mut v);
                }
                match when {
                    None => out.extend(v),
                    Some(c) => {
                        // flag * new + (not flag) * old
                        self.flag(c, out);
                        out.extend(v);
                        out.push(Insn::Mul);
                        self.ex(c, out);
                        out.push(Insn::Not);
                        self.load_sym(sym, out);
                        out.push(Insn::Mul);
                        out.push(Insn::Add);
                    }
                }
                self.store(sym, s.span, out);
            }
            StmtKind::Coil { set, place, when } => {
                let Some(sym) = self.place(place) else { return };
                self.load_sym(sym, out);
                self.ex(when, out);
                if *set {
                    out.push(Insn::Or);
                } else {
                    out.push(Insn::Not);
                    out.push(Insn::And);
                }
                self.store(sym, s.span, out);
            }
            StmtKind::Expr { expr } => {
                let ExprKind::Call {
                    callee,
                    args,
                    target,
                } = &expr.e
                else {
                    self.unsupported(s.span, "this statement");
                    return;
                };
                let Some((b, m)) = self.io_call(callee, target) else {
                    self.unsupported(s.span, "this call");
                    return;
                };
                match (m.as_str(), args.first()) {
                    ("write" | "set", Some(a)) => {
                        let boolean = matches!(a.value.ty, Type::Bool);
                        let Some(sym) = self.lookup(None, &b, true, boolean, s.span) else {
                            return;
                        };
                        self.ex(&a.value, out);
                        self.store(sym, s.span, out);
                    }
                    ("toggle", None) => {
                        let Some(sym) = self.lookup(None, &b, true, true, s.span) else {
                            return;
                        };
                        self.load_sym(sym, out);
                        out.push(Insn::Not);
                        self.store(sym, s.span, out);
                    }
                    _ => self.unsupported(s.span, &format!("`{b}.{m}`")),
                }
            }
            StmtKind::Move { .. } => self.unsupported(s.span, "a robot motion"),
            StmtKind::Fault { .. } => self.unsupported(s.span, "`fault`"),
            _ => self.unsupported(s.span, "this statement"),
        }
    }

    /// A duration as scans of this machine (rounded up).
    fn scans(&mut self, d: &Expr) -> Option<i16> {
        let Some(ns) = nirx::dur_ns(d) else {
            self.unsupported(d.span, "a duration that is not a constant");
            return None;
        };
        let n = if ns <= 0 {
            0
        } else {
            (ns + self.scan_ns - 1) / self.scan_ns
        };
        match i16::try_from(n).ok() {
            Some(v) => Some(v),
            None => {
                self.limit(
                    d.span,
                    &format!("a duration of {n} scans (the timer holds at most {MAX_TIMER})"),
                );
                None
            }
        }
    }

    fn timer_at_least(&mut self, d: &Expr) -> Option<Vec<Insn>> {
        let n = self.scans(d)?;
        Some(vec![Insn::Timer, Insn::Push(n), Insn::Ge])
    }

    fn finish(
        mut self,
        name: &str,
        kind: &str,
        span: [usize; 2],
        mut machine: Machine,
        state_names: Vec<String>,
        note_period: Option<i64>,
    ) -> Result<Compiled, Vec<Diag>> {
        machine.inputs = self.inputs.len();
        machine.outputs = self.outputs.len();
        machine.vars = self.vars.iter().map(|v| v.1).collect();
        machine.publish_initial = self.plc;
        if !self.diags.is_empty() {
            return Err(self.diags);
        }
        let limits = machine.limits();
        if !limits.is_empty() {
            self.limit(span, &format!("`{name}` has {}", limits.join("; ")));
            return Err(self.diags);
        }
        if let Err(e) = machine.check() {
            self.diag(
                "NE0610",
                span,
                format!("`{name}` is not well formed: {e}"),
                "this is a compiler error; report it",
            );
            return Err(self.diags);
        }
        let mut notes = Vec::new();
        let seq = kind == "sequence";
        if seq {
            notes.push(Note {
                class: Class::Equivalent,
                construct: format!("sequence `{name}`"),
                note: format!(
                    "the kernel advances a sequence once per scan ({} ms) instead of once per millisecond: every output change is within one scan of the reference interpreter",
                    self.scan_ns as f64 / 1e6
                ),
            });
        } else if let Some(p) = note_period.filter(|p| *p != self.scan_ns) {
            notes.push(Note {
                class: Class::Equivalent,
                construct: format!("state machine `{name}`"),
                note: format!("evaluated every {p} ns"),
            });
        }
        if self.stores.values().any(|n| *n > 1) {
            notes.push(Note {
                class: Class::Equivalent,
                construct: format!("outputs of `{name}`"),
                note: "output changes are reported once per scan: when several writes to one output happen in one scan only the last value is reported, while the interpreter traces each write".into(),
            });
        }
        if self.arith {
            notes.push(Note {
                class: Class::Equivalent,
                construct: format!("arithmetic of `{name}`"),
                note: "integers are 16-bit and wrap around; the reference interpreter enters the safe state on an overflow".into(),
            });
        }
        let table = Table {
            nst: 1,
            name: name.to_string(),
            kind: kind.to_string(),
            file: self.file.clone(),
            scan_ns: self.scan_ns,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            var_names: self.vars.iter().map(|v| v.0.clone()).collect(),
            state_names,
            machine,
        };
        Ok(Compiled {
            table,
            file: self.file.clone(),
            span,
            notes,
        })
    }
}

// ---- state machines ---------------------------------------------------------------

fn compile_fsm(nir: &Nir, m: usize, span: [usize; 2], fsm: &Fsm) -> Result<Compiled, Vec<Diag>> {
    let mut cx = Cx::new(nir, m);
    let mut period = None;
    match &fsm.trigger {
        Some(Trigger::Every { period: p, offset }) => {
            match nirx::dur_ns(p).filter(|p| *p > 0) {
                Some(p) => {
                    cx.scan_ns = p;
                    period = Some(p);
                }
                None => cx.unsupported(p.span, "a period that is not a positive constant"),
            }
            if let Some(o) = offset {
                if nirx::dur_ns(o) != Some(0) {
                    cx.unsupported(o.span, "an `offset`");
                }
            }
        }
        Some(Trigger::On { event }) => {
            cx.unsupported(event.span, "a state machine that runs on an event");
        }
        None => {}
    }
    for v in &fsm.vars {
        cx.lazy_var(&v.id, &v.name, v.init.clone());
    }
    let index = |name: &str| fsm.states.iter().position(|s| s.name == name);
    let mut states = Vec::new();
    for st in &fsm.states {
        let mut code = StateCode::default();
        if let Some(b) = &st.entry {
            cx.block(b, &mut code.entry);
        }
        if let Some(b) = &st.exit {
            cx.block(b, &mut code.exit);
        }
        if let Some(b) = &st.during {
            cx.block(b, &mut code.during);
        }
        for t in &st.transitions {
            let Some(target) = index(&t.target) else {
                cx.unsupported(
                    t.span,
                    &format!("a transition to the unknown state `{}`", t.target),
                );
                continue;
            };
            let mut guard: Option<Vec<Insn>> = None;
            if let Some(w) = &t.when {
                let mut g = Vec::new();
                cx.ex(w, &mut g);
                guard = Some(g);
            }
            if let Some(a) = &t.after {
                if let Some(mut tg) = cx.timer_at_least(a) {
                    guard = Some(match guard {
                        Some(mut g) => {
                            g.append(&mut tg);
                            g.push(Insn::Or);
                            g
                        }
                        None => tg,
                    });
                }
            }
            let mut action = Vec::new();
            if let Some(b) = &t.action {
                cx.block(b, &mut action);
            }
            code.transitions.push(Trans {
                target,
                guard,
                action,
            });
        }
        states.push(code);
    }
    let initial = fsm.states.iter().position(|s| s.initial).unwrap_or(0);
    let names = fsm.states.iter().map(|s| s.name.clone()).collect();
    let machine = Machine {
        sequence: false,
        publish_initial: false,
        inputs: 0,
        outputs: 0,
        vars: Vec::new(),
        initial,
        states,
    };
    cx.finish(&fsm.name, "fsm", span, machine, names, period)
}

// ---- sequences --------------------------------------------------------------------

enum SOp<'b> {
    Step(&'b str),
    Stmt(&'b Stmt),
}

fn flatten<'b>(items: &'b [SeqItem], out: &mut Vec<SOp<'b>>, cx: &mut Cx) {
    for it in items {
        match it {
            SeqItem::Step { name, body, .. } => {
                out.push(SOp::Step(name));
                flatten(body, out, cx);
            }
            SeqItem::Parallel { span, .. } => cx.unsupported(*span, "a `parallel` block"),
            SeqItem::Stmt { stmt } => out.push(SOp::Stmt(stmt)),
        }
    }
}

fn step_index(ops: &[SOp<'_>], name: &str) -> Option<usize> {
    ops.iter()
        .position(|o| matches!(o, SOp::Step(s) if *s == name))
}

#[derive(Clone, Copy)]
enum Dest {
    Wait(usize),
    Done,
}

/// Runs straight-line code from op `k` up to the next wait, like the interpreter does
/// within one tick: the actions to run and where the sequence stops.
fn run_from(cx: &mut Cx, ops: &[SOp<'_>], mut k: usize) -> Option<(Vec<Insn>, Dest)> {
    let mut code = Vec::new();
    let mut jumped: Vec<usize> = Vec::new();
    loop {
        let Some(op) = ops.get(k) else {
            return Some((code, Dest::Done));
        };
        match op {
            SOp::Step(_) => k += 1,
            SOp::Stmt(s) => match &s.s {
                StmtKind::Until { .. } | StmtKind::Wait { .. } => {
                    return Some((code, Dest::Wait(k)));
                }
                StmtKind::Jump { target } => match step_index(ops, target) {
                    Some(i) => {
                        if jumped.contains(&i) {
                            cx.unsupported(s.span, "a loop of jumps without a wait");
                            return None;
                        }
                        jumped.push(i);
                        k = i;
                    }
                    None => return Some((code, Dest::Done)),
                },
                StmtKind::Return { .. } => return Some((code, Dest::Done)),
                _ => {
                    cx.stmt(s, &mut code);
                    k += 1;
                }
            },
        }
    }
}

struct Builder {
    names: Vec<String>,
    states: Vec<StateCode>,
    map: BTreeMap<usize, usize>,
    queue: Vec<usize>,
    done: Option<usize>,
}

impl Builder {
    fn state_for(&mut self, ops: &[SOp<'_>], d: Dest) -> usize {
        match d {
            Dest::Done => {
                if let Some(s) = self.done {
                    return s;
                }
                self.names.push("done".into());
                self.states.push(StateCode::default());
                self.done = Some(self.states.len() - 1);
                self.states.len() - 1
            }
            Dest::Wait(w) => {
                if let Some(s) = self.map.get(&w) {
                    return *s;
                }
                let step = ops[..w]
                    .iter()
                    .rev()
                    .find_map(|o| match o {
                        SOp::Step(s) => Some(*s),
                        SOp::Stmt(_) => None,
                    })
                    .unwrap_or("start");
                let kind = match ops.get(w) {
                    Some(SOp::Stmt(s)) if matches!(s.s, StmtKind::Wait { .. }) => "wait",
                    _ => "until",
                };
                self.names.push(format!("{step}:{kind}"));
                self.states.push(StateCode::default());
                let s = self.states.len() - 1;
                self.map.insert(w, s);
                self.queue.push(w);
                s
            }
        }
    }
}

fn compile_sequence(
    nir: &Nir,
    m: usize,
    span: [usize; 2],
    name: &str,
    items: &[SeqItem],
) -> Result<Compiled, Vec<Diag>> {
    let mut cx = Cx::new(nir, m);
    let mut ops = Vec::new();
    flatten(items, &mut ops, &mut cx);
    if !cx.diags.is_empty() {
        return Err(cx.diags);
    }
    let mut b = Builder {
        names: vec!["start".into()],
        states: vec![StateCode::default()],
        map: BTreeMap::new(),
        queue: Vec::new(),
        done: None,
    };
    if let Some((action, dest)) = run_from(&mut cx, &ops, 0) {
        let target = b.state_for(&ops, dest);
        b.states[0].transitions.push(Trans {
            target,
            guard: None,
            action,
        });
    }
    while let Some(w) = b.queue.pop() {
        let Some(&me) = b.map.get(&w) else { continue };
        let SOp::Stmt(s) = &ops[w] else { continue };
        let mut trans = Vec::new();
        match &s.s {
            StmtKind::Until {
                cond,
                timeout,
                on_timeout,
            } => {
                let mut guard = Vec::new();
                cx.ex(cond, &mut guard);
                if let Some((action, dest)) = run_from(&mut cx, &ops, w + 1) {
                    let target = b.state_for(&ops, dest);
                    trans.push(Trans {
                        target,
                        guard: Some(guard),
                        action,
                    });
                }
                if let Some(t) = timeout {
                    if let Some(g) = cx.timer_at_least(t) {
                        let next = match on_timeout {
                            Some(step) => step_index(&ops, step),
                            None => Some(w + 1),
                        };
                        let run = match next {
                            Some(k) => run_from(&mut cx, &ops, k),
                            None => Some((Vec::new(), Dest::Done)),
                        };
                        if let Some((action, dest)) = run {
                            let target = b.state_for(&ops, dest);
                            trans.push(Trans {
                                target,
                                guard: Some(g),
                                action,
                            });
                        }
                    }
                }
            }
            StmtKind::Wait { duration } => {
                if let Some(g) = cx.timer_at_least(duration) {
                    if let Some((action, dest)) = run_from(&mut cx, &ops, w + 1) {
                        let target = b.state_for(&ops, dest);
                        trans.push(Trans {
                            target,
                            guard: Some(g),
                            action,
                        });
                    }
                }
            }
            _ => {}
        }
        b.states[me].transitions = trans;
    }
    let machine = Machine {
        sequence: true,
        publish_initial: false,
        inputs: 0,
        outputs: 0,
        vars: Vec::new(),
        initial: 0,
        states: b.states,
    };
    cx.finish(name, "sequence", span, machine, b.names, None)
}

/// Compiles every `fsm` and `sequence` of `nir`. Machines that cannot be compiled give
/// `NE0610` (not expressible) or `NE1004` (beyond the limits) with their location.
pub fn compile_nir(nir: &Nir) -> (Vec<Compiled>, Vec<Diag>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for (m, module) in nir.modules.iter().enumerate() {
        for item in &module.items {
            let r = match &item.def {
                ItemDef::Fsm { fsm } => compile_fsm(nir, m, item.span, fsm),
                ItemDef::Sequence { name, items, .. } => {
                    compile_sequence(nir, m, item.span, name, items)
                }
                _ => continue,
            };
            match r {
                Ok(c) => ok.push(c),
                Err(d) => bad.extend(d),
            }
        }
    }
    (ok, bad)
}
