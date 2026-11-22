//! Task D4.1: zones, mutexes, robot targets in zones, `NE0805`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{
    arbitrate, build, lock_signal, partition, zone_plans, zone_sources, CellDiag, EntryKind,
    Library, ZonePlan,
};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn text(path: &str, text: &str) -> SourceFile {
    SourceFile {
        path: path.into(),
        text: text.into(),
    }
}

fn x02() -> Vec<SourceFile> {
    vec![text(
        "x02_robot_zone.neuron",
        &std::fs::read_to_string(root().join("examples/corpus/cell/x02_robot_zone.neuron"))
            .unwrap(),
    )]
}

type Planned = (Vec<ZonePlan>, Vec<CellDiag>);

fn plans(sources: &[SourceFile]) -> Planned {
    let a = analyze(sources.to_vec(), None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    zone_plans(&nir, &plan, &lib)
}

const ROBOT_MODULE: &str = "module rm for robot
frame table = frame(origin: xyz(300, 0, 0) mm, rot: rpy(0, 0, 0) deg)
point far = table.xyz(900, 0, 10) mm
point inside = table.xyz(-100, 40, 10) mm
sequence go():
  move linear to far speed 50 mm/s
  move linear to inside speed 50 mm/s
";

const CELL_HEAD: &str = "cell C
  device arm: generic_scara @ ip(192.168.0.20)
  device plc: generic_s7_1200 @ ip(192.168.0.10)
  zone press: box(x: 100..220 mm, y: -40..40 mm, z: 0..120 mm)
  place rm on arm
";

fn robot_cell(mutex: &str) -> Vec<SourceFile> {
    vec![
        text("c.neuron", &format!("{CELL_HEAD}{mutex}")),
        text("rm.neuron", ROBOT_MODULE),
    ]
}

#[test]
fn x02_plans_one_lock_owned_by_the_plc() {
    let (z, d) = plans(&x02());
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(z.len(), 1);
    let z = &z[0];
    assert_eq!(z.zone, "press");
    assert_eq!(z.owner, "plc");
    assert_eq!(z.devices(), vec!["arm", "plc"]);
    assert_eq!(z.shape.x, [100.0, 220.0]);
    assert_eq!(z.shape.y, [-40.0, 40.0]);
    let arm = z.locks.iter().find(|l| l.party == "arm").unwrap();
    assert_eq!(arm.medium.as_deref(), Some("modbus_tcp"));
    assert!(arm.one_way_ns > 0);
    // The output of the PLC is an entry; no module is placed, so it is noted.
    assert!(z
        .entries
        .iter()
        .any(|e| matches!(&e.entry, EntryKind::Output { name } if name == "press_cylinder")));
    assert!(z.notes.iter().any(|n| n.contains("press_cylinder")));
}

#[test]
fn the_lock_modules_are_generated_for_both_parties() {
    let a = analyze(x02(), None, &SemaOptions::default());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    let (z, _) = zone_plans(&nir, &plan, &lib);
    let src = zone_sources(&nir, &plan, &lib, &z);
    let owner = &src["plc"][0].text;
    assert!(
        owner.starts_with("module lock_presscell_press_owner for plc"),
        "{owner}"
    );
    assert!(owner.contains(
        "z_press_grant_arm = z_press_req_arm and (z_press_grant_arm or not z_press_grant_plc)"
    ));
    assert!(owner.contains(
        "z_press_grant_plc = z_press_req_plc and (z_press_grant_plc or not z_press_grant_arm)"
    ));
    let arm = &src["arm"][0].text;
    assert!(arm.contains("var z_press_req_arm: bool = false"), "{arm}");
    assert!(arm.contains("var z_press_grant_arm: bool = false"));
}

#[test]
fn the_cell_builds_with_its_lock_modules() {
    let sources = x02();
    let a = analyze(sources.clone(), None, &SemaOptions::default());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    let b = build(&plan, &nir, &sources, &lib).unwrap();
    assert_eq!(b.zones.len(), 1);
    for dev in ["arm", "plc"] {
        let files = &b.device(dev).unwrap().files;
        assert!(
            files.contains_key("zones.json"),
            "{dev}: {:?}",
            files.keys()
        );
        assert!(
            files
                .keys()
                .any(|k| k.starts_with("links/lock_presscell_press")),
            "{dev}: {:?}",
            files.keys()
        );
    }
    let all: String = b.zones[0].notes.join("\n");
    assert!(!all.contains("is not part of its"), "{all}");
    // Deterministic.
    let b2 = build(&plan, &nir, &sources, &lib).unwrap();
    assert_eq!(b, b2);
}

#[test]
fn a_motion_into_the_zone_without_a_lock_is_ne0805() {
    let (z, d) = plans(&robot_cell(""));
    assert!(z.is_empty());
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, "NE0805");
    assert!(d[0].message.contains("zone `press`"), "{}", d[0].message);
    assert_eq!(d[0].file, "rm.neuron");
}

