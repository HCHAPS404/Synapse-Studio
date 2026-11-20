//! Task C5.1 and C5.2: the importers, checked by round trip.
//!
//! Every `plc` program of the corpus is emitted as Structured Text (generic and
//! `yaskawa` profiles), SCL and PLCopen XML, imported back, analyzed and run in the
//! reference interpreter with the random input schedules of the C2 tests; the output
//! trace must equal the original's. Every `robot` program is emitted as SPEL+, imported
//! back and run in the interpreter; motions and output changes must equal the original's.
//! Hand-written inputs check the translation of common constructs and the diagnostics.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use neuron_emit::{emitter, Class, EmitOptions, Package};
use neuron_import::{import, import_with, languages, Options};
use neuron_interp::{
    parse_scenario, Action, Interp, Options as IOptions, Scenario, Step, TraceKind, Value as IV,
};
use neuron_nir::{ExprKind, ItemDef, Nir, Type};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corpus(dir: &str, extra: &[&str]) -> Vec<(String, String)> {
    let d = root().join(dir);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    files.sort();
    files.extend(extra.iter().map(|e| root().join(e)));
    files
        .into_iter()
        .map(|f| {
            (
                f.file_stem().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&f).unwrap().replace("\r\n", "\n"),
            )
        })
        .collect()
}

fn plc_programs() -> Vec<(String, String)> {
    corpus("examples/corpus/plc", &["examples/conveyor_plc.neuron"])
}

fn robot_programs() -> Vec<(String, String)> {
    corpus("examples/corpus/robot", &[])
}

fn nir_of(name: &str, text: &str) -> Nir {
    let a = analyze(
        vec![SourceFile {
            path: format!("{name}.neuron"),
            text: text.into(),
        }],
        None,
        &SemaOptions::default(),
    );
    a.nir
        .clone()
        .unwrap_or_else(|| panic!("{name}:\n{}\n---\n{text}", a.render()))
}

#[derive(Clone, Debug)]
enum In {
    B(bool),
    I(i64),
}

fn inputs(nir: &Nir) -> Vec<(String, bool)> {
    let mut v = Vec::new();
    for item in &nir.modules[0].items {
        if let ItemDef::PlcSection { section, vars } = &item.def {
            if section == "inputs" {
                for x in vars {
                    v.push((x.name.clone(), matches!(x.vty, Type::Bool)));
                }
            }
        }
    }
    v
}

fn scan_ns(nir: &Nir) -> i64 {
    for item in &nir.modules[0].items {
        if let ItemDef::Scan { period } = &item.def {
            return neuron_emit::nirx::dur_ns(period).unwrap();
        }
    }
    10_000_000
}

/// Random inputs, each changing 3 ms after a scan instant (the schedules of the C2 tests).
fn schedule(nir: &Nir, seed: u64, end: i64) -> Vec<(i64, String, In)> {
    let scan = scan_ns(nir);
    let mut rng = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let mut next = || {
        rng = rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        rng >> 33
    };
    let ins = inputs(nir);
    let mut state: Vec<bool> = vec![false; ins.len()];
    let mut out = Vec::new();
    let mut t = 3_000_000;
    let pulses = seed == 5;
    while t < end {
        if pulses {
            for (k, (name, is_bool)) in ins.iter().enumerate() {
                let odds = if k == 0 { 3 } else { 60 };
                if *is_bool && next() % odds == 0 {
                    out.push((t, name.clone(), In::B(true)));
                    out.push((t + scan * 2, name.clone(), In::B(false)));
                }
            }
            t += scan * 4;
            continue;
        }
        for (k, (name, is_bool)) in ins.iter().enumerate() {
            if *is_bool {
                if next() % 9 == 0 {
                    state[k] = !state[k];
                    out.push((t, name.clone(), In::B(state[k])));
                }
            } else if next() % 15 == 0 {
                out.push((t, name.clone(), In::I(700 + (next() % 300) as i64)));
            }
        }
        let pause = if next() % 25 == 0 {
            300
        } else {
            1 + (next() % 4) as i64
        };
        t += scan * pause;
    }
    out
}

type Obs = Vec<(i64, String, String)>;

fn run_plc_ref(nir: &Nir, sched: &[(i64, String, In)], end: i64) -> Obs {
    let mut it = Interp::new(
        nir.clone(),
        IOptions {
            end_ns: end,
            ..IOptions::default()
        },
    )
    .unwrap();
    let steps = sched
        .iter()
        .map(|(t, n, v)| Step {
            t_ns: *t,
            action: Action::Set(
                n.clone(),
                match v {
                    In::B(b) => IV::Bool(*b),
                    In::I(i) => IV::Int(i128::from(*i)),
                },
            ),
        })
        .collect();
    it.load_scenario(&Scenario { steps });
    it.run();
    it.trace()
        .iter()
        .filter(|e| e.kind == TraceKind::Output)
        .map(|e| (e.t_ns, e.name.clone(), e.value.clone()))
        .collect()
}

