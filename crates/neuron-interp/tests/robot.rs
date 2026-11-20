//! Stage 1 of C3: the reference interpreter runs the robot corpus.
//!
//! The motion observable (`MOVE`), geometry values, waits inside loops, sequence locals
//! that live across ticks and `until` on an `io` input are checked against the six
//! programs of `examples/corpus/robot`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_interp::{parse_scenario, Interp, Options, TraceEvent, TraceKind};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;

const R01: &str = include_str!("../../../examples/corpus/robot/r01_home_and_wave.neuron");
const R02: &str = include_str!("../../../examples/corpus/robot/r02_palletize.neuron");
const R03: &str = include_str!("../../../examples/corpus/robot/r03_gripper_check.neuron");
const R04: &str = include_str!("../../../examples/corpus/robot/r04_welding_seam.neuron");
const R05: &str = include_str!("../../../examples/corpus/robot/r05_inspect_grid.neuron");
const R06: &str = include_str!("../../../examples/corpus/robot/r06_arc_path.neuron");

fn run(src: &str, scenario: &str, ms: i64) -> (Vec<TraceEvent>, Option<String>) {
    let a = analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: src.replace("\r\n", "\n"),
        }],
        None,
        &SemaOptions::default(),
    );
    let nir = match a.nir {
        Some(n) => n,
        None => panic!("analysis errors:\n{}", a.render()),
    };
    let opts = Options {
        end_ns: ms * 1_000_000,
        ..Options::default()
    };
    let mut it = Interp::new(nir, opts).unwrap();
    it.load_scenario(&parse_scenario(scenario).unwrap());
    it.run();
    (it.trace().to_vec(), it.current_fault().map(str::to_string))
}

/// `(ms, kind, value)` of every motion.
fn moves(t: &[TraceEvent]) -> Vec<(i64, String, String)> {
    t.iter()
        .filter(|e| e.kind == TraceKind::Motion)
        .map(|e| (e.t_ns / 1_000_000, e.name.clone(), e.value.clone()))
        .collect()
}

/// `(ms, value)` of every change of the output `name`.
fn outs(t: &[TraceEvent], name: &str) -> Vec<(i64, String)> {
    t.iter()
        .filter(|e| e.kind == TraceKind::Output && e.name == name)
        .map(|e| (e.t_ns / 1_000_000, e.value.clone()))
        .collect()
}

fn count(t: &[TraceEvent], kind: TraceKind) -> usize {
    t.iter().filter(|e| e.kind == kind).count()
}

fn s(x: &str) -> String {
    x.to_string()
}

#[test]
fn r01_visits_home_then_waves_three_times() {
    let (t, fault) = run(R01, "", 10_000);
    let home = "joints(0, -30, 60, 0, 60, 0) deg";
    let left = "joints(-30, -30, 60, 0, 60, 0) deg";
    let right = "joints(30, -30, 60, 0, 60, 0) deg";
    let mut want = vec![(0, s("ptp"), s(home))];
    for i in 0..3 {
        want.push((1000 + 2000 * i, s("ptp"), s(left)));
        want.push((2000 + 2000 * i, s("ptp"), s(right)));
    }
    want.push((7000, s("ptp"), s(home)));
    assert_eq!(moves(&t), want, "{}", neuron_interp::render(&t));
    assert_eq!(fault, None);
}

#[test]
fn r02_stacks_twelve_parts_when_the_part_is_sensed() {
    let (t, fault) = run(R02, "0 ms: part_ok = true", 40_000);
    assert_eq!(fault, None, "{}", neuron_interp::render(&t));
    assert_eq!(count(&t, TraceKind::Fault), 0);
    let m = moves(&t);
    assert_eq!(m.len(), 37);
    assert_eq!(m[0], (0, s("ptp"), s("joints(0, -20, 70, 0, 40, 0) deg")));
    for k in 0..12i64 {
        let (r, c) = (k / 4, k % 4);
        let at = |z: i64| format!("pallet.xyz({}, {}, {z}) mm", c * 80, r * 60);
        let base = 1000 + 3000 * k;
        let i = 1 + 3 * k as usize;
        assert_eq!(m[i], (base, s("linear"), at(50)), "cycle {k}");
        assert_eq!(m[i + 1], (base + 1000, s("linear"), at(0)), "cycle {k}");
        assert_eq!(m[i + 2], (base + 2000, s("linear"), at(50)), "cycle {k}");
    }
    // The gripper closes once, when the first part is lowered.
    assert_eq!(outs(&t, "suction"), vec![(3000, s("true"))]);
}

