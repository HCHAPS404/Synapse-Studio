//! Task D3.1: link endpoints, register maps and the plan of every link.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{
    build, link_plans, link_table, partition, register_csv, register_map, CellBuild, Crossing,
    Library, LinkPlan, Location, Pattern, REGISTER_BASE,
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

fn plans_of(sources: &[SourceFile], lib: &Library) -> Vec<LinkPlan> {
    let a = analyze(sources.to_vec(), None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let plan = partition(&nir, lib).unwrap().remove(0);
    link_plans(&nir, &plan, lib).unwrap()
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

/// The text of the generated programs of a device (not the plan files).
fn program(b: &CellBuild, device: &str) -> String {
    b.device(device)
        .unwrap()
        .files
        .iter()
        .filter(|(k, _)| {
            !k.starts_with("links/")
                && !matches!(
                    k.as_str(),
                    "links.json" | "register_map.csv" | "fidelity.json"
                )
                && !k.ends_with(".map.json")
        })
        .map(|(_, v)| v.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_endpoints_of_every_corpus_link_are_in_both_programs_and_in_the_link_table() {
    for name in ["x01_two_devices", "x02_robot_zone", "x04_event_from_fpga"] {
        let b = built(vec![file(&format!("examples/corpus/cell/{name}.neuron"))]);
        assert_eq!(b.links.len(), 1, "{name}");
        let l = &b.links[0];
        for ep in [&l.tx, &l.rx] {
            let prog = program(&b, &ep.device);
            assert!(
                prog.contains("l0_value"),
                "{name}: `l0_value` missing in {}",
                ep.device
            );
            let dev = b.device(&ep.device).unwrap();
            assert!(
                dev.files
                    .contains_key(&format!("links/{}.neuron", ep.module)),
                "{name}: {:?}",
                dev.files.keys()
            );
            assert!(dev.files["links.json"].contains(&ep.module), "{name}");
            assert!(
                l.notes.iter().all(|n| !n.contains("is not part of")),
                "{name}: {:?}",
                l.notes
            );
        }
    }
}

#[test]
fn crossings_follow_the_periods_and_the_receiver() {
    let lib = Library::builtin().unwrap();
    let corpus = |n: &str| plans_of(&[file(&format!("examples/corpus/cell/{n}.neuron"))], &lib);
    // A 100 us controller feeding a 1 ms PLC: Latch+ACK.
    let x01 = &corpus("x01_two_devices")[0];
    assert_eq!(x01.crossing, Crossing::LatchAck);
    assert_eq!(
        (x01.sender_period_ns, x01.receiver_period_ns),
        (100_000, 1_000_000)
    );
    // A robot receives an action.
    let x02 = &corpus("x02_robot_zone")[0];
    assert_eq!(x02.crossing, Crossing::Action);
    assert_eq!(x02.action_timeout_ns, Some(1_000_000_000));
    // An FPGA pulse into a PLC input: Latch+ACK on a wire, with an ack wire.
    let x04 = &corpus("x04_event_from_fpga")[0];
    assert_eq!(x04.crossing, Crossing::LatchAck);
    assert_eq!(
        x04.tx.location,
        Location::Pin {
            name: "l0_out".into(),
            ack: Some("l0_ack".into())
        }
    );
    // Equal periods are direct; a slower sender gives a stale flag.
    let two = |a: &str, b: &str| {
        plans_of(
            &[text(
                "c.neuron",
                &format!(
                    "cell C\n  device a: {a} @ usb\n  device b: {b} @ usb\n  link a.x -> b.y via zero_hop\n"
                ),
            )],
            &lib,
        )
        .remove(0)
    };
    assert_eq!(
        two("nucleo_f446re", "nucleo_f446re").crossing,
        Crossing::Direct
    );
    assert_eq!(
        two("generic_s7_1200", "nucleo_f446re").crossing,
        Crossing::SampleStale
    );
}

fn lab_links() -> Vec<SourceFile> {
    vec![
        file("examples/corpus/plc/p01_start_stop.neuron"),
        file("examples/corpus/mcu/m01_blink.neuron"),
        file("examples/corpus/robot/r01_home_and_wave.neuron"),
        file("examples/corpus/hdl/d01_counter.neuron"),
        file("examples/corpus/host/h01_gateway_counter.neuron"),
        text(
            "lab.neuron",
            "cell Lab\n  device plc: generic_s7_1200 @ ip(192.168.0.10)\n  device board: arduino_uno_r3 @ usb\n  device arm: generic_6axis @ ip(192.168.0.20)\n  device fpga: generic_artix7_board @ usb\n  device pi: raspberry_pi_5 @ ip(192.168.0.30)\n  place p01_start_stop on plc\n  place m01_blink on board\n  place r01_home_and_wave on arm\n  place d01_counter on fpga\n  place h01_gateway_counter on pi\n  link plc.motor -> pi.messages via s7\n  link pi.messages -> plc.start via modbus_tcp\n  link plc.pick_cmd -> arm.start via modbus_tcp every 10 ms\n  link board.led -> fpga.pulse via zero_hop(gpio)\n",
        ),
    ]
}

#[test]
fn value_types_come_from_the_placed_modules() {
    let lib = Library::builtin().unwrap();
    let p = plans_of(&lab_links(), &lib);
    assert_eq!(p.len(), 4);
    // plc.motor is `bool` in p01; the link to pi.messages carries it.
    assert_eq!((p[0].value_type.as_str(), p[0].value_bytes), ("bool", 1));
    assert!(
        p[0].type_source.contains("p01_start_stop"),
        "{}",
        p[0].type_source
    );
    // pi.messages is `u32` in h01: two registers.
    assert_eq!((p[1].value_type.as_str(), p[1].value_bytes), ("u32", 4));
    // The robot link has no declared signal: the default.
    assert!(p[2].type_source.starts_with("default"));
    assert!(p[2].notes.iter().any(|n| n.contains("default")));
    // A 1 s host program feeds a 10 ms PLC: sample and stale flag.
    assert_eq!(p[1].crossing, Crossing::SampleStale);
    // The 10 ms PLC feeds the 1 s host: Latch+ACK.
    assert_eq!(p[0].crossing, Crossing::LatchAck);
}

#[test]
fn the_receivers_real_period_goes_into_the_link_budget() {
    let lib = Library::builtin().unwrap();
    let sources = lab_links();
    let a = analyze(sources, None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    let t = link_table(&nir, &plan, &lib).unwrap();
    // `plc.motor -> pi.messages`: the host program runs every second.
    assert_eq!(t[0].latency.receiver_phase_ns, 1_000_000_000);
    // `pi.messages -> plc.start`: p01 scans every 10 ms.
    assert_eq!(t[1].latency.receiver_phase_ns, 10_000_000);
}

#[test]
fn registers_are_allocated_per_receiver_from_the_base() {
    let lib = Library::builtin().unwrap();
    let p = plans_of(&lab_links(), &lib);
    let owner = |l: &LinkPlan| match &l.tx.location {
        Location::Register { owner, .. } => owner.clone(),
        other => panic!("{other:?}"),
    };
    // Link 0 (Latch+ACK, bool) is the first block on `pi`: value, seq, ack.
    let Location::Register {
        value, seq, ack, ..
    } = &p[0].tx.location
    else {
        panic!()
    };
    assert_eq!(
        (owner(&p[0]).as_str(), *value, *seq, *ack),
        (
            "pi",
            REGISTER_BASE,
            Some(REGISTER_BASE + 1),
            Some(REGISTER_BASE + 2)
        )
    );
    // Link 1 (sample, u32) is the first block on `plc`: two registers, no handshake.
    let Location::Register {
        value,
        registers,
        seq,
        ..
    } = &p[1].tx.location
    else {
        panic!()
    };
    assert_eq!(
        (owner(&p[1]).as_str(), *value, *registers, *seq),
        ("plc", REGISTER_BASE, 2, None)
    );
    // Link 2 is an action on `arm`: value, seq, busy, done.
    let Location::Register {
        value,
        seq,
        busy,
        done,
        ..
    } = &p[2].tx.location
    else {
        panic!()
    };
    assert_eq!(
        (*value, *seq, *busy, *done),
        (
            REGISTER_BASE,
            Some(REGISTER_BASE + 1),
            Some(REGISTER_BASE + 2),
            Some(REGISTER_BASE + 3)
        )
    );
    // The wire has no registers.
    assert!(matches!(p[3].tx.location, Location::Pin { .. }));
    let rows = register_map(&p);
    assert_eq!(rows.iter().filter(|r| r.owner == "plc").count(), 1);
    let csv = register_csv(&p, "pi").unwrap();
    assert!(csv.starts_with("owner,link,field,address,registers,type,writer\n"));
    assert!(csv.contains("pi,0,ack,102,1,u16,pi"), "{csv}");
    assert!(register_csv(&p, "fpga").is_none());
}

#[test]
fn a_cell_with_a_link_between_every_class_builds_and_is_deterministic() {
    let a = built(lab_links());
    // Every backend of the five classes took its endpoint modules: no fidelity note.
    assert!(a
        .links
        .iter()
        .all(|l| l.notes.iter().all(|n| !n.contains("is not part of"))));
    assert_eq!(a, built(lab_links()));
    for dev in ["plc", "arm", "pi", "board", "fpga"] {
        assert!(
            a.device(dev).unwrap().files.contains_key("links.json"),
            "{dev}"
        );
    }
    // Endpoint names reach the generated programs, or the plan says why not.
    for l in &a.links {
        for ep in [&l.tx, &l.rx] {
            let prog = program(&a, &ep.device);
            let missing = !prog.contains(&format!("l{}_value", l.index));
            let noted = l.notes.iter().any(|n| n.contains("is not part of"));
            assert!(!missing || noted, "link {} on {}", l.index, ep.device);
        }
    }
    assert!(a
        .device("plc")
        .unwrap()
        .files
        .contains_key("register_map.csv"));
    assert!(!a
        .device("fpga")
        .unwrap()
        .files
        .contains_key("register_map.csv"));
}

#[test]
fn frame_and_pubsub_media_get_wire_tags_and_topics() {
    let mut lib = Library::builtin().unwrap();
    for (name, family) in [("uart", "frame"), ("mqtt", "pubsub")] {
        lib.add_text(
            &format!("media/{name}.toml"),
            &format!(
                "[medium]\nname = \"{name}\"\nfamily = \"{family}\"\nbitrate = \"115200 Bd\"\nframe_overhead_bytes = 14\ntransport_worst = \"1 ms\"\nspeaks = [\"host\"]\ntrust = \"generic\"\nnote = \"test medium\"\n"
            ),
        )
        .unwrap();
    }
    lib.add_text(
        "hosts/wire_host.toml",
        "[device]\nname = \"wire_host\"\nkind = \"host\"\ndialects = [\"host\", \"core\"]\nbackend = \"python\"\ntrust = \"generic\"\nnote = \"test\"\n[timing]\nscan_min = \"10 ms\"\nuart = { transport = \"1 ms\", jitter = \"1 ms\" }\nmqtt = { transport = \"1 ms\", jitter = \"1 ms\" }\n[safety]\nsafe_state_time = \"10 ms\"\n",
    )
    .unwrap();
    lib.check().unwrap();
    let p = plans_of(
        &[text(
            "w.neuron",
            "cell WireCell\n  device a: wire_host @ usb\n  device b: wire_host @ usb\n  link a.x -> b.y via uart every 10 ms\n  link a.y -> b.z via mqtt\n  link b.q -> a.r via uart\n",
        )],
        &lib,
    );
    assert_eq!(p[0].pattern, Pattern::Frame);
    assert_eq!(p[0].tx.location, Location::WireTag { id: 1 });
    assert_eq!(p[0].rx.location, p[0].tx.location);
    assert_eq!(p[1].pattern, Pattern::Pubsub);
    assert_eq!(
        p[1].tx.location,
        Location::Topic {
            name: "neuron/wirecell/l1".into()
        }
    );
    assert_eq!(p[2].tx.location, Location::WireTag { id: 3 });
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    /// The register map never overlaps, starts at the base on every device, and does
    /// not depend on how many times it is planned.
    #[test]
    fn register_blocks_never_overlap(
        links in proptest::collection::vec((any::<bool>(), 0usize..3), 1..8)
    ) {
        let lib = Library::builtin().unwrap();
        let mut src = String::from(
            "cell C\n  device a: nucleo_f446re @ usb\n  device b: generic_s7_1200 @ usb\n  device c: raspberry_pi_5 @ usb\n",
        );
        for (i, (forward, kind)) in links.iter().enumerate() {
            let (f, t) = match (forward, kind) {
                (true, 0) => ("a", "b"),
                (false, 0) => ("b", "a"),
                (true, 1) => ("b", "c"),
                (false, 1) => ("c", "b"),
                (true, _) => ("a", "c"),
                (false, _) => ("c", "a"),
            };
            let medium = if f == "a" && t == "c" || f == "c" && t == "a" { "modbus_tcp" } else if *kind == 1 { "s7" } else { "modbus_tcp" };
            src.push_str(&format!("  link {f}.s{i} -> {t}.t{i} via {medium}\n"));
        }
        let sources = [text("c.neuron", &src)];
        let p = plans_of(&sources, &lib);
        prop_assert_eq!(&p, &plans_of(&sources, &lib));
        let rows = register_map(&p);
        for owner in ["a", "b", "c"] {
            let mine: Vec<_> = rows.iter().filter(|r| r.owner == owner).collect();
            if let Some(first) = mine.first() {
                prop_assert_eq!(first.address, REGISTER_BASE);
            }
            for w in mine.windows(2) {
                prop_assert!(w[0].address + w[0].registers <= w[1].address, "{:?} {:?}", w[0], w[1]);
            }
        }
    }
}
