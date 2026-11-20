//! Setup, the event loop, and the activities: tasks, interrupts, rules, state machines,
//! sequences, PLC scans and the safety logic (spec 14 and 17).

use crate::eval::{dotted, MAX_ARRAY};
use crate::value::Value;
use crate::{
    err, fault, Ctx, Ev, Interp, InterpError, IoState, Ready, SigInfo, Snap, Stop, TraceEvent,
    TraceKind, R,
};
use neuron_nir::{
    Expr, ItemDef, Pattern, Rule, SafetyDef, SeqItem, Stmt, StmtKind, TaskAttr, Trigger, VarDecl,
    Widget,
};
use std::collections::BTreeMap;
use std::rc::Rc;

const DEFAULT_RATE_NS: i64 = 10_000_000;

/// Kind of an activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActKind {
    Task,
    Interrupt,
    Rule,
    Fsm,
    Sequence,
    Scan,
    Safety,
    Blocks,
}

/// Edge and hold state of a `when` rule.
#[derive(Debug, Default, Clone)]
pub(crate) struct RuleRt {
    pub prev: bool,
    pub since: Option<i64>,
    pub fired: bool,
}

/// Current state of a state machine.
#[derive(Debug, Default)]
pub(crate) struct FsmRt {
    pub state: Option<usize>,
    pub entered: i64,
}

/// A flattened step of a sequence.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Op {
    Step(String),
    Stmt(Stmt),
    Par(Vec<Vec<Op>>),
    /// Start of a loop whose body waits. `exit` is the index after its `LoopEnd`.
    LoopBegin {
        spec: Box<LoopSpec>,
        exit: usize,
    },
    /// Head of the loop: runs the next iteration or leaves the loop.
    LoopNext,
    /// End of the body: back to the `LoopNext` at `head`.
    LoopEnd {
        head: usize,
    },
}

/// The loop statement behind a flattened loop.
// Ops hold it boxed and there is one per flattened loop, so the variant sizes do not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub(crate) enum LoopSpec {
    For {
        pattern: Pattern,
        iter: Expr,
    },
    While {
        pattern: Option<Pattern>,
        cond: Expr,
        max: Option<Expr>,
    },
    Loop {
        max: Option<Expr>,
    },
}

/// Progress of one running flattened loop.
#[derive(Debug)]
pub(crate) struct LoopState {
    spec: LoopSpec,
    exit: usize,
    head: usize,
    items: Vec<Value>,
    idx: usize,
    n: u64,
    bound: u64,
}

/// What a sequence is waiting for.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Wait {
    Until {
        cond: Expr,
        deadline: Option<i64>,
        target: Option<String>,
    },
    Time(i64),
}

/// Position of a running sequence (or of one branch of a `parallel` block).
#[derive(Debug, Default)]
pub(crate) struct Runner {
    pub name: String,
    pub ops: Vec<Op>,
    pub pc: usize,
    pub wait: Option<Wait>,
    pub par: Option<Vec<Runner>>,
    pub done: bool,
    /// Loops being run, innermost last. Cleared by every jump.
    pub loops: Vec<LoopState>,
    /// Sequence locals (`let`, loop variables) kept between ticks.
    pub locals: BTreeMap<String, Value>,
    pub names: BTreeMap<String, String>,
    /// `robot` module: a step is entered only by a jump, never by falling through.
    pub handlers: bool,
    /// The last jump has not reached its step yet.
    pub jumped: bool,
}

/// Runtime state of an activity.
#[derive(Debug, Default)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ActState {
    #[default]
    None,
    Fsm(FsmRt),
    Seq(Runner),
    Rule(RuleRt),
    Safety(Vec<RuleRt>),
}

/// A schedulable activity.
#[derive(Debug)]
pub(crate) struct Act {
    pub name: String,
    pub module: usize,
    pub item: usize,
    pub kind: ActKind,
    pub prio: i64,
    pub period: Option<i64>,
    pub deadline: i64,
    pub st: ActState,
}

pub(crate) fn stop_err(s: Stop) -> InterpError {
    InterpError {
        message: match s {
            Stop::Fault(n) => format!("fault {n} while setting up the program"),
            _ => "unexpected control flow while setting up the program".to_string(),
        },
    }
}

/// `true` when running the statement can make a sequence wait (through nested loops).
fn waits(s: &Stmt) -> bool {
    match &s.s {
        StmtKind::Move { .. }
        | StmtKind::Wait { .. }
        | StmtKind::Until { .. }
        | StmtKind::Jump { .. } => true,
        StmtKind::For { body, .. } | StmtKind::While { body, .. } | StmtKind::Loop { body, .. } => {
            body.stmts.iter().any(waits)
        }
        _ => false,
    }
}

