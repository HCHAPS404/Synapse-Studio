//! Task D4.3: cell `verify:` over the composed machines, the mutex proof, shortest
//! counterexamples as plain data.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{
    partition, prove_zone, prove_zone_with, verify_cell, zone_plans, CellVerdict, Handshake,
    Library, LockLink, Party, ZonePlan,
};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

const TICK: u64 = 10_000_000;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn text(path: &str, text: &str) -> SourceFile {
    SourceFile {
        path: path.into(),
        text: text.into(),
    }
}

fn x02_zone() -> ZonePlan {
    let src = text(
        "x02.neuron",
        &std::fs::read_to_string(root().join("examples/corpus/cell/x02_robot_zone.neuron"))
            .unwrap(),
    );
    let a = analyze(vec![src], None, &SemaOptions::default());
    let nir = a.nir.unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    zone_plans(&nir, &plan, &lib).0.remove(0)
}

fn with_delay(mut z: ZonePlan, ns: u64) -> ZonePlan {
    for l in &mut z.locks {
        if l.medium.is_some() {
            l.one_way_ns = ns;
        }
    }
    z
}

fn verdict(sources: Vec<SourceFile>) -> CellVerdict {
    let a = analyze(sources.clone(), None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    let (zones, _) = zone_plans(&nir, &plan, &lib);
    verify_cell(&sources, &nir, &plan, &lib, &zones)
}

#[test]
fn the_mutex_of_x02_is_proved_with_its_real_lock_delay() {
    let z = x02_zone();
    let arm = z.locks.iter().find(|l| l.party == "arm").unwrap();
    assert!(arm.one_way_ns > 0);
    let p = prove_zone(&z, TICK);
    assert!(p.holds(), "{p:?}");
    assert_eq!(p.proved.len(), 1);
    assert_eq!(p.proved[0].property, "never P0.Inside and P1.Inside");
    assert!(p.proved[0].states > 10);
}

#[test]
fn the_mutex_holds_for_a_long_delay_and_for_three_parties() {
    let z = with_delay(x02_zone(), 4 * TICK);
    let p = prove_zone(&z, TICK);
    assert!(p.holds(), "{p:?}");
    let mut three = with_delay(x02_zone(), TICK);
    three.parties.push(Party {
        device: "third".into(),
        output: None,
    });
    three.locks.push(LockLink {
        party: "third".into(),
        medium: Some("modbus_tcp".into()),
        one_way_ns: TICK,
    });
    let p = prove_zone(&three, TICK);
    assert!(p.holds(), "{p:?}");
    assert_eq!(p.proved.len(), 3);
}

#[test]
fn the_two_phase_handshake_breaks_with_a_delay_and_the_trace_is_plain_data() {
    let z = with_delay(x02_zone(), 30_000_000);
    let bad = prove_zone_with(&z, TICK, Handshake::TwoPhase);
    assert_eq!(bad.violations.len(), 1, "{bad:?}");
    let c = &bad.violations[0];
    assert_eq!(c.property, "never P0.Inside and P1.Inside");
    let last = c.trace.last().unwrap();
    assert!(last.states.contains(&"P0.Inside".to_string()), "{last:?}");
    assert!(last.states.contains(&"P1.Inside".to_string()), "{last:?}");
    // Ticks are consecutive from 0 and every step names every machine and input.
    for (i, s) in c.trace.iter().enumerate() {
        assert_eq!(s.tick, i as u64);
        assert_eq!(s.time_ns, i as u64 * TICK);
        assert_eq!(s.states.len(), 3);
        assert!(s.inputs.contains_key("want_0"));
    }
    // It serializes, so the replay of D6 can read it.
    let json = serde_json::to_string(c).unwrap();
    assert!(json.contains("\"trace\""));
    // The four-phase protocol holds at the same delay.
    assert!(prove_zone(&z, TICK).holds());
    // Without delay even the two-phase handshake is fine.
    assert!(prove_zone_with(&with_delay(x02_zone(), 0), TICK, Handshake::TwoPhase).holds());
}

const CELL: &str = "cell V
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
";

fn cell_with(props: &str) -> Vec<SourceFile> {
    vec![text("v.neuron", &format!("{CELL}  verify:\n{props}"))]
}

#[test]
fn a_cell_property_holds_over_two_sequences_joined_by_a_link() {
    let v = verdict(cell_with("    never B.Picking and A.Idle\n"));
    assert!(v.violations.is_empty(), "{:?} {:?}", v.violations, v.notes);
    assert_eq!(v.proved.len(), 1, "{:?}", v.notes);
    assert_eq!(v.channels.len(), 1);
    assert_eq!(v.channels[0].delay_ns, 30_000_000);
}

#[test]
fn a_violated_cell_property_gives_the_shortest_counterexample() {
    let v = verdict(cell_with("    never B.Picking\n"));
    assert_eq!(v.violations.len(), 1, "{:?}", v.notes);
    let c = &v.violations[0];
    // part_ready rises at tick 0 and arrives at once; both sequences leave Idle at tick 1.
    assert_eq!(c.trace.len(), 2, "{c:?}");
    assert_eq!(c.trace[0].inputs.get("plc.part_ready"), Some(&true));
    assert!(c.trace[1].states.contains(&"B.Picking".to_string()));
    assert!(v.diags.iter().any(|d| d.code == "NE0901"));
}

#[test]
fn the_delay_of_the_link_is_visible_to_the_properties() {
    // Picking starts at most 3 ticks after Work; with a 30 ms budget `Picking` can come
    // up to 3 ticks after the plc left Idle, but never before.
    let v = verdict(cell_with(
        "    eventually_within 20 ms after A.Work: B.Picking\n",
    ));
    assert_eq!(v.violations.len(), 1, "{:?}", v.notes);
    let ok = verdict(cell_with(
        "    eventually_within 100 ms after A.Work: B.Picking\n",
    ));
    assert!(ok.violations.is_empty(), "{:?}", ok.notes);
}

#[test]
fn the_cell_with_a_mutex_reports_its_proof() {
    let src =
        std::fs::read_to_string(root().join("examples/corpus/cell/x02_robot_zone.neuron")).unwrap();
    let v = verdict(vec![text("x02.neuron", &src)]);
    assert_eq!(v.zones.len(), 1);
    assert!(v.zones[0].holds());
    assert!(v.diags.is_empty(), "{:?}", v.diags);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(7))]
    /// MUTEX-1: for every delay of the lock signals up to six ticks the four-phase
    /// lock never has two parties inside the zone; and a counterexample of the
    /// two-phase variant starts at tick 0 and ends with both parties inside.
    #[test]
    fn the_four_phase_lock_holds_for_every_delay(ticks in 0u64..7) {
        let z = with_delay(x02_zone(), ticks * TICK);
        prop_assert!(prove_zone(&z, TICK).holds());
        let two = prove_zone_with(&z, TICK, Handshake::TwoPhase);
        for c in &two.violations {
            prop_assert_eq!(c.trace[0].tick, 0);
            let last = c.trace.last().unwrap();
            prop_assert!(last.states.contains(&"P0.Inside".to_string()));
            prop_assert!(last.states.contains(&"P1.Inside".to_string()));
        }
        // Without delay the extra phase is not needed.
        if ticks == 0 {
            prop_assert!(two.violations.is_empty());
        }
    }
}
