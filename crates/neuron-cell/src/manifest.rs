//! The schema of device and medium manifests (`devices/<class>/*.toml`).
//!
//! Manifests use generic names and round placeholder numbers; `trust` says how far the
//! numbers can be believed (`generic` until session I1 measures them).
use crate::units::{duration_ns, rate_bd};
use crate::ManifestError;
use neuron_emit::robot::Robot;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Device class; each has its own folder under `devices/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Programmable logic controller (`devices/plcs`).
    Plc,
    /// Microcontroller board (`devices/controllers`).
    Mcu,
    /// FPGA board (`devices/fpgas`).
    Fpga,
    /// Robot arm (`devices/robots`).
    Robot,
    /// Computer running Python (`devices/hosts`).
    Host,
}

impl Kind {
    /// Folder under `devices/` that holds this class.
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Plc => "plcs",
            Kind::Mcu => "controllers",
            Kind::Fpga => "fpgas",
            Kind::Robot => "robots",
            Kind::Host => "hosts",
        }
    }

    /// The class of a folder name.
    pub fn from_folder(folder: &str) -> Option<Kind> {
        [Kind::Plc, Kind::Mcu, Kind::Fpga, Kind::Robot, Kind::Host]
            .into_iter()
            .find(|k| k.folder() == folder)
    }
}

/// The emitter or backend that builds the program of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// IEC 61131-3 Structured Text.
    St,
    /// SCL function block.
    Scl,
    /// PLCopen XML.
    Plcopen,
    /// SPEL+-style robot program.
    Spel,
    /// Python 3.
    Python,
    /// Portable C99 through the C backend.
    C,
    /// Native machine code.
    Native,
    /// Verilog.
    Verilog,
}

impl Backend {
    /// Name as written in manifests.
    pub fn name(self) -> &'static str {
        match self {
            Backend::St => "st",
            Backend::Scl => "scl",
            Backend::Plcopen => "plcopen",
            Backend::Spel => "spel",
            Backend::Python => "python",
            Backend::C => "c",
            Backend::Native => "native",
            Backend::Verilog => "verilog",
        }
    }

    fn fits(self, kind: Kind) -> bool {
        match kind {
            Kind::Plc => matches!(self, Backend::St | Backend::Scl | Backend::Plcopen),
            Kind::Mcu => matches!(self, Backend::C | Backend::Native),
            Kind::Fpga => self == Backend::Verilog,
            Kind::Robot => self == Backend::Spel,
            Kind::Host => matches!(self, Backend::Python | Backend::C),
        }
    }
}

/// How far the numbers of a manifest can be believed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trust {
    /// Round placeholder numbers, not measured.
    Generic,
    /// Measured on a lab device.
    Measured,
    /// Measured and confirmed against the vendor documentation.
    Confirmed,
}

/// Dialects a device can run.
pub const DIALECTS: &[&str] = &["core", "mcu", "plc", "robot", "hdl", "host", "ui"];

/// One address family of a device (`%I`, `%Q`, pins).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IoFamily {
    /// Address prefix (`%I`).
    pub prefix: String,
    /// Voltage domain in volts; absent for memory areas.
    #[serde(default)]
    pub volts: Option<f64>,
    /// `sink` or `source` for inputs.
    #[serde(default)]
    pub kind: Option<String>,
    /// Output current limit in mA.
    #[serde(default)]
    pub max_ma: Option<u32>,
}

/// Timing of a device on one medium, in nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediumTiming {
    /// Worst-case transport time.
    pub transport_ns: u64,
    /// Jitter.
    pub jitter_ns: u64,
}

/// A validated device manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// Name (the file stem).
    pub name: String,
    /// Class.
    pub kind: Kind,
    /// Dialects it runs.
    pub dialects: Vec<String>,
    /// Backend that builds for it.
    pub backend: Backend,
    /// Emitter profile.
    pub profile: String,
    /// Trust level of the numbers.
    pub trust: Trust,
    /// Vendor class (a generic family name such as `s7`), used to tell PLC types apart.
    pub vendor_class: String,
    /// Free text; required when `trust` is `generic`.
    pub note: String,
    /// Board of `targets/boards` (controllers only).
    pub target: Option<String>,
    /// FPGA board of `targets/fpga` (FPGAs only).
    pub board: Option<String>,
    /// Address families by name.
    pub io: BTreeMap<String, IoFamily>,
    /// Shortest scan or task period in nanoseconds.
    pub scan_min_ns: Option<u64>,
    /// Timing per medium; the keys are the media the device supports.
    pub timing: BTreeMap<String, MediumTiming>,
    /// Time to reach the safe state after a fault, in nanoseconds.
    pub safe_state_ns: u64,
    /// Limits of a robot (robots only).
    pub robot: Option<Robot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDevice {
    name: String,
    kind: Kind,
    dialects: Vec<String>,
    backend: Backend,
    #[serde(default = "generic")]
    profile: String,
    trust: Trust,
    #[serde(default)]
    vendor_class: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    board: Option<String>,
}

fn generic() -> String {
    "generic".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMediumTiming {
    transport: String,
    jitter: String,
}

#[derive(Deserialize)]
struct RawTiming {
    #[serde(default)]
    scan_min: Option<String>,
    #[serde(flatten)]
    media: BTreeMap<String, RawMediumTiming>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSafety {
    safe_state_time: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    device: RawDevice,
    #[serde(default)]
    io: BTreeMap<String, IoFamily>,
    timing: RawTiming,
    safety: RawSafety,
    #[serde(default)]
    robot: Option<Robot>,
}

pub(crate) fn invalid(file: &str, message: impl Into<String>) -> ManifestError {
    ManifestError::Invalid {
        file: file.into(),
        message: message.into(),
    }
}

impl Manifest {
    /// Reads and validates a device manifest. `file` only labels the errors.
    ///
    /// # Errors
    /// `NE0607` for malformed TOML, unknown fields, a backend that does not fit the class,
    /// an unknown dialect or unit, a missing `note` on `generic` numbers, or a missing
    /// `target`, `board` or `[robot]` table.
    ///
    /// ```
    /// let text = r#"
    /// [device]
    /// name = "demo_host"
    /// kind = "host"
    /// dialects = ["host", "core"]
    /// backend = "python"
    /// trust = "generic"
    /// note = "placeholder numbers"
    /// [timing]
    /// scan_min = "1 ms"
    /// [safety]
    /// safe_state_time = "10 ms"
    /// "#;
    /// let m = neuron_cell::Manifest::parse("demo_host.toml", text).unwrap();
    /// assert_eq!(m.safe_state_ns, 10_000_000);
    /// ```
    pub fn parse(file: &str, text: &str) -> Result<Manifest, ManifestError> {
        let raw: RawFile = toml::from_str(text).map_err(|e| ManifestError::Parse {
            file: file.into(),
            message: e.message().to_string(),
        })?;
        let d = raw.device;
        let bad = |m: String| invalid(file, m);
        if d.name.is_empty() {
            return Err(bad("`name` is empty".into()));
        }
        if d.dialects.is_empty() {
            return Err(bad("`dialects` is empty".into()));
        }
        if let Some(x) = d.dialects.iter().find(|x| !DIALECTS.contains(&x.as_str())) {
            return Err(bad(format!(
                "unknown dialect `{x}` (known: {})",
                DIALECTS.join(", ")
            )));
        }
        if !d.backend.fits(d.kind) {
            return Err(bad(format!(
                "backend `{}` cannot build for a {:?} device",
                d.backend.name(),
                d.kind
            )));
        }
        if d.trust == Trust::Generic && d.note.trim().is_empty() {
            return Err(bad(
                "`note` is required when trust is `generic` (say the numbers are placeholders)"
                    .into(),
            ));
        }
        let want = |present: bool, needed: bool, field: &str| -> Result<(), ManifestError> {
            match (present, needed) {
                (false, true) => Err(bad(format!("`{field}` is required for this class"))),
                (true, false) => Err(bad(format!("`{field}` is not allowed for this class"))),
                _ => Ok(()),
            }
        };
        want(d.target.is_some(), d.kind == Kind::Mcu, "device.target")?;
        want(d.board.is_some(), d.kind == Kind::Fpga, "device.board")?;
        want(raw.robot.is_some(), d.kind == Kind::Robot, "[robot]")?;
        if let Some(r) = &raw.robot {
            if r.name != d.name {
                return Err(bad(format!(
                    "[robot] name `{}` differs from device name `{}`",
                    r.name, d.name
                )));
            }
        }
        for (k, f) in &raw.io {
            if f.prefix.is_empty() {
                return Err(bad(format!("io `{k}`: empty prefix")));
            }
            if let Some(v) = f.volts {
                if !(v.is_finite() && v > 0.0 && v <= 1000.0) {
                    return Err(bad(format!("io `{k}`: volts {v} is out of range")));
                }
            }
        }
        let unit = |what: &str, t: &str, f: fn(&str) -> Result<u64, String>| {
            f(t).map_err(|m| bad(format!("{what}: {m}")))
        };
        let scan_min_ns = match &raw.timing.scan_min {
            Some(t) => Some(unit("timing.scan_min", t, duration_ns)?),
            None => None,
        };
        let mut timing = BTreeMap::new();
        for (m, t) in &raw.timing.media {
            timing.insert(
                m.clone(),
                MediumTiming {
                    transport_ns: unit(
                        &format!("timing.{m}.transport"),
                        &t.transport,
                        duration_ns,
                    )?,
                    jitter_ns: unit(&format!("timing.{m}.jitter"), &t.jitter, duration_ns)?,
                },
            );
        }
        let safe_state_ns = unit(
            "safety.safe_state_time",
            &raw.safety.safe_state_time,
            duration_ns,
        )?;
        Ok(Manifest {
            name: d.name,
            kind: d.kind,
            dialects: d.dialects,
            backend: d.backend,
            profile: d.profile,
            trust: d.trust,
            vendor_class: d.vendor_class,
            note: d.note,
            target: d.target,
            board: d.board,
            io: raw.io,
            scan_min_ns,
            timing,
            safe_state_ns,
            robot: raw.robot,
        })
    }
}

