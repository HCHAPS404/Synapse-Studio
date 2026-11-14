//! Task A8: safe state, faults and model checking (NE0802, NE0803, NE0901 to NE0904,
//! NL0901).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use proptest::prelude::*;

#[test]
fn every_output_needs_a_safe_state_ne0802() {
    let base = "module t for mcu\nio led: gpio.output @ D13\nio heater: pwm(freq: 1 kHz) @ TIM2.ch1(PA0)\ntask a every 10 ms:\n  led.toggle()\n";
    let a = run(base);
    assert_eq!(errors(&a), ["NE0802", "NE0802"], "{}", a.render());
    let partial = format!("{base}safety:\n  safe_state:\n    led.write(false)\n");
    let a = run(&partial);
    assert_eq!(errors(&a), ["NE0802"], "{}", a.render());
    assert!(a.render().contains("heater"));
    let full =
        format!("{base}safety:\n  safe_state:\n    led.write(false)\n    heater.duty = 0 %\n");
    ok(&full);
}

#[test]
fn plc_outputs_need_a_safe_state() {
    let src = "module t for plc\nscan 10 ms\ninputs:\n  a: bool @ %I0.0\noutputs:\n  m: bool @ %Q0.0\nrung r:\n  m = a\n";
    only(src, &["NE0802"]);
}

#[test]
fn inputs_do_not_need_a_safe_state() {
    ok("module t for mcu\nio b: gpio.input @ D2\nvar n: u16 = 0\ntask a every 10 ms:\n  n += 1\n");
}

#[test]
fn on_fault_may_not_drive_outputs_ne0803() {
    let src = "module t for mcu\nio led: gpio.output @ D13\nsafety:\n  safe_state:\n    led.write(false)\n  on fault:\n    led.write(true)\n";
    only(src, &["NE0803"]);
    let fine = "module t for mcu\nio led: gpio.output @ D13\nio dbg: uart(baud: 115200 Bd) @ USART2(tx: PA2, rx: PA3)\nsafety:\n  safe_state:\n    led.write(false)\n  on fault:\n    dbg.print(\"fault\")\n";
    ok(fine);
}

const DOOR: &str = "module t for mcu\nsignal go: bool\nfsm Door every 10 ms:\n  initial state Closed:\n    -> Open when go\n  state Open:\n    -> Closed when not go\n";

#[test]
fn violated_properties_come_with_a_shortest_counterexample_ne0901() {
    let src = format!("{DOOR}verify:\n  never Open\n");
    let a = run(&src);
    assert_eq!(errors(&a), ["NE0901"], "{}", a.render());
    let cx = &a.reports.counterexamples;
    assert_eq!(cx.len(), 1);
    let t = &cx[0].trace;
    assert_eq!(t.len(), 2, "{t:?}");
    assert_eq!(t[0].states, ["Door.Closed"]);
    assert_eq!(t[1].states, ["Door.Open"]);
    assert!(t[0].inputs.iter().any(|(n, v)| n == "go" && *v), "{t:?}");
    assert!(a.render().contains("counterexample"));
}

#[test]
fn properties_that_hold_pass() {
    let src = format!(
        "{DOOR}verify:\n  always not (Open and Closed)\n  never Open and not go and Closed\n"
    );
    let a = run(&src);
    assert!(a.is_ok(), "{}", a.render());
    assert!(a.reports.counterexamples.is_empty());
}

#[test]
fn implications_between_states_are_checked() {
    // Whenever the door is open, `go` was true one tick ago, but the property is about
    // the current inputs, so it can be violated.
    let src = format!("{DOOR}verify:\n  always Open implies go\n");
    only(&src, &["NE0901"]);
}

#[test]
fn bounded_response_properties() {
    let ok_src = format!("{DOOR}verify:\n  eventually_within 50 ms after go: Open\n");
    let a = run(&ok_src);
    assert!(a.is_ok(), "{}", a.render());
    // The adversary can keep `ready` false forever, so the door never opens.
    let bad = "module t for mcu\nsignal go: bool\nsignal ready: bool\nfsm Door every 10 ms:\n  initial state Closed:\n    -> Open when go and ready\n  state Open:\n    -> Closed when not go\nverify:\n  eventually_within 50 ms after go: Open\n";
    only(bad, &["NE0901"]);
}

#[test]
fn timed_transitions_use_the_declared_durations() {
    let src = "module t for mcu\nfsm F every 10 ms:\n  initial state A:\n    -> B after 100 ms\n  state B:\n    -> A after 100 ms\nverify:\n  eventually_within 100 ms after A: B\n";
    let a = run(src);
    assert!(a.is_ok(), "{}", a.render());
    let tight = src.replace("eventually_within 100 ms", "eventually_within 50 ms");
    only(&tight, &["NE0901"]);
}

#[test]
fn properties_about_inputs_only_are_inconclusive_nl0901() {
    let src = "module t for mcu\nsignal a: bool\nsignal b: bool\nverify:\n  always a implies b\n";
    let r = run(src);
    assert!(r.is_ok(), "{}", r.render());
    assert!(all_codes(&r).contains(&"NL0901"), "{}", r.render());
}

#[test]
fn unreachable_states_ne0902() {
    let src = "module t for mcu\nsignal go: bool\nfsm F every 10 ms:\n  initial state A:\n    -> B when go\n  state B:\n    -> A when not go\n  state Orphan:\n    -> A when go\n";
    only(src, &["NE0902"]);
    let dead = "module t for mcu\nfsm F every 10 ms:\n  initial state A:\n    -> B when false\n  state B:\n    -> A\n";
    only(dead, &["NE0902"]);
}

