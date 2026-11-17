//! Tasks B2 and B3: the generated C compiles, and runs like the interpreter.
//!
//! Property MOC-C1: for the same program and scenario, the program built from the generated C
//! with the host runtime prints the same trace as `neuron run` (up to the order of the lines
//! inside one instant). The tests that need a C compiler skip with a message when there is
//! none; set `NEURON_REQUIRE_TOOLCHAIN=1` (CI does) to make that an error.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_codegen_c::{build_avr, build_host, emit_c, find_tool, Emitted};
use neuron_interp::{parse_scenario, render, Interp, Options};
use neuron_mir::{lower, verify, Program, Source};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_target::{check, Registry};
use proptest::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;

const BLINK: &str = include_str!("../../../examples/blink_mcu.neuron");
const M01: &str = include_str!("../../../examples/corpus/mcu/m01_blink.neuron");
const M02: &str = include_str!("../../../examples/corpus/mcu/m02_button_toggle.neuron");
const M03: &str = include_str!("../../../examples/corpus/mcu/m03_debounced_button.neuron");
const M06: &str = include_str!("../../../examples/corpus/mcu/m06_traffic_light.neuron");
const M07: &str = include_str!("../../../examples/corpus/mcu/m07_overheat_guard.neuron");
const M08: &str = include_str!("../../../examples/corpus/mcu/m08_two_tasks_signal.neuron");
const M10: &str = include_str!("../../../examples/corpus/mcu/m10_event_task.neuron");
const M11: &str = include_str!("../../../examples/corpus/mcu/m11_interrupt_priorities.neuron");

fn nir_of(src: &str) -> neuron_nir::Nir {
    let a = analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: src.replace("\r\n", "\n"),
        }],
        None,
        &SemaOptions::default(),
    );
    match a.nir {
        Some(n) => n,
        None => panic!("analysis errors:\n{}", a.render()),
    }
}

fn pipeline(src: &str) -> (Program, Emitted) {
    let src = src.replace("\r\n", "\n");
    let nir = nir_of(&src);
    let reg = Registry::builtin().unwrap();
    let hw = check(&reg, &nir);
    assert!(hw.diags.is_empty(), "hardware: {:?}", hw.diags);
    let l = lower(
        &nir,
        &hw,
        &[Source {
            file: "t.neuron".into(),
            text: src.clone(),
        }],
    );
    assert!(l.diags.is_empty(), "lowering: {:?}", l.diags);
    let problems = verify(&l.program);
    assert!(problems.is_empty(), "verifier: {problems:?}");
    let e = emit_c(&l.program);
    (l.program, e)
}

