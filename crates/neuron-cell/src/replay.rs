//! The replay format of the digital twin (task D6.2, requirement D-R8).
//!
//! A [`Replay`] is one JSON document: the cell, its devices and links, and the
//! time-ordered events of either a co-simulation run ([`Replay::from_run`]) or a
//! counterexample of the cell checker ([`Replay::from_counterexample`]). The twin plays the
//! events in order. The JSON Schema is generated from these types
//! ([`replay_schema_json`]); the committed copy is `spec/schemas/replay.schema.json` and a
//! test fails when they differ. The text is canonical: two-space indent, fields in
//! declaration order, final newline, so equal runs give byte-identical files.
//!
//! ```
//! use neuron_cell::{Replay, ReplayEvent, ReplayEventKind};
//! let r = Replay {
//!     replay_version: "1".into(),
//!     kind: neuron_cell::ReplayKind::Cosimulation,
//!     cell: "C".into(),
//!     end_ns: 10,
//!     devices: vec![],
//!     links: vec![],
//!     events: vec![ReplayEvent {
//!         t_ns: 5,
//!         device: "plc".into(),
//!         kind: ReplayEventKind::Output,
//!         name: "motor".into(),
//!         value: "true".into(),
//!         link: None,
//!     }],
//! };
//! let text = r.to_json();
//! assert_eq!(Replay::from_json(&text).unwrap(), r);
//! ```
use crate::{CellCounterexample, CellEventKind, CellPlan, CellVerdict, SimRun};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Version of the replay format.
pub const REPLAY_VERSION: &str = "1";

/// Device name given to events that belong to the cell and not to one device (the state
/// of a cell `sequence` in a counterexample, a violation).
pub const CELL_DEVICE: &str = "cell";

/// Where a replay came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplayKind {
    /// A co-simulation run of the device programs.
    Cosimulation,
    /// The shortest counterexample of a cell property, as a trace of the model.
    Counterexample,
}

/// What a replay event is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplayEventKind {
    /// A signal became visible or an input changed (`name`, `value`).
    Signal,
    /// An output changed.
    Output,
    /// A fault was raised (`name` is the fault).
    Fault,
    /// A fault was reset.
    Reset,
    /// An event was raised.
    Event,
    /// A state machine or sequence is in state `value` (`name` is the machine).
    State,
    /// Text printed by a program.
    Log,
    /// A robot motion started.
    Motion,
    /// A value entered a link at its sender.
    Send,
    /// A value reached the receiver of a link.
    Deliver,
    /// A link was cut.
    Cut,
    /// A value was discarded because its link is cut.
    Drop,
    /// The property in `name` is violated at this instant (counterexamples).
    Violation,
}

impl From<CellEventKind> for ReplayEventKind {
    fn from(k: CellEventKind) -> Self {
        match k {
            CellEventKind::Output => ReplayEventKind::Output,
            CellEventKind::Signal => ReplayEventKind::Signal,
            CellEventKind::Fault => ReplayEventKind::Fault,
            CellEventKind::Reset => ReplayEventKind::Reset,
            CellEventKind::Event => ReplayEventKind::Event,
            CellEventKind::State => ReplayEventKind::State,
            CellEventKind::Log => ReplayEventKind::Log,
            CellEventKind::Motion => ReplayEventKind::Motion,
            CellEventKind::Send => ReplayEventKind::Send,
            CellEventKind::Deliver => ReplayEventKind::Deliver,
            CellEventKind::Cut => ReplayEventKind::Cut,
            CellEventKind::Drop => ReplayEventKind::Drop,
        }
    }
}

/// A device of the cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayDevice {
    /// Instance name; the `device` of the events.
    pub name: String,
    /// Manifest name (empty when the replay does not know it).
    pub manifest: String,
    /// Modules placed on it.
    pub modules: Vec<String>,
}

/// A link of the cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayLink {
    /// Position among the `link` items; the `link` of the events.
    pub index: u32,
    /// Source as written (`plc.part_ready`).
    pub from: String,
    /// Destination as written.
    pub to: String,
    /// Medium (empty when the replay does not know it).
    pub medium: String,
    /// Delay of a value on this link, in nanoseconds.
    pub latency_ns: u64,
    /// Latency budget, when the link has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within_ns: Option<u64>,
}

/// One event of the replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayEvent {
    /// Instant in nanoseconds since the start.
    pub t_ns: u64,
    /// Device instance, or [`CELL_DEVICE`].
    pub device: String,
    /// Kind.
    pub kind: ReplayEventKind,
    /// Name of the signal, output, fault, event or machine; `from -> to` for link events;
    /// the property for a violation.
    pub name: String,
    /// Value as text (empty when there is none).
    pub value: String,
    /// Link number for `send`, `deliver`, `cut` and `drop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<u32>,
}

/// The replay document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    /// Format version, `"1"`.
    pub replay_version: String,
    /// Where it came from.
    pub kind: ReplayKind,
    /// Cell name.
    pub cell: String,
    /// End of the run (or the instant of the violation) in nanoseconds.
    pub end_ns: u64,
    /// Devices in plan order.
    pub devices: Vec<ReplayDevice>,
    /// Links in source order.
    pub links: Vec<ReplayLink>,
    /// Events ordered by time (stable: equal instants keep their order).
    pub events: Vec<ReplayEvent>,
}

