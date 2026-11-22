//! The cell watchdog (task D3.3, requirement D-R5, property MOC-C2).
//!
//! Every device that receives a link gets a watchdog in the generated receiver endpoint
//! module of that link (see `endpoints`): the module declares `l<i>_alive`, which the
//! transport adapter sets on every value or heartbeat that arrives from the sender, and
//! raises the built-in fault `LinkLost` when `alive` has been false for the cell
//! watchdog period. A fault applies the `safe_state:` blocks of the whole device package
//! (spec 08 §8.2), so the safe state of every actuator of the device is reached by the
//! device's own program, not by the link.
//!
//! # Period
//!
//! Spec 11.3: each side sends a heartbeat every [`HEARTBEAT_NS`] (10 ms) and a peer is
//! lost after [`MISSED_HEARTBEATS`] (5) missed ones, so the cell watchdog is
//! [`DEFAULT_WATCHDOG_NS`] = 50 ms.
//!
//! # The static MOC-C2 bound
//!
//! After a link loss no actuator of the consuming device stays energized longer than
//! `watchdog + safe_state_time` ([`DeviceWatchdog::bound_ns`]), where `safe_state_time` is
//! the `[safety]` value of the device manifest. The bound is computed from the plan alone;
//! the co-simulation (task D6.1) cuts each link and checks that the observed time stays
//! inside it. Dependent actuators are listed per device: the link-to-output dataflow is
//! not known (a link is not yet connected to the signals of user modules), so the list is
//! every output of every module placed on the device, which is what a fault drives to
//! its safe value.
use crate::{CellPlan, LinkPlan};
use neuron_nir::{ExprKind, ItemDef, ModuleKind, Nir};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Heartbeat period of Neuron Wire (spec 11.3), in nanoseconds.
pub const HEARTBEAT_NS: u64 = 10_000_000;
/// Heartbeats that may be missed before a peer is lost.
pub const MISSED_HEARTBEATS: u64 = 5;
/// The default cell watchdog: [`MISSED_HEARTBEATS`] heartbeats, 50 ms.
pub const DEFAULT_WATCHDOG_NS: u64 = HEARTBEAT_NS * MISSED_HEARTBEATS;

/// The watchdog data of one link, for its receiving device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WatchdogPlan {
    /// Cell watchdog period, in nanoseconds.
    pub period_ns: u64,
    /// Safe-state time of the receiving device (manifest `[safety] safe_state_time`).
    pub safe_state_ns: u64,
    /// MOC-C2 bound: `period_ns + safe_state_ns`.
    pub loss_bound_ns: u64,
    /// Fault raised when the watchdog expires.
    pub fault: String,
}

impl WatchdogPlan {
    /// The plan for a receiver whose safe-state time is `safe_state_ns`.
    ///
    /// ```
    /// let w = neuron_cell::WatchdogPlan::new(10_000_000);
    /// assert_eq!(w.loss_bound_ns, 60_000_000);
    /// ```
    pub fn new(safe_state_ns: u64) -> WatchdogPlan {
        WatchdogPlan {
            period_ns: DEFAULT_WATCHDOG_NS,
            safe_state_ns,
            loss_bound_ns: DEFAULT_WATCHDOG_NS.saturating_add(safe_state_ns),
            fault: "LinkLost".to_string(),
        }
    }
}

/// What the cell watchdog guarantees for one device that consumes links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceWatchdog {
    /// Device instance.
    pub device: String,
    /// Indices of the links it consumes, ascending.
    pub links: Vec<usize>,
    /// Cell watchdog period, in nanoseconds.
    pub watchdog_ns: u64,
    /// Safe-state time from the manifest, in nanoseconds.
    pub safe_state_ns: u64,
    /// MOC-C2 bound: no dependent actuator stays energized longer than this after a loss.
    pub bound_ns: u64,
    /// Outputs of the modules placed on the device (they take their safe value).
    pub actuators: Vec<String>,
}

/// Outputs of the modules placed on `device`: PLC `outputs`, and `io` bindings that
/// drive something (`gpio.output`, `pwm`, `dac`, motion).
pub(crate) fn actuators_of(nir: &Nir, plan: &CellPlan, device: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    let Some(dev) = plan.device(device) else {
        return Vec::new();
    };
    for placed in &dev.modules {
        let Some(m) = nir
            .modules
            .iter()
            .find(|m| m.kind == ModuleKind::Module && m.name == placed.module)
        else {
            continue;
        };
        for item in &m.items {
            match &item.def {
                ItemDef::PlcSection { section, vars } if section == "outputs" => {
                    out.extend(vars.iter().map(|v| v.name.clone()));
                }
                ItemDef::Io { name, kind, .. } if drives(&kind.e) => {
                    out.insert(name.clone());
                }
                _ => {}
            }
        }
    }
    out.into_iter().collect()
}

/// Whether an `io` kind expression (`gpio.output`, `pwm(freq: 1 kHz)`) drives a pin.
fn drives(kind: &ExprKind) -> bool {
    let text = serde_json::to_string(kind).unwrap_or_default();
    ["output", "pwm", "dac", "\"motion\""]
        .iter()
        .any(|k| text.contains(k))
}

/// The watchdog of every device that consumes a link, ordered by device instance.
///
/// The bound of a device is `watchdog + safe_state_time`; every link of the device has
/// the same watchdog, so the bound does not depend on the link.
pub fn watchdog_plans(nir: &Nir, plan: &CellPlan, links: &[LinkPlan]) -> Vec<DeviceWatchdog> {
    let mut by_device: BTreeMap<String, (Vec<usize>, u64)> = BTreeMap::new();
    for l in links {
        let e = by_device
            .entry(l.rx.device.clone())
            .or_insert_with(|| (Vec::new(), l.watchdog.safe_state_ns));
        e.0.push(l.index);
    }
    by_device
        .into_iter()
        .map(|(device, (mut idx, safe))| {
            idx.sort_unstable();
            let w = WatchdogPlan::new(safe);
            DeviceWatchdog {
                actuators: actuators_of(nir, plan, &device),
                device,
                links: idx,
                watchdog_ns: w.period_ns,
                safe_state_ns: w.safe_state_ns,
                bound_ns: w.loss_bound_ns,
            }
        })
        .collect()
}

/// Renders the watchdog of the receiver side of link `n` for the dialects that have
/// `safety:` rules, as source lines to append to the endpoint module.
pub(crate) fn rule_source(n: usize, period_ns: u64) -> String {
    let d = crate::endpoints::neuron_duration(period_ns);
    format!(
        "when l{n}_alive == false for {d}:\n  fault LinkLost\nsafety:\n  safe_state:\n    pass\n"
    )
}