fn st_source(pkg: &Package) -> String {
    let mut parts: Vec<(&String, &String)> = pkg
        .files
        .iter()
        .filter(|(n, _)| n.ends_with(".st"))
        .collect();
    parts.sort_by_key(|(n, _)| match n.as_str() {
        "NeuronLib.st" => 0,
        "GlobalVars.st" => 1,
        "config.st" => 3,
        _ => 2,
    });
    parts
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn scl_source(pkg: &Package) -> String {
    let mut parts: Vec<(&String, &String)> = pkg
        .files
        .iter()
        .filter(|(n, _)| n.ends_with(".scl"))
        .collect();
    parts.sort_by_key(|(n, _)| u8::from(n.as_str() != "NeuronLib.scl"));
    parts
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

const END: i64 = 8_000_000_000;

/// Compares the traces of the original and the imported program under `seeds` schedules.
/// Returns the number of output changes compared.
fn compare_plc(label: &str, orig: &Nir, imported_src: &str, name: &str, seeds: u64) -> usize {
    let imp = nir_of(name, imported_src);
    assert_eq!(
        scan_ns(orig),
        scan_ns(&imp),
        "{label}: scan period\n{imported_src}"
    );
    let mut total = 0;
    let mut most = 0;
    for seed in 1..=seeds {
        let sched = schedule(orig, seed, END);
        let want = run_plc_ref(orig, &sched, END);
        let got = run_plc_ref(&imp, &sched, END);
        most = most.max(want.len());
        if got != want {
            let k = got
                .iter()
                .zip(&want)
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(want.len()));
            panic!(
                "{label} seed {seed}: first difference at change {k}: imported {:?}, original {:?} ({} vs {} changes)\n{imported_src}",
                got.get(k),
                want.get(k),
                got.len(),
                want.len()
            );
        }
        total += want.len();
    }
    assert!(most >= 2, "{label}: no seed exercised the program");
    total
}

fn opts(nir: &Nir) -> Options {
    Options {
        scan_ns: Some(scan_ns(nir)),
        file: String::new(),
    }
}

#[test]
fn structured_text_round_trips_for_every_plc_program() {
    let st = emitter("st").unwrap();
    let mut programs = 0;
    let mut events = 0;
    for profile in ["generic", "yaskawa"] {
        for (name, text) in plc_programs() {
            let orig = nir_of(&name, &text);
            let pkg = st
                .emit(
                    &orig,
                    &EmitOptions {
                        profile: profile.into(),
                        ..EmitOptions::default()
                    },
                )
                .unwrap();
            let src = st_source(&pkg);
            let got = import_with("st", &name, &src, &opts(&orig))
                .unwrap_or_else(|d| panic!("{name} {profile}: {d:?}\n{src}"));
            assert!(
                got.complete(),
                "{name} {profile}: {:?}\n{}",
                got.diags,
                got.source
            );
            events += compare_plc(&format!("{name} {profile}"), &orig, &got.source, &name, 5);
            programs += 1;
        }
    }
    eprintln!("ST round trip: {programs} programs, {events} output changes compared");
    assert_eq!(programs, 22);
}

#[test]
fn the_task_interval_of_the_configuration_gives_the_scan_period() {
    // The generic profile writes a CONFIGURATION: no option is needed.
    let (name, text) = plc_programs()
        .into_iter()
        .find(|(n, _)| n.starts_with("p06"))
        .unwrap();
    let orig = nir_of(&name, &text);
    let pkg = emitter("st")
        .unwrap()
        .emit(&orig, &EmitOptions::default())
        .unwrap();
    let got = import("st", &name, &st_source(&pkg)).unwrap();
    assert!(got.source.contains("scan 20 ms"), "{}", got.source);
    // The yaskawa profile does not: the default is assumed and reported.
    let pkg = emitter("st")
        .unwrap()
        .emit(
            &orig,
            &EmitOptions {
                profile: "yaskawa".into(),
                ..EmitOptions::default()
            },
        )
        .unwrap();
    let got = import("st", &name, &st_source(&pkg)).unwrap();
    assert!(got.source.contains("scan 10 ms"), "{}", got.source);
    assert!(got
        .fidelity
        .entries
        .iter()
        .any(|e| e.construct == "scan period" && e.class == Class::Equivalent));
}

#[test]
fn scl_round_trips_for_every_plc_program() {
    let scl = emitter("scl").unwrap();
    let mut events = 0;
    let mut programs = 0;
    for (name, text) in plc_programs() {
        let orig = nir_of(&name, &text);
        let pkg = scl.emit(&orig, &EmitOptions::default()).unwrap();
        let src = scl_source(&pkg);
        let got = import_with("scl", &name, &src, &opts(&orig))
            .unwrap_or_else(|d| panic!("{name}: {d:?}\n{src}"));
        assert!(got.complete(), "{name}: {:?}\n{}", got.diags, got.source);
        events += compare_plc(&format!("{name} scl"), &orig, &got.source, &name, 5);
        programs += 1;
    }
    eprintln!("SCL round trip: {programs} programs, {events} output changes compared");
    assert_eq!(programs, 11);
}

#[test]
fn plcopen_round_trips_for_every_plc_program() {
    let px = emitter("plcopen").unwrap();
    let (mut ld, mut sfc, mut st) = (0, 0, 0);
    let mut events = 0;
    let mut programs = 0;
    for (name, text) in plc_programs() {
        let orig = nir_of(&name, &text);
        let pkg = px.emit(&orig, &EmitOptions::default()).unwrap();
        let xml = pkg.files.values().next().unwrap().clone();
        if xml.contains("<LD>") {
            ld += 1;
        } else if xml.contains("<SFC>") {
            sfc += 1;
        } else {
            st += 1;
        }
        // The task interval is in the XML: no option is needed.
        let got = import("plcopen", &name, &xml).unwrap_or_else(|d| panic!("{name}: {d:?}\n{xml}"));
        assert!(got.complete(), "{name}: {:?}\n{}", got.diags, got.source);
        events += compare_plc(&format!("{name} plcopen"), &orig, &got.source, &name, 5);
        programs += 1;
    }
    eprintln!("PLCopen round trip: {programs} programs ({ld} LD, {sfc} SFC, {st} ST bodies), {events} output changes compared");
    assert!(ld >= 5 && sfc == 1 && st >= 1);
}

#[test]
fn a_sequence_comes_back_as_a_sequence_from_st_and_from_sfc() {
    let (name, text) = plc_programs()
        .into_iter()
        .find(|(n, _)| n.starts_with("p09"))
        .unwrap();
    let orig = nir_of(&name, &text);
    let from_st = import_with(
        "st",
        &name,
        &st_source(
            &emitter("st")
                .unwrap()
                .emit(&orig, &EmitOptions::default())
                .unwrap(),
        ),
        &opts(&orig),
    )
    .unwrap();
    let from_sfc = import(
        "plcopen",
        &name,
        emitter("plcopen")
            .unwrap()
            .emit(&orig, &EmitOptions::default())
            .unwrap()
            .files
            .values()
            .next()
            .unwrap(),
    )
    .unwrap();
    for (what, got) in [("st", &from_st), ("sfc", &from_sfc)] {
        for want in [
            "initial step Idle:",
            "until start",
            "step Fill:",
            "inlet = true",
            "until full",
            "inlet = false",
            "step Stir:",
            "wait 5 s",
            "step Drain:",
            "-> Idle",
        ] {
            assert!(
                got.source.contains(want),
                "{what}: missing `{want}` in\n{}",
                got.source
            );
        }
        assert!(got
            .fidelity
            .entries
            .iter()
            .any(|e| e.construct.starts_with("sequence") && e.class == Class::Equivalent));
    }
}

#[test]
fn the_imported_source_is_formatted_and_deterministic() {
    let mut checked = 0;
    for (name, text) in plc_programs() {
        let orig = nir_of(&name, &text);
        let sources = [
            (
                "st",
                st_source(
                    &emitter("st")
                        .unwrap()
                        .emit(&orig, &EmitOptions::default())
                        .unwrap(),
                ),
            ),
            (
                "scl",
                scl_source(
                    &emitter("scl")
                        .unwrap()
                        .emit(&orig, &EmitOptions::default())
                        .unwrap(),
                ),
            ),
            (
                "plcopen",
                emitter("plcopen")
                    .unwrap()
                    .emit(&orig, &EmitOptions::default())
                    .unwrap()
                    .files
                    .values()
                    .next()
                    .unwrap()
                    .clone(),
            ),
        ];
        for (lang, src) in sources {
            let o = opts(&orig);
            let a = import_with(lang, &name, &src, &o).unwrap();
            let b = import_with(lang, &name, &src, &o).unwrap();
            assert_eq!(a.source, b.source, "{name} {lang}");
            assert_eq!(a.fidelity.to_json(), b.fidelity.to_json(), "{name} {lang}");
            let f = neuron_syntax::format::format_source(&a.source).unwrap();
            assert!(!f.fell_back, "{name} {lang}");
            assert_eq!(f.text, a.source, "{name} {lang}: not formatted");
            checked += 1;
        }
    }
    for (name, text) in robot_programs() {
        let orig = nir_of(&name, &text);
        let pkg = emitter("spel")
            .unwrap()
            .emit(&orig, &EmitOptions::default())
            .unwrap();
        let prg = &pkg.files[&format!("{name}.prg")];
        let a = import("spel", &name, prg).unwrap();
        let b = import("spel", &name, prg).unwrap();
        assert_eq!(a.source, b.source);
        let f = neuron_syntax::format::format_source(&a.source).unwrap();
        assert!(!f.fell_back && f.text == a.source, "{name}: not formatted");
        checked += 1;
    }
    eprintln!("{checked} imported sources are formatted and deterministic");
}

// ---- SPEL+ -----------------------------------------------------------------------------

fn io_bits(nir: &Nir) -> Vec<(String, i64)> {
    nir.modules[0]
        .items
        .iter()
        .filter_map(|i| match &i.def {
            ItemDef::Io { name, at, .. } => match &at.e {
                ExprKind::Call { args, .. } => {
                    neuron_emit::nirx::int(&args[0].value).map(|b| (name.clone(), b))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

struct Run {
    motions: Vec<(i64, String, String)>,
    outputs: Vec<(i64, i64, String)>,
    fault: Option<String>,
}

/// Runs a robot program with `(ms, bit, value)` input changes.
fn run_robot(nir: &Nir, inputs: &[(i64, i64, bool)]) -> Run {
    let bits = io_bits(nir);
    let name_of = |b: i64| bits.iter().find(|(_, x)| *x == b).map(|(n, _)| n.clone());
    let scenario: String = inputs
        .iter()
        .filter_map(|(ms, b, on)| name_of(*b).map(|n| format!("{ms} ms: {n} = {on}\n")))
        .collect();
    let mut it = Interp::new(
        nir.clone(),
        IOptions {
            end_ns: 120_000 * 1_000_000,
            ..IOptions::default()
        },
    )
    .unwrap();
    it.load_scenario(&parse_scenario(&scenario).unwrap());
    it.run();
    let trace = it.trace().to_vec();
    Run {
        motions: trace
            .iter()
            .filter(|e| e.kind == TraceKind::Motion)
            .map(|e| (e.t_ns, e.name.clone(), e.value.clone()))
            .collect(),
        outputs: trace
            .iter()
            .filter(|e| e.kind == TraceKind::Output)
            .map(|e| {
                let bit = bits
                    .iter()
                    .find(|(n, _)| *n == e.name)
                    .map_or(-1, |(_, b)| *b);
                (e.t_ns, bit, e.value.clone())
            })
            .collect(),
        fault: it.current_fault().map(str::to_string),
    }
}

/// `(program, input changes as (ms, bit, value))`: both branches of `r02` and `r03`.
type RobotCase = (&'static str, &'static str, Vec<(i64, i64, bool)>);

fn robot_cases() -> Vec<RobotCase> {
    vec![
        ("r01", "r01_home_and_wave", vec![]),
        ("r02 part sensed", "r02_palletize", vec![(0, 5, true)]),
        ("r02 no part", "r02_palletize", vec![]),
        ("r03 held", "r03_gripper_check", vec![(0, 2, true)]),
        ("r03 held late", "r03_gripper_check", vec![(1500, 2, true)]),
        ("r03 never held", "r03_gripper_check", vec![]),
        ("r04", "r04_welding_seam", vec![]),
        ("r05", "r05_inspect_grid", vec![]),
        ("r06", "r06_arc_path", vec![]),
    ]
}

#[test]
fn spel_round_trips_for_every_robot_program() {
    let progs = robot_programs();
    let mut motions = 0;
    let mut changes = 0;
    let mut runs = 0;
    for (label, file, inputs) in robot_cases() {
        let (name, text) = progs.iter().find(|(n, _)| n == file).unwrap();
        let orig = nir_of(name, text);
        let pkg = emitter("spel")
            .unwrap()
            .emit(&orig, &EmitOptions::default())
            .unwrap();
        let prg = &pkg.files[&format!("{name}.prg")];
        let got = import("spel", name, prg).unwrap_or_else(|d| panic!("{label}: {d:?}\n{prg}"));
        assert!(got.complete(), "{label}: {:?}\n{}", got.diags, got.source);
        let imp = nir_of(name, &got.source);
        let want = run_robot(&orig, &inputs);
        let have = run_robot(&imp, &inputs);
        assert_eq!(
            have.motions, want.motions,
            "{label}: motions\n{}",
            got.source
        );
        assert_eq!(
            have.outputs, want.outputs,
            "{label}: output changes\n{}",
            got.source
        );
        assert_eq!(have.fault, want.fault, "{label}: fault\n{}", got.source);
        motions += want.motions.len();
        changes += want.outputs.len();
        runs += 1;
        eprintln!(
            "{label:<18} motions {:>3}  output changes {:>3}  fault {:?}",
            want.motions.len(),
            want.outputs.len(),
            want.fault
        );
    }
    eprintln!(
        "SPEL+ round trip: {runs} runs, {motions} motions and {changes} output changes compared"
    );
}

#[test]
fn the_blend_radius_is_not_in_a_spel_program_and_is_reported() {
    let progs = robot_programs();
    let (name, text) = progs.iter().find(|(n, _)| n.starts_with("r02")).unwrap();
    let orig = nir_of(name, text);
    let pkg = emitter("spel")
        .unwrap()
        .emit(&orig, &EmitOptions::default())
        .unwrap();
    let got = import("spel", name, &pkg.files["r02_palletize.prg"]).unwrap();
    assert!(got.source.contains("blend 10 mm"), "{}", got.source);
    assert!(got
        .fidelity
        .entries
        .iter()
        .any(|e| e.construct == "blend" && e.class == Class::Approximate));
    assert!(got
        .source
        .contains("frame pallet = frame(origin: xyz(400, -200, 0) mm, rot: rpy(0, 0, 90) deg)"));
    assert!(got
        .source
        .contains("tool vacuum = tool(offset: xyz(0, 0, 120) mm, payload: 1.2 kg)"));
    assert!(
        got.source.contains("until din5 timeout 500 ms -> Retry"),
        "{}",
        got.source
    );
    assert!(got.source.contains("fault PartLost"));
}

// ---- hand-written inputs ---------------------------------------------------------------

const LATCH: &str = "\
(* A motor with a start latch and an automatic stop two seconds later. *)
PROGRAM latch
VAR
    start AT %IX0.0 : BOOL;
    stop AT %IX0.1 : BOOL;
    motor AT %QX0.0 : BOOL;
    hold : TON;
END_VAR
hold(IN := motor, PT := T#2S);
IF start THEN
    motor := TRUE;
END_IF;
IF stop OR hold.Q THEN
    motor := FALSE;
END_IF;
END_PROGRAM

CONFIGURATION cfg
    RESOURCE res ON PLC
        TASK fast (INTERVAL := T#20MS, PRIORITY := 1);
        PROGRAM main WITH fast : latch;
    END_RESOURCE
END_CONFIGURATION
";

fn run_inputs(nir: &Nir, steps: &[(i64, &str, bool)], end: i64) -> Obs {
    let sched: Vec<(i64, String, In)> = steps
        .iter()
        .map(|(t, n, v)| (*t, (*n).to_string(), In::B(*v)))
        .collect();
    run_plc_ref(nir, &sched, end)
}

#[test]
fn a_hand_written_latch_with_a_timer() {
    let got = import("st", "latch", LATCH).unwrap();
    assert!(got.complete(), "{:?}", got.diags);
    for want in [
        "module latch for plc",
        "scan 20 ms",
        "start: bool @ %I0.0",
        "motor: bool @ %Q0.0",
        "let hold = ton(preset: 2 s)",
        "hold.input = motor",
        "set motor when start",
        "reset motor when stop or hold.q",
        "motor = false",
    ] {
        assert!(
            got.source.contains(want),
            "missing `{want}` in\n{}",
            got.source
        );
    }
    // The timer is the one construct that is not exact.
    let timers: Vec<_> = got
        .fidelity
        .entries
        .iter()
        .filter(|e| e.construct.starts_with("timer"))
        .collect();
    assert_eq!(timers.len(), 1);
    assert_eq!(timers[0].class, Class::Equivalent);
    // The source had no safe state: one is added and reported.
    assert!(got
        .fidelity
        .entries
        .iter()
        .any(|e| e.construct == "safe state" && e.class == Class::Approximate));
    let nir = nir_of("latch", &got.source);
    let obs = run_inputs(
        &nir,
        &[(100_000_000, "start", true), (140_000_000, "start", false)],
        4_000_000_000,
    );
    let motor: Vec<(i64, &str)> = obs
        .iter()
        .filter(|o| o.1 == "motor")
        .map(|o| (o.0, o.2.as_str()))
        .collect();
    let on = motor
        .iter()
        .find(|m| m.1 == "true")
        .unwrap_or_else(|| panic!("{obs:?}"));
    assert!(on.0 < 200_000_000, "{obs:?}");
    // It stops by itself about two seconds after it started.
    let off = motor
        .iter()
        .find(|m| m.0 > on.0 && m.1 == "false")
        .unwrap_or_else(|| panic!("{obs:?}"));
    assert!(
        (1_900_000_000..2_300_000_000).contains(&(off.0 - on.0)),
        "{obs:?}"
    );
}

#[test]
fn a_hand_written_counter_with_a_memory_variable() {
    let st = "\
PROGRAM counter
VAR
    item AT %IX0.0 : BOOL;
    clear AT %IX0.1 : BOOL;
    full AT %QX0.0 : BOOL;
    total AT %MW10 : INT;
    batch : CTU;
END_VAR
batch(CU := item, R := clear, PV := 5);
full := batch.Q;
IF item THEN
    total := total + 1;
END_IF;
END_PROGRAM
";
    let got = import_with(
        "st",
        "counter",
        st,
        &Options {
            scan_ns: Some(10_000_000),
            file: "counter.st".into(),
        },
    )
    .unwrap();
    assert!(got.complete(), "{:?}", got.diags);
    for want in [
        "total: i16 @ %MW10",
        "let batch = ctu(preset: 5)",
        "batch.input = item",
        "batch.reset = clear",
        "full = batch.q",
        "total += 1 when item",
    ] {
        assert!(
            got.source.contains(want),
            "missing `{want}` in\n{}",
            got.source
        );
    }
    let nir = nir_of("counter", &got.source);
    let mut steps: Vec<(i64, &str, bool)> = Vec::new();
    for k in 0..6 {
        steps.push((100_000_000 + k * 100_000_000, "item", true));
        steps.push((150_000_000 + k * 100_000_000, "item", false));
    }
    let obs = run_inputs(&nir, &steps, 1_500_000_000);
    let full: Vec<&str> = obs
        .iter()
        .filter(|o| o.1 == "full")
        .map(|o| o.2.as_str())
        .collect();
    assert_eq!(full.last(), Some(&"true"), "{obs:?}");
    // The gate opens with the fifth pulse, not before.
    let t_full = obs
        .iter()
        .find(|o| o.1 == "full" && o.2 == "true")
        .unwrap()
        .0;
    assert!((500_000_000..620_000_000).contains(&t_full), "{t_full}");
}

#[test]
fn a_construct_without_a_neuron_equivalent_is_ne1002_with_a_fidelity_entry() {
    let st = "\
PROGRAM loops
VAR
    a AT %IX0.0 : BOOL;
    q AT %QX0.0 : BOOL;
    n : INT;
END_VAR
WHILE n < 10 DO
    n := n + 1;
END_WHILE;
q := a;
END_PROGRAM
";
    let got = import("st", "loops", st).unwrap();
    assert_eq!(got.diags.len(), 1, "{:?}", got.diags);
    let d = &got.diags[0];
    assert_eq!(d.code, "NE1002");
    assert!(d.message.contains("WHILE"), "{}", d.message);
    assert_eq!(
        &st[d.span[0]..d.span[1]],
        "WHILE n < 10 DO\n    n := n + 1;\nEND_WHILE;"
    );
    assert!(!got.complete());
    let e = got
        .fidelity
        .entries
        .iter()
        .find(|e| e.class == Class::Unsupported)
        .unwrap();
    assert!(e.construct.contains("WHILE"));
    assert_eq!(got.fidelity.summary.unsupported, 1);
    assert_eq!(e.span, d.span);
    // The rest of the program is imported, and the gap is visible in the source.
    assert!(got.source.contains("q = a"), "{}", got.source);
    assert!(
        got.source.contains("# not imported (NE1002)"),
        "{}",
        got.source
    );
    nir_of("loops", &got.source);
}

#[test]
fn other_unsupported_constructs_each_get_their_own_ne1002() {
    let st = "\
PROGRAM misc
VAR
    a AT %IX0.0 : BOOL;
    q AT %QX0.0 : BOOL;
    v : INT;
    t : TON;
    user : MyBlock;
END_VAR
CASE v OF
    1: q := TRUE;
END_CASE;
user(x := a);
t(IN := a, PT := v);
IF a THEN
    q := TRUE;
END_IF;
END_PROGRAM
";
    let got = import("st", "misc", st).unwrap();
    let codes: Vec<&str> = got.diags.iter().map(|d| d.code).collect();
    assert_eq!(codes, vec!["NE1002"; 4], "{:?}", got.diags);
    assert!(got.diags[0].message.contains("CASE"));
    assert!(got.diags[1].message.contains("user"));
    assert!(got.diags[2].message.contains("preset"));
    assert!(got.diags[3].message.contains("`user`"));
    assert!(got.source.contains("set q when a"), "{}", got.source);
}

#[test]
fn syntax_errors_are_ne1001_in_every_language() {
    let cases: [(&str, &str); 6] = [
        (
            "st",
            "PROGRAM p\nVAR x : BOOL; END_VAR\nx := ;\nEND_PROGRAM\n",
        ),
        ("st", "PROGRAM p\nVAR x : BOOL; END_VAR\nx := TRUE;\n"),
        (
            "scl",
            "FUNCTION_BLOCK \"p\"\nVAR_INPUT x : Bool\nEND_VAR\nBEGIN\nEND_FUNCTION_BLOCK\n",
        ),
        ("plcopen", "<project><pou></project>"),
        ("plcopen", "<other/>"),
        ("spel", "Function main\n  P1 = JA(0, 0,\nFend\n"),
    ];
    for (lang, text) in cases {
        let d = import(lang, "bad", text).unwrap_err();
        assert!(d.iter().all(|x| x.code == "NE1001"), "{lang}: {d:?}");
        assert!(!d[0].message.is_empty());
    }
    let d = import(
        "st",
        "bad",
        "PROGRAM p\nVAR x : BOOL; END_VAR\nx := ;\nEND_PROGRAM\n",
    )
    .unwrap_err();
    // The error points at the `;` after `:=`.
    assert_eq!(
        d[0].span[0],
        "PROGRAM p\nVAR x : BOOL; END_VAR\nx := ".len()
    );
    let d = import("cobol", "x", "").unwrap_err();
    assert_eq!(d[0].code, "NE1001");
    assert!(d[0].message.contains("cobol"));
    assert_eq!(languages(), &["st", "scl", "plcopen", "spel"]);
}

#[test]
fn a_hand_written_spel_program() {
    let prg = "\
' A pick and place cell written by hand.
Function main
  Motor On
  Local 1, XY(300, 0, 50, 90, 0, 0)  ' frame bench
  P1 = JA(0, -20, 70, 0, 40, 0)
  PLabel 1, \"home\"
  P2 = XY(10, 20, 30, 0, 0, 0) /L1
  PLabel 2, \"dock\"
  Call job
Fend

Function job
  Integer k
  Speed 30
  Go P1
  SpeedS 150
  Move P2
  On 4
  Wait 0.25
  Off 4
  Wait Sw(6) = On, 1.5
  If TW Then GoTo Bad
  For k = 1 To 2
    Move P2 +Z(20)
    Move P2
  Next k
  Go P1
  Exit Function
Bad:
  Error 8002
  Exit Function
Fend

Function NeuronSafe
  Off 4
Fend
";
    let got = import("spel", "pickcell", prg).unwrap();
    assert!(got.complete(), "{:?}\n{}", got.diags, got.source);
    // The formatter aligns the `@` of the `io` lines: compare with single spaces.
    let flat: String = got
        .source
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ") + "\n")
        .collect();
    for want in [
        "module pickcell for robot",
        "frame bench = frame(origin: xyz(300, 0, 50) mm, rot: rpy(0, 0, 90) deg)",
        "io dout4: gpio.output @ dout(4)",
        "io din6: gpio.input @ din(6)",
        "point home = joints(0, -20, 70, 0, 40, 0) deg",
        "point dock = bench.xyz(10, 20, 30) mm",
        "sequence job():",
        "move ptp to home speed 30 %",
        "move linear to dock speed 150 mm/s",
        "dout4.write(true)",
        "wait 250 ms",
        "until din6 timeout 1500 ms -> Bad",
        "for k in 1..3:",
        "dock.above(20 mm)",
        "step Bad:",
        "fault Error8002",
        "dout4.write(false)",
    ] {
        assert!(flat.contains(want), "missing `{want}` in\n{}", got.source);
    }
    let nir = nir_of("pickcell", &got.source);
    // With the input high the wait ends at once and the program runs to the end:
    // 2 motions, 2 iterations of 2, and the last one.
    let run = run_robot(&nir, &[(0, 6, true)]);
    assert_eq!(run.fault, None, "{:?}", run.motions);
    assert_eq!(run.motions.len(), 7);
    assert_eq!(run.motions[0].1, "ptp");
    assert_eq!(run.motions[1].1, "linear");
    // With the input low the condition never holds: the time-out branch raises the fault.
    let run = run_robot(&nir, &[(0, 6, false)]);
    assert_eq!(
        run.fault.as_deref(),
        Some("Error8002"),
        "{:?} {:?}",
        run.motions,
        run.outputs
    );
    // Waiting for an input to go low is `not din6`.
    let low = import(
        "spel",
        "pickcell",
        &prg.replace("Sw(6) = On", "Sw(6) = Off"),
    )
    .unwrap();
    assert!(
        low.source.contains("until not din6 timeout 1500 ms -> Bad"),
        "{}",
        low.source
    );
    // And it behaves like `Wait Sw(6) = Off, 1.5`: with the input low the wait ends at
    // once; with it high the time-out branch raises the fault; when it falls the program
    // goes on from there.
    let low_nir = nir_of("pickcell", &low.source);
    let run = run_robot(&low_nir, &[(0, 6, false)]);
    assert_eq!(run.fault, None, "{:?}", run.motions);
    assert_eq!(run.motions.len(), 7);
    let run = run_robot(&low_nir, &[(0, 6, true)]);
    assert_eq!(run.fault.as_deref(), Some("Error8002"), "{:?}", run.motions);
    // The wait starts at about 2.25 s (two motions and `Wait 0.25`) and times out at
    // about 3.75 s; the input falls at 3 s, inside that window.
    let run = run_robot(&low_nir, &[(0, 6, true), (3000, 6, false)]);
    assert_eq!(run.fault, None, "{:?}", run.motions);
    assert_eq!(run.motions.len(), 7);
    assert!(
        (3_000_000_000..=3_002_000_000).contains(&run.motions[2].0),
        "{:?}",
        run.motions
    );
    // An unsupported line is reported and the rest is imported.
    let with_if = prg.replace(
        "  Go P1\n  Exit Function",
        "  If k > 1 Then\n    Print \"x\"\n  EndIf\n  Go P1\n  Exit Function",
    );
    let got = import("spel", "pickcell", &with_if).unwrap();
    assert!(
        got.diags.iter().any(|d| d.code == "NE1002"),
        "{:?}",
        got.diags
    );
}

#[test]
fn the_fidelity_report_counts_what_was_translated() {
    let (name, text) = plc_programs()
        .into_iter()
        .find(|(n, _)| n.starts_with("conveyor"))
        .unwrap();
    let orig = nir_of(&name, &text);
    let src = st_source(
        &emitter("st")
            .unwrap()
            .emit(&orig, &EmitOptions::default())
            .unwrap(),
    );
    let got = import("st", &name, &src).unwrap();
    assert_eq!(got.fidelity.tool, "import-st");
    assert_eq!(got.fidelity.target, "neuron.plc");
    assert!(
        got.fidelity.summary.exact >= 6,
        "{:?}",
        got.fidelity.summary
    );
    assert_eq!(got.fidelity.summary.equivalent, 1);
    assert_eq!(got.fidelity.summary.unsupported, 0);
    assert!(got.fidelity.to_json().ends_with("}\n"));
}

#[test]
fn a_module_name_that_is_a_keyword_or_not_an_identifier_is_renamed() {
    let st = "PROGRAM cell\nVAR go AT %IX0.0 : BOOL; lamp AT %QX0.0 : BOOL; END_VAR\nIF go THEN lamp := TRUE; END_IF;\nEND_PROGRAM\n";
    let spel = "Function main\n  On 4\nFend\n\nFunction NeuronSafe\n  Off 4\nFend\n";
    for (lang, name, text, want) in [
        // From the PROGRAM name (no name given).
        ("st", "", st, "cell_prog"),
        // From the name given (a file name).
        ("st", "cell", st, "cell_prog"),
        ("st", "2nd-line", st, "m2nd_line"),
        ("spel", "cell", spel, "cell_prog"),
        ("spel", "my cell", spel, "my_cell"),
    ] {
        let got = import(lang, name, text).unwrap();
        assert!(
            got.source.contains(&format!("module {want} for ")),
            "{lang} {name}\n{}",
            got.source
        );
        let renamed: Vec<_> = got
            .fidelity
            .entries
            .iter()
            .filter(|e| e.construct == "module renamed")
            .collect();
        assert_eq!(renamed.len(), 1, "{lang} {name}");
        assert_eq!(renamed[0].class, Class::Equivalent);
        assert!(renamed[0].note.contains(want), "{}", renamed[0].note);
        // The renamed module goes through the analyzer.
        nir_of(want, &got.source);
    }
    // A valid name is kept and not reported.
    let got = import("st", "latch", st).unwrap();
    assert!(got.source.contains("module latch for plc"));
    assert!(got
        .fidelity
        .entries
        .iter()
        .all(|e| e.construct != "module renamed"));
}
