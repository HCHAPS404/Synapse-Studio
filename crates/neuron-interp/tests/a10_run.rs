//! Task A10: the reference interpreter runs programs in simulated time.
//!
//! Properties: MOC-R1 (determinism), LET-1 (signals do not depend on execution delays)
//! and SAFE-1 (after a fault, outputs hold their safe values).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_interp::{parse_scenario, run_tests, Interp, Jitter, Options, TraceEvent, TraceKind};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;

const THERMOSTAT: &str = include_str!("../../../examples/thermostat.neuron");
const CONVEYOR: &str = include_str!("../../../examples/conveyor_plc.neuron");

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

fn run_with(src: &str, scenario: &str, ms: i64, jitter: Option<Jitter>) -> Vec<TraceEvent> {
    let opts = Options {
        end_ns: ms * 1_000_000,
        jitter,
        ..Options::default()
    };
    let mut it = Interp::new(nir_of(src), opts).unwrap();
    it.load_scenario(&parse_scenario(scenario).unwrap());
    it.run();
    it.trace().to_vec()
}

fn run(src: &str, scenario: &str, ms: i64) -> Vec<TraceEvent> {
    run_with(src, scenario, ms, None)
}

fn show(t: &[TraceEvent]) -> String {
    neuron_interp::render(t)
}

/// Entries of one kind and name, as `(milliseconds, value)`.
fn of(t: &[TraceEvent], kind: TraceKind, name: &str) -> Vec<(f64, String)> {
    t.iter()
        .filter(|e| e.kind == kind && e.name == name)
        .map(|e| (e.t_ns as f64 / 1.0e6, e.value.clone()))
        .collect()
}

#[test]
fn a_blinking_led_toggles_every_period() {
    let src = "module blink for mcu\nio led: gpio.output @ D13\ntask t every 500 ms:\n  led.toggle()\nsafety:\n  safe_state:\n    led.write(false)\n";
    let t = run(src, "", 1_200);
    assert_eq!(
        of(&t, TraceKind::Output, "led"),
        vec![
            (0.0, "true".to_string()),
            (500.0, "false".to_string()),
            (1000.0, "true".to_string())
        ],
        "{}",
        show(&t)
    );
}

#[test]
fn an_interrupt_runs_when_its_input_falls() {
    let src = "module m for mcu\nio led: gpio.output @ D13\nio button: gpio.input(pull: up) @ D2\non interrupt button.falling (prio 2):\n  led.toggle()\nsafety:\n  safe_state:\n    led.write(false)\n";
    let t = run(
        src,
        "10 ms: button = true\n20 ms: button = false\n30 ms: button = true\n40 ms: button = false\n",
        100,
    );
    assert_eq!(
        of(&t, TraceKind::Output, "led"),
        vec![(20.0, "true".to_string()), (40.0, "false".to_string())],
        "{}",
        show(&t)
    );
}

#[test]
fn signals_become_visible_at_the_deadline() {
    let src = "module m for mcu\nsignal level: u16\ntask t every 10 ms (prio 1, deadline 4 ms):\n  level = level + 1\nsafety:\n  safe_state:\n    pass\n";
    let t = run(src, "", 25);
    let sig = of(&t, TraceKind::Signal, "level");
    assert_eq!(
        sig,
        vec![
            (4.0, "1".to_string()),
            (14.0, "2".to_string()),
            (24.0, "3".to_string())
        ],
        "{}",
        show(&t)
    );
}

#[test]
fn a_task_reads_signals_as_of_its_release() {
    // `b` copies `a`. Both are released together; `b` sees the value `a` had before
    // `a`'s write at the deadline, so it always lags by one period.
    let src = "module m for mcu\nsignal a: u16\nsignal b: u16\ntask ta every 10 ms (prio 2):\n  a = a + 1\ntask tb every 10 ms (prio 1):\n  b = a\nsafety:\n  safe_state:\n    pass\n";
    let t = run(src, "", 45);
    let a = of(&t, TraceKind::Signal, "a");
    let b = of(&t, TraceKind::Signal, "b");
    assert_eq!(
        a.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(),
        ["1", "2", "3", "4"]
    );
    assert_eq!(
        b.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(),
        ["1", "2", "3"]
    );
}

