//! Co-simulation of a cell (task D6.1, requirement D-R8).
//!
//! One reference interpreter per device, each loaded with the device package of
//! [`crate::build`] (placed modules, link endpoints, watchdog and lock modules), stepped
//! together on one cell clock with 1 ns resolution. The clock jumps to the next instant at
//! which any device has something scheduled, a value is due, a drive or a cut happens, so
//! every device runs at its own period.
//!
//! How values cross. A link watches two sources on its sender: the user symbol named
//! after `device.` in the `link` (a `signal`, variable, PLC variable or `io`, when a placed
//! module declares it) and the endpoint signal `l<i>_value`. When a source changes, the
//! new value is queued and delivered `latency` later to the same two places on the
//! receiver: the user symbol and `l<i>_value`. The latency is the worst case of the link
//! table (spec 14.8), or zero with [`LinkModel::Ideal`]. A queue is FIFO, so values never
//! overtake each other. The transport adapter is modelled by `l<i>_alive` on the
//! receiver: true from the start, false from the instant a link is cut, which starts the
//! watchdog of the receiving device (task D3.3).
//!
//! A cut `link@t` drops every value sent at or after `t` and every value that would be
//! delivered at or after `t`. The observable trace is the union of the device traces,
//! tagged with the device name and ordered by time, then by device order in the plan, then
//! by the order inside the device.
//!
//! What is not modelled: the handshake registers (`seq`, `ack`, `busy`, `done`) of the
//! endpoints, heartbeats as frames, loss other than a cut, cell `sequence` items (they
//! need `run device.action()` on a device), FPGA circuits (the reference interpreter
//! does not run `circuit` items; such a device only takes part through its endpoints).
use crate::build::package_with_endpoints;
use crate::{
    device_package, endpoint_sources, link_plans, link_table, zone_plans, zone_sources, CellDiag,
    CellPlan, Library, LinkPlan,
};
use neuron_interp::{Interp, Options, TraceKind, Value};
use neuron_nir::Nir;
use neuron_sema::SourceFile;
use serde::Serialize;
use std::collections::BTreeMap;

/// Largest number of rounds at one instant before the run reports that it did not settle.
const SETTLE_ROUNDS: usize = 1000;

/// How the links behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkModel {
    /// Each link delays its values by the worst-case latency of the link table.
    Computed,
    /// Zero latency and no loss (the model of partition equivalence).
    Ideal,
}

/// A link loss: from `at_ns` no value is delivered on link number `link`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Cut {
    /// Position of the link among the `link` items of the cell.
    pub link: usize,
    /// Instant in nanoseconds.
    pub at_ns: u64,
}

/// A value written into a device from outside (a test stimulus).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveValue {
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Real number.
    Real(f64),
}

impl DriveValue {
    fn value(self) -> Value {
        match self {
            DriveValue::Bool(b) => Value::Bool(b),
            DriveValue::Int(i) => Value::Int(i128::from(i)),
            DriveValue::Real(r) => Value::Real(r),
        }
    }
}

/// A stimulus: write `name` of `device` at `t_ns`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Drive {
    /// Instant in nanoseconds.
    pub t_ns: u64,
    /// Device instance.
    pub device: String,
    /// Signal, variable or input name.
    pub name: String,
    /// Value.
    pub value: DriveValue,
}

/// Settings of a co-simulation.
#[derive(Debug, Clone, PartialEq)]
pub struct SimOptions {
    /// Instant at which the run stops, in nanoseconds.
    pub end_ns: u64,
    /// Link behaviour.
    pub link_model: LinkModel,
    /// Link losses.
    pub cuts: Vec<Cut>,
    /// Stimuli.
    pub drives: Vec<Drive>,
}

impl Default for SimOptions {
    fn default() -> Self {
        SimOptions {
            end_ns: 1_000_000_000,
            link_model: LinkModel::Computed,
            cuts: Vec::new(),
            drives: Vec::new(),
        }
    }
}

/// Why a co-simulation could not run.
#[derive(Debug, thiserror::Error)]
pub enum SimError {
    /// The cell does not compile far enough to have links and device packages.
    #[error("the cell does not compile: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Cell(Vec<CellDiag>),
    /// The interpreter could not load a device program.
    #[error("device `{device}`: {message}")]
    Device {
        /// Device instance.
        device: String,
        /// What the interpreter said.
        message: String,
    },
    /// A cut or a drive names something that does not exist.
    #[error("{0}")]
    Option(String),
}

