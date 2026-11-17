//! Task B6: the firmware that `avr-gcc` builds from the generated C runs in the simulator
//! and drives its pins like the reference interpreter does.
//!
//! The comparison is physical: the interpreter traces every write of an output, the chip only
//! changes a pin when the level changes, so repeated writes of the same level are dropped from
//! the interpreter trace first. Times are compared in whole milliseconds.
//! The tests skip when `avr-gcc` is missing, unless `NEURON_REQUIRE_TOOLCHAIN` is set.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_codegen_c::{build_avr, emit_c, find_tool};
use neuron_interp::{parse_scenario, render, Interp, Options};
use neuron_mir::{lower, verify, IoK, Program, Source};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_sim_avr::Avr;
use neuron_target::{check, Registry};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

const M01: &str = include_str!("../../../examples/corpus/mcu/m01_blink.neuron");
const M02: &str = include_str!("../../../examples/corpus/mcu/m02_button_toggle.neuron");
const M06: &str = include_str!("../../../examples/corpus/mcu/m06_traffic_light.neuron");
const M07: &str = include_str!("../../../examples/corpus/mcu/m07_overheat_guard.neuron");
const M08: &str = include_str!("../../../examples/corpus/mcu/m08_two_tasks_signal.neuron");
const M11: &str = include_str!("../../../examples/corpus/mcu/m11_interrupt_priorities.neuron");

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

fn have_avr() -> bool {
    if find_tool("NEURON_AVR_GCC", &["avr-gcc"]).is_some() {
        return true;
    }
    assert!(
        std::env::var_os("NEURON_REQUIRE_TOOLCHAIN").is_none(),
        "avr-gcc is required (NEURON_REQUIRE_TOOLCHAIN is set) but was not found"
    );
    eprintln!("skipped: avr-gcc not found (see docs/phase-b-prerequisites.md)");
    false
}