#[test]
fn the_thermostat_enters_the_safe_state_when_it_overheats() {
    let t = run(THERMOSTAT, "0 ms: sensor = 3500\n", 300);
    let f: Vec<_> = t.iter().filter(|e| e.kind == TraceKind::Fault).collect();
    assert_eq!(f.len(), 1, "{}", show(&t));
    assert_eq!(f[0].name, "Overheat");
    assert!(
        (f[0].t_ns as f64 / 1.0e6 - 10.0).abs() < 1e-9,
        "{}",
        show(&t)
    );
    let after: Vec<_> = t
        .iter()
        .filter(|e| e.t_ns >= f[0].t_ns && e.kind == TraceKind::Output)
        .collect();
    assert!(
        after
            .iter()
            .any(|e| e.name == "heater.duty" && e.value == "0 %"),
        "{}",
        show(&t)
    );
    assert!(
        after.iter().any(|e| e.name == "alarm" && e.value == "true"),
        "{}",
        show(&t)
    );
    assert!(
        t.iter()
            .all(|e| e.kind != TraceKind::Output || e.t_ns <= f[0].t_ns),
        "outputs changed after the fault:\n{}",
        show(&t)
    );
}

#[test]
fn the_thermostat_regulates_below_the_limit() {
    // 60 degC is 2048/4095*120 = 60.01 degC: the heater is not saturated.
    let t = run(THERMOSTAT, "0 ms: sensor = 2000\n", 500);
    assert!(t.iter().all(|e| e.kind != TraceKind::Fault), "{}", show(&t));
    let temp = of(&t, TraceKind::Signal, "temp");
    assert!(!temp.is_empty(), "{}", show(&t));
    assert!(temp[0].1.ends_with("degC"), "{:?}", temp);
}

#[test]
fn a_plc_program_latches_the_motor_and_stops_it_on_a_jam() {
    let t = run(
        CONVEYOR,
        "0 ms: stop = true\n100 ms: start = true\n200 ms: start = false\n300 ms: sensor = true\n",
        4_000,
    );
    // The first scan publishes the initial image of the outputs.
    let motor = of(&t, TraceKind::Output, "motor");
    assert_eq!(motor.len(), 3, "{}", show(&t));
    assert_eq!(motor[0], (0.0, "false".to_string()));
    assert_eq!(motor[1], (100.0, "true".to_string()));
    assert_eq!(motor[2].1, "false");
    assert!(
        (3_280.0..=3_320.0).contains(&motor[2].0),
        "the jam timer is 3 s after the sensor: {}",
        show(&t)
    );
}

#[test]
fn a_sequence_waits_and_loops() {
    let src = "module m for mcu\nio valve: gpio.output @ D3\nsequence Cycle:\n  initial step Fill:\n    valve.write(true)\n    wait 200 ms\n  step Drain:\n    valve.write(false)\n    wait 100 ms\n    -> Fill\nsafety:\n  safe_state:\n    valve.write(false)\n";
    let t = run(src, "", 750);
    let v: Vec<f64> = of(&t, TraceKind::Output, "valve")
        .iter()
        .map(|x| x.0)
        .collect();
    assert_eq!(v, vec![0.0, 200.0, 300.0, 500.0, 600.0], "{}", show(&t));
}

#[test]
fn a_state_machine_follows_its_transitions() {
    let src = "module m for mcu\nio motor: gpio.output @ D4\nio go: gpio.input @ D2\nfsm Door every 10 ms:\n  initial state Closed:\n    -> Opening when go.read()\n  state Opening:\n    entry: motor.write(true)\n    exit: motor.write(false)\n    -> Open after 500 ms\n  state Open:\n    -> Closed after 1 s\nsafety:\n  safe_state:\n    motor.write(false)\n";
    let t = run(src, "20 ms: go = true\n100 ms: go = false\n", 2_000);
    let st: Vec<(f64, String)> = of(&t, TraceKind::State, "Door");
    assert_eq!(
        st,
        vec![
            (0.0, "Closed".to_string()),
            (20.0, "Opening".to_string()),
            (520.0, "Open".to_string()),
            (1520.0, "Closed".to_string())
        ],
        "{}",
        show(&t)
    );
    assert_eq!(of(&t, TraceKind::Output, "motor").len(), 2);
}

#[test]
fn a_trap_enters_the_safe_state() {
    let src = "module m for mcu\nio led: gpio.output @ D13\nvar n: u8 = 250\ntask t every 1 ms:\n  n += 1\n  led.write(n > 252)\nsafety:\n  safe_state:\n    led.write(false)\n";
    let t = run(src, "", 20);
    let f: Vec<_> = t.iter().filter(|e| e.kind == TraceKind::Fault).collect();
    assert_eq!(f.len(), 1, "{}", show(&t));
    assert_eq!(f[0].name, "Overflow");
    assert_eq!(f[0].t_ns, 5_000_000, "{}", show(&t));
    let led = of(&t, TraceKind::Output, "led");
    assert_eq!(
        led.last().map(|x| x.1.as_str()),
        Some("false"),
        "{}",
        show(&t)
    );
}

