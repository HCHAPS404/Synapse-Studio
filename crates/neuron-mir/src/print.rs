//! A stable text form of the MIR, for the golden files.

use crate::mir::*;
use std::fmt::Write;

impl Program {
    /// Prints the program. The output is deterministic and independent of the machine.
    pub fn render(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "program {} board={:?} sysclk={}",
            self.name, self.board, self.sysclk_hz
        );
        for (i, io) in self.ios.iter().enumerate() {
            let _ = writeln!(
                s,
                "io {i} {} {:?} port={:?} bit={:?} aux={} cfg={}",
                io.name, io.kind, io.port, io.bit, io.aux, io.cfg
            );
        }
        for e in &self.events {
            let _ = writeln!(s, "event {e}");
        }
        for f in &self.faults {
            let _ = writeln!(s, "fault {f}");
        }
        for g in &self.globals {
            let init = g
                .init
                .as_ref()
                .map_or(String::new(), |e| format!(" = {}", expr(e)));
            let _ = writeln!(
                s,
                "{} {}: {}{}{}",
                if g.signal { "signal" } else { "var" },
                g.name,
                g.ty.name(),
                init,
                if g.shared { " shared" } else { "" }
            );
        }
        for f in &self.funcs {
            let params: Vec<String> = f
                .params
                .iter()
                .map(|(n, t)| format!("{n}: {}", t.name()))
                .collect();
            let _ = writeln!(
                s,
                "fn {}({}) -> {}{}",
                f.name,
                params.join(", "),
                f.ret.map_or("void", Ty::name),
                if f.atomic { " atomic" } else { "" }
            );
            block(&mut s, &f.body, 1);
        }
        for t in &self.tasks {
            let _ = writeln!(
                s,
                "task {} every={}ms offset={}ms prio={} event={:?} deadline={}ms reads={:?} writes={:?}",
                t.name, t.period_ms, t.offset_ms, t.prio, t.event, t.deadline_ms, t.reads, t.writes
            );
            block(&mut s, &t.body, 1);
        }
        for i in &self.irqs {
            let _ = writeln!(
                s,
                "irq {} io={} edge={:?} prio={}",
                i.name, i.io, i.edge, i.prio
            );
            block(&mut s, &i.body, 1);
        }
        for f in &self.fsms {
            let _ = writeln!(
                s,
                "fsm {} every={}ms initial={}",
                f.name, f.period_ms, f.initial
            );
            for (k, st) in f.states.iter().enumerate() {
                let _ = writeln!(s, "  state {k} {}", st.name);
                for (label, b) in [("entry", &st.entry), ("exit", &st.exit), ("do", &st.during)] {
                    if !b.is_empty() {
                        let _ = writeln!(s, "    {label}:");
                        block(&mut s, b, 3);
                    }
                }
                for t in &st.trans {
                    let w = t
                        .when
                        .as_ref()
                        .map_or(String::new(), |e| format!(" when {}", expr(e)));
                    let a = t
                        .after_ms
                        .map_or(String::new(), |m| format!(" after {m}ms"));
                    let _ = writeln!(s, "    -> {}{w}{a}", t.target);
                    block(&mut s, &t.action, 3);
                }
            }
        }
        for r in &self.rules {
            rule(&mut s, "rule", r);
        }
        let _ = writeln!(
            s,
            "safety watchdog={:?} rate={}ms",
            self.safety.watchdog_ms, self.safety.rate_ms
        );
        let _ = writeln!(s, "  safe_state:");
        block(&mut s, &self.safety.safe_state, 2);
        for r in &self.safety.rules {
            rule(&mut s, "  rule", r);
        }
        if let Some(r) = &self.safety.reset {
            let _ = writeln!(s, "  reset when {}", expr(r));
        }
        for (f, b) in &self.safety.on_fault {
            let _ = writeln!(s, "  on_fault {f:?}:");
            block(&mut s, b, 2);
        }
        s
    }
}

fn rule(s: &mut String, label: &str, r: &Rule) {
    let _ = writeln!(s, "{label} when {} hold={:?}", expr(&r.cond), r.hold_ms);
    block(s, &r.body, 2);
}

fn block(s: &mut String, b: &[Stmt], depth: usize) {
    for st in b {
        stmt(s, st, depth);
    }
}