#[test]
fn a_robot_that_is_not_a_party_is_ne0805_even_with_another_mutex() {
    // The mutex is between the plc only; the arm still enters.
    let (_, d) = plans(&robot_cell("  mutex press between plc\n"));
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, "NE0805");
}

#[test]
fn a_party_motion_is_listed_as_an_entry_and_the_far_target_is_not() {
    let (z, d) = plans(&robot_cell("  mutex press between arm, plc\n"));
    assert!(d.is_empty(), "{d:?}");
    let motions: Vec<_> = z[0]
        .entries
        .iter()
        .filter_map(|e| match &e.entry {
            EntryKind::Motion { target_mm, .. } => Some((e.device.as_str(), *target_mm)),
            EntryKind::Output { .. } => None,
        })
        .collect();
    assert_eq!(motions, vec![("arm", [200.0, 40.0, 10.0])]);
    assert!(z[0].entries.iter().all(|e| e.wrapped));
}

#[test]
fn a_missing_party_output_is_ne0805_when_a_module_is_placed() {
    let mut s = robot_cell("  mutex press between arm, plc.nothing\n  place pm on plc\n");
    s.push(text(
        "pm.neuron",
        "module pm for plc\nscan 10 ms\noutputs:\n  press_cylinder: bool\nsafety:\n  safe_state:\n    press_cylinder.write(false)\n",
    ));
    let (_, d) = plans(&s);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, "NE0805");
    assert!(
        d[0].message.contains("no output `nothing`"),
        "{}",
        d[0].message
    );
    // The real output passes.
    let mut ok = robot_cell("  mutex press between arm, plc.press_cylinder\n  place pm on plc\n");
    ok.push(text(
        "pm.neuron",
        "module pm for plc\nscan 10 ms\noutputs:\n  press_cylinder: bool\nsafety:\n  safe_state:\n    press_cylinder.write(false)\n",
    ));
    assert!(plans(&ok).1.is_empty());
}

#[test]
fn a_party_on_an_fpga_cannot_take_a_lock() {
    let s = vec![text(
        "c.neuron",
        "cell C\n  device plc: generic_s7_1200 @ ip(192.168.0.10)\n  device fpga: generic_artix7_board @ pin(A1)\n  zone z: box(x: 0..10 mm, y: 0..10 mm, z: 0..10 mm)\n  mutex z between plc, fpga\n",
    )];
    let a = analyze(s.clone(), None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    let (_, d) = zone_plans(&nir, &plan, &lib);
    assert!(
        d.iter()
            .any(|x| x.code == "NE0805" && x.message.contains("fpga")),
        "{d:?}"
    );
}

#[test]
fn the_owner_is_a_host_when_a_host_is_a_party() {
    let s = vec![text(
        "c.neuron",
        "cell C\n  device plc: generic_s7_1200 @ ip(192.168.0.10)\n  device pi: raspberry_pi_5 @ ip(192.168.0.30)\n  zone z: box(x: 0..10 mm, y: 0..10 mm, z: 0..10 mm)\n  mutex z between plc, pi\n  link plc.a -> pi.b via modbus_tcp every 10 ms within 100 ms\n",
    )];
    let (z, d) = plans(&s);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(z[0].owner, "pi");
}

#[test]
fn signal_names_are_stable() {
    assert_eq!(
        lock_signal("Press Zone", "arm-1", false),
        "z_press_zone_grant_arm_1"
    );
}

// ---- the arbiter: exclusion over random interleavings -----------------------------

proptest! {
    /// LOCK-1: whatever the requests do, at most one grant is up after every scan; a
    /// grant is never up without its request; the lowest-numbered request wins a tie.
    #[test]
    fn lock_exclusion(steps in proptest::collection::vec(proptest::collection::vec(any::<bool>(), 4), 1..60)) {
        let mut grants = vec![false; 4];
        for reqs in steps {
            let prev = grants.clone();
            grants = arbitrate(&reqs, &grants);
            prop_assert!(grants.iter().filter(|g| **g).count() <= 1, "{grants:?}");
            for i in 0..4 {
                prop_assert!(!grants[i] || reqs[i]);
                // A held grant with its request up is kept.
                if prev[i] && reqs[i] {
                    prop_assert!(grants[i]);
                }
            }
            if !prev.iter().any(|g| *g) {
                if let Some(first) = reqs.iter().position(|r| *r) {
                    prop_assert!(grants[first]);
                }
            }
        }
    }
}
