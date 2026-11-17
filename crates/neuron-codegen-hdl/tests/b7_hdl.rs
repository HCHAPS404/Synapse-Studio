//! Task B7: the hdl dialect to Verilog and FPGA constraints.
//!
//! The golden files fix the Verilog of every hdl program of the corpus. The simulation and
//! synthesis tests run iverilog and yosys when they are installed (CI installs them and sets
//! `NEURON_REQUIRE_HDL`, which turns a missing tool into a failure).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_codegen_hdl::tools::{find, simulate_iverilog, synth_check_yosys};
use neuron_codegen_hdl::{emit_constraints, emit_verilog, Family, FpgaBoard, Verilog};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

fn nir_of(src: &str) -> neuron_nir::Nir {
    let a = analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: src.replace("\r\n", "\n"),
        }],
        None,
        &SemaOptions::default(),
    );
    match a.nir {
        Some(n) => n,
        None => panic!("analysis errors:\n{}", a.render()),
    }
}

fn verilog(src: &str) -> Verilog {
    match emit_verilog(&nir_of(src)) {
        Ok(v) => v,
        Err(d) => panic!("{d:?}"),
    }
}

fn corpus() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = vec![(
        "pwm_hdl".to_string(),
        std::fs::read_to_string(root.join("examples/pwm_hdl.neuron")).unwrap(),
    )];
    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("examples/corpus/hdl"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    files.sort();
    for f in files {
        let name = f.file_stem().unwrap().to_string_lossy().to_string();
        out.push((name, std::fs::read_to_string(f).unwrap()));
    }
    out
}

fn have(tool: &str) -> bool {
    if find(tool).is_some() {
        return true;
    }
    assert!(
        std::env::var_os("NEURON_REQUIRE_HDL").is_none(),
        "{tool} is required (NEURON_REQUIRE_HDL is set) but was not found"
    );
    eprintln!("skipped: {tool} not found (see docs/hdl.md)");
    false
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("neuron-b7-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn every_hdl_program_of_the_corpus_has_golden_verilog() {
    for (name, src) in corpus() {
        let v = verilog(&src);
        assert!(v.text.contains("module "), "{name}");
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../conformance/golden/verilog/{name}.v"));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &v.text).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing {}; run with UPDATE_GOLDEN=1", path.display()))
            .replace("\r\n", "\n");
        assert!(
            want == v.text,
            "{name}: the generated Verilog changed; review it and run with UPDATE_GOLDEN=1"
        );
    }
}

#[test]
fn the_uart_transmitter_has_its_divisor_registers_and_ports() {
    let (_, src) = corpus()
        .into_iter()
        .find(|(n, _)| n == "d05_uart_tx")
        .unwrap();
    let v = verilog(&src);
    // 100 MHz / 115200 Bd rounded.
    assert!(
        v.text.contains("localparam integer divisor = 868;"),
        "{}",
        v.text
    );
    assert!(v.text.contains("reg [9:0] shift = 10'd1023;"), "{}", v.text);
    assert!(v.text.contains("always @(posedge clk) begin"));
    assert!(v.text.contains("{1'b1, data, 1'b0}"), "{}", v.text);
    assert!(v.text.contains("output reg  line"), "{}", v.text);
    assert_eq!(v.modules, vec!["tx"]);
}

#[test]
fn verilog_keywords_are_renamed() {
    let (_, src) = corpus()
        .into_iter()
        .find(|(n, _)| n == "d04_edge_detector")
        .unwrap();
    let v = verilog(&src);
    assert!(v.text.contains("module edge_n ("), "{}", v.text);
}

#[test]
fn a_synchronous_reset_port_resets_the_registers() {
    let src = "module r for hdl\nclock clk: 12 MHz @ pin(35)\n\ncircuit ctr(rst: in bit, q: out bit<4>) on clk:\n  reg n: bit<4> = 3\n  on rising(clk):\n    n = n +% 1\n  comb:\n    q = n\n";
    let v = verilog(src);
    assert!(v.text.contains("if (rst) begin"), "{}", v.text);
    assert!(v.text.contains("n <= 4'd3;"), "{}", v.text);
}

