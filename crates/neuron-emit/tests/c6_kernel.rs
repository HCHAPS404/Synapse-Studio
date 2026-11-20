//! Task C6: state tables and the micro-kernel, checked by running them.
//!
//! Every `fsm` and `sequence` of the corpus that the kernel can express (and two
//! synthetic programs that use inputs, variables, timeouts and `do` blocks) is compiled
//! to a table. The reference executor of the table is compared with the reference
//! interpreter under random inputs; the ST kernel in the virtual PLC, the C99 kernel built
//! with the host compiler and the SPEL+ kernel in the virtual controller are compared with
//! the reference executor, scan by scan. Random tables check that the kernels do not
//! depend on the table.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_emit::kernel::{compile_nir, from_hex, to_hex, Insn, Machine, StateCode, Table, Trans};
use neuron_emit::{emitter, EmitOptions, Package};
use neuron_interp::{Action, Interp, Options, Scenario, Step, TraceKind, Value as IV};
use neuron_nir::Nir;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_st::ast::IntTy;
use neuron_st::{parse as st_parse, Clock, Plc, Value as SV};
use proptest::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;

const END: i64 = 12_000_000_000;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel))
        .unwrap()
        .replace("\r\n", "\n")
}

fn nir_of(name: &str, text: &str) -> Nir {
    let a = analyze(
        vec![SourceFile {
            path: name.into(),
            text: text.into(),
        }],
        None,
        &SemaOptions::default(),
    );
    a.nir
        .clone()
        .unwrap_or_else(|| panic!("{name}:\n{}", a.render()))
}

fn report(name: &str, lines: &[String]) {
    let dir = root().join("out");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("c6-{name}.txt")), lines.join("\n") + "\n");
}

// ---- the programs ---------------------------------------------------------------

const DOOR: &str =
    "## C6 test: a door with inputs, a counter, a do block and every kind of transition.
module c6_door for mcu on arduino_uno_r3

io button: gpio.input @ D2
io sensor: gpio.input @ D3
io lamp: gpio.output @ D13
io buzzer: gpio.output @ D12

var presses: i16 = 0

fsm Door every 10 ms:
  initial state Idle:
    entry:
      lamp.write(false)
    -> Armed when button.read()
  state Armed:
    entry:
      lamp.write(true)
      presses += 1
    -> Idle when not sensor.read() and presses > 2
    -> Alarm when sensor.read() or after 300 ms
  state Alarm:
    entry:
      buzzer.write(true)
    exit:
      buzzer.write(false)
    do:
      presses += 1
    -> Idle when button.read() or after 1 s

safety:
  safe_state:
    lamp.write(false)
    buzzer.write(false)
";

const PRESS: &str =
    "## C6 test: a press cycle with a wait, a timeout, a counter and an integer output.
module c6_press for plc
scan 10 ms

inputs:
  start: bool @ %I0.0
  done: bool  @ %I0.1
  abort: bool @ %I0.2

outputs:
  clamp: bool @ %Q0.0
  press: bool @ %Q0.1
  lamp: bool  @ %Q0.2
  count: i16  @ %QW0

memory:
  cycles: i16 @ %MW0

sequence Cycle:
  initial step Idle:
    lamp = false
    until start
  step Clamp:
    clamp = true
    wait 200 ms
  step Press:
    press = true
    until done timeout 1 s -> Fault
    press = false
    cycles += 1
  step Release:
    clamp = false
    count = cycles
    wait 100 ms
    -> Idle
  step Fault:
    lamp = true
    press = false
    clamp = false
    until abort
    -> Idle

safety:
  safe_state:
    clamp = false
    press = false
    lamp = true
    count = 0
";

/// An integer input compared in guards and summed, so that 16-bit arithmetic wraps. The
/// reference interpreter enters the safe state on an overflow, so this machine is checked
/// against the reference executor only (the kernel columns), not against the interpreter.
const WRAP: &str = "## C6 test: an integer input, guards and 16-bit wrap-around.
module c6_wrap for plc
scan 10 ms

inputs:
  sensor: i16 @ %IW0
  go: bool @ %I0.0

outputs:
  total: i16 @ %QW0
  scaled: i16 @ %QW1
  high: bool @ %Q0.0

memory:
  acc: i16 @ %MW0

sequence Wrap:
  initial step Idle:
    high = false
    until go
  step Sum:
    acc = acc + sensor
    total = acc
    scaled = sensor * 3
    high = acc > 20000
    wait 20 ms
  step Diff:
    total = acc - sensor * 2
    until not go
    -> Idle

safety:
  safe_state:
    total = 0
    scaled = 0
    high = false
";

/// Input values that make the sums and products above wrap.
const WIDE: [i16; 8] = [32767, -32768, 20000, -20000, 12000, -1, 0, 17000];

fn wrap_program() -> Prog {
    Prog {
        name: "c6_wrap",
        file: "c6_wrap.neuron",
        text: WRAP.to_string(),
        bound: 1,
    }
}

struct Prog {
    name: &'static str,
    file: &'static str,
    text: String,
    /// Allowed distance between the reference executor and the interpreter, in scans.
    bound: i64,
}

