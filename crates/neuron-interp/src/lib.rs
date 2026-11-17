//! Reference interpreter for Neuron.
//!
//! Runs a NIR package ([`neuron_nir::Nir`]) in simulated time and records what it does.
//! Specification: `spec/language/17-interpreter.md`, with the models of computation of
//! chapter 14.
//!
//! ```
//! use neuron_interp::{Interp, Options};
//! use neuron_sema::{analyze, SemaOptions, SourceFile};
//!
//! let src = "module blink for mcu\nio led: gpio.output @ D13\ntask t every 500 ms:\n  led.toggle()\nsafety:\n  safe_state:\n    led.write(false)\n";
//! let a = analyze(vec![SourceFile { path: "blink.neuron".into(), text: src.into() }], None, &SemaOptions::default());
//! let nir = a.nir.expect("no errors");
//! let mut it = Interp::new(nir, Options { end_ns: 1_200_000_000, ..Options::default() }).expect("loads");
//! it.run();
//! let outputs: Vec<_> = it.trace().iter().map(|e| e.render()).collect();
//! assert_eq!(outputs.len(), 3); // toggles at 0, 500 and 1000 ms
//! ```

mod builtin;
mod calls;
mod eval;
mod sched;
mod testing;

pub mod scenario;
pub mod trace;
pub mod value;

pub use scenario::{parse_duration, parse_scenario, Action, Scenario, Step};
pub use testing::{run_tests, TestReport};
pub use trace::{fmt_time, render, TraceEvent, TraceKind};
pub use value::Value;

use neuron_nir::{ItemDef, Nir, Type};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::rc::Rc;
use value::UnitInfo;

/// Why a run could not start or stopped.
#[derive(Debug, Clone, PartialEq)]
pub struct InterpError {
    /// What went wrong.
    pub message: String,
}

impl fmt::Display for InterpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InterpError {}

pub(crate) fn err<T>(m: impl Into<String>) -> Result<T, InterpError> {
    Err(InterpError { message: m.into() })
}

/// Settings of a run.
#[derive(Debug, Clone)]
pub struct Options {
    /// Instant at which the run stops, in nanoseconds.
    pub end_ns: i64,
    /// Random execution delays, to check that signals do not depend on them (LET-1).
    pub jitter: Option<Jitter>,
    /// Time a device action (`run arm.pick()`) takes in the model of a device.
    pub action_ns: i64,
    /// Time a robot motion takes.
    pub move_ns: i64,
    /// Evaluation period of sequences, state machines without a period and rules.
    pub base_tick_ns: i64,
    /// Upper bound of processed events, a guard against runaway programs.
    pub max_events: u64,
}

/// Random execution delays for the LET-1 check.
#[derive(Debug, Clone, Copy)]
pub struct Jitter {
    /// Seed of the generator.
    pub seed: u64,
    /// Largest delay between a release and the start of its body.
    pub max_ns: i64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            end_ns: 1_000_000_000,
            jitter: None,
            action_ns: 100_000_000,
            move_ns: 1_000_000_000,
            base_tick_ns: 1_000_000,
            max_events: 5_000_000,
        }
    }
}

/// How a statement or expression stops early.
#[derive(Debug)]
pub(crate) enum Stop {
    Fault(String),
    Return(Value),
    Break,
    Continue,
}

pub(crate) type R<T> = Result<T, Stop>;

pub(crate) fn fault(name: &str) -> Stop {
    Stop::Fault(name.to_string())
}

/// State of an `io` binding.
#[derive(Debug, Clone)]
pub(crate) struct IoState {
    pub name: String,
    pub kind: String,
    pub value: Option<Value>,
    pub props: BTreeMap<String, Value>,
}

/// State of an asynchronous action.
#[derive(Debug, Clone)]
pub(crate) struct HandleState {
    pub busy: bool,
    pub done: bool,
    pub failed: bool,
    pub timeout: bool,
    /// Device that runs the action (empty for actions that finished at once).
    pub path: String,
}

/// Signals and inputs as they were when an activity was released.
pub(crate) type Snap = Box<(BTreeMap<String, Value>, BTreeMap<String, Value>)>;

/// Name and unit of a signal.
#[derive(Debug, Clone)]
pub(crate) struct SigInfo {
    pub name: String,
}