#[test]
fn a_reset_condition_resumes_the_program() {
    let src = "module m for mcu\nio led: gpio.output @ D13\nio ack: gpio.input @ D2\nio bad: gpio.input @ D3\ntask t every 10 ms:\n  led.toggle()\nsafety:\n  safe_state:\n    led.write(false)\n  when bad.read():\n    fault Bad\n  reset when ack.read()\n";
    let t = run(
        src,
        "35 ms: bad = true\n100 ms: bad = false\n150 ms: ack = true\n",
        300,
    );
    assert_eq!(
        t.iter().filter(|e| e.kind == TraceKind::Fault).count(),
        1,
        "{}",
        show(&t)
    );
    assert_eq!(
        t.iter().filter(|e| e.kind == TraceKind::Reset).count(),
        1,
        "{}",
        show(&t)
    );
    let reset_t = t.iter().find(|e| e.kind == TraceKind::Reset).unwrap().t_ns;
    let fault_t = t.iter().find(|e| e.kind == TraceKind::Fault).unwrap().t_ns;
    assert!(
        t.iter()
            .filter(|e| e.kind == TraceKind::Output && e.t_ns > fault_t && e.t_ns < reset_t)
            .count()
            == 0
    );
    assert!(
        t.iter()
            .any(|e| e.kind == TraceKind::Output && e.t_ns > reset_t),
        "{}",
        show(&t)
    );
}

#[test]
fn device_actions_complete_after_the_action_time() {
    let src = "cell Line\n  device plc: generic_s7_1200 @ ip(192.168.0.10)\n  sequence Cycle on plc:\n    initial step Run:\n      run plc.capper.cycle()\n      until plc.capper.done timeout 3 s -> Fault\n      -> Done\n    step Fault:\n      fault CycleFault\n    step Done:\n      -> Run\n";
    let t = run(src, "", 500);
    assert!(t.iter().all(|e| e.kind != TraceKind::Fault), "{}", show(&t));
    let states = of(&t, TraceKind::State, "Cycle");
    let names: Vec<&str> = states.iter().map(|s| s.1.as_str()).collect();
    assert_eq!(names[..2], ["Run", "Done"], "{}", show(&t));
    assert!((states[1].0 - 100.0).abs() <= 1.0, "{}", show(&t));
}

#[test]
fn a_timeout_jumps_to_the_named_step() {
    let src = "cell Line\n  device plc: generic_s7_1200 @ ip(192.168.0.10)\n  sequence Cycle on plc:\n    initial step Run:\n      run plc.capper.cycle()\n      until plc.capper.done timeout 50 ms -> Fault\n      -> Done\n    step Fault:\n      fault CycleFault\n    step Done:\n      -> Run\n";
    let t = run(src, "", 500);
    let f: Vec<_> = t.iter().filter(|e| e.kind == TraceKind::Fault).collect();
    assert_eq!(f.len(), 1, "{}", show(&t));
    assert_eq!(f[0].name, "CycleFault");
}

#[test]
fn classes_functions_and_contracts_run() {
    let src = "module calc for host\nclass Acc:\n  var total: i32 = 0\n\n  pub fn add(mut self, x: i32) -> i32:\n    requires x >= 0\n    self.total += x\n    return self.total\n\nfn twice(x: i32) -> i32 = x * 2\n\ntest \"acc\":\n  var a = Acc()\n  assert a.add(2) == 2, \"first\"\n  assert a.add(3) == 5, \"second\"\n  assert twice(a.total) == 10, \"function\"\n\ntest \"a failing assertion is reported\":\n  var a = Acc()\n  assert a.add(1) == 2, \"expected to fail\"\n";
    let r = run_tests(&nir_of(src), &Options::default());
    assert_eq!(r.len(), 2);
    assert!(r[0].passed, "{}", r[0].message);
    assert!(!r[1].passed);
    assert_eq!(r[1].message, "assertion failed: expected to fail");
}

