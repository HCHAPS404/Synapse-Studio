//! Task B4: the hardware commands of the tool, run as a user runs them.
//! The builds skip when `avr-gcc` or `gcc` is missing, unless `NEURON_REQUIRE_TOOLCHAIN` is set.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn neuron(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_neuron"))
        .args(args)
        .current_dir(cwd)
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

fn scratch(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("neuron-b4-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (n, t) in files {
        std::fs::write(dir.join(n), t).unwrap();
    }
    dir
}

fn have(env: &str, names: &[&str]) -> bool {
    if neuron_codegen_c::find_tool(env, names).is_some() {
        return true;
    }
    assert!(
        std::env::var_os("NEURON_REQUIRE_TOOLCHAIN").is_none(),
        "{names:?} is required (NEURON_REQUIRE_TOOLCHAIN is set) but was not found"
    );
    eprintln!("skipped: {names:?} not found (see docs/phase-b-prerequisites.md)");
    false
}

const BLINK: &str = "module blink for mcu on arduino_uno_r3\n\
io led: gpio.output @ D13\n\
task t every 500 ms (prio 1):\n  led.toggle()\n\
safety:\n  safe_state:\n    led.write(false)\n";

#[test]
fn targets_lists_every_board_with_its_tier() {
    let o = neuron(&["targets"], &std::env::temp_dir());
    assert!(o.status.success(), "{}", text(&o));
    let t = text(&o);
    for b in [
        "arduino_uno_r3",
        "arduino_mega_2560",
        "raspberry_pi_pico",
        "esp32_s3_devkit",
        "nucleo_n657x0_q",
    ] {
        assert!(t.contains(b), "{b} missing from\n{t}");
    }
    assert!(
        t.lines()
            .any(|l| l.starts_with("arduino_uno_r3") && l.contains("runtime")),
        "{t}"
    );
}

#[test]
fn targets_describes_one_board() {
    let o = neuron(&["targets", "arduino_uno_r3"], &std::env::temp_dir());
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(
        t.contains("atmega328p") && t.contains("D13") && t.contains("PB5"),
        "{t}"
    );
    let o = neuron(&["targets", "no_such_board"], &std::env::temp_dir());
    assert!(!o.status.success());
    assert!(text(&o).contains("NE0607"));
}

#[test]
fn check_reports_hardware_errors_with_a_location() {
    let src = "module bad for mcu on arduino_uno_r3\n\
io a: gpio.output @ D13\n\
io b: gpio.output @ D13\n\
task t every 10 ms (prio 1):\n  a.write(true)\n  b.write(true)\n\
safety:\n  safe_state:\n    a.write(false)\n    b.write(false)\n";
    let dir = scratch("check", &[("bad.neuron", src)]);
    let o = neuron(&["check", "bad.neuron"], &dir);
    let t = text(&o);
    assert!(!o.status.success(), "{t}");
    assert!(t.contains("error[NE0601]"), "{t}");
    assert!(t.contains("bad.neuron:3:"), "{t}");
}

#[test]
fn emit_c_prints_the_program_and_writes_the_runtime() {
    let dir = scratch("emit", &[("blink.neuron", BLINK)]);
    let o = neuron(&["emit-c", "blink.neuron"], &dir);
    assert!(o.status.success(), "{}", text(&o));
    assert!(String::from_utf8_lossy(&o.stdout).contains("#include \"nrt.h\""));
    let o = neuron(&["emit-c", "blink.neuron", "-o", "c"], &dir);
    assert!(o.status.success(), "{}", text(&o));
    for f in [
        "app.c",
        "app.map.json",
        "nrt.h",
        "nrt_core.c",
        "nrt_port_avr.c",
    ] {
        assert!(dir.join("c").join(f).is_file(), "{f}");
    }
}

#[test]
fn unsupported_constructs_stop_the_build_with_ne0610() {
    // `debounce` is a native function block, outside the subset of the C backend.
    let src = include_str!("../../../examples/corpus/mcu/m03_debounced_button.neuron");
    let dir = scratch("ne0610", &[("m.neuron", src)]);
    let o = neuron(&["emit-c", "m.neuron"], &dir);
    let t = text(&o);
    assert!(!o.status.success(), "{t}");
    assert!(t.contains("NE0610"), "{t}");
}

