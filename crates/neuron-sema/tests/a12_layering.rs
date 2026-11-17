//! Task A12: the standard library is a stack of layers (spec 12.1, `NE0106`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_sema::{analyze, SemaOptions, SourceFile};

fn render(files: &[(&str, &str)]) -> String {
    let sources = files
        .iter()
        .map(|(path, text)| SourceFile {
            path: (*path).to_string(),
            text: (*text).to_string(),
        })
        .collect();
    analyze(sources, None, &SemaOptions::default()).render()
}

const TIME: &str = "module std.time.timers for core\npub fn zero() -> u8 = 0\n";
const LOGIC: &str = "module std.logic.latches for core\npub fn one() -> u8 = 1\n";

#[test]
fn a_layer_may_import_an_earlier_layer() {
    let out = render(&[
        ("time.neuron", TIME),
        (
            "logic.neuron",
            "module std.logic.edges for core\nuse std.time.timers.{zero}\npub fn f() -> u8 = zero()\n",
        ),
    ]);
    assert!(!out.contains("NE0106"), "{out}");
}

#[test]
fn a_layer_may_import_its_own_layer() {
    let out = render(&[
        ("a.neuron", LOGIC),
        (
            "b.neuron",
            "module std.logic.edges for core\nuse std.logic.latches.{one}\npub fn f() -> u8 = one()\n",
        ),
    ]);
    assert!(!out.contains("NE0106"), "{out}");
}

#[test]
fn a_layer_cannot_import_a_later_layer() {
    let out = render(&[
        ("logic.neuron", LOGIC),
        (
            "time.neuron",
            "module std.time.timers for core\nuse std.logic.latches.{one}\npub fn f() -> u8 = one()\n",
        ),
    ]);
    assert!(out.contains("NE0106"), "{out}");
}

#[test]
fn a_user_module_may_import_any_layer() {
    let out = render(&[
        ("logic.neuron", LOGIC),
        (
            "app.neuron",
            "module app for core\nuse std.logic.latches.{one}\npub fn f() -> u8 = one()\n",
        ),
    ]);
    assert!(!out.contains("NE0106"), "{out}");
}
