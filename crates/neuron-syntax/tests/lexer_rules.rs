//! Lexical rules clarified in spec v0.2 (numbers, string interpolation).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_syntax::lexer::lex;

fn codes(src: &str) -> Vec<&'static str> {
    lex(src).diagnostics.iter().map(|d| d.code).collect()
}

fn help(src: &str) -> Option<String> {
    lex(src).diagnostics.first().and_then(|d| d.help.clone())
}

#[test]
fn leading_dot_real_is_an_error_with_a_fix() {
    assert_eq!(codes("x = .5"), vec!["NE0006"]);
    assert_eq!(help("x = .5").as_deref(), Some("write `0.5`"));
}

#[test]
fn trailing_dot_real_is_an_error_with_a_fix() {
    assert_eq!(codes("x = 5.\n"), vec!["NE0006"]);
    assert_eq!(help("x = 5.\n").as_deref(), Some("write `5.0`"));
    assert_eq!(codes("f(5.)"), vec!["NE0006"]);
}

#[test]
fn tuple_fields_ranges_and_methods_are_not_misread() {
    for ok in ["t.0.1", "0..8", "1..=9", "x[0].y", "3.14", "a.5"] {
        assert!(codes(ok).is_empty(), "{ok}");
    }
}

#[test]
fn misplaced_underscores_are_rejected() {
    for bad in ["1__0", "1_000_", "0x_FF", "0b_1", "8'b_1"] {
        assert_eq!(codes(bad), vec!["NE0006"], "{bad}");
    }
    for ok in ["1_000", "0xFF_FF", "0b1010_0000", "8'b1010_0000"] {
        assert!(codes(ok).is_empty(), "{ok}");
    }
}

#[test]
fn string_interpolation_is_validated() {
    assert!(codes(r#""T={t:.1} degC""#).is_empty());
    assert!(codes(r#""literal \{ and \}""#).is_empty());
    assert_eq!(codes(r#""open {x""#), vec!["NE0016"]);
    assert_eq!(codes(r#""stray }""#), vec!["NE0016"]);
    assert!(codes(r#""a {b {c}}""#).contains(&"NE0016"));
}