#[test]
fn r02_without_a_part_times_out_and_faults() {
    let (t, fault) = run(R02, "", 10_000);
    assert_eq!(
        fault.as_deref(),
        Some("PartLost"),
        "{}",
        neuron_interp::render(&t)
    );
    let m = moves(&t);
    assert_eq!(m.len(), 3, "{}", neuron_interp::render(&t));
    assert_eq!(m[1], (1000, s("linear"), s("pallet.xyz(0, 0, 50) mm")));
    assert_eq!(m[2], (2000, s("linear"), s("pallet.xyz(0, 0, 0) mm")));
    assert_eq!(
        outs(&t, "suction"),
        vec![(3000, s("true")), (3500, s("false"))]
    );
    let timeouts: Vec<_> = t
        .iter()
        .filter(|e| e.kind == TraceKind::Event && e.value == "timeout")
        .map(|e| (e.t_ns / 1_000_000, e.name.clone()))
        .collect();
    assert_eq!(timeouts, vec![(3500, s("stack"))]);
    let faults: Vec<_> = t
        .iter()
        .filter(|e| e.kind == TraceKind::Fault)
        .map(|e| (e.t_ns / 1_000_000, e.name.clone()))
        .collect();
    assert_eq!(faults, vec![(3500, s("PartLost"))]);
}

#[test]
fn r03_transfers_when_the_part_is_held() {
    let (t, fault) = run(R03, "0 ms: holding = true", 5000);
    assert_eq!(fault, None, "{}", neuron_interp::render(&t));
    assert_eq!(
        moves(&t),
        vec![
            (0, s("ptp"), s("joints(20, -35, 65, 0, 60, 0) deg")),
            (1000, s("ptp"), s("joints(-40, -35, 65, 0, 60, 0) deg")),
        ]
    );
    assert_eq!(
        outs(&t, "close"),
        vec![(1000, s("true")), (2000, s("false"))]
    );
}

#[test]
fn r03_faults_when_the_part_is_lost() {
    let (t, fault) = run(R03, "", 5000);
    assert_eq!(
        fault.as_deref(),
        Some("GripLost"),
        "{}",
        neuron_interp::render(&t)
    );
    assert_eq!(
        moves(&t),
        vec![(0, s("ptp"), s("joints(20, -35, 65, 0, 60, 0) deg"))]
    );
    assert_eq!(
        outs(&t, "close"),
        vec![(1000, s("true")), (1800, s("false"))]
    );
}

#[test]
fn r04_follows_the_seam_with_the_torch_on() {
    let (t, fault) = run(R04, "", 6000);
    assert_eq!(fault, None);
    assert_eq!(
        moves(&t),
        vec![
            (0, s("ptp"), s("part.xyz(0, 0, 80) mm")),
            (1000, s("linear"), s("part.xyz(0, 0, 5) mm")),
            (2000, s("linear"), s("part.xyz(200, 0, 5) mm")),
            (3000, s("linear"), s("part.xyz(0, 0, 80) mm")),
        ]
    );
    assert_eq!(outs(&t, "arc"), vec![(2000, s("true")), (3000, s("false"))]);
}

#[test]
fn r05_inspects_the_grid_cell_by_cell() {
    let (t, fault) = run(R05, "", 12_000);
    assert_eq!(fault, None);
    let m = moves(&t);
    assert_eq!(m.len(), 8);
    let home = "joints(0, -25, 55, 0, 60, 0) deg";
    assert_eq!(m[0], (0, s("ptp"), s(home)));
    assert_eq!(m[7], (8200, s("ptp"), s(home)));
    let mut on_off = Vec::new();
    for k in 0..6i64 {
        let (r, c) = (k / 3, k % 3);
        let want = format!("bench.xyz({}, {}, 150) mm", c * 60, r * 60);
        assert_eq!(
            m[1 + k as usize],
            (1000 + 1200 * k, s("linear"), want),
            "cell {k}"
        );
        on_off.push((2000 + 1200 * k, s("true")));
        on_off.push((2200 + 1200 * k, s("false")));
    }
    assert_eq!(outs(&t, "camera"), on_off);
}

#[test]
fn r06_draws_a_half_circle_with_an_arc() {
    let (t, fault) = run(R06, "", 4000);
    assert_eq!(fault, None);
    assert_eq!(
        moves(&t),
        vec![
            (0, s("linear"), s("board.xyz(0, -50, 0) mm")),
            (
                1000,
                s("arc"),
                s("board.xyz(0, 50, 0) mm via board.xyz(50, 0, 0) mm")
            ),
            (2000, s("linear"), s("board.xyz(0, -50, 0) mm")),
        ]
    );
}

/// A one-sequence robot module that moves to `target` expressions.
fn probe(decls: &str, targets: &[&str]) -> Vec<String> {
    let mut src = String::from("module probe for robot\n\nframe f = frame(origin: xyz(100, 0, 0) mm, rot: rpy(0, 0, 0) deg)\n");
    src.push_str(decls);
    src.push_str("\nsequence go():\n");
    for t in targets {
        src.push_str(&format!("  move linear to {t} speed 100 mm/s\n"));
    }
    let (tr, _) = run(&src, "", 1000 * targets.len() as i64 + 10);
    moves(&tr).into_iter().map(|m| m.2).collect()
}