/// How a medium carries values (design D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Register or data block read on a scan.
    Register,
    /// Framed bytes with sequence number and CRC.
    Frame,
    /// Publish and subscribe.
    Pubsub,
    /// A wire between two pins.
    ZeroHop,
}

/// A validated medium manifest (`devices/media/*.toml`).
#[derive(Debug, Clone, PartialEq)]
pub struct Medium {
    /// Name as written after `via`.
    pub name: String,
    /// Carrying pattern.
    pub family: Family,
    /// Bit rate in baud, when the medium has one.
    pub bitrate_bd: Option<u64>,
    /// Bytes added to every frame.
    pub frame_overhead_bytes: u32,
    /// Worst-case transport time in nanoseconds.
    pub transport_worst_ns: u64,
    /// The medium isolates the two ends electrically.
    pub isolated: bool,
    /// A change can be pushed without polling.
    pub event_driven: bool,
    /// Device classes that can speak it.
    pub speaks: Vec<Kind>,
    /// Names accepted as arguments (`zero_hop(gpio, opto_24v)`).
    pub options: Vec<String>,
    /// Options that isolate or shift the level (a subset of `options`).
    pub isolating: Vec<String>,
    /// Trust level of the numbers.
    pub trust: Trust,
    /// Free text; required when `trust` is `generic`.
    pub note: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMedium {
    name: String,
    family: Family,
    #[serde(default)]
    bitrate: Option<String>,
    #[serde(default)]
    frame_overhead_bytes: u32,
    transport_worst: String,
    #[serde(default)]
    isolated: bool,
    #[serde(default)]
    event_driven: bool,
    speaks: Vec<Kind>,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    isolating: Vec<String>,
    trust: Trust,
    #[serde(default)]
    note: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMediumFile {
    medium: RawMedium,
}

impl Medium {
    /// Reads and validates a medium manifest. `file` only labels the errors.
    ///
    /// # Errors
    /// `NE0607` for malformed TOML, unknown fields, bad units, an empty `speaks`,
    /// `isolating` names that are not options, or a missing `note` on `generic` numbers.
    ///
    /// ```
    /// let text = r#"
    /// [medium]
    /// name = "wire"
    /// family = "zero_hop"
    /// transport_worst = "10 us"
    /// speaks = ["mcu"]
    /// trust = "generic"
    /// note = "placeholder"
    /// "#;
    /// assert_eq!(neuron_cell::Medium::parse("wire.toml", text).unwrap().transport_worst_ns, 10_000);
    /// ```
    pub fn parse(file: &str, text: &str) -> Result<Medium, ManifestError> {
        let m = toml::from_str::<RawMediumFile>(text)
            .map_err(|e| ManifestError::Parse {
                file: file.into(),
                message: e.message().to_string(),
            })?
            .medium;
        let bad = |s: String| invalid(file, s);
        if m.name.is_empty() {
            return Err(bad("`name` is empty".into()));
        }
        if m.speaks.is_empty() {
            return Err(bad("`speaks` is empty".into()));
        }
        if let Some(x) = m.isolating.iter().find(|x| !m.options.contains(x)) {
            return Err(bad(format!("isolating `{x}` is not listed in `options`")));
        }
        if m.trust == Trust::Generic && m.note.trim().is_empty() {
            return Err(bad("`note` is required when trust is `generic`".into()));
        }
        let bitrate_bd = match &m.bitrate {
            Some(t) => Some(rate_bd(t).map_err(|e| bad(format!("bitrate: {e}")))?),
            None => None,
        };
        let transport_worst_ns =
            duration_ns(&m.transport_worst).map_err(|e| bad(format!("transport_worst: {e}")))?;
        Ok(Medium {
            name: m.name,
            family: m.family,
            bitrate_bd,
            frame_overhead_bytes: m.frame_overhead_bytes,
            transport_worst_ns,
            isolated: m.isolated,
            event_driven: m.event_driven,
            speaks: m.speaks,
            options: m.options,
            isolating: m.isolating,
            trust: m.trust,
            note: m.note,
        })
    }
}