/// Lookup tables over the package.
#[derive(Debug, Default)]
pub(crate) struct Index {
    /// Item id to (module, item).
    pub items: BTreeMap<String, (usize, usize)>,
    /// (class id, method name) to (module, item, method index).
    pub methods: BTreeMap<(String, String), (usize, usize, usize)>,
    /// (module, item name) to (module, item).
    pub by_name: BTreeMap<(usize, String), (usize, usize)>,
    /// Name of an input to the keys under which it is stored.
    pub inputs_by_name: BTreeMap<String, Vec<String>>,
}

/// What the running activity may see and do.
#[derive(Debug, Default, Clone)]
pub(crate) struct Ctx {
    pub act: Option<usize>,
    pub rel: i64,
    pub period: i64,
    pub deadline: i64,
    pub snap_sig: Option<BTreeMap<String, Value>>,
    pub snap_in: Option<BTreeMap<String, Value>>,
    pub writes: Vec<(String, Value)>,
    pub immediate: bool,
}

/// Something scheduled for a future instant.
#[derive(Debug, Clone)]
pub(crate) enum Ev {
    Release(usize),
    Raise(String),
    Input(String, Value),
    Commit(String, Value),
    Done(usize),
    Press(String),
    Exec(usize, i64, Snap),
}

/// An activation waiting to run at the current instant.
#[derive(Debug, Clone)]
pub(crate) struct Ready {
    pub act: usize,
    pub rel: i64,
    pub seq: u64,
    pub snap: Option<Snap>,
}

/// The interpreter.
pub struct Interp {
    pub(crate) nir: Rc<Nir>,
    pub(crate) opts: Options,
    pub(crate) now: i64,
    pub(crate) idx: Index,
    pub(crate) globals: BTreeMap<String, Value>,
    pub(crate) signals: BTreeMap<String, Value>,
    pub(crate) sig_info: BTreeMap<String, SigInfo>,
    pub(crate) locals: BTreeMap<String, Value>,
    pub(crate) names: BTreeMap<String, String>,
    pub(crate) ctx: Ctx,
    pub(crate) acts: Vec<sched::Act>,
    pub(crate) queue: BTreeMap<(i64, u8, u64), Ev>,
    pub(crate) seq_no: u64,
    pub(crate) ready: Vec<Ready>,
    pub(crate) inputs: BTreeMap<String, Value>,
    pub(crate) io: BTreeMap<String, IoState>,
    pub(crate) trace: Vec<TraceEvent>,
    pub(crate) fault: Option<String>,
    pub(crate) halted: bool,
    pub(crate) handles: Vec<HandleState>,
    pub(crate) edge_sites: BTreeMap<(usize, usize, usize), bool>,
    pub(crate) rng: u64,
    pub(crate) depth: usize,
    pub(crate) event_subs: BTreeMap<String, Vec<usize>>,
    pub(crate) rules: BTreeMap<(usize, usize), sched::RuleRt>,
    pub(crate) plc_in: Vec<(usize, String, String)>,
    pub(crate) plc_out: Vec<(usize, String, String)>,
    pub(crate) plc_last: BTreeMap<String, Value>,
    pub(crate) module_blocks: BTreeMap<usize, Vec<String>>,
    pub(crate) unit_cache: BTreeMap<String, Rc<UnitInfo>>,
    pub(crate) cur_module: usize,
    pub(crate) in_test: bool,
    pub(crate) fail: Option<(usize, [usize; 2], String)>,
    pub(crate) processed: u64,
    pub(crate) consts_busy: BTreeSet<String>,
    pub(crate) in_run: bool,
    pub(crate) steps: u64,
}

impl Interp {
    /// Loads a package: creates the state of every module and schedules its activities.
    ///
    /// # Errors
    /// Fails when the initial value of a variable cannot be computed.
    pub fn new(nir: Nir, opts: Options) -> Result<Interp, InterpError> {
        let seed = opts.jitter.map_or(0x9E37_79B9_7F4A_7C15, |j| j.seed | 1);
        let mut it = Interp {
            idx: Index::build(&nir),
            nir: Rc::new(nir),
            opts,
            now: 0,
            globals: BTreeMap::new(),
            signals: BTreeMap::new(),
            sig_info: BTreeMap::new(),
            locals: BTreeMap::new(),
            names: BTreeMap::new(),
            ctx: Ctx::default(),
            acts: Vec::new(),
            queue: BTreeMap::new(),
            seq_no: 0,
            ready: Vec::new(),
            inputs: BTreeMap::new(),
            io: BTreeMap::new(),
            trace: Vec::new(),
            fault: None,
            halted: false,
            handles: Vec::new(),
            edge_sites: BTreeMap::new(),
            rng: seed,
            depth: 0,
            event_subs: BTreeMap::new(),
            rules: BTreeMap::new(),
            plc_in: Vec::new(),
            plc_out: Vec::new(),
            plc_last: BTreeMap::new(),
            module_blocks: BTreeMap::new(),
            unit_cache: BTreeMap::new(),
            cur_module: 0,
            in_test: false,
            fail: None,
            processed: 0,
            consts_busy: BTreeSet::new(),
            in_run: false,
            steps: 0,
        };
        it.init()?;
        Ok(it)
    }

