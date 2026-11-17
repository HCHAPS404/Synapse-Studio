//! Task A11: the canonical formatter (spec chapter 13).
//!
//! Properties: P-FMT-1 (idempotence), P-FMT-2 (same tokens), P-FMT-3 (same structure),
//! P-FMT-4 (comments kept) and P-FMT-5 (totality), over the examples and every program
//! of the specification.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_syntax::format::{comment_count, format_source, signature};
use neuron_syntax::lexer::lex;
use neuron_syntax::outline::outline;
use neuron_syntax::{parse, TokenKind};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn examples() -> Vec<String> {
    let mut v: Vec<_> = std::fs::read_dir(root().join("examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    v.sort();
    v.into_iter()
        .map(|p| std::fs::read_to_string(p).unwrap().replace("\r\n", "\n"))
        .collect()
}

fn spec_programs() -> Vec<String> {
    let mut out = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(root().join("spec/language"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let mut cur: Option<String> = None;
        for line in text.lines() {
            if line.trim() == "```neuron" {
                cur = Some(String::new());
            } else if line.trim() == "```" {
                if let Some(b) = cur.take() {
                    if b.starts_with("module") || b.starts_with("cell") {
                        out.push(b);
                    }
                }
            } else if let Some(b) = cur.as_mut() {
                b.push_str(line);
                b.push('\n');
            }
        }
    }
    out
}

fn all_programs() -> Vec<String> {
    let mut v = examples();
    v.extend(spec_programs());
    v
}

/// Structure of a program without the line numbers of the outline.
fn structure(src: &str) -> String {
    let p = parse(src);
    outline(&p.file, src)
        .lines()
        .map(|l| {
            let t = l.trim_start();
            match t.split_once(' ') {
                Some((n, rest)) if n.chars().all(|c| c.is_ascii_digit()) => rest.to_string(),
                _ => t.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_examples_and_the_spec_programs_format_without_falling_back() {
    let progs = all_programs();
    assert!(progs.len() >= 15, "found {} programs", progs.len());
    for (i, p) in progs.iter().enumerate() {
        let out = format_source(p).unwrap_or_else(|e| panic!("program {i} does not parse: {e:?}"));
        assert!(!out.fell_back, "program {i} failed the self-check:\n{p}");
    }
}

#[test]
fn p_fmt_1_formatting_is_idempotent() {
    for (i, p) in all_programs().iter().enumerate() {
        let once = format_source(p).unwrap();
        let twice = format_source(&once.text).unwrap();
        assert_eq!(once.text, twice.text, "program {i}");
        assert!(!twice.fell_back);
    }
}

#[test]
fn p_fmt_2_and_3_tokens_and_structure_are_kept() {
    for (i, p) in all_programs().iter().enumerate() {
        let out = format_source(p).unwrap().text;
        assert_eq!(signature(p), signature(&out), "program {i}: tokens");
        assert_eq!(structure(p), structure(&out), "program {i}: structure");
    }
}

#[test]
fn p_fmt_4_every_comment_is_kept_once() {
    for (i, p) in all_programs().iter().enumerate() {
        let out = format_source(p).unwrap().text;
        assert_eq!(comment_count(p), comment_count(&out), "program {i}");
    }
}

#[test]
fn p_fmt_5_sources_with_errors_are_not_changed() {
    let bad = "module t\nfn f(:\n";
    assert!(format_source(bad).is_err());
    assert!(format_source("module t\n\tfn f():\n  pass\n").is_err());
}

#[test]
fn the_rules_of_chapter_13() {
    let f = |s: &str| format_source(s).unwrap().text;
    // white space, indentation, trailing spaces, final newline
    assert_eq!(
        f("module m for mcu   \nio led:   gpio.output   @ D13\ntask t every  500 ms:\n    led.toggle( )   \n\n\n"),
        "module m for mcu\nio led: gpio.output @ D13\ntask t every 500 ms:\n  led.toggle()\n"
    );
    // CRLF and a byte order mark
    assert_eq!(
        f("\u{feff}module m\r\nfn f():\r\n  pass\r\n"),
        "module m\nfn f():\n  pass\n"
    );
    // `@` of consecutive io bindings is aligned
    assert_eq!(
        f("module m for mcu\nio a: gpio.output @ D1\nio longer_name: gpio.input @ D2\n"),
        format!(
            "module m for mcu\nio a: gpio.output{}@ D1\nio longer_name: gpio.input @ D2\n",
            " ".repeat(10)
        )
    );
    // a blank line after a block item, none after a header, at most one elsewhere
    assert_eq!(
        f("module m for mcu\ntask t every 1 ms:\n\n  pass\ntask u every 2 ms:\n  pass\nvar x: u8 = 0\n\n\nvar y: u8 = 0\n"),
        "module m for mcu\ntask t every 1 ms:\n  pass\n\ntask u every 2 ms:\n  pass\n\nvar x: u8 = 0\n\nvar y: u8 = 0\n"
    );
    // comments: `#` followed by one space, trailing comments two spaces away
    assert_eq!(
        f("module m\n#note\nvar x: u8 = 0 # trailing\n##doc\nfn f():\n  pass\n"),
        "module m\n# note\nvar x: u8 = 0  # trailing\n## doc\nfn f():\n  pass\n"
    );
    // commas, brackets, operators
    assert_eq!(
        f("module m\nfn f(a:u8,b : u8)->u8:\n  let x=(a ,b)\n  return a+b\n"),
        "module m\nfn f(a: u8, b: u8) -> u8:\n  let x = (a, b)\n  return a+b\n"
    );
    // calls keep their adjacency: `f(x)` is a call, `f (x)` is not
    assert_eq!(
        f("module m for mcu\ntask t on ev  (prio 1):\n  pass\n"),
        "module m for mcu\ntask t on ev (prio 1):\n  pass\n"
    );
    // lines inside brackets are indented one level
    assert_eq!(
        f("module m\nfn f() -> u8:\n  let x = max(\n        1,\n   2,\n  )\n  return 0\n"),
        "module m\nfn f() -> u8:\n  let x = max(\n    1,\n    2,\n  )\n  return 0\n"
    );
}

#[test]
fn a_cell_keeps_its_items_one_level_in() {
    let src = "cell C\n  device a: generic_s7_1200 @ usb\n  sequence S on a:\n    initial step I:\n      until a.x\n  device b: generic_scara @ usb\n";
    let out = format_source(src).unwrap();
    assert!(!out.fell_back);
    assert!(
        out.text
            .contains("  sequence S on a:\n    initial step I:\n      until a.x\n\n  device b:"),
        "{}",
        out.text
    );
}

/// Widens the single spaces between tokens, adds trailing spaces and uses CRLF.
fn messy(src: &str, seed: u64) -> String {
    let lexed = lex(src);
    let mut out = String::new();
    let mut state = seed | 1;
    let mut at_line_start = true;
    let mut prev_number = false;
    for t in &lexed.tokens {
        let text = t.text(src);
        match t.kind {
            TokenKind::Whitespace if text == " " && !at_line_start && !prev_number => {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                out.push_str(&" ".repeat(1 + (state % 3) as usize));
                continue;
            }
            TokenKind::Newline => {
                out.push_str("   \r\n");
                at_line_start = true;
                prev_number = false;
                continue;
            }
            _ => {}
        }
        if text.contains('\n') {
            at_line_start = true;
        } else if !text.is_empty() && !matches!(t.kind, TokenKind::Whitespace) {
            at_line_start = false;
        }
        // A unit may follow a number or a closing parenthesis (`xyz(1, 2, 3) mm`)
        // and must stay exactly one space away (NE0014).
        prev_number = matches!(t.kind, TokenKind::Int | TokenKind::Real) || text == ")";
        out.push_str(text);
    }
    out
}

#[test]
fn messy_layout_formats_like_the_original() {
    for (i, p) in all_programs().iter().enumerate() {
        let canon = format_source(p).unwrap().text;
        for seed in 1..=3u64 {
            let m = messy(p, seed);
            let out =
                format_source(&m).unwrap_or_else(|e| panic!("program {i} seed {seed}: {e:?}\n{m}"));
            assert!(!out.fell_back, "program {i} seed {seed}");
            assert_eq!(out.text, canon, "program {i} seed {seed}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// P-FMT-5: arbitrary text never panics, and when it formats, formatting is stable.
    #[test]
    fn arbitrary_text_is_handled(s in "[ -~\n]{0,200}") {
        let src = format!("module t for mcu\n{s}");
        if let Ok(once) = format_source(&src) {
            let twice = format_source(&once.text).unwrap();
            prop_assert_eq!(once.text, twice.text);
        }
    }
}
