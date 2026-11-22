//! Cell-wide verification (task D4.3, requirement D-R7, spec 08 §8.3 and 15 §15.6).
//!
//! Two checks run the A8 model checker of `neuron-sema` on a product of machines joined
//! by bounded-delay channels:
//!
//! * [`verify_cell`] checks the `verify:` properties written in the cell over the cell's
//!   own `fsm` and `sequence` items and those of every module placed on a device. The
//!   inputs of a placed module's machines are qualified with the device
//!   (`sensor` on `plc` is `plc.sensor`), so a `link plc.x -> arm.y` becomes a channel
//!   between the atoms `plc.x` and `arm.y`.
//! * [`prove_zone`] checks the mutex of a zone: a model of the lock protocol of D4.1
//!   (four-phase request/grant, see `zones`) with one machine per party and one for the
//!   arbiter, the request and grant signals as channels, and the property that no two
//!   parties are inside the zone at once.
//!
//! # The channel abstraction
//!
//! Every tick the checker lets the environment choose each channel's delay `d`. A value
//! written at tick `t` is read by the destination at tick `t + d`, `0 <= d <= k`, in
//! order: the destination never sees an older value after a newer one, and `d` can grow
//! by one per tick. `k` is the delay in ticks, rounded up: for a `link` the `within`
//! budget (which `NE0701` already enforces), else the computed worst-case latency; for
//! a lock signal the worst one-way latency of its medium. The tick is the fastest scan
//! period of the devices that take part (and divides every duration in the product);
//! every machine is evaluated at every tick, which lets a slower device react more
//! often than it really can. Delays above 15 ticks coarsen the tick, which rounds delays
//! up. A channel whose destination no machine or property reads is left out, and its
//! destination stays a free input. Loss and duplication are not modeled: the watchdog of
//! D3.3 turns a lost link into a fault.
//!
//! # Counterexamples
//!
//! A violated property is `NE0901` with the shortest trace (breadth-first search over
//! the product), as plain data ([`CellCounterexample`]): the tick, its time, the state of
//! every machine and the value of every input. D6 replays it.
use crate::endpoints::endpoint_period;
use crate::zones::ZonePlan;
use crate::{link_table, CellDiag, CellPlan, Library};
use neuron_nir::Nir;
use neuron_sema::{
    CellChannel, CellMember, CellModel, CellVerification, ChannelEnd, SemaOptions, SourceFile,
};
use serde::Serialize;
use std::collections::BTreeMap;

/// One step of a counterexample.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellStep {
    /// Tick number; 0 is the initial state.
    pub tick: u64,
    /// Time of the tick, in nanoseconds.
    pub time_ns: u64,
    /// State of every machine, as `machine.state`.
    pub states: Vec<String>,
    /// Value of every input at this tick.
    pub inputs: BTreeMap<String, bool>,
}

/// A shortest trace from the initial state to the violation of one property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellCounterexample {
    /// The violated property as written.
    pub property: String,
    /// The steps, tick 0 first; the property is violated at the last one.
    pub trace: Vec<CellStep>,
}

/// A property that holds and the size of the search that proved it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProvedProperty {
    /// The property as written.
    pub property: String,
    /// States of the product that were explored.
    pub states: usize,
}

/// A link seen by the checker as a channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelInfo {
    /// Source (`plc.cap_done`).
    pub from: String,
    /// Destination (`arm.pick`).
    pub to: String,
    /// Delay bound used, in nanoseconds.
    pub delay_ns: u64,
}

/// Which handshake the lock model uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Handshake {
    /// The protocol of D4.1: a party asks only after it saw its grant low, and returns
    /// to idle only after it saw it low again.
    FourPhase,
    /// A party returns to idle as soon as it leaves and may ask at once. Kept to show
    /// that the extra phase is needed: with a delay of two ticks or more the arbiter's
    /// withdrawn grant can still be visible when the party asks again.
    TwoPhase,
}

