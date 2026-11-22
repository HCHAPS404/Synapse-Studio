//! Machine-readable output (`--format json`, task E2.2).
//!
//! The commands print their human text with `eprintln!` and `println!`. This module
//! shadows those four macros for the whole binary: normally they write to the standard
//! streams exactly as before; in JSON mode they fill buffers instead. When the command
//! ends, [`finish`] turns the captured text into one JSON document on the real standard
//! output: the diagnostics (parsed from their rendered form, which is the one layout every
//! stage uses), the other messages, and the files written to the output directory.
//!
//! The JSON document (`"format": 1`):
//!
//! ```json
//! {
//!   "format": 1, "command": "check", "ok": false,
//!   "neuron_version": "0.1.0", "language_edition": "2026",
//!   "diagnostics": [{"code": "NE0201", "severity": "error", "message": "...",
//!                    "file": "a.neuron", "line": 3, "column": 5,
//!                    "help": "...", "notes": ["..."]}],
//!   "messages": ["built app.hex for uno"],
//!   "output_dir": "out/x", "artifacts": [{"path": "app.hex", "size": 1234}]
//! }
//! ```
//!
//! `code`, `file`, `line`, `column` and `help` are `null` when a stage gives none (a
//! missing tool, a usage error). `ok` is true when no diagnostic is an error; the process
//! exit code stays the authority. `artifacts` lists every regular file under the output
//! directory, sorted, with `/` separators.

use serde_json::{json, Value};
use std::fmt::Arguments;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Writes formatted text to the standard error stream, or to the capture buffer.
macro_rules! eprint {
    ($($t:tt)*) => { $crate::report::write_err(format_args!($($t)*)) };
}

/// Like `eprint!`, with a newline.
macro_rules! eprintln {
    () => { $crate::report::write_err(format_args!("\n")) };
    ($($t:tt)*) => { $crate::report::write_err(format_args!("{}\n", format_args!($($t)*))) };
}

/// Writes formatted text to the standard output stream, or to the capture buffer.
macro_rules! print {
    ($($t:tt)*) => { $crate::report::write_out(format_args!($($t)*)) };
}

/// Like `print!`, with a newline.
macro_rules! println {
    () => { $crate::report::write_out(format_args!("\n")) };
    ($($t:tt)*) => { $crate::report::write_out(format_args!("{}\n", format_args!($($t)*))) };
}

/// The most files listed as artifacts.
const MAX_ARTIFACTS: usize = 10_000;
/// The most free-text messages kept.
const MAX_MESSAGES: usize = 200;
/// First line of the usage text: nothing after it is a message.
const USAGE_MARK: &str = "neuron - the Neuron language toolchain";

#[derive(Default)]
struct Capture {
    err: String,
    out: String,
    out_dir: Option<PathBuf>,
}

static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);

fn lock() -> MutexGuard<'static, Option<Capture>> {
    CAPTURE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts capturing: from now on the macros fill buffers.
pub fn begin() {
    *lock() = Some(Capture::default());
}

/// Writes to standard error, or to the capture buffer.
pub fn write_err(args: Arguments<'_>) {
    let mut guard = lock();
    if let Some(c) = guard.as_mut() {
        let _ = std::fmt::Write::write_fmt(&mut c.err, args);
    } else {
        drop(guard);
        let _ = std::io::stderr().write_fmt(args);
    }
}

/// Writes to standard output, or to the capture buffer.
pub fn write_out(args: Arguments<'_>) {
    let mut guard = lock();
    if let Some(c) = guard.as_mut() {
        let _ = std::fmt::Write::write_fmt(&mut c.out, args);
    } else {
        drop(guard);
        let _ = std::io::stdout().write_fmt(args);
    }
}

/// Records the directory a command writes to, for the artifact list (no effect unless
/// capturing).
pub fn set_output_dir(dir: &Path) {
    if let Some(c) = lock().as_mut() {
        c.out_dir = Some(dir.to_path_buf());
    }
}

