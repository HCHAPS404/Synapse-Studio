//! Task B8: firmware produced by the native backend, with no external compiler, runs in the
//! ATmega328P simulator and drives its pins like the reference interpreter does.
//!
//! These tests need no toolchain at all, so they always run.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_interp::{parse_scenario, render, Interp, Options};
use neuron_mir::{lower, verify, IoK, Program, Source};
use neuron_native_avr::compile;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_sim_avr::Avr;
use neuron_target::{check, Registry};
use proptest::prelude::*;
use std::collections::BTreeMap;

const M01: &str = include_str!("../../../examples/corpus/mcu/m01_blink.neuron");
const M02: &str = include_str!("../../../examples/corpus/mcu/m02_button_toggle.neuron");
const M06: &str = include_str!("../../../examples/corpus/mcu/m06_traffic_light.neuron");
const M07: &str = include_str!("../../../examples/corpus/mcu/m07_overheat_guard.neuron");
const M08: &str = include_str!("../../../examples/corpus/mcu/m08_two_tasks_signal.neuron");
const M11: &str = include_str!("../../../examples/corpus/mcu/m11_interrupt_priorities.neuron");
const BLINK: &str = include_str!("../../../examples/blink_mcu.neuron");

const F_CPU: u64 = 16_000_000;

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

fn program(src: &str) -> Program {
    let src = src.replace("\r\n", "\n");
    let nir = nir_of(&src);
    let hw = check(&Registry::builtin().unwrap(), &nir);
    assert!(hw.diags.is_empty(), "{:?}", hw.diags);
    let l = lower(
        &nir,
        &hw,
        &[Source {
            file: "t.neuron".into(),
            text: src,
        }],
    );
    assert!(l.diags.is_empty(), "{:?}", l.diags);
    assert!(verify(&l.program).is_empty());
    l.program
}

fn firmware(src: &str) -> (Program, Avr) {
    let p = program(src);
    let n = compile(&p).unwrap_or_else(|d| panic!("{d:?}"));
    let mut cpu = Avr::new(F_CPU);
    cpu.load_hex(&n.hex).unwrap();
    (p, cpu)
}

fn sim_pins(cpu: &Avr, p: &Program, ms: u32) -> Vec<(u32, String, bool)> {
    let names: BTreeMap<(char, u8), String> = p
        .ios
        .iter()
        .filter(|io| io.kind == IoK::GpioOut)
        .map(|io| ((io.port.unwrap(), io.bit.unwrap()), io.name.clone()))
        .collect();
    cpu.pin_events()
        .iter()
        .filter_map(|e| {
            names
                .get(&(e.port, e.bit))
                .map(|n| (cpu.ms(e.cycle).floor() as u32, n.clone(), e.level))
        })
        .filter(|(t, _, _)| *t < ms)
        .collect()
}

fn interp_pins(src: &str, scenario: &str, ms: u32) -> Vec<(u32, String, bool)> {
    let opts = Options {
        end_ns: i64::from(ms) * 1_000_000,
        ..Options::default()
    };
    let mut it = Interp::new(nir_of(&src.replace("\r\n", "\n")), opts).unwrap();
    it.load_scenario(&parse_scenario(scenario).unwrap());
    it.run();
    let mut last: BTreeMap<String, bool> = BTreeMap::new();
    let mut out = Vec::new();
    for line in render(it.trace()).lines() {
        let Some((time, rest)) = line.split_once(" ms  OUT") else {
            continue;
        };
        let Some((name, value)) = rest.trim().split_once(" = ") else {
            continue;
        };
        if name.contains('.') || !matches!(value, "true" | "false") {
            continue;
        }
        let level = value == "true";
        let prev = last.insert(name.to_string(), level).unwrap_or(false);
        let t = time.trim().parse::<f64>().unwrap().floor() as u32;
        if prev != level && t < ms {
            out.push((t, name.to_string(), level));
        }
    }
    out
}

fn run_ms(cpu: &mut Avr, ms: u32) {
    let target = cpu.cycle_of_ms(f64::from(ms));
    cpu.run_until(target).unwrap();
}

fn input(cpu: &mut Avr, port: char, bit: u8, ms: u32, level: bool) {
    let c = cpu.cycle_of_ms(f64::from(ms));
    cpu.schedule_pin(c, port, bit, level);
}

#[test]
fn blink_without_an_external_compiler() {
    let (p, mut cpu) = firmware(M01);
    run_ms(&mut cpu, 2100);
    let got = sim_pins(&cpu, &p, 2100);
    assert_eq!(got, interp_pins(M01, "", 2100));
    assert!(got.len() >= 4, "{got:?}");
}

#[test]
fn the_example_blink_with_its_button() {
    let (p, mut cpu) = firmware(BLINK);
    run_ms(&mut cpu, 1600);
    assert_eq!(sim_pins(&cpu, &p, 1600), interp_pins(BLINK, "", 1600));
}

