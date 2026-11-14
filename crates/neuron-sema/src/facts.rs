//! Bodies of activities and functions, with the state they access and what they call.
//!
//! This is the common input of the dialect rules (A6), the time and concurrency
//! analysis (A7) and the safety checks (A8).

use crate::check::{CallTarget, Checked, FnKey};
use crate::model::{key, Program, Res, SymKind};
use crate::resolve::Resolved;
use crate::visit::{each_expr, each_stmt, seq_stmts, stmt_exprs};
use neuron_syntax::ast::{
    CircuitMember, Expr, ExprKind, FsmDecl, ItemKind, SafetyMember, Stmt, StmtKind, Widget,
};
use neuron_syntax::Span;
use std::collections::{BTreeMap, BTreeSet};

/// What kind of code a body is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// A function or method.
    Fn,
    /// A `task`.
    Task,
    /// An `on interrupt` handler.
    Interrupt,
    /// A `when` rule.
    Rule,
    /// A state machine.
    Fsm,
    /// A `sequence`.
    Sequence,
    /// A rule inside `safety:`.
    SafetyRule,
    /// `safe_state:`.
    SafeState,
    /// `on fault:`.
    OnFault,
    /// A plc `rung`.
    Rung,
    /// A `test`.
    Test,
    /// An hdl circuit.
    Circuit,
    /// A UI event handler.
    UiEvent,
}

impl BodyKind {
    /// `true` for activities that must run to completion without waiting.
    pub fn is_run_to_completion(self) -> bool {
        !matches!(self, BodyKind::Fn | BodyKind::Sequence | BodyKind::Test)
    }
}

/// A body: the statements and expressions executed by one activity or function.
#[derive(Debug)]
pub struct Body<'a> {
    /// Kind.
    pub kind: BodyKind,
    /// Name (function, task, state machine, ...).
    pub name: String,
    /// Module (file) index.
    pub module: usize,
    /// Span of the name or of the construct.
    pub span: Span,
    /// Function identity, for `BodyKind::Fn`.
    pub fn_key: Option<FnKey>,
    /// Statements at the top of each block of the body.
    pub stmts: Vec<&'a Stmt>,
    /// Expressions that belong to the body but not to a statement (guards, initializers).
    pub exprs: Vec<&'a Expr>,
    /// Index of the item that defines the body.
    pub item: usize,
}

/// A read or write of module-level state.
#[derive(Debug, Clone)]
pub struct Access {
    /// `(module, name)` of the symbol.
    pub target: (usize, String),
    /// What the symbol is.
    pub kind: SymKind,
    /// Write (or read-modify-write).
    pub write: bool,
    /// Span of the access.
    pub span: Span,
    /// Span of the top-level statement of the body that contains the access.
    pub top: Span,
}

/// What a body does.
#[derive(Debug, Default)]
pub struct BodyFacts {
    /// Accesses to shared state and signals.
    pub accesses: Vec<Access>,
    /// Calls of user functions.
    pub calls: Vec<(FnKey, Span)>,
    /// Statements and calls that wait.
    pub blocking: Vec<(Span, String)>,
}

/// All bodies of a program with their facts.
#[derive(Debug)]
pub struct FactBase<'a> {
    /// The bodies, in program order.
    pub bodies: Vec<Body<'a>>,
    /// Facts of each body (same index).
    pub facts: Vec<BodyFacts>,
    /// Index of each function body by key.
    pub fn_index: BTreeMap<FnKey, usize>,
    /// Kind of every symbol that some body accesses.
    pub kinds: BTreeMap<(usize, String), SymKind>,
}

/// `(module, name, write)` of a piece of module-level state.
pub type Effect = (usize, String, bool);

const BLOCKING_NAMES: &[&str] = &[
    "delay", "sleep", "delay_ms", "delay_us", "wait_for", "block",
];

