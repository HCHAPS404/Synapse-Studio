//! Task A12: the library written in Neuron behaves like the interpreter's native blocks.
//!
//! Properties: STD-DIFF (a library class and the native block give the same outputs for
//! the same inputs), STD-PID-1 (the output of `Pid` stays inside its limits) and
//! STD-PID-2 (`Pid` does not integrate while its output saturates).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_interp::{run_tests, Options};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;

const LIB: &[(&str, &str)] = &[
    (
        "lib/core/math.neuron",
        include_str!("../../../lib/core/math.neuron"),
    ),
    (
        "lib/core/bits.neuron",
        include_str!("../../../lib/core/bits.neuron"),
    ),
    (
        "lib/units/constants.neuron",
        include_str!("../../../lib/units/constants.neuron"),
    ),
    (
        "lib/time/timers.neuron",
        include_str!("../../../lib/time/timers.neuron"),
    ),
    (
        "lib/time/clock.neuron",
        include_str!("../../../lib/time/clock.neuron"),
    ),
    (
        "lib/logic/edges.neuron",
        include_str!("../../../lib/logic/edges.neuron"),
    ),
    (
        "lib/logic/latches.neuron",
        include_str!("../../../lib/logic/latches.neuron"),
    ),
    (
        "lib/logic/counters.neuron",
        include_str!("../../../lib/logic/counters.neuron"),
    ),
    (
        "lib/control/pid.neuron",
        include_str!("../../../lib/control/pid.neuron"),
    ),
    (
        "lib/control/filters.neuron",
        include_str!("../../../lib/control/filters.neuron"),
    ),
    (
        "lib/control/hysteresis.neuron",
        include_str!("../../../lib/control/hysteresis.neuron"),
    ),
    (
        "lib/control/ramp.neuron",
        include_str!("../../../lib/control/ramp.neuron"),
    ),
    (
        "lib/test/asserts.neuron",
        include_str!("../../../lib/test/asserts.neuron"),
    ),
];

const HEADER: &str = "module diff for host\n\
use std.time.timers.{Ton, Tof, Tp}\n\
use std.logic.edges.{RTrig, FTrig}\n\
use std.logic.latches.{Sr, Rs}\n\
use std.logic.counters.{Ctu, Ctd}\n\
use std.control.pid.{Pid}\n\
use std.control.filters.{Lowpass, Highpass, MovingAverage}\n\
use std.control.hysteresis.{Hysteresis, Debounce}\n\
use std.control.ramp.{Ramp}\n\
use std.test.asserts.{close}\n\n";

/// Analyzes the library together with one generated module and returns the reports of the
/// tests of that module (their names start with `diff`).
fn run_diff(body: &str) -> Vec<(String, bool, String)> {
    let mut files: Vec<SourceFile> = LIB
        .iter()
        .map(|(p, t)| SourceFile {
            path: (*p).to_string(),
            text: t.replace("\r\n", "\n"),
        })
        .collect();
    files.push(SourceFile {
        path: "diff.neuron".into(),
        text: format!("{HEADER}{body}"),
    });
    let a = analyze(files, None, &SemaOptions::default());
    let nir = match a.nir {
        Some(n) => n,
        None => panic!("analysis errors:\n{}\n{body}", a.render()),
    };
    run_tests(&nir, &Options::default())
        .into_iter()
        .filter(|r| r.name.starts_with("diff"))
        .map(|r| (r.name, r.passed, r.message))
        .collect()
}

fn assert_all_pass(body: &str) {
    let reports = run_diff(body);
    assert!(!reports.is_empty(), "no diff tests ran for:\n{body}");
    for (name, passed, message) in reports {
        assert!(passed, "{name}: {message}\n{body}");
    }
}

fn bools(v: &[bool]) -> String {
    let items: Vec<&str> = v
        .iter()
        .map(|b| if *b { "true" } else { "false" })
        .collect();
    format!("[{}]", items.join(", "))
}

fn reals(v: &[i32]) -> String {
    let items: Vec<String> = v.iter().map(|x| format!("{x}.0")).collect();
    format!("[{}]", items.join(", "))
}

fn bit_inputs() -> impl Strategy<Value = Vec<bool>> {
    prop::collection::vec(any::<bool>(), 1..40)
}