fn tools_ok(name: &str, found: bool) -> bool {
    if found {
        return true;
    }
    assert!(
        std::env::var_os("NEURON_REQUIRE_TOOLCHAIN").is_none(),
        "{name} is required (NEURON_REQUIRE_TOOLCHAIN is set) but was not found"
    );
    eprintln!("skipped: {name} not found (see docs/phase-b-prerequisites.md)");
    false
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("neuron-b2-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Time of a trace line in milliseconds.
fn line_time(l: &str) -> f64 {
    l.split(" ms")
        .next()
        .unwrap_or("0")
        .trim()
        .parse()
        .unwrap_or(0.0)
}

/// Keeps the lines both sides produce, below `ms`, in a canonical order within an instant.
fn normalize(lines: Vec<String>, ms: u32) -> Vec<String> {
    // Real-valued signals are not compared: the interpreter prints them with their unit
    // (`131.964809 degC`) and the target keeps base units in single precision.
    let real_signal =
        |l: &str| l.contains(" ms  SIG") && l.rsplit(" = ").next().is_some_and(|v| v.contains('.'));
    let mut v: Vec<String> = lines
        .into_iter()
        .filter(|l| {
            ["OUT", "SIG", "STATE", "FAULT", "RESET", "EVENT"]
                .iter()
                .any(|k| l.contains(&format!(" ms  {k}")))
        })
        .filter(|l| !real_signal(l))
        .filter(|l| line_time(l) < f64::from(ms))
        .collect();
    v.sort_by(|a, b| {
        line_time(a)
            .partial_cmp(&line_time(b))
            .unwrap()
            .then_with(|| a.cmp(b))
    });
    v
}

fn interp_trace(src: &str, scenario: &str, ms: u32) -> Vec<String> {
    let nir = nir_of(&src.replace("\r\n", "\n"));
    let opts = Options {
        end_ns: i64::from(ms) * 1_000_000,
        ..Options::default()
    };
    let mut it = Interp::new(nir, opts).unwrap();
    it.load_scenario(&parse_scenario(scenario).unwrap());
    it.run();
    render(it.trace()).lines().map(String::from).collect()
}

fn host_trace(exe: &Path, dir: &Path, scenario: &str, ms: u32) -> Vec<String> {
    let scn = dir.join("scenario.scn");
    std::fs::write(&scn, scenario).unwrap();
    let out = Command::new(exe)
        .arg("--for")
        .arg(ms.to_string())
        .arg(&scn)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "host program failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(String::from)
        .collect()
}

fn host_exe(src: &str, name: &str) -> Option<(PathBuf, PathBuf)> {
    if !tools_ok(
        "a C compiler",
        find_tool("NEURON_CC", &["gcc", "cc", "clang"]).is_some(),
    ) {
        return None;
    }
    let (_, e) = pipeline(src);
    let dir = scratch(name);
    let exe = build_host(&e, &dir).unwrap_or_else(|err| panic!("{err}\n{}", e.c));
    Some((exe, dir))
}

fn assert_same(src: &str, name: &str, scenario: &str, ms: u32) {
    let Some((exe, dir)) = host_exe(src, name) else {
        return;
    };
    let want = normalize(interp_trace(src, scenario, ms), ms);
    let got = normalize(host_trace(&exe, &dir, scenario, ms), ms);
    assert!(!want.is_empty(), "{name}: the interpreter printed nothing");
    assert_eq!(
        got, want,
        "{name}: C trace (left) differs from the interpreter (right)"
    );
}

// ---- lowering ---------------------------------------------------------------------------

#[test]
fn the_supported_examples_lower_and_verify() {
    for (n, s) in [
        ("blink", BLINK),
        ("m01", M01),
        ("m02", M02),
        ("m06", M06),
        ("m07", M07),
        ("m08", M08),
        ("m10", M10),
        ("m11", M11),
    ] {
        let (p, e) = pipeline(s);
        assert!(e.c.contains("neuron_app"), "{n}");
        assert!(
            !p.tasks.is_empty() || !p.irqs.is_empty() || !p.fsms.is_empty(),
            "{n}"
        );
    }
}

#[test]
fn constructs_outside_the_subset_are_reported_as_ne0610() {
    // `debounce` is a native function block; the C backend does not run those yet.
    let nir = nir_of(M03);
    let reg = Registry::builtin().unwrap();
    let hw = check(&reg, &nir);
    let l = lower(
        &nir,
        &hw,
        &[Source {
            file: "t.neuron".into(),
            text: M03.into(),
        }],
    );
    assert!(!l.diags.is_empty());
    assert!(l.diags.iter().all(|d| d.code == "NE0610"), "{:?}", l.diags);
}

#[test]
fn the_generated_c_is_stable_and_golden() {
    for (name, src) in [
        ("blink_mcu", BLINK),
        ("m06_traffic_light", M06),
        ("m07_overheat_guard", M07),
    ] {
        let (_, e) = pipeline(src);
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../conformance/golden/c/{name}.c"));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &e.c).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| {
                panic!(
                    "missing {}; run tools/rust-check.ps1 -Golden",
                    path.display()
                )
            })
            .replace("\r\n", "\n");
        assert!(
            want == e.c,
            "{name}: the generated C changed; review it and run tools/rust-check.ps1 -Golden"
        );
    }
}

#[test]
fn the_source_map_points_back_to_neuron_lines() {
    let (_, e) = pipeline(BLINK);
    assert!(
        e.map_json.contains("\"files\":[\"t.neuron\"]"),
        "{}",
        e.map_json
    );
    assert!(e.map_json.contains("\"lines\":[["), "{}", e.map_json);
}

// ---- host builds and MOC-C1 ---------------------------------------------------------------

#[test]
fn blink_matches_the_interpreter() {
    assert_same(M01, "m01", "", 2100);
}

