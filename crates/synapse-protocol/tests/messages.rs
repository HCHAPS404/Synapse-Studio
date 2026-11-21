//! Examples in `spec/protocols/examples/` parse, round-trip and cover every message type.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::path::PathBuf;
use synapse_protocol::*;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/protocols/examples")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(dir().join(format!("{name}.json")))
        .unwrap_or_else(|_| panic!("missing example {name}.json"))
}

#[test]
fn every_message_type_has_a_round_tripping_example() {
    for t in Body::TYPES {
        let text = read(t);
        let m = parse(&text).unwrap_or_else(|e| panic!("{t}: {e}"));
        assert_eq!(m.body.type_name(), *t);
        let want: serde_json::Value = serde_json::from_str(&text).unwrap();
        let got: serde_json::Value = serde_json::from_str(&m.to_json()).unwrap();
        assert_eq!(want, got, "{t} does not re-serialize to the same JSON");
    }
}

#[test]
fn no_example_without_a_type() {
    let mut stems = BTreeSet::new();
    for e in std::fs::read_dir(dir()).unwrap() {
        let p = e.unwrap().path();
        stems.insert(p.file_stem().unwrap().to_string_lossy().to_string());
    }
    let mut want: BTreeSet<String> = Body::TYPES.iter().map(|s| s.to_string()).collect();
    want.insert("replay_file".into());
    assert_eq!(stems, want);
}

#[test]
fn replay_file_example_round_trips_and_validates() {
    let text = read("replay_file");
    let r: ReplayFile = serde_json::from_str(&text).unwrap();
    r.validate().unwrap();
    let want: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(want, serde_json::to_value(&r).unwrap());
}

#[test]
fn replay_validation_rejects_disorder_and_unknown_devices() {
    let text = read("replay_file");
    let mut r: ReplayFile = serde_json::from_str(&text).unwrap();
    r.events.reverse();
    assert!(r.validate().is_err());
    r.events.reverse();
    r.events[0].device = "nope".into();
    assert!(r.validate().is_err());
}

#[test]
fn invalid_messages_are_rejected() {
    for bad in [
        // missing type
        r#"{"v":1,"id":1,"t_ns":0}"#,
        // missing v
        r#"{"type":"ping","id":1,"t_ns":0}"#,
        // wrong v
        r#"{"v":2,"type":"ping","id":1,"t_ns":0}"#,
        // v not a number
        r#"{"v":"1","type":"ping","id":1,"t_ns":0}"#,
        // unknown type
        r#"{"v":1,"type":"no_such_type"}"#,
        // missing field
        r#"{"v":1,"type":"ping","id":1}"#,
        // negative unsigned number
        r#"{"v":1,"type":"sim_step","cell":"c","steps":-1}"#,
        "not json",
    ] {
        assert!(parse(bad).is_err(), "accepted: {bad}");
    }
}

#[test]
fn roles_follow_the_table() {
    let sim = parse(&read("sim_start")).unwrap().body;
    let estop = parse(&read("advisory_estop_request")).unwrap().body;
    let tele = parse(&read("telemetry")).unwrap().body;
    let status = parse(&read("deployment_status")).unwrap().body;
    let sub = parse(&read("subscribe")).unwrap().body;
    for r in [Role::Guest, Role::Viewer] {
        assert!(r.may_send(&sub));
        assert!(!r.may_send(&sim) && !r.may_send(&estop) && !r.may_send(&tele));
    }
    assert!(Role::Operator.may_send(&sim) && Role::Operator.may_send(&estop));
    assert!(!Role::Operator.may_send(&tele) && !Role::Operator.may_send(&status));
    assert!(Role::Twin.may_send(&tele) && !Role::Twin.may_send(&status));
    assert!(Role::Daemon.may_send(&status) && !Role::Daemon.may_send(&sim));
}

#[test]
fn trace_kinds_match_the_interpreter() {
    use neuron_interp::TraceKind as K;
    let pairs = [
        (K::Output, TraceKind::Out),
        (K::Signal, TraceKind::Sig),
        (K::Fault, TraceKind::Fault),
        (K::Reset, TraceKind::Reset),
        (K::Event, TraceKind::Event),
        (K::State, TraceKind::State),
        (K::Log, TraceKind::Log),
        (K::Motion, TraceKind::Move),
    ];
    for (a, b) in pairs {
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(json, format!("\"{}\"", a.tag()));
    }
}

proptest! {
    #[test]
    fn telemetry_round_trips(t in any::<i64>(), n in any::<i64>(), f in -1.0e9f64..1.0e9, b in any::<bool>(), s in "[a-z]{0,8}", pick in 0u8..4, q in 0u8..3) {
        let value = match pick {
            0 => Value::Bool(b),
            1 => Value::Number(n.into()),
            2 => Value::Number(serde_json::Number::from_f64(f).unwrap()),
            _ => Value::Text(s),
        };
        let quality = [Quality::Good, Quality::Stale, Quality::Bad][q as usize];
        let m = Message::new(Body::Telemetry {
            cell: "c".into(),
            samples: vec![Sample { id: "x".into(), t_ns: t, value, quality }],
        });
        let back = parse(&m.to_json()).unwrap();
        prop_assert_eq!(m, back);
    }
}
