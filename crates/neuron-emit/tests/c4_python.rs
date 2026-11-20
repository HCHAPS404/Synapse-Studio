//! Task C4.1: the Python emitter, checked by running its output.
//!
//! Every `host` program of the corpus (and a few more written here) is emitted to a
//! temporary directory and run with Python in simulated time. `--trace` must print the
//! interpreter's trace as exactly the same text, and `--test` must report the same
//! result, with the same message, for every `test` block as `neuron_interp::run_tests`.
//!
//! Python is found through `NEURON_PYTHON`, then `python`, then `py -3`. When there is
//! none the tests print a note and pass, unless `NEURON_REQUIRE_PYTHON=1`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_emit::{emitter, EmitOptions, Package};
use neuron_interp::{render, run_tests, Interp, Options};
use neuron_nir::Nir;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use std::path::PathBuf;
use std::process::Command;

const H01: &str = include_str!("../../../examples/corpus/host/h01_gateway_counter.neuron");
const H02: &str = include_str!("../../../examples/corpus/host/h02_moving_average.neuron");
const H03: &str = include_str!("../../../examples/corpus/host/h03_retry_loop.neuron");
const H04: &str = include_str!("../../../examples/corpus/host/h04_recursion_free_sum.neuron");
const H05: &str = include_str!("../../../examples/corpus/host/h05_fault_and_reset.neuron");

const X1: &str = "module x1_signals for host

signal speed: f32 mm/s
signal count: u16
signal flag: bool

var n: u16 = 0
var rises: u8 = 0

task a every 100 ms:
  n += 1
  count = n
  speed = speed + 0.5 mm/s

task b every 250 ms (prio 2):
  flag = count > 3

when count > 6:
  rises += 1
";

const X2: &str = "module x2_faults for host

signal out: u8

var k: u8 = 250
var latched: bool = false
var trips: u8 = 0

task bump every 10 ms:
  k += 1
  out = k

safety:
  rate 5 ms
  safe_state:
    trips += 1
  when k == 255:
    latched = true
  reset when latched
";

const X3: &str = "module x3_events for host

signal pings: u32

task src every 100 ms:
  emit ping

task sink on ping:
  pings = pings + 1
";

const X4: &str = "module x4_logic for host

fn collatz(n: u32) -> u32:
  var x: u32 = n
  var steps: u32 = 0
  while x != 1 max 200:
    if x % 2 == 0:
      x = x / 2
    else:
      x = 3 * x + 1
    steps += 1
  return steps

fn mean(values: [f64; 4]) -> f64:
  var s: f64 = 0.0
  for v in values:
    s += v
  return s / 4.0

signal steps: u32
signal avg: f64
signal level: f32
var i: u32 = 1
var gain: f32 = 0.1

task t every 50 ms:
  steps = collatz(i + 5)
  avg = mean([1.5, 2.5, 3.5, i as f64])
  level = gain * 3.0
  i += 1

test \"collatz of 6 takes 8 steps\":
  assert collatz(6) == 8, \"steps\"

test \"the signal follows the task\":
  advance(70 ms)
  assert steps == 8, \"first value\"

test \"this one fails on purpose\":
  assert collatz(1) == 0, \"zero steps\"
  assert false, \"boom\"

test \"overflow is a fault\":
  var k: u8 = 255
  k += 1
";

const BAD: &str = "module bad for host

interface Shape:
  fn area(self) -> f64

task t every 1 s:
  pass
";

/// Name of the interpreter, arguments in front of the script.
fn find_python() -> Option<Vec<String>> {
    let mut candidates: Vec<Vec<String>> = Vec::new();
    if let Ok(p) = std::env::var("NEURON_PYTHON") {
        if !p.trim().is_empty() {
            candidates.push(vec![p]);
        }
    }
    candidates.push(vec!["python".into()]);
    candidates.push(vec!["py".into(), "-3".into()]);
    for c in candidates {
        let ok = Command::new(&c[0])
            .args(&c[1..])
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if ok {
            return Some(c);
        }
    }
    None
}