fn real_inputs() -> impl Strategy<Value = Vec<i32>> {
    prop::collection::vec(-40i32..40, 1..40)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// STD-DIFF for the timers: `Ton`, `Tof` and `Tp` against `ton`, `tof` and `tp`.
    #[test]
    fn timers_match_the_native_blocks(xs in bit_inputs(), preset in 1u32..6) {
        let n = xs.len();
        let body = format!(
            "test \"diff timers\":\n  let xs = {xs}\n  var a = ton(preset: {preset} ms)\n  var b = Ton(preset: {preset} ms)\n  var c = tof(preset: {preset} ms)\n  var d = Tof(preset: {preset} ms)\n  var e = tp(preset: {preset} ms)\n  var f = Tp(preset: {preset} ms)\n  for i in 0..{n}:\n    assert a.step(xs[i]) == b.step(xs[i], 1 ms), \"ton differs\"\n    assert c.step(xs[i]) == d.step(xs[i], 1 ms), \"tof differs\"\n    assert e.step(xs[i]) == f.step(xs[i], 1 ms), \"tp differs\"\n",
            xs = bools(&xs)
        );
        assert_all_pass(&body);
    }

    /// STD-DIFF for edges, latches and counters.
    #[test]
    fn logic_matches_the_native_blocks(xs in bit_inputs(), ys in bit_inputs(), preset in 1i32..5) {
        let n = xs.len().min(ys.len());
        let body = format!(
            "test \"diff logic\":\n  let xs = {xs}\n  let ys = {ys}\n  var r1 = r_trig()\n  var r2 = RTrig()\n  var f1 = f_trig()\n  var f2 = FTrig()\n  var s1 = sr()\n  var s2 = Sr()\n  var t1 = rs()\n  var t2 = Rs()\n  var u1 = ctu(preset: {preset})\n  var u2 = Ctu(preset: {preset})\n  var d1 = ctd(preset: {preset})\n  var d2 = Ctd(preset: {preset})\n  for i in 0..{n}:\n    assert r1.step(xs[i]) == r2.step(xs[i]), \"r_trig differs\"\n    assert f1.step(xs[i]) == f2.step(xs[i]), \"f_trig differs\"\n    assert s1.step(xs[i], ys[i]) == s2.step(xs[i], ys[i]), \"sr differs\"\n    assert t1.step(xs[i], ys[i]) == t2.step(xs[i], ys[i]), \"rs differs\"\n    assert u1.step(xs[i]) == u2.step(xs[i], false), \"ctu differs\"\n    assert d1.step(xs[i]) == d2.step(xs[i], false), \"ctd differs\"\n",
            xs = bools(&xs),
            ys = bools(&ys),
        );
        assert_all_pass(&body);
    }

    /// STD-DIFF for hysteresis and debounce.
    #[test]
    fn switching_blocks_match_the_native_blocks(xs in real_inputs(), bs in bit_inputs(), time in 1u32..5) {
        let n = xs.len();
        let m = bs.len();
        let body = format!(
            "test \"diff hysteresis\":\n  let xs = {xs}\n  var a = hysteresis(low: -10.0, high: 10.0)\n  var b = Hysteresis(low: -10.0, high: 10.0)\n  for i in 0..{n}:\n    assert a.step(xs[i]) == b.step(xs[i]), \"hysteresis differs\"\n\ntest \"diff debounce\":\n  let bs = {bs}\n  var a = debounce(time: {time} ms)\n  var b = Debounce(time: {time} ms)\n  for i in 0..{m}:\n    advance(1 ms)\n    assert a.step(bs[i]) == b.step(bs[i], 1 ms), \"debounce differs\"\n",
            xs = reals(&xs),
            bs = bools(&bs),
        );
        assert_all_pass(&body);
    }

    /// STD-DIFF for the filters and the ramp (within single-precision rounding).
    #[test]
    fn filters_match_the_native_blocks(xs in real_inputs()) {
        let n = xs.len();
        let body = format!(
            "test \"diff filters\":\n  let xs = {xs}\n  var a = lowpass(cutoff: 20 Hz)\n  var b = Lowpass(cutoff_hz: 20.0)\n  var c = highpass(cutoff: 20 Hz)\n  var d = Highpass(cutoff_hz: 20.0)\n  var e = moving_average(window: 4)\n  var f = MovingAverage(window: 4)\n  var g = ramp(rate: 5.0)\n  var h = Ramp(rate: 5.0)\n  for i in 0..{n}:\n    assert close(a.step(xs[i]), b.step(xs[i], 0.001), 0.001), \"lowpass differs\"\n    assert close(c.step(xs[i]), d.step(xs[i], 0.001), 0.001), \"highpass differs\"\n    assert close(e.step(xs[i]), f.step(xs[i]), 0.001), \"moving average differs\"\n    assert close(g.step(xs[i]), h.step(xs[i], 0.001), 0.001), \"ramp differs\"\n",
            xs = reals(&xs)
        );
        assert_all_pass(&body);
    }

    /// STD-DIFF for the PID, and STD-PID-1 and STD-PID-2 on random errors.
    #[test]
    fn pid_matches_the_native_block_and_keeps_its_contract(
        es in real_inputs(),
        kp in 0i32..8,
        ki in 0i32..8,
    ) {
        let n = es.len();
        let body = format!(
            "test \"diff pid\":\n  let es = {es}\n  var a = pid(kp: {kp}.0, ki: {ki}.0, kd: 0.5, out: -10.0..10.0)\n  var b = Pid(kp: {kp}.0, ki: {ki}.0, kd: 0.5, out_min: -10.0, out_max: 10.0)\n  for i in 0..{n}:\n    assert close(a.step(es[i]), b.update(es[i], 0.001), 0.001), \"pid differs\"\n\ntest \"diff pid limits and anti-windup\":\n  let es = {es}\n  var c = Pid(kp: {kp}.0, ki: {ki}.0, kd: 0.5, out_min: -10.0, out_max: 10.0)\n  for i in 0..{n}:\n    let raw = c.kp * es[i] + c.integral + c.kd * (es[i] - c.previous) / 0.01\n    let before = c.integral\n    let y = c.update(es[i], 0.01)\n    assert y >= -10.0 and y <= 10.0, \"output out of its limits\"\n    if raw > 10.0 or raw < -10.0:\n      assert c.integral == before, \"integrated while saturated\"\n",
            es = reals(&es)
        );
        assert_all_pass(&body);
    }
}
