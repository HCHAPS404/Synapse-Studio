//! Task C4b: more of the language in the Python emitter, checked against the interpreter.
//!
//! Every program written here is emitted, run with Python in simulated time and compared
//! with the reference interpreter: `--trace` must print exactly the interpreter's trace
//! and `--test` the same verdict and message for every `test` block. Programs are grouped
//! by construct family (blocks, state machines and sequences, data types, contracts).
//!
//! Python is found like in `c4_python.rs` (`NEURON_PYTHON`, `python`, `py -3`); without
//! one the tests print a note and pass unless `NEURON_REQUIRE_PYTHON=1`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_emit::{emitter, EmitOptions, Package};
use neuron_interp::{render, run_tests, Interp, Options};
use neuron_nir::Nir;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use std::path::PathBuf;
use std::process::Command;

// ---- family 1: blocks ------------------------------------------------------------

const Y1: &str = "module y1_timers for host

signal delayed: bool
signal held: bool
signal pulse: bool
signal rises: bool
signal falls: bool
signal total: u16

let on_delay = ton(preset: 300 ms)
let off_delay = tof(preset: 200 ms)
let one_shot = tp(preset: 150 ms)
let counter = ctu(preset: 3)
let edge = r_trig()
let fall = f_trig()

var raw: bool = false
var ticks: u32 = 0

task driver every 50 ms:
  ticks += 1
  raw = (ticks / 6) % 2 == 1

task user every 20 ms:
  on_delay.input = raw
  off_delay.input = raw
  one_shot.input = raw
  edge.input = raw
  fall.input = raw
  counter.input = raw
  delayed = on_delay.q
  held = off_delay.q
  pulse = one_shot.q
  rises = edge.q
  falls = fall.q
  total = counter.cv

test \"the on delay is false at the start\":
  assert not delayed, \"idle\"

test \"the counter counts the rising edges\":
  advance(2 s)
  assert total >= 1, \"counted\"

test \"this one fails on purpose\":
  advance(1 s)
  assert total == 99, \"never\"
";

const Y2: &str = "module y2_methods for host

signal latched: bool
signal released: bool
signal ups: bool
signal downs: bool
signal u: f64
signal u2: f64
signal count: i32

let sr1 = sr()
let rs1 = rs()
let both = ctud(preset: 2)
let down = ctd(preset: 3)
let ctl = pid(kp: 2.0, ki: 0.5, kd: 0.1, out: -5.0..5.0, antiwindup: true)
let ctl2 = pid(kp: 1.0, ki: 2.0, out: 0.0..4.0, antiwindup: false)

var n: u32 = 0
var meas: f64 = 0.0
var meas2: f64 = 0.0

task counts every 100 ms:
  n += 1
  latched = sr1.call(n % 5 == 3, n % 5 == 4)
  released = rs1.call(n % 4 == 1, n % 4 == 2)
  both.input = n % 2 == 0
  both.down = n % 3 == 0
  ups = both.qu
  downs = both.qd
  down.load = n == 1
  down.input = n % 2 == 1
  count = down.cv

task control every 10 ms:
  let err = 3.0 - meas
  u = ctl.call(err)
  meas += u * 0.01
  let err2 = 3.0 - meas2
  u2 = ctl2.call(err2)
  meas2 += u2 * 0.01

test \"the latch follows its inputs\":
  advance(500 ms)
  assert latched or not latched, \"always\"

test \"the controller output stays in range\":
  advance(1 s)
  assert u <= 5.0 and u >= -5.0, \"range\"
  assert u2 <= 4.0, \"range 2\"

test \"this one fails on purpose\":
  advance(300 ms)
  assert u2 > 100.0, \"never\"
";

// ---- family 2: state machines and sequences -------------------------------------

const Y3: &str = "module y3_fsm for host

signal mode: u8
signal blink: bool
signal seen: u8

var presses: u8 = 0
var lamp: bool = false
var ticks: u32 = 0

task presser every 400 ms:
  presses += 1
  emit pressed

fsm Lamp every 10 ms:
  initial state Off:
    entry:
      lamp = false
      blink = false
    -> On when presses > 0
  state On:
    entry:
      lamp = true
      ticks = 0
    do:
      ticks += 1
    exit:
      blink = true
    -> Blinking after 500 ms
  state Blinking:
    entry:
      mode = 2
    -> Off when presses > 4

fsm Counter on pressed:
  initial state Idle:
    -> Busy when presses > 2
  state Busy:
    entry:
      seen = presses
    -> Idle when presses > 5

