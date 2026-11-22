//! Task D6.2: the replay format of the twin, its JSON Schema, and replays of
//! co-simulation runs and counterexamples.
//! Regenerate the schema with `tools/rust-check.ps1 -Golden` (sets `UPDATE_GOLDEN=1`).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{
    cosimulate, partition, replay_schema_json, verify_cell, zone_plans, CellPlan, Cut, Drive,
    DriveValue, Library, Replay, ReplayEventKind, ReplayKind, SimOptions, SimRun, CELL_DEVICE,
};
use neuron_nir::Nir;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use serde_json::Value;
use std::path::Path;

const MS: u64 = 1_000_000;

fn src(name: &str, text: &str) -> SourceFile {
    SourceFile {
        path: name.into(),
        text: text.into(),
    }
}

struct Cell {
    sources: Vec<SourceFile>,
    nir: Nir,
    plan: CellPlan,
    lib: Library,
}

fn load(sources: Vec<SourceFile>) -> Cell {
    let a = analyze(sources.clone(), None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    Cell {
        sources,
        nir,
        plan,
        lib,
    }
}

fn pair() -> Cell {
    load(vec![
        src(
            "producer.neuron",
            "module producer for host\nsignal ready: u32\n",
        ),
        src(
            "consumer.neuron",
            "module consumer for host\nsignal start: u32\nio led: gpio.output @ GPIO17\ntask poll every 5 ms:\n  led.write(start > 0)\nsafety:\n  safe_state:\n    led.write(false)\n",
        ),
        src(
            "pair.neuron",
            "cell Pair\n  device a: raspberry_pi_5 @ ip(192.168.0.30)\n  device b: raspberry_pi_5 @ ip(192.168.0.31)\n  place producer on a\n  place consumer on b\n  link a.ready -> b.start via modbus_tcp every 10 ms within 200 ms\n",
        ),
    ])
}

fn run_of(c: &Cell, drives: Vec<Drive>, cuts: Vec<Cut>) -> SimRun {
    cosimulate(
        &c.plan,
        &c.nir,
        &c.sources,
        &c.lib,
        &SimOptions {
            end_ns: 400 * MS,
            drives,
            cuts,
            ..SimOptions::default()
        },
    )
    .unwrap()
}

fn drive(t_ms: u64, v: i64) -> Drive {
    Drive {
        t_ns: t_ms * MS,
        device: "a".into(),
        name: "ready".into(),
        value: DriveValue::Int(v),
    }
}

// ---- a small JSON Schema checker (the subset schemars writes) --------------------

fn type_ok(v: &Value, t: &str) -> bool {
    match t {
        "string" => v.is_string(),
        "integer" => v.is_u64() || v.is_i64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => false,
    }
}

fn check(v: &Value, s: &Value, root: &Value, path: &str, errs: &mut Vec<String>) {
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        check(
            v,
            &root["$defs"][r.trim_start_matches("#/$defs/")],
            root,
            path,
            errs,
        );
        return;
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(alts) = s.get(key).and_then(Value::as_array) {
            let any = alts.iter().any(|a| {
                let mut e = Vec::new();
                check(v, a, root, path, &mut e);
                e.is_empty()
            });
            if !any {
                errs.push(format!("{path}: matches none of the alternatives"));
            }
            return;
        }
    }
    if let Some(c) = s.get("const") {
        if c != v {
            errs.push(format!("{path}: expected {c}, got {v}"));
        }
    }
    if let Some(e) = s.get("enum").and_then(Value::as_array) {
        if !e.contains(v) {
            errs.push(format!("{path}: {v} is not one of {e:?}"));
        }
    }
    if let Some(t) = s.get("type") {
        let names: Vec<&str> = match t {
            Value::String(x) => vec![x.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !names.iter().any(|n| type_ok(v, n)) {
            errs.push(format!("{path}: expected {names:?}"));
            return;
        }
    }
    if let (Some(obj), Some(props)) = (
        v.as_object(),
        s.get("properties").and_then(Value::as_object),
    ) {
        for req in s
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let k = req.as_str().unwrap();
            if !obj.contains_key(k) {
                errs.push(format!("{path}: missing `{k}`"));
            }
        }
        for (k, val) in obj {
            match props.get(k) {
                Some(ps) => check(val, ps, root, &format!("{path}.{k}"), errs),
                None if s.get("additionalProperties") == Some(&Value::Bool(false)) => {
                    errs.push(format!("{path}: unknown field `{k}`"));
                }
                None => {}
            }
        }
    }
    if let (Some(items), Some(is)) = (v.as_array(), s.get("items")) {
        for (i, it) in items.iter().enumerate() {
            check(it, is, root, &format!("{path}[{i}]"), errs);
        }
    }
}

fn validates(text: &str) -> Vec<String> {
    let schema: Value = serde_json::from_str(&replay_schema_json()).unwrap();
    let doc: Value = serde_json::from_str(text).unwrap();
    let mut errs = Vec::new();
    check(&doc, &schema, &schema, "$", &mut errs);
    errs
}

// ---- tests -----------------------------------------------------------------------

#[test]
fn the_committed_schema_matches_the_types() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/schemas/replay.schema.json");
    let got = replay_schema_json();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .expect("missing spec/schemas/replay.schema.json; run tools/rust-check.ps1 -Golden")
        .replace("\r\n", "\n");
    assert!(
        want == got,
        "the replay schema changed; review it and run tools/rust-check.ps1 -Golden"
    );
}