/// What an event of the cell trace is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CellEventKind {
    /// An output changed.
    Output,
    /// A signal became visible.
    Signal,
    /// A fault was raised.
    Fault,
    /// A fault was reset.
    Reset,
    /// An event was raised.
    Event,
    /// A state machine changed state.
    State,
    /// Text printed by a program.
    Log,
    /// A robot motion started.
    Motion,
    /// A value entered a link at its sender.
    Send,
    /// A value left a link at its receiver.
    Deliver,
    /// A link was cut.
    Cut,
    /// A value was discarded because its link is cut.
    Drop,
}

impl CellEventKind {
    /// Short tag of the text rendering.
    pub fn tag(self) -> &'static str {
        match self {
            CellEventKind::Output => "OUT",
            CellEventKind::Signal => "SIG",
            CellEventKind::Fault => "FAULT",
            CellEventKind::Reset => "RESET",
            CellEventKind::Event => "EVENT",
            CellEventKind::State => "STATE",
            CellEventKind::Log => "LOG",
            CellEventKind::Motion => "MOVE",
            CellEventKind::Send => "SEND",
            CellEventKind::Deliver => "DELIV",
            CellEventKind::Cut => "CUT",
            CellEventKind::Drop => "DROP",
        }
    }

    fn of(k: TraceKind) -> CellEventKind {
        match k {
            TraceKind::Output => CellEventKind::Output,
            TraceKind::Signal => CellEventKind::Signal,
            TraceKind::Fault => CellEventKind::Fault,
            TraceKind::Reset => CellEventKind::Reset,
            TraceKind::Event => CellEventKind::Event,
            TraceKind::State => CellEventKind::State,
            TraceKind::Log => CellEventKind::Log,
            TraceKind::Motion => CellEventKind::Motion,
        }
    }
}

/// One entry of the observable cell trace (spec 14.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellEvent {
    /// Instant in nanoseconds since the start.
    pub t_ns: u64,
    /// Device instance the event belongs to (the receiver for a delivery or a cut).
    pub device: String,
    /// Kind.
    pub kind: CellEventKind,
    /// Name of the signal, output, fault, event, or `from -> to` for a link event.
    pub name: String,
    /// Value as text (empty when there is none).
    pub value: String,
    /// Link number for `send`, `deliver`, `cut` and `drop`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<usize>,
}

impl CellEvent {
    /// One line of text: `   500.000 ms  plc    OUT    motor = true`.
    pub fn render(&self) -> String {
        let t = neuron_interp::fmt_time(i64::try_from(self.t_ns).unwrap_or(i64::MAX));
        let head = format!(
            "{t:>14}  {:<8} {:<5}  {}",
            self.device,
            self.kind.tag(),
            self.name
        );
        if self.value.is_empty() {
            head
        } else {
            format!("{head} = {}", self.value)
        }
    }
}

/// `true` for names generated by the cell compiler for link endpoints, watchdogs and
/// locks (`l0_value`, `l2_alive`, `z_press_req_arm`): plumbing, not program behaviour.
pub fn is_plumbing(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('l') else {
        return name.starts_with("z_");
    };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && rest[digits..].starts_with('_')
}

/// A device of a co-simulation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SimDevice {
    /// Instance name.
    pub instance: String,
    /// Manifest name.
    pub manifest: String,
    /// Modules placed on it.
    pub modules: Vec<String>,
    /// `false` when its package is empty and nothing runs.
    pub runs: bool,
}

/// A link of a co-simulation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SimLink {
    /// Position among the `link` items.
    pub index: usize,
    /// Source as written.
    pub from: String,
    /// Destination as written.
    pub to: String,
    /// Medium.
    pub medium: String,
    /// Sender device.
    pub sender: String,
    /// Receiver device.
    pub receiver: String,
    /// Delivery delay in the run, in nanoseconds.
    pub latency_ns: u64,
    /// Latency budget (`within`) of the link, if it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub within_ns: Option<u64>,
    /// Watchdog period of the receiver.
    pub watchdog_ns: u64,
    /// MOC-C2 bound: watchdog plus safe-state time.
    pub bound_ns: u64,
    /// Fault the watchdog raises.
    pub fault: String,
}

/// One value that crossed a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Delivery {
    /// Link number.
    pub link: usize,
    /// Instant the sender's value changed.
    pub sent_ns: u64,
    /// Instant it reached the receiver.
    pub delivered_ns: u64,
    /// Value as text.
    pub value: String,
}

