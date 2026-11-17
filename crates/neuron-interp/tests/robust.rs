//! The interpreter is total: whatever valid program it is given, it neither panics nor
//! hangs. Programs that the analysis rejects are never run.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_interp::{Interp, Options};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn examples() -> Vec<(String, String)> {
    let mut v: Vec<_> = std::fs::read_dir(root().join("examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    v.sort();
    v.into_iter()
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap().replace("\r\n", "\n"),
            )
        })
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

/// Runs a program for `ms` of simulated time. Returns `false` when it was not valid.
fn try_run(files: Vec<(String, String)>, manifest: Option<&str>, ms: i64) -> bool {
    let a = analyze(
        files
            .into_iter()
            .map(|(path, text)| SourceFile { path, text })
            .collect(),
        manifest,
        &SemaOptions::default(),
    );
    let Some(nir) = a.nir else { return false };
    let opts = Options {
        end_ns: ms * 1_000_000,
        ..Options::default()
    };
    if let Ok(mut it) = Interp::new(nir, opts) {
        it.run();
        let _ = neuron_interp::render(it.trace());
    }
    true
}

#[test]
fn every_valid_example_runs() {
    let mut ran = 0;
    for (name, text) in examples() {
        if try_run(vec![(name, text)], None, 500) {
            ran += 1;
        }
    }
    assert!(ran >= 5, "only {ran} examples were valid");
}

#[test]
fn the_examples_package_runs_as_a_whole() {
    let manifest = std::fs::read_to_string(root().join("examples/neuron.toml")).unwrap();
    assert!(try_run(examples(), Some(&manifest), 1_000));
}

#[test]
fn every_valid_spec_program_runs() {
    let mut ran = 0;
    for p in spec_programs() {
        if try_run(vec![("t.neuron".to_string(), p)], None, 300) {
            ran += 1;
        }
    }
    assert!(ran >= 8, "only {ran} spec programs were valid");
}

fn mutate(text: &str, kind: u8, at: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let i = at % lines.len();
    match kind % 3 {
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
        _ => text.replace(lines[i].split_whitespace().next().unwrap_or("x"), "zz"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(60))]

    /// Variations of the examples that still analyze without errors run without panics.
    #[test]
    fn mutated_examples_run_without_panics(idx in 0usize..16, kind in 0u8..3, at in 0usize..4000) {
        let ex = examples();
        let (name, text) = &ex[idx % ex.len()];
        let _ = try_run(vec![(name.clone(), mutate(text, kind, at))], None, 200);
    }
}