fn python_or_skip(test: &str) -> Option<Vec<String>> {
    match find_python() {
        Some(p) => Some(p),
        None => {
            assert!(
                std::env::var("NEURON_REQUIRE_PYTHON").as_deref() != Ok("1"),
                "NEURON_REQUIRE_PYTHON=1 but no Python was found"
            );
            println!("{test}: skipped, no Python found (set NEURON_PYTHON)");
            None
        }
    }
}

fn nir_of(src: &str) -> Nir {
    let a = analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: src.replace("\r\n", "\n"),
        }],
        None,
        &SemaOptions::default(),
    );
    match a.nir.clone() {
        Some(n) => n,
        None => panic!("analysis errors:\n{}", a.render()),
    }
}

fn emit(nir: &Nir) -> Result<Package, Vec<neuron_emit::Diag>> {
    emitter("python")
        .unwrap()
        .emit(nir, &EmitOptions::default())
}

fn write(pkg: &Package, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("neuron_c4_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    pkg.write(&dir).unwrap();
    dir
}

fn run_py(py: &[String], dir: &PathBuf, file: &str, args: &[&str]) -> (bool, String, String) {
    let o = Command::new(&py[0])
        .args(&py[1..])
        .arg(file)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&o.stderr).replace("\r\n", "\n"),
    )
}

fn interp_trace(nir: &Nir, end_ms: i64) -> String {
    let mut it = Interp::new(
        nir.clone(),
        Options {
            end_ns: end_ms * 1_000_000,
            ..Options::default()
        },
    )
    .unwrap();
    it.run();
    render(it.trace())
}

fn interp_tests(nir: &Nir) -> Vec<String> {
    run_tests(nir, &Options::default())
        .into_iter()
        .map(|r| {
            if r.passed {
                format!("PASS {}", r.name)
            } else {
                format!("FAIL {}: {}", r.name, r.message)
            }
        })
        .collect()
}

struct Case {
    name: &'static str,
    src: &'static str,
    end_ms: i64,
    /// Lines of the interpreter's trace that must exist (a case that compares nothing
    /// would prove nothing).
    min_lines: usize,
}

fn cases() -> Vec<Case> {
    let c = |name, src, end_ms, min_lines| Case {
        name,
        src,
        end_ms,
        min_lines,
    };
    vec![
        c("h01_gateway_counter", H01, 5000, 4),
        c("h02_moving_average", H02, 3000, 0),
        c("h03_retry_loop", H03, 3000, 0),
        c("h04_recursion_free_sum", H04, 3000, 0),
        c("h05_fault_and_reset", H05, 5000, 1),
        c("x1_signals", X1, 2000, 10),
        c("x2_faults", X2, 400, 6),
        c("x3_events", X3, 1000, 10),
        c("x4_logic", X4, 600, 6),
    ]
}

