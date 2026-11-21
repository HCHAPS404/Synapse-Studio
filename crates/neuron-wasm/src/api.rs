//! The compiler API: strings in, JSON strings out.
//!
//! Every function returns one JSON object. On success it has `"ok": true` plus the fields
//! documented on the function. On failure it is
//! `{"ok": false, "error": {"kind": "...", "message": "..."}, "diagnostics": [...]}`,
//! where `kind` is one of:
//!
//! | `kind` | Meaning |
//! |---|---|
//! | `bad_request` | The input is not what the function expects (malformed JSON, unknown file, bad scenario). |
//! | `unknown_language` | No emitter or importer has that name. |
//! | `syntax` | `format`: the text does not parse. |
//! | `analysis` | The program has errors, so there is no NIR to run, test or emit. |
//! | `run` | The interpreter could not load the program. |
//! | `emit` | The emitter cannot translate a construct (`NE0610` and friends). |
//! | `import` | The importer cannot read the text (`NE1001`). |
//! | `internal` | A bug in the compiler (a panic was caught). |
//!
//! No function panics on any input.
//!
//! # Files
//!
//! `files_json` is either an object `{"<path>": "<text>", ...}` or an array
//! `[{"path": "...", "text": "..."}, ...]`. An entry called `neuron.toml` (at any depth)
//! is the package manifest; every other entry is a Neuron source file.
//!
//! # Positions
//!
//! Byte offsets (`span`, `start`/`end` arguments) are UTF-8 byte offsets into the text
//! after a leading byte order mark is removed. `line` and `col` are 1-based, and `col`
//! counts Unicode scalar values. JavaScript strings are UTF-16: convert offsets when the
//! text has characters outside the Basic Multilingual Plane.
//!
//! ```
//! let files = r#"{"blink.neuron": "module blink for mcu\nio led: gpio.output @ D13\nsafety:\n  safe_state:\n    led.write(false)\n"}"#;
//! let out: serde_json::Value = serde_json::from_str(&neuron_wasm::api::check(files)).unwrap();
//! assert_eq!(out["ok"], true);
//! assert_eq!(out["has_errors"], false);
//! ```

use neuron_emit::EmitOptions;
use neuron_interp::{Interp, Options};
use neuron_sema::{analyze, Analysis, SemaOptions, SourceFile};
use neuron_syntax::{Diagnostic, LineIndex};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// A failed call: becomes `{"ok": false, "error": ..., "diagnostics": ...}`.
struct ApiError {
    kind: &'static str,
    message: String,
    diagnostics: Vec<Value>,
}

type Reply = Result<Value, ApiError>;

fn fail(kind: &'static str, message: impl Into<String>) -> ApiError {
    ApiError {
        kind,
        message: message.into(),
        diagnostics: Vec::new(),
    }
}

fn bad(message: impl Into<String>) -> ApiError {
    fail("bad_request", message)
}

/// Runs `f`, turns its result (or a panic) into the JSON text of the reply.
fn finish(f: impl FnOnce() -> Reply) -> String {
    let reply = match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => {
            let what = p
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            Err(fail("internal", format!("internal compiler error: {what}")))
        }
    };
    match reply {
        Ok(v) => v.to_string(),
        Err(e) => json!({
            "ok": false,
            "error": {"kind": e.kind, "message": e.message},
            "diagnostics": e.diagnostics,
        })
        .to_string(),
    }
}

// ---------------------------------------------------------------------------------------
// Input and shared output shapes

struct Package {
    sources: Vec<SourceFile>,
    manifest: Option<String>,
}

