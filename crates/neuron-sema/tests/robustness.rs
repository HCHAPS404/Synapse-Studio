//! The analysis is total: it never panics, whatever the input.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;

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
