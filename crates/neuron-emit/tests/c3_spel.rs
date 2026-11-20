//! Task C3: the SPEL+-style emitter, checked by running its output.
//!
//! Every program of `examples/corpus/robot` is emitted, parsed and run in the virtual
//! controller of `neuron-spel`, and run in the reference interpreter with the same input
//! schedule. The motions (instant, kind, resolved target) and the output changes (instant,
//! bit, value) must be equal; numbers are compared with a tolerance of 1e-6 mm or degrees,
//! instants exactly (the controller uses the interpreter's 1 s per motion and a 1 ms grid).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_emit::robot::Robot;
use neuron_emit::{emitter, nirx, Class, EmitOptions, Package};
use neuron_interp::{parse_scenario, Interp, Options, TraceKind};
use neuron_nir::{ItemDef, Nir};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_spel::{Controller, Limits, MotionKind, Outcome, Target};
use proptest::prelude::*;

const R01: &str = include_str!("../../../examples/corpus/robot/r01_home_and_wave.neuron");
const R02: &str = include_str!("../../../examples/corpus/robot/r02_palletize.neuron");
const R03: &str = include_str!("../../../examples/corpus/robot/r03_gripper_check.neuron");
const R04: &str = include_str!("../../../examples/corpus/robot/r04_welding_seam.neuron");
const R05: &str = include_str!("../../../examples/corpus/robot/r05_inspect_grid.neuron");
const R06: &str = include_str!("../../../examples/corpus/robot/r06_arc_path.neuron");

fn nir_of(src: &str) -> Nir {
    let a = analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: src.replace("\r\n", "\n"),
        }],
        None,
        &SemaOptions::default(),
    );
    match a.nir.clone() {
        Some(n) => n,
        None => panic!("analysis errors:\n{}", a.render()),
    }
}

fn emit(nir: &Nir, robot: &str) -> Result<Package, Vec<neuron_emit::Diag>> {
    emitter("spel").unwrap().emit(
        nir,
        &EmitOptions {
            robot: robot.into(),
            ..EmitOptions::default()
        },
    )
}

/// A target in a form both sides can be compared in.
#[derive(Debug, Clone, PartialEq)]
enum T {
    J(Vec<f64>),
    /// Number of the local (1-based, declaration order of the frames) and `x, y, z` in mm.
    P(usize, [f64; 3]),
}

fn close(a: &T, b: &T) -> bool {
    let near = |x: &[f64], y: &[f64]| {
        x.len() == y.len() && x.iter().zip(y).all(|(p, q)| (p - q).abs() <= 1e-6)
    };
    match (a, b) {
        (T::J(x), T::J(y)) => near(x, y),
        (T::P(f, x), T::P(g, y)) => f == g && near(x, y),
        _ => false,
    }
}

