//! The cell compiler (Phase D): device manifests (D1), the partitioner (D2), and the
//! later stages that synthesize links, check the cell and simulate it.
//!
//! Manifests live in `devices/<class>/*.toml` (`plcs`, `controllers`, `fpgas`, `robots`,
//! `hosts`) and `devices/media/*.toml`. Numbers are generic placeholders until the lab
//! session I1 measures them; the `trust` field says so and every report repeats it.
//!
//! ```
//! let lib = neuron_cell::Library::builtin().unwrap();
//! let scara = lib.device("generic_scara").unwrap();
//! assert_eq!(scara.kind, neuron_cell::Kind::Robot);
//! ```
#![forbid(unsafe_code)]
mod build;
mod cosim;
mod diag;
mod dossier;
mod electrical;
mod endpoints;
mod library;
mod links;
mod manifest;
mod partition;
mod replay;
mod tags;
mod units;
mod verify;
mod watchdog;

pub mod wire;
mod zones;
pub use build::{build, device_package, device_package_with, CellBuild, DeviceBuild};
pub use cosim::{
    behaviour_by_device, cosimulate, cosimulate_single, is_plumbing, CellEvent, CellEventKind, Cut,
    Delivery, Drive, DriveValue, LinkModel, LossReport, SimDevice, SimError, SimLink, SimOptions,
    SimRun,
};
pub use diag::CellDiag;
pub use dossier::{dossier, BuildStatus, Dossier, DossierOptions, SAFETY_WORDING};
pub use electrical::check_electrical;
pub use endpoints::{
    endpoint_sources, link_plans, register_csv, register_map, Crossing, Endpoint, EndpointSource,
    LinkPlan, Location, Pattern, RegisterRow, ACTION_TIMEOUT_NS, DEFAULT_ENDPOINT_PERIOD_NS,
    REGISTER_BASE,
};
pub use library::{CellManifests, Library, ManifestError, ResolvedDevice, ResolvedMedium};
pub use links::{
    fmt_ns, latency, latency_with, link_table, Alternative, Latency, LinkReport, Shape,
};
pub use manifest::{
    Backend, Family, IoFamily, Kind, Manifest, Medium, MediumTiming, Trust, DIALECTS,
};
pub use partition::{partition, CellPlan, DevicePlan, How, Placed};
pub use replay::{
    replay_schema_json, Replay, ReplayDevice, ReplayEvent, ReplayEventKind, ReplayKind, ReplayLink,
    CELL_DEVICE, REPLAY_VERSION,
};
pub use tags::{address_fits, check_tags, parse_direct_address, type_bits, DirectAddress};
pub use units::{duration_ns, rate_bd};
pub use verify::{
    prove_zone, prove_zone_with, verify_cell, CellCounterexample, CellStep, CellVerdict,
    ChannelInfo, Handshake, ProvedProperty, ZoneProof,
};
pub use watchdog::{
    watchdog_plans, DeviceWatchdog, WatchdogPlan, DEFAULT_WATCHDOG_NS, HEARTBEAT_NS,
    MISSED_HEARTBEATS,
};
pub use zones::{
    arbitrate, lock_signal, zone_plans, zone_sources, EntryKind, LockLink, Party, ZoneBox,
    ZoneEntry, ZonePlan,
};
