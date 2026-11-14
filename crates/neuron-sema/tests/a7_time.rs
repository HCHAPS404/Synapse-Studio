//! Task A7: time, concurrency and memory (NE0301, NE0304, NE0306, NE0401, NE0402,
//! NE0406, NE0407, NE0604, NL0402, NL0403).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use neuron_sema::SemaOptions;
use proptest::prelude::*;

const SHARED: &str = "module t for mcu\nvar shared: u16 = 0\ntask hi every 10 ms (prio 3):\n  shared += 1\ntask lo every 100 ms (prio 1):\n  shared += 1\n";

#[test]
fn resource_ceilings_are_the_highest_accessor_priority() {
    let a = run(SHARED);
    assert!(a.is_ok(), "{}", a.render());
    let r = &a.reports.resources;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].name, "shared");
    assert_eq!(r[0].ceiling, 3);
    assert!(r[0].shared && !r[0].cross_core);
    assert_eq!(r[0].accessors, ["hi", "lo"]);
}

#[test]
fn unshared_state_is_reported_as_not_shared() {
    let a = run("module t for mcu\nvar a: u16 = 0\nvar b: u16 = 0\ntask x every 10 ms:\n  a += 1\ntask y every 10 ms:\n  b += 1\n");
    assert!(
        a.reports.resources.iter().all(|r| !r.shared),
        "{:?}",
        a.reports.resources
    );
}

#[test]
fn state_reached_through_functions_counts() {
    let src = "module t for mcu\nvar n: u16 = 0\nfn bump():\n  n += 1\ntask a every 10 ms (prio 2):\n  bump()\ntask b every 10 ms (prio 1):\n  n += 1\n";
    let a = run(src);
    let n = a
        .reports
        .resources
        .iter()
        .find(|r| r.name == "n")
        .expect("resource n");
    assert_eq!(n.ceiling, 2);
    assert!(n.shared);
}

#[test]
fn undeclared_accesses_ne0301() {
    let src = "module t for mcu\nvar a: u16 = 0\nvar b: u16 = 0\ntask t every 10 ms (reads a):\n  let x = b\n";
    only(src, &["NE0301"]);
    let ok_src = "module t for mcu\nvar a: u16 = 0\nvar b: u16 = 0\ntask t every 10 ms (reads a b, writes a):\n  a = b\n";
    ok(ok_src);
    let write = "module t for mcu\nvar a: u16 = 0\ntask t every 10 ms (reads a):\n  a = 1\n";
    only(write, &["NE0301"]);
}

#[test]
fn vars_shared_across_cores_ne0304() {
    let src = "module t for mcu\nvar n: u16 = 0\ntask a every 10 ms (core 0):\n  n += 1\ntask b every 10 ms (core 1):\n  n += 1\n";
    only(src, &["NE0304"]);
}

#[test]
fn signals_have_one_writer_ne0306() {
    let two = "module t for mcu\nsignal s: u16 = 0\ntask a every 10 ms:\n  s = 1\ntask b every 10 ms:\n  s = 2\n";
    only(two, &["NE0306"]);
    let one = "module t for mcu\nsignal s: u16 = 0\ntask a every 10 ms:\n  s = 1\ntask b every 10 ms:\n  let x = s\n";
    ok(one);
}

#[test]
fn periods_must_be_multiples_of_the_base_tick_ne0401() {
    only(
        "module t for mcu\ntask a every 1500 us:\n  pass\n",
        &["NE0401"],
    );
    ok("module t for mcu\ntask a every 2 ms:\n  pass\n");
    let opts = SemaOptions {
        base_tick_ns: Some(500_000),
        ..SemaOptions::default()
    };
    let a = run_with("module t for mcu\ntask a every 1500 us:\n  pass\n", &opts);
    assert!(a.is_ok(), "{}", a.render());
}

#[test]
fn plc_periods_follow_the_scan_time() {
    only(
        "module t for plc\nscan 10 ms\ntask a every 15 ms:\n  pass\n",
        &["NE0401"],
    );
    ok("module t for plc\nscan 10 ms\ntask a every 30 ms:\n  pass\n");
}

#[test]
fn deadlines_longer_than_periods_ne0406() {
    only(
        "module t for mcu\ntask a every 10 ms (deadline 20 ms):\n  pass\n",
        &["NE0406"],
    );
}

#[test]
fn durations_must_be_constants_ne0407() {
    only(
        "module t for mcu\nvar p: u16 ms = 10 ms\ntask a every p:\n  pass\n",
        &["NE0407"],
    );
    ok("module t for mcu\nconst p = 10 ms\ntask a every p:\n  pass\n");
}

const HEAVY: &str = "module t for mcu\nvar x: u32 = 0\ntask heavy every 1 ms (prio 1):\n  for i in 0..1000:\n    x += 1\n";

#[test]
fn deadline_misses_are_found_with_a_timing_model_ne0402() {
    let opts = SemaOptions {
        cpu_hz: Some(1_000_000),
        ..SemaOptions::default()
    };
    has_with(HEAVY, &opts, "NE0402");
    let fast = SemaOptions {
        cpu_hz: Some(1_000_000_000),
        ..SemaOptions::default()
    };
    let a = run_with(HEAVY, &fast);
    assert!(a.is_ok(), "{}", a.render());
    let t = &a.reports.timing[0];
    assert!(t.wcet_ns.is_some() && t.response_ns.is_some());
    assert_eq!(t.schedulable, Some(true));
}