#[test]
fn the_schema_names_its_root_and_closes_its_objects() {
    let s: Value = serde_json::from_str(&replay_schema_json()).unwrap();
    assert_eq!(s["title"], "Replay");
    for def in ["ReplayEvent", "ReplayDevice", "ReplayLink"] {
        assert_eq!(s["$defs"][def]["additionalProperties"], false, "{def}");
    }
    assert_eq!(s["additionalProperties"], false);
}

#[test]
fn the_checker_rejects_what_the_schema_forbids() {
    let c = pair();
    let good = Replay::from_run(&run_of(&c, vec![drive(7, 3)], vec![])).to_json();
    assert!(validates(&good).is_empty(), "{:?}", validates(&good));
    let mut v: Value = serde_json::from_str(&good).unwrap();
    v["events"][0]["extra"] = Value::from(1);
    assert!(!validates(&v.to_string()).is_empty(), "unknown field");
    let mut v: Value = serde_json::from_str(&good).unwrap();
    v["events"][0]["kind"] = Value::from("explosion");
    assert!(!validates(&v.to_string()).is_empty(), "unknown kind");
    let mut v: Value = serde_json::from_str(&good).unwrap();
    v.as_object_mut().unwrap().remove("cell");
    assert!(!validates(&v.to_string()).is_empty(), "missing field");
    let mut v: Value = serde_json::from_str(&good).unwrap();
    v["end_ns"] = Value::from("soon");
    assert!(!validates(&v.to_string()).is_empty(), "wrong type");
}

#[test]
fn a_cosimulation_replay_has_the_run_and_round_trips() {
    let c = pair();
    let run = run_of(
        &c,
        vec![drive(7, 3), drive(60, 9)],
        vec![Cut {
            link: 0,
            at_ns: 120 * MS,
        }],
    );
    let r = Replay::from_run(&run);
    assert_eq!(r.kind, ReplayKind::Cosimulation);
    assert_eq!((r.cell.as_str(), r.end_ns), ("Pair", 400 * MS));
    assert_eq!(r.devices.len(), 2);
    assert_eq!(r.links.len(), 1);
    assert_eq!(r.links[0].latency_ns, run.links[0].latency_ns);
    assert_eq!(r.events.len(), run.events.len());
    assert!(r.events.windows(2).all(|w| w[0].t_ns <= w[1].t_ns));
    let kinds = |k| r.events.iter().filter(|e| e.kind == k).count();
    assert_eq!(kinds(ReplayEventKind::Deliver), run.deliveries.len());
    assert_eq!(kinds(ReplayEventKind::Cut), 1);
    assert_eq!(kinds(ReplayEventKind::Fault), 1);
    assert!(kinds(ReplayEventKind::Signal) > 0 && kinds(ReplayEventKind::Output) > 0);
    // Canonical text: two-space indent, one final newline, and it reads back identical.
    let text = r.to_json();
    assert!(text.ends_with("}\n") && !text.ends_with("\n\n"));
    assert!(text.starts_with("{\n  \"replay_version\": \"1\""));
    assert!(validates(&text).is_empty(), "{:?}", validates(&text));
    let back = Replay::from_json(&text).unwrap();
    assert_eq!(back, r);
    assert_eq!(back.to_json(), text);
    // The order of events is the order of the run.
    let names: Vec<_> = r.events.iter().map(|e| e.name.as_str()).collect();
    let run_names: Vec<_> = run.events.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, run_names);
}