/// Flattens a statement: a loop that waits becomes `LoopBegin`, `LoopNext`, body,
/// `LoopEnd`; anything else stays one op.
fn flatten_stmt(s: &Stmt, out: &mut Vec<Op>) {
    let (spec, body) = match &s.s {
        StmtKind::For {
            pattern,
            iter,
            body,
            ..
        } if waits(s) => (
            LoopSpec::For {
                pattern: pattern.clone(),
                iter: iter.clone(),
            },
            body,
        ),
        StmtKind::While {
            pattern,
            cond,
            max,
            body,
        } if waits(s) => (
            LoopSpec::While {
                pattern: pattern.clone(),
                cond: cond.clone(),
                max: max.clone(),
            },
            body,
        ),
        StmtKind::Loop { max, body } if waits(s) => (LoopSpec::Loop { max: max.clone() }, body),
        _ => {
            out.push(Op::Stmt(s.clone()));
            return;
        }
    };
    let begin = out.len();
    out.push(Op::LoopBegin {
        spec: Box::new(spec),
        exit: 0,
    });
    out.push(Op::LoopNext);
    for st in &body.stmts {
        flatten_stmt(st, out);
    }
    out.push(Op::LoopEnd { head: begin + 1 });
    let end = out.len();
    if let Op::LoopBegin { exit, .. } = &mut out[begin] {
        *exit = end;
    }
}

fn flatten(items: &[SeqItem], out: &mut Vec<Op>) {
    for it in items {
        match it {
            SeqItem::Step { name, body, .. } => {
                out.push(Op::Step(name.clone()));
                flatten(body, out);
            }
            SeqItem::Parallel { branches, .. } => {
                let subs = branches
                    .iter()
                    .map(|b| {
                        let mut v = Vec::new();
                        flatten(&b.items, &mut v);
                        v
                    })
                    .collect();
                out.push(Op::Par(subs));
            }
            SeqItem::Stmt { stmt } => flatten_stmt(stmt, out),
        }
    }
}

fn find_widget<'a>(ws: &'a [Widget], name: &str) -> Option<&'a Widget> {
    for w in ws {
        if w.name.as_deref() == Some(name) {
            return Some(w);
        }
        if let Some(c) = find_widget(&w.children, name) {
            return Some(c);
        }
    }
    None
}

impl Interp {
    pub(crate) fn trace_push(&mut self, kind: TraceKind, name: String, value: String) {
        self.trace.push(TraceEvent {
            t_ns: self.now,
            kind,
            name,
            value,
        });
    }

    pub(crate) fn push_event(&mut self, t: i64, ev: Ev) {
        let rank = match &ev {
            Ev::Commit(..) => 0,
            Ev::Input(..) | Ev::Press(_) | Ev::Done(_) => 1,
            Ev::Raise(_) => 2,
            Ev::Release(_) => 3,
            Ev::Exec(..) => 4,
        };
        self.seq_no += 1;
        let t = t.max(self.now);
        self.queue.insert((t, rank, self.seq_no), ev);
    }

    fn init_value(&mut self, e: &Expr) -> Result<Value, InterpError> {
        self.ev(e).map_err(stop_err)
    }

    fn dur(&mut self, e: &Expr) -> R<i64> {
        let v = self.ev(e)?;
        Ok((v.as_f64().unwrap_or(0.0) * 1.0e9).round() as i64)
    }

    // ---- setup ------------------------------------------------------------------