fn firmware(src: &str, name: &str) -> (Program, String) {
    let p = program(src);
    let e = emit_c(&p);
    let dir: PathBuf =
        std::env::temp_dir().join(format!("neuron-b6-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let b = build_avr(&e, "atmega328p", p.sysclk_hz, &dir)
        .unwrap_or_else(|err| panic!("{err}\n{}", e.c));
    (p, std::fs::read_to_string(b.hex).unwrap())
}

/// `(whole milliseconds, output name, level)` from the simulator.
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

/// The same view from the interpreter trace: whole milliseconds, plain outputs only, and only
/// the writes that change the level (an output starts low).
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
fn blink_runs_on_the_simulated_chip_like_the_interpreter() {
    if !have_avr() {
        return;
    }
    let (p, hex) = firmware(M01, "m01");
    let mut cpu = Avr::new(F_CPU);
    cpu.load_hex(&hex).unwrap();
    run_ms(&mut cpu, 2100);
    let got = sim_pins(&cpu, &p, 2100);
    assert_eq!(got, interp_pins(M01, "", 2100));
    assert!(got.len() >= 4, "{got:?}");
}

#[test]
fn the_tick_is_one_millisecond_of_cpu_time() {
    if !have_avr() {
        return;
    }
    let (_, hex) = firmware(M01, "tick");
    let mut cpu = Avr::new(F_CPU);
    cpu.load_hex(&hex).unwrap();
    run_ms(&mut cpu, 1600);
    let t: Vec<u64> = cpu.pin_events().iter().map(|e| e.cycle).collect();
    assert!(t.len() >= 3);
    for w in t.windows(2) {
        let dt = w[1] - w[0];
        // 500 ms is 8,000,000 cycles; the latency of the handler adds a few hundred at most.
        assert!(
            (7_999_000..=8_001_000).contains(&dt),
            "period of {dt} cycles"
        );
    }
}

#[test]
fn an_external_interrupt_toggles_the_led() {
    if !have_avr() {
        return;
    }
    let (p, hex) = firmware(M02, "m02");
    let mut cpu = Avr::new(F_CPU);
    cpu.load_hex(&hex).unwrap();
    for (ms, level) in [(10, true), (20, false), (30, true), (40, false)] {
        input(&mut cpu, 'D', 2, ms, level);
    }
    run_ms(&mut cpu, 100);
    let scn = "at 10 ms: button = true\nat 20 ms: button = false\nat 30 ms: button = true\nat 40 ms: button = false\n";
    assert_eq!(sim_pins(&cpu, &p, 100), interp_pins(M02, scn, 100));
}

#[test]
fn a_state_machine_drives_three_pins() {
    if !have_avr() {
        return;
    }
    let (p, hex) = firmware(M06, "m06");
    let mut cpu = Avr::new(F_CPU);
    cpu.load_hex(&hex).unwrap();
    run_ms(&mut cpu, 9500);
    assert_eq!(sim_pins(&cpu, &p, 9500), interp_pins(M06, "", 9500));
}

#[test]
fn a_signal_with_a_deadline_is_visible_on_time() {
    if !have_avr() {
        return;
    }
    let (p, hex) = firmware(M08, "m08");
    let mut cpu = Avr::new(F_CPU);
    cpu.load_hex(&hex).unwrap();
    run_ms(&mut cpu, 200);
    assert_eq!(sim_pins(&cpu, &p, 200), interp_pins(M08, "", 200));
}

#[test]
fn interrupts_that_share_a_variable_with_a_task() {
    if !have_avr() {
        return;
    }
    let (p, hex) = firmware(M11, "m11");
    let mut cpu = Avr::new(F_CPU);
    cpu.load_hex(&hex).unwrap();
    // fast is D2 (INT0) and slow is D3 (INT1); both rising.
    for (ms, port_bit, level) in [
        (3, 2, true),
        (6, 2, false),
        (7, 3, true),
        (9, 3, false),
        (51, 2, true),
    ] {
        input(&mut cpu, 'D', port_bit, ms, level);
    }
    run_ms(&mut cpu, 120);
    let scn = "at 3 ms: fast = true\nat 6 ms: fast = false\nat 7 ms: slow = true\nat 9 ms: slow = false\nat 51 ms: fast = true\n";
    assert_eq!(sim_pins(&cpu, &p, 120), interp_pins(M11, scn, 120));
}

#[test]
fn the_overheat_guard_with_adc_float_math_and_a_watchdog() {
    if !have_avr() {
        return;
    }
    let (p, hex) = firmware(M07, "m07");
    let mut cpu = Avr::new(F_CPU);
    cpu.load_hex(&hex).unwrap();
    cpu.set_adc(0, 900);
    run_ms(&mut cpu, 250);
    // At 250 ms the sensor cools down and the operator acknowledges on D2.
    cpu.set_adc(0, 100);
    input(&mut cpu, 'D', 2, 250, true);
    run_ms(&mut cpu, 600);
    assert!(
        !cpu.watchdog_reset(),
        "the watchdog must be fed by the main loop"
    );
    let scn = "at 0 ms: sensor = 900\nat 250 ms: sensor = 100\nat 250 ms: ack = true\n";
    assert_eq!(sim_pins(&cpu, &p, 600), interp_pins(M07, scn, 600));
}

#[test]
fn a_uart_print_comes_out_of_the_serial_port() {
    if !have_avr() {
        return;
    }
    let src = "module hello for mcu on arduino_uno_r3\nio dbg: uart(baud: 115200 Bd) @ UART0\nvar n: u16 = 0\ntask say every 100 ms (prio 1):\n  n += 1\n  dbg.print(\"count={n}\\n\")\nsafety:\n  safe_state:\n    pass\n";
    let (_, hex) = firmware(src, "uart");
    let mut cpu = Avr::new(F_CPU);
    cpu.load_hex(&hex).unwrap();
    run_ms(&mut cpu, 350);
    let text: String = cpu.uart_output().iter().map(|(_, b)| *b as char).collect();
    assert_eq!(text, "count=1\ncount=2\ncount=3\ncount=4\n");
    // 115200 Bd with the U2X divisor: a byte every ~10 bit times; they are queued, never lost.
}

#[test]
fn a_pwm_duty_reaches_the_compare_register() {
    if !have_avr() {
        return;
    }
    let src = "module fade for mcu on arduino_uno_r3\nio led: pwm(freq: 1 kHz) @ D9\ntask t every 10 ms (prio 1):\n  led.duty = 25 %\nsafety:\n  safe_state:\n    led.duty = 0 %\n";
    let (_, hex) = firmware(src, "pwm");
    let mut cpu = Avr::new(F_CPU);
    cpu.load_hex(&hex).unwrap();
    run_ms(&mut cpu, 30);
    // 25 % of 255 rounded.
    assert_eq!(cpu.pwm_compare(1, 'a'), Some(64));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 6, failure_persistence: None, ..ProptestConfig::default() })]

    /// MOC-A1: the chip and the interpreter agree for random presses of a button.
    #[test]
    fn random_presses_agree_between_chip_and_interpreter(
        steps in prop::collection::btree_map(1u32..90, any::<bool>(), 0..8)
    ) {
        if !have_avr() { return Ok(()); }
        let (p, hex) = firmware(M02, "m02p");
        let mut cpu = Avr::new(F_CPU);
        cpu.load_hex(&hex).unwrap();
        let mut scn = String::new();
        for (t, v) in &steps {
            input(&mut cpu, 'D', 2, *t, *v);
            scn.push_str(&format!("at {t} ms: button = {v}\n"));
        }
        run_ms(&mut cpu, 100);
        prop_assert_eq!(sim_pins(&cpu, &p, 100), interp_pins(M02, &scn, 100));
    }
}
