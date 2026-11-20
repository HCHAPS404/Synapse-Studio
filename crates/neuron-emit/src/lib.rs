//! Emitters from NIR to the native languages of other devices (Phase C).
//!
//! An [`Emitter`] reads a NIR package and returns a [`Package`]: generated files, a wiring
//! report, a [`Fidelity`] report that lists every construct that was not translated
//! exactly (spec 11 §11.4), and source maps from each generated line back to the
//! `.neuron` span (§11.5). Emitters are pure and deterministic: the same NIR always gives
//! the same bytes. A construct the emitter cannot translate is `NE0610`; nothing is
//! silently dropped.
//!
//! | Name | Input | Output |
//! |---|---|---|
//! | `st` | `plc` modules | IEC 61131-3 Structured Text (`--profile generic` or `yaskawa`) |
//! | `scl` | `plc` modules | Siemens-style SCL function block, `Main` organization block, tag table |
//! | `plcopen` | `plc` modules | PLCopen XML (TC6 2.01): LD for rungs, SFC for sequences |
//! | `spel` | `robot` modules | SPEL+-style program and command-interpreter slave |
//! | `python` | `host` modules | Python 3 with an asyncio runtime in simulated or real time |
//! | `kernel` | `fsm` and `sequence` | state tables and the ST, C99 and SPEL+ kernels that run them |
//!
//! ```
//! let names: Vec<&str> = neuron_emit::names().to_vec();
//! assert!(names.contains(&"st"));
//! ```

#![forbid(unsafe_code)]

mod fidelity;
pub mod iec;
pub mod nirx;
mod package;
pub mod python;
pub mod robot;
pub mod spel;

pub use fidelity::{Class, Entry, Fidelity, Policy, Summary};
pub use package::{Mapping, Mark, Package, SourceMap, Src, Writer};

use neuron_nir::Nir;
use serde::Serialize;

/// A diagnostic of an emitter or importer (same shape as the hardware checks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    /// Code (`NE0610`, `NE0804`, `NE1003`).
    pub code: &'static str,
    /// Message.
    pub message: String,
    /// Suggested fix.
    pub help: Option<String>,
    /// Source file.
    pub file: String,
    /// Byte range in that file.
    pub span: [usize; 2],
}

/// Options of one emission.
#[derive(Debug, Clone, Default)]
pub struct EmitOptions {
    /// Dialect profile (`generic`, `yaskawa`); empty for the default.
    pub profile: String,
    /// Robot manifest name for `spel` (`generic_6axis` by default).
    pub robot: String,
}

/// What an emitter declares about itself (spec 11 §11.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Manifest {
    /// Emitter name (`st`).
    pub name: &'static str,
    /// Generated language.
    pub language: &'static str,
    /// Neuron dialects it reads.
    pub dialects: &'static [&'static str],
    /// Profiles it accepts.
    pub profiles: &'static [&'static str],
    /// Constructs it translates.
    pub constructs: &'static [&'static str],
}

/// An emitter plugin.
pub trait Emitter {
    /// Its manifest.
    fn manifest(&self) -> Manifest;
    /// Translates the modules of `nir` that it reads.
    ///
    /// # Errors
    /// Every construct it cannot translate, with its location.
    fn emit(&self, nir: &Nir, opts: &EmitOptions) -> Result<Package, Vec<Diag>>;
}

/// Names of the built-in emitters.
pub fn names() -> &'static [&'static str] {
    &["st", "scl", "plcopen", "spel", "python"]
}

/// A built-in emitter by name.
pub fn emitter(name: &str) -> Option<Box<dyn Emitter>> {
    Some(match name {
        "st" => Box::new(iec::St),
        "scl" => Box::new(iec::Scl),
        "plcopen" => Box::new(iec::PlcOpen),
        "spel" => Box::new(spel::Spel),
        "python" => Box::new(python::Python),
        _ => return None,
    })
}

/// Canonical JSON: two-space indentation, keys in declaration order, final newline.
pub fn to_json<T: Serialize>(v: &T) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_name_has_an_emitter_with_its_manifest() {
        for n in super::names() {
            let e = super::emitter(n);
            assert_eq!(e.map(|e| e.manifest().name), Some(*n));
        }
        assert!(super::emitter("cobol").is_none());
    }
}
