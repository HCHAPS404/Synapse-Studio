//! Task D3.3: the cell watchdog in the receiver endpoints and the MOC-C2 bound.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{
    build, link_plans, partition, watchdog_plans, CellBuild, Library, WatchdogPlan,
    DEFAULT_WATCHDOG_NS, HEARTBEAT_NS, MISSED_HEARTBEATS,
};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn file(rel: &str) -> SourceFile {
    SourceFile {
        path: Path::new(rel)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        text: std::fs::read_to_string(root().join(rel)).unwrap(),
    }
}
fn text(path: &str, text: &str) -> SourceFile {
    SourceFile {
        path: path.into(),
        text: text.into(),
    }
}
fn built(sources: Vec<SourceFile>) -> CellBuild {
    let a = analyze(sources.clone(), None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    build(&plan, &nir, &sources, &lib).unwrap_or_else(|e| {
        panic!(
            "{}",
            e.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}
fn program(b: &CellBuild, device: &str) -> String {
    b.device(device)
        .unwrap()
        .files
        .iter()
        .filter(|(k, _)| !k.starts_with("links/") && k.as_str() != "links.json")
        .map(|(_, v)| v.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_default_watchdog_is_five_heartbeats() {
    assert_eq!(HEARTBEAT_NS, 10_000_000);
    assert_eq!(MISSED_HEARTBEATS, 5);
    assert_eq!(DEFAULT_WATCHDOG_NS, 50_000_000);
}

#[test]
fn every_receiver_of_the_corpus_has_a_watchdog_and_a_bound() {
    // (cell, receiver, manifest safe-state time in ns)
    let cases = [
        ("x01_two_devices", "plc", 10_000_000u64),
        ("x04_event_from_fpga", "plc", 10_000_000),
        ("x02_robot_zone", "arm", 50_000_000),
    ];
    for (name, rx, safe) in cases {
        let b = built(vec![file(&format!("examples/corpus/cell/{name}.neuron"))]);
        let l = &b.links[0];
        assert_eq!(l.rx.device, rx, "{name}");
        assert_eq!(l.watchdog.period_ns, 50_000_000, "{name}");
        assert_eq!(l.watchdog.safe_state_ns, safe, "{name}");
        assert_eq!(l.watchdog.loss_bound_ns, 50_000_000 + safe, "{name}");
        assert_eq!(l.watchdog.fault, "LinkLost");
        let w = b.watchdogs.iter().find(|w| w.device == rx).unwrap();
        assert_eq!(w.bound_ns, 50_000_000 + safe, "{name}");
        assert_eq!(w.links, vec![0], "{name}");
        // Only the receiver consumes a link.
        assert_eq!(b.watchdogs.len(), 1, "{name}");
        let json = &b.device(rx).unwrap().files["links.json"];
        assert!(json.contains("\"loss_bound_ns\""), "{name}: {json}");
        assert!(json.contains("\"watchdog\""), "{name}");
        if rx == "plc" {
            // The watchdog is in the program the backend produced.
            let p = program(&b, rx);
            assert!(p.contains("l0_alive"), "{name}: no `l0_alive` in {rx}");
            assert!(p.contains("l0_lost"), "{name}: no `l0_lost` in {rx}");
            assert!(
                p.contains("neuron_fault := TRUE"),
                "{name}: the `LinkLost` fault is not in the {rx} program:\n{p}"
            );
            assert!(
                l.notes.iter().all(|n| !n.contains("is not part of")),
                "{name}: {:?}",
                l.notes
            );
        }
    }
}

#[test]
fn a_robot_receiver_says_it_has_no_watchdog_logic() {
    let b = built(vec![file("examples/corpus/cell/x02_robot_zone.neuron")]);
    assert!(
        b.links[0]
            .notes
            .iter()
            .any(|n| n.contains("no watchdog logic")),
        "{:?}",
        b.links[0].notes
    );
}

#[test]
fn the_bottling_cell_lists_the_actuators_that_take_their_safe_value() {
    let mut sources: Vec<SourceFile> = std::fs::read_dir(root().join("examples"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .map(|p| SourceFile {
            path: p.file_name().unwrap().to_string_lossy().into_owned(),
            text: std::fs::read_to_string(&p).unwrap(),
        })
        .collect();
    sources.sort_by(|a, b| a.path.cmp(&b.path));
    // The whole bottling cell does not build yet (known gaps of the C and SPEL+
    // backends), so the plan is read without building.
    let a = analyze(sources, None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    let links = link_plans(&nir, &plan, &lib).unwrap();
    let dogs = watchdog_plans(&nir, &plan, &links);
    // fpga -> plc and plc -> arm: two consumers.
    let names: Vec<&str> = dogs.iter().map(|w| w.device.as_str()).collect();
    assert_eq!(names, ["arm", "plc"]);
    let plc = dogs.iter().find(|w| w.device == "plc").unwrap();
    assert_eq!(plc.bound_ns, 60_000_000);
    assert!(plc.actuators.contains(&"motor".to_string()), "{plc:?}");
    let arm = dogs.iter().find(|w| w.device == "arm").unwrap();
    assert_eq!(arm.bound_ns, 100_000_000);
}

#[test]
fn two_links_into_one_device_share_one_bound_and_both_build() {
    let cell = "cell Two\n  device ctl: nucleo_f446re @ usb\n  device plc: generic_s7_1200 @ ip(192.168.0.10)\n  link ctl.a -> plc.x via modbus_tcp every 20 ms within 50 ms\n  link ctl.b -> plc.y via modbus_tcp every 20 ms within 50 ms\n";
    let b = built(vec![text("two.neuron", cell)]);
    assert_eq!(b.watchdogs.len(), 1);
    assert_eq!(b.watchdogs[0].links, vec![0, 1]);
    for l in &b.links {
        assert!(
            l.notes.iter().all(|n| !n.contains("is not part of")),
            "{:?}",
            l.notes
        );
    }
    let p = program(&b, "plc");
    assert!(p.contains("l0_alive") && p.contains("l1_alive"));
}

#[test]
fn the_build_is_deterministic() {
    let src = || vec![file("examples/corpus/cell/x01_two_devices.neuron")];
    assert_eq!(built(src()), built(src()));
}

proptest! {
    /// MOC-C2 bound: watchdog plus safe-state time, for any safe-state time.
    #[test]
    fn the_bound_is_watchdog_plus_safe_state_time(safe in 0u64..=10_000_000_000) {
        let w = WatchdogPlan::new(safe);
        prop_assert_eq!(w.period_ns, DEFAULT_WATCHDOG_NS);
        prop_assert_eq!(w.loss_bound_ns, DEFAULT_WATCHDOG_NS + safe);
        prop_assert!(w.loss_bound_ns >= w.period_ns.max(w.safe_state_ns));
    }
}