fn stmt(s: &mut String, st: &Stmt, d: usize) {
    let pad = "  ".repeat(d);
    let line = st.loc.line;
    match &st.kind {
        StmtKind::Let { name, ty, value } => {
            let _ = writeln!(
                s,
                "{pad}let {name}: {} = {}  @{line}",
                ty.name(),
                expr(value)
            );
        }
        StmtKind::Assign { place, value } => {
            let _ = writeln!(s, "{pad}{} = {}  @{line}", place_str(place), expr(value));
        }
        StmtKind::IoPut { io, value } => {
            let _ = writeln!(s, "{pad}io{io}.write({})  @{line}", expr(value));
        }
        StmtKind::IoFlip { io } => {
            let _ = writeln!(s, "{pad}io{io}.toggle()  @{line}");
        }
        StmtKind::IoPwm { io, value } => {
            let _ = writeln!(s, "{pad}io{io}.duty = {}  @{line}", expr(value));
        }
        StmtKind::Print { io, parts, newline } => {
            let p: Vec<String> = parts
                .iter()
                .map(|x| match x {
                    PrintPart::Text(t) => format!("{t:?}"),
                    PrintPart::Value(e) => expr(e),
                })
                .collect();
            let _ = writeln!(
                s,
                "{pad}io{io}.print({}){}  @{line}",
                p.join(", "),
                if *newline { "+nl" } else { "" }
            );
        }
        StmtKind::CallVoid { name, args } => {
            let a: Vec<String> = args.iter().map(expr).collect();
            let _ = writeln!(s, "{pad}{name}({})  @{line}", a.join(", "));
        }
        StmtKind::If {
            cond,
            then,
            otherwise,
        } => {
            let _ = writeln!(s, "{pad}if {}  @{line}", expr(cond));
            block(s, then, d + 1);
            if !otherwise.is_empty() {
                let _ = writeln!(s, "{pad}else");
                block(s, otherwise, d + 1);
            }
        }
        StmtKind::For {
            var,
            ty,
            lo,
            hi,
            body,
        } => {
            let _ = writeln!(s, "{pad}for {var}: {} in {lo}..{hi}  @{line}", ty.name());
            block(s, body, d + 1);
        }
        StmtKind::While { cond, max, body } => {
            let _ = writeln!(s, "{pad}while {} max {max}  @{line}", expr(cond));
            block(s, body, d + 1);
        }
        StmtKind::Return(v) => {
            let _ = writeln!(
                s,
                "{pad}return {}  @{line}",
                v.as_ref().map_or(String::new(), expr)
            );
        }
        StmtKind::Break => {
            let _ = writeln!(s, "{pad}break  @{line}");
        }
        StmtKind::Continue => {
            let _ = writeln!(s, "{pad}continue  @{line}");
        }
        StmtKind::Fault(f) => {
            let _ = writeln!(s, "{pad}fault {f}  @{line}");
        }
        StmtKind::Emit(e) => {
            let _ = writeln!(s, "{pad}emit #{e}  @{line}");
        }
    }
}

fn place_str(p: &Place) -> String {
    match p {
        Place::Local(n) | Place::Global(n) => n.clone(),
        Place::Signal { name, deferred } => format!("{name}{}", if *deferred { "'" } else { "" }),
    }
}

fn expr(e: &Expr) -> String {
    match e {
        Expr::Int(v, t) => format!("{v}:{}", t.name()),
        Expr::Float(f) => format!("{f}"),
        Expr::Bool(b) => b.to_string(),
        Expr::Local(n, _) | Expr::Global(n, _) => n.clone(),
        Expr::Signal(n, _, SigAccess::Current) => format!("{n}@now"),
        Expr::Signal(n, _, SigAccess::Snapshot) => format!("{n}@release"),
        Expr::Unary(op, x, _) => format!("({op:?} {})", expr(x)),
        Expr::Binary(op, a, b, t) => format!("({} {op:?} {} : {})", expr(a), expr(b), t.name()),
        Expr::Cast(x, t) => format!("({} as {})", expr(x), t.name()),
        Expr::IfElse(c, a, b, _) => format!("(if {} then {} else {})", expr(c), expr(a), expr(b)),
        Expr::Call(n, a, _) | Expr::Builtin(n, a, _) => {
            let v: Vec<String> = a.iter().map(expr).collect();
            format!("{n}({})", v.join(", "))
        }
        Expr::IoGet(i, _) => format!("io{i}.read()"),
        Expr::Scale(v) => {
            let p: Vec<String> = v.iter().map(expr).collect();
            format!("scale({})", p.join(", "))
        }
    }
}
