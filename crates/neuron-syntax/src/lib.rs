//! Neuron language syntax: spans, diagnostics, lexer, AST and parser.
//!
//! Specification: `spec/language/` and `spec/grammar/neuron.ebnf`.

#[allow(clippy::large_enum_variant)]
pub mod ast;
pub mod diag;
pub mod format;
pub mod lexer;
pub mod line_index;
pub mod outline;
pub mod parser;
pub mod span;
pub mod token;

pub use diag::{Diagnostic, Severity};
pub use lexer::{lex, Lexed};
pub use line_index::LineIndex;
pub use parser::{parse, Parsed};
pub use span::Span;
pub use token::{Keyword, Punct, Token, TokenKind};

/// Version of the Neuron language edition implemented by this crate.
pub const LANGUAGE_EDITION: &str = "2026";
