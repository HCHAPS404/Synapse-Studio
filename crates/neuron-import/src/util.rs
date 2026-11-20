//! Small helpers shared by the importers: Neuron literals, identifiers and `NE1001`.

use neuron_emit::{Class, Diag, Fidelity};

/// A duration in nanoseconds as a Neuron literal in the largest unit that keeps it
/// whole (`2 s`, `1500 ms`, `250 us`, `7 ns`).
pub(crate) fn dur(ns: i64) -> String {
    let mag = ns.unsigned_abs();
    for (unit, k) in [("s", 1_000_000_000u64), ("ms", 1_000_000), ("us", 1_000)] {
        if mag != 0 && mag % k == 0 {
            let sign = if ns < 0 { "-" } else { "" };
            return format!("{sign}{} {unit}", mag / k);
        }
    }
    if mag == 0 {
        return "0 ms".to_string();
    }
    format!("{ns} ns")
}

/// A real literal: always with a decimal point (`1.0`, `0.25`).
pub(crate) fn real(v: f64) -> String {
    let s = format!("{v}");
    if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
        s
    } else {
        format!("{s}.0")
    }
}

/// A number written the shortest way (`2`, `1.2`), for robot programs.
pub(crate) fn num(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    format!("{v}")
}

/// `NE1001`: the input cannot be parsed.
pub(crate) fn syntax(file: &str, pos: usize, len: usize, message: &str) -> Diag {
    Diag {
        code: "NE1001",
        message: message.to_string(),
        help: Some("the importers read the subset listed in docs/translation.md".to_string()),
        file: file.to_string(),
        span: [pos, pos + len.max(1)],
    }
}

/// A name that is a valid Neuron identifier.
pub(crate) fn ident(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'm');
    }
    out
}

/// A module name that is a valid identifier and not a reserved keyword (`cell` becomes
/// `cell_prog`, `3axis` becomes `m3axis`). When it differs from `s`, the fidelity
/// report gets an `equivalent` entry "module renamed".
pub(crate) fn module_name(s: &str, fid: &mut Fidelity, file: &str, span: [usize; 2]) -> String {
    let mut out = ident(s);
    if neuron_syntax::Keyword::lookup(&out).is_some() {
        out.push_str("_prog");
    }
    if out != s {
        fid.record(
            file,
            span,
            Class::Equivalent,
            "module renamed",
            &format!(
                "`{s}` is not a valid Neuron module name (a reserved word or invalid characters); the module is called `{out}`"
            ),
        );
    }
    out
}
