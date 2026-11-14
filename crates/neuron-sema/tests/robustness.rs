//! The analysis is total: it never panics, whatever the input.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn examples() -> Vec<(String, String)> {
    let dir = root().join("examples");
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    v.sort();
    v.into_iter()
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect()
}

/// Code blocks of the specification that start a module or a cell.
fn spec_programs() -> Vec<String> {
    let dir = root().join("spec/language");
    let mut out = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)
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

#[test]
fn every_spec_program_can_be_analyzed() {
    let progs = spec_programs();
    assert!(progs.len() >= 10, "found {} programs", progs.len());
    for p in &progs {
        let a = run(p);
        // Rendering exercises every span.
        let _ = a.render();
    }
}

#[test]
fn every_example_can_be_analyzed_alone() {
    for (name, text) in examples() {
        let a = analyze(
            vec![SourceFile { path: name, text }],
            None,
            &SemaOptions {
                cpu_hz: Some(16_000_000),
                ram_bytes: Some(2048),
                ..SemaOptions::default()
            },
        );
        let _ = a.render();
    }
}

fn mutate(text: &str, kind: u8, at: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let i = at % lines.len();
    match kind % 4 {
        0 => lines
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, l)| format!("{l}\n"))
            .collect(),
        1 => lines
            .iter()
            .enumerate()
            .flat_map(|(j, l)| {
                if j == i {
                    vec![format!("{l}\n"), format!("{l}\n")]
                } else {
                    vec![format!("{l}\n")]
                }
            })
            .collect(),
        2 => {
            let mut cut = (at * 7) % (text.len() + 1);
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text[..cut].to_string()
        }
        _ => text.replace(lines[i].split_whitespace().next().unwrap_or("x"), "zz"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Mutated programs (lines deleted, duplicated, truncated, renamed) never crash the analysis.
    #[test]
    fn mutated_examples_never_panic(idx in 0usize..16, kind in 0u8..4, at in 0usize..4000, cpu in proptest::bool::ANY) {
        let ex = examples();
        let (name, text) = &ex[idx % ex.len()];
        let src = mutate(text, kind, at);
        let opts = SemaOptions {
            cpu_hz: if cpu { Some(8_000_000) } else { None },
            ram_bytes: if cpu { Some(1024) } else { None },
            ..SemaOptions::default()
        };
        let a = analyze(vec![SourceFile { path: name.clone(), text: src }], None, &opts);
        let _ = a.render();
    }

    /// Arbitrary text never crashes the analysis either.
    #[test]
    fn arbitrary_text_never_panics(s in "[ -~\n]{0,300}") {
        let a = run(&format!("module t for mcu\n{s}"));
        let _ = a.render();
    }
}
