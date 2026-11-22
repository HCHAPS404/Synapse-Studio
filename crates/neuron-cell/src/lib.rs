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
mod diag;
mod library;
mod links;
mod manifest;
mod partition;
mod units;

pub use build::{build, device_package, CellBuild, DeviceBuild};
pub use diag::CellDiag;
pub use library::{CellManifests, Library, ManifestError, ResolvedDevice, ResolvedMedium};
pub use links::{fmt_ns, latency, link_table, Alternative, Latency, LinkReport};
pub use manifest::{
    Backend, Family, IoFamily, Kind, Manifest, Medium, MediumTiming, Trust, DIALECTS,
};
pub use partition::{partition, CellPlan, DevicePlan, How, Placed};
pub use units::{duration_ns, rate_bd};