#[test]
fn a_replay_of_another_version_or_with_extra_fields_is_refused() {
    let c = pair();
    let text = Replay::from_run(&run_of(&c, vec![], vec![])).to_json();
    assert!(Replay::from_json(
        &text.replace("\"replay_version\": \"1\"", "\"replay_version\": \"2\"")
    )
    .unwrap_err()
    .contains("unsupported"));
    assert!(Replay::from_json(&text.replacen("\"cell\"", "\"cells\"", 1)).is_err());
    assert!(Replay::from_json("{}").is_err());
}

const VERIFY_CELL: &str = "cell V
  device plc: generic_s7_1200 @ ip(192.168.0.10)
  device arm: generic_scara @ ip(192.168.0.20)
  link plc.part_ready -> arm.pick via modbus_tcp every 10 ms within 30 ms
  sequence A on plc:
    initial step Idle:
      until plc.part_ready
    step Work:
      wait 100 ms
  sequence B on arm:
    initial step Idle:
      until arm.pick
    step Picking:
      wait 100 ms
  verify:
    never B.Picking
";

#[test]
fn a_counterexample_becomes_a_replay_that_reaches_the_same_violation() {
    let c = load(vec![src("v.neuron", VERIFY_CELL)]);
    let (zones, _) = zone_plans(&c.nir, &c.plan, &c.lib);
    let verdict = verify_cell(&c.sources, &c.nir, &c.plan, &c.lib, &zones);
    let cex = &verdict.violations[0];
    let r = Replay::from_counterexample(&c.plan, &verdict, cex);
    assert_eq!(r.kind, ReplayKind::Counterexample);
    assert_eq!(r.devices.len(), 2);
    assert_eq!(r.links.len(), 1);
    assert_eq!(r.links[0].latency_ns, verdict.channels[0].delay_ns);
    // The violation is the property, at the time of the last step.
    let last = cex.trace.last().unwrap();
    assert_eq!(r.violation(), Some((last.time_ns, "never B.Picking")));
    assert_eq!(r.events.last().unwrap().kind, ReplayEventKind::Violation);
    // Folding the state events up to then gives the states of the last step.
    let folded = r.states_at(last.time_ns);
    let want: std::collections::BTreeMap<String, String> = last
        .states
        .iter()
        .map(|s| {
            let (m, st) = s.rsplit_once('.').unwrap();
            (m.to_string(), st.to_string())
        })
        .collect();
    assert_eq!(folded, want);
    assert_eq!(folded.get("B").map(String::as_str), Some("Picking"));
    // Before the last step the property still held.
    let before = r.states_at(cex.trace[cex.trace.len() - 2].time_ns);
    assert_ne!(before.get("B").map(String::as_str), Some("Picking"));
    // The first input of the trace is a signal event on the device that owns it.
    assert!(r.events.iter().any(|e| {
        e.kind == ReplayEventKind::Signal
            && e.device == "plc"
            && e.name == "part_ready"
            && e.value == "true"
    }));
    assert!(r
        .events
        .iter()
        .all(|e| e.device == CELL_DEVICE || r.devices.iter().any(|d| d.name == e.device)));
    let text = r.to_json();
    assert!(validates(&text).is_empty(), "{:?}", validates(&text));
    assert_eq!(Replay::from_json(&text).unwrap(), r);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10))]

    /// Every replay of a random run validates against the schema, reads back equal and
    /// is byte-identical when produced twice.
    #[test]
    fn replays_of_random_runs_validate_and_are_canonical(
        times in proptest::collection::btree_set(1u64..250, 0..5),
        cut in proptest::option::of(1u64..300),
    ) {
        let c = pair();
        let make = || {
            let drives = times.iter().map(|t| drive(*t, i64::try_from(*t).unwrap())).collect();
            let cuts = cut.map(|c| Cut { link: 0, at_ns: c * MS }).into_iter().collect();
            Replay::from_run(&run_of(&c, drives, cuts)).to_json()
        };
        let (a, b) = (make(), make());
        prop_assert_eq!(&a, &b);
        prop_assert!(validates(&a).is_empty());
        prop_assert_eq!(Replay::from_json(&a).unwrap().to_json(), a);
    }
}