fn parse_files(files_json: &str) -> Result<Package, ApiError> {
    let v: Value = serde_json::from_str(files_json)
        .map_err(|e| bad(format!("files_json is not valid JSON: {e}")))?;
    let mut entries: Vec<(String, String)> = Vec::new();
    match v {
        Value::Object(m) => {
            for (path, text) in m {
                let Value::String(text) = text else {
                    return Err(bad(format!("the text of `{path}` must be a string")));
                };
                entries.push((path, text));
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let path = item.get("path").and_then(Value::as_str);
                let text = item.get("text").and_then(Value::as_str);
                let (Some(path), Some(text)) = (path, text) else {
                    return Err(bad(format!(
                        "entry {i} of files_json needs the strings `path` and `text`"
                    )));
                };
                entries.push((path.to_string(), text.to_string()));
            }
        }
        _ => {
            return Err(bad(
                "files_json must be an object {path: text} or an array of {path, text}",
            ))
        }
    }
    let mut seen = BTreeSet::new();
    let mut sources = Vec::new();
    let mut manifest = None;
    for (path, text) in entries {
        if !seen.insert(path.clone()) {
            return Err(bad(format!("the path `{path}` appears twice")));
        }
        if path == "neuron.toml" || path.ends_with("/neuron.toml") {
            manifest = Some(text);
        } else {
            sources.push(SourceFile { path, text });
        }
    }
    if sources.is_empty() {
        return Err(bad("files_json has no source files"));
    }
    Ok(Package { sources, manifest })
}

fn analysis(files_json: &str) -> Result<Analysis, ApiError> {
    let p = parse_files(files_json)?;
    Ok(analyze(
        p.sources,
        p.manifest.as_deref(),
        &SemaOptions::default(),
    ))
}

fn position(idx: &LineIndex, text: &str, offset: usize) -> Value {
    let (line, col) = idx.line_col(text, offset);
    json!({"line": line, "col": col})
}

/// JSON of one diagnostic of the analyzer or the parser.
fn diag_json(d: &Diagnostic, path: &str, text: &str) -> Value {
    let idx = LineIndex::new(text);
    let labels: Vec<Value> = d
        .labels
        .iter()
        .map(|(span, message)| {
            json!({
                "message": message,
                "span": [span.start, span.end],
                "start": position(&idx, text, span.start),
                "end": position(&idx, text, span.end),
            })
        })
        .collect();
    json!({
        "code": d.code,
        "severity": d.severity.as_str(),
        "message": d.message,
        "help": d.help,
        "file": path,
        "span": [d.span.start, d.span.end],
        "start": position(&idx, text, d.span.start),
        "end": position(&idx, text, d.span.end),
        "labels": labels,
        "rendered": d.render(path, text),
    })
}

fn analysis_diags(a: &Analysis) -> Vec<Value> {
    a.diagnostics
        .iter()
        .map(|d| diag_json(&d.diag, &a.paths[d.file], &a.texts[d.file]))
        .collect()
}

fn analysis_error(a: &Analysis) -> ApiError {
    ApiError {
        kind: "analysis",
        message: "the program has errors".to_string(),
        diagnostics: analysis_diags(a),
    }
}

/// JSON of a diagnostic of an emitter or importer. `files` maps paths to texts, so the
/// line and column can be computed when the file is known.
fn tool_diag_json(d: &neuron_emit::Diag, severity: &str, files: &[(&str, &str)]) -> Value {
    let place = files.iter().find(|(p, _)| *p == d.file).map(|(_, text)| {
        let idx = LineIndex::new(text);
        (
            position(&idx, text, d.span[0]),
            position(&idx, text, d.span[1]),
        )
    });
    json!({
        "code": d.code,
        "severity": severity,
        "message": d.message,
        "help": d.help,
        "file": d.file,
        "span": d.span,
        "start": place.as_ref().map(|p| p.0.clone()),
        "end": place.as_ref().map(|p| p.1.clone()),
    })
}

fn tool_diags(diags: &[neuron_emit::Diag], severity: &str, files: &[(&str, &str)]) -> Vec<Value> {
    diags
        .iter()
        .map(|d| tool_diag_json(d, severity, files))
        .collect()
}

// ---------------------------------------------------------------------------------------
// The functions

/// Analyzes a package, like `neuron check`.
///
/// Reply: `{"ok": true, "has_errors": bool, "files": [path], "diagnostics": [D]}` with
/// `D = {code, severity ("error"|"warning"|"note"), message, help (string|null), file,
/// span: [start, end], start: {line, col}, end: {line, col}, labels: [{message, span,
/// start, end}], rendered}`. `rendered` is the text `neuron check` prints.
pub fn check(files_json: &str) -> String {
    finish(|| {
        let a = analysis(files_json)?;
        Ok(json!({
            "ok": true,
            "has_errors": !a.is_ok(),
            "files": a.paths,
            "diagnostics": analysis_diags(&a),
        }))
    })
}

