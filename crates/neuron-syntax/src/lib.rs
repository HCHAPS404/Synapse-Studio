//! Neuron language syntax: source spans and (soon) lexer, parser and formatter.
//!
//! This crate is a scaffold created in Task 2. The real lexer and parser arrive in
//! Task 11 after the language specification (Task 5) is approved.

/// Version of the Neuron language edition implemented by this crate.
pub const LANGUAGE_EDITION: &str = "2026";

/// A half-open byte range `[start, end)` inside a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn rejects_reversed_span() {
        assert!(Span::new(5, 2).is_none());
    }

    proptest! {
        #[test]
        fn merge_covers_both(a in 0usize..1000, b in 0usize..1000, c in 0usize..1000, d in 0usize..1000) {
            let s1 = Span::new(a.min(b), a.max(b)).unwrap();
            let s2 = Span::new(c.min(d), c.max(d)).unwrap();
            let m = s1.merge(&s2);
            prop_assert!(m.start <= s1.start && m.start <= s2.start);
            prop_assert!(m.end >= s1.end && m.end >= s2.end);
        }

        #[test]
        fn merge_is_commutative(a in 0usize..1000, b in 0usize..1000, c in 0usize..1000, d in 0usize..1000) {
            let s1 = Span::new(a.min(b), a.max(b)).unwrap();
            let s2 = Span::new(c.min(d), c.max(d)).unwrap();
            prop_assert_eq!(s1.merge(&s2), s2.merge(&s1));
        }
    }
}