/// The value of `--format` in `args` (removed from them), when the command supports it.
///
/// `Ok(true)` is `--format json`; `Ok(false)` is the default or `--format human`.
///
/// # Errors
/// A message when the value is missing or unknown.
pub fn take_format(args: &mut Vec<String>) -> Result<bool, String> {
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        let value = if args[i] == "--format" {
            let Some(v) = args.get(i + 1).cloned() else {
                return Err("`--format` needs a value (human or json)".to_string());
            };
            args.drain(i..=i + 1);
            v
        } else if let Some(v) = args[i].strip_prefix("--format=") {
            let v = v.to_string();
            args.remove(i);
            v
        } else {
            i += 1;
            continue;
        };
        match value.as_str() {
            "json" => json = true,
            "human" => json = false,
            other => return Err(format!("`--format` is human or json, not `{other}`")),
        }
    }
    Ok(json)
}

/// One parsed diagnostic.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diag {
    /// `NE0201` and so on; `None` for a message without a code.
    pub code: Option<String>,
    /// `error`, `warning` or `note`.
    pub severity: String,
    /// The one-line message.
    pub message: String,
    /// File of the primary location.
    pub file: Option<String>,
    /// 1-based line.
    pub line: Option<u64>,
    /// 1-based column.
    pub column: Option<u64>,
    /// The `= help:` text.
    pub help: Option<String>,
    /// The `= note:` lines.
    pub notes: Vec<String>,
}

impl Diag {
    fn to_json(&self) -> Value {
        json!({
            "code": self.code,
            "severity": self.severity,
            "message": self.message,
            "file": self.file,
            "line": self.line,
            "column": self.column,
            "help": self.help,
            "notes": self.notes,
        })
    }
}

/// The header of a diagnostic: `error[NE0201]: text`, or `error: text` without a code.
fn header(line: &str) -> Option<Diag> {
    for severity in ["error", "warning", "note"] {
        let Some(rest) = line.strip_prefix(severity) else {
            continue;
        };
        if let Some(rest) = rest.strip_prefix('[') {
            let (code, message) = rest.split_once("]: ")?;
            let valid = code.len() == 6
                && code.starts_with('N')
                && code[2..].bytes().all(|b| b.is_ascii_digit());
            if !valid {
                return None;
            }
            return Some(Diag {
                code: Some(code.to_string()),
                severity: severity.to_string(),
                message: message.to_string(),
                ..Diag::default()
            });
        }
        // `note:` lines without a code are plain messages, not diagnostics.
        if severity != "note" {
            if let Some(message) = rest.strip_prefix(": ") {
                return Some(Diag {
                    code: None,
                    severity: severity.to_string(),
                    message: message.to_string(),
                    ..Diag::default()
                });
            }
        }
    }
    None
}

/// `path:line:col` (the path may hold colons).
fn location(text: &str) -> Option<(String, u64, u64)> {
    let mut parts = text.trim().rsplitn(3, ':');
    let col = parts.next()?.trim().parse().ok()?;
    let line = parts.next()?.trim().parse().ok()?;
    let file = parts.next()?;
    Some((file.to_string(), line, col))
}

/// Splits the text a command wrote to standard error into diagnostics and other messages.
#[must_use]
pub fn parse(text: &str) -> (Vec<Diag>, Vec<String>) {
    let mut diags: Vec<Diag> = Vec::new();
    let mut messages: Vec<String> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        if line == USAGE_MARK {
            break;
        }
        if let Some(d) = header(line) {
            diags.push(d);
            open = true;
            continue;
        }
        let in_block = open
            && line
                .chars()
                .next()
                .is_some_and(|c| c.is_whitespace() || c.is_ascii_digit());
        if in_block {
            let Some(d) = diags.last_mut() else { continue };
            let t = line.trim_start();
            if let Some(loc) = t.strip_prefix("--> ") {
                if let Some((f, l, c)) = location(loc) {
                    d.file = Some(f);
                    d.line = Some(l);
                    d.column = Some(c);
                } else {
                    d.file = Some(loc.trim().to_string());
                }
            } else if let Some(h) = t.strip_prefix("= help: ") {
                d.help = Some(h.to_string());
            } else if let Some(n) = t.strip_prefix("= note: ") {
                d.notes.push(n.to_string());
            }
            continue;
        }
        open = false;
        let t = line.trim();
        if !t.is_empty() && messages.len() < MAX_MESSAGES {
            messages.push(t.to_string());
        }
    }
    (diags, messages)
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if out.len() >= MAX_ARTIFACTS {
            return;
        }
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() {
            collect(root, &p, out);
        } else if meta.is_file() {
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, meta.len()));
        }
    }
}