fn programs() -> Vec<Prog> {
    vec![
        Prog {
            name: "m06_traffic_light",
            file: "examples/corpus/mcu/m06_traffic_light.neuron",
            text: read("examples/corpus/mcu/m06_traffic_light.neuron"),
            bound: 0,
        },
        Prog {
            name: "p09_sfc_mixer",
            file: "examples/corpus/plc/p09_sfc_mixer.neuron",
            text: read("examples/corpus/plc/p09_sfc_mixer.neuron"),
            bound: 1,
        },
        Prog {
            name: "c6_door",
            file: "c6_door.neuron",
            text: DOOR.to_string(),
            bound: 0,
        },
        Prog {
            name: "c6_press",
            file: "c6_press.neuron",
            text: PRESS.to_string(),
            bound: 1,
        },
    ]
}

fn compile_one(p: &Prog) -> (Nir, neuron_emit::kernel::Compiled) {
    let nir = nir_of(p.file, &p.text);
    let (mut list, diags) = compile_nir(&nir);
    assert!(diags.is_empty(), "{}: {diags:?}", p.name);
    assert_eq!(list.len(), 1, "{}", p.name);
    (nir, list.remove(0))
}

// ---- random inputs ----------------------------------------------------------------

/// Random inputs, each changing 3 ms after a scan instant so the order is never ambiguous.
/// Seed 5 is a pulse pattern (the first input pulses often, the others seldom).
fn schedule(t: &Table, seed: u64, end: i64) -> Vec<(i64, usize, i16)> {
    let scan = t.scan_ns;
    let mut rng = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let mut next = || {
        rng = rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        rng >> 33
    };
    let n = t.inputs.len();
    let mut state = vec![false; n];
    let mut out = Vec::new();
    let mut at = 3_000_000;
    let pulses = seed == 5;
    while at < end {
        if pulses {
            for (k, p) in t.inputs.iter().enumerate() {
                let odds = if k == 0 { 3 } else { 60 };
                if p.boolean && next() % odds == 0 {
                    out.push((at, k, 1));
                    out.push((at + scan * 2, k, 0));
                }
            }
            at += scan * 4;
            continue;
        }
        for (k, p) in t.inputs.iter().enumerate() {
            if p.boolean {
                if next() % 9 == 0 {
                    state[k] = !state[k];
                    out.push((at, k, i16::from(state[k])));
                }
            } else if next() % 15 == 0 {
                out.push((at, k, 700 + (next() % 300) as i16));
            }
        }
        // Now and then the inputs stay still for about three seconds, so that timers fire.
        let pause = if next() % 25 == 0 {
            300
        } else {
            1 + (next() % 4) as i64
        };
        at += scan * pause;
    }
    out.sort_by_key(|e| e.0);
    out
}

// ---- the reference interpreter ---------------------------------------------------

type Obs = Vec<(i64, String, String)>;

struct Ref {
    outputs: Obs,
    states: Vec<(i64, String)>,
}

fn run_interp(nir: &Nir, t: &Table, sched: &[(i64, usize, i16)], end: i64) -> Ref {
    let mut it = Interp::new(
        nir.clone(),
        Options {
            end_ns: end,
            ..Options::default()
        },
    )
    .unwrap();
    let steps = sched
        .iter()
        .map(|(at, i, v)| {
            let p = &t.inputs[*i];
            Step {
                t_ns: *at,
                action: Action::Set(
                    p.name.clone(),
                    if p.boolean {
                        IV::Bool(*v != 0)
                    } else {
                        IV::Int(i128::from(*v))
                    },
                ),
            }
        })
        .collect();
    it.load_scenario(&Scenario { steps });
    it.run();
    let mut outputs: Obs = it
        .trace()
        .iter()
        .filter(|e| e.kind == TraceKind::Output)
        .map(|e| (e.t_ns, e.name.clone(), e.value.clone()))
        .collect();
    outputs.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let states = it
        .trace()
        .iter()
        .filter(|e| e.kind == TraceKind::State && e.name == t.name)
        .map(|e| (e.t_ns, e.value.clone()))
        .collect();
    Ref { outputs, states }
}

fn exec_obs(t: &Table, tr: &neuron_emit::kernel::Trace) -> Obs {
    let mut o: Obs = tr
        .changes
        .iter()
        .map(|c| {
            (
                c.scan as i64 * t.scan_ns,
                t.outputs[c.output].name.clone(),
                t.outputs[c.output].show(c.value),
            )
        })
        .collect();
    o.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    o
}

fn cut(o: Obs, end: i64) -> Obs {
    o.into_iter().filter(|(t, _, _)| *t < end).collect()
}

/// Same values in the same order per output, each change within `bound` ns of the
/// reference. Returns the largest distance found.
fn within(got: &Obs, want: &Obs, bound: i64) -> Result<i64, String> {
    let names: std::collections::BTreeSet<&String> = want.iter().map(|(_, n, _)| n).collect();
    let mut worst = 0;
    for n in names {
        let a: Vec<_> = got.iter().filter(|x| &x.1 == n).collect();
        let b: Vec<_> = want.iter().filter(|x| &x.1 == n).collect();
        if a.len().abs_diff(b.len()) > 1 {
            return Err(format!(
                "`{n}`: {} changes in the kernel, {} in the interpreter",
                a.len(),
                b.len()
            ));
        }
        for i in 0..a.len().min(b.len()) {
            if a[i].2 != b[i].2 {
                return Err(format!("`{n}` change {i}: {} vs {}", a[i].2, b[i].2));
            }
            let d = (a[i].0 - b[i].0).abs();
            worst = worst.max(d);
            if d > bound {
                return Err(format!(
                    "`{n}` change {i}: at {} ns vs {} ns (bound {bound} ns)",
                    a[i].0, b[i].0
                ));
            }
        }
    }
    Ok(worst)
}

