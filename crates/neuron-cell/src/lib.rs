//! The cell compiler (Phase D). This part (task D1) loads and validates the manifests of
//! the devices and link media that a `cell` file names.
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
mod library;
mod manifest;
mod units;

pub use library::{CellManifests, Library, ManifestError, ResolvedDevice, ResolvedMedium};
pub use manifest::{
    Backend, Family, IoFamily, Kind, Manifest, Medium, MediumTiming, Trust, DIALECTS,
};
pub use units::{duration_ns, rate_bd};