#[test]
fn without_a_timing_model_the_analysis_is_reported_unavailable() {
    let a = run(HEAVY);
    assert!(a.is_ok(), "{}", a.render());
    assert!(all_codes(&a).contains(&"NL0402"), "{}", a.render());
    assert!(a.reports.timing[0].wcet_ns.is_none());
}

#[test]
fn response_time_includes_higher_priority_work_and_blocking() {
    let src = "module t for mcu\nvar shared: u32 = 0\ntask hi every 1 ms (prio 3):\n  shared += 1\ntask lo every 10 ms (prio 1):\n  for i in 0..20:\n    shared += 1\n";
    let opts = SemaOptions {
        cpu_hz: Some(1_000_000),
        ..SemaOptions::default()
    };
    let a = run_with(src, &opts);
    let hi = a
        .reports
        .timing
        .iter()
        .find(|t| t.name == "hi")
        .expect("hi");
    let lo = a
        .reports
        .timing
        .iter()
        .find(|t| t.name == "lo")
        .expect("lo");
    // `hi` can be blocked by the critical section of `lo` (the whole loop statement).
    assert!(hi.blocking_ns > 0, "{:?}", a.reports.timing);
    assert!(hi.response_ns.expect("response") >= hi.wcet_ns.expect("wcet") + hi.blocking_ns);
    // `lo` is never blocked by a higher priority task and is preempted by `hi`.
    assert_eq!(lo.blocking_ns, 0);
    assert!(lo.response_ns.expect("response") > lo.wcet_ns.expect("wcet"));
}

#[test]
fn interrupts_need_a_minimum_interval_to_be_analyzed_nl0403() {
    let src = "module t for mcu\nio b: gpio.input @ D2\nvar n: u16 = 0\non interrupt b.falling (prio 2):\n  n += 1\n";
    let a = run(src);
    assert!(all_codes(&a).contains(&"NL0403"), "{}", a.render());
    let with = "module t for mcu\nio b: gpio.input @ D2\nvar n: u16 = 0\n@min_interval(5 ms)\non interrupt b.falling (prio 2):\n  n += 1\n";
    let a = run(with);
    assert!(!all_codes(&a).contains(&"NL0403"), "{}", a.render());
}

#[test]
fn memory_budget_ne0604() {
    let src =
        "module t for mcu\nvar buf: [u8; 512] = [0; 512]\ntask a every 10 ms:\n  buf[0] = 1\n";
    let small = SemaOptions {
        ram_bytes: Some(256),
        ..SemaOptions::default()
    };
    has_with(src, &small, "NE0604");
    let big = SemaOptions {
        ram_bytes: Some(4096),
        ..SemaOptions::default()
    };
    let a = run_with(src, &big);
    assert!(a.is_ok(), "{}", a.render());
    let m = &a.reports.memory[0];
    assert_eq!(m.static_bytes, 512);
    assert_eq!(
        m.total_bytes,
        m.static_bytes + m.stack_bytes + m.runtime_bytes
    );
}

#[test]
fn declared_stack_budgets_are_checked_ne0604() {
    let src = "module t for mcu\ntask a every 10 ms (stack 8):\n  let big = [0; 64]\n";
    only(src, &["NE0604"]);
    let fine = "module t for mcu\ntask a every 10 ms (stack 512):\n  let big = [0; 64]\n";
    ok(fine);
}

proptest! {
    /// SRP: for any set of tasks sharing one variable, the ceiling is the highest
    /// priority among the accessors, and no task is blocked by a task that has an
    /// equal or higher priority than itself.
    #[test]
    fn srp_ceiling_and_blocking_bounds(prios in proptest::collection::vec(1i64..6, 2..6)) {
        let mut src = String::from("module t for mcu\nvar shared: u32 = 0\n");
        for (i, p) in prios.iter().enumerate() {
            src.push_str(&format!("task t{i} every {} ms (prio {p}):\n  shared += 1\n", 10 * (i + 1)));
        }
        let opts = SemaOptions { cpu_hz: Some(1_000_000_000), ..SemaOptions::default() };
        let a = run_with(&src, &opts);
        prop_assert!(a.is_ok(), "{}", a.render());
        let r = &a.reports.resources[0];
        prop_assert_eq!(r.ceiling, *prios.iter().max().unwrap());
        let longest = a.reports.timing.iter().filter_map(|t| t.wcet_ns).max().unwrap();
        for t in &a.reports.timing {
            let lower_exists = a.reports.timing.iter().any(|o| o.priority < t.priority);
            if !lower_exists {
                prop_assert_eq!(t.blocking_ns, 0);
            }
            // SRP-3: blocking is bounded by one critical section of a lower priority task.
            prop_assert!(t.blocking_ns <= longest);
        }
    }

    /// MEM-1: the reported total is the sum of its parts for any mix of arrays.
    #[test]
    fn memory_total_is_the_sum_of_parts(sizes in proptest::collection::vec(1u32..64, 1..5)) {
        let mut src = String::from("module t for mcu\n");
        let mut expected = 0u64;
        for (i, n) in sizes.iter().enumerate() {
            src.push_str(&format!("var a{i}: [u8; {n}] = [0; {n}]\n"));
            expected += u64::from(*n);
        }
        let a = run(&src);
        prop_assert!(a.is_ok(), "{}", a.render());
        let m = &a.reports.memory[0];
        prop_assert_eq!(m.static_bytes, expected);
        prop_assert_eq!(m.total_bytes, m.static_bytes + m.stack_bytes + m.runtime_bytes);
    }
}