fn link_index(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// Splits `a.b.c` at the last dot: `("a.b", "c")`; no dot gives `("", name)`.
fn split_last(name: &str) -> (&str, &str) {
    name.rsplit_once('.').unwrap_or(("", name))
}

impl Replay {
    /// The replay of a co-simulation run.
    pub fn from_run(run: &SimRun) -> Replay {
        Replay {
            replay_version: REPLAY_VERSION.into(),
            kind: ReplayKind::Cosimulation,
            cell: run.cell.clone(),
            end_ns: run.end_ns,
            devices: run
                .devices
                .iter()
                .map(|d| ReplayDevice {
                    name: d.instance.clone(),
                    manifest: d.manifest.clone(),
                    modules: d.modules.clone(),
                })
                .collect(),
            links: run
                .links
                .iter()
                .map(|l| ReplayLink {
                    index: link_index(l.index),
                    from: l.from.clone(),
                    to: l.to.clone(),
                    medium: l.medium.clone(),
                    latency_ns: l.latency_ns,
                    within_ns: l.within_ns,
                })
                .collect(),
            events: run
                .events
                .iter()
                .map(|e| ReplayEvent {
                    t_ns: e.t_ns,
                    device: e.device.clone(),
                    kind: e.kind.into(),
                    name: e.name.clone(),
                    value: e.value.clone(),
                    link: e.link.map(link_index),
                })
                .collect(),
        }
    }

    /// The replay of a counterexample (task D4.3).
    ///
    /// The trace of the model becomes events: at tick 0 the state of every machine
    /// (`state`, device [`CELL_DEVICE`]) and the value of every input (`signal` on the
    /// device named before the dot of `plc.part_ready`); later ticks add the machines that
    /// moved and the inputs that changed. The last event is a `violation` carrying the
    /// property, at the time of the last tick. Folding the `state` events up to that
    /// instant ([`Replay::states_at`]) gives exactly the states of the last step. Devices
    /// and links come from `plan` and `verdict.channels`.
    pub fn from_counterexample(
        plan: &CellPlan,
        verdict: &CellVerdict,
        cex: &CellCounterexample,
    ) -> Replay {
        let mut events = Vec::new();
        let mut states: BTreeMap<String, String> = BTreeMap::new();
        let mut inputs: BTreeMap<String, bool> = BTreeMap::new();
        for step in &cex.trace {
            for s in &step.states {
                let (machine, state) = split_last(s);
                if states.get(machine).map(String::as_str) != Some(state) {
                    states.insert(machine.to_string(), state.to_string());
                    events.push(ReplayEvent {
                        t_ns: step.time_ns,
                        device: CELL_DEVICE.into(),
                        kind: ReplayEventKind::State,
                        name: machine.to_string(),
                        value: state.to_string(),
                        link: None,
                    });
                }
            }
            for (name, v) in &step.inputs {
                if inputs.get(name) != Some(v) {
                    inputs.insert(name.clone(), *v);
                    let (device, signal) = name.split_once('.').unwrap_or((CELL_DEVICE, name));
                    events.push(ReplayEvent {
                        t_ns: step.time_ns,
                        device: device.to_string(),
                        kind: ReplayEventKind::Signal,
                        name: signal.to_string(),
                        value: v.to_string(),
                        link: None,
                    });
                }
            }
        }
        let end_ns = cex.trace.last().map_or(0, |s| s.time_ns);
        events.push(ReplayEvent {
            t_ns: end_ns,
            device: CELL_DEVICE.into(),
            kind: ReplayEventKind::Violation,
            name: cex.property.clone(),
            value: String::new(),
            link: None,
        });
        Replay {
            replay_version: REPLAY_VERSION.into(),
            kind: ReplayKind::Counterexample,
            cell: plan.cell.clone(),
            end_ns,
            devices: plan
                .devices
                .iter()
                .map(|d| ReplayDevice {
                    name: d.instance.clone(),
                    manifest: d.manifest.clone(),
                    modules: d.modules.iter().map(|p| p.module.clone()).collect(),
                })
                .collect(),
            links: verdict
                .channels
                .iter()
                .enumerate()
                .map(|(i, c)| ReplayLink {
                    index: link_index(i),
                    from: c.from.clone(),
                    to: c.to.clone(),
                    medium: String::new(),
                    latency_ns: c.delay_ns,
                    within_ns: None,
                })
                .collect(),
            events,
        }
    }

    /// Canonical JSON: two-space indent, final newline.
    pub fn to_json(&self) -> String {
        neuron_emit::to_json(self)
    }

    /// Reads a replay; unknown fields and a wrong version are refused.
    ///
    /// # Errors
    /// A message when the text is not a replay of this version.
    pub fn from_json(text: &str) -> Result<Replay, String> {
        let r: Replay = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if r.replay_version != REPLAY_VERSION {
            return Err(format!(
                "unsupported replay_version `{}` (expected `{REPLAY_VERSION}`)",
                r.replay_version
            ));
        }
        Ok(r)
    }

    /// The state of every machine after the `state` events up to and including `t_ns`:
    /// machine name to state name.
    pub fn states_at(&self, t_ns: u64) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for e in self
            .events
            .iter()
            .filter(|e| e.t_ns <= t_ns && e.kind == ReplayEventKind::State)
        {
            out.insert(e.name.clone(), e.value.clone());
        }
        out
    }

    /// The first violation in the replay: its instant and the property.
    pub fn violation(&self) -> Option<(u64, &str)> {
        self.events
            .iter()
            .find(|e| e.kind == ReplayEventKind::Violation)
            .map(|e| (e.t_ns, e.name.as_str()))
    }
}

/// The JSON Schema of [`Replay`] as canonical JSON text.
///
/// The committed copy is `spec/schemas/replay.schema.json`.
pub fn replay_schema_json() -> String {
    neuron_emit::to_json(&schemars::schema_for!(Replay))
}
