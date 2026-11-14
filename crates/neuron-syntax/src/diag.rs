//! Diagnostics (spec/language/10-diagnostics.md).

use crate::line_index::LineIndex;
use crate::span::Span;

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// The program is rejected (`NE` codes).
    Error,
    /// A lint (`NL` codes).
    Warning,
    /// Additional information.
    Note,
}

impl Severity {
    /// Lower-case name used when rendering.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

/// A diagnostic attached to source code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Code such as `NE0001`.
    pub code: &'static str,
    /// Severity.
    pub severity: Severity,
    /// One-line message in plain English.
    pub message: String,
    /// Primary span.
    pub span: Span,
    /// Secondary labels.
    pub labels: Vec<(Span, String)>,
    /// Suggested fix or hint.
    pub help: Option<String>,
}

impl Diagnostic {
    /// Creates an error.
    pub fn error(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            span,
            labels: Vec::new(),
            help: None,
        }
    }

    /// Creates a warning (lint).
    pub fn warning(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, message, span)
        }
    }

    /// Adds a help text.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Adds a secondary label.
    pub fn with_label(mut self, span: Span, text: impl Into<String>) -> Self {
        self.labels.push((span, text.into()));
        self
    }

    /// Returns `true` for errors.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// Renders the diagnostic in the compiler's text format.
    pub fn render(&self, path: &str, src: &str) -> String {
        let index = LineIndex::new(src);
        let (line, col) = index.line_col(src, self.span.start);
        let text = index.line_text(src, line);
        let gutter = line.to_string().len();
        let pad = " ".repeat(gutter);
        let mut out = format!(
            "{}[{}]: {}\n{pad}--> {path}:{line}:{col}\n{pad} |\n{line} | {text}\n",
            self.severity.as_str(),
            self.code,
            self.message
        );
        let width = caret_width(src, &index, self.span, line);
        out.push_str(&format!(
            "{pad} | {}{}\n",
            " ".repeat(col - 1),
            "^".repeat(width)
        ));
        for (span, label) in &self.labels {
            let (l, c) = index.line_col(src, span.start);
            out.push_str(&format!("{pad} = note: {label} ({path}:{l}:{c})\n"));
        }
        if let Some(help) = &self.help {
            out.push_str(&format!("{pad} = help: {help}\n"));
        }
        out
    }
}

fn caret_width(src: &str, index: &LineIndex, span: Span, line: usize) -> usize {
    let (end_line, end_col) = index.line_col(src, span.end);
    let (_, start_col) = index.line_col(src, span.start);
    if end_line == line && end_col > start_col {
        end_col - start_col
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_location_and_caret() {
        let src = "let x = 1\nlet t = 5ms\n";
        let d = Diagnostic::error(
            "NE0006",
            "invalid numeric literal",
            Span { start: 18, end: 21 },
        )
        .with_help("write `5 ms`");
        let r = d.render("a.neuron", src);
        assert!(r.contains("error[NE0006]: invalid numeric literal"));
        assert!(r.contains("--> a.neuron:2:9"));
        assert!(r.contains("        ^^^"));
        assert!(r.contains("help: write `5 ms`"));
    }
}