#[test]
fn geometry_values_resolve_to_millimetres_and_degrees() {
    let got = probe(
        "point a = f.xyz(1, 2, 3) mm\npoint b = f.xyz(1, 2, 0.5) m\npoint j = joints(0, 90, 0, 0, 0, 0) deg\n",
        &[
            "a",
            "a.above(50 mm)",
            "b",
            "b.above(2 cm)",
            "j",
            "f.xyz(7, 8, 9) mm",
        ],
    );
    assert_eq!(
        got,
        vec![
            "f.xyz(1, 2, 3) mm",
            "f.xyz(1, 2, 53) mm",
            "f.xyz(1000, 2000, 500) mm",
            "f.xyz(1000, 2000, 520) mm",
            "joints(0, 90, 0, 0, 0, 0) deg",
            "f.xyz(7, 8, 9) mm",
        ]
    );
}

#[test]
fn sequence_locals_live_across_ticks_in_a_waiting_loop() {
    let src = "module probe for robot\n\nframe f = frame(origin: xyz(0, 0, 0) mm, rot: rpy(0, 0, 0) deg)\n\nsequence go():\n  for i in 0..4:\n    let p = f.xyz(i * 10, 0, 0) mm\n    move linear to p speed 100 mm/s\n    wait 100 ms\n    move linear to p.above(5 mm) speed 100 mm/s\n";
    let (t, _) = run(src, "", 10_000);
    let m = moves(&t);
    assert_eq!(m.len(), 8, "{}", neuron_interp::render(&t));
    assert_eq!(m[0].2, "f.xyz(0, 0, 0) mm");
    assert_eq!(m[1], (1100, s("linear"), s("f.xyz(0, 0, 5) mm")));
    assert_eq!(m[2], (2100, s("linear"), s("f.xyz(10, 0, 0) mm")));
    assert_eq!(m[7].2, "f.xyz(30, 0, 5) mm");
}

/// A gripper release: open, then wait until the part is gone (`{cond}`), then move.
fn release(cond: &str) -> String {
    format!(
        "module probe for robot\n\nio close: gpio.output @ dout(1)\nio holding: gpio.input @ din(2)\n\npoint away = joints(0, 0, 0, 0, 0, 0) deg\n\nsequence release():\n  close.write(false)\n  until {cond} timeout 800 ms -> Stuck\n  move ptp to away speed 50 %\n  step Stuck:\n    fault Stuck\n\nsafety:\n  safe_state:\n    close.write(false)\n"
    )
}

#[test]
fn until_on_a_negated_io_input_waits_until_the_input_falls() {
    for cond in ["not holding", "holding == false", "false == holding"] {
        let src = release(cond);
        // The input falls at 300 ms: the wait ends there and the motion starts.
        let (t, fault) = run(&src, "0 ms: holding = true\n300 ms: holding = false", 3000);
        assert_eq!(fault, None, "{cond}\n{}", neuron_interp::render(&t));
        let m = moves(&t);
        assert_eq!(m.len(), 1, "{cond}\n{}", neuron_interp::render(&t));
        assert!(
            (300..=302).contains(&m[0].0),
            "{cond}: motion at {} ms",
            m[0].0
        );
        // The input stays high: the time-out branch is taken.
        let (t, fault) = run(&src, "0 ms: holding = true", 3000);
        assert_eq!(fault.as_deref(), Some("Stuck"), "{cond}");
        assert!(moves(&t).is_empty(), "{cond}");
        // The input is low from the start: no wait at all.
        let (t, fault) = run(&src, "", 3000);
        assert_eq!(fault, None, "{cond}");
        assert_eq!(moves(&t)[0].0, 0, "{cond}");
    }
}

#[test]
fn if_on_a_negated_io_input_in_a_sequence_reads_the_input() {
    let src = "module probe for robot\n\nio lamp: gpio.output @ dout(1)\nio part_ok: gpio.input @ din(2)\n\nsequence check():\n  if not part_ok:\n    lamp.write(true)\n  wait 10 ms\n\nsafety:\n  safe_state:\n    lamp.write(false)\n";
    let (t, _) = run(src, "0 ms: part_ok = true", 100);
    assert!(outs(&t, "lamp").is_empty(), "{}", neuron_interp::render(&t));
    let (t, _) = run(src, "", 100);
    assert_eq!(outs(&t, "lamp"), vec![(0, s("true"))]);
}

#[test]
fn an_mcu_task_writes_the_negation_of_an_input() {
    let src = "module probe for mcu on arduino_uno_r3\n\nio led: gpio.output @ D13\nio button: gpio.input @ D2\n\ntask follow every 10 ms (prio 1):\n  led.write(not button)\n\nsafety:\n  safe_state:\n    led.write(false)\n";
    let (t, _) = run(src, "0 ms: button = true\n55 ms: button = false", 100);
    let o = outs(&t, "led");
    let fell = o.iter().find(|(_, v)| v == "true").map(|(ms, _)| *ms);
    assert_eq!(fell, Some(60), "{o:?}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// `p.above(d mm)` is the same point `d` higher along the frame's Z axis.
    #[test]
    fn above_adds_to_z_only(x in -500i32..500, y in -500i32..500, z in -500i32..500, d in 0i32..400) {
        let got = probe(
            &format!("point p = f.xyz({x}, {y}, {z}) mm\n"),
            &[&format!("p.above({d} mm)")],
        );
        prop_assert_eq!(got, vec![format!("f.xyz({x}, {y}, {}) mm", z + d)]);
    }
}