    pub(crate) fn init(&mut self) -> Result<(), InterpError> {
        let nir = Rc::clone(&self.nir);
        for (m, module) in nir.modules.iter().enumerate() {
            self.cur_module = m;
            for item in &module.items {
                self.init_item(m, &item.id, &item.def)?;
            }
        }
        for (m, module) in nir.modules.iter().enumerate() {
            self.cur_module = m;
            let plc = module.dialect == "plc";
            let mut has_scan = false;
            for (i, item) in module.items.iter().enumerate() {
                match &item.def {
                    ItemDef::Scan { period } => {
                        has_scan = true;
                        let p = self.dur(period).map_err(stop_err)?;
                        if p <= 0 {
                            return err("the scan time must be positive");
                        }
                        self.add_act("scan", m, i, ActKind::Scan, 1, Some(p), 0, 0, None);
                    }
                    ItemDef::Task {
                        name,
                        trigger,
                        attrs,
                        ..
                    } => {
                        let (period, offset, key) = self.trigger_of(trigger)?;
                        let (prio, deadline) = self.attrs_of(attrs, period.unwrap_or(0))?;
                        self.add_act(
                            name,
                            m,
                            i,
                            ActKind::Task,
                            prio,
                            period,
                            deadline,
                            offset,
                            key,
                        );
                    }
                    ItemDef::Interrupt { source, attrs, .. } => {
                        let (prio, _) = self.attrs_of(attrs, 0)?;
                        let key = dotted(source);
                        let name = format!("interrupt {key}");
                        self.add_act(
                            &name,
                            m,
                            i,
                            ActKind::Interrupt,
                            1_000_000 + prio,
                            None,
                            0,
                            0,
                            Some(key),
                        );
                    }
                    ItemDef::Rule { .. } if !plc => {
                        let a = self.add_act(
                            "when",
                            m,
                            i,
                            ActKind::Rule,
                            1,
                            Some(DEFAULT_RATE_NS),
                            0,
                            0,
                            None,
                        );
                        self.acts[a].st = ActState::Rule(RuleRt::default());
                    }
                    ItemDef::Fsm { fsm } => {
                        let (period, offset, key) = match &fsm.trigger {
                            Some(t) => self.trigger_of(t)?,
                            None => (Some(DEFAULT_RATE_NS), 0, None),
                        };
                        let a =
                            self.add_act(&fsm.name, m, i, ActKind::Fsm, 1, period, 0, offset, key);
                        self.acts[a].st = ActState::Fsm(FsmRt::default());
                    }
                    ItemDef::Sequence { name, items, .. } => {
                        let mut ops = Vec::new();
                        flatten(items, &mut ops);
                        let a = self.add_act(
                            name,
                            m,
                            i,
                            ActKind::Sequence,
                            1,
                            Some(self.opts.base_tick_ns),
                            0,
                            0,
                            None,
                        );
                        self.acts[a].st = ActState::Seq(Runner {
                            name: name.clone(),
                            ops,
                            handlers: module.dialect == "robot",
                            ..Runner::default()
                        });
                    }
                    ItemDef::Safety { members } => {
                        let mut rate = DEFAULT_RATE_NS;
                        for mem in members {
                            if let SafetyDef::Rate { value } = &mem.def {
                                rate = self.dur(value).map_err(stop_err)?.max(1);
                            }
                        }
                        let a = self.add_act(
                            "safety",
                            m,
                            i,
                            ActKind::Safety,
                            2_000_000,
                            Some(rate),
                            0,
                            0,
                            None,
                        );
                        self.acts[a].st = ActState::Safety(vec![RuleRt::default(); members.len()]);
                    }
                    _ => {}
                }
            }
            if plc && !has_scan {
                self.add_act(
                    "scan",
                    m,
                    usize::MAX,
                    ActKind::Scan,
                    1,
                    Some(DEFAULT_RATE_NS),
                    0,
                    0,
                    None,
                );
            }
            if !plc && self.module_blocks.get(&m).is_some_and(|b| !b.is_empty()) {
                self.add_act(
                    "blocks",
                    m,
                    usize::MAX,
                    ActKind::Blocks,
                    0,
                    Some(self.opts.base_tick_ns),
                    0,
                    0,
                    None,
                );
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn add_act(
        &mut self,
        name: &str,
        module: usize,
        item: usize,
        kind: ActKind,
        prio: i64,
        period: Option<i64>,
        deadline: i64,
        offset: i64,
        key: Option<String>,
    ) -> usize {
        let a = self.acts.len();
        self.acts.push(Act {
            name: name.to_string(),
            module,
            item,
            kind,
            prio,
            period,
            deadline,
            st: ActState::None,
        });
        if period.is_some() {
            self.push_event(offset, Ev::Release(a));
        }
        if let Some(k) = key {
            self.event_subs.entry(k).or_default().push(a);
        }
        a
    }

    fn trigger_of(
        &mut self,
        t: &Trigger,
    ) -> Result<(Option<i64>, i64, Option<String>), InterpError> {
        match t {
            Trigger::Every { period, offset } => {
                let p = self.dur(period).map_err(stop_err)?;
                let o = match offset {
                    Some(e) => self.dur(e).map_err(stop_err)?,
                    None => 0,
                };
                if p <= 0 {
                    return err("a period must be positive");
                }
                Ok((Some(p), o, None))
            }
            Trigger::On { event } => Ok((None, 0, Some(dotted(event)))),
        }
    }

    fn attrs_of(
        &mut self,
        attrs: &[TaskAttr],
        default_deadline: i64,
    ) -> Result<(i64, i64), InterpError> {
        let mut prio = 1;
        let mut deadline = default_deadline;
        for a in attrs {
            match a {
                TaskAttr::Prio { value } => {
                    prio = self
                        .init_value(value)?
                        .as_int()
                        .and_then(|v| i64::try_from(v).ok())
                        .unwrap_or(1);
                }
                TaskAttr::Deadline { value } => {
                    deadline = self.dur(value).map_err(stop_err)?;
                }
                _ => {}
            }
        }
        Ok((prio, deadline))
    }

    fn init_var(&mut self, m: usize, v: &VarDecl) -> Result<(), InterpError> {
        let val = match &v.init {
            Some(e) => self.init_value(e)?.deep(),
            None => self.default_value(&v.vty),
        };
        if let Value::Obj(rc) = &val {
            if rc.borrow().native {
                self.module_blocks.entry(m).or_default().push(v.id.clone());
            }
        }
        self.globals.insert(v.id.clone(), val);
        Ok(())
    }

    fn init_item(&mut self, m: usize, id: &str, def: &ItemDef) -> Result<(), InterpError> {
        match def {
            ItemDef::Io { name, kind, .. } => {
                self.io.insert(
                    id.to_string(),
                    IoState {
                        name: name.clone(),
                        kind: dotted(kind),
                        value: None,
                        props: Default::default(),
                    },
                );
                self.idx
                    .inputs_by_name
                    .entry(name.clone())
                    .or_default()
                    .push(id.to_string());
            }
            ItemDef::Var { var } => self.init_var(m, var)?,
            ItemDef::Signal {
                name, vty, init, ..
            } => {
                let v = match (init, vty) {
                    (Some(e), _) => self.init_value(e)?,
                    (None, Some(t)) => self.default_value(t),
                    (None, None) => Value::Real(0.0),
                };
                self.signals.insert(id.to_string(), v);
                self.sig_info
                    .insert(id.to_string(), SigInfo { name: name.clone() });
            }
            ItemDef::PlcSection { section, vars } => {
                for v in vars {
                    self.init_var(m, v)?;
                    match section.as_str() {
                        "inputs" => {
                            self.plc_in.push((m, v.id.clone(), v.name.clone()));
                            self.idx
                                .inputs_by_name
                                .entry(v.name.clone())
                                .or_default()
                                .push(v.id.clone());
                        }
                        "outputs" => self.plc_out.push((m, v.id.clone(), v.name.clone())),
                        _ => {}
                    }
                }
            }
            ItemDef::Fsm { fsm } => {
                for v in &fsm.vars {
                    self.init_var(m, v)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ---- the event loop -----------------------------------------------------------

    /// Runs until instant `end` (in nanoseconds).
    pub fn run_until(&mut self, end: i64) {
        loop {
            let t = match self.queue.keys().next() {
                Some(k) if k.0 <= end => k.0,
                _ => break,
            };
            self.now = t;
            let mut batch = Vec::new();
            while let Some(key) = self.queue.keys().next().copied() {
                if key.0 != t {
                    break;
                }
                if let Some(ev) = self.queue.remove(&key) {
                    batch.push(ev);
                }
            }
            for ev in batch {
                self.processed += 1;
                self.process(ev);
            }
            self.drain_ready();
            if self.processed > self.opts.max_events {
                break;
            }
        }
        if self.now < end {
            self.now = end;
        }
    }

    fn process(&mut self, ev: Ev) {
        match ev {
            Ev::Commit(id, v) => self.commit_signal(&id, v),
            Ev::Input(name, v) => self.apply_named_input(&name, &v),
            Ev::Raise(key) => self.raise(&key),
            Ev::Press(w) => self.press(&w),
            Ev::Release(a) => {
                if let Some(p) = self.acts[a].period {
                    let t = self.now.saturating_add(p);
                    self.push_event(t, Ev::Release(a));
                }
                self.make_ready(a, None);
            }
            Ev::Exec(a, rel, snap) => self.make_ready_at(a, rel, Some(snap)),
            Ev::Done(h) => {
                let path = match self.handles.get_mut(h) {
                    Some(st) => {
                        st.busy = false;
                        st.done = true;
                        st.path.clone()
                    }
                    None => return,
                };
                if !path.is_empty() {
                    self.inputs
                        .insert(format!("{path}.busy"), Value::Bool(false));
                    self.inputs
                        .insert(format!("{path}.done"), Value::Bool(true));
                    self.trace_push(TraceKind::Event, path, "done".to_string());
                }
            }
        }
    }

    fn make_ready(&mut self, a: usize, snap: Option<Snap>) {
        let rel = self.now;
        self.make_ready_at(a, rel, snap);
    }

    fn make_ready_at(&mut self, a: usize, rel: i64, snap: Option<Snap>) {
        if self.halted && self.acts[a].kind != ActKind::Safety {
            return;
        }
        self.seq_no += 1;
        self.ready.push(Ready {
            act: a,
            rel,
            seq: self.seq_no,
            snap,
        });
    }

    pub(crate) fn drain_ready(&mut self) {
        // An activity that raises an event it waits for itself would never end.
        let mut guard = 0u32;
        while !self.ready.is_empty() {
            guard += 1;
            if guard > 100_000 {
                self.ready.clear();
                self.trace_push(TraceKind::Event, "event storm".to_string(), String::new());
                break;
            }
            let mut best = 0;
            for (k, r) in self.ready.iter().enumerate() {
                let b = &self.ready[best];
                let (pk, pb) = (self.acts[r.act].prio, self.acts[b.act].prio);
                if pk > pb || (pk == pb && r.seq < b.seq) {
                    best = k;
                }
            }
            let r = self.ready.remove(best);
            self.run_ready(r);
        }
    }

    fn next_rand(&mut self, max: i64) -> i64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        i64::try_from(x % (u64::try_from(max.max(0)).unwrap_or(0) + 1)).unwrap_or(0)
    }

    fn run_ready(&mut self, r: Ready) {
        let a = r.act;
        let kind = self.acts[a].kind;
        if self.halted && kind != ActKind::Safety {
            return;
        }
        let let_sem = matches!(kind, ActKind::Task | ActKind::Interrupt);
        if r.snap.is_none() && let_sem {
            if let Some(j) = self.opts.jitter {
                let d = self.next_rand(j.max_ns);
                if d > 0 {
                    let snap = Box::new((self.signals.clone(), self.inputs.clone()));
                    let t = self.now + d;
                    self.push_event(t, Ev::Exec(a, r.rel, snap));
                    return;
                }
            }
        }
        let (snap_sig, snap_in) = match r.snap {
            Some(b) => (Some(b.0), Some(b.1)),
            None if let_sem => (Some(self.signals.clone()), None),
            None => (None, None),
        };
        self.ctx = Ctx {
            act: Some(a),
            rel: r.rel,
            period: self.acts[a].period.unwrap_or(0),
            deadline: self.acts[a].deadline,
            snap_sig,
            snap_in,
            writes: Vec::new(),
            immediate: !let_sem,
        };
        self.locals.clear();
        self.names.clear();
        self.steps = 0;
        self.cur_module = self.acts[a].module;
        let res = self.exec_act(a);
        let writes = std::mem::take(&mut self.ctx.writes);
        let (deadline, rel) = (self.ctx.deadline, self.ctx.rel);
        self.ctx = Ctx::default();
        match res {
            Err(Stop::Fault(name)) => self.raise_fault(&name),
            _ => {
                for (id, v) in writes {
                    if deadline > 0 {
                        self.push_event(rel.saturating_add(deadline), Ev::Commit(id, v));
                    } else {
                        self.commit_signal(&id, v);
                    }
                }
            }
        }
    }

    fn exec_act(&mut self, a: usize) -> R<()> {
        let nir = Rc::clone(&self.nir);
        let (m, i, kind) = (self.acts[a].module, self.acts[a].item, self.acts[a].kind);
        match kind {
            ActKind::Task | ActKind::Interrupt => match &nir.modules[m].items[i].def {
                ItemDef::Task { body, .. } | ItemDef::Interrupt { body, .. } => {
                    self.exec_block(body)
                }
                _ => Ok(()),
            },
            ActKind::Rule => {
                let ItemDef::Rule { rule } = &nir.modules[m].items[i].def else {
                    return Ok(());
                };
                let mut st = std::mem::take(&mut self.acts[a].st);
                let r = match &mut st {
                    ActState::Rule(rt) => self.rule_step(rule, rt),
                    _ => Ok(()),
                };
                self.acts[a].st = st;
                r
            }
            ActKind::Fsm => {
                let mut st = std::mem::take(&mut self.acts[a].st);
                let r = match &mut st {
                    ActState::Fsm(rt) => self.fsm_step(m, i, rt),
                    _ => Ok(()),
                };
                self.acts[a].st = st;
                r
            }
            ActKind::Sequence => {
                let mut st = std::mem::take(&mut self.acts[a].st);
                let r = match &mut st {
                    ActState::Seq(rn) => {
                        // Sequence locals live across ticks.
                        self.locals = std::mem::take(&mut rn.locals);
                        self.names = std::mem::take(&mut rn.names);
                        let r = self.advance(rn);
                        rn.locals = std::mem::take(&mut self.locals);
                        rn.names = std::mem::take(&mut self.names);
                        r
                    }
                    _ => Ok(()),
                };
                self.acts[a].st = st;
                r
            }
            ActKind::Safety => {
                let mut st = std::mem::take(&mut self.acts[a].st);
                let r = match &mut st {
                    ActState::Safety(rts) => self.safety_step(m, i, rts),
                    _ => Ok(()),
                };
                self.acts[a].st = st;
                r
            }
            ActKind::Scan => self.scan_step(m, a),
            ActKind::Blocks => {
                let dt = self.dt();
                self.tick_module_blocks(m, dt);
                Ok(())
            }
        }
    }

    // ---- rules, state machines, sequences ------------------------------------------

    fn rule_step(&mut self, rule: &Rule, rt: &mut RuleRt) -> R<()> {
        let c = self.ev_val(&rule.cond)?.truthy();
        match &rule.hold {
            Some(h) => {
                let d = self.dur(h)?;
                if c {
                    let since = *rt.since.get_or_insert(self.now);
                    if !rt.fired && self.now - since >= d {
                        rt.fired = true;
                        self.exec_block(&rule.body)?;
                    }
                } else {
                    rt.since = None;
                    rt.fired = false;
                }
            }
            None => {
                let fire = c && !rt.prev;
                rt.prev = c;
                if fire {
                    self.exec_block(&rule.body)?;
                }
            }
        }
        Ok(())
    }

    fn fsm_step(&mut self, m: usize, i: usize, rt: &mut FsmRt) -> R<()> {
        let nir = Rc::clone(&self.nir);
        let ItemDef::Fsm { fsm } = &nir.modules[m].items[i].def else {
            return Ok(());
        };
        let cur = match rt.state {
            Some(s) => s,
            None => {
                let s = fsm.states.iter().position(|s| s.initial).unwrap_or(0);
                rt.state = Some(s);
                rt.entered = self.now;
                if let Some(st) = fsm.states.get(s) {
                    self.trace_push(TraceKind::State, fsm.name.clone(), st.name.clone());
                    if let Some(b) = &st.entry {
                        self.exec_block(b)?;
                    }
                }
                s
            }
        };
        let Some(st) = fsm.states.get(cur) else {
            return Ok(());
        };
        if let Some(b) = &st.during {
            self.exec_block(b)?;
        }
        for t in &st.transitions {
            let mut fire = t.when.is_none() && t.after.is_none();
            if let Some(w) = &t.when {
                fire = self.ev_val(w)?.truthy();
            }
            if !fire {
                if let Some(d) = &t.after {
                    let dur = self.dur(d)?;
                    fire = self.now - rt.entered >= dur;
                }
            }
            if !fire {
                continue;
            }
            if let Some(b) = &st.exit {
                self.exec_block(b)?;
            }
            if let Some(b) = &t.action {
                self.exec_block(b)?;
            }
            let Some(ti) = fsm.states.iter().position(|s| s.name == t.target) else {
                break;
            };
            rt.state = Some(ti);
            rt.entered = self.now;
            self.trace_push(
                TraceKind::State,
                fsm.name.clone(),
                fsm.states[ti].name.clone(),
            );
            if let Some(b) = &fsm.states[ti].entry {
                self.exec_block(b)?;
            }
            break;
        }
        Ok(())
    }

    /// Name of the current state of the state machine called `name`.
    pub(crate) fn fsm_state_name(&self, name: &str) -> String {
        for a in &self.acts {
            if a.kind == ActKind::Fsm && a.name == name {
                if let (ActState::Fsm(rt), ItemDef::Fsm { fsm }) =
                    (&a.st, &self.nir.modules[a.module].items[a.item].def)
                {
                    return rt
                        .state
                        .and_then(|s| fsm.states.get(s))
                        .map(|s| s.name.clone())
                        .unwrap_or_default();
                }
            }
        }
        String::new()
    }

    pub(crate) fn fsm_field(&self, id: &str, name: &str) -> Value {
        let Some(&(m, i)) = self.idx.items.get(id) else {
            return Value::Unit;
        };
        let ItemDef::Fsm { fsm } = &self.nir.modules[m].items[i].def else {
            return Value::Unit;
        };
        if name == "state" {
            Value::Str(self.fsm_state_name(&fsm.name))
        } else {
            Value::Unit
        }
    }

    fn jump(rn: &mut Runner, target: &str) {
        match rn
            .ops
            .iter()
            .position(|o| matches!(o, Op::Step(n) if n == target))
        {
            Some(p) => {
                rn.pc = p;
                rn.jumped = true;
            }
            None => rn.done = true,
        }
        rn.loops.clear();
    }

    /// Value of a condition. An `io` input reads its current value.
    fn cond_true(&mut self, e: &Expr) -> R<bool> {
        match self.ev(e)? {
            Value::Io(k) => Ok(self.io_read(&k).truthy()),
            v => Ok(v.truthy()),
        }
    }

    /// Runs one `LoopNext`: starts the next iteration or leaves the loop.
    fn loop_next(&mut self, rn: &mut Runner) -> R<()> {
        let Some(mut st) = rn.loops.pop() else {
            rn.pc += 1;
            return Ok(());
        };
        let go = match st.spec.clone() {
            LoopSpec::For { pattern, .. } => match st.items.get(st.idx).cloned() {
                Some(item) => {
                    st.idx += 1;
                    self.bind_pat(&pattern, &item)?;
                    true
                }
                None => false,
            },
            LoopSpec::While { pattern, cond, .. } => {
                if st.n >= st.bound {
                    false
                } else {
                    let c = self.ev(&cond)?;
                    let ok = match &pattern {
                        Some(p) => self.bind_pat(p, &c)?,
                        None => match c {
                            Value::Io(k) => self.io_read(&k).truthy(),
                            other => other.truthy(),
                        },
                    };
                    if ok {
                        st.n += 1;
                    }
                    ok
                }
            }
            LoopSpec::Loop { .. } => {
                if st.n >= st.bound {
                    false
                } else {
                    st.n += 1;
                    true
                }
            }
        };
        if go {
            rn.pc += 1;
            rn.loops.push(st);
        } else {
            rn.pc = st.exit;
        }
        Ok(())
    }

    /// Starts a flattened loop: evaluates what it iterates over.
    fn loop_begin(&mut self, rn: &mut Runner, spec: &LoopSpec, exit: usize) -> R<()> {
        let mut st = LoopState {
            spec: spec.clone(),
            exit,
            head: rn.pc + 1,
            items: Vec::new(),
            idx: 0,
            n: 0,
            bound: 0,
        };
        match spec {
            LoopSpec::For { iter, .. } => {
                st.items = match self.ev(iter)? {
                    Value::Range(lo, hi, inc) => {
                        let (a, b) = (lo.as_int().unwrap_or(0), hi.as_int().unwrap_or(0));
                        let end = if inc { b.saturating_add(1) } else { b };
                        if end.saturating_sub(a) > MAX_ARRAY as i128 {
                            return Err(fault("LoopBound"));
                        }
                        (a..end).map(Value::Int).collect()
                    }
                    Value::Array(v) | Value::Tuple(v) => v,
                    _ => Vec::new(),
                };
            }
            LoopSpec::While { max, .. } | LoopSpec::Loop { max } => {
                st.bound = self.loop_bound(max.as_ref())?;
            }
        }
        rn.loops.push(st);
        rn.pc += 1;
        Ok(())
    }

    /// Runs a sequence (or a branch) until it has to wait.
    fn advance(&mut self, rn: &mut Runner) -> R<()> {
        for _ in 0..10_000 {
            if rn.done {
                return Ok(());
            }
            if let Some(subs) = rn.par.as_mut() {
                let mut all = true;
                for s in subs.iter_mut() {
                    self.advance(s)?;
                    all &= s.done;
                }
                if all {
                    rn.par = None;
                    rn.pc += 1;
                    continue;
                }
                return Ok(());
            }
            if let Some(w) = rn.wait.clone() {
                match w {
                    Wait::Time(t) => {
                        if self.now >= t {
                            rn.wait = None;
                            rn.pc += 1;
                            continue;
                        }
                        return Ok(());
                    }
                    Wait::Until {
                        cond,
                        deadline,
                        target,
                    } => {
                        if self.cond_true(&cond)? {
                            rn.wait = None;
                            rn.pc += 1;
                            continue;
                        }
                        if deadline.is_some_and(|d| self.now >= d) {
                            rn.wait = None;
                            self.trace_push(
                                TraceKind::Event,
                                rn.name.clone(),
                                "timeout".to_string(),
                            );
                            match target {
                                Some(t) => Self::jump(rn, &t),
                                None => rn.pc += 1,
                            }
                            continue;
                        }
                        return Ok(());
                    }
                }
            }
            let Some(op) = rn.ops.get(rn.pc).cloned() else {
                rn.done = true;
                return Ok(());
            };
            match op {
                Op::Step(name) => {
                    if rn.handlers && !rn.jumped {
                        // Robot programs: the steps after the motions are handlers that
                        // only a jump or a timeout enters.
                        rn.done = true;
                        return Ok(());
                    }
                    rn.jumped = false;
                    self.trace_push(TraceKind::State, rn.name.clone(), name);
                    rn.pc += 1;
                }
                Op::LoopBegin { spec, exit } => self.loop_begin(rn, &spec, exit)?,
                Op::LoopNext => self.loop_next(rn)?,
                Op::LoopEnd { head } => rn.pc = head,
                Op::Par(branches) => {
                    rn.par = Some(
                        branches
                            .into_iter()
                            .map(|ops| Runner {
                                name: rn.name.clone(),
                                ops,
                                handlers: rn.handlers,
                                ..Runner::default()
                            })
                            .collect(),
                    );
                }
                Op::Stmt(s) => match &s.s {
                    StmtKind::Until {
                        cond,
                        timeout,
                        on_timeout,
                    } => {
                        let deadline = match timeout {
                            Some(t) => Some(self.now.saturating_add(self.dur(t)?)),
                            None => None,
                        };
                        rn.wait = Some(Wait::Until {
                            cond: cond.clone(),
                            deadline,
                            target: on_timeout.clone(),
                        });
                    }
                    StmtKind::Wait { duration } => {
                        let d = self.dur(duration)?;
                        rn.wait = Some(Wait::Time(self.now.saturating_add(d)));
                    }
                    StmtKind::Jump { target } => Self::jump(rn, target),
                    StmtKind::Move { kind, to, via, .. } => {
                        let mut target = self.ev(to)?.show();
                        if let Some(v) = via {
                            target = format!("{target} via {}", self.ev(v)?.show());
                        }
                        self.trace_push(TraceKind::Motion, kind.clone(), target);
                        rn.wait = Some(Wait::Time(self.now.saturating_add(self.opts.move_ns)));
                    }
                    _ => {
                        match self.exec_stmt(&s) {
                            Ok(()) => {}
                            Err(Stop::Return(_)) => {
                                rn.done = true;
                                return Ok(());
                            }
                            Err(Stop::Break) if !rn.loops.is_empty() => {
                                if let Some(l) = rn.loops.pop() {
                                    rn.pc = l.exit;
                                }
                                continue;
                            }
                            Err(Stop::Continue) if !rn.loops.is_empty() => {
                                if let Some(l) = rn.loops.last() {
                                    rn.pc = l.head;
                                }
                                continue;
                            }
                            Err(e) => return Err(e),
                        }
                        rn.pc += 1;
                    }
                },
            }
        }
        Ok(())
    }

    // ---- PLC scan ---------------------------------------------------------------

    fn scan_step(&mut self, m: usize, a: usize) -> R<()> {
        let nir = Rc::clone(&self.nir);
        for (pm, id, _) in self.plc_in.clone() {
            if pm == m {
                if let Some(v) = self.input_get(&id) {
                    self.globals.insert(id, v);
                }
            }
        }
        for (j, it) in nir.modules[m].items.iter().enumerate() {
            match &it.def {
                ItemDef::Rung { body, .. } => self.exec_block(body)?,
                ItemDef::Rule { rule } => {
                    let mut rt = self.rules.remove(&(m, j)).unwrap_or_default();
                    let r = self.rule_step(rule, &mut rt);
                    self.rules.insert((m, j), rt);
                    r?;
                }
                _ => {}
            }
        }
        let dt = self.acts[a].period.unwrap_or(DEFAULT_RATE_NS) as f64 / 1.0e9;
        self.tick_module_blocks(m, dt);
        for (pm, id, name) in self.plc_out.clone() {
            if pm != m {
                continue;
            }
            let v = self.globals.get(&id).cloned().unwrap_or(Value::Unit);
            let changed = self.plc_last.get(&id).is_none_or(|p| !p.same(&v));
            if changed {
                self.trace_push(TraceKind::Output, name, v.show());
                self.plc_last.insert(id, v);
            }
        }
        Ok(())
    }

    // ---- safety -----------------------------------------------------------------

    fn safety_step(&mut self, m: usize, i: usize, rts: &mut [RuleRt]) -> R<()> {
        let nir = Rc::clone(&self.nir);
        let ItemDef::Safety { members } = &nir.modules[m].items[i].def else {
            return Ok(());
        };
        for (k, mem) in members.iter().enumerate() {
            match &mem.def {
                SafetyDef::Rule { rule } => self.rule_step(rule, &mut rts[k])?,
                SafetyDef::Reset { cond }
                    if self.fault.is_some() && self.ev_val(cond)?.truthy() =>
                {
                    self.fault = None;
                    self.halted = false;
                    self.trace_push(TraceKind::Reset, "fault".to_string(), String::new());
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Enters the safe state: applies every `safe_state` block, stops every activity
    /// except the safety logic, and runs the `on fault` handlers.
    pub(crate) fn raise_fault(&mut self, name: &str) {
        if self.fault.is_some() {
            return;
        }
        self.fault = Some(name.to_string());
        self.halted = true;
        self.trace_push(TraceKind::Fault, name.to_string(), String::new());
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_names = std::mem::take(&mut self.names);
        let saved_ctx = std::mem::replace(
            &mut self.ctx,
            Ctx {
                immediate: true,
                ..Ctx::default()
            },
        );
        let nir = Rc::clone(&self.nir);
        for (m, module) in nir.modules.iter().enumerate() {
            self.cur_module = m;
            for item in &module.items {
                if let ItemDef::Safety { members } = &item.def {
                    for mem in members {
                        if let SafetyDef::SafeState { body } = &mem.def {
                            let _ = self.exec_block(body);
                        }
                    }
                }
            }
        }
        for (m, module) in nir.modules.iter().enumerate() {
            self.cur_module = m;
            for item in &module.items {
                if let ItemDef::Safety { members } = &item.def {
                    for mem in members {
                        if let SafetyDef::OnFault { fault, body } = &mem.def {
                            if fault.as_deref().is_none_or(|f| f == name) {
                                let _ = self.exec_block(body);
                            }
                        }
                    }
                }
            }
        }
        self.locals = saved_locals;
        self.names = saved_names;
        self.ctx = saved_ctx;
    }

    // ---- signals, inputs and events -------------------------------------------------

    pub(crate) fn commit_signal(&mut self, id: &str, v: Value) {
        let changed = self.signals.get(id).is_none_or(|p| !p.same(&v));
        self.signals.insert(id.to_string(), v.clone());
        if changed {
            let name = self
                .sig_info
                .get(id)
                .map_or_else(|| id.to_string(), |i| i.name.clone());
            self.trace_push(TraceKind::Signal, name, v.show());
        }
    }

    /// Raises an event by name: wakes every task that waits for it.
    pub(crate) fn raise(&mut self, key: &str) {
        if let Some(subs) = self.event_subs.get(key).cloned() {
            for a in subs {
                self.make_ready(a, None);
            }
        }
    }

    /// `emit E`: records the event and raises it.
    pub(crate) fn emit_event(&mut self, key: &str) {
        self.trace_push(TraceKind::Event, key.to_string(), String::new());
        self.raise(key);
    }

    pub(crate) fn apply_named_input(&mut self, name: &str, v: &Value) {
        let keys = self
            .idx
            .inputs_by_name
            .get(name)
            .cloned()
            .unwrap_or_else(|| vec![name.to_string()]);
        for k in keys {
            self.apply_input(&k, v.clone());
        }
    }

    pub(crate) fn apply_input(&mut self, key: &str, v: Value) {
        let old = self.inputs.insert(key.to_string(), v.clone());
        let name = self
            .io
            .get(key)
            .map(|s| s.name.clone())
            .or_else(|| {
                self.plc_in
                    .iter()
                    .find(|(_, id, _)| id == key)
                    .map(|(_, _, n)| n.clone())
            })
            .unwrap_or_else(|| key.to_string());
        let (was, is) = (old.as_ref().is_some_and(Value::truthy), v.truthy());
        let differs = old.as_ref().is_none_or(|o| !o.same(&v));
        if was != is {
            self.raise(&format!("{name}.{}", if is { "rising" } else { "falling" }));
        }
        if differs {
            self.raise(&format!("{name}.changed"));
        }
    }

    /// A press of a widget of a `ui` module: runs its `on press` handler.
    fn press(&mut self, widget: &str) {
        let nir = Rc::clone(&self.nir);
        let saved_ctx = std::mem::replace(
            &mut self.ctx,
            Ctx {
                immediate: true,
                ..Ctx::default()
            },
        );
        for (m, module) in nir.modules.iter().enumerate() {
            for item in &module.items {
                if let ItemDef::Screen { widgets, .. } = &item.def {
                    if let Some(w) = find_widget(widgets, widget) {
                        self.cur_module = m;
                        for ev in w.events.iter().filter(|e| e.name == "press") {
                            self.locals.clear();
                            if let Err(Stop::Fault(f)) = self.exec_block(&ev.body) {
                                self.raise_fault(&f);
                            }
                        }
                    }
                }
            }
        }
        self.ctx = saved_ctx;
        self.drain_ready();
    }
}