// ---- (a) the reference executor against the interpreter -------------------------

#[test]
fn the_reference_executor_matches_the_interpreter() {
    let mut lines = Vec::new();
    for p in programs() {
        let (nir, c) = compile_one(&p);
        let t = &c.table;
        let scan = t.scan_ns;
        let words = t.words().unwrap();
        let bound = p.bound * scan;
        let mut compared = 0;
        let mut worst = 0;
        let mut state_changes = 0;
        for seed in 1..=5 {
            let sched = schedule(t, seed, END);
            let want = run_interp(&nir, t, &sched, END);
            let tr = t.machine.run_schedule(scan, &sched, END);
            let got = cut(exec_obs(t, &tr), END - 2 * scan);
            let wanted = cut(want.outputs.clone(), END - 2 * scan);
            if bound == 0 {
                assert_eq!(got, wanted, "{} seed {seed}", p.name);
                let st: Vec<(i64, String)> = tr
                    .states
                    .iter()
                    .map(|(k, s)| (*k as i64 * scan, t.state_names[*s].clone()))
                    .filter(|e| e.0 < END - 2 * scan)
                    .collect();
                let ws: Vec<(i64, String)> = want
                    .states
                    .iter()
                    .filter(|e| e.0 < END - 2 * scan)
                    .cloned()
                    .collect();
                assert_eq!(st, ws, "{} seed {seed}: states", p.name);
                state_changes += st.len();
            } else {
                match within(&got, &wanted, bound) {
                    Ok(w) => worst = worst.max(w),
                    Err(e) => panic!("{} seed {seed}: {e}", p.name),
                }
            }
            compared += wanted.len();
        }
        assert!(
            compared >= 20,
            "{}: only {compared} output changes compared",
            p.name
        );
        lines.push(format!(
            "{:<20} {} table words, {} states, {} transitions; 5 random schedules of 12 s; {} output changes compared; {}; worst distance {} ms (bound {} ms)",
            p.name,
            words.len(),
            t.machine.states.len(),
            t.machine.transitions(),
            compared,
            if bound == 0 {
                format!("exact, {state_changes} state changes equal")
            } else {
                "within the bound".to_string()
            },
            worst as f64 / 1e6,
            bound as f64 / 1e6
        ));
    }
    report("a-executor-vs-interpreter", &lines);
}

// ---- encoding ---------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn rand_expr(r: &mut Rng, io: (usize, usize, usize), depth: u32, out: &mut Vec<Insn>) {
    let (nin, nout, nvar) = io;
    if depth == 0 || r.below(3) == 0 {
        match r.below(6) {
            1 if nin > 0 => out.push(Insn::Ldi(r.below(nin as u64) as u8)),
            2 if nvar > 0 => out.push(Insn::Ldv(r.below(nvar as u64) as u8)),
            3 if nout > 0 => out.push(Insn::Ldo(r.below(nout as u64) as u8)),
            4 => out.push(Insn::Timer),
            _ => out.push(Insn::Push(r.below(200) as i16 - 100)),
        }
        return;
    }
    if r.below(4) == 0 {
        rand_expr(r, io, depth - 1, out);
        out.push(if r.below(2) == 0 {
            Insn::Not
        } else {
            Insn::Neg
        });
        return;
    }
    rand_expr(r, io, depth - 1, out);
    rand_expr(r, io, depth - 1, out);
    out.push(
        [
            Insn::And,
            Insn::Or,
            Insn::Eq,
            Insn::Ne,
            Insn::Lt,
            Insn::Le,
            Insn::Gt,
            Insn::Ge,
            Insn::Add,
            Insn::Sub,
            Insn::Mul,
        ][r.below(11) as usize],
    );
}

fn rand_block(r: &mut Rng, io: (usize, usize, usize), out: &mut Vec<Insn>) {
    for _ in 0..r.below(3) {
        match r.below(5) {
            0 if io.2 > 0 => {
                rand_expr(r, io, 2, out);
                out.push(Insn::Stv(r.below(io.2 as u64) as u8));
            }
            1 => out.push(Insn::Tclr),
            _ if io.1 > 0 => {
                rand_expr(r, io, 2, out);
                out.push(Insn::Sto(r.below(io.1 as u64) as u8));
            }
            _ => {}
        }
    }
}

/// A random machine inside the limits of the kernel.
fn random_machine(seed: u64) -> Machine {
    (0..)
        .map(|k| random_machine_try(seed.wrapping_add(k * 1_000_003)))
        .find(|m| m.limits().is_empty())
        .unwrap()
}

