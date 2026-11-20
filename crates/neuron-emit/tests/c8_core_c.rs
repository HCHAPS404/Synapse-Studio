//! Task C8: the `c` emitter (`core` modules to portable C99), checked by running it.
//!
//! Every program is analyzed, emitted, built with the host C compiler (`-std=c99 -pedantic
//! -Wall -Wextra -Werror`) into a driver that the test writes (`NEURON_CC`, or `gcc` on
//! `PATH`; a missing compiler is an error when `NEURON_REQUIRE_TOOLCHAIN` is set and a skip
//! otherwise) and called on a table of inputs. The reference interpreter runs the same calls
//! as `test` blocks: a value must be equal (reals: within 1e-4 relative) and a call where the
//! C sets `nrt_fault` must stop the interpreter with the same fault.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use neuron_emit::{emitter, Class, EmitOptions, Package};
use neuron_interp::{run_tests, Options};
use neuron_nir::Nir;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use std::path::{Path, PathBuf};
use std::process::Command;

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
            path: format!("{name}.neuron"),
            text: text.into(),
        }],
        None,
        &SemaOptions::default(),
    );
    a.nir
        .clone()
        .unwrap_or_else(|| panic!("{name}:\n{}\n---\n{text}", a.render()))
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("neuron-c8-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn emit_c(name: &str, text: &str) -> Package {
    let nir = nir_of(name, text);
    emitter("c")
        .unwrap()
        .emit(&nir, &EmitOptions::default())
        .unwrap_or_else(|d| panic!("{name}: {d:?}"))
}

/// The C compiler, or `None` (a skip) when there is none and it is not required.
fn cc() -> Option<String> {
    let cc = std::env::var("NEURON_CC").unwrap_or_else(|_| "gcc".to_string());
    if Command::new(&cc)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return Some(cc);
    }
    assert!(
        std::env::var_os("NEURON_REQUIRE_TOOLCHAIN").is_none(),
        "a C compiler is required (NEURON_REQUIRE_TOOLCHAIN is set) but `{cc}` was not found"
    );
    eprintln!("skipped: no C compiler found (NEURON_CC or gcc on PATH)");
    None
}

/// Deterministic generator for the extra rows.
struct Lcg(u64);