/// What happened to the receiver after a cut (MOC-C2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LossReport {
    /// Link number.
    pub link: usize,
    /// Receiving device.
    pub device: String,
    /// Instant of the cut.
    pub cut_ns: u64,
    /// Instant the receiver raised the watchdog fault, if it did.
    pub fault_ns: Option<u64>,
    /// `watchdog + safe-state time`, from the plan.
    pub bound_ns: u64,
    /// `true` when the fault came within the bound.
    pub within_bound: bool,
}

/// The result of a co-simulation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SimRun {
    /// Cell name.
    pub cell: String,
    /// End of the run in nanoseconds.
    pub end_ns: u64,
    /// Link behaviour used.
    pub link_model: LinkModel,
    /// Devices in plan order.
    pub devices: Vec<SimDevice>,
    /// Links in source order.
    pub links: Vec<SimLink>,
    /// Cuts applied.
    pub cuts: Vec<Cut>,
    /// The observable cell trace.
    pub events: Vec<CellEvent>,
    /// Every delivery, in order.
    pub deliveries: Vec<Delivery>,
    /// One report per cut link.
    pub losses: Vec<LossReport>,
    /// Things the run could not do, one line each.
    pub notes: Vec<String>,
}

impl SimRun {
    /// The cell trace as text, one line per event.
    pub fn render(&self) -> String {
        let mut s = String::new();
        for e in &self.events {
            s.push_str(&e.render());
            s.push('\n');
        }
        s
    }

    /// The events of program behaviour: no link plumbing and no link events.
    pub fn behaviour(&self) -> Vec<&CellEvent> {
        self.events
            .iter()
            .filter(|e| {
                !matches!(
                    e.kind,
                    CellEventKind::Send
                        | CellEventKind::Deliver
                        | CellEventKind::Cut
                        | CellEventKind::Drop
                ) && !is_plumbing(&e.name)
            })
            .collect()
    }
}

struct Dev {
    name: String,
    interp: Option<Interp>,
    cursor: usize,
    stalled: bool,
}

struct Pending {
    due: i64,
    sent: i64,
    value: Value,
}

struct LinkRt {
    info: SimLink,
    /// Sender sources and the last value text seen on each.
    sources: Vec<(String, Option<String>)>,
    /// Destinations on the receiver.
    dests: Vec<String>,
    cut_at: Option<i64>,
    queue: Vec<Pending>,
    sender_ix: usize,
    receiver_ix: usize,
}

fn to_i64(ns: u64) -> i64 {
    i64::try_from(ns).unwrap_or(i64::MAX)
}

fn to_u64(ns: i64) -> u64 {
    u64::try_from(ns).unwrap_or(0)
}

/// The user symbol a link end names: `plc.start` gives `start`; a dotted member such as
/// `clamp.done` is not a plain symbol and gives `None`.
fn symbol(end: &str) -> Option<String> {
    let (_, rest) = end.split_once('.')?;
    (!rest.contains('.') && !rest.is_empty()).then(|| rest.to_string())
}

struct Clock<'a> {
    devs: Vec<Dev>,
    links: Vec<LinkRt>,
    events: Vec<CellEvent>,
    deliveries: Vec<Delivery>,
    notes: Vec<String>,
    drives: &'a [Drive],
    next_drive: usize,
    cuts: &'a [Cut],
    applied_cuts: Vec<bool>,
}