#[test]
fn constraints_follow_the_family_of_the_board() {
    let (_, src) = corpus()
        .into_iter()
        .find(|(n, _)| n == "d01_counter")
        .unwrap();
    let v = verilog(&src);
    let x7 = FpgaBoard::builtin("artix7_35t_board").unwrap();
    let xdc = emit_constraints(&v, "counter8", &x7).unwrap();
    assert!(xdc.contains("PACKAGE_PIN W5"), "{xdc}");
    assert!(
        xdc.contains("create_clock -add -name sys_clk -period 10.000"),
        "{xdc}"
    );

    let src = "module b for hdl\nclock clk: 12 MHz @ pin(35)\n\ncircuit blink(led: out bit @ pin(39)) on clk:\n  reg s: bit = 0\n  on rising(clk):\n    s = ~s\n  comb:\n    led = s\n";
    let v = verilog(src);
    let ice = FpgaBoard::builtin("ice40up5k_breakout").unwrap();
    assert_eq!(ice.family, Family::Ice40);
    let pcf = emit_constraints(&v, "blink", &ice).unwrap();
    assert!(
        pcf.contains("set_io clk 35") && pcf.contains("set_io led 39"),
        "{pcf}"
    );
    // A pin that the board does not have.
    let bad = src.replace("pin(39)", "pin(99)");
    let err = emit_constraints(&verilog(&bad), "blink", &ice).unwrap_err();
    assert_eq!(err[0].code, "NE0603");
}

#[test]
fn a_crossing_without_a_synchronizer_is_ne0702() {
    let src = "module c for hdl\nclock fast: 100 MHz @ pin(W5)\nclock slow: 10 MHz @ pin(E3)\n\ncircuit bridge(din: in bit, dout: out bit) on fast:\n  reg a: bit = 0\n  reg b: bit = 0\n  on rising(fast):\n    a = din\n  on rising(slow):\n    b = a\n  comb:\n    dout = b\n";
    let err = emit_verilog(&nir_of(src)).unwrap_err();
    assert!(
        err.iter()
            .any(|d| d.code == "NE0702" && d.message.contains("`a`")),
        "{err:?}"
    );
}

#[test]
fn unsupported_constructs_are_ne0610() {
    let src = "module f for hdl\nclock clk: 12 MHz @ pin(35)\n\ncircuit c(q: out bit) on clk:\n  reg n: bit<4> = 0\n  on rising(clk):\n    for i in 0..4:\n      n = n +% 1\n  comb:\n    q = n[0]\n";
    let a = analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: src.into(),
        }],
        None,
        &SemaOptions::default(),
    );
    // The analysis may already refuse loops in hdl; when it accepts them, the backend refuses.
    if let Some(nir) = a.nir {
        let err = emit_verilog(&nir).unwrap_err();
        assert!(err.iter().all(|d| d.code == "NE0610"), "{err:?}");
    }
}

#[test]
fn the_output_is_deterministic() {
    for (_, src) in corpus() {
        assert_eq!(verilog(&src).text, verilog(&src).text);
    }
}

/// Runs a testbench that prints one line per clock and returns the lines.
fn sim(design: &str, tb: &str, name: &str) -> Option<Vec<String>> {
    if !have("iverilog") || !have("vvp") {
        return None;
    }
    let out = simulate_iverilog(design, tb, &scratch(name)).unwrap();
    Some(
        out.lines()
            .filter(|l| l.starts_with("v "))
            .map(str::to_string)
            .collect(),
    )
}

#[test]
fn the_counter_counts_in_simulation() {
    let (_, src) = corpus()
        .into_iter()
        .find(|(n, _)| n == "d01_counter")
        .unwrap();
    let v = verilog(&src);
    let tb = "`timescale 1ns/1ps\nmodule tb;\n  reg sys = 0;\n  wire [7:0] value;\n  counter8 dut(.sys(sys), .value(value));\n  integer i;\n  initial begin\n    for (i = 0; i < 300; i = i + 1) begin\n      #5 sys = 1; #1 $display(\"v %0d\", value); #4 sys = 0;\n    end\n    $finish;\n  end\nendmodule\n";
    let Some(lines) = sim(&v.text, tb, "counter") else {
        return;
    };
    let want: Vec<String> = (1..=300u32).map(|i| format!("v {}", i % 256)).collect();
    assert_eq!(lines, want);
}

