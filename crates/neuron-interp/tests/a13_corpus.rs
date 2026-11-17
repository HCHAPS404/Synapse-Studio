//! Task A13: every program of the validation corpus analyzes, runs and passes its tests.
//!
//! The corpus lives in `examples/corpus/<dialect>/`. Each file is analyzed on its own, as
//! a user would open it, then run for two simulated seconds, then its `test` blocks run.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_interp::{run_tests, Interp, Options, TraceKind};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use std::path::PathBuf;

fn corpus_files() -> Vec<(String, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/corpus");
    let mut out = Vec::new();
    let mut dirs: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
            .collect();
        files.sort();
        for f in files {
            let name = format!(
                "{}/{}",
                dir.file_name().unwrap().to_string_lossy(),
                f.file_name().unwrap().to_string_lossy()
            );
            let text = std::fs::read_to_string(&f).unwrap().replace("\r\n", "\n");
            out.push((name, text));
        }
    }
    out
}

#[test]
fn the_corpus_has_at_least_fifty_programs_in_every_dialect() {
    let files = corpus_files();
    assert!(files.len() >= 50, "only {} programs", files.len());
    for dir in ["mcu", "plc", "hdl", "robot", "ui", "host", "core", "cell"] {
        assert!(
            files.iter().any(|(n, _)| n.starts_with(&format!("{dir}/"))),
            "no program for the dialect directory {dir}"
        );
    }
}

#[test]
fn every_corpus_program_analyzes_runs_and_passes_its_tests() {
    let mut failures = Vec::new();
    for (name, text) in corpus_files() {
        let a = analyze(
            vec![SourceFile {
                path: name.clone(),
                text,
            }],
            None,
            &SemaOptions::default(),
        );
        let Some(nir) = a.nir else {
            failures.push(format!("{name}: does not analyze\n{}", a.render()));
            continue;
        };
        let opts = Options {
            end_ns: 2_000_000_000,
            ..Options::default()
        };
        match Interp::new(nir.clone(), opts.clone()) {
            Ok(mut it) => {
                it.run();
                for e in it.trace() {
                    let runtime_fault = e.kind == TraceKind::Fault
                        && matches!(
                            e.name.as_str(),
                            "Overflow" | "DivideByZero" | "ContractViolation" | "LoopBound"
                        );
                    if runtime_fault {
                        failures.push(format!("{name}: runtime fault {}", e.name));
                    }
                }
            }
            Err(e) => failures.push(format!("{name}: cannot run: {e:?}")),
        }
        for r in run_tests(&nir, &opts) {
            if !r.passed {
                failures.push(format!("{name}: test `{}` failed: {}", r.name, r.message));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