/// Formats one source text, like `neuron fmt`.
///
/// Reply: `{"ok": true, "text": string, "changed": bool, "fell_back": bool}`. When the
/// text does not parse the reply is an error of kind `syntax` with the parser's
/// diagnostics (file `<input>`, positions in the text with `\r\n` and `\r` read as `\n`).
pub fn format(text: &str) -> String {
    finish(|| match neuron_syntax::format::format_source(text) {
        Ok(f) => Ok(json!({
            "ok": true,
            "changed": f.text != text,
            "fell_back": f.fell_back,
            "text": f.text,
        })),
        Err(diags) => {
            let norm = text
                .strip_prefix('\u{feff}')
                .unwrap_or(text)
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            Err(ApiError {
                kind: "syntax",
                message: "the source has syntax errors and was not changed".to_string(),
                diagnostics: diags
                    .iter()
                    .map(|d| diag_json(d, "<input>", &norm))
                    .collect(),
            })
        }
    })
}

fn file_index(a: &Analysis, file: &str) -> Result<usize, ApiError> {
    a.paths
        .iter()
        .position(|p| p == file)
        .ok_or_else(|| bad(format!("the file `{file}` is not in files_json")))
}

/// What the cursor is on: the declaration of the name and the type of the expression.
///
/// `file` is a key of `files_json` and `offset` a byte offset into its text. Reply:
/// `{"ok": true, "hover": null | {"markdown": string, "start": n, "end": n, "range":
/// {start: {line, col}, end: {line, col}}}}`. `hover` is `null` when there is nothing
/// to say. The package is analyzed on every call.
pub fn hover(files_json: &str, file: &str, offset: usize) -> String {
    finish(|| {
        let a = analysis(files_json)?;
        let fi = file_index(&a, file)?;
        let text = &a.texts[fi];
        let Some((name, start, end)) = neuron_lsp::name_at(text, offset) else {
            return Ok(json!({"ok": true, "hover": null}));
        };
        let mut parts: Vec<String> = Vec::new();
        if let Some((_, _, header, doc)) = neuron_lsp::find_declaration(&a.texts, &name) {
            parts.push(format!("```neuron\n{header}\n```"));
            if !doc.is_empty() {
                parts.push(doc);
            }
        }
        if let Some(ty) = a
            .nir
            .as_ref()
            .and_then(|nir| neuron_lsp::type_at(nir, fi, offset))
        {
            parts.push(format!("type: `{ty}`"));
        }
        if parts.is_empty() {
            return Ok(json!({"ok": true, "hover": null}));
        }
        let idx = LineIndex::new(text);
        Ok(json!({
            "ok": true,
            "hover": {
                "markdown": parts.join("\n\n"),
                "start": start,
                "end": end,
                "range": {
                    "start": position(&idx, text, start),
                    "end": position(&idx, text, end),
                },
            },
        }))
    })
}

/// Where the name under the cursor is declared.
///
/// Arguments as for [`hover`]. Reply: `{"ok": true, "definition": null | {"file": path,
/// "line": n, "start": {line, col}, "end": {line, col}, "header": string, "doc":
/// string}}`.
pub fn definition(files_json: &str, file: &str, offset: usize) -> String {
    finish(|| {
        let a = analysis(files_json)?;
        let fi = file_index(&a, file)?;
        let Some((name, _, _)) = neuron_lsp::name_at(&a.texts[fi], offset) else {
            return Ok(json!({"ok": true, "definition": null}));
        };
        let Some((di, line0, header, doc)) = neuron_lsp::find_declaration(&a.texts, &name) else {
            return Ok(json!({"ok": true, "definition": null}));
        };
        let text = &a.texts[di];
        let idx = LineIndex::new(text);
        let width = idx.line_text(text, line0 + 1).chars().count();
        Ok(json!({
            "ok": true,
            "definition": {
                "file": a.paths[di],
                "line": line0 + 1,
                "start": {"line": line0 + 1, "col": 1},
                "end": {"line": line0 + 1, "col": width + 1},
                "header": header,
                "doc": doc,
            },
        }))
    })
}