/// The document for one command (the pure part of [`finish`]).
#[must_use]
pub fn document(command: &str, stderr: &str, out_dir: Option<&Path>) -> Value {
    let (diags, messages) = parse(stderr);
    let mut files = Vec::new();
    if let Some(dir) = out_dir {
        collect(dir, dir, &mut files);
        files.sort();
    }
    let ok = !diags.iter().any(|d| d.severity == "error");
    json!({
        "format": 1,
        "command": command,
        "ok": ok,
        "neuron_version": env!("CARGO_PKG_VERSION"),
        "language_edition": neuron_syntax::LANGUAGE_EDITION,
        "diagnostics": diags.iter().map(Diag::to_json).collect::<Vec<_>>(),
        "messages": messages,
        "output_dir": out_dir.map(|d| d.to_string_lossy().replace('\\', "/")),
        "artifacts": files
            .iter()
            .map(|(p, s)| json!({"path": p, "size": s}))
            .collect::<Vec<_>>(),
    })
}

/// Ends the capture and prints the JSON document on the real standard output. The exit
/// code of the command is returned unchanged.
pub fn finish(command: &str, code: ExitCode) -> ExitCode {
    let captured = lock().take().unwrap_or_default();
    let doc = document(command, &captured.err, captured.out_dir.as_deref());
    let text = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string());
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.write_all(b"\n");
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    const RENDERED: &str = "\
error[NE0201]: type mismatch
 --> src/a.neuron:3:5
  |
3 |     let x: int = true
  |                  ^^^^
  = note: declared here (src/a.neuron:2:1)
  = help: write `0`

warning[NL0001]: naming style
 --> C:\\w\\b.neuron:1:1
  = help: use snake_case
error: the program has errors, so nothing was built
built app.hex for uno
";

    #[test]
    fn parses_rendered_diagnostics() {
        let (d, m) = parse(RENDERED);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].code.as_deref(), Some("NE0201"));
        assert_eq!(d[0].severity, "error");
        assert_eq!(d[0].file.as_deref(), Some("src/a.neuron"));
        assert_eq!((d[0].line, d[0].column), (Some(3), Some(5)));
        assert_eq!(d[0].help.as_deref(), Some("write `0`"));
        assert_eq!(d[0].notes, vec!["declared here (src/a.neuron:2:1)"]);
        assert_eq!(d[1].severity, "warning");
        assert_eq!(d[1].file.as_deref(), Some("C:\\w\\b.neuron"));
        assert_eq!(d[2].code, None);
        assert_eq!(m, vec!["built app.hex for uno"]);
    }

    #[test]
    fn usage_text_is_not_a_message() {
        let (d, m) = parse(&format!("error: bad\n\n{USAGE_MARK}\nusage:\n  neuron x\n"));
        assert_eq!(d.len(), 1);
        assert!(m.is_empty());
    }

    #[test]
    fn plain_notes_and_unknown_codes_are_messages() {
        let (d, m) = parse("note: 2 module(s) were left out\nerror[XX1]: nope\n");
        assert!(d.is_empty());
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn format_flag_is_removed() {
        let mut a: Vec<String> = ["x", "--format", "json", "y"].map(String::from).to_vec();
        assert_eq!(take_format(&mut a), Ok(true));
        assert_eq!(a, ["x", "y"]);
        let mut b: Vec<String> = ["--format=human", "z"].map(String::from).to_vec();
        assert_eq!(take_format(&mut b), Ok(false));
        assert_eq!(b, ["z"]);
        let mut c: Vec<String> = vec!["--format".into()];
        assert!(take_format(&mut c).is_err());
        let mut d: Vec<String> = ["--format", "xml"].map(String::from).to_vec();
        assert!(take_format(&mut d).is_err());
    }

    #[test]
    fn document_has_the_documented_keys() {
        let doc = document("check", "error[NE0001]: tab\n --> a.neuron:1:1\n", None);
        assert_eq!(doc["format"], 1);
        assert_eq!(doc["ok"], false);
        assert_eq!(doc["diagnostics"][0]["code"], "NE0001");
        assert!(doc["artifacts"].as_array().is_some_and(Vec::is_empty));
    }
}
