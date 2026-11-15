//! Neuron Intermediate Representation (NIR).
//!
//! NIR is the program after name resolution and type and unit checking, serialized
//! as canonical JSON (spec 11 §11.1, spec 16). This crate has no dependency on the
//! compiler so that editors, the digital twin and emitters can read and write NIR on
//! their own (including from WebAssembly).
//!
//! ```
//! use neuron_nir::{from_json, to_json, Nir};
//!
//! let nir = Nir { nir_version: "1.0".into(), edition: "2026".into(), package: None, modules: vec![] };
//! let text = to_json(&nir);
//! assert!(text.ends_with('\n'));
//! assert_eq!(from_json(&text).unwrap(), nir);
//! ```

mod model;
mod print;

pub use model::*;
pub use print::{
    expr as print_expr, factors_text, print_module, type_expr as print_type, unit_text,
};

/// Version of the NIR format written by this crate.
pub const NIR_VERSION: &str = "1.0";

/// Canonical JSON text of a NIR document: UTF-8, LF, two-space indentation, keys in
/// declaration (schema) order and a final newline. The same NIR always gives the same bytes.
pub fn to_json(nir: &Nir) -> String {
    let mut s = serde_json::to_string_pretty(nir).unwrap_or_default();
    s.push('\n');
    s
}

/// Reads a NIR document. Fails on malformed JSON, unknown variants or a major version
/// other than 1.
pub fn from_json(text: &str) -> Result<Nir, String> {
    let nir: Nir = serde_json::from_str(text).map_err(|e| e.to_string())?;
    match nir.nir_version.split('.').next() {
        Some("1") => Ok(nir),
        _ => Err(format!("unsupported nir_version `{}`", nir.nir_version)),
    }
}

/// The JSON Schema of the NIR format, as canonical JSON text.
///
/// The committed copy is `spec/schemas/nir.schema.json`; a test fails when they differ.
pub fn schema_json() -> String {
    let schema = schemars::schema_for!(Nir);
    let mut s = serde_json::to_string_pretty(&schema).unwrap_or_default();
    s.push('\n');
    s
}

/// Prints every module of a NIR document as `(file, source text)` pairs.
pub fn print(nir: &Nir) -> Vec<(String, String)> {
    nir.modules
        .iter()
        .map(|m| (m.file.clone(), print_module(m)))
        .collect()
}
