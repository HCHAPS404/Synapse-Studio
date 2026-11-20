//! Task C7: `neuron emit` and `neuron import`, run as a user runs them.
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
    let dir = std::env::temp_dir().join(format!("neuron-c7-{}-{name}", std::process::id()));
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

const P01: &str = "examples/corpus/plc/p01_start_stop.neuron";
const P03: &str = "examples/corpus/plc/p03_on_delay.neuron";
const P09: &str = "examples/corpus/plc/p09_sfc_mixer.neuron";
const R01: &str = "examples/corpus/robot/r01_home_and_wave.neuron";
const R02: &str = "examples/corpus/robot/r02_palletize.neuron";
const M06: &str = "examples/corpus/mcu/m06_traffic_light.neuron";
const H01: &str = "examples/corpus/host/h01_gateway_counter.neuron";

fn has(dir: &Path, names: &[&str]) {
    for n in names {
        assert!(dir.join(n).is_file(), "{n} is missing in {}", dir.display());
    }
}

fn first_with(dir: &Path, ext: &str, skip: &[&str]) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == ext)
                && !skip.iter().any(|k| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(k))
                })
        })
        .collect();
    found.sort();
    found
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no .{ext} file in {}", dir.display()))
}

// ---- emit ----------------------------------------------------------------------------

#[test]
fn emit_st_generic_and_yaskawa() {
    let d = scratch("st");
    let g = s(&d.join("generic"));
    let t = ok(&["emit", "st", P01, "-o", &g]);
    assert!(t.contains("wrote") && t.contains("fidelity:"), "{t}");
    has(
        &d.join("generic"),
        &[
            "p01_start_stop.st",
            "config.st",
            "fidelity.json",
            "wiring.md",
        ],
    );
    let y = s(&d.join("yaskawa"));
    ok(&["emit", "st", P01, "--profile", "yaskawa", "-o", &y]);
    has(
        &d.join("yaskawa"),
        &["p01_start_stop.st", "GlobalVars.st", "fidelity.json"],
    );
    assert!(!d.join("yaskawa").join("config.st").exists());
}

#[test]
fn emit_scl_and_plcopen() {
    let d = scratch("scl");
    let a = s(&d.join("scl"));
    ok(&["emit", "scl", P01, "-o", &a]);
    has(
        &d.join("scl"),
        &["p01_start_stop.scl", "PLC_tags.csv", "fidelity.json"],
    );
    let b = s(&d.join("xml"));
    ok(&["emit", "plcopen", P09, "-o", &b]);
    let xml = first_with(&d.join("xml"), "xml", &[]);
    let body = std::fs::read_to_string(xml).unwrap();
    assert!(body.contains("<project") && body.contains("<SFC"), "{body}");
}

#[test]
fn emit_spel_with_the_default_robot_and_ne0804() {
    let d = scratch("spel");
    let a = s(&d.join("ok"));
    let t = ok(&["emit", "spel", R02, "-o", &a]);
    assert!(t.contains("fidelity:"), "{t}");
    has(
        &d.join("ok"),
        &[
            "r02_palletize.prg",
            "NeuronLib.prg",
            "NeuronSlave.prg",
            "fidelity.json",
        ],
    );
    // A manifest with a short reach: the targets of the palletizing loop leave it.
    let toml = d.join("small.toml");
    std::fs::write(
        &toml,
        "[robot]\nname = \"small\"\nkind = \"articulated\"\nreach_mm = 100.0\n\
         shoulder_height_mm = 300.0\nz_min_mm = -100.0\nz_max_mm = 900.0\n\
         joint_limits_deg = [[-170.0, 170.0], [-135.0, 65.0], [-60.0, 150.0], \
         [-190.0, 190.0], [-135.0, 135.0], [-360.0, 360.0]]\n",
    )
    .unwrap();
    let b = s(&d.join("small"));
    let t = fails(&["emit", "spel", R02, "--robot", &s(&toml), "-o", &b], 1);
    assert!(t.contains("NE0804"), "{t}");
    assert!(t.contains("r02_palletize.neuron:"), "{t}");
    assert!(!d.join("small").exists(), "nothing is written on errors");
    // An unknown manifest name is a diagnostic, not a crash.
    let t = fails(
        &["emit", "spel", R02, "--robot", "no_such_robot", "-o", &b],
        1,
    );
    assert!(t.contains("no_such_robot"), "{t}");
}