#[test]
fn trace_and_tests_equal_the_interpreter() {
    let Some(py) = python_or_skip("trace_and_tests_equal_the_interpreter") else {
        return;
    };
    let mut summary = Vec::new();
    for c in cases() {
        let nir = nir_of(c.src);
        let pkg = emit(&nir).unwrap_or_else(|d| panic!("{}: {d:?}", c.name));
        let dir = write(&pkg, c.name);
        let file = format!("{}.py", c.name);

        // The trace.
        let want = interp_trace(&nir, c.end_ms);
        let arg = format!("{}ms", c.end_ms);
        let (ok, got, err) = run_py(&py, &dir, &file, &["--trace", &arg]);
        assert!(ok, "{}: python failed:\n{err}", c.name);
        assert_eq!(got, want, "{}: the trace differs\n{err}", c.name);
        let lines = want.lines().count();
        assert!(
            lines >= c.min_lines,
            "{}: only {lines} trace lines, wanted {}",
            c.name,
            c.min_lines
        );

        // The tests.
        let want_tests = interp_tests(&nir);
        let (ok, got, err) = run_py(&py, &dir, &file, &["--test"]);
        let got_lines: Vec<String> = got
            .lines()
            .filter(|l| l.starts_with("PASS ") || l.starts_with("FAIL "))
            .map(str::to_string)
            .collect();
        assert_eq!(got_lines, want_tests, "{}: the tests differ\n{err}", c.name);
        let all_pass = want_tests.iter().all(|l| l.starts_with("PASS "));
        assert_eq!(ok, all_pass, "{}: exit status of --test", c.name);
        summary.push(format!(
            "{}: {lines} trace lines equal, {} tests agree",
            c.name,
            want_tests.len()
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
    println!("{}", summary.join("\n"));
}

#[test]
fn x4_reports_a_failure_and_a_fault_like_the_interpreter() {
    // The interpreter itself must report what the Python test expects to see.
    let want = interp_tests(&nir_of(X4));
    assert_eq!(want.len(), 4, "{want:?}");
    assert!(want[0].starts_with("PASS "), "{want:?}");
    assert!(want[1].starts_with("PASS "), "{want:?}");
    assert_eq!(
        want[2],
        "FAIL this one fails on purpose: assertion failed: boom"
    );
    assert_eq!(
        want[3],
        "FAIL overflow is a fault: the program raised fault Overflow"
    );
}

#[test]
fn realtime_mode_runs_and_ros2_asks_for_rclpy() {
    let Some(py) = python_or_skip("realtime_mode_runs_and_ros2_asks_for_rclpy") else {
        return;
    };
    let nir = nir_of(X3);
    let dir = write(&emit(&nir).unwrap(), "x3_rt");
    let (ok, out, err) = run_py(&py, &dir, "x3_events.py", &["--realtime", "250ms"]);
    assert!(ok, "{err}");
    assert!(out.contains("EVENT  ping"), "{out}");
    // `--ros2` runs the wrapper, which needs rclpy (not installed where the tests run;
    // `c4b_python.rs` runs it against a stub).
    let (ok, _, err) = run_py(&py, &dir, "x3_events.py", &["--ros2"]);
    assert!(!ok);
    assert!(err.contains("rclpy was not found"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn output_is_deterministic_and_complete() {
    let nir = nir_of(X4);
    let (a, b) = (emit(&nir).unwrap(), emit(&nir).unwrap());
    assert_eq!(a.all_files(), b.all_files());
    let files = a.all_files();
    for f in [
        "x4_logic.py",
        "neuron_rt.py",
        "fidelity.json",
        "wiring.md",
        "x4_logic.py.map.json",
    ] {
        assert!(files.contains_key(f), "missing {f}: {:?}", files.keys());
    }
    assert!(files["wiring.md"].contains("--ros2"));
    assert!(files["x4_logic.py"].is_ascii());
    // Standard library only.
    for f in ["x4_logic.py", "neuron_rt.py"] {
        for line in files[f].lines() {
            if let Some(m) = line.strip_prefix("import ") {
                let m = m.split(" as ").next().unwrap_or(m);
                assert!(
                    ["sys", "heapq", "math", "struct", "time", "neuron_rt"].contains(&m),
                    "{f}: {line}"
                );
            }
        }
    }
    assert_eq!(a.fidelity.summary.unsupported, 0);
}

#[test]
fn what_cannot_be_translated_is_ne0610_with_its_place() {
    let nir = nir_of(BAD);
    let diags = emit(&nir).unwrap_err();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, "NE0610");
    assert!(
        diags[0].message.contains("interface"),
        "{}",
        diags[0].message
    );
    let at = diags[0].span[0];
    assert!(
        BAD[at..].starts_with("interface Shape"),
        "span {:?}",
        diags[0].span
    );
    // A package without a host module has nothing to translate.
    let nir = nir_of(include_str!(
        "../../../examples/corpus/robot/r01_home_and_wave.neuron"
    ));
    let d = emit(&nir).unwrap_err();
    assert_eq!(d[0].code, "NE0610");
}