impl Clock<'_> {
    fn push(
        &mut self,
        t: i64,
        device: &str,
        kind: CellEventKind,
        name: String,
        value: String,
        link: Option<usize>,
    ) {
        self.events.push(CellEvent {
            t_ns: to_u64(t),
            device: device.to_string(),
            kind,
            name,
            value,
            link,
        });
    }

    fn harvest(&mut self, d: usize) -> bool {
        let dev = &mut self.devs[d];
        let Some(it) = &dev.interp else {
            return false;
        };
        let fresh: Vec<_> = it.trace()[dev.cursor..].to_vec();
        dev.cursor += fresh.len();
        let name = dev.name.clone();
        let any = !fresh.is_empty();
        for e in fresh {
            self.events.push(CellEvent {
                t_ns: to_u64(e.t_ns),
                device: name.clone(),
                kind: CellEventKind::of(e.kind),
                name: e.name,
                value: e.value,
                link: None,
            });
        }
        any
    }

    fn write(&mut self, d: usize, name: &str, v: Value) -> bool {
        self.devs[d]
            .interp
            .as_mut()
            .is_some_and(|it| it.write_named(name, v))
    }

    /// Runs one round at instant `t`. Returns `true` when anything happened that can
    /// make another round necessary.
    fn round(&mut self, t: i64) -> bool {
        let mut acted = false;
        for d in &mut self.devs {
            if let Some(it) = d.interp.as_mut() {
                it.advance_clock(t);
            }
        }
        // Stimuli due now.
        while self.next_drive < self.drives.len() && to_i64(self.drives[self.next_drive].t_ns) <= t
        {
            let dr = self.drives[self.next_drive].clone();
            self.next_drive += 1;
            acted = true;
            let ix = self.devs.iter().position(|d| d.name == dr.device);
            let ok = ix.is_some_and(|d| self.write(d, &dr.name, dr.value.value()));
            if !ok {
                self.notes.push(format!(
                    "drive `{}.{}` at {} found nothing to write",
                    dr.device,
                    dr.name,
                    neuron_interp::fmt_time(to_i64(dr.t_ns))
                ));
            }
        }
        // Cuts due now.
        for c in 0..self.cuts.len() {
            if self.applied_cuts[c] || to_i64(self.cuts[c].at_ns) > t {
                continue;
            }
            self.applied_cuts[c] = true;
            acted = true;
            let Some(l) = self
                .links
                .iter()
                .position(|l| l.info.index == self.cuts[c].link)
            else {
                continue;
            };
            let at = to_i64(self.cuts[c].at_ns);
            if self.links[l].cut_at.is_none_or(|p| at < p) {
                self.links[l].cut_at = Some(at);
            }
            let (rx, name, ix) = (
                self.links[l].info.receiver.clone(),
                format!("{} -> {}", self.links[l].info.from, self.links[l].info.to),
                self.links[l].info.index,
            );
            self.push(at, &rx, CellEventKind::Cut, name, String::new(), Some(ix));
            let r = self.links[l].receiver_ix;
            self.write(r, &format!("l{ix}_alive"), Value::Bool(false));
        }
        // Deliveries due now.
        for l in 0..self.links.len() {
            let mut k = 0;
            while k < self.links[l].queue.len() {
                if self.links[l].queue[k].due > t {
                    k += 1;
                    continue;
                }
                let p = self.links[l].queue.remove(k);
                acted = true;
                let (ix, rx, name) = (
                    self.links[l].info.index,
                    self.links[l].info.receiver.clone(),
                    format!("{} -> {}", self.links[l].info.from, self.links[l].info.to),
                );
                if self.links[l].cut_at.is_some_and(|c| p.due >= c) {
                    self.push(
                        p.due,
                        &rx,
                        CellEventKind::Drop,
                        name,
                        p.value.show(),
                        Some(ix),
                    );
                    continue;
                }
                let r = self.links[l].receiver_ix;
                for dest in self.links[l].dests.clone() {
                    self.write(r, &dest, p.value.clone());
                }
                self.deliveries.push(Delivery {
                    link: ix,
                    sent_ns: to_u64(p.sent),
                    delivered_ns: to_u64(p.due),
                    value: p.value.show(),
                });
                self.push(
                    p.due,
                    &rx,
                    CellEventKind::Deliver,
                    name,
                    p.value.show(),
                    Some(ix),
                );
            }
        }
        // Run every device to `t`.
        for d in 0..self.devs.len() {
            if self.devs[d].stalled {
                continue;
            }
            if let Some(it) = self.devs[d].interp.as_mut() {
                it.run_until(t);
            }
            acted |= self.harvest(d);
        }
        // Senders.
        for l in 0..self.links.len() {
            let s = self.links[l].sender_ix;
            for k in 0..self.links[l].sources.len() {
                let name = self.links[l].sources[k].0.clone();
                let Some(v) = self.devs[s]
                    .interp
                    .as_ref()
                    .and_then(|it| it.read_named(&name))
                else {
                    continue;
                };
                let text = v.show();
                if self.links[l].sources[k].1.as_deref() == Some(text.as_str()) {
                    continue;
                }
                self.links[l].sources[k].1 = Some(text.clone());
                let (ix, tx, lname, latency) = (
                    self.links[l].info.index,
                    self.links[l].info.sender.clone(),
                    format!("{} -> {}", self.links[l].info.from, self.links[l].info.to),
                    to_i64(self.links[l].info.latency_ns),
                );
                self.push(
                    t,
                    &tx,
                    CellEventKind::Send,
                    lname.clone(),
                    text.clone(),
                    Some(ix),
                );
                if self.links[l].cut_at.is_some_and(|c| t >= c) {
                    let rx = self.links[l].info.receiver.clone();
                    self.push(t, &rx, CellEventKind::Drop, lname, text, Some(ix));
                    continue;
                }
                self.links[l].queue.push(Pending {
                    due: t.saturating_add(latency),
                    sent: t,
                    value: v,
                });
                acted |= latency == 0;
            }
        }
        acted
    }

    /// The next instant after `t` at which anything is due, if before or at `end`.
    fn next_instant(&mut self, t: i64, end: i64) -> Option<i64> {
        let mut best: Option<i64> = None;
        let mut take = |x: i64| best = Some(best.map_or(x, |b| b.min(x)));
        for d in &mut self.devs {
            if d.stalled {
                continue;
            }
            if let Some(n) = d.interp.as_ref().and_then(Interp::next_event_ns) {
                if n <= t {
                    // The interpreter stopped with work left (its event budget): ignore it.
                    d.stalled = true;
                    self.notes.push(format!(
                        "device `{}` stopped at {}: its event budget is spent",
                        d.name,
                        neuron_interp::fmt_time(t)
                    ));
                } else {
                    take(n);
                }
            }
        }
        for l in &self.links {
            for p in &l.queue {
                take(p.due.max(t + 1));
            }
        }
        if let Some(d) = self.drives.get(self.next_drive) {
            take(to_i64(d.t_ns).max(t + 1));
        }
        for (c, done) in self.cuts.iter().zip(&self.applied_cuts) {
            if !done {
                take(to_i64(c.at_ns).max(t + 1));
            }
        }
        best.filter(|b| *b <= end)
    }
}