#[test]
fn emit_python_and_kernel() {
    let d = scratch("py");
    let a = s(&d.join("py"));
    ok(&["emit", "python", H01, "-o", &a]);
    has(&d.join("py"), &["neuron_rt.py", "fidelity.json"]);
    first_with(&d.join("py"), "py", &["neuron_rt"]);
    let b = s(&d.join("kernel"));
    let t = ok(&["emit", "kernel", M06, "-o", &b]);
    assert!(t.contains("fidelity:"), "{t}");
    has(
        &d.join("kernel"),
        &[
            "neuron_kernel.st",
            "nrt_kernel.c",
            "nrt_kernel.h",
            "neuron_kernel.spel",
        ],
    );
    first_with(&d.join("kernel"), "hex", &[]);
}

#[test]
fn emit_c_writes_plain_c99_for_core_modules_and_ne0610_otherwise() {
    let d = scratch("c99");
    let a = s(&d.join("c"));
    let t = ok(&[
        "emit",
        "c",
        "examples/corpus/core/c02_enum_match.neuron",
        "-o",
        &a,
    ]);
    assert!(t.contains("fidelity:"), "{t}");
    has(
        &d.join("c"),
        &[
            "c02_enum_match.c",
            "c02_enum_match.h",
            "nrt_core.h",
            "nrt_core.c",
            "fidelity.json",
            "c02_enum_match.c.map.json",
        ],
    );
    let h = std::fs::read_to_string(d.join("c").join("c02_enum_match.h")).unwrap();
    assert!(h.contains("uint16_t c02_enum_match_speed_limit("), "{h}");
    // A module that is not `core`, and a `core` construct outside the subset.
    let b = s(&d.join("plc"));
    let t = fails(&["emit", "c", P01, "-o", &b], 1);
    assert!(t.contains("NE0610"), "{t}");
    assert!(!d.join("plc").exists());
    let t = fails(
        &[
            "emit",
            "c",
            "examples/corpus/core/c03_option_result.neuron",
            "-o",
            &b,
        ],
        1,
    );
    assert!(
        t.contains("NE0610") && t.contains("c03_option_result.neuron:"),
        "{t}"
    );
    // The firmware command is a different thing and still there.
    assert!(text(&neuron(&["help"])).contains("emit-c"));
}

#[test]
fn the_policy_refuses_a_translation_and_writes_nothing() {
    let d = scratch("policy");
    let a = s(&d.join("refused"));
    let t = fails(&["emit", "st", P03, "--policy", "exact", "-o", &a], 1);
    assert!(t.contains("NE1003"), "{t}");
    assert!(t.contains("p03_on_delay.neuron:"), "{t}");
    assert!(!d.join("refused").exists(), "nothing is written");
    let b = s(&d.join("accepted"));
    ok(&["emit", "st", P03, "--policy", "approximate", "-o", &b]);
    has(&d.join("accepted"), &["p03_on_delay.st"]);
    fails(&["emit", "st", P03, "--policy", "perfect", "-o", &b], 2);
}

#[test]
fn an_unsupported_construct_is_ne0610_with_its_location() {
    let d = scratch("ne0610");
    let a = s(&d.join("out"));
    let t = fails(&["emit", "kernel", R01, "-o", &a], 1);
    assert!(t.contains("NE0610"), "{t}");
    assert!(t.contains("r01_home_and_wave.neuron:"), "{t}");
    assert!(!d.join("out").exists());
}

#[test]
fn emit_usage_errors_exit_with_2() {
    fails(&["emit"], 2);
    fails(&["emit", "cobol", P01], 2);
    fails(&["emit", "st"], 2);
    fails(&["emit", "python", H01, "--ros2"], 2);
    fails(&["emit", "st", P01, "--policy"], 2);
    fails(&["import"], 2);
    fails(&["import", "cobol", P01], 2);
}

#[test]
fn emit_analyzes_first_and_stops_on_errors() {
    let d = scratch("bad");
    let f = d.join("bad.neuron");
    std::fs::write(&f, "module bad for plc\nlamp = oops\n").unwrap();
    let o = s(&d.join("out"));
    fails(&["emit", "st", &s(&f), "-o", &o], 1);
    assert!(!d.join("out").exists());
}

