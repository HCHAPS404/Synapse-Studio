//! Time, concurrency and memory analysis (task A7, spec 15 §15.5).
//!
//! * Activities, priorities, cores and periods; `NE0401`, `NE0406`, `NE0407`.
//! * Shared resources, SRP ceilings and blocking; `NE0301`, `NE0304`, `NE0306`.
//! * Worst-case execution time with an abstract cost model and response-time
//!   analysis; `NE0402`, `NL0402`, `NL0403`.
//! * Static memory and stack budget (property MEM-1); `NE0604`.

use crate::check::{CallTarget, Checked, DefInfo, FnKey};
use crate::consteval::Consts;
use crate::facts::{BodyKind, FactBase};
use crate::model::{key, FileDiag, Program, SymKind};
use crate::report::{ActivityTiming, MemoryReport, ResourceInfo};
use crate::resolve::Resolved;
use crate::ty::{NumKind, Ty};
use crate::visit::{each_expr, each_stmt, stmt_exprs};
use crate::SemaOptions;
use neuron_syntax::ast::{
    Expr, Item, ItemKind, PatternKind, SafetyMember, Spanned, Stmt, StmtKind, TaskAttr, Trigger,
};
use neuron_syntax::{Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};

/// An activity that the scheduler releases.
#[derive(Debug, Clone)]
pub struct Activity {
    /// Index of the body.
    pub body: usize,
    /// Module index.
    pub module: usize,
    /// Display name.
    pub name: String,
    /// `task`, `interrupt`, `fsm` or `safety`.
    pub kind: &'static str,
    /// Effective priority.
    pub prio: i64,
    /// Core.
    pub core: u32,
    /// Period (or minimum inter-arrival time) in nanoseconds.
    pub period: Option<u64>,
    /// Deadline in nanoseconds.
    pub deadline: Option<u64>,
    /// Span of the activity name.
    pub span: Span,
    /// `reads` declaration, if any.
    pub reads: Option<Vec<String>>,
    /// `writes` declaration, if any.
    pub writes: Option<Vec<String>>,
    /// `stack` declaration.
    pub stack: Option<(u64, Span)>,
}

type ResKey = (usize, String);

struct Cx<'a> {
    prog: &'a Program,
    resolved: &'a Resolved,
    checked: &'a Checked,
    facts: &'a FactBase<'a>,
    consts: Consts<'a>,
    opts: &'a SemaOptions,
    diags: &'a mut Vec<FileDiag>,
    fn_cost: BTreeMap<FnKey, u64>,
    fn_busy: BTreeSet<FnKey>,
    fn_stack: BTreeMap<FnKey, u64>,
    stack_busy: BTreeSet<FnKey>,
}

/// Size in bytes of a value of type `t` (packed layout, a lower bound).
pub fn size_of(t: &Ty, checked: &Checked, depth: u8) -> u64 {
    if depth > 8 {
        return 8;
    }
    let sz = |x: &Ty| size_of(x, checked, depth + 1);
    match t {
        Ty::Bool => 1,
        Ty::Num(k, _) => match k {
            NumKind::Int { bits, .. } => u64::from(*bits) / 8,
            NumKind::Index => 4,
            NumKind::Float(b) => u64::from(*b) / 8,
            NumKind::Fixed { int, frac, .. } => {
                (u64::from(*int) + u64::from(*frac)).div_ceil(8).max(1)
            }
            NumKind::Duration => 8,
            NumKind::LitInt => 4,
            NumKind::LitReal => 4,
        },
        Ty::Bits { width, .. } => u64::from(*width).div_ceil(8),
        Ty::Instant => 8,
        Ty::Str(Some(n)) => u64::from(*n),
        Ty::Str(None) => 0,
        Ty::Array(e, Some(n)) => sz(e) * n,
        Ty::Array(_, None) => 0,
        Ty::Slice(_) => 0,
        Ty::Tuple(v) => v.iter().map(sz).sum(),
        Ty::Range(e) => 2 * sz(e),
        Ty::Option(e) => 1 + sz(e),
        Ty::Result(a, b) => 1 + sz(a).max(sz(b)),
        Ty::Adt { def, args } => match checked.defs.get(def) {
            Some(DefInfo::Struct { generics, fields })
            | Some(DefInfo::Class {
                generics, fields, ..
            }) => {
                let subst: Vec<(String, Ty)> =
                    generics.iter().cloned().zip(args.iter().cloned()).collect();
                fields.iter().map(|f| sz(&f.ty.subst(&subst))).sum()
            }
            Some(DefInfo::Enum { variants, .. }) => {
                1 + variants
                    .iter()
                    .map(|(_, p)| p.iter().map(sz).sum::<u64>())
                    .max()
                    .unwrap_or(0)
            }
            _ => 4,
        },
        Ty::Channel(e, Some(n)) => 4 + sz(e) * n,
        Ty::Pool(e, Some(n)) => (sz(e) + 4) * n,
        Ty::Handle(_) => 4,
        Ty::Io(_) => 4,
        Ty::Opaque(_) | Ty::Unknown => 8,
        _ => 0,
    }
}