/// Runs a cell in the reference interpreter, one instance per device, on one clock.
///
/// `plan`, `nir` and `sources` are the ones given to [`crate::build`].
///
/// # Errors
/// [`SimError::Cell`] when the links or locks of the cell do not compile (for example
/// `NE0701`), [`SimError::Device`] when the interpreter cannot load a device program,
/// [`SimError::Option`] for a cut of a link that does not exist.
pub fn cosimulate(
    plan: &CellPlan,
    nir: &Nir,
    sources: &[SourceFile],
    lib: &Library,
    opts: &SimOptions,
) -> Result<SimRun, SimError> {
    let plans = link_plans(nir, plan, lib).map_err(SimError::Cell)?;
    let reports = link_table(nir, plan, lib).map_err(SimError::Cell)?;
    let (zones, zone_diags) = zone_plans(nir, plan, lib);
    if !zone_diags.is_empty() {
        return Err(SimError::Cell(zone_diags));
    }
    for c in &opts.cuts {
        if !plans.iter().any(|l| l.index == c.link) {
            return Err(SimError::Option(format!(
                "`--cut {}@..`: the cell has no link number {}",
                c.link, c.link
            )));
        }
    }
    let mut by_device = endpoint_sources(nir, plan, lib, &plans);
    for (dev, mut srcs) in zone_sources(nir, plan, lib, &zones) {
        by_device.entry(dev).or_default().append(&mut srcs);
    }
    let mut notes = Vec::new();
    let mut devs = Vec::new();
    let mut infos = Vec::new();
    for dev in &plan.devices {
        let Some(man) = lib.device(&dev.manifest) else {
            return Err(SimError::Cell(vec![CellDiag::new(
                "NE0607",
                &plan.file,
                None,
                format!("unknown device `{}`", dev.manifest),
            )]));
        };
        let eps = by_device.remove(&dev.instance).unwrap_or_default();
        let pkg = if eps.is_empty() {
            device_package(nir, dev, man)
        } else {
            match package_with_endpoints(dev, man, sources, &eps) {
                Ok((pkg, _)) => pkg,
                Err(why) => {
                    notes.push(format!(
                        "device `{}` runs without its link endpoints: {why}",
                        dev.instance
                    ));
                    device_package(nir, dev, man)
                }
            }
        };
        let runs = !pkg.modules.is_empty();
        let interp = if runs {
            Some(
                Interp::new(
                    pkg,
                    Options {
                        max_events: 50_000_000,
                        ..Options::default()
                    },
                )
                .map_err(|e| SimError::Device {
                    device: dev.instance.clone(),
                    message: e.message,
                })?,
            )
        } else {
            None
        };
        infos.push(SimDevice {
            instance: dev.instance.clone(),
            manifest: dev.manifest.clone(),
            modules: dev.modules.iter().map(|p| p.module.clone()).collect(),
            runs,
        });
        devs.push(Dev {
            name: dev.instance.clone(),
            interp,
            cursor: 0,
            stalled: false,
        });
    }
    let mut used = vec![false; reports.len()];
    let mut links = Vec::new();
    for p in &plans {
        let Some(r) = reports.iter().enumerate().position(|(k, r)| {
            !used[k] && r.from == p.from && r.to == p.to && r.medium == p.medium
        }) else {
            continue;
        };
        used[r] = true;
        let sender_ix = devs.iter().position(|d| d.name == p.tx.device).unwrap_or(0);
        let receiver_ix = devs.iter().position(|d| d.name == p.rx.device).unwrap_or(0);
        let latency_ns = match opts.link_model {
            LinkModel::Computed => reports[r].latency.total_ns,
            LinkModel::Ideal => 0,
        };
        let mut sources_of = Vec::new();
        if let Some(s) = symbol(&p.from) {
            sources_of.push(s);
        }
        sources_of.push(format!("l{}_value", p.index));
        let mut dests = Vec::new();
        if let Some(s) = symbol(&p.to) {
            dests.push(s);
        }
        dests.push(format!("l{}_value", p.index));
        let sources_of = sources_of
            .into_iter()
            .map(|n| {
                let seen = devs[sender_ix]
                    .interp
                    .as_ref()
                    .and_then(|it| it.read_named(&n))
                    .map(|v| v.show());
                (n, seen)
            })
            .collect();
        links.push(LinkRt {
            info: sim_link(p, &reports[r], latency_ns),
            sources: sources_of,
            dests,
            cut_at: None,
            queue: Vec::new(),
            sender_ix,
            receiver_ix,
        });
    }
    let mut drives = opts.drives.clone();
    drives.sort_by_key(|d| d.t_ns);
    let mut cuts = opts.cuts.clone();
    cuts.sort_by_key(|c| (c.at_ns, c.link));
    let mut clock = Clock {
        devs,
        links,
        events: Vec::new(),
        deliveries: Vec::new(),
        notes,
        drives: &drives,
        next_drive: 0,
        applied_cuts: vec![false; cuts.len()],
        cuts: &cuts,
    };
    // The transport adapter reports every link alive from the start.
    for l in 0..clock.links.len() {
        let (r, ix) = (clock.links[l].receiver_ix, clock.links[l].info.index);
        clock.write(r, &format!("l{ix}_alive"), Value::Bool(true));
    }
    let end = to_i64(opts.end_ns);
    let mut t = 0i64;
    loop {
        let mut rounds = 0;
        while clock.round(t) {
            rounds += 1;
            if rounds >= SETTLE_ROUNDS {
                clock.notes.push(format!(
                    "the cell did not settle at {} after {SETTLE_ROUNDS} rounds",
                    neuron_interp::fmt_time(t)
                ));
                break;
            }
        }
        match clock.next_instant(t, end) {
            Some(n) => t = n,
            None => break,
        }
    }
    let mut events = clock.events;
    events.sort_by_key(|e| e.t_ns);
    let losses = loss_reports(&clock.links, &events);
    Ok(SimRun {
        cell: plan.cell.clone(),
        end_ns: opts.end_ns,
        link_model: opts.link_model,
        devices: infos,
        links: clock.links.into_iter().map(|l| l.info).collect(),
        cuts: cuts.clone(),
        events,
        deliveries: clock.deliveries,
        losses,
        notes: clock.notes,
    })
}

