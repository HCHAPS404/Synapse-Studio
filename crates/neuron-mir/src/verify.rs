//! The MIR verifier: every name that a program uses exists and is used with the right shape.

use crate::mir::*;
use std::collections::BTreeSet;

struct V<'a> {
    p: &'a Program,
    globals: BTreeSet<&'a str>,
    signals: BTreeSet<&'a str>,
    funcs: BTreeSet<&'a str>,
    errors: Vec<String>,
}

/// Checks a program. An empty result means that the program is consistent.
pub fn verify(p: &Program) -> Vec<String> {
    let mut v = V {
        p,
        globals: p
            .globals
            .iter()
            .filter(|g| !g.signal)
            .map(|g| g.name.as_str())
            .collect(),
        signals: p
            .globals
            .iter()
            .filter(|g| g.signal)
            .map(|g| g.name.as_str())
            .collect(),
        funcs: p.funcs.iter().map(|f| f.name.as_str()).collect(),
        errors: Vec::new(),
    };
    for g in &p.globals {
        if let Some(i) = &g.init {
            if i.ty() != g.ty && !(g.ty.is_float() && matches!(i, Expr::Float(_))) {
                v.errors.push(format!(
                    "`{}` has the type {} but its initial value has {}",
                    g.name,
                    g.ty.name(),
                    i.ty().name()
                ));
            }
        }
    }
    for f in &p.funcs {
        v.block(&f.body, f.ret, &format!("fn {}", f.name));
    }
    for t in &p.tasks {
        v.block(&t.body, None, &format!("task {}", t.name));
        for s in t.reads.iter().chain(&t.writes) {
            if !v.signals.contains(s.as_str()) {
                v.errors
                    .push(format!("task {} uses the unknown signal `{s}`", t.name));
            }
        }
        if t.period_ms == 0 && t.event.is_none() {
            v.errors
                .push(format!("task {} has neither a period nor an event", t.name));
        }
        if let Some(e) = t.event {
            if e >= p.events.len() {
                v.errors
                    .push(format!("task {} waits for the unknown event #{e}", t.name));
            }
        }
    }
    for i in &p.irqs {
        v.block(&i.body, None, &format!("irq {}", i.name));
        if i.io >= p.ios.len() || p.ios[i.io].kind != IoK::GpioIn {
            v.errors
                .push(format!("irq {} is not bound to an input", i.name));
        }
    }
    for f in &p.fsms {
        for (k, st) in f.states.iter().enumerate() {
            let w = format!("fsm {} state {k}", f.name);
            v.block(&st.entry, None, &w);
            v.block(&st.exit, None, &w);
            v.block(&st.during, None, &w);
            for t in &st.trans {
                if t.target >= f.states.len() {
                    v.errors
                        .push(format!("{w}: transition to the unknown state {}", t.target));
                }
                if let Some(c) = &t.when {
                    v.expr(c, &w);
                }
                v.block(&t.action, None, &w);
            }
        }
        if f.initial >= f.states.len() {
            v.errors
                .push(format!("fsm {} has no initial state", f.name));
        }
    }
    for r in p.rules.iter().chain(&p.safety.rules) {
        v.expr(&r.cond, "rule");
        v.block(&r.body, None, "rule");
    }
    v.block(&p.safety.safe_state, None, "safe_state");
    if let Some(r) = &p.safety.reset {
        v.expr(r, "reset");
    }
    for (_, b) in &p.safety.on_fault {
        v.block(b, None, "on fault");
    }
    v.errors
}