/// Runs the program in the reference interpreter, like `neuron run --for`.
///
/// `end_ns` is the simulated time to run, in nanoseconds. `scenario` is the text of a
/// scenario file (see `neuron-interp`'s `scenario` module). Reply: `{"ok": true,
/// "end_ns": n, "now_ns": n, "fault": string|null, "trace": string, "events":
/// [{t_ns, kind, name, value}], "diagnostics": [D]}`. `trace` is the exact text
/// `neuron run` prints; `events` has the same entries in structured form (`kind` is
/// `OUT`, `SIG`, `FAULT`, `RESET`, `EVENT`, `STATE`, `LOG` or `MOVE`).
pub fn run(files_json: &str, end_ns: i64, scenario: Option<&str>) -> String {
    finish(|| {
        if end_ns < 0 {
            return Err(bad("end_ns must not be negative"));
        }
        let mut a = analysis(files_json)?;
        let Some(nir) = a.nir.take() else {
            return Err(analysis_error(&a));
        };
        let opts = Options {
            end_ns,
            ..Options::default()
        };
        let mut it = Interp::new(nir, opts).map_err(|e| fail("run", e.message))?;
        if let Some(text) = scenario.filter(|s| !s.trim().is_empty()) {
            let sc =
                neuron_interp::parse_scenario(text).map_err(|e| bad(format!("scenario: {e}")))?;
            it.load_scenario(&sc);
        }
        it.run();
        let events: Vec<Value> = it
            .trace()
            .iter()
            .map(|e| {
                json!({
                    "t_ns": e.t_ns,
                    "kind": e.kind.tag(),
                    "name": e.name,
                    "value": e.value,
                })
            })
            .collect();
        Ok(json!({
            "ok": true,
            "end_ns": end_ns,
            "now_ns": it.now_ns(),
            "fault": it.current_fault(),
            "trace": neuron_interp::render(it.trace()),
            "events": events,
            "diagnostics": analysis_diags(&a),
        }))
    })
}

/// Runs every `test` block, like `neuron test`.
///
/// Reply: `{"ok": true, "total": n, "passed": n, "failed": n, "tests": [{file, module,
/// name, passed, message, at: null | {file, span, start, end}}]}`. `module` is the index
/// of the module in the package; `at` is the failed assertion.
pub fn test(files_json: &str) -> String {
    finish(|| {
        let a = analysis(files_json)?;
        let Some(nir) = a.nir.as_ref() else {
            return Err(analysis_error(&a));
        };
        let reports = neuron_interp::run_tests(nir, &Options::default());
        let tests: Vec<Value> = reports
            .iter()
            .map(|r| {
                let at = r.at.map(|(m, span)| {
                    let text = &a.texts[m];
                    let idx = LineIndex::new(text);
                    json!({
                        "file": a.paths[m],
                        "span": span,
                        "start": position(&idx, text, span[0]),
                        "end": position(&idx, text, span[1]),
                    })
                });
                json!({
                    "file": a.paths[r.module],
                    "module": r.module,
                    "name": r.name,
                    "passed": r.passed,
                    "message": r.message,
                    "at": at,
                })
            })
            .collect();
        let passed = reports.iter().filter(|r| r.passed).count();
        Ok(json!({
            "ok": true,
            "total": reports.len(),
            "passed": passed,
            "failed": reports.len() - passed,
            "tests": tests,
        }))
    })
}