fn sim_link(p: &LinkPlan, r: &crate::LinkReport, latency_ns: u64) -> SimLink {
    SimLink {
        index: p.index,
        from: p.from.clone(),
        to: p.to.clone(),
        medium: p.medium.clone(),
        sender: p.tx.device.clone(),
        receiver: p.rx.device.clone(),
        latency_ns,
        within_ns: r.within_ns,
        watchdog_ns: p.watchdog.period_ns,
        bound_ns: p.watchdog.loss_bound_ns,
        fault: p.watchdog.fault.clone(),
    }
}

fn loss_reports(links: &[LinkRt], events: &[CellEvent]) -> Vec<LossReport> {
    let mut out = Vec::new();
    for l in links {
        let Some(cut) = l.cut_at else {
            continue;
        };
        let cut_ns = to_u64(cut);
        let fault_ns = events
            .iter()
            .find(|e| {
                e.device == l.info.receiver
                    && e.kind == CellEventKind::Fault
                    && e.name == l.info.fault
                    && e.t_ns >= cut_ns
            })
            .map(|e| e.t_ns);
        out.push(LossReport {
            link: l.info.index,
            device: l.info.receiver.clone(),
            cut_ns,
            fault_ns,
            bound_ns: l.info.bound_ns,
            within_bound: fault_ns.is_some_and(|f| f - cut_ns <= l.info.bound_ns),
        });
    }
    out
}