/// The result of checking the mutex of one zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ZoneProof {
    /// Zone name.
    pub zone: String,
    /// Handshake that was modeled.
    pub handshake: Handshake,
    /// Tick of the model, in nanoseconds.
    pub tick_ns: u64,
    /// Delay of each lock signal in ticks, per party device (request and grant share
    /// the medium of the party).
    pub delay_ns: BTreeMap<String, u64>,
    /// Pairs of parties for which "never both inside" was proved.
    pub proved: Vec<ProvedProperty>,
    /// Shortest counterexamples of the pairs that can be inside together.
    pub violations: Vec<CellCounterexample>,
    /// Properties the checker could not decide (state budget, too many inputs).
    pub undecided: Vec<String>,
}

impl ZoneProof {
    /// `true` when no pair can be inside together and nothing is undecided (a mutex with
    /// one party holds trivially).
    pub fn holds(&self) -> bool {
        self.violations.is_empty() && self.undecided.is_empty()
    }
}

/// The result of [`verify_cell`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellVerdict {
    /// Cell name.
    pub cell: String,
    /// The links that were composed as channels.
    pub channels: Vec<ChannelInfo>,
    /// `verify:` properties of the cell that hold.
    pub proved: Vec<ProvedProperty>,
    /// Shortest counterexamples of the violated ones.
    pub violations: Vec<CellCounterexample>,
    /// The mutex of every zone.
    pub zones: Vec<ZoneProof>,
    /// `NE0901` for each violated property or mutex.
    #[serde(skip)]
    pub diags: Vec<CellDiag>,
    /// Things that were not composed or not decided (`NL0901`), one per line.
    pub notes: Vec<String>,
}

fn to_plain(c: &neuron_sema::Counterexample) -> CellCounterexample {
    CellCounterexample {
        property: c.property.clone(),
        trace: c
            .trace
            .iter()
            .map(|s| CellStep {
                tick: s.tick,
                time_ns: s.time_ns,
                states: s.states.clone(),
                inputs: s.inputs.iter().cloned().collect(),
            })
            .collect(),
    }
}

fn proved_of(v: &CellVerification) -> Vec<ProvedProperty> {
    v.proved
        .iter()
        .map(|p| ProvedProperty {
            property: p.property.clone(),
            states: p.states,
        })
        .collect()
}

