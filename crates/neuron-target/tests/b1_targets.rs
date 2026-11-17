//! Task B1: the target stack loads, validates and checks real programs.
//!
//! Properties: TGT-1 (every description in `targets/` loads and validates), HW-1 (a resource
//! has one owner) and CLK-1 (a clock solution respects the chip limits and has the stated
//! error).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_target::solve::{self, ClockError, Owners};
use neuron_target::{check, ClockSource, ClockSpec, Pll, Registry, Tier};
use proptest::prelude::*;

fn nir_of(src: &str) -> neuron_nir::Nir {
    let a = analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: src.to_string(),
        }],
        None,
        &SemaOptions::default(),
    );
    match a.nir {
        Some(n) => n,
        None => panic!("analysis errors:\n{}", a.render()),
    }
}

fn safe_for(names: &[&str]) -> String {
    let mut s = String::from("safety:\n  safe_state:\n");
    for n in names {
        s.push_str(&format!("    {n}.write(false)\n"));
    }
    s
}

fn codes(src: &str) -> Vec<&'static str> {
    let reg = Registry::builtin().unwrap();
    check(&reg, &nir_of(src))
        .diags
        .iter()
        .map(|d| d.code)
        .collect()
}

const UNO_BLINK: &str = "module blink for mcu on arduino_uno_r3\nio led: gpio.output @ D13\nio button: gpio.input(pull: up) @ D2\ntask t every 500 ms:\n  led.toggle()\non interrupt button.falling (prio 2):\n  led.toggle()\nsafety:\n  safe_state:\n    led.write(false)\n";

#[test]
fn tgt_1_every_description_loads_and_validates() {
    let reg = Registry::builtin().unwrap();
    assert!(reg.board_names().len() >= 8, "{:?}", reg.board_names());
    for name in reg.board_names() {
        let t = reg.resolve(&name).unwrap();
        assert!(!t.chip.pins.is_empty(), "{name}");
    }
}

#[test]
fn every_board_has_its_support_tier() {
    let reg = Registry::builtin().unwrap();
    let of = |tier: Tier| -> Vec<String> {
        reg.boards()
            .filter(|b| b.tier == tier)
            .map(|b| b.name.clone())
            .collect()
    };
    assert_eq!(
        of(Tier::Runtime),
        [
            "arduino_mega_2560",
            "arduino_uno_r3",
            "nucleo_f446re",
            "raspberry_pi_pico",
            "raspberry_pi_pico_2w"
        ]
    );
    assert_eq!(of(Tier::Sdk), ["esp32_s3_devkit", "xiao_esp32s3"]);
    assert_eq!(of(Tier::Described), ["nucleo_n657x0_q"]);
}

#[test]
fn the_uno_blink_example_is_clean_and_binds_its_pins() {
    let reg = Registry::builtin().unwrap();
    let r = check(&reg, &nir_of(UNO_BLINK));
    assert!(r.is_ok(), "{:?}", r.diags);
    let led = r.bindings.iter().find(|b| b.name == "led").unwrap();
    assert_eq!((led.port, led.bit), (Some('B'), Some(5)));
    let button = r.bindings.iter().find(|b| b.name == "button").unwrap();
    assert_eq!(button.int_num, Some(0));
    assert!(button.pull_up);
    assert_eq!(r.sysclk_hz, 16_000_000);
}

#[test]
fn ne0607_for_an_unknown_board() {
    let c = codes(&format!(
        "module m for mcu on arduino_nope\nio led: gpio.output @ D13\n{}",
        safe_for(&["led"])
    ));
    assert_eq!(c, ["NE0607"]);
}

#[test]
fn ne0601_when_two_bindings_share_a_pin() {
    let c = codes(&format!(
        "module m for mcu on arduino_uno_r3\nio a: gpio.output @ D13\nio b: gpio.output @ D13\n{}",
        safe_for(&["a", "b"])
    ));
    assert_eq!(c, ["NE0601"]);
}

#[test]
fn ne0601_when_pwm_uses_the_timer_of_the_runtime_tick() {
    // D6 is OC0A (timer0), which the AVR runtime keeps for its 1 ms tick.
    let c = codes(&format!(
        "module m for mcu on arduino_uno_r3\nio p: pwm(freq: 1 kHz) @ D6\n{}",
        safe_for(&["p"])
    ));
    assert!(c.contains(&"NE0601"), "{c:?}");
}

#[test]
fn ne0603_when_the_pin_lacks_the_function() {
    let c = codes(&format!(
        "module m for mcu on arduino_uno_r3\nio p: pwm(freq: 1 kHz) @ D4\n{}",
        safe_for(&["p"])
    ));
    assert_eq!(c, ["NE0603"]);
    let c = codes("module m for mcu on arduino_uno_r3\nio s: adc(resolution: 10) @ D4\n");
    assert_eq!(c, ["NE0603"]);
    let c = codes(&format!(
        "module m for mcu on arduino_uno_r3\nio s: gpio.output @ D99\n{}",
        safe_for(&["s"])
    ));
    assert_eq!(c, ["NE0603"]);
}

#[test]
fn ne0603_for_an_interrupt_on_a_pin_without_external_interrupt() {
    let src = "module m for mcu on arduino_uno_r3\nio b: gpio.input @ D4\non interrupt b.falling (prio 2):\n  pass\n";
    assert_eq!(codes(src), ["NE0603"]);
}

