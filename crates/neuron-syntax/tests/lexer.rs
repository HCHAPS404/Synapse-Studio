//! Lexer tests (spec/language/02-lexical.md), including properties LEX-1 and LEX-2.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_syntax::lexer::lex;
use neuron_syntax::token::{Keyword, Punct, TokenKind as K};
use proptest::prelude::*;

/// Significant kinds and their text, without layout.
fn atoms(src: &str) -> Vec<(K, String)> {
    let out = lex(src);
    out.significant()
        .filter(|t| !t.kind.is_layout())
        .map(|t| (t.kind, t.text(src).to_string()))
        .collect()
}

fn kinds(src: &str) -> Vec<K> {
    lex(src).significant().map(|t| t.kind).collect()
}

fn codes(src: &str) -> Vec<&'static str> {
    lex(src).diagnostics.iter().map(|d| d.code).collect()
}

// ---- literals -----------------------------------------------------------

#[test]
fn integers_in_all_bases() {
    for s in ["42", "1_000_000", "0x2A", "0xFF_FF", "0b1010_0000", "0o755"] {
        assert_eq!(atoms(s), vec![(K::Int, s.to_string())], "{s}");
    }
}

#[test]
fn reals() {
    for s in ["3.14", "0.5", "1e-3", "2.5e6", "6.022e23"] {
        assert_eq!(atoms(s), vec![(K::Real, s.to_string())], "{s}");
    }
}

#[test]
fn range_is_not_a_real() {
    assert_eq!(
        atoms("0..8"),
        vec![
            (K::Int, "0".into()),
            (K::Punct(Punct::DotDot), "..".into()),
            (K::Int, "8".into())
        ]
    );
    assert_eq!(atoms("1..=9")[1].0, K::Punct(Punct::DotDotEq));
}

#[test]
fn tuple_field_after_dot_is_integer() {
    let a = atoms("t.0.1");
    assert_eq!(a.iter().filter(|(k, _)| *k == K::Int).count(), 2);
    assert!(a.iter().all(|(k, _)| *k != K::Real));
}

#[test]
fn quantities_are_number_then_unit() {
    assert_eq!(
        atoms("9.81 m/s^2"),
        vec![
            (K::Real, "9.81".into()),
            (K::Ident, "m".into()),
            (K::Punct(Punct::Slash), "/".into()),
            (K::Ident, "s".into()),
            (K::Punct(Punct::Caret), "^".into()),
            (K::Int, "2".into()),
        ]
    );
    assert_eq!(atoms("50 %")[1].0, K::Punct(Punct::Percent));
    assert_eq!(atoms("5 \u{b5}s")[1], (K::Ident, "\u{b5}s".into()));
}

#[test]
fn number_glued_to_unit_is_an_error_with_fix() {
    let out = lex("let t = 5ms");
    assert_eq!(out.diagnostics.len(), 1);
    assert_eq!(out.diagnostics[0].code, "NE0006");
    assert_eq!(
        out.diagnostics[0].help.as_deref(),
        Some("separate the number from its unit: `5 ms`")
    );
}

#[test]
fn bit_vectors() {
    for s in ["8'b1010_0000", "16'hBEEF", "4'd9", "10'b11_1111_1111"] {
        assert_eq!(atoms(s), vec![(K::BitsLit, s.to_string())], "{s}");
    }
    assert_eq!(codes("4'b"), vec!["NE0006"]);
}

#[test]
fn iec_addresses() {
    for s in ["%I0.0", "%Q0.1", "%IW64", "%MD10", "%QX1.7", "%DB1.DBX0.0"] {
        assert_eq!(atoms(s), vec![(K::IecAddr, s.to_string())], "{s}");
    }
    // modulo, not an address
    assert_eq!(atoms("a % b")[1].0, K::Punct(Punct::Percent));
    assert_eq!(atoms("x %Mx")[1].0, K::Punct(Punct::Percent));
}

