//! `neuron explain NE####`: the long explanation of a diagnostic.
//!
//! The text comes from `docs/errors/`, which is embedded at build time, so the command
//! needs no files at run time and can never disagree with the documentation.

const SYNTAX: &str = include_str!("../../../docs/errors/syntax.md");
const ANALYSIS: &str = include_str!("../../../docs/errors/analysis.md");
const TARGETS: &str = include_str!("../../../docs/errors/targets.md");
const HDL: &str = include_str!("../../../docs/errors/hdl.md");
const TRANSLATE: &str = include_str!("../../../docs/errors/translate.md");
const CELL: &str = include_str!("../../../docs/errors/cell.md");
const SERVICES: &str = include_str!("../../../docs/errors/services.md");
const RESERVED: &str = include_str!("../../../docs/errors/reserved.md");

/// Number of `#` that start a Markdown heading, and the heading text.
fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 {
        return None;
    }
    line[hashes..].strip_prefix(' ').map(|t| (hashes, t))
}

/// The section of `doc` whose heading starts with `code`.
fn section(doc: &str, code: &str) -> Option<String> {
    let mut out = String::new();
    let mut level = 0usize;
    let mut found = false;
    for line in doc.lines() {
        if let Some((l, text)) = heading(line) {
            if found && l <= level {
                break;
            }
            if !found && text.starts_with(code) {
                found = true;
                level = l;
                out.push_str(text);
                out.push('\n');
                continue;
            }
        }
        if found {
            out.push_str(line);
            out.push('\n');
        }
    }
    found.then(|| format!("{}\n", out.trim_end()))
}

/// The row of the table of codes that are reserved for later tasks.
fn reserved(code: &str) -> Option<String> {
    let row = RESERVED
        .lines()
        .find(|l| l.starts_with('|') && l.contains(code))?;
    let cells: Vec<&str> = row
        .split('|')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();
    match cells.as_slice() {
        [c, meaning, by] => Some(format!(
            "{c} {meaning}\nThis code is reserved: its analysis is part of {by}.\n"
        )),
        _ => None,
    }
}

/// Explains a code such as `NE0201` (any letter case). `None` when the code is unknown.
///
/// ```
/// let text = neuron_cli::explain::explain("ne0201").expect("documented");
/// assert!(text.starts_with("NE0201"));
/// ```
pub fn explain(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_uppercase();
    section(SYNTAX, &code)
        .or_else(|| section(ANALYSIS, &code))
        .or_else(|| section(TARGETS, &code))
        .or_else(|| section(HDL, &code))
        .or_else(|| section(TRANSLATE, &code))
        .or_else(|| section(CELL, &code))
        .or_else(|| section(SERVICES, &code))
        .or_else(|| reserved(&code))
}

#[cfg(test)]
mod tests {
    use super::explain;

    #[test]
    fn syntax_analysis_and_reserved_codes_are_explained() {
        assert!(explain("NE0001").is_some_and(|t| t.contains("2 spaces")));
        assert!(explain("ne0201").is_some_and(|t| t.starts_with("NE0201")));
        assert!(explain("NE0701").is_some_and(|t| t.contains("receiver phase")));
        assert!(explain("NE0703").is_some_and(|t| t.contains("one writer")));
        assert!(explain("NE0805").is_some_and(|t| t.contains("four-phase")));
        assert!(explain("NE0702").is_some_and(|t| t.contains("two_ff")));
        assert!(explain("NE1003").is_some_and(|t| t.contains("--policy")));
        assert!(explain("NE0601").is_some_and(|t| t.contains("timer0")));
        assert!(explain("ne0610").is_some_and(|t| t.starts_with("NE0610")));
        assert!(explain("NE0704").is_some_and(|t| t.contains("place m on d")));
        assert!(explain("NE1101").is_some_and(|t| t.contains("nothing was executed")));
        assert!(explain("ne1112").is_some_and(|t| t.starts_with("NE1112")));
        assert_eq!(explain("NE9999"), None);
    }

    #[test]
    fn a_section_stops_at_the_next_heading() {
        let t = explain("NE0001").unwrap_or_default();
        assert!(!t.contains("NE0002"), "{t}");
    }
}
