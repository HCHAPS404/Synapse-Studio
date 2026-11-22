//! Task D2.2: every device of a cell builds through its own backend.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{build, partition, CellBuild, CellDiag, Library};
use neuron_sema::{analyze, SemaOptions, SourceFile};
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

fn run(sources: Vec<SourceFile>) -> Result<CellBuild, Vec<CellDiag>> {
    let a = analyze(sources.clone(), None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    build(&plan, &nir, &sources, &lib)
}

/// One device of every class, each running a corpus program that its backend translates.
fn lab() -> Vec<SourceFile> {
    vec![
        file("examples/corpus/plc/p01_start_stop.neuron"),
        file("examples/corpus/mcu/m01_blink.neuron"),
        file("examples/corpus/robot/r01_home_and_wave.neuron"),
        file("examples/corpus/hdl/d01_counter.neuron"),
        file("examples/corpus/host/h01_gateway_counter.neuron"),
        SourceFile {
            path: "lab.neuron".into(),
            text: "cell Lab\n  device plc: generic_s7_1200 @ ip(192.168.0.10)\n  device board: arduino_uno_r3 @ usb\n  device arm: generic_6axis @ ip(192.168.0.20)\n  device fpga: generic_artix7_board @ usb\n  device pi: raspberry_pi_5 @ ip(192.168.0.30)\n  place p01_start_stop on plc\n  place m01_blink on board\n  place r01_home_and_wave on arm\n  place d01_counter on fpga\n  place h01_gateway_counter on pi\n".into(),
        },
    ]
}

#[test]
fn every_device_class_builds_with_its_backend() {
    let b = run(lab()).unwrap_or_else(|e| {
        panic!(
            "{}",
            e.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    assert_eq!(b.devices.len(), 5);
    let has = |dev: &str, suffix: &str| {
        b.device(dev)
            .unwrap()
            .files
            .keys()
            .any(|k| k.ends_with(suffix))
    };
    assert!(
        has("plc", ".scl"),
        "S7-class PLC gets SCL: {:?}",
        b.device("plc").unwrap().files.keys()
    );
    assert!(has("board", ".c"), "controller gets C");
    assert!(has("arm", ".prg"), "robot gets a SPEL+-style program");
    assert!(
        has("fpga", "top.v") && has("fpga", "top.xdc"),
        "FPGA gets Verilog and XDC"
    );
    assert!(has("pi", ".py"), "host gets Python");
    // Translations carry their fidelity report.
    for dev in ["plc", "arm", "pi"] {
        assert!(has(dev, "fidelity.json"), "{dev}");
    }
}

#[test]
fn the_build_is_deterministic() {
    assert_eq!(run(lab()).unwrap(), run(lab()).unwrap());
}

#[test]
fn a_device_with_nothing_placed_builds_to_no_files() {
    let b = run(vec![SourceFile {
        path: "c.neuron".into(),
        text: "cell C\n  device plc: generic_s7_1200 @ usb\n".into(),
    }])
    .unwrap();
    assert!(b.device("plc").unwrap().files.is_empty());
}

#[test]
fn backend_limits_in_the_bottling_cell_are_reported_per_device() {
    // The bottling example goes beyond today's backends: `thermostat` uses function
    // blocks (`pid`, `lowpass`), which the C backend does not support yet, and `pick` is
    // a six-axis program that the cell places, automatically, on a SCARA arm.
    let dir = root().join("examples");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    files.sort();
    let sources = files
        .iter()
        .map(|f| {
            file(&format!(
                "examples/{}",
                f.file_name().unwrap().to_string_lossy()
            ))
        })
        .collect();
    let e = run(sources).unwrap_err();
    assert!(
        e.iter().all(|d| ["NE0610", "NE0804"].contains(&d.code)),
        "{e:?}"
    );
    assert!(e.iter().any(|d| d.message.starts_with("device `ctl`")));
    assert!(e.iter().any(|d| d.message.starts_with("device `arm`")));
    assert!(!e
        .iter()
        .any(|d| d.message.starts_with("device `plc`") || d.message.starts_with("device `fpga`")));
}