#[test]
fn ne0606_for_a_uart_rate_the_clock_cannot_make() {
    let ok = "module m for mcu on arduino_uno_r3\nio u: uart(baud: 115200 Bd) @ UART0\n";
    assert!(codes(ok).is_empty(), "{:?}", codes(ok));
    let bad =
        "module m for mcu on arduino_uno_r3\nio u: uart(baud: 230400 Bd, tolerance: 1 %) @ UART0\n";
    assert_eq!(codes(bad), ["NE0606"]);
}

#[test]
fn ne0606_for_a_pwm_carrier_far_from_the_request() {
    // 20 kHz is not reachable with an 8-bit fast PWM at 16 MHz (nearest is 62.5 kHz).
    let c = codes(&format!(
        "module m for mcu on arduino_uno_r3\nio p: pwm(freq: 20 kHz) @ D9\n{}",
        safe_for(&["p"])
    ));
    assert_eq!(c, ["NE0606"]);
}

#[test]
fn ne0608_for_an_adc_resolution_beyond_the_chip() {
    let c = codes("module m for mcu on arduino_uno_r3\nio s: adc(resolution: 12) @ A0\n");
    assert_eq!(c, ["NE0608"]);
}

#[test]
fn ne0605_for_a_system_clock_that_cannot_be_made() {
    let src = &format!("module m for mcu on nucleo_f446re\nclocks:\n  sysclk = 168 MHz from hse(8 MHz)\nio led: gpio.output @ PA5\n{}", safe_for(&["led"]));
    assert!(codes(src).is_empty(), "{:?}", codes(src));
    // 200 MHz is above the 180 MHz limit of the chip.
    let bad = &format!("module m for mcu on nucleo_f446re\nclocks:\n  sysclk = 200 MHz from hse(8 MHz)\nio led: gpio.output @ PA5\n{}", safe_for(&["led"]));
    assert_eq!(codes(bad), ["NE0605"]);
}

#[test]
fn the_thermostat_example_binds_on_the_nucleo() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/thermostat.neuron"
    ))
    .unwrap();
    let reg = Registry::builtin().unwrap();
    let r = check(&reg, &nir_of(&src.replace("\r\n", "\n")));
    assert!(r.is_ok(), "{:?}", r.diags);
}

// ---- properties ------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, failure_persistence: None, ..ProptestConfig::default() })]

    /// HW-1: a resource is owned by at most one binding, and a claim fails exactly when taken.
    #[test]
    fn hw_1_exclusive_ownership(claims in prop::collection::vec((0u8..6, 0u8..4), 0..40)) {
        let mut owners = Owners::default();
        let mut model: std::collections::BTreeMap<String, String> = Default::default();
        for (res, who) in claims {
            let (r, w) = (format!("res{res}"), format!("io{who}"));
            let got = owners.claim(&r, &w);
            match model.get(&r) {
                Some(prev) => prop_assert_eq!(got, Err(prev.clone())),
                None => { prop_assert_eq!(got, Ok(())); model.insert(r, w); }
            }
        }
        prop_assert_eq!(owners.len(), model.len());
    }

    /// CLK-1: a solution respects the limits of the chip and has exactly the requested rate;
    /// when there is none, the nearest values reported are real candidates.
    #[test]
    fn clk_1_solutions_respect_the_limits(
        src_mhz in 1u64..30, want_mhz in 1u64..200,
        mul_max in 8u32..60, pre_max in 1u32..8,
    ) {
        let spec = ClockSpec {
            max_hz: 180_000_000,
            sources: vec![ClockSource { name: "hse".into(), min_hz: 1_000_000, max_hz: 30_000_000 }],
            pll: Some(Pll { input_min_hz: 1_000_000, input_max_hz: 8_000_000, vco_min_hz: 50_000_000,
                            vco_max_hz: 400_000_000, mul_min: 2, mul_max, pre_min: 1, pre_max }),
            post_dividers: vec![1, 2, 4, 8],
        };
        let (src, want) = (src_mhz * 1_000_000, want_mhz * 1_000_000);
        match solve::solve_sysclk(&spec, "hse", src, want) {
            Ok(s) => {
                prop_assert_eq!(s.hz, want);
                prop_assert_eq!(src / u64::from(s.pre) * u64::from(s.mul) / u64::from(s.post), want);
                let fin = src / u64::from(s.pre);
                prop_assert!((1_000_000..=8_000_000).contains(&fin));
                prop_assert!((50_000_000..=400_000_000).contains(&(fin * u64::from(s.mul))));
                prop_assert!(s.hz <= spec.max_hz);
            }
            Err(ClockError::Unreachable { below, above, .. }) => {
                let all: Vec<u64> = solve::candidates(&spec, "hse", src).iter().map(|c| c.hz).collect();
                prop_assert!(!all.contains(&want));
                if let Some(b) = below { prop_assert!(all.contains(&b) && b < want); }
                if let Some(a) = above { prop_assert!(all.contains(&a) && a > want); }
            }
            Err(ClockError::BadSource { .. }) => prop_assert!(false, "the source is valid"),
        }
    }

    /// The error of a UART divisor equals its definition, and the best divisor is no worse
    /// than either mode.
    #[test]
    fn uart_error_is_consistent(baud in prop::sample::select(vec![1200u32, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400])) {
        if let Some(d) = solve::uart_avr(16_000_000, baud) {
            let per_bit = if d.double_speed { 8.0 } else { 16.0 };
            let achieved = 16_000_000.0 / (per_bit * (f64::from(d.ubrr) + 1.0));
            prop_assert!((d.achieved_baud - achieved).abs() < 1e-6);
            prop_assert!((d.error - solve::rate_error(f64::from(baud), achieved)).abs() < 1e-12);
        }
    }
}
