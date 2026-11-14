//! Smaller behaviors of the public API.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

#[test]
fn a_byte_order_mark_is_ignored() {
    let a = run("\u{feff}module t\nfn f() -> u8:\n  return 1\n");
    assert!(a.is_ok(), "{}", a.render());
}

#[test]
fn diagnostics_are_sorted_and_deterministic() {
    let src = "module t for mcu\nio led: gpio.output @ D13\nfn f(a: f32 mm, b: f32 s) -> f32 mm:\n  return a + b\ntask x every 1500 us:\n  led.toggle()\n";
    let a = run(src);
    let b = run(src);
    assert_eq!(a.render(), b.render());
    let starts: Vec<usize> = a.diagnostics.iter().map(|d| d.diag.span.start).collect();
    let mut sorted = starts.clone();
    sorted.sort_unstable();
    assert_eq!(starts, sorted);
}

#[test]
fn rendering_includes_codes_paths_and_help() {
    let a = run("module t\nfn f(a: f32 mm, b: f32 s) -> f32 mm:\n  return a + b\n");
    let text = a.render();
    assert!(text.contains("error[NE0201]"), "{text}");
    assert!(text.contains("t.neuron:3:"), "{text}");
    assert!(text.contains("help:"), "{text}");
}

#[test]
fn deep_unit_exponents_do_not_hang() {
    let a = run("module t\nfn f() -> u8:\n  let x = 5 m^2000000000\n  return 1\n");
    // Whatever the verdict, the analysis terminates and reports something sensible.
    let _ = a.render();
}