#[test]
fn the_edge_detector_and_the_shift_register_in_simulation() {
    let all = corpus();
    let edge = verilog(
        &all.iter()
            .find(|(n, _)| n == "d04_edge_detector")
            .unwrap()
            .1,
    );
    let tb = "`timescale 1ns/1ps\nmodule tb;\n  reg sys = 0; reg s = 0;\n  wire p;\n  edge_n dut(.sys(sys), .signal_in(s), .pulse(p));\n  integer i;\n  initial begin\n    for (i = 0; i < 12; i = i + 1) begin\n      s = (i >= 3 && i < 8); #1 $display(\"v %0d\", p); #4 sys = 1; #5 sys = 0;\n    end\n    $finish;\n  end\nendmodule\n";
    if let Some(lines) = sim(&edge.text, tb, "edge") {
        // The pulse is high only in the cycle where the input rises (i = 3).
        let want: Vec<String> = (0..12).map(|i| format!("v {}", u8::from(i == 3))).collect();
        assert_eq!(lines, want);
    }
    let sh = verilog(
        &all.iter()
            .find(|(n, _)| n == "d03_shift_register")
            .unwrap()
            .1,
    );
    let tb = "`timescale 1ns/1ps\nmodule tb;\n  reg sys = 0; reg din = 0; reg en = 1;\n  wire [7:0] q;\n  shifter dut(.sys(sys), .din(din), .enable(en), .q(q));\n  integer i;\n  initial begin\n    for (i = 0; i < 8; i = i + 1) begin\n      din = (i % 3 == 0); #5 sys = 1; #1 $display(\"v %0d\", q); #4 sys = 0;\n    end\n    $finish;\n  end\nendmodule\n";
    if let Some(lines) = sim(&sh.text, tb, "shift") {
        let mut bits: u32 = 0;
        let want: Vec<String> = (0..8)
            .map(|i| {
                bits = ((bits << 1) | u32::from(i % 3 == 0)) & 0xff;
                format!("v {bits}")
            })
            .collect();
        assert_eq!(lines, want);
    }
}

#[test]
fn the_uart_transmitter_sends_a_frame_in_simulation() {
    // The same transmitter with a divisor of 4 clocks per bit, so the frame is short.
    let (_, src) = corpus()
        .into_iter()
        .find(|(n, _)| n == "d05_uart_tx")
        .unwrap();
    let src = src.replace("clk.freq / 115200 Bd", "4");
    let v = verilog(&src);
    let tb = "`timescale 1ns/1ps\nmodule tb;\n  reg clk = 0; reg [7:0] data = 8'hA5; reg start = 0;\n  wire line, busy;\n  tx dut(.clk(clk), .data(data), .start(start), .line(line), .busy(busy));\n  integer i;\n  initial begin\n    for (i = 0; i < 48; i = i + 1) begin\n      start = (i == 1); #5 clk = 1; #1 $display(\"v %0d %0d\", line, busy); #4 clk = 0;\n    end\n    $finish;\n  end\nendmodule\n";
    let Some(lines) = sim(&v.text, tb, "uart") else {
        return;
    };
    // Sample the line in the middle of each bit after the start: 0, then 0xA5 LSB first, then 1.
    let bits: Vec<char> = lines.iter().map(|l| l.as_bytes()[2] as char).collect();
    let frame: String = (0..10).map(|b| bits[2 + 4 * b + 1]).collect();
    assert_eq!(frame, "0101001011", "{lines:?}");
}

#[test]
fn every_program_synthesizes_with_yosys() {
    if !have("yosys") {
        return;
    }
    for (name, src) in corpus() {
        let v = verilog(&src);
        let top = v.modules.last().unwrap().clone();
        let cells = synth_check_yosys(
            &v.text,
            &top,
            Family::Ice40,
            &scratch(&format!("yosys-{name}")),
        )
        .unwrap();
        assert!(cells > 0, "{name}: no cells");
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, failure_persistence: None, ..ProptestConfig::default() })]

    /// The declared width of a register is the width of the emitted Verilog register.
    #[test]
    fn register_widths_are_kept(w in 1u32..=32) {
        let src = format!("module p for hdl\nclock clk: 12 MHz @ pin(35)\n\ncircuit c(q: out bit<{w}>) on clk:\n  reg n: bit<{w}> = 0\n  on rising(clk):\n    n = n +% 1\n  comb:\n    q = n\n");
        let v = verilog(&src);
        let decl = if w == 1 { "reg n = 1'd0;".to_string() } else { format!("reg [{}:0] n = {w}'d0;", w - 1) };
        prop_assert!(v.text.contains(&decl), "{}", v.text);
    }
}
