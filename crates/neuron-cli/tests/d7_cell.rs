//! Task D7: `neuron cell check|build|sim|dossier`, run as a user runs them.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn neuron(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_neuron"))
        .args(args)
        .current_dir(root())
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("neuron-d7-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Runs `neuron <args>` and requires success; returns the combined output.
fn ok(args: &[&str]) -> String {
    let o = neuron(args);
    let t = text(&o);
    assert!(o.status.success(), "neuron {args:?} failed:\n{t}");
    t
}

/// Runs `neuron <args>`, requires exit code `code`; returns the combined output.
fn fails(args: &[&str], code: i32) -> String {
    let o = neuron(args);
    let t = text(&o);
    assert_eq!(o.status.code(), Some(code), "neuron {args:?}:\n{t}");
    t
}

const X: &str = "examples/corpus/cell";

/// A package directory with the given files, written fresh.
fn package(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = scratch(name);
    for (file, body) in files {
        std::fs::write(dir.join(file), body).unwrap();
    }
    dir
}

fn example(name: &str) -> String {
    std::fs::read_to_string(root().join("examples").join(name)).unwrap()
}

/// `neuron cell check` of the package must fail with `code` rendered with its location.
fn check_fails_with(name: &str, files: &[(&str, &str)], code: &str) -> String {
    let dir = package(name, files);
    let t = fails(&["cell", "check", &s(&dir)], 1);
    assert!(t.contains(&format!("error[{code}]")), "{code}:\n{t}");
    assert!(t.contains(" --> "), "{code} has no location:\n{t}");
    t
}

// ---- check ------------------------------------------------------------------------------

#[test]
fn check_passes_every_corpus_cell_and_prints_the_summary() {
    for f in [
        "x01_two_devices",
        "x02_robot_zone",
        "x03_cycle_sequence",
        "x04_event_from_fpga",
    ] {
        let t = ok(&["cell", "check", &format!("{X}/{f}.neuron")]);
        assert!(t.contains("devices") && t.contains("verify:"), "{f}:\n{t}");
    }
    let t = ok(&["cell", "check", &format!("{X}/x01_two_devices.neuron")]);
    assert!(
        t.contains("l0  ctl.ready -> plc.start  modbus_tcp  worst 33.005 ms  budget 50 ms  ok  trust generic"),
        "{t}"
    );
    let t = ok(&["cell", "check", &format!("{X}/x02_robot_zone.neuron")]);
    assert!(t.contains("zone press: mutex proved"), "{t}");
}

#[test]
fn check_needs_a_cell_and_a_path() {
    fails(&["cell", "check"], 2);
    fails(&["cell"], 2);
    fails(&["cell", "frobnicate", "x"], 2);
    let dir = package(
        "nocell",
        &[("m.neuron", "module m for host\nsignal a: bool\n")],
    );
    let t = fails(&["cell", "check", &s(&dir)], 1);
    assert!(t.contains("no `cell`"), "{t}");
}

#[test]
fn the_usage_lists_the_cell_commands() {
    let t = ok(&["--help"]);
    for c in ["cell check", "cell build", "cell sim", "cell dossier"] {
        assert!(t.contains(&format!("neuron {c}")), "{c}");
    }
}

// ---- one test per diagnostic family -----------------------------------------------------

#[test]
fn an_unknown_device_type_is_ne0607() {
    let t = check_fails_with(
        "ne0607",
        &[("c.neuron", "cell C\n  device a: no_such_device @ usb\n")],
        "NE0607",
    );
    assert!(t.contains("no_such_device"), "{t}");
}

#[test]
fn a_link_over_its_budget_is_ne0701_with_alternatives() {
    let t = check_fails_with(
        "ne0701",
        &[(
            "c.neuron",
            "cell C\n  device a: nucleo_f446re @ usb\n  device b: nucleo_f446re @ usb\n  link a.x -> b.y via modbus_tcp within 200 us\n",
        )],
        "NE0701",
    );
    assert!(t.contains("= help:") && t.contains("via zero_hop"), "{t}");
}

#[test]
fn a_tag_with_mismatched_units_is_ne0703() {
    let module = |name: &str, unit: &str| {
        format!("module {name} for plc\nscan 10 ms\nmemory:\n  speed: f32 {unit}\n")
    };
    let (ma, mb) = (module("ma", "mm/s"), module("mb", "m/s"));
    check_fails_with(
        "ne0703",
        &[
            (
                "c.neuron",
                "cell C\n  device a: generic_s7_1200 @ ip(192.168.0.10)\n  device b: generic_s7_1200 @ ip(192.168.0.11)\n  place ma on a\n  place mb on b\n  tag v: f32 mm/s @ a.speed, b.speed\n",
            ),
            ("ma.neuron", &ma),
            ("mb.neuron", &mb),
        ],
        "NE0703",
    );
}

#[test]
fn a_module_on_a_device_that_cannot_run_it_is_ne0704() {
    let conveyor = example("conveyor_plc.neuron");
    let t = check_fails_with(
        "ne0704",
        &[
            ("conveyor_plc.neuron", &conveyor),
            (
                "c.neuron",
                "cell C\n  device ctl: nucleo_f446re @ usb\n  place conveyor on ctl\n",
            ),
        ],
        "NE0704",
    );
    assert!(t.contains("conveyor"), "{t}");
}

#[test]
fn a_module_that_fits_no_device_is_ne0705() {
    let conveyor = example("conveyor_plc.neuron");
    check_fails_with(
        "ne0705",
        &[
            ("conveyor_plc.neuron", &conveyor),
            ("c.neuron", "cell C\n  device arm: generic_scara @ usb\n"),
        ],
        "NE0705",
    );
}

#[test]
fn a_wired_link_between_two_voltage_domains_is_ne0801() {
    let t = check_fails_with(
        "ne0801",
        &[(
            "c.neuron",
            "cell C\n  device fpga: generic_artix7_board @ usb\n  device plc: generic_s7_1200 @ usb\n  link fpga.x -> plc.y via zero_hop(gpio) within 5 ms\n",
        )],
        "NE0801",
    );
    assert!(t.contains("opto_24v"), "{t}");
}

#[test]
fn a_robot_motion_into_a_zone_without_a_lock_is_ne0805() {
    const ROBOT: &str = "module rm for robot\nframe table = frame(origin: xyz(300, 0, 0) mm, rot: rpy(0, 0, 0) deg)\npoint inside = table.xyz(-100, 40, 10) mm\nsequence go():\n  move linear to inside speed 50 mm/s\n";
    check_fails_with(
        "ne0805",
        &[
            (
                "c.neuron",
                "cell C\n  device arm: generic_scara @ ip(192.168.0.20)\n  device plc: generic_s7_1200 @ ip(192.168.0.10)\n  zone press: box(x: 100..220 mm, y: -40..40 mm, z: 0..120 mm)\n  place rm on arm\n",
            ),
            ("rm.neuron", ROBOT),
        ],
        "NE0805",
    );
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
";

#[test]
fn a_violated_cell_property_is_ne0901_with_a_counterexample() {
    let src = format!("{VERIFY_CELL}  verify:\n    never B.Picking\n");
    let t = check_fails_with("ne0901", &[("v.neuron", &src)], "NE0901");
    assert!(t.contains("counterexample for `never B.Picking`"), "{t}");
    assert!(t.contains("tick   0"), "{t}");
}

#[test]
fn neuron_check_also_checks_the_cells_of_a_package() {
    // The analyzer no longer checks `verify:` of a cell (D4.3): `neuron check` does.
    let bad = package(
        "check-bad",
        &[(
            "v.neuron",
            &format!("{VERIFY_CELL}  verify:\n    never B.Picking\n"),
        )],
    );
    let t = fails(&["check", &s(&bad)], 1);
    assert!(t.contains("error[NE0901]"), "{t}");
    assert!(t.contains("v.neuron: 1 error(s)"), "{t}");
    let good = package(
        "check-good",
        &[(
            "v.neuron",
            &format!("{VERIFY_CELL}  verify:\n    never B.Picking and A.Idle\n"),
        )],
    );
    ok(&["check", &s(&good)]);
    // Modules that fit no device do not fail `neuron check` of a package with cells.
    let t = ok(&["check", "examples"]);
    assert!(t.contains("left out of the cell checks"), "{t}");
}

// ---- build ------------------------------------------------------------------------------

#[test]
fn build_writes_the_files_of_every_device() {
    let out = scratch("build");
    let t = ok(&[
        "cell",
        "build",
        &format!("{X}/x02_robot_zone.neuron"),
        "-o",
        &s(&out),
    ]);
    assert!(t.contains("arm: generic_scara (spel)"), "{t}");
    for f in [
        "links.json",
        "watchdogs.json",
        "arm/NeuronSlave.prg",
        "arm/zones.json",
        "plc/PLC_tags.csv",
        "plc/register_map.csv",
        "plc/links/link_presscell_0_tx.neuron",
    ] {
        assert!(out.join(f).is_file(), "{f} is missing in {}", out.display());
    }
    // Deterministic.
    let again = scratch("build2");
    ok(&[
        "cell",
        "build",
        &format!("{X}/x02_robot_zone.neuron"),
        "-o",
        &s(&again),
    ]);
    for f in ["links.json", "plc/PLC_tags.csv", "arm/NeuronSlave.prg"] {
        assert_eq!(
            std::fs::read_to_string(out.join(f)).unwrap(),
            std::fs::read_to_string(again.join(f)).unwrap(),
            "{f}"
        );
    }
}

#[test]
fn build_refuses_a_cell_with_errors_and_writes_nothing() {
    let dir = package(
        "build-bad",
        &[(
            "c.neuron",
            "cell C\n  device a: nucleo_f446re @ usb\n  device b: nucleo_f446re @ usb\n  link a.x -> b.y via modbus_tcp within 200 us\n",
        )],
    );
    let out = scratch("build-bad-out").join("nothing");
    let t = fails(&["cell", "build", &s(&dir), "-o", &s(&out)], 1);
    assert!(
        t.contains("NE0701") && t.contains("nothing was built"),
        "{t}"
    );
    assert!(!out.exists());
}

#[test]
fn several_cells_need_a_choice() {
    let t = fails(&["cell", "build", X], 2);
    assert!(t.contains("--cell"), "{t}");
    let out = scratch("choice");
    ok(&["cell", "build", X, "--cell", "Tracking", "-o", &s(&out)]);
    assert!(out.join("fpga/top.v").is_file());
    let t = fails(&["cell", "build", X, "--cell", "Nope"], 1);
    assert!(t.contains("no cell `Nope`"), "{t}");
}

// ---- sim --------------------------------------------------------------------------------

const PRODUCER: &str =
    "module producer for host\nsignal ready: bool\ntask tick every 20 ms:\n  ready = not ready\n";
const CONSUMER: &str = "module consumer for host\nsignal start: bool\nio led: gpio.output @ GPIO17\ntask poll every 5 ms:\n  led.write(start)\nsafety:\n  safe_state:\n    led.write(false)\n";
const PAIR: &str = "cell Pair\n  device a: raspberry_pi_5 @ ip(192.168.0.30)\n  device b: raspberry_pi_5 @ ip(192.168.0.31)\n  place producer on a\n  place consumer on b\n  link a.ready -> b.start via modbus_tcp every 20 ms within 200 ms\n";

fn pair(name: &str) -> PathBuf {
    package(
        name,
        &[
            ("pair.neuron", PAIR),
            ("producer.neuron", PRODUCER),
            ("consumer.neuron", CONSUMER),
        ],
    )
}

#[test]
fn sim_runs_the_devices_together_and_writes_the_replay() {
    let dir = pair("sim");
    let replay = scratch("sim-out").join("replay.json");
    let t = ok(&[
        "cell",
        "sim",
        &s(&dir),
        "--for",
        "300ms",
        "--replay",
        &s(&replay),
    ]);
    assert!(t.contains("SEND") && t.contains("DELIV"), "{t}");
    assert!(t.contains("delivery(ies)"), "{t}");
    let json = std::fs::read_to_string(&replay).unwrap();
    assert!(json.contains("\"replay_version\": \"1\""), "{json}");
    assert!(json.contains("\"cosimulation\""), "{json}");
    // Same run, same bytes.
    let t2 = ok(&["cell", "sim", &s(&dir), "--for", "300ms"]);
    let t1 = ok(&["cell", "sim", &s(&dir), "--for", "300ms"]);
    assert_eq!(t1, t2);
}

#[test]
fn sim_cuts_a_link_by_number_or_by_name_and_reports_the_loss() {
    let dir = pair("sim-cut");
    for cut in [
        "l0@100ms",
        "0@100 ms",
        "a.ready->b.start@100ms",
        "a.ready -> b.start@100ms",
    ] {
        let t = ok(&["cell", "sim", &s(&dir), "--for", "400ms", "--cut", cut]);
        assert!(t.contains("CUT"), "{cut}:\n{t}");
        assert!(
            t.contains("loss of l0 at 100 ms: `b` fault at") && t.contains("(within)"),
            "{cut}:\n{t}"
        );
    }
}

#[test]
fn sim_refuses_a_cut_it_cannot_resolve() {
    let dir = pair("sim-badcut");
    for cut in ["l7@10ms", "nonsense@10ms", "l0", "l0@soon", "x.y->z.w@10ms"] {
        let t = fails(&["cell", "sim", &s(&dir), "--cut", cut], 2);
        assert!(t.contains("error:"), "{cut}:\n{t}");
    }
    fails(&["cell", "sim", &s(&dir), "--for", "banana"], 2);
    fails(&["cell", "sim", &s(&dir), "--cut"], 2);
    fails(&["cell", "sim", &s(&dir), "--bogus", "1"], 2);
}

#[test]
fn sim_drives_an_input_from_outside() {
    let dir = pair("sim-drive");
    let t = ok(&[
        "cell",
        "sim",
        &s(&dir),
        "--for",
        "100ms",
        "--drive",
        "a.ready=true@30ms",
    ]);
    assert!(t.contains("ready"), "{t}");
    fails(&["cell", "sim", &s(&dir), "--drive", "ready"], 2);
}

// ---- dossier ----------------------------------------------------------------------------

#[test]
fn dossier_writes_markdown_and_one_html_file() {
    let out = scratch("dossier");
    let t = ok(&[
        "cell",
        "dossier",
        &format!("{X}/x02_robot_zone.neuron"),
        "-o",
        &s(&out),
        "--for",
        "100ms",
    ]);
    assert!(t.contains("arm: built") && t.contains("plc: built"), "{t}");
    let md = std::fs::read_to_string(out.join("dossier.md")).unwrap();
    let html = std::fs::read_to_string(out.join("dossier.html")).unwrap();
    assert!(
        md.contains("design aid, not a certified safety tool"),
        "{md}"
    );
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(!html.contains("http"), "the HTML must be self-contained");
    let again = scratch("dossier2");
    ok(&[
        "cell",
        "dossier",
        &format!("{X}/x02_robot_zone.neuron"),
        "-o",
        &s(&again),
        "--for",
        "100ms",
    ]);
    assert_eq!(
        md,
        std::fs::read_to_string(again.join("dossier.md")).unwrap()
    );
}