#[test]
fn states_without_a_way_out_ne0903() {
    let dead = "module t for mcu\nsignal go: bool\nfsm F every 10 ms:\n  initial state A:\n    -> B when go\n  state B:\n    do:\n      pass\n";
    only(dead, &["NE0903"]);
    // `final` states and fault states are allowed to stop.
    let fin = "module t for mcu\nsignal go: bool\nfsm F every 10 ms:\n  initial state A:\n    -> B when go\n  final state B:\n    do:\n      pass\n";
    ok(fin);
    let fault = "module t for mcu\nsignal go: bool\nfsm F every 10 ms:\n  initial state A:\n    -> B when go\n  state B:\n    entry: fault Jam\n";
    ok(fault);
}

#[test]
fn a_sequence_that_waits_forever_ne0903() {
    let src = "module t for mcu\nsequence s:\n  initial step a:\n    until false\n";
    only(src, &["NE0903"]);
}

#[test]
fn state_machines_need_exactly_one_initial_state_ne0904() {
    only(
        "module t for mcu\nfsm F every 10 ms:\n  state A:\n    -> A\n",
        &["NE0904"],
    );
    let two = "module t for mcu\nfsm F every 10 ms:\n  initial state A:\n    -> B\n  initial state B:\n    -> A\n";
    only(two, &["NE0904"]);
}

#[test]
fn sequences_are_model_checked_too() {
    let src = "module t for mcu\nsignal start: bool\nsignal done: bool\nsequence Cycle:\n  initial step Idle:\n    until start\n  step Work:\n    until done timeout 2 s -> Fault\n  step Fault:\n    fault CycleFault\n";
    ok(src);
    let with_verify = format!("{src}fsm Dummy every 10 ms:\n  initial state X:\n    -> X\nverify:\n  never Dummy.X and not Dummy.X\n");
    ok(&with_verify);
}

#[test]
fn spec_door_example_is_clean() {
    let src = "module t for mcu\nio lock: gpio.output @ D3\nio motor: gpio.output @ D4\nsignal open_cmd: bool\nsignal close_cmd: bool\nsignal end_open: bool\nsignal end_closed: bool\nsignal obstacle: bool\nfsm Door every 10 ms:\n  initial state Closed:\n    entry: lock.write(true)\n    -> Opening when open_cmd\n  state Opening:\n    entry: motor.write(true)\n    exit: motor.write(false)\n    -> Open when end_open\n    -> Fault after 5 s\n  state Open:\n    -> Closing when close_cmd or after 30 s\n  state Closing:\n    entry: motor.write(true)\n    -> Closed when end_closed\n    -> Opening when obstacle\n  state Fault:\n    entry: fault DoorJam\nsafety:\n  safe_state:\n    lock.write(true)\n    motor.write(false)\nverify:\n  never Open and Closed\n  eventually_within 10 s after open_cmd: Open\n";
    let a = run(src);
    // The last property fails: the adversary can drop `open_cmd` and never raise `end_open`.
    assert_eq!(errors(&a), ["NE0901"], "{}", a.render());
    assert!(a.render().contains("eventually_within"));
}

proptest! {
    /// The model checker finds a violation of `never S` exactly when state `S` is
    /// reachable in the transition graph, and the counterexample is a shortest path.
    #[test]
    fn counterexamples_are_shortest_paths(
        n in 2usize..6,
        edges in proptest::collection::vec((0usize..6, 0usize..6), 1..9),
        target in 1usize..6,
    ) {
        let n = n.max(2);
        let target = target % n;
        let mut edges: Vec<(usize, usize)> = edges.into_iter().map(|(a, b)| (a % n, b % n)).collect();
        edges.sort_unstable();
        edges.dedup();
        // Every state needs a way out to avoid NE0903.
        for s in 0..n {
            if !edges.iter().any(|(a, _)| *a == s) {
                edges.push((s, s));
            }
        }
        let mut src = String::from("module t for mcu\n");
        for (i, _) in edges.iter().enumerate() {
            src.push_str(&format!("signal e{i}: bool\n"));
        }
        src.push_str("fsm F every 10 ms:\n");
        for s in 0..n {
            src.push_str(&format!("  {}state S{s}:\n", if s == 0 { "initial " } else { "" }));
            for (i, (a, b)) in edges.iter().enumerate() {
                if *a == s {
                    src.push_str(&format!("    -> S{b} when e{i}\n"));
                }
            }
        }
        src.push_str(&format!("verify:\n  never S{target}\n"));
        // Breadth-first distance from S0 to the target.
        let mut dist = vec![usize::MAX; n];
        dist[0] = 0;
        let mut queue = std::collections::VecDeque::from([0usize]);
        while let Some(s) = queue.pop_front() {
            for (a, b) in &edges {
                if *a == s && dist[*b] == usize::MAX {
                    dist[*b] = dist[s] + 1;
                    queue.push_back(*b);
                }
            }
        }
        let a = run(&src);
        let codes = errors(&a);
        // States that are unreachable are reported by NE0902 as well.
        let unreachable = (0..n).filter(|s| dist[*s] == usize::MAX).count();
        prop_assert_eq!(codes.iter().filter(|c| **c == "NE0902").count(), unreachable, "{}", a.render());
        if dist[target] == usize::MAX {
            prop_assert!(!codes.contains(&"NE0901"), "{}", a.render());
        } else {
            prop_assert!(codes.contains(&"NE0901"), "{}", a.render());
            let trace = &a.reports.counterexamples[0].trace;
            prop_assert_eq!(trace.len(), dist[target] + 1, "{:?}", trace);
            let last = trace.last().unwrap();
            let want = format!("F.S{target}");
            prop_assert_eq!(&last.states[0], &want);
        }
    }
}