    /// Schedules the steps of a scenario.
    pub fn load_scenario(&mut self, sc: &Scenario) {
        for s in &sc.steps {
            let ev = match &s.action {
                Action::Set(name, v) => Ev::Input(name.clone(), v.clone()),
                Action::Pulse(name) => Ev::Raise(name.clone()),
                Action::Press(w) => Ev::Press(w.clone()),
            };
            self.push_event(s.t_ns, ev);
        }
    }

    /// Runs until the end instant of the options.
    pub fn run(&mut self) {
        let end = self.opts.end_ns;
        self.run_until(end);
    }

    /// The observation so far.
    pub fn trace(&self) -> &[TraceEvent] {
        &self.trace
    }

    /// The fault the program is in, if any.
    pub fn current_fault(&self) -> Option<&str> {
        self.fault.as_deref()
    }

    /// The current instant in nanoseconds.
    pub fn now_ns(&self) -> i64 {
        self.now
    }

    /// Reads the committed value of a signal by name, as text.
    pub fn signal(&self, name: &str) -> Option<String> {
        self.sig_info
            .iter()
            .find(|(_, i)| i.name == name)
            .and_then(|(id, _)| self.signals.get(id))
            .map(Value::show)
    }
}

impl Index {
    fn build(nir: &Nir) -> Index {
        let mut ix = Index::default();
        for (m, module) in nir.modules.iter().enumerate() {
            for (i, item) in module.items.iter().enumerate() {
                ix.items.insert(item.id.clone(), (m, i));
                if let Some(name) = item_name(&item.def) {
                    ix.by_name.entry((m, name.to_string())).or_insert((m, i));
                }
                if let ItemDef::Class { methods, .. } = &item.def {
                    for (k, meth) in methods.iter().enumerate() {
                        if let ItemDef::Fn { func } = &meth.def {
                            ix.methods
                                .insert((item.id.clone(), func.sig.name.clone()), (m, i, k));
                        }
                    }
                }
            }
        }
        ix
    }
}

/// Name of an item, when it has one.
pub(crate) fn item_name(def: &ItemDef) -> Option<&str> {
    Some(match def {
        ItemDef::Const { name, .. }
        | ItemDef::Signal { name, .. }
        | ItemDef::Struct { name, .. }
        | ItemDef::Enum { name, .. }
        | ItemDef::Class { name, .. }
        | ItemDef::Interface { name, .. }
        | ItemDef::Alias { name, .. }
        | ItemDef::Task { name, .. }
        | ItemDef::Sequence { name, .. }
        | ItemDef::Io { name, .. }
        | ItemDef::Device { name, .. }
        | ItemDef::Zone { name, .. }
        | ItemDef::Event { name, .. }
        | ItemDef::Tag { name, .. }
        | ItemDef::Clock { name, .. }
        | ItemDef::Geometry { name, .. }
        | ItemDef::Display { name, .. }
        | ItemDef::Screen { name, .. }
        | ItemDef::Theme { name, .. }
        | ItemDef::Rung { name, .. }
        | ItemDef::Unit { name, .. } => name,
        ItemDef::Var { var } => &var.name,
        ItemDef::Fn { func } => &func.sig.name,
        ItemDef::Fsm { fsm } => &fsm.name,
        ItemDef::Circuit { circuit } => &circuit.name,
        _ => return None,
    })
}

/// `true` when a resolved type is an integer type with a known range.
pub(crate) fn int_kind(ty: &Type) -> Option<&str> {
    match ty {
        Type::Num { kind, unit: None } if kind.starts_with(['u', 'i']) && kind != "integer" => {
            Some(kind.as_str())
        }
        _ => None,
    }
}