#[test]
fn test_blocks_drive_inputs_and_advance_time() {
    let src = "module toggle_test for mcu\nio led: gpio.output @ D13\nio button: gpio.input @ D2\n\non interrupt button.rising (prio 2):\n  led.toggle()\n\nsafety:\n  safe_state:\n    led.write(false)\n\ntest \"the button toggles the led\":\n  drive(button, true)\n  assert led.read() == true, \"on after the first press\"\n  drive(button, false)\n  drive(button, true)\n  assert led.read() == false, \"off after the second press\"\n\ntest \"time passes only when asked\":\n  assert elapsed() == 0.0, \"starts at zero\"\n  advance(250 ms)\n  assert elapsed() > 0.2, \"after advance\"\n";
    let r = run_tests(&nir_of(src), &Options::default());
    assert_eq!(r.len(), 2);
    for t in &r {
        assert!(t.passed, "{}: {}", t.name, t.message);
    }
}

/// I11 (chapter 17.8): a test runs its own module and what it refers to. `h05` faults at
/// 2 s on purpose; the traffic light test needs 8 s and must not be stopped by it.
#[test]
fn a_fault_in_another_module_does_not_stop_a_test() {
    let files = [
        (
            "a_h05.neuron",
            include_str!("../../../examples/corpus/host/h05_fault_and_reset.neuron"),
        ),
        (
            "b_m06.neuron",
            include_str!("../../../examples/corpus/mcu/m06_traffic_light.neuron"),
        ),
        (
            "c_probe.neuron",
            "module probe for host\n\ntest \"fails on purpose\":\n  advance(10 ms)\n  assert elapsed() > 1.0, \"too early\"\n",
        ),
    ];
    let a = analyze(
        files
            .iter()
            .map(|(p, t)| SourceFile {
                path: (*p).into(),
                text: t.replace("\r\n", "\n"),
            })
            .collect(),
        None,
        &SemaOptions::default(),
    );
    let nir = a.nir.clone().unwrap_or_else(|| panic!("{}", a.render()));
    let r = run_tests(&nir, &Options::default());
    let light = r
        .iter()
        .find(|t| nir.modules[t.module].file == "b_m06.neuron")
        .expect("the traffic light test runs");
    assert!(light.passed, "{}", light.message);
    let probe = r
        .iter()
        .find(|t| nir.modules[t.module].file == "c_probe.neuron")
        .expect("the probe test runs");
    assert!(!probe.passed);
    // The location of a failed assertion names the module of the package, not of the scope.
    assert_eq!(probe.at.map(|(m, _)| m), Some(probe.module));
}

#[test]
fn the_same_run_gives_the_same_trace() {
    let sc = "0 ms: sensor = 1800\n200 ms: sensor = 2400\n400 ms: sensor = 1500\n";
    assert_eq!(run(THERMOSTAT, sc, 800), run(THERMOSTAT, sc, 800));
}

#[test]
fn let_1_signals_do_not_depend_on_execution_delays() {
    let sc = "0 ms: sensor = 1800\n200 ms: sensor = 2400\n400 ms: sensor = 1500\n";
    let signals = |t: Vec<TraceEvent>| -> Vec<TraceEvent> {
        t.into_iter()
            .filter(|e| e.kind == TraceKind::Signal)
            .collect()
    };
    let base = signals(run(THERMOSTAT, sc, 800));
    assert!(!base.is_empty());
    for seed in 1..=5u64 {
        let j = Some(Jitter {
            seed,
            max_ns: 900_000,
        });
        assert_eq!(
            base,
            signals(run_with(THERMOSTAT, sc, 800, j)),
            "seed {seed}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// SAFE-1: after a fault, no output changes until a reset.
    #[test]
    fn safe_1_outputs_hold_their_safe_values_after_a_fault(
        steps in proptest::collection::vec((0u32..600, 0u32..4096), 1..6)
    ) {
        let mut sc = String::new();
        for (ms, v) in &steps {
            sc.push_str(&format!("{ms} ms: sensor = {v}\n"));
        }
        let t = run(THERMOSTAT, &sc, 1_000);
        if let Some(f) = t.iter().find(|e| e.kind == TraceKind::Fault) {
            let late: Vec<_> = t
                .iter()
                .filter(|e| e.kind == TraceKind::Output && e.t_ns > f.t_ns)
                .collect();
            prop_assert!(late.is_empty(), "outputs changed after the fault: {:?}", late);
            let duty = t.iter().rev().find(|e| e.name == "heater.duty" && e.t_ns <= f.t_ns);
            prop_assert!(duty.is_some_and(|d| d.value == "0 %"));
        }
    }
}