// ---- import --------------------------------------------------------------------------

/// Emits `example`, imports the main generated file and checks that the result passes
/// `neuron check`.
fn round_trip(emitter: &str, example: &str, ext: &str, lang: &str, skip: &[&str]) -> String {
    let d = scratch(&format!("rt-{emitter}"));
    let out = s(&d.join("emitted"));
    ok(&["emit", emitter, example, "-o", &out]);
    let file = first_with(&d.join("emitted"), ext, skip);
    let neu = d.join("imported").join("back.neuron");
    let t = ok(&["import", lang, &s(&file), "-o", &s(&neu)]);
    assert!(neu.is_file(), "{t}");
    let report = d.join("imported").join("back.fidelity.json");
    let json = std::fs::read_to_string(report).unwrap();
    assert!(json.contains("\"tool\": \"import-"), "{json}");
    let c = ok(&["check", &s(&neu)]);
    assert!(c.contains("ok"), "{c}");
    std::fs::read_to_string(&neu).unwrap()
}

#[test]
fn import_st_round_trip() {
    let src = round_trip(
        "st",
        P01,
        "st",
        "st",
        &["NeuronLib", "config", "GlobalVars"],
    );
    assert!(src.contains("plc"), "{src}");
}

#[test]
fn import_scl_round_trip() {
    round_trip("scl", P01, "scl", "scl", &["NeuronLib"]);
}

#[test]
fn import_plcopen_round_trip() {
    let src = round_trip("plcopen", P09, "xml", "plcopen", &[]);
    assert!(src.contains("sequence"), "{src}");
}

#[test]
fn import_spel_round_trip() {
    let src = round_trip("spel", R02, "prg", "spel", &["NeuronLib", "NeuronSlave"]);
    assert!(src.contains("robot"), "{src}");
}

#[test]
fn import_c_and_verilog() {
    let d = scratch("cv");
    let c = d.join("math.c");
    std::fs::write(
        &c,
        "#include <stdint.h>\n\nint32_t clamp(int32_t x, int32_t lo, int32_t hi) {\n    \
         if (x < lo) return lo;\n    if (x > hi) return hi;\n    return x;\n}\n",
    )
    .unwrap();
    let neu = d.join("math.neuron");
    ok(&["import", "c", &s(&c), "-o", &s(&neu)]);
    ok(&["check", &s(&neu)]);
    assert!(std::fs::read_to_string(&neu).unwrap().contains("clamp"));
    // Verilog: the golden file of the HDL backend; without -o the source goes to stdout.
    let v = "conformance/golden/verilog/d01_counter.v";
    let o = neuron(&["import", "verilog", v]);
    assert!(o.status.success(), "{}", text(&o));
    let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(stdout.contains("hdl"), "{stdout}");
    let neu = d.join("d01.neuron");
    std::fs::write(&neu, stdout).unwrap();
    ok(&["check", &s(&neu)]);
}

#[test]
fn import_reports_ne1002_and_continues() {
    let d = scratch("ne1002");
    let f = d.join("loop.c");
    std::fs::write(
        &f,
        "#include <stdint.h>\n\nint32_t twice(int32_t x) {\n    return x + x;\n}\n\n\
         int32_t spin(int32_t x) {\n    while (x > 0) {\n        x = x - 1;\n    }\n    return x;\n}\n",
    )
    .unwrap();
    let t = ok(&["import", "c", &s(&f)]);
    assert!(t.contains("NE1002"), "{t}");
    assert!(t.contains("loop.c:"), "{t}");
    assert!(t.contains("twice"), "{t}");
}

#[test]
fn import_ne1001_is_a_nonzero_exit() {
    let d = scratch("ne1001");
    let f = d.join("broken.st");
    std::fs::write(&f, "PROGRAM broken\nVAR x : ;\nEND_PROGRAM\n").unwrap();
    let out = d.join("never.neuron");
    let t = fails(&["import", "st", &s(&f), "-o", &s(&out)], 1);
    assert!(t.contains("NE1001"), "{t}");
    assert!(t.contains("broken.st:"), "{t}");
    assert!(!out.exists());
    fails(&["import", "st", &s(&d.join("missing.st"))], 2);
}
