//! Explicit-state model checker for `fsm` and `sequence` (task A8, spec 15 §15.6).
//!
//! State machines and sequences are compiled to *automata* with boolean guards.
//! Everything the automata do not define (inputs, sensors, comparisons such as
//! `temp > 90 degC`) is an *atom*: a boolean that the environment may change at
//! every tick. Time is discretized with a tick equal to the greatest common divisor
//! of all durations. A property is checked by breadth-first search over the product
//! of the automata, the valuations of the atoms and a small monitor, so the first
//! counterexample found is a shortest one.

use crate::consteval::Consts;
use crate::model::{key, FileDiag, Program, Res};
use crate::report::{Counterexample, TraceStep};
use crate::resolve::Resolved;
use crate::visit::each_stmt;
use crate::SemaOptions;
use neuron_syntax::ast::{
    BinOp, CircuitMember, Expr, ExprKind, FsmDecl, Ident, ItemKind, Lit, Property, SeqItem,
    SequenceDecl, Stmt, StmtKind, UnOp,
};
use neuron_syntax::{Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};

/// Boolean guard over atoms and automaton states.
#[derive(Debug, Clone)]
enum G {
    True,
    False,
    Atom(usize),
    In(usize, usize),
    Not(Box<G>),
    And(Box<G>, Box<G>),
    Or(Box<G>, Box<G>),
}