#[test]
fn strings_and_escapes() {
    assert_eq!(atoms(r#""Temp: {temp:.1} degC\n""#)[0].0, K::Str);
    assert_eq!(atoms(r#"r"C:\raw\path""#)[0].0, K::RawStr);
    assert_eq!(atoms(r#"b"\x01\x02""#)[0].0, K::ByteStr);
    assert_eq!(atoms(r"'A'")[0].0, K::Char);
    assert_eq!(atoms(r"'\''")[0].0, K::Char);
    assert_eq!(codes(r#""bad \q""#), vec!["NE0007"]);
    assert_eq!(codes("\"open\nx"), vec!["NE0004"]);
    assert_eq!(codes(r#"b"\u{e9}t\u{e9}""#), Vec::<&str>::new());
    assert_eq!(codes("b\"\u{e9}\""), vec!["NE0008"]);
}

// ---- words and operators -------------------------------------------------

#[test]
fn reserved_and_contextual_keywords() {
    assert_eq!(kinds("task")[0], K::Keyword(Keyword::Task));
    assert_eq!(kinds("Self")[0], K::Keyword(Keyword::SelfType));
    assert_eq!(kinds("none")[0], K::Keyword(Keyword::NoneLit));
    // contextual keywords are identifiers for the lexer
    assert_eq!(kinds("step")[0], K::Ident);
    assert_eq!(atoms("`step`"), vec![(K::Ident, "`step`".into())]);
    assert_eq!(codes("`step"), vec!["NE0003"]);
}

#[test]
fn every_keyword_round_trips() {
    for k in Keyword::ALL {
        assert_eq!(Keyword::lookup(k.as_str()), Some(*k));
        assert_eq!(kinds(k.as_str())[0], K::Keyword(*k));
    }
}

#[test]
fn longest_operator_wins() {
    let ops = [
        "<<=", ">>=", "..=", "**", "+%", "-%", "*%", "+|", "-|", "*|", "|>", "->", "=>", "==",
        "!=", "<=", ">=", "<<", ">>", "..", "++", "??", "+=", "-=",
    ];
    for op in ops {
        let a = atoms(&format!("a {op} b"));
        assert_eq!(a.len(), 3, "{op}");
        assert_eq!(a[1].1, op);
    }
}

#[test]
fn bang_alone_suggests_not() {
    let out = lex("if !x: pass");
    assert_eq!(out.diagnostics[0].code, "NE0003");
    assert!(out.diagnostics[0].help.as_deref().unwrap().contains("not"));
}

// ---- comments -----------------------------------------------------------

#[test]
fn comments_are_trivia() {
    let src = "# line\n## doc\nx #[ block #[ nested ]# still ]# y\n";
    let out = lex(src);
    assert!(out.diagnostics.is_empty());
    let k: Vec<K> = out.tokens.iter().map(|t| t.kind).collect();
    assert!(k.contains(&K::LineComment));
    assert!(k.contains(&K::DocComment));
    assert_eq!(k.iter().filter(|k| **k == K::BlockComment).count(), 1);
    assert_eq!(
        atoms(src),
        vec![(K::Ident, "x".into()), (K::Ident, "y".into())]
    );
    assert_eq!(codes("#[ open"), vec!["NE0004"]);
}

// ---- layout -------------------------------------------------------------

#[test]
fn indentation_produces_blocks() {
    let src = "task t every 10 ms:\n  if x:\n    a()\n  b()\nc()\n";
    let k = kinds(src);
    let ind = k.iter().filter(|k| **k == K::Indent).count();
    let ded = k.iter().filter(|k| **k == K::Dedent).count();
    assert_eq!((ind, ded), (2, 2));
    assert_eq!(k.last(), Some(&K::Eof));
}

#[test]
fn blank_and_comment_lines_do_not_change_indentation() {
    let src = "a:\n  b\n\n    # comment deeper\n  c\n";
    let k = kinds(src);
    assert_eq!(k.iter().filter(|k| **k == K::Indent).count(), 1);
    assert!(lex(src).diagnostics.is_empty());
}

#[test]
fn newlines_inside_brackets_continue_the_line() {
    let src = "f(a,\n    b,\n  c)\nnext\n";
    let k = kinds(src);
    assert_eq!(k.iter().filter(|k| **k == K::Newline).count(), 2);
    assert_eq!(k.iter().filter(|k| **k == K::Indent).count(), 0);
}

#[test]
fn crlf_is_a_single_newline() {
    let k = kinds("a\r\nb\r\n");
    assert_eq!(k.iter().filter(|k| **k == K::Newline).count(), 2);
}

#[test]
fn missing_final_newline_is_synthesized() {
    let k = kinds("a:\n  b");
    assert_eq!(
        k,
        vec![
            K::Ident,
            K::Punct(Punct::Colon),
            K::Newline,
            K::Indent,
            K::Ident,
            K::Newline,
            K::Dedent,
            K::Eof
        ]
    );
}

#[test]
fn tab_indentation_is_ne0001() {
    assert_eq!(codes("a:\n\tb\n"), vec!["NE0001"]);
    // a tab on a blank line is fine
    assert!(codes("a\n\t\nb\n").is_empty());
}

#[test]
fn bad_dedent_is_ne0002() {
    assert_eq!(codes("a:\n    b\n  c\n"), vec!["NE0002"]);
}

// ---- the specification itself lexes cleanly ----------------------------

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn example_programs_lex_without_errors() {
    let dir = repo_root().join("examples");
    let mut n = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "neuron") {
            let src = std::fs::read_to_string(&path).unwrap();
            let out = lex(&src);
            assert!(
                out.diagnostics.is_empty(),
                "{}: {:?}",
                path.display(),
                out.diagnostics
            );
            n += 1;
        }
    }
    assert!(n >= 2);
}

#[test]
fn spec_code_blocks_lex_without_errors() {
    let dir = repo_root().join("spec/language");
    let mut blocks = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let md = std::fs::read_to_string(&path).unwrap();
        let mut in_block = false;
        let mut code = String::new();
        for line in md.lines() {
            if !in_block && line.trim_start() == "```neuron" {
                in_block = true;
                code.clear();
            } else if in_block && line.trim_start() == "```" {
                in_block = false;
                blocks += 1;
                let out = lex(&code);
                assert!(
                    out.diagnostics.is_empty(),
                    "{}:\n{code}\n{:?}",
                    path.display(),
                    out.diagnostics
                );
            } else if in_block {
                code.push_str(line);
                code.push('\n');
            }
        }
    }
    assert!(blocks >= 20, "only {blocks} neuron blocks found");
}

// ---- properties ---------------------------------------------------------

const FRAGMENTS: &[&str] = &[
    "module",
    "task",
    "step",
    "x",
    "led",
    " ",
    "  ",
    "    ",
    "\t",
    "\n",
    "\r\n",
    "\r",
    ":",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "#",
    "## doc",
    "#[",
    "]#",
    "\"",
    "'",
    "`",
    "\\",
    "5",
    "3.14",
    "0x",
    "0b1",
    "8'b10",
    "4'",
    "%",
    "%I0.0",
    "%DB1.DBX0.0",
    "->",
    "|>",
    "..",
    ".",
    "+%",
    "*",
    "=",
    "!",
    "\u{b5}s",
    "\u{e9}",
    "\u{1f600}",
    "ms",
    "e",
    "1e",
    "r\"",
    "b\"",
    "_",
];

fn source() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(FRAGMENTS), 0..80).prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// LEX-1: the stream covers the source contiguously and reproduces it exactly.
    #[test]
    fn lex1_lossless(src in source()) {
        let out = lex(&src);
        let mut pos = 0;
        let mut rebuilt = String::new();
        for t in &out.tokens {
            prop_assert_eq!(t.span.start, pos, "gap or overlap at {}", pos);
            prop_assert!(src.is_char_boundary(t.span.start) && src.is_char_boundary(t.span.end));
            rebuilt.push_str(t.text(&src));
            pos = t.span.end;
        }
        prop_assert_eq!(pos, src.len());
        prop_assert_eq!(rebuilt, src);
        prop_assert_eq!(out.tokens.last().map(|t| t.kind), Some(K::Eof));
    }

    /// LEX-2: indents and dedents balance and never go negative.
    #[test]
    fn lex2_indentation_balance(src in source()) {
        let out = lex(&src);
        let mut depth: i64 = 0;
        for t in &out.tokens {
            match t.kind {
                K::Indent => depth += 1,
                K::Dedent => depth -= 1,
                _ => {}
            }
            prop_assert!(depth >= 0);
        }
        prop_assert_eq!(depth, 0);
    }

    /// Arbitrary Unicode input never panics and stays lossless.
    #[test]
    fn arbitrary_text_is_lossless(src in "\\PC{0,200}") {
        let out = lex(&src);
        let rebuilt: String = out.tokens.iter().map(|t| t.text(&src)).collect();
        prop_assert_eq!(rebuilt, src);
    }
}