#[test]
fn the_tick_is_one_millisecond() {
    let (_, mut cpu) = firmware(M01);
    run_ms(&mut cpu, 1600);
    let t: Vec<u64> = cpu.pin_events().iter().map(|e| e.cycle).collect();
    assert!(t.len() >= 3);
    for w in t.windows(2) {
        let dt = w[1] - w[0];
        assert!(
            (7_999_000..=8_001_000).contains(&dt),
            "period of {dt} cycles"
        );
    }
}

#[test]
fn an_external_interrupt_toggles_the_led() {
    let (p, mut cpu) = firmware(M02);
    for (ms, level) in [(10, true), (20, false), (30, true), (40, false)] {
        input(&mut cpu, 'D', 2, ms, level);
    }
    run_ms(&mut cpu, 100);
    let scn = "at 10 ms: button = true\nat 20 ms: button = false\nat 30 ms: button = true\nat 40 ms: button = false\n";
    assert_eq!(sim_pins(&cpu, &p, 100), interp_pins(M02, scn, 100));
}

#[test]
fn a_state_machine_with_timeouts() {
    let (p, mut cpu) = firmware(M06);
    run_ms(&mut cpu, 9500);
    assert_eq!(sim_pins(&cpu, &p, 9500), interp_pins(M06, "", 9500));
}

#[test]
fn a_signal_with_logical_execution_time() {
    let (p, mut cpu) = firmware(M08);
    run_ms(&mut cpu, 200);
    assert_eq!(sim_pins(&cpu, &p, 200), interp_pins(M08, "", 200));
}

#[test]
fn two_interrupts_share_a_counter_with_a_task() {
    let (p, mut cpu) = firmware(M11);
    for (ms, bit, level) in [
        (3, 2, true),
        (6, 2, false),
        (7, 3, true),
        (9, 3, false),
        (51, 2, true),
    ] {
        input(&mut cpu, 'D', bit, ms, level);
    }
    run_ms(&mut cpu, 120);
    let scn = "at 3 ms: fast = true\nat 6 ms: fast = false\nat 7 ms: slow = true\nat 9 ms: slow = false\nat 51 ms: fast = true\n";
    assert_eq!(sim_pins(&cpu, &p, 120), interp_pins(M11, scn, 120));
}

#[test]
fn an_overflow_faults_into_the_safe_state() {
    let src = "module ov for mcu on arduino_uno_r3\nio led: gpio.output @ D13\nvar n: u8 = 250\ntask t every 10 ms (prio 1):\n  n = n + 1\n  led.write(n % 2 == 1)\nsafety:\n  safe_state:\n    led.write(false)\n";
    let (p, mut cpu) = firmware(src);
    run_ms(&mut cpu, 120);
    assert_eq!(sim_pins(&cpu, &p, 120), interp_pins(src, "", 120));
}

#[test]
fn constructs_outside_the_native_subset_are_ne0610() {
    // Floating point and the ADC.
    let p = program(M07);
    let err = compile(&p).unwrap_err();
    assert!(
        !err.is_empty() && err.iter().all(|d| d.code == "NE0610"),
        "{err:?}"
    );
}

#[test]
fn the_image_is_small() {
    let n = compile(&program(M01)).unwrap();
    // A 2-byte word count; the avr-gcc build of the same program is larger because of its C runtime.
    assert!(n.words < 1500, "{} words", n.words);
    assert!(n.listing.contains("act_task_blink:"), "{}", n.listing);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 8, failure_persistence: None, ..ProptestConfig::default() })]

    /// MOC-N1: random presses of the button agree between the native image and the interpreter.
    #[test]
    fn random_presses_agree(steps in prop::collection::btree_map(1u32..90, any::<bool>(), 0..8)) {
        let (p, mut cpu) = firmware(M02);
        let mut scn = String::new();
        for (t, v) in &steps {
            input(&mut cpu, 'D', 2, *t, *v);
            scn.push_str(&format!("at {t} ms: button = {v}\n"));
        }
        run_ms(&mut cpu, 100);
        prop_assert_eq!(sim_pins(&cpu, &p, 100), interp_pins(M02, &scn, 100));
    }

    /// Checked u8 and u16 additions fault exactly when Rust's `checked_add` fails.
    #[test]
    fn checked_additions_agree_with_rust(a in any::<u8>(), b in any::<u8>(), wide in any::<bool>()) {
        let (ty, x, y) = if wide { ("u16", u32::from(a) * 257, u32::from(b) * 257) } else { ("u8", u32::from(a), u32::from(b)) };
        let src = format!("module add for mcu on arduino_uno_r3\nio led: gpio.output @ D13\nvar x: {ty} = {x}\nvar y: {ty} = {y}\nvar z: {ty} = 0\ntask t every 10 ms (prio 1):\n  z = x + y\n  led.write(true)\nsafety:\n  safe_state:\n    led.write(false)\n");
        let (p, mut cpu) = firmware(&src);
        run_ms(&mut cpu, 5);
        let overflow = if wide { (x as u16).checked_add(y as u16).is_none() } else { (x as u8).checked_add(y as u8).is_none() };
        let got = sim_pins(&cpu, &p, 5);
        if overflow {
            prop_assert!(got.is_empty(), "{:?}", got);
        } else {
            prop_assert_eq!(got, vec![(0, "led".to_string(), true)]);
        }
    }
}