/// The `verify:` properties of the cell over the composed machines.
///
/// `sources` are the analyzed source files of the package, `zones` the locks of D4.1
/// (their mutexes are proved with [`prove_zone`] and reported in [`CellVerdict::zones`]).
/// The cell's own properties see the machines of the cell and of every placed module;
/// `NL0901` notes say what could not be decided or composed.
pub fn verify_cell(
    sources: &[SourceFile],
    nir: &Nir,
    plan: &CellPlan,
    lib: &Library,
    zones: &[ZonePlan],
) -> CellVerdict {
    let mut verdict = CellVerdict {
        cell: plan.cell.clone(),
        channels: Vec::new(),
        proved: Vec::new(),
        violations: Vec::new(),
        zones: Vec::new(),
        diags: Vec::new(),
        notes: Vec::new(),
    };
    // Channels from the links.
    let mut channels = Vec::new();
    match link_table(nir, plan, lib) {
        Ok(rows) => {
            for r in rows {
                let delay_ns = r.within_ns.unwrap_or(r.latency.total_ns);
                verdict.channels.push(ChannelInfo {
                    from: r.from.clone(),
                    to: r.to.clone(),
                    delay_ns,
                });
                channels.push(CellChannel {
                    from: ChannelEnd::Atom(r.from),
                    to: r.to,
                    delay_ns,
                });
            }
        }
        Err(ds) => verdict.notes.push(format!(
            "NL0901: the links were not composed ({}); the destinations of the links stay free inputs, which only adds behaviors",
            ds.iter().map(|d| d.code).collect::<Vec<_>>().join(", ")
        )),
    }
    let mut members: Vec<CellMember> = Vec::new();
    for d in &plan.devices {
        for p in &d.modules {
            members.push(CellMember {
                module: p.module.clone(),
                device: d.instance.clone(),
            });
        }
    }
    let periods: Vec<u64> = plan
        .devices
        .iter()
        .filter_map(|d| {
            lib.device(&d.manifest)
                .map(|m| endpoint_period(nir, plan, &d.instance, m))
        })
        .collect();
    let model = CellModel {
        owner: plan.cell.clone(),
        members,
        channels,
        tick_ns: periods.into_iter().min(),
    };
    match neuron_sema::verify_cell(sources.to_vec(), &model, &SemaOptions::default()) {
        Ok(v) => {
            verdict.proved = proved_of(&v);
            verdict.violations = v.counterexamples.iter().map(to_plain).collect();
            for d in &v.diagnostics {
                let span = Some([d.diag.span.start, d.diag.span.end]);
                let file = v.paths.get(d.file).cloned().unwrap_or_default();
                match d.diag.code {
                    "NE0901" => {
                        let mut c = CellDiag::new("NE0901", &file, span, d.diag.message.clone());
                        if let Some(h) = &d.diag.help {
                            c = c.help(h.clone());
                        }
                        verdict.diags.push(c);
                    }
                    "NL0901" => verdict
                        .notes
                        .push(format!("NL0901: {} ({file})", d.diag.message)),
                    _ => {}
                }
            }
        }
        Err(e) => verdict.notes.push(format!("NL0901: {e}")),
    }
    for z in zones {
        let tick = zone_tick(nir, plan, lib, z);
        let proof = prove_zone(z, tick);
        if !proof.holds() {
            for v in &proof.violations {
                verdict.diags.push(
                    CellDiag::new(
                        "NE0901",
                        &plan.file,
                        Some(z.span),
                        format!(
                            "the mutex of zone `{}` is violated: `{}` after {} tick(s)",
                            z.zone,
                            v.property,
                            v.trace.len().saturating_sub(1)
                        ),
                    )
                    .help(format!(
                        "counterexample: {}",
                        v.trace
                            .iter()
                            .map(|s| format!("t={} ns [{}]", s.time_ns, s.states.join(", ")))
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    )),
                );
            }
            for u in &proof.undecided {
                verdict
                    .notes
                    .push(format!("NL0901: mutex of zone `{}`: {u}", z.zone));
            }
        }
        verdict.zones.push(proof);
    }
    verdict
}

/// The tick of a zone model: the fastest endpoint period among the parties' devices.
fn zone_tick(nir: &Nir, plan: &CellPlan, lib: &Library, z: &ZonePlan) -> u64 {
    z.devices()
        .iter()
        .filter_map(|d| {
            let man = plan.device(d).and_then(|p| lib.device(&p.manifest))?;
            Some(endpoint_period(nir, plan, d, man))
        })
        .min()
        .unwrap_or(crate::DEFAULT_ENDPOINT_PERIOD_NS)
}

/// The Neuron source of the lock protocol model of `z`.
fn lock_model(z: &ZonePlan, handshake: Handshake) -> String {
    let devs = z.devices();
    let mut s = String::from("module lockmodel for mcu\n");
    for i in 0..devs.len() {
        s.push_str(&format!(
            "signal want_{i}: bool\nsignal req_seen_{i}: bool\nsignal grant_seen_{i}: bool\n"
        ));
    }
    for i in 0..devs.len() {
        s.push_str(&format!("fsm P{i} every 10 ms:\n  initial state Idle:\n"));
        match handshake {
            Handshake::FourPhase => s.push_str(&format!(
                "    -> Wait when want_{i} and not grant_seen_{i}\n  state Wait:\n    -> Inside when grant_seen_{i}\n  state Inside:\n    -> Leaving when not want_{i}\n  state Leaving:\n    -> Idle when not grant_seen_{i}\n"
            )),
            Handshake::TwoPhase => s.push_str(&format!(
                "    -> Wait when want_{i}\n  state Wait:\n    -> Inside when grant_seen_{i}\n  state Inside:\n    -> Idle when not want_{i}\n"
            )),
        }
    }
    s.push_str("fsm Owner every 10 ms:\n  initial state Free:\n");
    for i in 0..devs.len() {
        s.push_str(&format!("    -> G{i} when req_seen_{i}\n"));
    }
    for i in 0..devs.len() {
        s.push_str(&format!(
            "  state G{i}:\n    -> Free when not req_seen_{i}\n"
        ));
    }
    s.push_str("verify:\n");
    for i in 0..devs.len() {
        for j in i + 1..devs.len() {
            s.push_str(&format!("  never P{i}.Inside and P{j}.Inside\n"));
        }
    }
    s
}