/// Runs the modules of a cell in **one** interpreter, with each link an ideal wire from
/// the sender's symbol to the receiver's symbol, delivered at the same instant. This is
/// the reference of the partition-equivalence property: the same modules, no devices, no
/// endpoints. Only links whose two ends are user symbols take part (the others have
/// nothing to connect in a single program). Drives ignore the device name.
///
/// # Errors
/// [`SimError::Cell`] when the links do not compile, [`SimError::Device`] when the
/// interpreter cannot load the package.
pub fn cosimulate_single(
    plan: &CellPlan,
    nir: &Nir,
    lib: &Library,
    opts: &SimOptions,
) -> Result<Vec<CellEvent>, SimError> {
    let plans = link_plans(nir, plan, lib).map_err(SimError::Cell)?;
    let mut pkg = nir.clone();
    pkg.modules
        .retain(|m| m.kind == neuron_nir::ModuleKind::Module);
    let mut it = Interp::new(
        pkg,
        Options {
            max_events: 50_000_000,
            ..Options::default()
        },
    )
    .map_err(|e| SimError::Device {
        device: plan.cell.clone(),
        message: e.message,
    })?;
    let wires: Vec<(String, String)> = plans
        .iter()
        .filter_map(|p| Some((symbol(&p.from)?, symbol(&p.to)?)))
        .collect();
    let mut last: Vec<Option<String>> = wires
        .iter()
        .map(|(s, _)| it.read_named(s).map(|v| v.show()))
        .collect();
    let mut drives = opts.drives.clone();
    drives.sort_by_key(|d| d.t_ns);
    let mut next_drive = 0;
    let end = to_i64(opts.end_ns);
    let mut cursor = 0;
    let mut events = Vec::new();
    let mut t = 0i64;
    loop {
        let mut rounds = 0;
        loop {
            let mut acted = false;
            it.advance_clock(t);
            while next_drive < drives.len() && to_i64(drives[next_drive].t_ns) <= t {
                it.write_named(&drives[next_drive].name, drives[next_drive].value.value());
                next_drive += 1;
                acted = true;
            }
            it.run_until(t);
            for (k, (s, d)) in wires.iter().enumerate() {
                if let Some(v) = it.read_named(s) {
                    let text = v.show();
                    if last[k].as_deref() != Some(text.as_str()) {
                        last[k] = Some(text);
                        it.write_named(d, v);
                        acted = true;
                    }
                }
            }
            let fresh: Vec<_> = it.trace()[cursor..].to_vec();
            cursor += fresh.len();
            for e in fresh {
                events.push(CellEvent {
                    t_ns: to_u64(e.t_ns),
                    device: String::new(),
                    kind: CellEventKind::of(e.kind),
                    name: e.name,
                    value: e.value,
                    link: None,
                });
            }
            rounds += 1;
            if !acted || rounds >= SETTLE_ROUNDS {
                break;
            }
        }
        let next = it.next_event_ns().filter(|n| *n > t);
        let drive = drives.get(next_drive).map(|d| to_i64(d.t_ns).max(t + 1));
        let n = match (next, drive) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => break,
        };
        if n > end {
            break;
        }
        t = n;
    }
    Ok(events)
}

/// A map from device instance to the program behaviour it showed, used by tests and the
/// dossier: `device -> events without plumbing`.
pub fn behaviour_by_device(run: &SimRun) -> BTreeMap<String, Vec<&CellEvent>> {
    let mut out: BTreeMap<String, Vec<&CellEvent>> = BTreeMap::new();
    for e in run.behaviour() {
        out.entry(e.device.clone()).or_default().push(e);
    }
    out
}