/// Translates the program to another language, like `neuron emit`.
///
/// `lang` is one of `neuron_emit::names()` (`st`, `scl`, `plcopen`, `spel`, `python`,
/// `kernel`, `c`). `profile` is the dialect profile (`generic`, `yaskawa`), empty for the
/// default. Reply: `{"ok": true, "lang": string, "files": {path: text}, "fidelity":
/// object, "diagnostics": [D]}`. `files` has what `neuron emit` writes (generated files,
/// `fidelity.json`, `wiring.md`, source maps); `fidelity` is the parsed content of
/// `fidelity.json`; `diagnostics` are the warnings of the analysis. When the emitter
/// refuses a construct the reply is an error of kind `emit` whose `diagnostics` are its
/// diagnostics.
pub fn emit(lang: &str, files_json: &str, profile: Option<&str>) -> String {
    finish(|| {
        let Some(emitter) = neuron_emit::emitter(lang) else {
            return Err(fail(
                "unknown_language",
                format!(
                    "unknown emitter `{lang}`; the emitters are {}",
                    neuron_emit::names().join(", ")
                ),
            ));
        };
        let a = analysis(files_json)?;
        let Some(nir) = a.nir.as_ref() else {
            return Err(analysis_error(&a));
        };
        let opts = EmitOptions {
            profile: profile.unwrap_or_default().to_string(),
            robot: String::new(),
        };
        match emitter.emit(nir, &opts) {
            Ok(pkg) => Ok(json!({
                "ok": true,
                "lang": lang,
                "files": pkg.all_files(),
                "fidelity": serde_json::to_value(&pkg.fidelity).unwrap_or(Value::Null),
                "diagnostics": analysis_diags(&a),
            })),
            Err(diags) => {
                let files: Vec<(&str, &str)> = a
                    .paths
                    .iter()
                    .zip(&a.texts)
                    .map(|(p, t)| (p.as_str(), t.as_str()))
                    .collect();
                Err(ApiError {
                    kind: "emit",
                    message: format!("the program cannot be translated to `{lang}`"),
                    diagnostics: tool_diags(&diags, "error", &files),
                })
            }
        }
    })
}

/// Reads source text written in another language and returns Neuron source, like
/// `neuron import`.
///
/// `lang` is one of `neuron_import::languages()`; `name` is the name of the module that
/// is created. Reply: `{"ok": true, "lang": string, "name": string, "source": string,
/// "fidelity": object, "complete": bool, "diagnostics": [D]}` where the diagnostics are
/// the `NE1002` warnings of the constructs that were not imported (file = `name`). When
/// the text cannot be read the reply is an error of kind `import` (`NE1001`).
pub fn import(lang: &str, name: &str, text: &str) -> String {
    finish(|| {
        if !neuron_import::languages().contains(&lang) {
            return Err(fail(
                "unknown_language",
                format!(
                    "unknown language `{lang}`; the importers read {}",
                    neuron_import::languages().join(", ")
                ),
            ));
        }
        let norm = text
            .strip_prefix('\u{feff}')
            .unwrap_or(text)
            .replace("\r\n", "\n");
        let files = [(name, norm.as_str())];
        match neuron_import::import(lang, name, text) {
            Ok(imp) => Ok(json!({
                "ok": true,
                "lang": lang,
                "name": name,
                "source": imp.source,
                "complete": imp.complete(),
                "fidelity": serde_json::to_value(&imp.fidelity).unwrap_or(Value::Null),
                "diagnostics": tool_diags(&imp.diags, "warning", &files),
            })),
            Err(diags) => Err(ApiError {
                kind: "import",
                message: format!("the text could not be read as {lang}"),
                diagnostics: tool_diags(&diags, "error", &files),
            }),
        }
    })
}

/// Versions of the compiler and its formats.
///
/// Reply: `{"ok": true, "compiler": "0.0.1", "language_edition": "2026", "nir_schema":
/// "1.0", "protocol": 1, "emitters": [name], "importers": [name]}`.
pub fn version() -> String {
    finish(|| {
        Ok(json!({
            "ok": true,
            "compiler": env!("CARGO_PKG_VERSION"),
            "language_edition": neuron_syntax::LANGUAGE_EDITION,
            "nir_schema": neuron_nir::NIR_VERSION,
            "protocol": synapse_protocol::PROTOCOL_VERSION,
            "emitters": neuron_emit::names(),
            "importers": neuron_import::languages(),
        }))
    })
}
