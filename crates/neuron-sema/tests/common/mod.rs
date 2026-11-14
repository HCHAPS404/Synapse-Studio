#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use neuron_sema::{analyze, Analysis, SemaOptions, SourceFile};

pub fn run_with(src: &str, opts: &SemaOptions) -> Analysis {
    analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: src.into(),
        }],
        None,
        opts,
    )
}

pub fn run(src: &str) -> Analysis {
    run_with(src, &SemaOptions::default())
}

pub fn pkg(files: &[(&str, &str)], manifest: Option<&str>) -> Analysis {
    analyze(
        files
            .iter()
            .map(|(p, t)| SourceFile {
                path: (*p).into(),
                text: (*t).into(),
            })
            .collect(),
        manifest,
        &SemaOptions::default(),
    )
}

/// Error codes in order.
pub fn errors(a: &Analysis) -> Vec<&'static str> {
    a.errors().map(|d| d.diag.code).collect()
}

/// All codes (errors and warnings).
pub fn all_codes(a: &Analysis) -> Vec<&'static str> {
    a.codes()
}

/// Asserts that the program has no errors.
pub fn ok(src: &str) {
    let a = run(src);
    assert!(a.is_ok(), "expected no errors, got:\n{}", a.render());
}

/// Asserts that the errors are exactly `codes`.
pub fn only(src: &str, codes: &[&str]) {
    let a = run(src);
    assert_eq!(errors(&a), codes, "unexpected diagnostics:\n{}", a.render());
}

/// Asserts that `code` is among the errors.
pub fn has(src: &str, code: &str) {
    let a = run(src);
    assert!(
        errors(&a).contains(&code),
        "expected {code}, got:\n{}",
        a.render()
    );
}

pub fn has_with(src: &str, opts: &SemaOptions, code: &str) {
    let a = run_with(src, opts);
    assert!(
        errors(&a).contains(&code),
        "expected {code}, got:\n{}",
        a.render()
    );
}
