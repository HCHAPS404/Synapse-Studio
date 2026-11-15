//! Semantic analysis for Neuron: names, types and units, dialect rules, time and safety.
//!
//! Specification: `spec/language/15-analysis.md` and the earlier chapters.
//!
//! ```
//! use neuron_sema::{analyze, SemaOptions, SourceFile};
//!
//! let src = "module blink for mcu\nio led: gpio.output @ D13\nsafety:\n  safe_state:\n    led.write(false)\n";
//! let a = analyze(
//!     vec![SourceFile { path: "blink.neuron".into(), text: src.into() }],
//!     None,
//!     &SemaOptions::default(),
//! );
//! assert!(a.errors().next().is_none());
//! ```

mod check;
mod consteval;
mod facts;
mod lower;
mod manifest;
mod mc;
mod model;
mod prelude;
mod report;
mod resolve;
mod rules;
mod safety;
mod timing;
mod ty;
pub mod units;
mod visit;

pub use lower::rat_text;
pub use manifest::Manifest;
pub use model::{FileDiag, SourceFile};
pub use neuron_nir as nir;
pub use report::{ActivityTiming, Counterexample, MemoryReport, Reports, ResourceInfo, TraceStep};

use neuron_syntax::Diagnostic;

/// Settings for the analysis. The defaults describe "no target model": checks
/// that need a target (response-time analysis, memory budget) are reported as
/// unavailable instead of guessing.
#[derive(Debug, Clone)]
pub struct SemaOptions {
    /// CPU clock used to turn operation counts into time. `None` disables WCET analysis.
    pub cpu_hz: Option<u64>,
    /// Cycles charged per abstract operation of the cost model.
    pub cycles_per_op: u64,
    /// Total RAM of the target in bytes. `None` disables the memory budget check.
    pub ram_bytes: Option<u64>,
    /// RAM reserved for the runtime itself.
    pub runtime_ram_bytes: u64,
    /// Base tick in nanoseconds (default: 1 ms for `mcu`, the scan time for `plc`).
    pub base_tick_ns: Option<u64>,
    /// Maximum number of states the model checker explores per property.
    pub max_model_states: usize,
}

impl Default for SemaOptions {
    fn default() -> Self {
        SemaOptions {
            cpu_hz: None,
            cycles_per_op: 4,
            ram_bytes: None,
            runtime_ram_bytes: 256,
            base_tick_ns: None,
            max_model_states: 400_000,
        }
    }
}

/// Result of analyzing a package.
#[derive(Debug)]
pub struct Analysis {
    /// Paths of the analyzed files, by index.
    pub paths: Vec<String>,
    /// Source texts, by index (used to render diagnostics).
    pub texts: Vec<String>,
    /// All diagnostics, in a deterministic order (file, position, code).
    pub diagnostics: Vec<FileDiag>,
    /// Timing, resources, memory and counterexamples.
    pub reports: Reports,
    /// The program in NIR form (spec 16). `None` when there is any error.
    pub nir: Option<neuron_nir::Nir>,
}

impl Analysis {
    /// Iterates over the errors.
    pub fn errors(&self) -> impl Iterator<Item = &FileDiag> {
        self.diagnostics.iter().filter(|d| d.diag.is_error())
    }

    /// `true` when no diagnostic is an error.
    pub fn is_ok(&self) -> bool {
        self.errors().next().is_none()
    }

    /// Codes of all diagnostics, in order.
    pub fn codes(&self) -> Vec<&'static str> {
        self.diagnostics.iter().map(|d| d.diag.code).collect()
    }

    /// Renders every diagnostic in the compiler's text format.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for d in &self.diagnostics {
            out.push_str(&d.diag.render(&self.paths[d.file], &self.texts[d.file]));
            out.push('\n');
        }
        out
    }
}

fn sort_diags(diags: &mut [FileDiag]) {
    diags.sort_by(|a, b| {
        (a.file, a.diag.span.start, a.diag.code, &a.diag.message).cmp(&(
            b.file,
            b.diag.span.start,
            b.diag.code,
            &b.diag.message,
        ))
    });
}

/// Analyzes the files of one package.
///
/// `manifest` is the text of `neuron.toml`, when there is one.
pub fn analyze(sources: Vec<SourceFile>, manifest: Option<&str>, opts: &SemaOptions) -> Analysis {
    let mut diags: Vec<FileDiag> = Vec::new();
    let manifest = match manifest.map(Manifest::parse) {
        Some(Ok(m)) => Some(m),
        Some(Err(msg)) => {
            diags.push(FileDiag {
                file: 0,
                diag: Diagnostic::error(
                    "NE0108",
                    format!("invalid neuron.toml: {msg}"),
                    neuron_syntax::Span::at(0),
                ),
            });
            None
        }
        None => None,
    };
    // Editors on Windows often save a byte order mark; it is not part of the program.
    let sources: Vec<SourceFile> = sources
        .into_iter()
        .map(|s| SourceFile {
            text: s.text.trim_start_matches('\u{feff}').to_string(),
            ..s
        })
        .collect();
    let paths = sources.iter().map(|s| s.path.clone()).collect();
    let texts = sources.iter().map(|s| s.text.clone()).collect();
    let prog = model::Program::build(sources, manifest, &mut diags);
    let resolved = resolve::resolve_program(&prog, &mut diags);
    let checked = check::check_program(&prog, &resolved, &mut diags);
    let fact_base = facts::build(&prog, &resolved, &checked);
    rules::check_rules(&prog, &resolved, &checked, &fact_base, &mut diags);
    let timing = timing::analyze_timing(&prog, &resolved, &checked, &fact_base, opts, &mut diags);
    safety::check_safety(&prog, &checked, &fact_base, &mut diags);
    let mut counterexamples = Vec::new();
    for (mi, f) in prog.files.iter().enumerate() {
        if !f.syntax_errors {
            mc::verify_module(&prog, &resolved, mi, opts, &mut diags, &mut counterexamples);
        }
    }
    sort_diags(&mut diags);
    diags.dedup();
    let nir = if diags.iter().any(|d| d.diag.is_error()) {
        None
    } else {
        Some(lower::lower_program(&prog, &resolved, &checked))
    };
    Analysis {
        nir,
        paths,
        texts,
        diagnostics: diags,
        reports: Reports {
            timing: timing.timing,
            resources: timing.resources,
            memory: timing.memory,
            counterexamples,
        },
    }
}