#[test]
fn a_button_interrupt_matches_the_interpreter() {
    let scn = "at 10 ms: button = true\nat 20 ms: button = false\nat 30 ms: button = true\nat 40 ms: button = false\n";
    assert_same(M02, "m02", scn, 100);
}

#[test]
fn a_state_machine_matches_the_interpreter() {
    assert_same(M06, "m06", "", 9500);
}

#[test]
fn signals_with_a_deadline_match_the_interpreter() {
    assert_same(M08, "m08", "", 200);
}

#[test]
fn events_and_event_tasks_match_the_interpreter() {
    let scn = "at 5 ms: sensor = true\nat 8 ms: sensor = false\nat 12 ms: sensor = true\n";
    assert_same(M10, "m10", scn, 60);
}

#[test]
fn interrupts_sharing_data_with_a_task_match_the_interpreter() {
    let scn = "at 3 ms: fast = true\nat 6 ms: fast = false\nat 7 ms: slow = true\nat 9 ms: slow = false\nat 51 ms: fast = true\n";
    assert_same(M11, "m11", scn, 120);
}

#[test]
fn the_overheat_guard_faults_and_resets_like_the_interpreter() {
    // 900 of 1023 is about 132 degC: the rule fires at the first safety poll after the
    // signal becomes visible. The reset input arrives at 250 ms.
    let scn = "at 0 ms: sensor = 900\nat 250 ms: sensor = 100\nat 250 ms: ack = true\n";
    assert_same(M07, "m07", scn, 600);
}

#[test]
fn an_overflow_is_a_fault_in_both() {
    let src = "module m for mcu on arduino_uno_r3\nio led: gpio.output @ D13\nio ack: gpio.input @ D4\nvar n: u8 = 250\ntask count every 1 ms (prio 1):\n  n += 1\n  led.write(n % 2 == 0)\nsafety:\n  safe_state:\n    led.write(false)\n  reset when ack.read()\n";
    assert_same(src, "overflow", "at 40 ms: ack = true\n", 80);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 12, failure_persistence: None, ..ProptestConfig::default() })]

    /// MOC-C1 under random inputs: any sequence of button edges gives the same trace.
    #[test]
    fn random_button_presses_match_the_interpreter(
        steps in prop::collection::btree_map(1u32..90, any::<bool>(), 0..10)
    ) {
        let scn: String = steps.iter().map(|(t, v)| format!("at {t} ms: button = {v}\n")).collect();
        let Some((exe, dir)) = host_exe(M02, "m02p") else { return Ok(()); };
        let want = normalize(interp_trace(M02, &scn, 100), 100);
        let got = normalize(host_trace(&exe, &dir, &scn, 100), 100);
        prop_assert_eq!(got, want);
    }
}

// ---- the target compiler ------------------------------------------------------------------

#[test]
fn the_generated_c_builds_for_the_uno_without_warnings() {
    if !tools_ok(
        "avr-gcc",
        find_tool("NEURON_AVR_GCC", &["avr-gcc"]).is_some(),
    ) {
        return;
    }
    for (n, s) in [
        ("blink", BLINK),
        ("m02", M02),
        ("m06", M06),
        ("m07", M07),
        ("m08", M08),
        ("m10", M10),
        ("m11", M11),
    ] {
        let (p, e) = pipeline(s);
        let dir = scratch(&format!("avr-{n}"));
        let b = build_avr(&e, "atmega328p", p.sysclk_hz, &dir)
            .unwrap_or_else(|err| panic!("{n}: {err}\n{}", e.c));
        assert!(b.hex.exists(), "{n}");
    }
}

#[test]
fn the_same_program_builds_for_the_mega() {
    if !tools_ok(
        "avr-gcc",
        find_tool("NEURON_AVR_GCC", &["avr-gcc"]).is_some(),
    ) {
        return;
    }
    let mega = BLINK
        .replace("arduino_uno_r3", "arduino_mega_2560")
        .replace("@ D2", "@ D21");
    let (p, e) = pipeline(&mega);
    let dir = scratch("mega");
    let b = build_avr(&e, "atmega2560", p.sysclk_hz, &dir)
        .unwrap_or_else(|err| panic!("{err}\n{}", e.c));
    assert!(b.hex.exists());
}