test \"the lamp machine starts off\":
  advance(20 ms)
  assert state_of(\"Lamp\") == \"Off\", \"starts off\"

test \"it reaches blinking and goes off again\":
  advance(1500 ms)
  assert state_of(\"Lamp\") == \"Blinking\", \"blinking\"
  advance(3 s)
  assert state_of(\"Lamp\") == \"Off\", \"off again\"

test \"this one fails on purpose\":
  advance(600 ms)
  assert state_of(\"Lamp\") == \"Nowhere\", \"never\"
";

const Y4: &str = "module y4_sequence for host

signal phase: u8
signal finished: u16
signal note: u8

var go: bool = false
var level: u32 = 0
var cycles: u32 = 0
var never: bool = false

task env every 100 ms:
  level += 10
  go = level > 30

sequence Cycle:
  initial step Idle:
    phase = 0
    until go
  step Fill:
    phase = 1
    until level > 80 timeout 2 s -> Broken
    cycles += 1
  step Stir:
    phase = 2
    wait 300 ms
    for i in 0..4:
      wait 50 ms
      if i == 1:
        continue
      cycles += i as u32
    var tries: u32 = 0
    while tries < 3 max 10:
      wait 20 ms
      tries += 1
  step Drain:
    phase = 3
    level = 0
    go = false
    finished = finished + 1
    -> Idle
  step Broken:
    phase = 99

sequence Watch:
  step Wait:
    until never timeout 500 ms -> Retry
  step Retry:
    note = 1
    loop max 3:
      wait 70 ms
      note += 1
      if note > 3:
        break
    until never timeout 250 ms
    note = 9

test \"the cycle reaches the drain step\":
  advance(2 s)
  assert finished >= 1, \"finished\"

test \"this one fails on purpose\":
  advance(100 ms)
  assert phase == 42, \"never\"
";

// ---- family 3: data types --------------------------------------------------------

const Y5: &str = "module y5_types for host

struct Point:
  x: i32
  y: i32 = 5

enum Mode:
  Idle
  Run(u8)
  Stop

signal out: i32
signal code: u8
signal found: u16

var p: Point = Point(x: 1, y: 2)
var mode: Mode = Mode.Idle
var n: u8 = 0

fn limit(m: Mode) -> u8:
  match m:
    Mode.Idle => return 1
    Mode.Run(s) if s > 3 => return s
    Mode.Run(s) => return 3
    Mode.Stop => return 0

fn first_even(a: u8, b: u8) -> u8?:
  if a % 2 == 0:
    return a
  if b % 2 == 0:
    return b
  return none

fn checked(v: u8) -> Result<u8, u8>:
  if v == 0:
    return Err(7)
  return Ok(v)

fn chain(v: u8) -> Result<u8, u8>:
  let a = checked(v)?
  return Ok(a + 1)

task t every 100 ms:
  n += 1
  p.x += 2
  out = p.x + p.y
  if n % 3 == 0:
    mode = Mode.Stop
  elif n % 2 == 0:
    mode = Mode.Run(n)
  else:
    mode = Mode.Idle
  code = limit(mode)
  match chain(n % 4):
    Ok(v) => found = v as u16
    Err(e) => found = 100 + e as u16
  match first_even(n * 2 + 1, 3):
    none => out = out + 1000
    _ => pass

test \"limits follow the mode\":
  assert limit(Mode.Idle) == 1, \"idle\"
  assert limit(Mode.Run(9)) == 9, \"fast\"
  assert limit(Mode.Run(2)) == 3, \"slow\"
  assert limit(Mode.Stop) == 0, \"stop\"

test \"results chain and propagate\":
  assert chain(3) == Ok(4), \"ok\"
  assert chain(0) == Err(7), \"err\"
  assert first_even(3, 4) == 4, \"second\"
  assert first_even(1, 3) == none, \"none\"

test \"structs copy by value\":
  var a = Point(x: 1)
  var b = a
  b.x = 9
  assert a.x == 1, \"copy\"
  assert a.y == 5, \"default\"
  assert b.x == 9, \"changed\"

test \"this one fails on purpose\":
  assert chain(1) == Err(1), \"never\"
";

// ---- family 4: contracts ---------------------------------------------------------

const Y6: &str = "module y6_contracts for host

signal out: u16

var total: u16 = 0

fn add(a: u16, b: u16) -> u16:
  requires a < 1000
  ensures result == a + b
  return a + b

fn bump(x: u16) -> u16:
  ensures result == old(total) + x
  total += x
  return total

fn broken(x: u16) -> u16:
  ensures result > x
  return x

fn chain(v: u8) -> Result<u8, u8>:
  ensures result == Ok(v + 1)
  if v == 0:
    return Err(7)
  return Ok(v + 1)

task t every 100 ms:
  out = add(total, 3)
  bump(2)

test \"contracts hold\":
  assert add(1, 2) == 3, \"add\"
  assert bump(5) == 5, \"bump\"
  assert chain(2) == Ok(3), \"chain\"

test \"a requires violation is a fault\":
  var r: u16 = add(2000, 1)

test \"an ensures violation is a fault\":
  var r: u16 = broken(4)

test \"an ensures violation after an early return\":
  var r: Result<u8, u8> = chain(0)

test \"this one fails on purpose\":
  assert add(1, 1) == 3, \"never\"
";

// ---- ROS 2 wrapper ---------------------------------------------------------------

const Z1: &str = "module z1_ros for host

signal level: u16
signal alarm: bool
signal temp: f64

var n: u16 = 0

task t every 10 ms:
  n += 1
  level = n
  temp = n as f64 * 0.5
  alarm = n % 7 > 4
";

const Z2: &str = "module z2_ros_in for host

signal pressed: u8

var count: u8 = 0

task on_press on button_pressed:
  count += 1
  pressed = count
";

/// A minimal `rclpy` and `std_msgs`: records what the wrapper creates and prints one line
/// per event; `spin` fires the timers `STUB_TICKS` times and injects the subscription
/// messages listed in `STUB_INJECT` (`tick|topic|value;...`) before the matching tick.
const STUB_RCLPY_INIT: &str = r#"import os
import sys

import neuron_rt

TICK = [0]
NODES = []


def init(args=None):
    print("INIT")


def shutdown():
    print("SHUTDOWN")


def spin(node):
    ticks = int(os.environ["STUB_TICKS"])
    inject = [x.split("|") for x in os.environ.get("STUB_INJECT", "").split(";") if x]
    for k in range(1, ticks + 1):
        TICK[0] = k
        for tick, topic, value in inject:
            if int(tick) == k:
                for sub in node.subscriptions:
                    if sub[0] == topic:
                        msg = sub[2]()
                        msg.data = (value == "true") if sub[2].__name__ == "Bool" else float(value)
                        print("INJECT %d %s %s" % (k, topic, value))
                        sub[1](msg)
        for period, cb in node.timers:
            cb()
    sys.stdout.flush()
"#;

const STUB_RCLPY_NODE: &str = r#"import rclpy
import neuron_rt


class _Pub:
    def __init__(self, topic):
        self.topic = topic

    def publish(self, msg):
        print("PUB %d %s %s" % (rclpy.TICK[0], self.topic, neuron_rt.show(msg.data)))


class Node:
    def __init__(self, name):
        self.name = name
        self.subscriptions = []
        self.timers = []
        print("NODE %s" % name)

    def create_publisher(self, msg_type, topic, qos):
        print("PUBLISHER %s %s" % (topic, msg_type.__name__))
        return _Pub(topic)

    def create_subscription(self, msg_type, topic, cb, qos):
        print("SUBSCRIPTION %s %s" % (topic, msg_type.__name__))
        self.subscriptions.append((topic, cb, msg_type))

    def create_timer(self, period, cb):
        print("TIMER %s" % period)
        self.timers.append((period, cb))

    def destroy_node(self):
        pass
"#;

const STUB_STD_MSGS: &str = r#"class Bool:
    def __init__(self):
        self.data = False


class Int64:
    def __init__(self):
        self.data = 0


class Float64:
    def __init__(self):
        self.data = 0.0


class String:
    def __init__(self):
        self.data = ""
"#;

/// Writes the stub packages into `dir/stub` and returns that path.
fn write_stub(dir: &std::path::Path) -> PathBuf {
    let stub = dir.join("stub");
    for (rel, text) in [
        ("rclpy/__init__.py", STUB_RCLPY_INIT),
        ("rclpy/node.py", STUB_RCLPY_NODE),
        ("std_msgs/__init__.py", ""),
        ("std_msgs/msg/__init__.py", STUB_STD_MSGS),
    ] {
        let p = stub.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    stub
}

/// Runs `file --ros2` with the stub on `PYTHONPATH`; returns the stdout lines.
fn run_ros2(
    py: &[String],
    dir: &PathBuf,
    stub: &std::path::Path,
    file: &str,
    ticks: u32,
    inject: &str,
) -> (bool, Vec<String>, String) {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let pypath = format!("{}{sep}{}", stub.display(), dir.display());
    let o = Command::new(&py[0])
        .args(&py[1..])
        .arg(file)
        .arg("--ros2")
        .current_dir(dir)
        .env("PYTHONPATH", pypath)
        .env("STUB_TICKS", ticks.to_string())
        .env("STUB_INJECT", inject)
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n");
    (
        o.status.success(),
        out.lines().map(str::to_string).collect(),
        String::from_utf8_lossy(&o.stderr).replace("\r\n", "\n"),
    )
}

#[test]
fn ros2_wrapper_publishes_the_interpreter_signal_changes() {
    let Some(py) = python_or_skip("ros2_wrapper_publishes_the_interpreter_signal_changes") else {
        return;
    };
    let nir = nir_of(Z1);
    let pkg = emit(&nir).unwrap_or_else(|d| panic!("{d:?}"));
    assert!(pkg.all_files().contains_key("z1_ros_ros2.py"));
    let dir = write(&pkg, "z1_ros");
    let stub = write_stub(&dir);
    const TICKS: u32 = 400;
    let (ok, lines, err) = run_ros2(&py, &dir, &stub, "z1_ros.py", TICKS, "");
    assert!(ok, "the wrapper failed:\n{err}\n{lines:?}");

    // What the wrapper created.
    for want in [
        "NODE z1_ros",
        "PUBLISHER neuron/z1_ros/level Int64",
        "PUBLISHER neuron/z1_ros/alarm Bool",
        "PUBLISHER neuron/z1_ros/temp Float64",
        "TIMER 0.001",
        "INIT",
        "SHUTDOWN",
    ] {
        assert!(
            lines.iter().any(|l| l == want),
            "missing `{want}` in {lines:?}"
        );
    }

    // The messages equal the interpreter's signal changes, in order, within the same tick.
    let trace = interp_trace(&nir, i64::from(TICKS));
    let want: Vec<String> = trace
        .lines()
        .filter(|l| l.contains("  SIG    "))
        .map(|l| {
            let (t, rest) = l.split_once(" ms").unwrap();
            let t: f64 = t.trim().parse().unwrap();
            let tick = (t.ceil() as u32).max(1);
            let (name, value) = rest
                .split_once("SIG    ")
                .unwrap()
                .1
                .split_once(" = ")
                .unwrap();
            format!("PUB {tick} neuron/z1_ros/{name} {value}")
        })
        .collect();
    let got: Vec<String> = lines
        .iter()
        .filter(|l| l.starts_with("PUB "))
        .cloned()
        .collect();
    assert!(want.len() >= 60, "only {} signal changes", want.len());
    assert_eq!(got, want);

    let _ = std::fs::remove_dir_all(&dir);

    // A message on a subscription reaches the program: `true` on the event topic raises
    // `button_pressed`, which releases the task and publishes `pressed` in the same tick;
    // `false` raises nothing.
    let nir = nir_of(Z2);
    let dir = write(&emit(&nir).unwrap(), "z2_ros_in");
    let stub = write_stub(&dir);
    let (ok, lines, err) = run_ros2(
        &py,
        &dir,
        &stub,
        "z2_ros_in.py",
        100,
        "30|neuron/z2_ros_in/event/button_pressed|true;40|neuron/z2_ros_in/event/button_pressed|false;60|neuron/z2_ros_in/event/button_pressed|true",
    );
    assert!(ok, "{err}\n{lines:?}");
    let pubs: Vec<&str> = lines
        .iter()
        .filter(|l| l.starts_with("PUB "))
        .map(String::as_str)
        .collect();
    assert_eq!(
        pubs,
        [
            "PUB 30 neuron/z2_ros_in/pressed 1",
            "PUB 60 neuron/z2_ros_in/pressed 2"
        ]
    );
    for want in [
        "SUBSCRIPTION neuron/z2_ros_in/event/button_pressed Bool",
        "INJECT 30 neuron/z2_ros_in/event/button_pressed true",
    ] {
        assert!(
            lines.iter().any(|l| l == want),
            "missing `{want}` in {lines:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- harness ---------------------------------------------------------------------

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
    let dir = std::env::temp_dir().join(format!("neuron_c4b_{}_{name}", std::process::id()));
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

/// A program, how long to run it and what the interpreter's trace must at least contain.
struct Case {
    name: &'static str,
    src: &'static str,
    end_ms: i64,
    min_lines: usize,
    /// Trace lines (substrings) that must appear, so the case proves the construct ran.
    must_contain: &'static [&'static str],
}

/// Emits `c`, runs it and compares the trace and the test verdicts with the interpreter.
/// Returns the summary line.
fn compare(py: &[String], c: &Case) -> String {
    let nir = nir_of(c.src);
    let pkg = emit(&nir).unwrap_or_else(|d| panic!("{}: {d:?}", c.name));
    let dir = write(&pkg, c.name);
    let file = format!("{}.py", c.name);

    let want = interp_trace(&nir, c.end_ms);
    let arg = format!("{}ms", c.end_ms);
    let (ok, got, err) = run_py(py, &dir, &file, &["--trace", &arg]);
    assert!(ok, "{}: python failed:\n{err}", c.name);
    assert_eq!(got, want, "{}: the trace differs\n{err}", c.name);
    let lines = want.lines().count();
    assert!(
        lines >= c.min_lines,
        "{}: only {lines} trace lines, wanted {}",
        c.name,
        c.min_lines
    );
    for m in c.must_contain {
        assert!(
            want.contains(m),
            "{}: the trace has no `{m}`:\n{want}",
            c.name
        );
    }

    let want_tests = interp_tests(&nir);
    let (ok, got, err) = run_py(py, &dir, &file, &["--test"]);
    let got_lines: Vec<String> = got
        .lines()
        .filter(|l| l.starts_with("PASS ") || l.starts_with("FAIL "))
        .map(str::to_string)
        .collect();
    assert_eq!(got_lines, want_tests, "{}: the tests differ\n{err}", c.name);
    let all_pass = want_tests.iter().all(|l| l.starts_with("PASS "));
    assert_eq!(ok, all_pass, "{}: exit status of --test", c.name);
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}: {lines} trace lines equal, {} tests agree",
        c.name,
        want_tests.len()
    )
}

fn run_cases(test: &str, cases: &[Case]) {
    let Some(py) = python_or_skip(test) else {
        return;
    };
    let summary: Vec<String> = cases.iter().map(|c| compare(&py, c)).collect();
    println!("{}", summary.join("\n"));
}

#[test]
fn blocks_timers_counters_and_edges_equal_the_interpreter() {
    run_cases(
        "blocks_timers_counters_and_edges_equal_the_interpreter",
        &[Case {
            name: "y1_timers",
            src: Y1,
            end_ms: 3000,
            min_lines: 20,
            must_contain: &[
                "SIG    delayed = true",
                "SIG    held = true",
                "SIG    total = ",
            ],
        }],
    );
}

#[test]
fn state_machines_equal_the_interpreter() {
    run_cases(
        "state_machines_equal_the_interpreter",
        &[Case {
            name: "y3_fsm",
            src: Y3,
            end_ms: 6000,
            min_lines: 15,
            must_contain: &[
                "STATE  Lamp = On",
                "STATE  Lamp = Blinking",
                "STATE  Counter = Busy",
            ],
        }],
    );
}

#[test]
fn sequences_equal_the_interpreter() {
    run_cases(
        "sequences_equal_the_interpreter",
        &[Case {
            name: "y4_sequence",
            src: Y4,
            end_ms: 4000,
            min_lines: 15,
            must_contain: &["STATE  Cycle = Drain", "EVENT  Watch = timeout"],
        }],
    );
}

#[test]
fn structs_enums_match_option_and_result_equal_the_interpreter() {
    run_cases(
        "structs_enums_match_option_and_result_equal_the_interpreter",
        &[Case {
            name: "y5_types",
            src: Y5,
            end_ms: 2000,
            min_lines: 20,
            must_contain: &["SIG    out = ", "SIG    code = ", "SIG    found = "],
        }],
    );
}

#[test]
fn contracts_equal_the_interpreter() {
    run_cases(
        "contracts_equal_the_interpreter",
        &[Case {
            name: "y6_contracts",
            src: Y6,
            end_ms: 1500,
            min_lines: 10,
            must_contain: &["SIG    out = "],
        }],
    );
}

#[test]
fn blocks_latches_and_pid_equal_the_interpreter() {
    run_cases(
        "blocks_latches_and_pid_equal_the_interpreter",
        &[Case {
            name: "y2_methods",
            src: Y2,
            end_ms: 2000,
            min_lines: 20,
            must_contain: &["SIG    latched = true", "SIG    u = "],
        }],
    );
}