impl V<'_> {
    fn block(&mut self, b: &[Stmt], ret: Option<Ty>, w: &str) {
        for s in b {
            self.stmt(s, ret, w);
        }
    }

    fn io(&mut self, i: usize, w: &str) {
        if i >= self.p.ios.len() {
            self.errors.push(format!("{w}: unknown io #{i}"));
        }
    }

    fn stmt(&mut self, s: &Stmt, ret: Option<Ty>, w: &str) {
        match &s.kind {
            StmtKind::Let { value, .. } => self.expr(value, w),
            StmtKind::Assign { place, value } => {
                match place {
                    Place::Global(n) if !self.globals.contains(n.as_str()) => {
                        self.errors
                            .push(format!("{w}: assignment to the unknown variable `{n}`"));
                    }
                    Place::Signal { name, .. } if !self.signals.contains(name.as_str()) => {
                        self.errors
                            .push(format!("{w}: assignment to the unknown signal `{name}`"));
                    }
                    _ => {}
                }
                self.expr(value, w);
            }
            StmtKind::IoPut { io, value } | StmtKind::IoPwm { io, value } => {
                self.io(*io, w);
                self.expr(value, w);
            }
            StmtKind::IoFlip { io } => self.io(*io, w),
            StmtKind::Print { io, parts, .. } => {
                self.io(*io, w);
                for p in parts {
                    if let PrintPart::Value(e) = p {
                        self.expr(e, w);
                    }
                }
            }
            StmtKind::CallVoid { name, args } => {
                match self.p.funcs.iter().find(|f| &f.name == name) {
                    Some(f) if f.params.len() != args.len() => {
                        self.errors.push(format!(
                            "{w}: `{name}` takes {} arguments, not {}",
                            f.params.len(),
                            args.len()
                        ));
                    }
                    None => self
                        .errors
                        .push(format!("{w}: call of the unknown function `{name}`")),
                    _ => {}
                }
                for a in args {
                    self.expr(a, w);
                }
            }
            StmtKind::If {
                cond,
                then,
                otherwise,
            } => {
                self.expr(cond, w);
                self.block(then, ret, w);
                self.block(otherwise, ret, w);
            }
            StmtKind::For {
                ty, body, lo, hi, ..
            } => {
                if !ty.is_int() {
                    self.errors
                        .push(format!("{w}: a loop variable must be an integer"));
                }
                if hi < lo {
                    self.errors
                        .push(format!("{w}: the range {lo}..{hi} is empty or reversed"));
                }
                self.block(body, ret, w);
            }
            StmtKind::While { cond, body, max } => {
                if *max < 0 {
                    self.errors.push(format!("{w}: a negative loop bound"));
                }
                self.expr(cond, w);
                self.block(body, ret, w);
            }
            StmtKind::Return(v) => {
                if let Some(e) = v {
                    self.expr(e, w);
                }
            }
            StmtKind::Emit(e) => {
                if *e >= self.p.events.len() {
                    self.errors
                        .push(format!("{w}: emit of the unknown event #{e}"));
                }
            }
            StmtKind::Break | StmtKind::Continue | StmtKind::Fault(_) => {}
        }
    }

    fn expr(&mut self, e: &Expr, w: &str) {
        match e {
            Expr::Global(n, _) if !self.globals.contains(n.as_str()) => {
                self.errors
                    .push(format!("{w}: use of the unknown variable `{n}`"));
            }
            Expr::Signal(n, _, _) if !self.signals.contains(n.as_str()) => {
                self.errors
                    .push(format!("{w}: use of the unknown signal `{n}`"));
            }
            Expr::Call(n, a, _) => {
                if !self.funcs.contains(n.as_str()) {
                    self.errors
                        .push(format!("{w}: call of the unknown function `{n}`"));
                }
                for x in a {
                    self.expr(x, w);
                }
            }
            Expr::IoGet(i, _) => self.io(*i, w),
            Expr::Unary(_, x, _) | Expr::Cast(x, _) => self.expr(x, w),
            Expr::Binary(op, a, b, t) => {
                if op.is_compare() && *t != Ty::Bool {
                    self.errors
                        .push(format!("{w}: a comparison must have type bool"));
                }
                self.expr(a, w);
                self.expr(b, w);
            }
            Expr::IfElse(c, a, b, _) => {
                self.expr(c, w);
                self.expr(a, w);
                self.expr(b, w);
            }
            Expr::Builtin(_, a, _) | Expr::Scale(a) => {
                for x in a {
                    self.expr(x, w);
                }
            }
            _ => {}
        }
    }
}