fn random_machine_try(seed: u64) -> Machine {
    let mut r = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
    r.next();
    let io = (
        r.below(4) as usize,
        1 + r.below(4) as usize,
        r.below(4) as usize,
    );
    let ns = 1 + r.below(6) as usize;
    let mut states = Vec::new();
    for _ in 0..ns {
        let mut s = StateCode::default();
        rand_block(&mut r, io, &mut s.entry);
        rand_block(&mut r, io, &mut s.exit);
        rand_block(&mut r, io, &mut s.during);
        for _ in 0..r.below(4) {
            let guard = if r.below(4) == 0 {
                None
            } else {
                let mut g = Vec::new();
                rand_expr(&mut r, io, 3, &mut g);
                Some(g)
            };
            let mut action = Vec::new();
            rand_block(&mut r, io, &mut action);
            s.transitions.push(Trans {
                target: r.below(ns as u64) as usize,
                guard,
                action,
            });
        }
        states.push(s);
    }
    Machine {
        sequence: r.below(2) == 0,
        publish_initial: r.below(2) == 0,
        inputs: io.0,
        outputs: io.1,
        vars: (0..io.2).map(|_| r.below(50) as i16 - 25).collect(),
        initial: r.below(ns as u64) as usize,
        states,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// decode(encode(t)) == t, the hex text round-trips, and any single flipped bit of
    /// any word is detected.
    #[test]
    fn encoding_round_trips_and_detects_a_flipped_bit(seed in any::<u64>()) {
        let m = random_machine(seed);
        prop_assert_eq!(m.check(), Ok(()));
        let w = m.encode().unwrap();
        prop_assert_eq!(w.len(), m.total_words());
        prop_assert_eq!(&Machine::decode(&w).unwrap(), &m);
        prop_assert_eq!(from_hex(&to_hex(&w)).unwrap(), w.clone());
        for i in 0..w.len() {
            for bit in 0..16 {
                let mut x = w.clone();
                x[i] ^= 1 << bit;
                prop_assert!(Machine::decode(&x).is_err(), "word {} bit {} not detected", i, bit);
            }
        }
    }
}

#[test]
fn the_checksum_detects_a_flipped_word_of_a_real_table() {
    let p = &programs()[1];
    let (_, c) = compile_one(p);
    let w = c.table.words().unwrap();
    assert_eq!(Machine::decode(&w).unwrap(), c.table.machine);
    for i in 0..w.len() {
        let mut x = w.clone();
        x[i] ^= 0x0100;
        assert!(Machine::decode(&x).is_err(), "word {i}");
    }
    let mut x = w.clone();
    x.swap(14, 15);
    if x != w {
        assert!(Machine::decode(&x).is_err(), "two swapped words");
    }
}

// ---- limits and unsupported constructs ---------------------------------------------

fn diags_of(name: &str, text: &str) -> Vec<neuron_emit::Diag> {
    let nir = nir_of(name, text);
    compile_nir(&nir).1
}

#[test]
fn a_table_beyond_the_limits_is_ne1004() {
    // 70 states.
    let mut src = String::from("module c6_big for mcu on arduino_uno_r3\n\nio led: gpio.output @ D13\n\nfsm Big every 10 ms:\n");
    for i in 0..70 {
        let next = (i + 1) % 70;
        src.push_str(&format!(
            "  {}state S{i}:\n    entry:\n      led.write({})\n    -> S{next} after 1 s\n",
            if i == 0 { "initial " } else { "" },
            i % 2 == 0
        ));
    }
    src.push_str("\nsafety:\n  safe_state:\n    led.write(false)\n");
    let d = diags_of("c6_big.neuron", &src);
    assert!(d.iter().any(|d| d.code == "NE1004"), "{d:?}");
    let e = d.iter().find(|d| d.code == "NE1004").unwrap();
    assert!(e.message.contains("70 states"), "{}", e.message);
    assert!(e.span[1] <= src.len() && e.span[0] < e.span[1]);

    // A timeout of 400 s at 10 ms is 40000 scans, more than the timer holds.
    let long = "module c6_long for mcu on arduino_uno_r3\n\nio led: gpio.output @ D13\n\nfsm Slow every 10 ms:\n  initial state A:\n    entry:\n      led.write(true)\n    -> B after 400 s\n  state B:\n    entry:\n      led.write(false)\n    -> A after 1 s\n\nsafety:\n  safe_state:\n    led.write(false)\n";
    let d = diags_of("c6_long.neuron", long);
    assert!(
        d.iter()
            .any(|d| d.code == "NE1004" && d.message.contains("40000 scans")),
        "{d:?}"
    );

    // The machine type reports each limit.
    let mut m = random_machine(7);
    m.states = vec![StateCode::default(); 65];
    m.initial = 0;
    let l = m.limits();
    assert!(l.iter().any(|x| x.contains("65 states")), "{l:?}");
    assert!(m.encode().is_err());
    let mut m = random_machine(9);
    m.states = vec![
        StateCode {
            transitions: vec![
                Trans {
                    target: 0,
                    guard: None,
                    action: vec![]
                };
                129
            ],
            ..StateCode::default()
        };
        1
    ];
    m.initial = 0;
    assert!(m.limits().iter().any(|x| x.contains("129 transitions")));
    let mut m = random_machine(11);
    m.vars = vec![0; 33];
    assert!(m.limits().iter().any(|x| x.contains("33 variables")));
    let mut m = random_machine(12);
    m.states[0].entry = vec![Insn::Tclr; 300];
    assert!(m.limits().iter().any(|x| x.contains("program words")));
}

#[test]
fn what_the_kernel_cannot_express_is_ne0610_with_a_location() {
    for (file, why) in [
        ("examples/corpus/robot/r01_home_and_wave.neuron", "motion"),
        ("examples/corpus/cell/x03_cycle_sequence.neuron", "cell"),
    ] {
        let text = read(file);
        let d = diags_of(file, &text);
        assert!(!d.is_empty(), "{file}");
        for x in &d {
            assert_eq!(x.code, "NE0610", "{file}: {x:?}");
            assert_eq!(x.file, file);
            assert!(
                x.span[0] < x.span[1] && x.span[1] <= text.len(),
                "{why}: {x:?}"
            );
        }
    }
    // The emitter reports the same and writes nothing.
    let text = read("examples/corpus/robot/r01_home_and_wave.neuron");
    let nir = nir_of("r01.neuron", &text);
    let e = emitter("kernel").unwrap();
    assert!(e.emit(&nir, &EmitOptions::default()).is_err());
}

// ---- the emitter --------------------------------------------------------------------

#[test]
fn the_kernel_emitter_writes_tables_and_kernels_deterministically() {
    let mut lines = Vec::new();
    for p in programs() {
        let nir = nir_of(p.file, &p.text);
        let e = emitter("kernel").unwrap();
        let a = e.emit(&nir, &EmitOptions::default()).unwrap();
        let b = e.emit(&nir, &EmitOptions::default()).unwrap();
        assert_eq!(a.all_files(), b.all_files(), "{}", p.name);
        let files = a.all_files();
        let (_, c) = compile_one(&p);
        let base = &c.table.name;
        for f in [
            format!("{base}.nst.json"),
            format!("{base}.nst.hex"),
            "neuron_kernel.st".to_string(),
            "nrt_kernel.c".to_string(),
            "nrt_kernel.h".to_string(),
            "neuron_kernel.spel".to_string(),
            "fidelity.json".to_string(),
            "wiring.md".to_string(),
        ] {
            assert!(files.contains_key(&f), "{} lacks {f}", p.name);
        }
        let words = from_hex(&files[&format!("{base}.nst.hex")]).unwrap();
        assert_eq!(Machine::decode(&words).unwrap(), c.table.machine);
        let json: Table = serde_json::from_str(&files[&format!("{base}.nst.json")]).unwrap();
        assert_eq!(json, c.table);
        assert!(files["wiring.md"].contains("| Output |"));
        lines.push(format!(
            "{:<20} files: {}; fidelity worst class {}; {} notes",
            p.name,
            files.keys().cloned().collect::<Vec<_>>().join(", "),
            a.fidelity.worst(),
            a.fidelity.entries.len()
        ));
    }
    report("e-emitter", &lines);
}

#[test]
fn the_fidelity_report_states_the_bound() {
    let (_, c) = compile_one(&programs()[1]);
    let n = c
        .notes
        .iter()
        .find(|n| n.construct.contains("sequence"))
        .unwrap();
    assert_eq!(n.class, neuron_emit::Class::Equivalent);
    assert!(n.note.contains("within one scan"), "{}", n.note);
    // A state machine whose period is the scan has nothing to report about time.
    let (_, c) = compile_one(&programs()[0]);
    assert!(c.notes.iter().all(|n| !n.note.contains("evaluated every")));
}

// ---- (b) the ST kernel ---------------------------------------------------------------

fn st_harness(words: &[u16], nin: usize, nout: usize) -> String {
    let mut s = String::from("PROGRAM Main\nVAR_INPUT\n");
    for i in 0..nin {
        s.push_str(&format!("    in{i} : INT;\n"));
    }
    s.push_str("END_VAR\nVAR_OUTPUT\n");
    for i in 0..nout {
        s.push_str(&format!("    rep{i} : DINT;\n"));
    }
    s.push_str("END_VAR\nVAR\n    kern : NEURON_KERNEL;\n    tbl : ARRAY [0..1023] OF INT;\n    inp : ARRAY [0..31] OF INT;\n    first : BOOL := TRUE;\n");
    for i in 0..nout {
        s.push_str(&format!("    cnt{i} : DINT;\n"));
    }
    s.push_str("END_VAR\nIF first THEN\n    first := FALSE;\n");
    for (k, w) in words.iter().enumerate() {
        s.push_str(&format!("    tbl[{k}] := {};\n", *w as i16));
    }
    s.push_str("END_IF;\n");
    for i in 0..nin {
        s.push_str(&format!("inp[{i}] := in{i};\n"));
    }
    s.push_str(&format!(
        "kern(TBL := tbl, LEN := {}, INP := inp);\n",
        words.len()
    ));
    for i in 0..nout {
        s.push_str(&format!(
            "IF kern.CHG[{i}] <> 0 THEN\n    cnt{i} := cnt{i} + 1;\n    rep{i} := cnt{i} * 65536 + kern.OV[{i}] + 32768;\nEND_IF;\n"
        ));
    }
    s.push_str("END_PROGRAM\n");
    s
}

type Changes = Vec<(u64, usize, i16)>;

fn exec_changes(m: &Machine, sched: &[(i64, usize, i16)], scan: i64, scans: u64) -> Changes {
    let t = m.run_schedule(scan, sched, scans as i64 * scan);
    let mut v: Changes = t
        .changes
        .iter()
        .map(|c| (c.scan, c.output, c.value))
        .collect();
    v.sort();
    v
}

fn run_st(
    kernel_st: &str,
    m: &Machine,
    scan: i64,
    sched: &[(i64, usize, i16)],
    scans: u64,
) -> Changes {
    let words = m.encode().unwrap();
    let src = format!("{kernel_st}\n{}", st_harness(&words, m.inputs, m.outputs));
    let unit = st_parse(&src).unwrap_or_else(|e| panic!("{e}"));
    let mut plc = Plc::new(unit, Some("Main"), scan, Clock::ScanCounted).unwrap();
    let ins: Vec<(i64, String, SV)> = sched
        .iter()
        .map(|(t, i, v)| {
            (
                *t,
                format!("in{i}"),
                SV::Int(
                    i64::from(*v),
                    IntTy {
                        bits: 16,
                        signed: true,
                    },
                ),
            )
        })
        .collect();
    plc.run(scans as i64 * scan, &ins).unwrap();
    let mut out: Changes = Vec::new();
    for c in plc.trace() {
        let Some(i) = c.name.strip_prefix("rep") else {
            continue;
        };
        let SV::Int(v, _) = c.value else { continue };
        if v / 65536 == 0 {
            continue;
        }
        out.push((
            (c.t_ns / scan) as u64,
            i.parse().unwrap(),
            ((v % 65536) - 32768) as i16,
        ));
    }
    out.sort();
    out
}

fn kernel_files() -> Package {
    let p = &programs()[0];
    let nir = nir_of(p.file, &p.text);
    emitter("kernel")
        .unwrap()
        .emit(&nir, &EmitOptions::default())
        .unwrap()
}

#[test]
fn the_st_kernel_in_the_virtual_plc_equals_the_reference_executor() {
    let pkg = kernel_files();
    let kernel = &pkg.files["neuron_kernel.st"];
    let mut lines = Vec::new();
    for p in programs() {
        let (_, c) = compile_one(&p);
        let t = &c.table;
        let scans = (END / t.scan_ns) as u64;
        let mut total = 0;
        for seed in 1..=5 {
            let sched = schedule(t, seed, END);
            let want = exec_changes(&t.machine, &sched, t.scan_ns, scans);
            let got = run_st(kernel, &t.machine, t.scan_ns, &sched, scans);
            assert_eq!(got, want, "{} seed {seed}", p.name);
            total += want.len();
        }
        assert!(total >= 20, "{}: only {total} changes", p.name);
        lines.push(format!(
            "{:<20} ST kernel = reference executor over 5 schedules x {scans} scans: {total} output changes, identical scan, output and value",
            p.name
        ));
    }
    // Random tables: the kernel does not depend on the table.
    let mut total = 0;
    for seed in 0..12 {
        let m = random_machine(seed);
        let (sched, scans) = random_schedule(&m, seed, 150);
        let want = exec_changes(&m, &sched, 10_000_000, scans);
        let got = run_st(kernel, &m, 10_000_000, &sched, scans);
        assert_eq!(got, want, "random table {seed}");
        total += want.len();
    }
    lines.push(format!(
        "12 random tables x 150 scans: ST kernel = reference executor ({total} output changes)"
    ));
    report("b-st-kernel", &lines);
}

fn random_schedule(m: &Machine, seed: u64, scans: u64) -> (Vec<(i64, usize, i16)>, u64) {
    let mut r = Rng(seed.wrapping_add(77));
    let mut out = Vec::new();
    if m.inputs > 0 {
        for k in 0..scans {
            if r.below(3) == 0 {
                out.push((
                    k as i64 * 10_000_000 + 3_000_000,
                    r.below(m.inputs as u64) as usize,
                    r.below(9) as i16 - 4,
                ));
            }
        }
    }
    (out, scans)
}

// ---- (c) the C99 kernel ---------------------------------------------------------------

const C_DRIVER: &str = r#"#include <stdio.h>
#include <stdlib.h>
#include "nrt_kernel.h"

int main(int argc, char **argv)
{
    static uint16_t tbl[NRT_MAX_TABLE];
    unsigned n = 0;
    unsigned v;
    nrt_kernel_t k;
    int16_t in[NRT_MAX_IO];
    long scans, s;
    int nin, nout, i, x, rc;
    FILE *f;
    if (argc < 3) return 2;
    f = fopen(argv[1], "r");
    if (!f) return 2;
    while (n < NRT_MAX_TABLE && fscanf(f, "%u", &v) == 1) tbl[n++] = (uint16_t)v;
    fclose(f);
    rc = nrt_init(&k, tbl, (uint16_t)n);
    printf("init %d\n", rc);
    if (rc != 0) return 0;
    f = fopen(argv[2], "r");
    if (!f) return 2;
    if (fscanf(f, "%d %d %ld", &nin, &nout, &scans) != 3) return 2;
    for (s = 0; s < scans; s++) {
        for (i = 0; i < nin; i++) {
            if (fscanf(f, "%d", &x) != 1) return 2;
            in[i] = (int16_t)x;
        }
        nrt_scan(&k, in);
        for (i = 0; i < nout; i++) {
            if (k.chg[i]) printf("%ld %d %d\n", s, i, (int)k.ov[i]);
        }
    }
    fclose(f);
    return 0;
}
"#;

fn gcc_available() -> bool {
    Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_exe(exe: &Path, args: &[&Path]) -> String {
    // Windows App Control may block a fresh executable once (os error 4551): try again.
    let mut last = None;
    for _ in 0..5 {
        match Command::new(exe).args(args).output() {
            Ok(o) => return String::from_utf8_lossy(&o.stdout).into_owned(),
            Err(e) => last = Some(e),
        }
    }
    panic!("cannot run {}: {:?}", exe.display(), last);
}

/// Per-scan inputs for the C driver.
fn input_matrix(m: &Machine, scan: i64, sched: &[(i64, usize, i16)], scans: u64) -> String {
    let mut s = format!("{} {} {}\n", m.inputs, m.outputs, scans);
    let mut cur = vec![0i16; m.inputs];
    let mut next = 0;
    for k in 0..scans {
        let now = k as i64 * scan;
        while next < sched.len() && sched[next].0 <= now {
            cur[sched[next].1] = sched[next].2;
            next += 1;
        }
        s.push_str(&cur.iter().map(i16::to_string).collect::<Vec<_>>().join(" "));
        s.push('\n');
    }
    s
}

fn parse_changes(out: &str) -> (i32, Changes) {
    let mut lines = out.lines();
    let init: i32 = lines
        .next()
        .and_then(|l| l.strip_prefix("init "))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut v: Changes = lines
        .map(|l| {
            let p: Vec<i64> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            (p[0] as u64, p[1] as usize, p[2] as i16)
        })
        .collect();
    v.sort();
    (init, v)
}

#[test]
fn the_c99_kernel_built_with_gcc_equals_the_reference_executor() {
    if !gcc_available() {
        assert!(
            std::env::var("NEURON_REQUIRE_TOOLCHAIN").as_deref() != Ok("1"),
            "gcc is required (NEURON_REQUIRE_TOOLCHAIN=1) but not found"
        );
        eprintln!("skipped: gcc not found");
        return;
    }
    let pkg = kernel_files();
    let dir = root().join("out/c6");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("nrt_kernel.c"), &pkg.files["nrt_kernel.c"]).unwrap();
    std::fs::write(dir.join("nrt_kernel.h"), &pkg.files["nrt_kernel.h"]).unwrap();
    std::fs::write(dir.join("driver.c"), C_DRIVER).unwrap();
    let exe = dir.join("driver.exe");
    let _ = std::fs::remove_file(&exe);
    let cc = Command::new("gcc")
        .current_dir(&dir)
        .args([
            "-std=c99",
            "-pedantic",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-O1",
            "nrt_kernel.c",
            "driver.c",
            "-o",
            "driver.exe",
        ])
        .output()
        .unwrap();
    assert!(
        cc.status.success(),
        "gcc failed:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let table_file = dir.join("table.txt");
    let sched_file = dir.join("schedule.txt");
    let run = |m: &Machine, scan: i64, sched: &[(i64, usize, i16)], scans: u64| -> (i32, Changes) {
        let words = m.encode().unwrap();
        std::fs::write(
            &table_file,
            words
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        )
        .unwrap();
        std::fs::write(&sched_file, input_matrix(m, scan, sched, scans)).unwrap();
        parse_changes(&run_exe(&exe, &[&table_file, &sched_file]))
    };
    let mut lines = Vec::new();
    for p in programs() {
        let (_, c) = compile_one(&p);
        let t = &c.table;
        let scans = (END / t.scan_ns) as u64;
        let mut total = 0;
        for seed in 1..=5 {
            let sched = schedule(t, seed, END);
            let want = exec_changes(&t.machine, &sched, t.scan_ns, scans);
            let (init, got) = run(&t.machine, t.scan_ns, &sched, scans);
            assert_eq!(init, 0, "{}", p.name);
            assert_eq!(got, want, "{} seed {seed}", p.name);
            total += want.len();
        }
        assert!(total >= 20, "{}: only {total} changes", p.name);
        lines.push(format!(
            "{:<20} C99 kernel (gcc -std=c99 -pedantic -Wall -Wextra -Werror) = reference executor over 5 schedules x {scans} scans: {total} output changes identical",
            p.name
        ));
    }
    let mut total = 0;
    for seed in 0..60 {
        let m = random_machine(seed);
        let (sched, scans) = random_schedule(&m, seed, 400);
        let want = exec_changes(&m, &sched, 10_000_000, scans);
        let (init, got) = run(&m, 10_000_000, &sched, scans);
        assert_eq!(init, 0);
        assert_eq!(got, want, "random table {seed}");
        total += want.len();
    }
    lines.push(format!(
        "60 random tables x 400 scans: C99 kernel = reference executor ({total} output changes)"
    ));
    // A damaged table is refused: a flipped word gives the checksum error, a bad magic its own.
    let (_, c) = compile_one(&programs()[0]);
    let mut w = c.table.words().unwrap();
    w[20] ^= 0x0040;
    std::fs::write(
        &table_file,
        w.iter().map(u16::to_string).collect::<Vec<_>>().join(" "),
    )
    .unwrap();
    std::fs::write(&sched_file, "0 3 0\n").unwrap();
    let (init, _) = parse_changes(&run_exe(&exe, &[&table_file, &sched_file]));
    assert_eq!(init, 4, "checksum");
    w[20] ^= 0x0040;
    w[0] ^= 1;
    std::fs::write(
        &table_file,
        w.iter().map(u16::to_string).collect::<Vec<_>>().join(" "),
    )
    .unwrap();
    let (init, _) = parse_changes(&run_exe(&exe, &[&table_file, &sched_file]));
    assert_eq!(init, 1, "magic");
    lines.push(
        "a flipped word is refused by nrt_init with the checksum error (4), a bad magic with 1"
            .to_string(),
    );
    report("c-c-kernel", &lines);
}

// ---- (d) the SPEL+ kernel ---------------------------------------------------------------

fn spel_harness(words: &[u16], t: &Table, scans: u64) -> String {
    let mut s = String::from("Function NkLoad\n");
    for (k, w) in words.iter().enumerate() {
        s.push_str(&format!("  nk_tbl({k}) = {}\n", *w as i16));
    }
    s.push_str("Fend\n\nFunction main\n  Call NkLoad\n");
    s.push_str(&format!("  nk_len = {}\n", words.len()));
    s.push_str(&format!("  For nk_k = 0 To {}\n", scans - 1));
    for (i, p) in t.inputs.iter().enumerate() {
        if p.boolean {
            s.push_str(&format!("    nk_in({i}) = Sw({i})\n"));
        } else {
            s.push_str(&format!("    nk_in({i}) = InW({i})\n"));
        }
    }
    s.push_str("    Call NeuronKernel\n    If nk_err <> 0 Then\n      Quit All\n    EndIf\n");
    for (i, p) in t.outputs.iter().enumerate() {
        if p.boolean {
            s.push_str(&format!(
                "    If nk_chg({i}) <> 0 Then\n      If nk_ov({i}) <> 0 Then\n        On {i}\n      Else\n        Off {i}\n      EndIf\n    EndIf\n"
            ));
        } else {
            s.push_str(&format!(
                "    If nk_chg({i}) <> 0 Then\n      OutW {i}, nk_ov({i})\n    EndIf\n"
            ));
        }
    }
    s.push_str(&format!("    Wait {}\n", t.scan_ns as f64 / 1e9));
    s.push_str("  Next nk_k\nFend\n");
    s
}

#[test]
fn the_spel_kernel_in_the_virtual_controller_equals_the_reference_executor() {
    let pkg = kernel_files();
    let kernel = &pkg.files["neuron_kernel.spel"];
    let mut lines = Vec::new();
    let mut progs = programs();
    progs.push(wrap_program());
    for p in progs {
        let (_, c) = compile_one(&p);
        let t = &c.table;
        let scans = (END / t.scan_ns) as u64;
        let mut total = 0;
        let mut int_total = 0;
        for seed in 1..=5 {
            let mut sched = schedule(t, seed, END);
            if p.name == "c6_wrap" {
                for e in &mut sched {
                    if !t.inputs[e.1].boolean {
                        e.2 = WIDE[e.2 as usize % WIDE.len()];
                    }
                }
            }
            // The controller records the changes of boolean outputs and output words; it
            // starts them at 0, so the first report of a 0 is not a change.
            let all = exec_changes(&t.machine, &sched, t.scan_ns, scans);
            let mut seen = std::collections::BTreeSet::new();
            let want: Changes = all
                .into_iter()
                .filter(|(_, o, v)| {
                    let first = seen.insert(*o);
                    !(first && *v == 0)
                })
                .collect();
            int_total += want
                .iter()
                .filter(|(_, o, _)| !t.outputs[*o].boolean)
                .count();
            let words = t.words().unwrap();
            let src = format!("{kernel}\n{}", spel_harness(&words, t, scans));
            let prog = neuron_spel::parse(&src).unwrap_or_else(|e| panic!("{e}"));
            let mut ctl = neuron_spel::Controller::new(
                prog,
                neuron_spel::Options {
                    max_steps: 200_000_000,
                    ..neuron_spel::Options::default()
                },
            );
            for (at, i, v) in &sched {
                if t.inputs[*i].boolean {
                    ctl.set_input(*at, *i as i64, *v != 0);
                } else {
                    ctl.set_input_word(*at, *i as i64, i64::from(*v));
                }
            }
            let outcome = ctl.run();
            assert_eq!(outcome, neuron_spel::Outcome::Finished, "{}", p.name);
            let mut got: Changes = ctl
                .outputs()
                .iter()
                .map(|c| ((c.t_ns / t.scan_ns) as u64, c.bit as usize, i16::from(c.on)))
                .chain(ctl.word_outputs().iter().map(|c| {
                    (
                        (c.t_ns / t.scan_ns) as u64,
                        c.word as usize,
                        i16::try_from(c.value).unwrap(),
                    )
                }))
                .collect();
            got.sort();
            assert_eq!(got, want, "{} seed {seed}", p.name);
            total += want.len();
        }
        assert!(total >= 10, "{}: only {total} changes", p.name);
        if p.name == "c6_press" || p.name == "c6_wrap" {
            assert!(
                int_total >= 5,
                "{}: only {int_total} integer changes",
                p.name
            );
        }
        lines.push(format!(
            "{:<20} SPEL+ kernel in neuron-spel = reference executor (boolean outputs and integer words) over 5 schedules x {scans} scans: {total} output changes identical, {int_total} of them integer",
            p.name
        ));
    }
    report("d-spel-kernel", &lines);
}
