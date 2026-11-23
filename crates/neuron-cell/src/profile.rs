//! Calibration timing profiles (task E6.3, requirement E-R17).
//!
//! A profile is a small TOML file in the manifest `[timing]` shape, one per device and medium,
//! produced by the daemon's calibration pipeline (`synapse_daemon.calibrate`) from measured
//! round trips. `neuron cell check --profile file.toml` reads one or more profiles into a
//! [`TimingOverlay`] and overrides the manifest timing of the named device on the named medium
//! when the link budget is computed ([`crate::link_table_with`]), labelling that link's trust
//! `measured`.
//!
//! The numbers in the repo are `generic` placeholders; a profile with `trust = "measured"`
//! carries real measurements and the samples that justify them. A profile with fewer than
//! [`MIN_MEASURED_SAMPLES`] samples, a non-positive value, or an unknown unit is `NE1112`.
//!
//! ```
//! let text = r#"
//! [profile]
//! device = "generic_s7_1200"
//! medium = "modbus_tcp"
//! trust = "measured"
//! samples = 2000
//! transport = "2 ms"
//! jitter = "1 ms"
//! "#;
//! let p = neuron_cell::TimingProfile::parse("p.toml", text).unwrap();
//! assert_eq!(p.device, "generic_s7_1200");
//! assert_eq!(p.transport_ns, 2_000_000);
//! ```
use crate::units::duration_ns;
use crate::{CellDiag, MediumTiming, Trust};
use serde::Deserialize;
use std::collections::BTreeMap;

/// Fewest samples a `measured` profile may be built from (a tiny sample is not a measurement).
pub const MIN_MEASURED_SAMPLES: u64 = 30;

/// The error code of an invalid calibration profile.
pub const PROFILE_INVALID: &str = "NE1112";

/// Trust of a profile's numbers: `generic` placeholders or `measured` on a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileTrust {
    /// Placeholder numbers, as in the repo until session I1.
    Generic,
    /// Measured on a lab device.
    Measured,
    /// Measured and confirmed against the vendor documentation.
    Confirmed,
}

impl From<ProfileTrust> for Trust {
    fn from(t: ProfileTrust) -> Trust {
        match t {
            ProfileTrust::Generic => Trust::Generic,
            ProfileTrust::Measured => Trust::Measured,
            ProfileTrust::Confirmed => Trust::Confirmed,
        }
    }
}

/// A validated calibration profile: the timing of one device on one medium.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimingProfile {
    /// Device manifest name the profile measures (`generic_s7_1200`).
    pub device: String,
    /// Medium name the profile measures (`modbus_tcp`).
    pub medium: String,
    /// Trust of the numbers.
    pub trust: Trust,
    /// Samples the measurement is built from.
    pub samples: u64,
    /// Worst-case transport time in nanoseconds.
    pub transport_ns: u64,
    /// Jitter in nanoseconds.
    pub jitter_ns: u64,
    /// When it was measured (free text, e.g. `2026-11-24`).
    pub measured_at: String,
    /// Tool that produced it (free text).
    pub tool: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    device: String,
    medium: String,
    trust: ProfileTrust,
    #[serde(default)]
    samples: u64,
    transport: String,
    jitter: String,
    #[serde(default)]
    measured_at: String,
    #[serde(default)]
    tool: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    profile: RawProfile,
}

impl TimingProfile {
    /// The `MediumTiming` this profile supplies.
    pub fn timing(&self) -> MediumTiming {
        MediumTiming {
            transport_ns: self.transport_ns,
            jitter_ns: self.jitter_ns,
        }
    }

    /// Reads and validates a profile TOML. `file` only labels the error.
    ///
    /// # Errors
    /// [`CellDiag`] with [`PROFILE_INVALID`] (`NE1112`) for malformed TOML, an unknown field,
    /// an empty `device`/`medium`, a non-positive transport/jitter, an unknown or wrong unit, a
    /// `measured`/`confirmed` profile with fewer than [`MIN_MEASURED_SAMPLES`] samples, or more
    /// samples claimed than are present.
    pub fn parse(file: &str, text: &str) -> Result<TimingProfile, CellDiag> {
        let bad = |m: String| CellDiag::new(PROFILE_INVALID, file, None, m);
        let raw: RawFile =
            toml::from_str(text).map_err(|e| bad(format!("invalid profile: {}", e.message())))?;
        let p = raw.profile;
        if p.device.trim().is_empty() {
            return Err(bad("`device` is empty".into()));
        }
        if p.medium.trim().is_empty() {
            return Err(bad("`medium` is empty".into()));
        }
        let trust: Trust = p.trust.into();
        let dur = |what: &str, t: &str| -> Result<u64, CellDiag> {
            let ns = duration_ns(t).map_err(|e| bad(format!("{what}: {e}")))?;
            if ns == 0 {
                return Err(bad(format!("{what} must be positive, got `{t}`")));
            }
            Ok(ns)
        };
        let transport_ns = dur("transport", &p.transport)?;
        let jitter_ns = dur("jitter", &p.jitter)?;
        if trust != Trust::Generic && p.samples < MIN_MEASURED_SAMPLES {
            return Err(bad(format!(
                "a `{}` profile needs at least {MIN_MEASURED_SAMPLES} samples, got {}",
                format!("{trust:?}").to_lowercase(),
                p.samples
            )));
        }
        Ok(TimingProfile {
            device: p.device,
            medium: p.medium,
            trust,
            samples: p.samples,
            transport_ns,
            jitter_ns,
            measured_at: p.measured_at,
            tool: p.tool,
        })
    }
}

/// An overlay of measured timing, keyed by `(device manifest name, medium name)`.
///
/// `neuron cell check --profile` builds one from profile files; the link budget looks a
/// `(device, medium)` pair up here first and uses the manifest value only when the overlay has
/// none, so behaviour is unchanged when no profile is given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimingOverlay {
    entries: BTreeMap<(String, String), TimingProfile>,
}

impl TimingOverlay {
    /// An empty overlay: the budget uses manifest timing only (the default).
    pub fn new() -> TimingOverlay {
        TimingOverlay::default()
    }

    /// `true` when no profile was loaded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add a profile. A later profile for the same `(device, medium)` replaces an earlier one.
    pub fn insert(&mut self, profile: TimingProfile) {
        self.entries
            .insert((profile.device.clone(), profile.medium.clone()), profile);
    }

    /// The profile for a device manifest on a medium, if the overlay has one.
    pub fn get(&self, device: &str, medium: &str) -> Option<&TimingProfile> {
        self.entries.get(&(device.to_string(), medium.to_string()))
    }

    /// Read and merge a profile file into the overlay.
    ///
    /// # Errors
    /// `NE1112` when the file is not a valid profile ([`TimingProfile::parse`]).
    pub fn load(&mut self, file: &str, text: &str) -> Result<(), CellDiag> {
        self.insert(TimingProfile::parse(file, text)?);
        Ok(())
    }
}