fn frames(nir: &Nir) -> Vec<String> {
    nir.modules[0]
        .items
        .iter()
        .filter_map(|i| match &i.def {
            ItemDef::Geometry { kind, name, .. } if kind == "frame" => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn io_bits(nir: &Nir) -> Vec<(String, i64)> {
    nir.modules[0]
        .items
        .iter()
        .filter_map(|i| match &i.def {
            ItemDef::Io { name, at, .. } => match &at.e {
                neuron_nir::ExprKind::Call { args, .. } => {
                    nirx::int(&args[0].value).map(|b| (name.clone(), b))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn nums(s: &str) -> Vec<f64> {
    s.split(", ").map(|x| x.trim().parse().unwrap()).collect()
}

/// `joints(0, -30, 60, 0, 60, 0) deg` or `pallet.xyz(80, 60, 50) mm`.
fn from_interp(s: &str, frames: &[String]) -> T {
    if let Some(r) = s.strip_prefix("joints(") {
        return T::J(nums(r.strip_suffix(") deg").unwrap()));
    }
    let (f, rest) = s.split_once(".xyz(").unwrap();
    let v = nums(rest.strip_suffix(") mm").unwrap());
    let k = frames.iter().position(|n| n == f).unwrap() + 1;
    T::P(k, [v[0], v[1], v[2]])
}

fn from_spel(t: &Target) -> T {
    match t {
        Target::Joints(v) => T::J(v.clone()),
        Target::Point { local, xyz, .. } => T::P(local.unwrap() as usize, *xyz),
    }
}

fn limits(r: &Robot) -> Limits {
    Limits {
        reach_mm: r.reach_mm,
        shoulder_mm: r.shoulder_height_mm,
        z_min_mm: r.z_min_mm,
        z_max_mm: r.z_max_mm,
        joints: r.joint_limits_deg.clone(),
    }
}

struct Case {
    name: &'static str,
    src: &'static str,
    /// `(ms, input, value)`
    inputs: Vec<(i64, &'static str, bool)>,
    /// Name and error number of the expected fault.
    fault: Option<(&'static str, i64)>,
    motions: usize,
    changes: usize,
}

/// Runs both sides and compares them. Returns `(motions, changes)` compared.
fn differential(c: &Case) -> (usize, usize) {
    let nir = nir_of(c.src);
    let pkg = emit(&nir, "").unwrap_or_else(|d| panic!("{}: {d:?}", c.name));
    let file = format!("{}.prg", nir.modules[0].name);
    let text = pkg.files.get(&file).unwrap_or_else(|| panic!("no {file}"));
    let prog = neuron_spel::parse(text).unwrap_or_else(|e| panic!("{}: {e}\n{text}", c.name));
    let bits = io_bits(&nir);
    let bit_of = |n: &str| bits.iter().find(|(x, _)| x == n).unwrap().1;
    let fr = frames(&nir);

    // The controller.
    let mut ctl = Controller::new(prog, neuron_spel::Options::default());
    ctl.set_limits(limits(&Robot::builtin("generic_6axis").unwrap()));
    for (ms, name, on) in &c.inputs {
        ctl.set_input(ms * 1_000_000, bit_of(name), *on);
    }
    let outcome = ctl.run();

    // The reference.
    let scenario: String = c
        .inputs
        .iter()
        .map(|(ms, n, on)| format!("{ms} ms: {n} = {on}\n"))
        .collect();
    let mut it = Interp::new(
        nir.clone(),
        Options {
            end_ns: 120_000 * 1_000_000,
            ..Options::default()
        },
    )
    .unwrap();
    it.load_scenario(&parse_scenario(&scenario).unwrap());
    it.run();
    let trace = it.trace().to_vec();

    // Motions: instant, kind, targets.
    let want: Vec<_> = trace
        .iter()
        .filter(|e| e.kind == TraceKind::Motion)
        .collect();
    let got = ctl.motions();
    assert_eq!(got.len(), want.len(), "{}: number of motions", c.name);
    assert_eq!(want.len(), c.motions, "{}: motions", c.name);
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g.t_ns, w.t_ns, "{}: instant of motion {i}", c.name);
        let kind = match g.kind {
            MotionKind::Ptp => "ptp",
            MotionKind::Linear => "linear",
            MotionKind::Arc => "arc",
        };
        assert_eq!(kind, w.name, "{}: kind of motion {i}", c.name);
        let (end, via) = match w.value.split_once(" via ") {
            Some((a, b)) => (a, Some(b)),
            None => (w.value.as_str(), None),
        };
        let (ge, we) = (from_spel(&g.target), from_interp(end, &fr));
        assert!(
            close(&ge, &we),
            "{}: target of motion {i}: {ge:?} vs {we:?}",
            c.name
        );
        match (&g.via, via) {
            (None, None) => {}
            (Some(gv), Some(wv)) => {
                let (gv, wv) = (from_spel(gv), from_interp(wv, &fr));
                assert!(
                    close(&gv, &wv),
                    "{}: via of motion {i}: {gv:?} vs {wv:?}",
                    c.name
                );
            }
            other => panic!("{}: via of motion {i}: {other:?}", c.name),
        }
    }

    // Output changes: instant, bit, value.
    let want: Vec<(i64, i64, bool)> = trace
        .iter()
        .filter(|e| e.kind == TraceKind::Output)
        .map(|e| (e.t_ns, bit_of(&e.name), e.value == "true"))
        .collect();
    let got: Vec<(i64, i64, bool)> = ctl
        .outputs()
        .iter()
        .map(|o| (o.t_ns, o.bit, o.on))
        .collect();
    assert_eq!(got, want, "{}: output changes", c.name);
    assert_eq!(want.len(), c.changes, "{}: changes", c.name);
    let n_changes = want.len();

    // The fault.
    match c.fault {
        None => {
            assert_eq!(it.current_fault(), None, "{}", c.name);
            assert_eq!(ctl.fault(), None, "{}", c.name);
            assert_eq!(outcome, Outcome::Finished, "{}", c.name);
        }
        Some((name, code)) => {
            assert_eq!(it.current_fault(), Some(name), "{}", c.name);
            assert_eq!(ctl.fault(), Some(code), "{}", c.name);
            assert_eq!(outcome, Outcome::Quit, "{}", c.name);
            assert!(
                pkg.wiring.contains(&format!("| {code} | `{name}` |")),
                "{}",
                c.name
            );
        }
    }
    (c.motions, n_changes)
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "r01",
            src: R01,
            inputs: vec![],
            fault: None,
            motions: 8,
            changes: 0,
        },
        Case {
            name: "r02 part sensed",
            src: R02,
            inputs: vec![(0, "part_ok", true)],
            fault: None,
            motions: 37,
            changes: 1,
        },
        Case {
            name: "r02 no part",
            src: R02,
            inputs: vec![],
            fault: Some(("PartLost", 8001)),
            motions: 3,
            changes: 2,
        },
        Case {
            name: "r03 held",
            src: R03,
            inputs: vec![(0, "holding", true)],
            fault: None,
            motions: 2,
            changes: 2,
        },
        Case {
            name: "r03 held late",
            src: R03,
            inputs: vec![(1500, "holding", true)],
            fault: None,
            motions: 2,
            changes: 2,
        },
        Case {
            name: "r03 never held",
            src: R03,
            inputs: vec![],
            fault: Some(("GripLost", 8001)),
            motions: 1,
            changes: 2,
        },
        Case {
            name: "r04",
            src: R04,
            inputs: vec![],
            fault: None,
            motions: 4,
            changes: 2,
        },
        Case {
            name: "r05",
            src: R05,
            inputs: vec![],
            fault: None,
            motions: 8,
            changes: 12,
        },
        Case {
            name: "r06",
            src: R06,
            inputs: vec![],
            fault: None,
            motions: 3,
            changes: 0,
        },
    ]
}

#[test]
fn every_robot_program_runs_the_same_in_the_controller_and_the_interpreter() {
    let mut total = (0, 0);
    for c in cases() {
        let (m, o) = differential(&c);
        eprintln!("{:<18} motions {m:>3}  output changes {o:>3}", c.name);
        total.0 += m;
        total.1 += o;
    }
    eprintln!(
        "total: {} motions, {} output changes compared",
        total.0, total.1
    );
}

#[test]
fn the_output_is_deterministic() {
    for src in [R01, R02, R03, R04, R05, R06] {
        let nir = nir_of(src);
        let a = emit(&nir, "").unwrap().all_files();
        let b = emit(&nir, "").unwrap().all_files();
        assert_eq!(a, b);
    }
}

#[test]
fn the_palletizer_follows_the_mapping_of_the_design() {
    let pkg = emit(&nir_of(R02), "").unwrap();
    let t = &pkg.files["r02_palletize.prg"];
    for want in [
        "Local 1, XY(400, -200, 0, 90, 0, 0)",
        "TLSet 1, XY(0, 0, 120, 0, 0, 0)",
        "Weight 1.2",
        "P1 = JA(0, -20, 70, 0, 40, 0)",
        "PLabel 1, \"home\"",
        "For r = 0 To 2",
        "For c = 0 To 3",
        "/L1",
        "Move P2 +Z(50)",
        "CP On",
        "CP Off",
        "SpeedS 400",
        "Speed 60",
        "On 3",
        "Wait Sw(5) = On, 0.5",
        "If TW Then GoTo Retry",
        "Retry:",
        "Error 8001",
        "OnErr GoTo NeuronFault",
        "Function NeuronSafe",
        "Off 3",
        "Next c",
        "Next r",
    ] {
        assert!(t.contains(want), "missing `{want}` in\n{t}");
    }
    assert!(pkg.files.contains_key("NeuronLib.prg"));
    assert!(!t.contains("NeuronCheck"), "constant targets need no check");
    // A motion with a blend radius is approximate.
    let f = pkg.fidelity.to_json();
    let v: serde_json::Value = serde_json::from_str(&f).unwrap();
    assert!(v["summary"]["approximate"].as_u64().unwrap() >= 1, "{f}");
    let entries = v["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["class"] == "approximate" && e["construct"] == "blend"),
        "{f}"
    );
    assert!(pkg
        .fidelity
        .entries
        .iter()
        .all(|e| e.class <= Class::Approximate));
    // The slave and the source maps.
    let slave = &pkg.files["NeuronSlave.prg"];
    for want in ["GO", "MOVE", "OUT", "IN", "SPEED", "HOME", "\"OK\"", "ERR"] {
        assert!(slave.contains(want), "slave lacks {want}");
    }
    let map = &pkg.maps["r02_palletize.prg"];
    assert!(map.mappings.len() > 10);
    assert!(map
        .mappings
        .iter()
        .any(|m| m.view.as_deref() == Some("sequence:stack")));
    assert!(map
        .mappings
        .iter()
        .any(|m| m.view.as_deref() == Some("step:Retry")));
}

fn probe(decls: &str, body: &str) -> String {
    format!(
        "module probe for robot\n\nframe f = frame(origin: xyz(200, 0, 0) mm, rot: rpy(0, 0, 0) deg)\n{decls}\nsequence go():\n{body}"
    )
}

fn codes(r: Result<Package, Vec<neuron_emit::Diag>>) -> Vec<&'static str> {
    match r {
        Ok(_) => vec![],
        Err(d) => d.iter().map(|d| d.code).collect(),
    }
}

#[test]
fn an_unreachable_constant_point_is_ne0804() {
    // Beyond the reach, below the floor, a joint out of range, a wrong number of joints.
    for decl in [
        "point p = f.xyz(900, 0, 0) mm\n",
        "point p = f.xyz(0, 0, -400) mm\n",
        "point p = f.xyz(0, 0, 2000) mm\n",
        "point p = joints(0, 0, 0, 0, 0, 400) deg\n",
        "point p = joints(0, 0, 0, 0, 0) deg\n",
    ] {
        let nir = nir_of(&probe(decl, "  move ptp to p speed 10 %\n"));
        assert_eq!(codes(emit(&nir, "")), vec!["NE0804"], "{decl}");
    }
    let ok = nir_of(&probe(
        "point p = f.xyz(100, 0, 0) mm\n",
        "  move ptp to p speed 10 %\n",
    ));
    assert!(emit(&ok, "").is_ok());
    // The diagnostic points at the target.
    let src = probe(
        "point p = f.xyz(900, 0, 0) mm\n",
        "  move ptp to p speed 10 %\n",
    );
    let d = emit(&nir_of(&src), "").unwrap_err();
    let s = &src.replace("\r\n", "\n")[d[0].span[0]..d[0].span[1]];
    assert!(s.contains("f.xyz(900"), "{s}");
}

#[test]
fn a_target_in_a_loop_is_checked_for_every_iteration() {
    // Iterations 0 to 4 are inside the reach; iteration 5 is not (x = 200 + 5 * 100 = 700).
    let bad = probe(
        "",
        "  for i in 0..6:\n    move linear to f.xyz(i * 100, 0, 0) mm speed 100 mm/s\n",
    );
    assert_eq!(codes(emit(&nir_of(&bad), "")), vec!["NE0804"]);
    let good = probe(
        "",
        "  for i in 0..5:\n    move linear to f.xyz(i * 100, 0, 0) mm speed 100 mm/s\n",
    );
    let pkg = emit(&nir_of(&good), "").unwrap();
    assert!(!pkg.files["probe.prg"].contains("NeuronCheck"));
}

#[test]
fn a_computed_target_calls_neuron_check_which_raises_the_workspace_error() {
    let src = probe(
        "var reach: f64 = 100\n",
        "  move linear to f.xyz(reach, 0, 0) mm speed 100 mm/s\n",
    );
    let pkg = emit(&nir_of(&src), "").unwrap();
    let t = &pkg.files["probe.prg"];
    assert!(t.contains("Call NeuronCheck("), "{t}");
    assert!(pkg.files["NeuronLib.prg"].contains("Function NeuronCheck"));
    // In the controller the check passes for a target in the workspace and raises for one outside.
    let run = |reach: &str| {
        let text = t.replace("reach = 100", &format!("reach = {reach}"));
        let mut c = Controller::new(
            neuron_spel::parse(&text).unwrap(),
            neuron_spel::Options::default(),
        );
        c.set_limits(limits(&Robot::builtin("generic_6axis").unwrap()));
        c.run();
        (c.fault(), c.motions().len())
    };
    assert_eq!(run("100"), (None, 1));
    assert_eq!(run("900"), (Some(neuron_spel::WORKSPACE_ERROR), 0));
}

#[test]
fn a_scara_manifest_rejects_a_six_joint_target() {
    let nir = nir_of(R01);
    assert_eq!(codes(emit(&nir, "generic_scara")), vec!["NE0804"; 3]);
    assert!(emit(&nir, "nope").is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// A joint target is accepted exactly when every joint is inside the manifest limits.
    #[test]
    fn joint_targets_are_accepted_iff_inside_the_limits(
        j in proptest::collection::vec(-400i32..400, 6)
    ) {
        let r = Robot::builtin("generic_6axis").unwrap();
        let inside = j
            .iter()
            .zip(&r.joint_limits_deg)
            .all(|(v, l)| f64::from(*v) >= l[0] && f64::from(*v) <= l[1]);
        let src = probe(
            &format!(
                "point p = joints({}) deg\n",
                j.iter().map(i32::to_string).collect::<Vec<_>>().join(", ")
            ),
            "  move ptp to p speed 10 %\n",
        );
        let res = emit(&nir_of(&src), "");
        prop_assert_eq!(res.is_ok(), inside, "{:?}", codes(res));
    }
}
