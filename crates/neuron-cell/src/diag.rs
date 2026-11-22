//! Diagnostics of the cell compiler.
use serde::Serialize;

/// One problem found while compiling a cell, with its `NE####` code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellDiag {
    /// Code such as `NE0704`.
    pub code: &'static str,
    /// Source file of the cell.
    pub file: String,
    /// Byte range of the item, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<[usize; 2]>,
    /// One-line message.
    pub message: String,
    /// Hints, one per line.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub help: Vec<String>,
}

impl CellDiag {
    /// A diagnostic without hints.
    pub fn new(
        code: &'static str,
        file: &str,
        span: Option<[usize; 2]>,
        message: impl Into<String>,
    ) -> CellDiag {
        CellDiag {
            code,
            file: file.to_string(),
            span,
            message: message.into(),
            help: Vec::new(),
        }
    }

    /// Adds a hint.
    pub fn help(mut self, h: impl Into<String>) -> CellDiag {
        self.help.push(h.into());
        self
    }
}

impl std::fmt::Display for CellDiag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error[{}]: {} ({})", self.code, self.message, self.file)?;
        for h in &self.help {
            write!(f, "\n  = help: {h}")?;
        }
        Ok(())
    }
}
