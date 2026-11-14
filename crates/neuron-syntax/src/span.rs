//! Source spans.

/// A half-open byte range `[start, end)` inside a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    /// Inclusive start offset in bytes.
    pub start: usize,
    /// Exclusive end offset in bytes.
    pub end: usize,
}

impl Span {
    /// Creates a span. Returns `None` if `start > end`.
    pub fn new(start: usize, end: usize) -> Option<Self> {
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    /// A zero-width span at `offset`.
    pub fn at(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// Length of the span in bytes.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Returns `true` if the span covers no bytes.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Smallest span that covers both `self` and `other`.
    pub fn merge(&self, other: &Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// The text covered by this span, or an empty string if out of bounds.
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        src.get(self.start..self.end).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn span(a: usize, b: usize) -> Span {
        Span {
            start: a.min(b),
            end: a.max(b),
        }
    }

    #[test]
    fn rejects_reversed_span() {
        assert!(Span::new(5, 2).is_none());
    }

    proptest! {
        #[test]
        fn merge_covers_both(a in 0usize..1000, b in 0usize..1000, c in 0usize..1000, d in 0usize..1000) {
            let (s1, s2) = (span(a, b), span(c, d));
            let m = s1.merge(&s2);
            prop_assert!(m.start <= s1.start && m.start <= s2.start);
            prop_assert!(m.end >= s1.end && m.end >= s2.end);
        }

        #[test]
        fn merge_is_commutative(a in 0usize..1000, b in 0usize..1000, c in 0usize..1000, d in 0usize..1000) {
            let (s1, s2) = (span(a, b), span(c, d));
            prop_assert_eq!(s1.merge(&s2), s2.merge(&s1));
        }
    }
}