impl G {
    fn mentions_state(&self) -> bool {
        match self {
            G::In(..) => true,
            G::Not(a) => a.mentions_state(),
            G::And(a, b) | G::Or(a, b) => a.mentions_state() || b.mentions_state(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
struct Trans {
    to: usize,
    guard: Option<G>,
    after: Option<u64>,
}

#[derive(Debug, Clone)]
struct Comp {
    name: String,
    states: Vec<String>,
    init: usize,
    trans: Vec<Vec<Trans>>,
    cap: u64,
}

#[derive(Debug, Clone)]
enum Prop {
    Never(G),
    Always(G, Option<G>),
    Within { ticks: u64, after: G, goal: G },
}

fn eval(g: &G, cfg: &[u16], val: u32) -> bool {
    match g {
        G::True => true,
        G::False => false,
        G::Atom(i) => val & (1 << i) != 0,
        G::In(c, s) => usize::from(cfg[*c]) == *s,
        G::Not(a) => !eval(a, cfg, val),
        G::And(a, b) => eval(a, cfg, val) && eval(b, cfg, val),
        G::Or(a, b) => eval(a, cfg, val) || eval(b, cfg, val),
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

struct Build<'a> {
    prog: &'a Program,
    resolved: &'a Resolved,
    consts: Consts<'a>,
    mi: usize,
    atoms: Vec<String>,
    comps: Vec<Comp>,
    comp_of_fsm: BTreeMap<String, usize>,
    durations: Vec<u64>,
}

struct Segment<'a> {
    name: String,
    span: Span,
    initial: bool,
    stmts: Vec<&'a Stmt>,
    par: Option<Vec<(String, Vec<Segment<'a>>)>>,
}

fn build_segments<'a>(items: &'a [SeqItem], counter: &mut usize) -> Vec<Segment<'a>> {
    fn flush<'a>(cur: Segment<'a>, segs: &mut Vec<Segment<'a>>, keep_empty: bool) {
        if keep_empty || !cur.stmts.is_empty() {
            segs.push(cur);
        }
    }
    fn walk<'a>(
        items: &'a [SeqItem],
        segs: &mut Vec<Segment<'a>>,
        cur: &mut Segment<'a>,
        keep_cur: &mut bool,
        counter: &mut usize,
    ) {
        for it in items {
            match it {
                SeqItem::Step {
                    initial,
                    name,
                    body,
                    span,
                } => {
                    let old = std::mem::replace(
                        cur,
                        Segment {
                            name: name.name.clone(),
                            span: name.span,
                            initial: *initial,
                            stmts: Vec::new(),
                            par: None,
                        },
                    );
                    flush(old, segs, *keep_cur);
                    *keep_cur = true;
                    let _ = span;
                    walk(body, segs, cur, keep_cur, counter);
                }
                SeqItem::Parallel { branches, span } => {
                    *counter += 1;
                    let n = *counter;
                    let old = std::mem::replace(
                        cur,
                        Segment {
                            name: format!("after parallel#{n}"),
                            span: *span,
                            initial: false,
                            stmts: Vec::new(),
                            par: None,
                        },
                    );
                    flush(old, segs, *keep_cur);
                    *keep_cur = false;
                    let bs = branches
                        .iter()
                        .map(|(id, b)| (id.name.clone(), build_segments(b, counter)))
                        .collect();
                    segs.push(Segment {
                        name: format!("parallel#{n}"),
                        span: *span,
                        initial: false,
                        stmts: Vec::new(),
                        par: Some(bs),
                    });
                }
                SeqItem::Stmt(s) => cur.stmts.push(s),
            }
        }
    }
    let mut segs = Vec::new();
    let mut cur = Segment {
        name: "start".to_string(),
        span: Span::default(),
        initial: false,
        stmts: Vec::new(),
        par: None,
    };
    let mut keep = false;
    walk(items, &mut segs, &mut cur, &mut keep, counter);
    flush(cur, &mut segs, keep);
    segs
}

impl Build<'_> {
    fn text(&self, e: &Expr) -> String {
        let src = &self.prog.files[self.mi].text;
        e.span
            .text(src)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn atom(&mut self, e: &Expr) -> G {
        let t = self.text(e);
        let idx = match self.atoms.iter().position(|a| *a == t) {
            Some(i) => i,
            None => {
                self.atoms.push(t);
                self.atoms.len() - 1
            }
        };
        G::Atom(idx)
    }

    fn state_ref(&self, id: &Ident) -> Option<G> {
        match self.resolved.res.get(&key(self.mi, id.span))? {
            Res::State { fsm, state } => {
                let c = *self.comp_of_fsm.get(fsm)?;
                let s = self.comps[c].states.iter().position(|x| x == state)?;
                Some(G::In(c, s))
            }
            _ => None,
        }
    }

    fn guard(&mut self, e: &Expr) -> G {
        match &e.kind {
            ExprKind::Lit(Lit::Bool(true)) => G::True,
            ExprKind::Lit(Lit::Bool(false)) => G::False,
            ExprKind::Unary(UnOp::Not, x) => G::Not(Box::new(self.guard(x))),
            ExprKind::Binary(BinOp::And, a, b) => {
                G::And(Box::new(self.guard(a)), Box::new(self.guard(b)))
            }
            ExprKind::Binary(BinOp::Or, a, b) => {
                G::Or(Box::new(self.guard(a)), Box::new(self.guard(b)))
            }
            ExprKind::Name(id) => match self.state_ref(id) {
                Some(g) => g,
                None => self.atom(e),
            },
            ExprKind::Field(_, id) => match self.state_ref(id) {
                Some(g) => g,
                None => self.atom(e),
            },
            _ => self.atom(e),
        }
    }

    fn duration(&mut self, e: &Expr) -> Option<u64> {
        let d = self.consts.duration_ns(self.mi, e)?;
        let d = u64::try_from(d).ok().filter(|d| *d > 0)?;
        self.durations.push(d);
        Some(d)
    }
}

/// Raw (pre-tick) transition used while compiling.
struct RawTrans {
    to: usize,
    guard: Option<G>,
    after_ns: Option<u64>,
}

struct RawComp {
    name: String,
    states: Vec<String>,
    init: usize,
    trans: Vec<Vec<RawTrans>>,
}

fn fmt_time(ns: u64) -> String {
    if ns % 1_000_000 == 0 {
        format!("{} ms", ns / 1_000_000)
    } else if ns % 1_000 == 0 {
        format!("{} us", ns / 1_000)
    } else {
        format!("{ns} ns")
    }
}

/// States created by the compiler (`start`, `idle`, `done`) have an empty span and are
/// never reported by the static checks.
fn is_synthetic(spans: &[Span], state: usize) -> bool {
    spans.get(state).is_none_or(|s| *s == Span::default())
}

fn contains_fault(stmts: &[&Stmt]) -> bool {
    let mut found = false;
    for s in stmts {
        each_stmt(std::slice::from_ref(*s), &mut |x| {
            if matches!(x.kind, StmtKind::Fault { .. }) {
                found = true;
            }
        });
    }
    found
}

fn block_has_fault(b: &neuron_syntax::ast::Block) -> bool {
    let v: Vec<&Stmt> = b.stmts.iter().collect();
    contains_fault(&v)
}

/// Verification of one module: static checks, then the `verify` properties.
pub fn verify_module(
    prog: &Program,
    resolved: &Resolved,
    mi: usize,
    opts: &SemaOptions,
    diags: &mut Vec<FileDiag>,
    out: &mut Vec<Counterexample>,
) {
    let file = &prog.files[mi];
    let mut b = Build {
        prog,
        resolved,
        consts: Consts::new(prog, &resolved.res),
        mi,
        atoms: Vec::new(),
        comps: Vec::new(),
        comp_of_fsm: BTreeMap::new(),
        durations: Vec::new(),
    };
    fn push_err(
        diags: &mut Vec<FileDiag>,
        file: usize,
        code: &'static str,
        msg: String,
        span: Span,
        help: Option<&str>,
    ) {
        let mut d = Diagnostic::error(code, msg, span);
        if let Some(h) = help {
            d = d.with_help(h);
        }
        diags.push(FileDiag { file, diag: d });
    }

    // ---- collect fsms and sequences, register component names first ----
    let mut fsms: Vec<&FsmDecl> = Vec::new();
    let mut seqs: Vec<&SequenceDecl> = Vec::new();
    for it in &file.ast.items {
        match &it.kind {
            ItemKind::Fsm(f) => fsms.push(f),
            ItemKind::Circuit(c) => {
                for m in &c.members {
                    if let CircuitMember::Fsm(f) = &m.node {
                        fsms.push(f);
                    }
                }
            }
            ItemKind::Sequence(s) => seqs.push(s),
            _ => {}
        }
    }
    let mut raws: Vec<RawComp> = Vec::new();
    let mut spans: Vec<Vec<Span>> = Vec::new();
    let mut final_flags: Vec<Vec<bool>> = Vec::new();
    // Placeholder components so that `In(comp, state)` can be resolved while compiling guards.
    for f in &fsms {
        let ci = b.comps.len();
        b.comp_of_fsm.insert(f.name.name.clone(), ci);
        b.comps.push(Comp {
            name: f.name.name.clone(),
            states: f.states.iter().map(|s| s.name.name.clone()).collect(),
            init: 0,
            trans: Vec::new(),
            cap: 1,
        });
    }
    for f in &fsms {
        let initials: Vec<usize> = f
            .states
            .iter()
            .enumerate()
            .filter(|(_, s)| s.initial)
            .map(|(i, _)| i)
            .collect();
        if !f.states.is_empty() && initials.len() != 1 {
            push_err(
                diags,
                mi,
                "NE0904",
                format!(
                    "state machine `{}` must have exactly one `initial` state, found {}",
                    f.name.name,
                    initials.len()
                ),
                f.name.span,
                Some("mark one state with `initial state`"),
            );
        }
        let idx_of = |n: &str| f.states.iter().position(|s| s.name.name == n);
        let mut trans: Vec<Vec<RawTrans>> = Vec::new();
        let mut finals = Vec::new();
        for s in &f.states {
            let faulty = [&s.entry, &s.during]
                .into_iter()
                .flatten()
                .any(block_has_fault);
            finals.push(s.is_final || faulty);
            let mut ts = Vec::new();
            for t in &s.transitions {
                let Some(to) = idx_of(&t.target.name) else {
                    continue;
                };
                let guard = t.when.as_ref().map(|w| b.guard(w));
                let after = t.after.as_ref().and_then(|a| b.duration(a));
                let guard = match (&t.after, after, guard) {
                    (Some(a), None, Some(g)) => Some(G::Or(Box::new(g), Box::new(b.atom(a)))),
                    (Some(a), None, None) => Some(b.atom(a)),
                    (_, _, g) => g,
                };
                ts.push(RawTrans {
                    to,
                    guard,
                    after_ns: after,
                });
            }
            trans.push(ts);
        }
        spans.push(f.states.iter().map(|s| s.name.span).collect());
        final_flags.push(finals);
        raws.push(RawComp {
            name: f.name.name.clone(),
            states: f.states.iter().map(|s| s.name.name.clone()).collect(),
            init: initials.first().copied().unwrap_or(0),
            trans,
        });
    }
    for s in &seqs {
        let mut counter = 0usize;
        let segs = build_segments(&s.items, &mut counter);
        compile_sequence(
            &mut b,
            &s.name.name,
            &segs,
            &mut raws,
            &mut spans,
            &mut final_flags,
            None,
        );
    }

    // ---- static checks: NE0903 deadlock, NE0902 unreachable ----
    for (ci, rc) in raws.iter().enumerate() {
        if rc.states.is_empty() {
            continue;
        }
        for (si, ts) in rc.trans.iter().enumerate() {
            if ts.is_empty() && !final_flags[ci][si] && !is_synthetic(&spans[ci], si) {
                let span = spans[ci].get(si).copied().unwrap_or_default();
                push_err(
                    diags,
                    mi,
                    "NE0903",
                    format!(
                        "state `{}.{}` has no way out and is not `final`",
                        rc.name, rc.states[si]
                    ),
                    span,
                    Some("add a transition, or mark the state `final`"),
                );
            }
        }
        let mut seen = vec![false; rc.states.len()];
        let mut stack = vec![rc.init];
        while let Some(s) = stack.pop() {
            if seen[s] {
                continue;
            }
            seen[s] = true;
            for t in &rc.trans[s] {
                let dead = matches!(&t.guard, Some(G::False)) && t.after_ns.is_none();
                if !dead && !seen[t.to] {
                    stack.push(t.to);
                }
            }
        }
        for (si, ok) in seen.iter().enumerate() {
            if !ok && !is_synthetic(&spans[ci], si) {
                let span = spans[ci].get(si).copied().unwrap_or_default();
                push_err(
                    diags,
                    mi,
                    "NE0902",
                    format!("state `{}.{}` can never be reached", rc.name, rc.states[si]),
                    span,
                    Some("add a transition that leads to it, or remove it"),
                );
            }
        }
    }
    // Sequence steps blocked forever: `until false`.
    for s in &seqs {
        let mut all: Vec<&Stmt> = Vec::new();
        let mut v = Vec::new();
        crate::visit::seq_stmts(&s.items, &mut v);
        all.extend(v);
        for st in all {
            if let StmtKind::Until {
                cond,
                timeout: None,
                ..
            } = &st.kind
            {
                if matches!(&cond.kind, ExprKind::Lit(Lit::Bool(false))) {
                    push_err(
                        diags,
                        mi,
                        "NE0903",
                        format!("`until false` in `{}` waits forever", s.name.name),
                        st.span,
                        Some("add a `timeout`, or a condition that can become true"),
                    );
                }
            }
        }
    }

    // ---- properties ----
    let mut props: Vec<(&neuron_syntax::ast::Spanned<Property>, Prop)> = Vec::new();
    let mut prop_durations = Vec::new();
    for it in &file.ast.items {
        if let ItemKind::Verify(ps) = &it.kind {
            for p in ps {
                let prop = match &p.node {
                    Property::Never(e) => Prop::Never(b.guard(e)),
                    Property::Always { cond, implies } => {
                        let c = b.guard(cond);
                        Prop::Always(c, implies.as_ref().map(|i| b.guard(i)))
                    }
                    Property::EventuallyWithin {
                        within,
                        after,
                        goal,
                    } => {
                        let a = b.guard(after);
                        let g = b.guard(goal);
                        match b
                            .consts
                            .duration_ns(mi, within)
                            .and_then(|d| u64::try_from(d).ok())
                        {
                            Some(d) if d > 0 => {
                                prop_durations.push(d);
                                b.durations.push(d);
                                Prop::Within {
                                    ticks: d,
                                    after: a,
                                    goal: g,
                                }
                            }
                            _ => {
                                push_err(
                                    diags,
                                    mi,
                                    "NE0407",
                                    "the bound of `eventually_within` must be a constant duration"
                                        .to_string(),
                                    within.span,
                                    None,
                                );
                                continue;
                            }
                        }
                    }
                };
                props.push((p, prop));
            }
        }
    }
    if props.is_empty() {
        return;
    }

    // ---- time discretization ----
    // Without any duration in the module a tick has no natural size: use 1 ms.
    let mut tick = match b.durations.iter().copied().fold(0, gcd) {
        0 => 1_000_000,
        g => g,
    };
    let max_d = b.durations.iter().copied().max().unwrap_or(1);
    let mut coarse = false;
    if max_d / tick > 4096 {
        tick *= max_d.div_ceil(tick).div_ceil(4096);
        coarse = true;
    }
    let ticks_of = |ns: u64| ns.div_ceil(tick);
    let mut comps: Vec<Comp> = Vec::new();
    for rc in &raws {
        let mut cap = 1;
        let trans: Vec<Vec<Trans>> = rc
            .trans
            .iter()
            .map(|ts| {
                ts.iter()
                    .map(|t| {
                        let after = t.after_ns.map(ticks_of);
                        if let Some(a) = after {
                            cap = cap.max(a + 1);
                        }
                        Trans {
                            to: t.to,
                            guard: t.guard.clone(),
                            after,
                        }
                    })
                    .collect()
            })
            .collect();
        // A machine without states still occupies a slot so that indices stay aligned.
        let mut states = rc.states.clone();
        if states.is_empty() {
            states.push("<empty>".to_string());
        }
        let mut trans = trans;
        trans.resize_with(states.len(), Vec::new);
        comps.push(Comp {
            name: rc.name.clone(),
            states,
            init: rc.init.min(rc.states.len().saturating_sub(1)),
            trans,
            cap,
        });
    }
    if coarse {
        diags.push(FileDiag {
            file: mi,
            diag: Diagnostic::warning(
                "NL0901",
                format!("durations were rounded up to a verification tick of {tick} ns"),
                file.ast.header.span,
            ),
        });
    }
    let atoms = b.atoms.clone();
    for (p, prop) in props {
        let text = p
            .span
            .text(&file.text)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let mentions = match &prop {
            Prop::Never(g) => g.mentions_state(),
            Prop::Always(g, i) => g.mentions_state() || i.as_ref().is_some_and(G::mentions_state),
            Prop::Within { after, goal, .. } => after.mentions_state() || goal.mentions_state(),
        };
        if !mentions {
            diags.push(FileDiag {
                file: mi,
                diag: Diagnostic::warning(
                    "NL0901",
                    format!("`{text}` mentions only external inputs, so it cannot be verified without a model of the plant"),
                    p.span,
                ),
            });
            continue;
        }
        let prop = match prop {
            Prop::Within { ticks, after, goal } => Prop::Within {
                ticks: ticks_of(ticks).max(1),
                after,
                goal,
            },
            other => other,
        };
        match check(&comps, &atoms, &prop, tick, opts) {
            Outcome::Holds => {}
            Outcome::Inconclusive(why) => diags.push(FileDiag {
                file: mi,
                diag: Diagnostic::warning(
                    "NL0901",
                    format!("`{text}` could not be decided: {why}"),
                    p.span,
                ),
            }),
            Outcome::Violated(trace) => {
                let summary: Vec<String> = trace
                    .iter()
                    .map(|s| format!("t={} [{}]", fmt_time(s.time_ns), s.states.join(", ")))
                    .collect();
                diags.push(FileDiag {
                    file: mi,
                    diag: Diagnostic::error(
                        "NE0901",
                        format!(
                            "property `{text}` is violated after {} tick(s)",
                            trace.len().saturating_sub(1)
                        ),
                        p.span,
                    )
                    .with_help(format!("counterexample: {}", summary.join(" -> "))),
                });
                out.push(Counterexample {
                    module: prog.modules[mi].name.clone(),
                    property: text,
                    trace,
                });
            }
        }
    }
}

fn compile_sequence(
    b: &mut Build<'_>,
    name: &str,
    segs: &[Segment<'_>],
    raws: &mut Vec<RawComp>,
    spans: &mut Vec<Vec<Span>>,
    finals: &mut Vec<Vec<bool>>,
    start_guard: Option<G>,
) -> usize {
    // State layout: optional `idle`, the segments, then `done`.
    let idle = usize::from(start_guard.is_some());
    let ci = raws.len();
    let mut names: Vec<String> = Vec::new();
    if start_guard.is_some() {
        names.push("idle".to_string());
    }
    names.extend(segs.iter().map(|s| s.name.clone()));
    names.push("done".to_string());
    let done = names.len() - 1;
    let seg_state = |i: usize| idle + i;
    // Reserve the component so that parallel branches can refer to it.
    raws.push(RawComp {
        name: name.to_string(),
        states: names.clone(),
        init: if idle == 1 {
            0
        } else {
            segs.iter().position(|s| s.initial).map_or(0, seg_state)
        },
        trans: Vec::new(),
    });
    spans.push(Vec::new());
    finals.push(Vec::new());
    let lookup = |n: &str| segs.iter().position(|s| s.name == n).map(seg_state);
    let mut trans: Vec<Vec<RawTrans>> = Vec::new();
    let mut st_spans: Vec<Span> = Vec::new();
    let mut st_final: Vec<bool> = Vec::new();
    if let Some(g) = &start_guard {
        trans.push(vec![RawTrans {
            to: seg_state(0).min(done),
            guard: Some(g.clone()),
            after_ns: None,
        }]);
        st_spans.push(Span::default());
        st_final.push(false);
    }
    for (i, seg) in segs.iter().enumerate() {
        let next = if i + 1 < segs.len() {
            seg_state(i + 1)
        } else {
            done
        };
        let mut ts: Vec<RawTrans> = Vec::new();
        let faulty = contains_fault(&seg.stmts);
        st_spans.push(seg.span);
        st_final.push(faulty);
        if let Some(branches) = &seg.par {
            let state = seg_state(i);
            let mut done_guards: Vec<G> = Vec::new();
            for (bn, bsegs) in branches {
                let mut counter = 0usize;
                let _ = &mut counter;
                let bci = compile_sequence(
                    b,
                    &format!("{name}.{bn}"),
                    bsegs,
                    raws,
                    spans,
                    finals,
                    Some(G::In(ci, state)),
                );
                let bdone = raws[bci].states.len() - 1;
                done_guards.push(G::In(bci, bdone));
            }
            let all = done_guards
                .into_iter()
                .reduce(|a, c| G::And(Box::new(a), Box::new(c)))
                .unwrap_or(G::True);
            ts.push(RawTrans {
                to: next,
                guard: Some(all),
                after_ns: None,
            });
            trans.push(ts);
            continue;
        }
        let mut conds: Vec<G> = Vec::new();
        let mut timeouts: Vec<RawTrans> = Vec::new();
        let mut wait_ns: Option<u64> = None;
        let mut jump: Option<usize> = None;
        for s in &seg.stmts {
            match &s.kind {
                StmtKind::Until {
                    cond,
                    timeout,
                    on_timeout,
                } => {
                    conds.push(b.guard(cond));
                    if let Some(t) = timeout {
                        let to = on_timeout
                            .as_ref()
                            .and_then(|id| lookup(&id.name))
                            .unwrap_or(done);
                        let ns = b.duration(t);
                        let (guard, after_ns) = match ns {
                            Some(_) => (None, ns),
                            None => (Some(b.atom(t)), None),
                        };
                        timeouts.push(RawTrans {
                            to,
                            guard,
                            after_ns,
                        });
                    }
                }
                StmtKind::Wait(e) => {
                    let d = b.duration(e);
                    wait_ns = match (wait_ns, d) {
                        (Some(a), Some(c)) => Some(a + c),
                        (None, d) => d,
                        (a, None) => a,
                    };
                }
                StmtKind::Jump(id) => jump = lookup(&id.name),
                _ => {}
            }
        }
        if !faulty {
            let to = jump.unwrap_or(next);
            let main = if !conds.is_empty() {
                let g = conds
                    .into_iter()
                    .reduce(|a, c| G::And(Box::new(a), Box::new(c)));
                RawTrans {
                    to,
                    guard: g,
                    after_ns: None,
                }
            } else if wait_ns.is_some() {
                RawTrans {
                    to,
                    guard: None,
                    after_ns: wait_ns,
                }
            } else {
                RawTrans {
                    to,
                    guard: None,
                    after_ns: None,
                }
            };
            ts.push(main);
            ts.extend(timeouts);
        }
        trans.push(ts);
    }
    // `done`: restart for branches, terminal otherwise.
    if let Some(g) = &start_guard {
        trans.push(vec![RawTrans {
            to: 0,
            guard: Some(G::Not(Box::new(g.clone()))),
            after_ns: None,
        }]);
        st_final.push(false);
    } else {
        trans.push(Vec::new());
        st_final.push(true);
    }
    st_spans.push(Span::default());
    raws[ci].trans = trans;
    spans[ci] = st_spans;
    finals[ci] = st_final;
    ci
}

enum Outcome {
    Holds,
    Violated(Vec<TraceStep>),
    Inconclusive(String),
}

fn step(comps: &[Comp], cfg: &[u16], val: u32) -> Vec<u16> {
    let n = comps.len();
    let mut next = cfg.to_vec();
    for (ci, c) in comps.iter().enumerate() {
        let s = usize::from(cfg[ci]);
        let dwell = u64::from(cfg[n + ci]);
        let mut fired = None;
        for t in &c.trans[s] {
            let g = t.guard.as_ref().map(|g| eval(g, cfg, val));
            let a = t.after.map(|a| dwell + 1 >= a);
            let enabled = match (g, a) {
                (None, None) => true,
                (Some(x), None) => x,
                (None, Some(y)) => y,
                (Some(x), Some(y)) => x || y,
            };
            if enabled {
                fired = Some(t.to);
                break;
            }
        }
        match fired {
            Some(to) => {
                next[ci] = u16::try_from(to).unwrap_or(0);
                next[n + ci] = 0;
            }
            None => next[n + ci] = u16::try_from((dwell + 1).min(c.cap)).unwrap_or(u16::MAX),
        }
    }
    next
}

/// Evaluates the property at `(cfg, val)`; returns `(violated, new monitor value)`.
fn prop_step(prop: &Prop, cfg: &[u16], val: u32, monitor: u16) -> (bool, u16) {
    match prop {
        Prop::Never(g) => (eval(g, cfg, val), 0),
        Prop::Always(g, implies) => {
            let ok = match implies {
                Some(q) => !eval(g, cfg, val) || eval(q, cfg, val),
                None => eval(g, cfg, val),
            };
            (!ok, 0)
        }
        Prop::Within { ticks, after, goal } => {
            let goal_now = eval(goal, cfg, val);
            if monitor == 0 {
                if eval(after, cfg, val) && !goal_now {
                    (false, u16::try_from(*ticks).unwrap_or(u16::MAX).max(1))
                } else {
                    (false, 0)
                }
            } else if goal_now {
                (false, 0)
            } else if monitor == 1 {
                (true, 0)
            } else {
                (false, monitor - 1)
            }
        }
    }
}

fn check(
    comps: &[Comp],
    atoms: &[String],
    prop: &Prop,
    tick_ns: u64,
    opts: &SemaOptions,
) -> Outcome {
    let n = comps.len();
    if atoms.len() > 14 {
        return Outcome::Inconclusive(format!(
            "{} independent conditions are too many to enumerate (limit 14)",
            atoms.len()
        ));
    }
    let valuations: u32 = 1 << atoms.len();
    let mut init = vec![0u16; 2 * n + 1];
    for (i, c) in comps.iter().enumerate() {
        init[i] = u16::try_from(c.init).unwrap_or(0);
    }
    let mut nodes = vec![Node {
        cfg: init.clone(),
        parent: usize::MAX,
        val: 0,
    }];
    let mut seen: BTreeSet<Vec<u16>> = BTreeSet::new();
    seen.insert(init);
    let mut idx = 0;
    let mut work: u64 = 0;
    while idx < nodes.len() {
        if nodes.len() > opts.max_model_states || work > 30_000_000 {
            return Outcome::Inconclusive("the state space is too large".to_string());
        }
        let cfg = nodes[idx].cfg.clone();
        for val in 0..valuations {
            work += 1;
            let (violated, mon) = prop_step(prop, &cfg[..2 * n], val, cfg[2 * n]);
            if violated {
                return Outcome::Violated(trace(comps, atoms, &nodes, idx, val, tick_ns));
            }
            let mut nxt = step(comps, &cfg, val);
            nxt[2 * n] = mon;
            if seen.insert(nxt.clone()) {
                nodes.push(Node {
                    cfg: nxt,
                    parent: idx,
                    val,
                });
            }
        }
        idx += 1;
    }
    Outcome::Holds
}

struct Node {
    cfg: Vec<u16>,
    parent: usize,
    val: u32,
}

fn trace(
    comps: &[Comp],
    atoms: &[String],
    nodes: &[Node],
    last: usize,
    last_val: u32,
    tick_ns: u64,
) -> Vec<TraceStep> {
    let mut path = vec![last];
    let mut cur = last;
    while nodes[cur].parent != usize::MAX {
        cur = nodes[cur].parent;
        path.push(cur);
    }
    path.reverse();
    let mut out = Vec::new();
    for (k, &ni) in path.iter().enumerate() {
        let val = if k + 1 < path.len() {
            nodes[path[k + 1]].val
        } else {
            last_val
        };
        let states = comps
            .iter()
            .enumerate()
            .map(|(ci, c)| format!("{}.{}", c.name, c.states[usize::from(nodes[ni].cfg[ci])]))
            .collect();
        let inputs = atoms
            .iter()
            .enumerate()
            .map(|(i, a)| (a.clone(), val & (1 << i) != 0))
            .collect();
        out.push(TraceStep {
            tick: k as u64,
            time_ns: (k as u64) * tick_ns,
            states,
            inputs,
        });
    }
    out
}
