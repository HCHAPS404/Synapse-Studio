//! Task D2.1: partitioner, `NE0704`, `NE0705` and `CellPlan`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{partition, How, Kind, Library};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_dir(dir: &Path) -> Vec<SourceFile> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| SourceFile {
            path: f.file_name().unwrap().to_string_lossy().into_owned(),
            text: std::fs::read_to_string(f).unwrap(),
        })
        .collect()
}

fn nir(sources: Vec<SourceFile>) -> neuron_nir::Nir {
    let a = analyze(sources, None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    a.nir.clone().unwrap()
}

fn src(path: &str, text: &str) -> SourceFile {
    SourceFile {
        path: path.into(),
        text: text.into(),
    }
}

fn example(name: &str) -> SourceFile {
    src(
        name,
        &std::fs::read_to_string(root().join("examples").join(name)).unwrap(),
    )
}

fn modules(plan: &neuron_cell::CellPlan, dev: &str) -> Vec<(String, How)> {
    plan.device(dev)
        .unwrap()
        .modules
        .iter()
        .map(|p| (p.module.clone(), p.how))
        .collect()
}

#[test]
fn the_bottling_cell_partitions_into_one_program_per_device() {
    let lib = Library::builtin().unwrap();
    let plans = partition(&nir(read_dir(&root().join("examples"))), &lib).unwrap();
    assert_eq!(plans.len(), 1);
    let p = &plans[0];
    assert_eq!(p.cell, "BottlingLine");
    assert_eq!(p.devices.len(), 4);
    assert_eq!(
        modules(p, "ctl"),
        [("thermostat".to_string(), How::Explicit)]
    );
    assert_eq!(modules(p, "plc"), [("conveyor".to_string(), How::Explicit)]);
    assert_eq!(p.device("plc").unwrap().sequences, ["Cycle"]);
    // `pick` (robot) and `pwm8` (hdl) are not placed: they go to the only device that
    // runs their dialect. `blink` and `panel` are written for boards outside the cell.
    assert_eq!(modules(p, "arm"), [("pick".to_string(), How::Automatic)]);
    assert_eq!(modules(p, "fpga"), [("pwm8".to_string(), How::Automatic)]);
    for d in &p.devices {
        assert!(
            !d.modules.is_empty() || !d.sequences.is_empty(),
            "{} runs nothing",
            d.instance
        );
    }
    assert_eq!(p.device("fpga").unwrap().kind, Kind::Fpga);
}

#[test]
fn every_corpus_cell_gets_a_plan_per_device() {
    let lib = Library::builtin().unwrap();
    for f in read_dir(&root().join("examples/corpus/cell")) {
        let name = f.path.clone();
        let plans = partition(&nir(vec![f]), &lib).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(plans.len(), 1, "{name}");
        assert!(!plans[0].devices.is_empty(), "{name}");
    }
    let x03 = nir(vec![src(
        "x03.neuron",
        &std::fs::read_to_string(root().join("examples/corpus/cell/x03_cycle_sequence.neuron"))
            .unwrap(),
    )]);
    let plan = &partition(&x03, &lib).unwrap()[0];
    assert_eq!(plan.device("plc").unwrap().sequences, ["Cycle"]);
}

#[test]
fn a_module_on_a_device_that_cannot_run_it_is_ne0704() {
    let lib = Library::builtin().unwrap();
    let n = nir(vec![
        example("conveyor_plc.neuron"),
        src(
            "c.neuron",
            "cell C\n  device ctl: nucleo_f446re @ usb\n  place conveyor on ctl\n",
        ),
    ]);
    let e = partition(&n, &lib).unwrap_err();
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].code, "NE0704");
    assert!(e[0].help[0].contains("not `plc`"), "{:?}", e[0]);
    assert!(e[0].span.is_some());
}

#[test]
fn a_module_written_for_another_board_is_ne0704() {
    let mut lib = Library::builtin().unwrap();
    let uno = include_str!("../../../devices/controllers/nucleo_f446re.toml")
        .replace("name = \"nucleo_f446re\"", "name = \"demo_uno\"")
        .replace("target = \"nucleo_f446re\"", "target = \"arduino_uno_r3\"");
    lib.add_text("controllers/demo_uno.toml", &uno).unwrap();
    lib.check().unwrap();
    let n = nir(vec![
        example("thermostat.neuron"),
        src(
            "c.neuron",
            "cell C\n  device ctl: demo_uno @ usb\n  place thermostat on ctl\n",
        ),
    ]);
    let e = partition(&n, &lib).unwrap_err();
    assert_eq!(e[0].code, "NE0704");
    assert!(
        e[0].help[0].contains("written for board `nucleo_f446re`"),
        "{:?}",
        e[0]
    );
}

#[test]
fn unknown_devices_and_double_placement_are_reported() {
    let lib = Library::builtin().unwrap();
    let n = nir(vec![
        example("conveyor_plc.neuron"),
        src(
            "c.neuron",
            "cell C\n  device plc: generic_s7_1200 @ usb\n  place conveyor on plc\n  place conveyor on plc\n",
        ),
    ]);
    let e = partition(&n, &lib).unwrap_err();
    assert_eq!(e.len(), 1);
    assert!(e[0].message.contains("placed twice"));
}

#[test]
fn a_module_that_fits_no_device_is_ne0705_with_the_reasons() {
    let lib = Library::builtin().unwrap();
    let n = nir(vec![
        example("conveyor_plc.neuron"),
        src("c.neuron", "cell C\n  device arm: generic_scara @ usb\n"),
    ]);
    let e = partition(&n, &lib).unwrap_err();
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].code, "NE0705");
    assert!(e[0].message.contains("`conveyor`"));
    assert!(
        e[0].help
            .iter()
            .any(|h| h.contains("`arm` (generic_scara)")),
        "{:?}",
        e[0]
    );
}

#[test]
fn a_sequence_on_an_fpga_is_ne0704() {
    let lib = Library::builtin().unwrap();
    let n = nir(vec![src(
        "c.neuron",
        "cell C\n  device fpga: generic_artix7_board @ usb\n  sequence S on fpga:\n    initial step A:\n      until fpga.go\n",
    )]);
    let e = partition(&n, &lib).unwrap_err();
    assert_eq!(e[0].code, "NE0704");
    assert!(e[0].message.contains("sequence `S`"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]
    /// The plan does not depend on the order of the source files (determinism).
    #[test]
    fn the_plan_does_not_depend_on_file_order(seed in any::<u64>()) {
        let lib = Library::builtin().unwrap();
        let base = read_dir(&root().join("examples"));
        let want = partition(&nir(base.clone()), &lib).unwrap();
        let mut shuffled = base;
        let mut s = seed;
        for i in (1..shuffled.len()).rev() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            shuffled.swap(i, (s >> 33) as usize % (i + 1));
        }
        let got = partition(&nir(shuffled), &lib).unwrap();
        prop_assert_eq!(got, want);
    }
}