fn push_fsm<'a>(f: &'a FsmDecl, stmts: &mut Vec<&'a Stmt>, exprs: &mut Vec<&'a Expr>) {
    for s in &f.states {
        for b in [&s.entry, &s.exit, &s.during].into_iter().flatten() {
            stmts.extend(b.stmts.iter());
        }
        for t in &s.transitions {
            if let Some(w) = &t.when {
                exprs.push(w);
            }
            if let Some(a) = &t.after {
                exprs.push(a);
            }
            if let Some(b) = &t.action {
                stmts.extend(b.stmts.iter());
            }
        }
    }
}

fn widget_events<'a>(w: &'a Widget, out: &mut Vec<(&'a str, Span, Vec<&'a Stmt>)>) {
    for (name, b) in &w.events {
        out.push((name.name.as_str(), b.span, b.stmts.iter().collect()));
    }
    for c in &w.children {
        widget_events(c, out);
    }
}

/// Enumerates the bodies of all analyzable files.
pub fn collect_bodies(prog: &Program) -> Vec<Body<'_>> {
    let mut out = Vec::new();
    for (mi, f) in prog.files.iter().enumerate() {
        if f.syntax_errors {
            continue;
        }
        for (ii, item) in f.ast.items.iter().enumerate() {
            match &item.kind {
                ItemKind::Fn(func) => {
                    let (stmts, exprs) = fn_parts(&func.body);
                    out.push(Body {
                        kind: BodyKind::Fn,
                        name: func.sig.name.name.clone(),
                        module: mi,
                        span: func.sig.name.span,
                        fn_key: Some(FnKey {
                            module: mi,
                            owner: None,
                            name: func.sig.name.name.clone(),
                        }),
                        stmts,
                        exprs,
                        item: ii,
                    });
                }
                ItemKind::Class(c) => {
                    for m in &c.methods {
                        if let ItemKind::Fn(func) = &m.kind {
                            let (stmts, exprs) = fn_parts(&func.body);
                            out.push(Body {
                                kind: BodyKind::Fn,
                                name: format!("{}.{}", c.name.name, func.sig.name.name),
                                module: mi,
                                span: func.sig.name.span,
                                fn_key: Some(FnKey {
                                    module: mi,
                                    owner: Some(c.name.name.clone()),
                                    name: func.sig.name.name.clone(),
                                }),
                                stmts,
                                exprs,
                                item: ii,
                            });
                        }
                    }
                }
                ItemKind::Task(t) => out.push(Body {
                    kind: BodyKind::Task,
                    name: t.name.name.clone(),
                    module: mi,
                    span: t.name.span,
                    fn_key: None,
                    stmts: t.body.stmts.iter().collect(),
                    exprs: Vec::new(),
                    item: ii,
                }),
                ItemKind::Interrupt { body, source, .. } => out.push(Body {
                    kind: BodyKind::Interrupt,
                    name: expr_text(source),
                    module: mi,
                    span: source.span,
                    fn_key: None,
                    stmts: body.stmts.iter().collect(),
                    exprs: Vec::new(),
                    item: ii,
                }),
                ItemKind::Rule(r) => out.push(Body {
                    kind: BodyKind::Rule,
                    name: "when".to_string(),
                    module: mi,
                    span: r.span,
                    fn_key: None,
                    stmts: r.body.stmts.iter().collect(),
                    exprs: vec![&r.cond],
                    item: ii,
                }),
                ItemKind::Fsm(fsm) => {
                    let (mut stmts, mut exprs) = (Vec::new(), Vec::new());
                    push_fsm(fsm, &mut stmts, &mut exprs);
                    out.push(Body {
                        kind: BodyKind::Fsm,
                        name: fsm.name.name.clone(),
                        module: mi,
                        span: fsm.name.span,
                        fn_key: None,
                        stmts,
                        exprs,
                        item: ii,
                    });
                }
                ItemKind::Sequence(s) => {
                    let mut stmts = Vec::new();
                    seq_stmts(&s.items, &mut stmts);
                    out.push(Body {
                        kind: BodyKind::Sequence,
                        name: s.name.name.clone(),
                        module: mi,
                        span: s.name.span,
                        fn_key: None,
                        stmts,
                        exprs: Vec::new(),
                        item: ii,
                    });
                }
                ItemKind::Safety(members) => {
                    for m in members {
                        match &m.node {
                            SafetyMember::Rule(r) => out.push(Body {
                                kind: BodyKind::SafetyRule,
                                name: "safety rule".to_string(),
                                module: mi,
                                span: r.span,
                                fn_key: None,
                                stmts: r.body.stmts.iter().collect(),
                                exprs: vec![&r.cond],
                                item: ii,
                            }),
                            SafetyMember::SafeState(b) => out.push(Body {
                                kind: BodyKind::SafeState,
                                name: "safe_state".to_string(),
                                module: mi,
                                span: m.span,
                                fn_key: None,
                                stmts: b.stmts.iter().collect(),
                                exprs: Vec::new(),
                                item: ii,
                            }),
                            SafetyMember::OnFault { body, .. } => out.push(Body {
                                kind: BodyKind::OnFault,
                                name: "on fault".to_string(),
                                module: mi,
                                span: m.span,
                                fn_key: None,
                                stmts: body.stmts.iter().collect(),
                                exprs: Vec::new(),
                                item: ii,
                            }),
                            _ => {}
                        }
                    }
                }
                ItemKind::Rung { name, body } => out.push(Body {
                    kind: BodyKind::Rung,
                    name: name.name.clone(),
                    module: mi,
                    span: name.span,
                    fn_key: None,
                    stmts: body.stmts.iter().collect(),
                    exprs: Vec::new(),
                    item: ii,
                }),
                ItemKind::Test { name, body } => out.push(Body {
                    kind: BodyKind::Test,
                    name: name.clone(),
                    module: mi,
                    span: body.span,
                    fn_key: None,
                    stmts: body.stmts.iter().collect(),
                    exprs: Vec::new(),
                    item: ii,
                }),
                ItemKind::Circuit(c) => {
                    let (mut stmts, mut exprs) = (Vec::new(), Vec::new());
                    for m in &c.members {
                        match &m.node {
                            CircuitMember::Comb(b) => stmts.extend(b.stmts.iter()),
                            CircuitMember::OnEdge { body, .. } => stmts.extend(body.stmts.iter()),
                            CircuitMember::Fsm(fsm) => push_fsm(fsm, &mut stmts, &mut exprs),
                            CircuitMember::Reg { init: Some(e), .. } => exprs.push(e),
                            CircuitMember::Const { value, .. } => exprs.push(value),
                            _ => {}
                        }
                    }
                    out.push(Body {
                        kind: BodyKind::Circuit,
                        name: c.name.name.clone(),
                        module: mi,
                        span: c.name.span,
                        fn_key: None,
                        stmts,
                        exprs,
                        item: ii,
                    });
                }
                ItemKind::Screen { widgets, .. } => {
                    let mut evs = Vec::new();
                    for w in widgets {
                        widget_events(w, &mut evs);
                    }
                    for (name, span, stmts) in evs {
                        out.push(Body {
                            kind: BodyKind::UiEvent,
                            name: name.to_string(),
                            module: mi,
                            span,
                            fn_key: None,
                            stmts,
                            exprs: Vec::new(),
                            item: ii,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn fn_parts(body: &neuron_syntax::ast::FnBody) -> (Vec<&Stmt>, Vec<&Expr>) {
    match body {
        neuron_syntax::ast::FnBody::Block(b) => (b.stmts.iter().collect(), Vec::new()),
        neuron_syntax::ast::FnBody::Expr(e) => (Vec::new(), vec![e]),
    }
}

/// Source-like text of simple expressions (used to name interrupts).
pub fn expr_text(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Name(id) => id.name.clone(),
        ExprKind::Field(b, id) => format!("{}.{}", expr_text(b), id.name),
        ExprKind::Call(c, _) => expr_text(c),
        _ => "interrupt".to_string(),
    }
}

/// The identifier at the root of a place expression.
pub fn root_name(e: &Expr) -> Option<&neuron_syntax::ast::Ident> {
    match &e.kind {
        ExprKind::Name(id) => Some(id),
        ExprKind::Field(b, _) | ExprKind::Index(b, _, _) => root_name(b),
        ExprKind::Call(c, _) => root_name(c),
        _ => None,
    }
}

fn is_tracked(kind: SymKind) -> bool {
    kind.is_shared_state() || kind == SymKind::Signal
}

fn io_reads(method: &str) -> bool {
    matches!(
        method,
        "read" | "ready" | "byte" | "rising" | "falling" | "changed"
    )
}

/// Computes the facts of one body.
pub fn body_facts(
    prog: &Program,
    resolved: &Resolved,
    checked: &Checked,
    body: &Body<'_>,
) -> BodyFacts {
    let mi = body.module;
    let mut facts = BodyFacts::default();
    let mut tops: Vec<(&Stmt, Span)> = Vec::new();
    for &s in &body.stmts {
        tops.push((s, s.span));
    }
    let mut per_top: Vec<(Vec<&Stmt>, Span)> = Vec::new();
    for (s, span) in tops {
        let mut nested: Vec<&Stmt> = Vec::new();
        each_stmt(std::slice::from_ref(s), &mut |x| nested.push(x));
        per_top.push((nested, span));
    }
    // Guard expressions that belong to no statement.
    let mut groups: Vec<(Vec<&Expr>, Vec<&Stmt>, Span)> = Vec::new();
    if !body.exprs.is_empty() {
        groups.push((body.exprs.clone(), Vec::new(), body.span));
    }
    for (nested, span) in per_top {
        groups.push((Vec::new(), nested, span));
    }
    for (extra, stmts, top) in groups {
        let mut write_roots: BTreeMap<(usize, usize), bool> = BTreeMap::new();
        let mut exprs: Vec<&Expr> = extra;
        for &s in &stmts {
            match &s.kind {
                StmtKind::Assign { place, .. } => {
                    if let Some(r) = root_name(place) {
                        write_roots.insert((r.span.start, r.span.end), true);
                    }
                }
                StmtKind::Coil { place, .. } => {
                    if let Some(r) = root_name(place) {
                        write_roots.insert((r.span.start, r.span.end), true);
                    }
                }
                StmtKind::Wait(_) => facts.blocking.push((s.span, "wait".to_string())),
                StmtKind::Until { .. } => facts.blocking.push((s.span, "until".to_string())),
                StmtKind::Jump(_) => facts.blocking.push((s.span, "->".to_string())),
                StmtKind::Move(_) => facts.blocking.push((s.span, "move".to_string())),
                _ => {}
            }
            stmt_exprs(s, &mut |e| exprs.push(e));
        }
        // Calls decide whether a receiver is written.
        let mut visited: Vec<&Expr> = Vec::new();
        for e in exprs {
            each_expr(e, &mut |x| visited.push(x));
        }
        for x in &visited {
            if let ExprKind::Call(callee, _) = &x.kind {
                if let ExprKind::Field(recv, m) = &callee.kind {
                    if let Some(r) = root_name(recv) {
                        let target = checked.calls.get(&key(mi, x.span));
                        let write = match target {
                            Some(CallTarget::Method { mut_self, .. }) => *mut_self,
                            Some(CallTarget::Io { method, .. }) => !io_reads(method),
                            Some(CallTarget::Fn(_)) => false,
                            _ => !io_reads(&m.name),
                        };
                        if write {
                            write_roots.insert((r.span.start, r.span.end), true);
                        }
                    }
                }
                match checked.calls.get(&key(mi, x.span)) {
                    Some(CallTarget::Fn(k)) | Some(CallTarget::Method { key: k, .. }) => {
                        facts.calls.push((k.clone(), x.span));
                    }
                    _ => {}
                }
                let callee_name = match &callee.kind {
                    ExprKind::Name(id) | ExprKind::Field(_, id) => Some(id.name.as_str()),
                    _ => None,
                };
                if let Some(n) = callee_name {
                    let user = matches!(
                        checked.calls.get(&key(mi, x.span)),
                        Some(CallTarget::Fn(_) | CallTarget::Method { .. })
                    );
                    if !user && BLOCKING_NAMES.contains(&n) {
                        facts.blocking.push((x.span, n.to_string()));
                    }
                }
            }
        }
        for x in &visited {
            let ExprKind::Name(id) = &x.kind else {
                continue;
            };
            let Some(Res::Item { module, name }) = resolved.res.get(&key(mi, id.span)) else {
                continue;
            };
            let Some(sym) = prog.symbol(*module, name) else {
                continue;
            };
            if !is_tracked(sym.kind) {
                continue;
            }
            let write = write_roots.contains_key(&(id.span.start, id.span.end));
            facts.accesses.push(Access {
                target: (*module, name.clone()),
                kind: sym.kind,
                write,
                span: id.span,
                top,
            });
        }
    }
    facts
}

/// Builds the bodies and facts of a program.
pub fn build<'a>(prog: &'a Program, resolved: &Resolved, checked: &Checked) -> FactBase<'a> {
    let bodies = collect_bodies(prog);
    let facts: Vec<BodyFacts> = bodies
        .iter()
        .map(|b| body_facts(prog, resolved, checked, b))
        .collect();
    let mut fn_index = BTreeMap::new();
    for (i, b) in bodies.iter().enumerate() {
        if let Some(k) = &b.fn_key {
            fn_index.insert(k.clone(), i);
        }
    }
    let mut kinds = BTreeMap::new();
    for f in &facts {
        for a in &f.accesses {
            kinds.insert(a.target.clone(), a.kind);
        }
    }
    FactBase {
        bodies,
        facts,
        fn_index,
        kinds,
    }
}

impl FactBase<'_> {
    /// For every function, the shared state it reads or writes, including through calls.
    pub fn fn_access_closure(&self) -> BTreeMap<FnKey, BTreeSet<(usize, String)>> {
        self.fn_effects_closure()
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    v.into_iter()
                        .filter(|(m, n, _)| {
                            self.kinds.get(&(*m, n.clone())).copied() != Some(SymKind::Signal)
                        })
                        .map(|(m, n, _)| (m, n))
                        .collect(),
                )
            })
            .collect()
    }

    /// For every function, `(module, name, kind, write)` of the state it touches,
    /// including through calls.
    pub fn fn_effects_closure(&self) -> BTreeMap<FnKey, BTreeSet<Effect>> {
        let mut acc: BTreeMap<FnKey, BTreeSet<Effect>> = BTreeMap::new();
        for (i, b) in self.bodies.iter().enumerate() {
            if let Some(k) = &b.fn_key {
                let set = self.facts[i]
                    .accesses
                    .iter()
                    .map(|a| (a.target.0, a.target.1.clone(), a.write))
                    .collect();
                acc.insert(k.clone(), set);
            }
        }
        loop {
            let mut changed = false;
            for (i, b) in self.bodies.iter().enumerate() {
                let Some(k) = &b.fn_key else {
                    continue;
                };
                for (callee, _) in &self.facts[i].calls {
                    let extra: Vec<Effect> = acc
                        .get(callee)
                        .map(|s| s.iter().cloned().collect())
                        .unwrap_or_default();
                    if let Some(mine) = acc.get_mut(k) {
                        for t in extra {
                            changed |= mine.insert(t);
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        acc
    }
}
