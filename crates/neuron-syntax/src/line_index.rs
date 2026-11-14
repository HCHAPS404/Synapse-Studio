//! Conversion between byte offsets and line/column positions.

/// Index of line start offsets for a source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    /// Builds the index. A line starts at offset 0 and after every newline byte.
    pub fn new(src: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    /// Number of lines.
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// 1-based line and column (in characters) of `offset`.
    pub fn line_col(&self, src: &str, offset: usize) -> (usize, usize) {
        let offset = offset.min(src.len());
        let line = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let start = self.starts[line];
        let col = match src.get(start..offset) {
            Some(s) => s.chars().count(),
            None => offset - start,
        };
        (line + 1, col + 1)
    }

    /// Text of a 1-based line without its line terminator.
    pub fn line_text<'a>(&self, src: &'a str, line: usize) -> &'a str {
        if line == 0 || line > self.starts.len() {
            return "";
        }
        let start = self.starts[line - 1];
        let end = self.starts.get(line).copied().unwrap_or(src.len());
        src.get(start..end)
            .unwrap_or_default()
            .trim_end_matches(['\n', '\r'])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions() {
        let src = "ab\ncd\r\n\u{b5}x";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_col(src, 0), (1, 1));
        assert_eq!(idx.line_col(src, 4), (2, 2));
        assert_eq!(idx.line_text(src, 2), "cd");
        assert_eq!(idx.line_col(src, src.len() - 1), (3, 2));
    }
}