impl Cx<'_> {
    fn err(
        &mut self,
        file: usize,
        code: &'static str,
        msg: impl Into<String>,
        span: Span,
        help: Option<&str>,
    ) {
        let mut d = Diagnostic::error(code, msg, span);
        if let Some(h) = help {
            d = d.with_help(h);
        }
        self.diags.push(FileDiag { file, diag: d });
    }

    fn warn(&mut self, file: usize, code: &'static str, msg: impl Into<String>, span: Span) {
        self.diags.push(FileDiag {
            file,
            diag: Diagnostic::warning(code, msg, span),
        });
    }

    fn dur(&mut self, module: usize, e: &Expr, what: &str) -> Option<u64> {
        match self.consts.duration_ns(module, e) {
            Some(v) if v > 0 => u64::try_from(v).ok(),
            Some(_) => {
                self.err(
                    module,
                    "NE0407",
                    format!("{what} must be positive"),
                    e.span,
                    None,
                );
                None
            }
            None => {
                self.err(
                    module,
                    "NE0407",
                    format!("{what} must be a constant duration"),
                    e.span,
                    Some("write a duration such as `10 ms`, or use a `const`"),
                );
                None
            }
        }
    }

    fn int_attr(&mut self, module: usize, e: &Expr, what: &str) -> Option<i64> {
        match self
            .consts
            .int(module, e)
            .and_then(|v| i64::try_from(v).ok())
        {
            Some(v) => Some(v),
            None => {
                self.err(
                    module,
                    "NE0407",
                    format!("{what} must be a constant integer"),
                    e.span,
                    None,
                );
                None
            }
        }
    }

    // ---- activities -------------------------------------------------------

    fn activities(&mut self) -> Vec<Activity> {
        let prog = self.prog;
        let facts = self.facts;
        let mut out = Vec::new();
        for (bi, b) in facts.bodies.iter().enumerate() {
            let item: &Item = &prog.files[b.module].ast.items[b.item];
            let mi = b.module;
            let (kind, base_prio, trigger, attrs): (
                &'static str,
                i64,
                Option<&Trigger>,
                &[Spanned<TaskAttr>],
            ) = match (&b.kind, &item.kind) {
                (BodyKind::Task, ItemKind::Task(t)) => ("task", 0, Some(&t.trigger), &t.attrs[..]),
                (BodyKind::Interrupt, ItemKind::Interrupt { attrs, .. }) => {
                    ("interrupt", 1000, None, &attrs[..])
                }
                (BodyKind::Fsm, ItemKind::Fsm(f)) => ("fsm", 0, f.trigger.as_ref(), &[][..]),
                (BodyKind::SafetyRule, ItemKind::Safety(_)) => ("safety", 2000, None, &[][..]),
                _ => continue,
            };
            let mut prio = 1i64;
            let mut core = 0u32;
            let mut deadline: Option<u64> = None;
            let mut stack: Option<(u64, Span)> = None;
            let mut reads: Option<Vec<String>> = None;
            let mut writes: Option<Vec<String>> = None;
            for a in attrs {
                match &a.node {
                    TaskAttr::Prio(e) => {
                        if let Some(v) = self.int_attr(mi, e, "the priority") {
                            prio = v;
                        }
                    }
                    TaskAttr::Core(e) => {
                        if let Some(v) = self.int_attr(mi, e, "the core") {
                            core = u32::try_from(v).unwrap_or(0);
                        }
                    }
                    TaskAttr::Deadline(e) => deadline = self.dur(mi, e, "the deadline"),
                    TaskAttr::Stack(e) => {
                        if let Some(v) = self.int_attr(mi, e, "the stack size") {
                            stack = u64::try_from(v).ok().map(|x| (x, a.span));
                        }
                    }
                    TaskAttr::Reads(ids) => {
                        reads
                            .get_or_insert_with(Vec::new)
                            .extend(ids.iter().map(|i| i.name.clone()));
                    }
                    TaskAttr::Writes(ids) => {
                        writes
                            .get_or_insert_with(Vec::new)
                            .extend(ids.iter().map(|i| i.name.clone()));
                    }
                }
            }
            let mut period: Option<u64> = None;
            if let Some(Trigger::Every { period: p, offset }) = trigger {
                period = self.dur(mi, p, "the period");
                if let Some(o) = offset {
                    if self.consts.duration_ns(mi, o).is_none() {
                        self.err(
                            mi,
                            "NE0407",
                            "the offset must be a constant duration",
                            o.span,
                            None,
                        );
                    }
                }
            }
            if kind == "interrupt" {
                if let Some(a) = item.attrs.iter().find(|a| a.name.name == "min_interval") {
                    if let Some(arg) = a.args.first() {
                        period = self.dur(mi, &arg.value, "the minimum interval");
                    }
                }
            }
            if kind == "safety" {
                if let ItemKind::Safety(members) = &item.kind {
                    for m in members {
                        if let SafetyMember::Rate(e) = &m.node {
                            period = self.dur(mi, e, "the safety rate");
                        }
                    }
                }
            }
            if deadline.is_none() {
                deadline = period;
            }
            if let (Some(d), Some(p), "task" | "fsm") = (deadline, period, kind) {
                if d > p {
                    self.err(
                        mi,
                        "NE0406",
                        format!("the deadline ({d} ns) is longer than the period ({p} ns)"),
                        b.span,
                        Some("a release must finish before the next one"),
                    );
                }
            }
            out.push(Activity {
                body: bi,
                module: mi,
                name: b.name.clone(),
                kind,
                prio: base_prio + prio,
                core,
                period,
                deadline,
                span: b.span,
                reads,
                writes,
                stack,
            });
        }
        out
    }

    fn base_tick(&mut self, module: usize) -> Option<u64> {
        if let Some(t) = self.opts.base_tick_ns {
            return Some(t);
        }
        match self.prog.modules[module].dialect {
            neuron_syntax::ast::Dialect::Mcu => Some(1_000_000),
            neuron_syntax::ast::Dialect::Plc => {
                let items = &self.prog.files[module].ast.items;
                for it in items {
                    if let ItemKind::Scan(e) = &it.kind {
                        return self.dur(module, e, "the scan time");
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn check_ticks(&mut self, acts: &[Activity]) {
        let mut ticks: BTreeMap<usize, Option<u64>> = BTreeMap::new();
        for a in acts {
            if a.kind == "interrupt" {
                continue;
            }
            let tick = match ticks.get(&a.module).copied() {
                Some(t) => t,
                None => {
                    let t = self.base_tick(a.module);
                    ticks.insert(a.module, t);
                    t
                }
            };
            if let (Some(t), Some(p)) = (tick, a.period) {
                if p % t != 0 {
                    self.err(
                        a.module,
                        "NE0401",
                        format!("the period of `{}` ({p} ns) is not a multiple of the base tick ({t} ns)", a.name),
                        a.span,
                        Some("choose a multiple of the tick, or a target with a finer timer"),
                    );
                }
            }
        }
        for (mi, f) in self.prog.files.iter().enumerate() {
            if f.syntax_errors {
                continue;
            }
            for it in &f.ast.items {
                match &it.kind {
                    ItemKind::Scan(e) => {
                        let _ = self.dur(mi, e, "the scan time");
                    }
                    ItemKind::Safety(ms) => {
                        for m in ms {
                            if let SafetyMember::Watchdog(e) = &m.node {
                                let _ = self.dur(mi, e, "the watchdog timeout");
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ---- cost model ---------------------------------------------------------

    fn expr_cost(&mut self, module: usize, e: &Expr) -> u64 {
        let mut nodes = 0u64;
        let mut callees: Vec<FnKey> = Vec::new();
        let mut others = 0u64;
        each_expr(e, &mut |x| {
            nodes += 1;
            if let neuron_syntax::ast::ExprKind::Call(..) = &x.kind {
                match self.checked.calls.get(&key(module, x.span)) {
                    Some(CallTarget::Fn(k)) | Some(CallTarget::Method { key: k, .. }) => {
                        callees.push(k.clone())
                    }
                    _ => others += 4,
                }
            }
        });
        let mut total = nodes + others;
        for k in callees {
            total += self.fn_cost_of(&k);
        }
        total
    }

    fn fn_cost_of(&mut self, k: &FnKey) -> u64 {
        if let Some(c) = self.fn_cost.get(k) {
            return *c;
        }
        if !self.fn_busy.insert(k.clone()) {
            return 0;
        }
        let facts = self.facts;
        let cost = match facts.fn_index.get(k) {
            Some(&bi) => {
                let b = &facts.bodies[bi];
                let mut c = 2;
                for e in &b.exprs {
                    c += self.expr_cost(b.module, e);
                }
                for &s in &b.stmts {
                    c += self.stmt_cost(b.module, s);
                }
                c
            }
            None => 4,
        };
        self.fn_busy.remove(k);
        self.fn_cost.insert(k.clone(), cost);
        cost
    }

    fn block_cost(&mut self, module: usize, stmts: &[Stmt]) -> u64 {
        stmts.iter().map(|s| self.stmt_cost(module, s)).sum()
    }

    fn stmt_cost(&mut self, module: usize, s: &Stmt) -> u64 {
        let mut exprs: Vec<&Expr> = Vec::new();
        stmt_exprs(s, &mut |e| exprs.push(e));
        let mut base = 1;
        for e in exprs {
            base += self.expr_cost(module, e);
        }
        match &s.kind {
            StmtKind::If {
                branches,
                otherwise,
            } => {
                let mut worst = otherwise
                    .as_ref()
                    .map_or(0, |o| self.block_cost(module, &o.stmts));
                for (_, b) in branches {
                    worst = worst.max(self.block_cost(module, &b.stmts));
                }
                base + worst
            }
            StmtKind::Match { arms, .. } => {
                let mut worst = 0;
                for a in arms {
                    worst = worst.max(self.block_cost(module, &a.body.stmts));
                }
                base + worst
            }
            StmtKind::For { iter, body, .. } => {
                let n = self
                    .checked
                    .iter_len
                    .get(&key(module, iter.span))
                    .copied()
                    .flatten()
                    .unwrap_or(1);
                base + n * self.block_cost(module, &body.stmts)
            }
            StmtKind::While { max, body, .. } | StmtKind::Loop { max, body } => {
                let n = max
                    .as_ref()
                    .and_then(|m| self.consts.int(module, m))
                    .and_then(|v| u64::try_from(v).ok())
                    .unwrap_or(1);
                base + n * self.block_cost(module, &body.stmts)
            }
            StmtKind::Atomic(b) => base + self.block_cost(module, &b.stmts),
            StmtKind::Rule(r) => base + self.block_cost(module, &r.body.stmts),
            _ => base,
        }
    }

    fn body_ops(&mut self, bi: usize) -> u64 {
        let facts = self.facts;
        let b = &facts.bodies[bi];
        let mut c = 2;
        for e in &b.exprs {
            c += self.expr_cost(b.module, e);
        }
        for &s in &b.stmts {
            c += self.stmt_cost(b.module, s);
        }
        c
    }

    fn ops_to_ns(&self, ops: u64) -> Option<u64> {
        let hz = self.opts.cpu_hz?;
        if hz == 0 {
            return None;
        }
        let cycles = u128::from(ops) * u128::from(self.opts.cycles_per_op);
        let ns = (cycles * 1_000_000_000).div_ceil(u128::from(hz));
        u64::try_from(ns).ok()
    }

    // ---- stack and memory -------------------------------------------------

    fn local_bytes(&self, b: &crate::facts::Body<'_>) -> u64 {
        let mut total = 8u64;
        let mut stmts: Vec<&Stmt> = Vec::new();
        for &s in &b.stmts {
            each_stmt(std::slice::from_ref(s), &mut |x| stmts.push(x));
        }
        for s in stmts {
            if let StmtKind::Let { pattern, .. } = &s.kind {
                if let PatternKind::Bind(id) = &pattern.kind {
                    if let Some(crate::model::Res::Local(i)) =
                        self.resolved.res.get(&key(b.module, id.span))
                    {
                        if let Some(t) = self.checked.local_ty.get(*i) {
                            total += size_of(t, self.checked, 0);
                        }
                    }
                }
            }
        }
        if let Some(k) = &b.fn_key {
            if let Some(sig) = self.checked.sigs.get(k) {
                total += sig
                    .params
                    .iter()
                    .map(|p| size_of(&p.ty, self.checked, 0))
                    .sum::<u64>();
            }
        }
        total
    }

    fn stack_of_fn(&mut self, k: &FnKey) -> u64 {
        if let Some(v) = self.fn_stack.get(k) {
            return *v;
        }
        if !self.stack_busy.insert(k.clone()) {
            return 0;
        }
        let facts = self.facts;
        let v = match facts.fn_index.get(k) {
            Some(&bi) => {
                let own = self.local_bytes(&facts.bodies[bi]);
                let mut deepest = 0;
                for (callee, _) in &facts.facts[bi].calls {
                    deepest = deepest.max(self.stack_of_fn(callee));
                }
                own + deepest
            }
            None => 8,
        };
        self.stack_busy.remove(k);
        self.fn_stack.insert(k.clone(), v);
        v
    }

    fn activity_stack(&mut self, a: &Activity) -> u64 {
        let facts = self.facts;
        let own = self.local_bytes(&facts.bodies[a.body]);
        let mut deepest = 0;
        for (callee, _) in &facts.facts[a.body].calls {
            deepest = deepest.max(self.stack_of_fn(callee));
        }
        let frame = if a.kind == "interrupt" { 32 } else { 16 };
        frame + own + deepest
    }
}

/// Everything A7 produces.
#[derive(Debug, Default)]
pub struct TimingOutput {
    /// Timing per activity.
    pub timing: Vec<ActivityTiming>,
    /// Shared resources.
    pub resources: Vec<ResourceInfo>,
    /// Memory per module.
    pub memory: Vec<MemoryReport>,
}

struct Use {
    activity: usize,
    write: bool,
    cs_ops: u64,
}

/// Runs the time, concurrency and memory analysis.
pub fn analyze_timing(
    prog: &Program,
    resolved: &Resolved,
    checked: &Checked,
    facts: &FactBase<'_>,
    opts: &SemaOptions,
    diags: &mut Vec<FileDiag>,
) -> TimingOutput {
    let mut cx = Cx {
        prog,
        resolved,
        checked,
        facts,
        consts: Consts::new(prog, &resolved.res),
        opts,
        diags,
        fn_cost: BTreeMap::new(),
        fn_busy: BTreeSet::new(),
        fn_stack: BTreeMap::new(),
        stack_busy: BTreeSet::new(),
    };
    let acts = cx.activities();
    cx.check_ticks(&acts);
    let effects = facts.fn_effects_closure();

    // Statement costs and the effects of each activity.
    let mut top_cost: Vec<Vec<(Span, u64)>> = vec![Vec::new(); facts.bodies.len()];
    for a in &acts {
        let b = &facts.bodies[a.body];
        let costs: Vec<(Span, u64)> = b
            .stmts
            .iter()
            .map(|&s| (s.span, cx.stmt_cost(b.module, s)))
            .collect();
        top_cost[a.body] = costs;
    }
    let cost_at = |bi: usize, span: Span, cx_top: &[Vec<(Span, u64)>]| -> u64 {
        cx_top[bi]
            .iter()
            .find(|(s, _)| s.start <= span.start && span.end <= s.end)
            .map_or(1, |(_, c)| *c)
    };

    let mut uses: BTreeMap<ResKey, Vec<Use>> = BTreeMap::new();
    let mut kinds: BTreeMap<ResKey, SymKind> = BTreeMap::new();
    let mut touched: Vec<BTreeMap<ResKey, (bool, Span)>> = Vec::new();
    for (ai, a) in acts.iter().enumerate() {
        let f = &facts.facts[a.body];
        let mut mine: BTreeMap<ResKey, (bool, u64, Span)> = BTreeMap::new();
        for acc in &f.accesses {
            let cost = cost_at(a.body, acc.top, &top_cost);
            let e = mine
                .entry(acc.target.clone())
                .or_insert((false, 0, acc.span));
            e.0 |= acc.write;
            e.1 = e.1.max(cost);
            kinds.insert(acc.target.clone(), acc.kind);
        }
        for (callee, span) in &f.calls {
            if let Some(effs) = effects.get(callee) {
                let cost = cost_at(a.body, *span, &top_cost);
                for (m, n, write) in effs {
                    let e = mine.entry((*m, n.clone())).or_insert((false, 0, *span));
                    e.0 |= *write;
                    e.1 = e.1.max(cost);
                    if let Some(kind) = facts.kinds.get(&(*m, n.clone())) {
                        kinds.insert((*m, n.clone()), *kind);
                    }
                }
            }
        }
        let mut t = BTreeMap::new();
        for (k, (write, ops, span)) in mine {
            t.insert(k.clone(), (write, span));
            uses.entry(k).or_default().push(Use {
                activity: ai,
                write,
                cs_ops: ops,
            });
        }
        touched.push(t);
    }

    // Resources, ceilings, cross-core sharing, NE0304 and NE0306.
    let mut out = TimingOutput::default();
    let mut ceilings: BTreeMap<ResKey, i64> = BTreeMap::new();
    for (k, us) in &uses {
        let kind = kinds[k];
        let ceiling = us.iter().map(|u| acts[u.activity].prio).max().unwrap_or(0);
        let cores: BTreeSet<u32> = us.iter().map(|u| acts[u.activity].core).collect();
        let accessors: Vec<String> = {
            let mut v: Vec<String> = us.iter().map(|u| acts[u.activity].name.clone()).collect();
            v.sort();
            v.dedup();
            v
        };
        if kind != SymKind::Signal {
            ceilings.insert(k.clone(), ceiling);
            out.resources.push(ResourceInfo {
                module: prog.modules[k.0].name.clone(),
                name: k.1.clone(),
                kind: match kind {
                    SymKind::Io => "io",
                    SymKind::PlcVar(_) => "plc variable",
                    _ => "var",
                },
                accessors: accessors.clone(),
                ceiling,
                shared: us.len() > 1,
                cross_core: cores.len() > 1,
            });
        }
        if kind == SymKind::Var && cores.len() > 1 {
            if let Some(sym) = prog.symbol(k.0, &k.1) {
                cx.err(
                    k.0,
                    "NE0304",
                    format!(
                        "`{}` is a `var` shared by activities on different cores",
                        k.1
                    ),
                    sym.span,
                    Some("use a `channel` or a `signal` between cores"),
                );
            }
        }
        if kind == SymKind::Signal {
            let writers: Vec<&Use> = us.iter().filter(|u| u.write).collect();
            if writers.len() > 1 {
                let second = writers[1].activity;
                let span = touched[second]
                    .get(k)
                    .map_or(acts[second].span, |(_, s)| *s);
                let first_name = acts[writers[0].activity].name.clone();
                cx.err(
                    acts[second].module,
                    "NE0306",
                    format!(
                        "signal `{}` is written by `{}` and by `{first_name}`",
                        k.1, acts[second].name
                    ),
                    span,
                    Some("a signal has one writer so that its value at each deadline is well defined"),
                );
            }
        }
    }

    // NE0301: declared access lists.
    for (ai, a) in acts.iter().enumerate() {
        if a.reads.is_none() && a.writes.is_none() {
            continue;
        }
        for (k, (write, span)) in &touched[ai] {
            let reads = a.reads.as_deref().unwrap_or(&[]);
            let writes = a.writes.as_deref().unwrap_or(&[]);
            let ok = if *write {
                writes.contains(&k.1)
            } else {
                reads.contains(&k.1) || writes.contains(&k.1)
            };
            if !ok {
                let verb = if *write { "writes" } else { "reads" };
                cx.err(
                    a.module,
                    "NE0301",
                    format!(
                        "`{}` {verb} `{}` but does not declare it in `{verb}`",
                        a.name, k.1
                    ),
                    *span,
                    Some("add it to the `reads`/`writes` list of the task, or remove the access"),
                );
            }
        }
    }

    // Worst-case execution times.
    let wcet: Vec<Option<u64>> = acts
        .iter()
        .map(|a| {
            let ops = cx.body_ops(a.body);
            cx.ops_to_ns(ops)
        })
        .collect();

    // Blocking by lower-priority critical sections (SRP).
    let mut blocking = vec![0u64; acts.len()];
    for (i, ai) in acts.iter().enumerate() {
        let mut worst = 0u64;
        for (k, us) in &uses {
            let ceiling = match ceilings.get(k) {
                Some(c) => *c,
                None => continue,
            };
            if ceiling < ai.prio {
                continue;
            }
            for u in us {
                let aj = &acts[u.activity];
                if aj.module == ai.module && aj.core == ai.core && aj.prio < ai.prio {
                    if let Some(ns) = cx.ops_to_ns(u.cs_ops) {
                        worst = worst.max(ns);
                    }
                }
            }
        }
        blocking[i] = worst;
    }

    // Response-time analysis.
    let mut response: Vec<Option<u64>> = vec![None; acts.len()];
    let mut modules_with_deadlines: BTreeMap<usize, Span> = BTreeMap::new();
    for (i, a) in acts.iter().enumerate() {
        if a.deadline.is_some() || a.period.is_some() {
            modules_with_deadlines.entry(a.module).or_insert(a.span);
        }
        let (Some(c), Some(d)) = (wcet[i], a.deadline.or(a.period)) else {
            continue;
        };
        let equal_block = acts
            .iter()
            .enumerate()
            .filter(|(j, o)| {
                *j != i && o.module == a.module && o.core == a.core && o.prio == a.prio
            })
            .filter_map(|(j, _)| wcet[j])
            .max()
            .unwrap_or(0);
        let b = blocking[i].max(equal_block);
        let hp: Vec<(u64, u64)> = acts
            .iter()
            .enumerate()
            .filter(|(j, o)| *j != i && o.module == a.module && o.core == a.core && o.prio > a.prio)
            .filter_map(|(j, o)| Some((o.period?, wcet[j]?)))
            .collect();
        let mut r = c + b;
        let cap = d.saturating_mul(4).max(1);
        loop {
            let interference: u64 = hp
                .iter()
                .map(|(t, cj)| r.div_ceil((*t).max(1)).saturating_mul(*cj))
                .fold(0u64, |s, x| s.saturating_add(x));
            let next = c.saturating_add(b).saturating_add(interference);
            if next == r || next > cap {
                r = next;
                break;
            }
            r = next;
        }
        response[i] = Some(r);
        if r > d {
            cx.err(
                a.module,
                "NE0402",
                format!(
                    "`{}` can miss its deadline: response time {r} ns exceeds {d} ns (WCET {c} ns, blocking {b} ns)",
                    a.name
                ),
                a.span,
                Some("raise the priority, shorten the work, or relax the deadline"),
            );
        }
    }
    if opts.cpu_hz.is_none() {
        for (mi, span) in modules_with_deadlines {
            cx.warn(
                mi,
                "NL0402",
                "WCET analysis is unavailable because no timing model is configured; deadlines are monitored at run time",
                span,
            );
        }
    }
    for a in &acts {
        if a.kind == "interrupt" && a.period.is_none() {
            cx.warn(
                a.module,
                "NL0403",
                format!("interrupt `{}` has no `@min_interval`; it is left out of the response-time analysis", a.name),
                a.span,
            );
        }
    }

    for (i, a) in acts.iter().enumerate() {
        out.timing.push(ActivityTiming {
            module: prog.modules[a.module].name.clone(),
            name: a.name.clone(),
            kind: a.kind,
            priority: a.prio,
            core: a.core,
            period_ns: a.period,
            deadline_ns: a.deadline,
            wcet_ns: wcet[i],
            blocking_ns: blocking[i],
            response_ns: response[i],
            schedulable: match (response[i], a.deadline) {
                (Some(r), Some(d)) => Some(r <= d),
                _ => None,
            },
        });
    }

    // Memory (MEM-1).
    for (mi, m) in prog.modules.iter().enumerate() {
        if prog.files[mi].syntax_errors {
            continue;
        }
        let mut breakdown: Vec<(String, u64)> = Vec::new();
        let mut static_bytes = 0u64;
        for (name, sym) in &m.symbols {
            let factor = match sym.kind {
                SymKind::Var | SymKind::PlcVar(_) | SymKind::Io => 1,
                SymKind::Signal => 2,
                _ => continue,
            };
            let ty = checked
                .item_ty
                .get(&(mi, name.clone()))
                .cloned()
                .unwrap_or(Ty::Unknown);
            let sz = size_of(&ty, checked, 0) * factor;
            if sz > 0 {
                static_bytes += sz;
                breakdown.push((format!("static `{name}`"), sz));
            }
        }
        let mine: Vec<&Activity> = acts.iter().filter(|a| a.module == mi).collect();
        let mut levels: BTreeMap<(u32, i64), u64> = BTreeMap::new();
        for a in &mine {
            let computed = cx.activity_stack(a);
            if let Some((declared, span)) = a.stack {
                if computed > declared {
                    cx.err(
                        mi,
                        "NE0604",
                        format!(
                            "`{}` needs about {computed} bytes of stack but declares {declared}",
                            a.name
                        ),
                        span,
                        Some("raise the `stack` budget or reduce locals and call depth"),
                    );
                }
            }
            let used = a.stack.map_or(computed, |(d, _)| d);
            let e = levels.entry((a.core, a.prio)).or_insert(0);
            *e = (*e).max(used);
        }
        let mut stack_bytes = 0u64;
        for ((core, prio), s) in &levels {
            stack_bytes += s;
            breakdown.push((format!("stack core {core} priority {prio}"), *s));
        }
        let runtime_bytes = opts.runtime_ram_bytes;
        let total = static_bytes + stack_bytes + runtime_bytes;
        out.memory.push(MemoryReport {
            module: m.name.clone(),
            static_bytes,
            stack_bytes,
            runtime_bytes,
            total_bytes: total,
            ram_bytes: opts.ram_bytes,
            breakdown: breakdown.clone(),
        });
        if let Some(ram) = opts.ram_bytes {
            if total > ram
                && matches!(
                    m.dialect,
                    neuron_syntax::ast::Dialect::Mcu | neuron_syntax::ast::Dialect::Ui
                )
            {
                let mut top = breakdown;
                top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                let parts: Vec<String> = top
                    .iter()
                    .take(4)
                    .map(|(n, s)| format!("{n}: {s} B"))
                    .collect();
                cx.err(
                    mi,
                    "NE0604",
                    format!(
                        "memory budget exceeded: {total} bytes needed, {ram} available (static {static_bytes}, stack {stack_bytes}, runtime {runtime_bytes})"
                    ),
                    prog.files[mi].ast.header.span,
                    Some(&format!("largest parts: {}", parts.join(", "))),
                );
            }
        }
    }
    out
}