impl Lcg {
    fn draw(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

// ---------------------------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Kind {
    Signed,
    Unsigned,
    Bool,
    /// A real: the Neuron unit of the result and its factor to base SI units.
    Real(&'static str, f64),
}

/// One argument: its Neuron text and its C text.
struct Arg {
    n: String,
    c: String,
}

fn c_int(v: i64) -> String {
    if i32::try_from(v).is_ok() {
        v.to_string()
    } else {
        format!("{v}ll")
    }
}

fn i(v: i64) -> Arg {
    Arg {
        n: v.to_string(),
        c: c_int(v),
    }
}

fn r(v: f64) -> Arg {
    Arg {
        n: format!("{v:?}"),
        c: format!("{v:?}"),
    }
}

fn raw(n: &str, c: &str) -> Arg {
    Arg {
        n: n.into(),
        c: c.into(),
    }
}

struct Fun {
    name: &'static str,
    kind: Kind,
    calls: Vec<Vec<Arg>>,
}

fn ints(name: &'static str, kind: Kind, rows: &[&[i64]]) -> Fun {
    Fun {
        name,
        kind,
        calls: rows
            .iter()
            .map(|row| row.iter().map(|v| i(*v)).collect())
            .collect(),
    }
}

/// Every pair of the pool.
fn square(pool: &[i64]) -> Vec<Vec<i64>> {
    let mut out = Vec::new();
    for a in pool {
        for b in pool {
            out.push(vec![*a, *b]);
        }
    }
    out
}

fn from_rows(name: &'static str, kind: Kind, rows: Vec<Vec<i64>>) -> Fun {
    Fun {
        name,
        kind,
        calls: rows
            .iter()
            .map(|row| row.iter().map(|v| i(*v)).collect())
            .collect(),
    }
}

struct Prog {
    module: &'static str,
    src: String,
    funs: Vec<Fun>,
}

#[derive(Debug, Clone, PartialEq)]
enum Res {
    Val(String),
    Fault(String),
}

fn prefix(module: &str) -> String {
    module
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Builds the driver, compiles it with the emitted files and returns one result per call.
fn c_results(cc: &str, pkg: &Package, p: &Prog, tag: &str) -> Vec<Res> {
    let pre = prefix(p.module);
    let dir = scratch(&format!("{}-{tag}", pre));
    pkg.write(&dir).unwrap();
    let mut main = format!("#include <stdio.h>\n#include \"{pre}.h\"\n\nint main(void) {{\n");
    let mut k = 0usize;
    for f in &p.funs {
        for call in &f.calls {
            let args: Vec<&str> = call.iter().map(|a| a.c.as_str()).collect();
            let (ty, fmt, cast) = match f.kind {
                Kind::Signed => ("long long", "%lld", "(long long)"),
                Kind::Unsigned => ("unsigned long long", "%llu", "(unsigned long long)"),
                Kind::Bool => ("int", "%d", "(int)"),
                Kind::Real(..) => ("double", "%.17g", "(double)"),
            };
            main.push_str(&format!(
                "  {{\n    nrt_fault = 0;\n    {ty} v = {cast}{pre}_{}({});\n    if (nrt_fault != 0) {{\n      printf(\"{k} F %s\\n\", nrt_fault_name(nrt_fault));\n    }} else {{\n      printf(\"{k} R {fmt}\\n\", v);\n    }}\n  }}\n",
                f.name,
                args.join(", ")
            ));
            k += 1;
        }
    }
    main.push_str("  return 0;\n}\n");
    std::fs::write(dir.join("drv.c"), main).unwrap();
    let exe = dir.join("drv.exe");
    let out = Command::new(cc)
        .current_dir(&dir)
        .args([
            "-std=c99",
            "-pedantic",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-O2",
            "-o",
        ])
        .arg(&exe)
        .arg(format!("{pre}.c"))
        .args(["nrt_core.c", "drv.c", "-lm"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}: gcc failed:\n{}\n{}",
        p.module,
        String::from_utf8_lossy(&out.stderr),
        std::fs::read_to_string(dir.join(format!("{pre}.c"))).unwrap_or_default()
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success(), "{}: the C program failed", p.module);
    let mut res = Vec::new();
    for line in String::from_utf8_lossy(&run.stdout).lines() {
        let mut it = line.splitn(3, ' ');
        let _idx = it.next().unwrap();
        let tag = it.next().unwrap();
        let rest = it.next().unwrap().to_string();
        res.push(if tag == "F" {
            Res::Fault(rest)
        } else {
            Res::Val(rest)
        });
    }
    assert_eq!(res.len(), k, "{}: results", p.module);
    res
}

/// The Neuron source of the tests: one `test` per call.
fn test_blocks(p: &Prog, rename: &dyn Fn(&str) -> String, res: &[Res]) -> String {
    let mut out = String::new();
    let mut k = 0usize;
    for f in &p.funs {
        for call in &f.calls {
            let args: Vec<&str> = call.iter().map(|a| a.n.as_str()).collect();
            let c = format!("{}({})", rename(f.name), args.join(", "));
            out.push_str(&format!("\ntest \"c{k}\":\n"));
            match (&res[k], f.kind) {
                (Res::Fault(_), _) => out.push_str(&format!("  assert {c} == {c}, \"c{k}\"\n")),
                (Res::Val(v), Kind::Signed | Kind::Unsigned) => {
                    out.push_str(&format!("  assert {c} == {v}, \"c{k}\"\n"));
                }
                (Res::Val(v), Kind::Bool) => {
                    if v == "1" {
                        out.push_str(&format!("  assert {c}, \"c{k}\"\n"));
                    } else {
                        out.push_str(&format!("  assert not {c}, \"c{k}\"\n"));
                    }
                }
                (Res::Val(v), Kind::Real(unit, scale)) => {
                    let x = v.parse::<f64>().unwrap() / scale;
                    let tol = 1e-4 * x.abs().max(1.0);
                    let u = if unit.is_empty() {
                        String::new()
                    } else {
                        format!(" {unit}")
                    };
                    out.push_str(&format!(
                        "  assert {c} > {:.9}{u} and {c} < {:.9}{u}, \"c{k}\"\n",
                        x - tol,
                        x + tol
                    ));
                }
            }
            k += 1;
        }
    }
    out
}

/// Runs the tests against the C results; returns (calls compared, faults compared).
fn compare(p: &Prog, src: &str, rename: &dyn Fn(&str) -> String, res: &[Res]) -> (usize, usize) {
    let text = format!("{src}{}", test_blocks(p, rename, res));
    let nir = nir_of(p.module, &text);
    let reports = run_tests(&nir, &Options::default());
    let mut bad = Vec::new();
    let mut faults = 0;
    for (k, want) in res.iter().enumerate() {
        let name = format!("c{k}");
        let Some(rep) = reports.iter().find(|t| t.name == name) else {
            bad.push(format!("{name}: no report"));
            continue;
        };
        match want {
            Res::Fault(code) => {
                faults += 1;
                let expect = format!("raised fault {code}");
                if rep.passed || !rep.message.contains(&expect) {
                    bad.push(format!(
                        "{name}: C faulted with {code}, interpreter: passed={} `{}`",
                        rep.passed, rep.message
                    ));
                }
            }
            Res::Val(v) => {
                if !rep.passed {
                    bad.push(format!(
                        "{name}: C gave {v}, interpreter: `{}`",
                        rep.message
                    ));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{}: {} of {} calls differ:\n{}",
        p.module,
        bad.len(),
        res.len(),
        bad.join("\n")
    );
    (res.len(), faults)
}

/// Emits, builds, runs and compares one program. Returns (calls, faults) or `None` on a skip.
fn check(p: &Prog) -> Option<(usize, usize)> {
    let pkg = emit_c(p.module, &p.src);
    let cc = cc()?;
    let res = c_results(&cc, &pkg, p, "main");
    let ident = |n: &str| n.to_string();
    let (calls, faults) = compare(p, &p.src, &ident, &res);
    eprintln!(
        "{}: {} functions, {calls} calls compared with the interpreter ({faults} faults), fidelity {}",
        p.module,
        p.funs.len(),
        summary(&pkg)
    );
    Some((calls, faults))
}

fn summary(pkg: &Package) -> String {
    let s = &pkg.fidelity.summary;
    format!(
        "{} exact, {} equivalent, {} approximate",
        s.exact, s.equivalent, s.approximate
    )
}

// ---------------------------------------------------------------------------------------
// Programs written in the test
// ---------------------------------------------------------------------------------------

const S: Kind = Kind::Signed;
const U: Kind = Kind::Unsigned;

fn intmath() -> Prog {
    let src = "## Integer maths with bounded loops.
module t_intmath for core

pub fn sum_to(n: i32) -> i32:
  var s: i32 = 0
  var i: i32 = 1
  while i <= n max 1000:
    s += i
    i += 1
  return s

pub fn count_multiples(k: u32) -> u32:
  var c: u32 = 0
  for i in 0..100:
    if i % 3 == 0:
      c += 1
  return c + k

pub fn factorial(n: u32) -> u32:
  var r: u32 = 1
  var k: u32 = 2
  while k <= n max 20:
    r *= k
    k += 1
  return r

pub fn collatz_steps(n: u32) -> u32:
  var x: u32 = n
  var steps: u32 = 0
  while x != 1 and steps < 200 max 200:
    if x % 2 == 0:
      x = x / 2
    else:
      x = 3 * x + 1
    steps += 1
  return steps

pub fn ipow(base: u32, exp: u32) -> u32 = base ** exp
";
    Prog {
        module: "t_intmath",
        src: src.into(),
        funs: vec![
            ints("sum_to", S, &[&[0], &[1], &[2], &[10], &[100], &[1000]]),
            ints(
                "count_multiples",
                U,
                &[&[0], &[1], &[5], &[4_294_967_295], &[4_294_967_262]],
            ),
            ints(
                "factorial",
                U,
                &[&[0], &[1], &[5], &[10], &[12], &[13], &[20]],
            ),
            ints(
                "collatz_steps",
                U,
                &[&[1], &[6], &[7], &[27], &[97], &[0], &[3_000_000_001]],
            ),
            ints(
                "ipow",
                U,
                &[
                    &[2, 10],
                    &[3, 20],
                    &[10, 9],
                    &[2, 31],
                    &[2, 32],
                    &[0, 0],
                    &[7, 0],
                    &[10, 10],
                ],
            ),
        ],
    }
}

fn sat() -> Prog {
    let src = "## Saturating, wrapping and checked arithmetic.
module t_sat for core

pub fn sat_add8(a: u8, b: u8) -> u8 = a +| b
pub fn sat_sub8(a: u8, b: u8) -> u8 = a -| b
pub fn sat_mul16(a: i16, b: i16) -> i16 = a *| b
pub fn wrap_add8(a: u8, b: u8) -> u8 = a +% b
pub fn wrap_sub8(a: u8, b: u8) -> u8 = a -% b
pub fn wrap_add_i8(a: i8, b: i8) -> i8 = a +% b
pub fn add8(a: u8, b: u8) -> u8 = a + b
pub fn dec8(a: u8) -> u8 = a - 1
pub fn neg8(a: i8) -> i8 = -a
";
    let u8s = [0, 1, 100, 127, 128, 200, 254, 255];
    let i16s = [-32767, -200, -1, 0, 1, 181, 182, 32767];
    let i8s = [-127, -1, 0, 1, 100, 127];
    Prog {
        module: "t_sat",
        src: src.into(),
        funs: vec![
            from_rows("sat_add8", U, square(&u8s)),
            from_rows("sat_sub8", U, square(&u8s)),
            from_rows("sat_mul16", S, square(&i16s)),
            from_rows("wrap_add8", U, square(&u8s)),
            from_rows("wrap_sub8", U, square(&u8s)),
            from_rows("wrap_add_i8", S, square(&i8s)),
            ints(
                "add8",
                U,
                &[&[0, 0], &[200, 55], &[200, 56], &[255, 1], &[255, 0]],
            ),
            ints("dec8", U, &[&[0], &[1], &[255]]),
            ints("neg8", S, &[&[5], &[-5], &[0], &[127], &[-127]]),
        ],
    }
}

fn gcd() -> Prog {
    let src = "## Euclid's algorithm and the least common multiple.
module t_gcd for core

pub fn gcd(a: u32, b: u32) -> u32:
  var x: u32 = a
  var y: u32 = b
  while y != 0 max 64:
    let t = x % y
    x = y
    y = t
  return x

pub fn lcm(a: u32, b: u32) -> u32:
  if a == 0 or b == 0:
    return 0
  return a / gcd(a, b) * b
";
    let pool = [
        0,
        1,
        2,
        12,
        18,
        35,
        100,
        255,
        1000,
        65_535,
        123_456,
        4_294_967_295,
    ];
    let mut rows = square(&pool);
    let mut rng = Lcg(0xC8);
    for _ in 0..40 {
        rows.push(vec![
            i64::try_from(rng.draw() & 0x7fff_ffff).unwrap(),
            i64::try_from(rng.draw() & 0xffff).unwrap(),
        ]);
    }
    Prog {
        module: "t_gcd",
        src: src.into(),
        funs: vec![
            from_rows("gcd", U, rows),
            ints(
                "lcm",
                U,
                &[
                    &[0, 5],
                    &[4, 6],
                    &[21, 6],
                    &[100, 35],
                    &[255, 77],
                    &[4_000_000_000, 3],
                    &[65_536, 65_537],
                ],
            ),
        ],
    }
}

fn bits() -> Prog {
    let src = "## Bit operations.
module t_bits for core

pub fn popcount(x: u32) -> u32:
  var c: u32 = 0
  for i in 0..32:
    if (x >> i) & 1 == 1:
      c += 1
  return c

pub fn parity(x: u32) -> u32:
  var p: u32 = 0
  var v: u32 = x
  while v != 0 max 32:
    p = p ^ (v & 1)
    v = v >> 1
  return p

pub fn set_bit(x: u32, n: u32) -> u32:
  requires n < 32
  return x | (1 << n)

pub fn flip(x: u8) -> u8 = ~x
pub fn shl4(x: u8) -> u8 = x << 4
pub fn shr3(x: u8) -> u8 = x >> 3
pub fn mask_low(x: u32, n: u32) -> u32 = x & ((1 << n) - 1)
";
    let pool = [
        0,
        1,
        3,
        255,
        256,
        65_535,
        123_456_789,
        2_147_483_648,
        4_294_967_295,
    ];
    let singles: Vec<Vec<i64>> = pool.iter().map(|v| vec![*v]).collect();
    let mut set_rows = Vec::new();
    for x in [0, 5, 4_294_967_295] {
        for n in [0, 1, 7, 31, 32, 40] {
            set_rows.push(vec![x, n]);
        }
    }
    Prog {
        module: "t_bits",
        src: src.into(),
        funs: vec![
            from_rows("popcount", U, singles.clone()),
            from_rows("parity", U, singles),
            from_rows("set_bit", U, set_rows),
            ints("flip", U, &[&[0], &[1], &[170], &[255]]),
            ints("shl4", U, &[&[0], &[1], &[15], &[16], &[255]]),
            ints("shr3", U, &[&[0], &[8], &[255]]),
            ints(
                "mask_low",
                U,
                &[
                    &[4_294_967_295, 8],
                    &[305_419_896, 16],
                    &[255, 0],
                    &[255, 31],
                    &[255, 32],
                ],
            ),
        ],
    }
}

fn enums() -> Prog {
    let src = "## An enum, match on an enum and on integers.
module t_enum for core

pub enum Mode:
  Off
  Manual
  Auto

pub fn limit(m: Mode) -> u16:
  match m:
    Mode.Off => return 0
    Mode.Manual => return 40
    Mode.Auto => return 100

pub fn next_mode(m: Mode) -> Mode:
  match m:
    Mode.Off => return Mode.Manual
    Mode.Manual => return Mode.Auto
    Mode.Auto => return Mode.Off

pub fn next_limit(m: Mode) -> u16 = limit(next_mode(m))

pub fn grade(score: u32) -> u32:
  match score:
    0 => return 10
    1..50 => return 20
    50..=79 => return 30
    _ => return 40
";
    let modes = ["Off", "Manual", "Auto"];
    let mode = |m: &str| raw(&format!("Mode.{m}"), &format!("t_enum_Mode_{m}"));
    Prog {
        module: "t_enum",
        src: src.into(),
        funs: vec![
            Fun {
                name: "limit",
                kind: U,
                calls: modes.iter().map(|m| vec![mode(m)]).collect(),
            },
            Fun {
                name: "next_limit",
                kind: U,
                calls: modes.iter().map(|m| vec![mode(m)]).collect(),
            },
            ints(
                "grade",
                U,
                &[&[0], &[1], &[49], &[50], &[79], &[80], &[4_000_000_000]],
            ),
        ],
    }
}

fn structs() -> Prog {
    let src = "## A struct, a copy of it and field assignment.
module t_struct for core

pub struct Point:
  x: i32
  y: i32

pub fn manhattan(p: Point) -> i32 = abs(p.x) + abs(p.y)

pub fn dot(a: Point, b: Point) -> i32 = a.x * b.x + a.y * b.y

pub fn shifted_x(p: Point, dx: i32) -> i32:
  var q = p
  q.x += dx
  return q.x + p.x
";
    let pt = |x: i64, y: i64| {
        raw(
            &format!("Point(x: {x}, y: {y})"),
            &format!("(t_struct_Point){{ .x = {x}, .y = {y} }}"),
        )
    };
    Prog {
        module: "t_struct",
        src: src.into(),
        funs: vec![
            Fun {
                name: "manhattan",
                kind: S,
                calls: vec![
                    vec![pt(3, -4)],
                    vec![pt(0, 0)],
                    vec![pt(-100, -200)],
                    vec![pt(2_000_000_000, 2_000_000_000)],
                ],
            },
            Fun {
                name: "dot",
                kind: S,
                calls: vec![
                    vec![pt(1, 2), pt(3, 4)],
                    vec![pt(-5, 5), pt(5, 5)],
                    vec![pt(60_000, 0), pt(60_000, 0)],
                ],
            },
            Fun {
                name: "shifted_x",
                kind: S,
                calls: vec![
                    vec![pt(10, 0), i(5)],
                    vec![pt(10, 0), i(-15)],
                    vec![pt(2_000_000_000, 0), i(2_000_000_000)],
                ],
            },
        ],
    }
}

fn calls() -> Prog {
    let src = "## One function calling another, a constant, division.
module t_calls for core

const LIMIT: i32 = 1000

fn square(x: i32) -> i32 = x * x

pub fn sum_squares(a: i32, b: i32) -> i32 = square(a) + square(b)

pub fn clamp_sq(x: i32) -> i32:
  let s = square(x)
  if s > LIMIT:
    return LIMIT
  return s

pub fn cube(x: i32) -> i32 = x ** 3

pub fn ratio(a: i32, b: i32) -> i32 = a / b

pub fn remainder(a: i32, b: i32) -> i32 = a % b
";
    Prog {
        module: "t_calls",
        src: src.into(),
        funs: vec![
            ints(
                "sum_squares",
                S,
                &[&[3, 4], &[0, 0], &[-5, 5], &[40_000, 40_000]],
            ),
            ints(
                "clamp_sq",
                S,
                &[&[0], &[10], &[31], &[32], &[-40], &[50_000]],
            ),
            ints("cube", S, &[&[3], &[-3], &[1290], &[1291]]),
            ints(
                "ratio",
                S,
                &[&[7, 2], &[-7, 2], &[7, -2], &[0, 5], &[1, 0], &[-8, 0]],
            ),
            ints("remainder", S, &[&[7, 3], &[-7, 3], &[7, 0]]),
        ],
    }
}

fn reals() -> Prog {
    let src = "## Reals: arithmetic, library functions and a checked cast.
module t_real for core

pub fn avg(a: f64, b: f64) -> f64 = (a + b) / 2.0
pub fn hyp(a: f64, b: f64) -> f64 = sqrt(a * a + b * b)
pub fn widen(x: i32) -> f64 = x as f64
pub fn widen32(x: f32) -> f64 = x as f64
pub fn lim(x: f64) -> f64 = clamp(x, -1.0, 1.0)
pub fn rd(x: f64) -> f64 = round(x)
pub fn fl(x: f64) -> f64 = floor(x)
";
    let one = |v: &[f64]| -> Vec<Vec<Arg>> { v.iter().map(|x| vec![r(*x)]).collect() };
    Prog {
        module: "t_real",
        src: src.into(),
        funs: vec![
            Fun {
                name: "avg",
                kind: Kind::Real("", 1.0),
                calls: vec![
                    vec![r(1.0), r(2.0)],
                    vec![r(-3.5), r(0.5)],
                    vec![r(0.0), r(0.0)],
                ],
            },
            Fun {
                name: "hyp",
                kind: Kind::Real("", 1.0),
                calls: vec![
                    vec![r(3.0), r(4.0)],
                    vec![r(5.0), r(12.0)],
                    vec![r(0.0), r(0.0)],
                ],
            },
            Fun {
                name: "widen",
                kind: Kind::Real("", 1.0),
                calls: vec![vec![i(-7)], vec![i(0)], vec![i(2_147_483_647)]],
            },
            Fun {
                name: "widen32",
                kind: Kind::Real("", 1.0),
                calls: one(&[0.5, -2.25, 100.0]),
            },
            Fun {
                name: "lim",
                kind: Kind::Real("", 1.0),
                calls: one(&[-5.0, 0.25, 7.0]),
            },
            Fun {
                name: "rd",
                kind: Kind::Real("", 1.0),
                calls: one(&[2.5, -2.5, 2.4, 0.0]),
            },
            Fun {
                name: "fl",
                kind: Kind::Real("", 1.0),
                calls: one(&[2.5, -2.5, 3.0]),
            },
        ],
    }
}

fn totals(parts: &[Option<(usize, usize)>]) -> (usize, usize) {
    parts
        .iter()
        .flatten()
        .fold((0, 0), |(c, f), (a, b)| (c + a, f + b))
}

#[test]
fn integer_maths_with_bounded_loops_matches_the_interpreter() {
    let (calls, faults) = totals(&[check(&intmath())]);
    assert!(
        calls == 0 || (calls >= 20 && faults >= 4),
        "{calls} {faults}"
    );
}

#[test]
fn saturating_wrapping_and_checked_arithmetic_match_the_interpreter() {
    let (calls, faults) = totals(&[check(&sat())]);
    assert!(
        calls == 0 || (calls >= 300 && faults >= 3),
        "{calls} {faults}"
    );
}

#[test]
fn gcd_and_lcm_match_the_interpreter() {
    let (calls, faults) = totals(&[check(&gcd())]);
    assert!(
        calls == 0 || (calls >= 150 && faults >= 2),
        "{calls} {faults}"
    );
}

#[test]
fn bit_operations_match_the_interpreter() {
    let (calls, faults) = totals(&[check(&bits())]);
    assert!(
        calls == 0 || (calls >= 40 && faults >= 3),
        "{calls} {faults}"
    );
}

#[test]
fn enums_and_match_match_the_interpreter() {
    let (calls, _) = totals(&[check(&enums())]);
    assert!(calls == 0 || calls >= 13, "{calls}");
}

#[test]
fn structs_match_the_interpreter() {
    let (calls, faults) = totals(&[check(&structs())]);
    assert!(
        calls == 0 || (calls >= 10 && faults >= 3),
        "{calls} {faults}"
    );
}

#[test]
fn calls_constants_and_division_match_the_interpreter() {
    let (calls, faults) = totals(&[check(&calls())]);
    assert!(
        calls == 0 || (calls >= 20 && faults >= 6),
        "{calls} {faults}"
    );
}

#[test]
fn reals_and_checked_casts_match_the_interpreter() {
    let (calls, faults) = totals(&[check(&reals())]);
    assert!(
        calls == 0 || (calls >= 20 && faults == 0),
        "{calls} {faults}"
    );
}

// ---------------------------------------------------------------------------------------
// The corpus and the library
// ---------------------------------------------------------------------------------------

fn corpus_c01() -> Prog {
    Prog {
        module: "c01_units",
        src: read("examples/corpus/core/c01_units.neuron"),
        funs: vec![
            Fun {
                name: "travel_time",
                kind: Kind::Real("s", 1.0),
                calls: vec![
                    vec![raw("300 mm", "0.3"), raw("100 mm/s", "0.1")],
                    vec![raw("1 m", "1.0"), raw("250 mm/s", "0.25")],
                    vec![raw("300 mm", "0.3"), raw("0 mm/s", "0.0")],
                ],
            },
            Fun {
                name: "stroke",
                kind: Kind::Real("mm", 0.001),
                calls: vec![vec![raw("25 mm", "0.025")], vec![raw("10 mm", "0.01")]],
            },
        ],
    }
}

fn corpus_c02() -> Prog {
    let mode = |m: &str| raw(&format!("Mode.{m}"), &format!("c02_enum_match_Mode_{m}"));
    Prog {
        module: "c02_enum_match",
        src: read("examples/corpus/core/c02_enum_match.neuron"),
        funs: vec![Fun {
            name: "speed_limit",
            kind: U,
            calls: vec![vec![mode("Off")], vec![mode("Manual")], vec![mode("Auto")]],
        }],
    }
}

fn corpus_c05() -> Prog {
    let rect = |w: u32, h: u32| {
        raw(
            &format!("Rect(width: {w} mm, height: {h} mm)"),
            &format!(
                "(c05_struct_geometry_Rect){{ .width = {:?}, .height = {:?} }}",
                f64::from(w) / 1000.0,
                f64::from(h) / 1000.0
            ),
        )
    };
    Prog {
        module: "c05_struct_geometry",
        src: read("examples/corpus/core/c05_struct_geometry.neuron"),
        funs: vec![
            Fun {
                name: "area",
                kind: Kind::Real("mm^2", 1e-6),
                calls: vec![vec![rect(40, 25)], vec![rect(100, 100)]],
            },
            Fun {
                name: "fits",
                kind: Kind::Bool,
                calls: vec![
                    vec![rect(40, 25), rect(100, 100)],
                    vec![rect(100, 100), rect(40, 25)],
                    vec![rect(40, 25), rect(40, 25)],
                ],
            },
        ],
    }
}

fn lib_bits() -> Prog {
    Prog {
        module: "std.core.bits",
        src: read("lib/core/bits.neuron"),
        funs: vec![
            ints(
                "bit_get",
                Kind::Bool,
                &[&[0b1010, 1], &[0b1010, 0], &[4_294_967_295, 31], &[1, 32]],
            ),
            ints("bit_set", U, &[&[0, 3], &[5, 0], &[0, 31], &[0, 40]]),
            ints(
                "bit_clear",
                U,
                &[&[0b1111, 2], &[0, 5], &[4_294_967_295, 31]],
            ),
            ints(
                "popcount",
                U,
                &[&[0], &[0b1011], &[4_294_967_295], &[2_147_483_648]],
            ),
        ],
    }
}

fn lib_math() -> Prog {
    let real = Kind::Real("", 1.0);
    Prog {
        module: "std.core.math",
        src: read("lib/core/math.neuron"),
        funs: vec![
            Fun {
                name: "lerp",
                kind: real,
                calls: vec![
                    vec![r(0.0), r(10.0), r(0.0)],
                    vec![r(0.0), r(10.0), r(1.0)],
                    vec![r(0.0), r(10.0), r(0.25)],
                ],
            },
            Fun {
                name: "map_range",
                kind: real,
                calls: vec![
                    vec![r(5.0), r(0.0), r(10.0), r(0.0), r(100.0)],
                    vec![r(15.0), r(10.0), r(20.0), r(100.0), r(0.0)],
                    vec![r(1.0), r(2.0), r(2.0), r(0.0), r(1.0)],
                ],
            },
            ints("abs_diff", U, &[&[3, 10], &[10, 3], &[0, 4_000_000_000]]),
            Fun {
                name: "between",
                kind: Kind::Bool,
                calls: vec![
                    vec![r(5.0), r(0.0), r(10.0)],
                    vec![r(10.0), r(0.0), r(10.0)],
                    vec![r(11.0), r(0.0), r(10.0)],
                ],
            },
        ],
    }
}

#[test]
fn the_core_corpus_programs_match_the_interpreter() {
    let (calls, faults) = totals(&[
        check(&corpus_c01()),
        check(&corpus_c02()),
        check(&corpus_c05()),
    ]);
    assert!(
        calls == 0 || (calls >= 13 && faults >= 1),
        "{calls} {faults}"
    );
}

#[test]
fn the_core_library_modules_match_the_interpreter() {
    let (calls, faults) = totals(&[check(&lib_bits()), check(&lib_math())]);
    assert!(
        calls == 0 || (calls >= 20 && faults >= 3),
        "{calls} {faults}"
    );
}

// ---------------------------------------------------------------------------------------
// Report, maps, determinism
// ---------------------------------------------------------------------------------------

#[test]
fn the_package_has_header_runtime_maps_and_a_fidelity_report() {
    let p = gcd();
    let pkg = emit_c(p.module, &p.src);
    let files = pkg.all_files();
    for n in [
        "t_gcd.c",
        "t_gcd.h",
        "nrt_core.h",
        "nrt_core.c",
        "fidelity.json",
        "wiring.md",
        "t_gcd.c.map.json",
    ] {
        assert!(files.contains_key(n), "{n} is missing: {:?}", files.keys());
    }
    let h = &files["t_gcd.h"];
    assert!(
        h.contains("uint32_t t_gcd_gcd(uint32_t a, uint32_t b);"),
        "{h}"
    );
    assert!(
        h.contains("uint32_t t_gcd_lcm(uint32_t a, uint32_t b);"),
        "{h}"
    );
    assert!(files["wiring.md"].contains("nrt_fault"));
    let map = &pkg.maps["t_gcd.c"];
    assert!(map.mappings.len() >= 4, "{}", map.mappings.len());
    assert!(map.mappings.iter().all(|m| m.src.file == "t_gcd.neuron"));
    let f = &pkg.fidelity;
    assert!(f.entries.iter().any(|e| {
        e.class == Class::Equivalent
            && e.construct == "checked integer arithmetic"
            && e.note.contains("nrt_fault")
    }));
    assert_eq!(f.summary.unsupported, 0);
    assert!(f.summary.exact >= 2, "{:?}", f.summary);

    // Units: erased to base SI values, and the report says it.
    let c01 = emit_c("c01_units", &read("examples/corpus/core/c01_units.neuron"));
    assert!(c01
        .fidelity
        .entries
        .iter()
        .any(|e| e.construct == "units" && e.note.contains("base SI")));
    let c = &c01.all_files()["c01_units.c"];
    assert!(
        c.contains("double c01_units_travel_time(double distance, double speed)"),
        "{c}"
    );
}

#[test]
fn the_output_is_deterministic() {
    for p in [intmath(), bits(), structs(), enums()] {
        let a = emit_c(p.module, &p.src).all_files();
        let b = emit_c(p.module, &p.src).all_files();
        assert_eq!(a, b, "{}", p.module);
    }
}

#[test]
fn emitted_c_is_c99_pedantic_clean_without_a_driver() {
    let Some(cc) = cc() else {
        return;
    };
    let mut names = Vec::new();
    let dir = scratch("clean");
    for p in [
        intmath(),
        sat(),
        gcd(),
        bits(),
        enums(),
        structs(),
        calls(),
        reals(),
    ] {
        let pkg = emit_c(p.module, &p.src);
        pkg.write(&dir).unwrap();
        names.push(format!("{}.c", p.module));
    }
    names.push("nrt_core.c".into());
    for n in &names {
        let o = Command::new(&cc)
            .current_dir(&dir)
            .args([
                "-std=c99",
                "-pedantic",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-c",
                "-o",
            ])
            .arg(format!("{n}.o"))
            .arg(n)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{n}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

// ---------------------------------------------------------------------------------------
// Round trip through the C importer
// ---------------------------------------------------------------------------------------

fn plain() -> Prog {
    let src = "## The part of the subset that needs no checked arithmetic.
module t_plain for core

pub fn clamp_i(x: i32, lo: i32, hi: i32) -> i32:
  if x < lo:
    return lo
  if x > hi:
    return hi
  return x

pub fn in_range(x: i32, lo: i32, hi: i32) -> bool = lo <= x and x < hi

pub fn mix(a: u32, b: u32) -> u32 = (a & b) | (a ^ b)

pub fn median_of_ends(a: i32, b: i32, c: i32) -> i32:
  if a < b:
    return clamp_i(c, a, b)
  return clamp_i(c, b, a)

pub fn sign(x: i32) -> i32:
  if x > 0:
    return 1
  if x < 0:
    return -1
  return 0
";
    let pool = [-1_000_000_000, -17, -1, 0, 1, 5, 99, 1_000_000_000];
    let mut triples = Vec::new();
    for a in pool {
        for b in [-5, 0, 7] {
            for c in [-100, 3, 100] {
                triples.push(vec![a, b, c]);
            }
        }
    }
    Prog {
        module: "t_plain",
        src: src.into(),
        funs: vec![
            from_rows("clamp_i", S, triples.clone()),
            from_rows("in_range", Kind::Bool, triples.clone()),
            from_rows("median_of_ends", S, triples),
            from_rows(
                "mix",
                U,
                square(&[0, 1, 255, 65_535, 4_294_967_295, 123_456_789]),
            ),
            from_rows("sign", S, pool.iter().map(|v| vec![*v]).collect()),
        ],
    }
}

#[test]
fn emitted_c_of_a_subset_program_imports_back_with_the_same_results() {
    let p = plain();
    let pkg = emit_c(p.module, &p.src);
    let c = pkg.all_files()["t_plain.c"].clone();
    // The importer reads one self-contained file: the standard headers replace the
    // generated one.
    let text = format!(
        "#include <stdint.h>\n#include <stdbool.h>\n{}",
        c.replace("#include \"t_plain.h\"\n", "")
    );
    let imp = match neuron_import::import("c", "t_plain_rt", &text) {
        Ok(i) => i,
        Err(d) => panic!("{d:?}\n{text}"),
    };
    assert!(imp.complete(), "{:?}\n{text}\n{}", imp.diags, imp.source);
    let Some(cc) = cc() else {
        return;
    };
    let res = c_results(&cc, &pkg, &p, "rt");
    assert!(res.iter().all(|x| matches!(x, Res::Val(_))));
    let rename = |n: &str| format!("t_plain_{n}");
    let (calls, _) = compare(&p, &imp.source, &rename, &res);
    eprintln!("round trip C -> import -> interpreter: {calls} calls equal");
    assert!(calls >= 200, "{calls}");
}

// ---------------------------------------------------------------------------------------
// NE0610
// ---------------------------------------------------------------------------------------

#[test]
fn constructs_outside_the_subset_are_ne0610_with_a_location() {
    let cases: Vec<(&str, String)> = vec![
        (
            "option and result",
            read("examples/corpus/core/c03_option_result.neuron"),
        ),
        (
            "class with contracts",
            read("examples/corpus/core/c04_class_contract.neuron"),
        ),
        (
            "array parameter",
            "module m for core\n\npub fn first(a: [u8; 4]) -> u8 = a[0]\n".into(),
        ),
        (
            "tuple",
            "module m for core\n\npub fn pair(a: u8) -> (u8, u8) = (a, a)\n".into(),
        ),
    ];
    for (label, text) in &cases {
        let nir = nir_of("m", text);
        let err = emitter("c")
            .unwrap()
            .emit(&nir, &EmitOptions::default())
            .expect_err(label);
        assert!(!err.is_empty(), "{label}");
        for d in &err {
            assert_eq!(d.code, "NE0610", "{label}: {d:?}");
            assert!(d.span[1] > d.span[0], "{label}: no location: {d:?}");
            assert!(d.file.ends_with("m.neuron"), "{label}: {d:?}");
            assert!(d.help.is_some(), "{label}");
        }
    }
    // Not a `core` module at all.
    let nir = nir_of("p01", &read("examples/corpus/plc/p01_start_stop.neuron"));
    let err = emitter("c")
        .unwrap()
        .emit(&nir, &EmitOptions::default())
        .expect_err("plc");
    assert_eq!(err[0].code, "NE0610");
}
