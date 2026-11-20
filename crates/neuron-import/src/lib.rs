//! Importers: source text of another language to Neuron source (Phase C, tasks C5.1, C5.2).
//!
//! | Language | Input | Output |
//! |---|---|---|
//! | `st` | IEC 61131-3 Structured Text (the files `neuron emit st` writes, or hand-written) | a `plc` module |
//! | `scl` | the SCL a `neuron emit scl` run writes (function block and organization block) | a `plc` module |
//! | `plcopen` | PLCopen XML TC6 2.01 with LD, SFC or ST bodies | a `plc` module |
//! | `spel` | the SPEL+-style subset of `neuron-spel` | a `robot` module |
//!
//! Importers write Neuron *source*, not NIR: the result is formatted like hand-written
//! code and goes through the normal analyzer. Each import returns the source, a fidelity
//! report (spec 11 §11.4) and diagnostics: `NE1001` when the input cannot be parsed (the
//! import fails) and `NE1002` for every construct that has no Neuron equivalent (the
//! import goes on without it, and the report lists it as `unsupported`).
//!
//! ```
//! let st = "PROGRAM latch\nVAR go AT %IX0.0 : BOOL; lamp AT %QX0.0 : BOOL; END_VAR\n\
//!           IF go THEN lamp := TRUE; END_IF;\nEND_PROGRAM\n";
//! let out = neuron_import::import("st", "latch", st).unwrap();
//! assert!(out.source.contains("set lamp when go"));
//! assert!(out.diags.is_empty());
//! ```

#![forbid(unsafe_code)]

mod plcopen;
mod spel;
mod st;
mod util;

pub use neuron_emit::{Class, Diag, Fidelity};

/// Options of an import.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The scan period in nanoseconds, for sources that do not say it (a `TASK` interval
    /// in the source wins). `None` assumes 10 ms and records it in the fidelity report.
    pub scan_ns: Option<i64>,
    /// Name of the input file, used in diagnostics.
    pub file: String,
}

/// The result of an import.
#[derive(Debug, Clone)]
pub struct Imported {
    /// Neuron source, formatted.
    pub source: String,
    /// What was preserved exactly and what was not.
    pub fidelity: Fidelity,
    /// Warnings: `NE1002` for each construct that was not imported.
    pub diags: Vec<Diag>,
}

impl Imported {
    /// `true` when every construct of the input was imported.
    pub fn complete(&self) -> bool {
        self.diags.is_empty()
    }
}

/// The languages `import` reads.
pub fn languages() -> &'static [&'static str] {
    &["st", "scl", "plcopen", "spel"]
}

/// Imports `text` written in `lang` as a module called `name`.
///
/// # Errors
/// `NE1001` when the text cannot be parsed, or when the language is unknown.
pub fn import(lang: &str, name: &str, text: &str) -> Result<Imported, Vec<Diag>> {
    import_with(lang, name, text, &Options::default())
}

/// [`import`] with options.
///
/// # Errors
/// As [`import`].
pub fn import_with(
    lang: &str,
    name: &str,
    text: &str,
    opts: &Options,
) -> Result<Imported, Vec<Diag>> {
    let file = if opts.file.is_empty() {
        name.to_string()
    } else {
        opts.file.clone()
    };
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n");
    let out = match lang {
        "st" | "scl" => st::import(st::Input {
            file: &file,
            name,
            text: &text,
            base: 0,
            scan_ns: opts.scan_ns,
            tool: if lang == "scl" {
                "import-scl"
            } else {
                "import-st"
            },
            sfc: None,
        })?,
        "plcopen" => plcopen::import(&file, name, &text, opts.scan_ns)?,
        "spel" => spel::import(&file, name, &text)?,
        other => {
            return Err(vec![Diag {
                code: "NE1001",
                message: format!(
                    "unknown language `{other}`; the importers read {}",
                    languages().join(", ")
                ),
                help: None,
                file,
                span: [0, 0],
            }])
        }
    };
    Ok(Imported {
        source: out.source,
        fidelity: out.fid,
        diags: out.diags,
    })
}