/// Proves (or refutes) the mutex of `z` for the generated lock protocol, with the
/// delay of each lock signal taken from the plan ([`crate::LockLink::one_way_ns`]) and
/// a model tick of `tick_ns`.
///
/// ```
/// # use neuron_cell::*;
/// # let lib = Library::builtin().unwrap();
/// # let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/corpus/cell/x02_robot_zone.neuron")).unwrap();
/// # let a = neuron_sema::analyze(vec![neuron_sema::SourceFile { path: "x02.neuron".into(), text: src }], None, &neuron_sema::SemaOptions::default());
/// # let nir = a.nir.unwrap();
/// # let plan = partition(&nir, &lib).unwrap().remove(0);
/// let (zones, _) = zone_plans(&nir, &plan, &lib);
/// let proof = prove_zone(&zones[0], 10_000_000);
/// assert!(proof.holds());
/// ```
pub fn prove_zone(z: &ZonePlan, tick_ns: u64) -> ZoneProof {
    prove_zone_with(z, tick_ns, Handshake::FourPhase)
}

/// [`prove_zone`] for a chosen handshake; [`Handshake::TwoPhase`] exists to show the
/// counterexample that the four-phase protocol avoids.
pub fn prove_zone_with(z: &ZonePlan, tick_ns: u64, handshake: Handshake) -> ZoneProof {
    let devs = z.devices();
    let mut delay_ns = BTreeMap::new();
    let mut channels = Vec::new();
    for (i, d) in devs.iter().enumerate() {
        let delay = z
            .locks
            .iter()
            .find(|l| l.party == *d)
            .map_or(0, |l| l.one_way_ns);
        delay_ns.insert((*d).to_string(), delay);
        channels.push(CellChannel {
            from: ChannelEnd::InState(vec![format!("P{i}.Wait"), format!("P{i}.Inside")]),
            to: format!("req_seen_{i}"),
            delay_ns: delay,
        });
        channels.push(CellChannel {
            from: ChannelEnd::InState(vec![format!("Owner.G{i}")]),
            to: format!("grant_seen_{i}"),
            delay_ns: delay,
        });
    }
    let mut proof = ZoneProof {
        zone: z.zone.clone(),
        handshake,
        tick_ns,
        delay_ns,
        proved: Vec::new(),
        violations: Vec::new(),
        undecided: Vec::new(),
    };
    if devs.len() < 2 {
        // One party cannot be in the zone together with another: nothing to prove.
        return proof;
    }
    let model = CellModel {
        owner: "lockmodel".into(),
        members: Vec::new(),
        channels,
        tick_ns: Some(tick_ns),
    };
    let src = vec![SourceFile {
        path: "lockmodel.neuron".into(),
        text: lock_model(z, handshake),
    }];
    match neuron_sema::verify_cell(src, &model, &SemaOptions::default()) {
        Ok(v) => {
            proof.proved = proved_of(&v);
            proof.violations = v.counterexamples.iter().map(to_plain).collect();
            for d in &v.diagnostics {
                if d.diag.code == "NL0901" && d.diag.message.contains("could not be decided") {
                    proof.undecided.push(d.diag.message.clone());
                }
            }
        }
        Err(e) => proof.undecided.push(e),
    }
    proof
}