#[test]
fn build_makes_a_hex_for_the_uno() {
    if !have("NEURON_AVR_GCC", &["avr-gcc"]) {
        return;
    }
    let dir = scratch("build", &[("blink.neuron", BLINK)]);
    let o = neuron(&["build", "blink.neuron"], &dir);
    assert!(o.status.success(), "{}", text(&o));
    let hex = dir.join("build/arduino_uno_r3/app.hex");
    let body = std::fs::read_to_string(&hex).unwrap();
    assert!(
        body.starts_with(':') && body.trim_end().ends_with(":00000001FF"),
        "{body}"
    );
}

#[test]
fn board_override_retargets_to_the_mega() {
    if !have("NEURON_AVR_GCC", &["avr-gcc"]) {
        return;
    }
    let dir = scratch("mega", &[("blink.neuron", BLINK)]);
    let o = neuron(
        &[
            "build",
            "blink.neuron",
            "--board",
            "arduino_mega_2560",
            "--out",
            "m",
        ],
        &dir,
    );
    assert!(o.status.success(), "{}", text(&o));
    assert!(dir.join("m/app.hex").is_file());
}

#[test]
fn described_boards_are_checked_but_not_built() {
    // The N657 board is a placeholder: described and checked, no runtime port.
    let src = BLINK
        .replace("arduino_uno_r3", "nucleo_n657x0_q")
        .replace("D13", "PA5");
    let dir = scratch("pico", &[("blink.neuron", &src)]);
    let o = neuron(&["emit-c", "blink.neuron"], &dir);
    assert!(o.status.success(), "{}", text(&o));
    let o = neuron(&["build", "blink.neuron"], &dir);
    let t = text(&o);
    assert!(!o.status.success(), "{t}");
    assert!(t.contains("described"), "{t}");
}

#[test]
fn a_host_build_runs_in_simulated_time() {
    if !have("NEURON_CC", &["gcc", "cc", "clang"]) {
        return;
    }
    let dir = scratch("host", &[("blink.neuron", BLINK)]);
    let o = neuron(&["build", "blink.neuron", "--host", "--out", "h"], &dir);
    assert!(o.status.success(), "{}", text(&o));
    let exe = dir.join(format!("h/app_host{}", std::env::consts::EXE_SUFFIX));
    let run = Command::new(exe).args(["--for", "1100"]).output().unwrap();
    let out = String::from_utf8_lossy(&run.stdout);
    assert!(
        out.contains("led = true") && out.contains("led = false"),
        "{out}"
    );
}

#[test]
fn a_native_build_needs_no_compiler() {
    let dir = scratch("native", &[("blink.neuron", BLINK)]);
    let o = neuron(&["build", "blink.neuron", "--native", "--out", "n"], &dir);
    assert!(o.status.success(), "{}", text(&o));
    let hex = std::fs::read_to_string(dir.join("n/app.hex")).unwrap();
    assert!(hex.trim_end().ends_with(":00000001FF"));
    assert!(dir.join("n/app.lst").is_file());
}

#[test]
fn emit_verilog_writes_the_design_and_its_constraints() {
    let src = include_str!("../../../examples/pwm_hdl.neuron");
    let dir = scratch("verilog", &[("pwm.neuron", src)]);
    let o = neuron(&["emit-verilog", "pwm.neuron"], &dir);
    assert!(o.status.success(), "{}", text(&o));
    assert!(String::from_utf8_lossy(&o.stdout).contains("module pwm ("));
    let o = neuron(
        &[
            "emit-verilog",
            "pwm.neuron",
            "--board",
            "artix7_35t_board",
            "-o",
            "fpga",
        ],
        &dir,
    );
    assert!(o.status.success(), "{}", text(&o));
    let xdc = std::fs::read_to_string(dir.join("fpga/top.xdc")).unwrap();
    assert!(xdc.contains("PACKAGE_PIN W5"), "{xdc}");
    let t = text(&neuron(&["targets"], &dir));
    assert!(
        t.contains("ice40up5k_breakout") && t.contains("artix7_35t_board"),
        "{t}"
    );
}

#[test]
fn a_screen_on_the_uno_that_cannot_fit_is_ne0604() {
    let src = include_str!("../../../examples/panel_ui.neuron")
        .replace("esp32_s3_devkit", "arduino_uno_r3");
    let dir = scratch("ne0604", &[("panel.neuron", &src)]);
    let o = neuron(&["emit-c", "panel.neuron"], &dir);
    let t = text(&o);
    assert!(!o.status.success(), "{t}");
    assert!(t.contains("NE0604"), "{t}");
}

#[test]
fn flash_needs_a_port() {
    let dir = scratch("flash", &[("blink.neuron", BLINK)]);
    let o = neuron(&["flash", "blink.neuron"], &dir);
    assert!(!o.status.success());
    assert!(text(&o).contains("--port"));
}
