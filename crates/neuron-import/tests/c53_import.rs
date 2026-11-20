//! Task C5.3: the C and Verilog importers.
//!
//! C: hand-written files with bounded loops, saturating arithmetic, bit manipulation and calls
//! between functions are imported, analyzed, and every function is run in the reference
//! interpreter on a table of inputs; the results must equal the results of the same C
//! compiled and run by the host `gcc` (`NEURON_CC`, or `gcc` on `PATH`; a missing compiler
//! is an error when `NEURON_REQUIRE_TOOLCHAIN` is set and a skip otherwise). A second test
//! compares the same functions with a Rust reference, so there is always an independent check.
//!
//! Verilog: the Verilog that `neuron-codegen-hdl` writes for every `hdl` program of the corpus
//! is imported back, analyzed and emitted again; the re-emitted Verilog equals the original
//! and both circuits give the same output waveform in a cycle simulator of the NIR circuits
//! (no Verilog simulator is needed). Hand-written modules (a counter with enable and
//! synchronous reset, an edge detector, a combinational ALU with `case`) are checked against
//! models written in Rust.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use neuron_codegen_hdl::emit_verilog;
use neuron_import::{import, Class, Imported};
use neuron_interp::{run_tests, Options};
use neuron_nir as n;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn nir_of(name: &str, text: &str) -> n::Nir {
    let a = analyze(
        vec![SourceFile {
            path: format!("{name}.neuron"),
            text: text.into(),
        }],
        None,
        &SemaOptions::default(),
    );
    a.nir.clone().unwrap_or_else(|| {
        let head: Vec<&str> = text.lines().take(120).collect();
        panic!("{name}:\n{}\n---\n{}", a.render(), head.join("\n"))
    })
}

fn imported(lang: &str, name: &str, text: &str) -> Imported {
    match import(lang, name, text) {
        Ok(i) => i,
        Err(d) => panic!("{name}: {d:?}\n{text}"),
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("neuron-c53-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Deterministic generator for the tables and the stimulus.
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
// C
// ---------------------------------------------------------------------------------------

const MATH_C: &str = r#"#include <stdint.h>
#include <stdbool.h>

#define LIMIT 100
const uint32_t SEED = 7;

/* Euclid's algorithm. */
uint32_t gcd(uint32_t a, uint32_t b) {
    while (b != 0) /* max 64 */ {
        uint32_t t = a % b;
        a = b;
        b = t;
    }
    return a;
}

uint32_t lcm(uint32_t a, uint32_t b) {
    if (a == 0 || b == 0) {
        return 0;
    }
    return a / gcd(a, b) * b;
}

uint32_t fib(uint32_t n) {
    uint32_t a = 0;
    uint32_t b = 1;
    uint32_t i = 0;
    while (i < n) /* max 47 */ {
        uint32_t t = a + b;
        a = b;
        b = t;
        i++;
    }
    return a;
}

uint32_t sat_add(uint32_t a, uint32_t b) {
    uint32_t s = a + b;
    if (s < a) {
        return 4294967295u;
    }
    return s;
}

uint8_t sat_add8(uint8_t a, uint8_t b) {
    uint8_t s = a + b;
    if (s < a) {
        return 255;
    }
    return s;
}

int32_t clamp(int32_t x, int32_t lo, int32_t hi) {
    if (x < lo) return lo;
    if (x > hi) return hi;
    return x;
}

uint32_t rotl(uint32_t x, uint32_t n) {
    n = n & 31;
    return (x << n) | (x >> ((32 - n) & 31));
}

uint32_t popcount(uint32_t x) {
    uint32_t c = 0;
    for (int i = 0; i < 32; i++) {
        if (((x >> i) & 1) == 1) {
            c++;
        }
    }
    return c;
}

uint32_t parity(uint32_t x) {
    uint32_t p = 0;
    while (x != 0) /* max 32 */ {
        p = p ^ (x & 1);
        x = x >> 1;
    }
    return p;
}

uint32_t hash8(uint32_t seed) {
    uint32_t h = seed;
    for (int i = 0; i < 8; i++) {
        h = h * 33 + 7;
        if (h & 1) {
            h = h ^ 0xA5A5A5A5;
        }
    }
    return h;
}

uint32_t mix(uint32_t x) {
    return (x ^ SEED) * 31;
}

int32_t abs_diff(int32_t a, int32_t b) {
    return a > b ? a - b : b - a;
}

bool in_range(int32_t x) {
    return x >= 0 && x < LIMIT;
}

int32_t sign(int32_t x) {
    if (x > 0) {
        return 1;
    } else if (x < 0) {
        return -1;
    } else {
        return 0;
    }
}

int32_t sum_to(int32_t n) {
    int32_t s = 0;
    for (int i = 1; i <= n; i++) /* max 1000 */ {
        s += i;
    }
    return s;
}

int32_t first_multiple(int32_t n, int32_t k) {
    int32_t r = -1;
    for (int m = 1; m <= 100; m++) {
        if (m % k != 0) {
            continue;
        }
        if (m > n) {
            r = m;
            break;
        }
    }
    return r;
}
"#;

const U32: &[i64] = &[
    0,
    1,
    2,
    3,
    7,
    12,
    18,
    255,
    256,
    1000,
    65_535,
    65_536,
    123_456,
    2_147_483_647,
    2_147_483_648,
    4_294_967_295,
];
const I32: &[i64] = &[
    -1_000_000_000,
    -1000,
    -17,
    -1,
    0,
    1,
    5,
    99,
    100,
    101,
    1000,
    1_000_000_000,
];
const U8: &[i64] = &[0, 1, 2, 100, 127, 128, 200, 254, 255];
const FIB_N: &[i64] = &[0, 1, 2, 3, 5, 10, 20, 30, 40, 46, 47];
const SUM_N: &[i64] = &[0, 1, 2, 10, 100, 999, 1000];
const COUNT: &[i64] = &[0, 1, 2, 5, 13, 31, 32, 33, 63];
const MULTIPLE_N: &[i64] = &[-5, 0, 1, 10, 50, 99, 100, 150];
const MULTIPLE_K: &[i64] = &[1, 2, 3, 7, 10, 13, 50, 100];

/// A function of the C file: C types of the arguments, argument pools, `bool` result.
struct Func {
    name: &'static str,
    types: &'static [&'static str],
    pools: &'static [&'static [i64]],
    boolean: bool,
}

const FUNCS: &[Func] = &[
    Func {
        name: "gcd",
        types: &["uint32_t", "uint32_t"],
        pools: &[U32, U32],
        boolean: false,
    },
    Func {
        name: "lcm",
        types: &["uint32_t", "uint32_t"],
        pools: &[&[0, 1, 4, 6, 21, 100, 255], &[0, 1, 6, 10, 35, 77]],
        boolean: false,
    },
    Func {
        name: "fib",
        types: &["uint32_t"],
        pools: &[FIB_N],
        boolean: false,
    },
    Func {
        name: "sat_add",
        types: &["uint32_t", "uint32_t"],
        pools: &[U32, U32],
        boolean: false,
    },
    Func {
        name: "sat_add8",
        types: &["uint8_t", "uint8_t"],
        pools: &[U8, U8],
        boolean: false,
    },
    Func {
        name: "clamp",
        types: &["int32_t", "int32_t", "int32_t"],
        pools: &[I32, &[-100, 0, 5], &[10, 100, 1000]],
        boolean: false,
    },
    Func {
        name: "rotl",
        types: &["uint32_t", "uint32_t"],
        pools: &[U32, COUNT],
        boolean: false,
    },
    Func {
        name: "popcount",
        types: &["uint32_t"],
        pools: &[U32],
        boolean: false,
    },
    Func {
        name: "parity",
        types: &["uint32_t"],
        pools: &[U32],
        boolean: false,
    },
    Func {
        name: "hash8",
        types: &["uint32_t"],
        pools: &[U32],
        boolean: false,
    },
    Func {
        name: "mix",
        types: &["uint32_t"],
        pools: &[U32],
        boolean: false,
    },
    Func {
        name: "abs_diff",
        types: &["int32_t", "int32_t"],
        pools: &[I32, I32],
        boolean: false,
    },
    Func {
        name: "in_range",
        types: &["int32_t"],
        pools: &[I32],
        boolean: true,
    },
    Func {
        name: "sign",
        types: &["int32_t"],
        pools: &[I32],
        boolean: false,
    },
    Func {
        name: "sum_to",
        types: &["int32_t"],
        pools: &[SUM_N],
        boolean: false,
    },
    Func {
        name: "first_multiple",
        types: &["int32_t", "int32_t"],
        pools: &[MULTIPLE_N, MULTIPLE_K],
        boolean: false,
    },
];

/// The argument rows of a function: the whole product when it is small, else 60 draws.
fn rows(f: &Func) -> Vec<Vec<i64>> {
    let total: usize = f.pools.iter().map(|p| p.len()).product();
    let mut out = Vec::new();
    if total <= 80 {
        for k in 0..total {
            let mut rest = k;
            let mut row = Vec::new();
            for p in f.pools {
                row.push(p[rest % p.len()]);
                rest /= p.len();
            }
            out.push(row);
        }
    } else {
        let mut rng = Lcg(0x5EED ^ f.name.len() as u64);
        for _ in 0..60 {
            out.push(
                f.pools
                    .iter()
                    .map(|p| p[rng.draw() as usize % p.len()])
                    .collect(),
            );
        }
    }
    out
}

fn call_text(f: &Func, row: &[i64]) -> String {
    format!(
        "{}({})",
        f.name,
        row.iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Neuron `test` blocks that call every function on the table and compare the results.
fn test_blocks(expected: &BTreeMap<&str, Vec<i64>>) -> String {
    let mut out = String::new();
    for f in FUNCS {
        out.push_str(&format!("\ntest \"{}\":\n", f.name));
        for (row, want) in rows(f).iter().zip(&expected[f.name]) {
            let call = call_text(f, row);
            let want = if f.boolean {
                (*want != 0).to_string()
            } else {
                want.to_string()
            };
            out.push_str(&format!("  assert {call} == {want}, \"{call}\"\n"));
        }
    }
    out
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

/// Compiles the C file with a `main` that prints every result, and returns the results.
fn gcc_results(cc: &str) -> BTreeMap<&'static str, Vec<i64>> {
    let mut main = String::from("#include <stdio.h>\n");
    main.push_str(MATH_C);
    main.push_str("\nint main(void) {\n");
    for f in FUNCS {
        for row in rows(f) {
            let args: Vec<String> = f
                .types
                .iter()
                .zip(&row)
                .map(|(t, v)| format!("({t})({v}LL)"))
                .collect();
            main.push_str(&format!(
                "  printf(\"{} %lld\\n\", (long long){}({}));\n",
                f.name,
                f.name,
                args.join(", ")
            ));
        }
    }
    main.push_str("  return 0;\n}\n");
    let dir = scratch("gcc");
    std::fs::write(dir.join("drv.c"), main).unwrap();
    let exe = dir.join("drv.exe");
    let out = Command::new(cc)
        .current_dir(&dir)
        .args(["-O0", "-o"])
        .arg(&exe)
        .arg("drv.c")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "gcc failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success(), "the C program failed");
    let mut res: BTreeMap<&'static str, Vec<i64>> = BTreeMap::new();
    for line in String::from_utf8_lossy(&run.stdout).lines() {
        let (name, v) = line.split_once(' ').unwrap();
        let f = FUNCS.iter().find(|f| f.name == name).unwrap();
        res.entry(f.name).or_default().push(v.parse().unwrap());
    }
    res
}

/// Runs the test blocks and returns the number of assertions that were run.
fn run_table(imp: &Imported, expected: &BTreeMap<&str, Vec<i64>>) -> usize {
    let src = format!("{}{}", imp.source, test_blocks(expected));
    let nir = nir_of("math", &src);
    let reports = run_tests(&nir, &Options::default());
    assert_eq!(reports.len(), FUNCS.len(), "one test per function");
    for r in &reports {
        assert!(r.passed, "test `{}` failed: {}", r.name, r.message);
    }
    FUNCS.iter().map(|f| rows(f).len()).sum()
}

#[test]
fn imported_c_functions_give_the_results_of_gcc() {
    let imp = imported("c", "math", MATH_C);
    assert!(imp.complete(), "{:?}", imp.diags);
    for f in FUNCS {
        assert!(
            imp.source.contains(&format!("fn {}(", f.name)),
            "{} is missing:\n{}",
            f.name,
            imp.source
        );
    }
    let Some(cc) = cc() else {
        return;
    };
    let want = gcc_results(&cc);
    let n = run_table(&imp, &want);
    eprintln!(
        "C round trip: {} functions, {n} inputs compared with gcc ({cc})",
        FUNCS.len()
    );
    assert!(n >= 400, "{n}");
}

// Rust reference of the functions, for the check that needs no compiler.
fn reference(name: &str, a: &[i64]) -> i64 {
    let u = |i: usize| a[i] as u32;
    match name {
        "gcd" => {
            let (mut x, mut y) = (u(0), u(1));
            while y != 0 {
                (x, y) = (y, x % y);
            }
            i64::from(x)
        }
        "lcm" => {
            let (x, y) = (u(0), u(1));
            if x == 0 || y == 0 {
                return 0;
            }
            let (mut p, mut q) = (x, y);
            while q != 0 {
                (p, q) = (q, p % q);
            }
            i64::from((x / p).wrapping_mul(y))
        }
        "fib" => {
            let (mut x, mut y) = (0u32, 1u32);
            for _ in 0..u(0) {
                (x, y) = (y, x.wrapping_add(y));
            }
            i64::from(x)
        }
        "sat_add" => i64::from(u(0).saturating_add(u(1))),
        "sat_add8" => i64::from((a[0] as u8).saturating_add(a[1] as u8)),
        "clamp" => {
            if a[0] < a[1] {
                a[1]
            } else if a[0] > a[2] {
                a[2]
            } else {
                a[0]
            }
        }
        "rotl" => i64::from(u(0).rotate_left(u(1) & 31)),
        "popcount" => i64::from(u(0).count_ones()),
        "parity" => i64::from(u(0).count_ones() & 1),
        "hash8" => {
            let mut h = u(0);
            for _ in 0..8 {
                h = h.wrapping_mul(33).wrapping_add(7);
                if h & 1 == 1 {
                    h ^= 0xA5A5_A5A5;
                }
            }
            i64::from(h)
        }
        "mix" => i64::from((u(0) ^ 7).wrapping_mul(31)),
        "abs_diff" => (a[0] - a[1]).abs(),
        "in_range" => i64::from((0..100).contains(&a[0])),
        "sign" => a[0].signum(),
        "sum_to" => (1..=a[0]).sum::<i64>(),
        "first_multiple" => (1..=100).find(|m| m % a[1] == 0 && *m > a[0]).unwrap_or(-1),
        other => panic!("no reference for {other}"),
    }
}

#[test]
fn imported_c_functions_give_the_results_of_the_rust_reference() {
    let imp = imported("c", "math", MATH_C);
    let mut want: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for f in FUNCS {
        let v: Vec<i64> = rows(f).iter().map(|r| reference(f.name, r)).collect();
        want.insert(f.name, v);
    }
    let n = run_table(&imp, &want);
    eprintln!("C round trip: {n} inputs compared with the Rust reference");
}

#[test]
fn the_c_import_is_deterministic_formatted_and_reported() {
    let a = imported("c", "math", MATH_C);
    let b = imported("c", "math", MATH_C);
    assert_eq!(a.source, b.source);
    let again = neuron_syntax::format::format_source(&a.source).unwrap();
    assert_eq!(again.text, a.source, "the import is already formatted");
    assert!(a.source.starts_with("## Imported from C"));
    assert!(a.source.contains("module math for core"));
    assert!(a.source.contains("const LIMIT = 100"));
    assert!(a.source.contains("const SEED: u32 = 7"));
    assert!(
        a.source.contains("while b_v != 0 max 64:"),
        "bound comment:\n{}",
        a.source
    );
    assert!(
        a.source.contains("while i < 32 max 32:"),
        "constant range:\n{}",
        a.source
    );
    assert!(
        a.source.contains("while m <= 100 max 100:"),
        "inclusive range with continue:\n{}",
        a.source
    );
    assert_eq!(a.fidelity.summary.exact as usize, 2 + FUNCS.len());
    assert!(a
        .fidelity
        .entries
        .iter()
        .any(|e| e.class == Class::Equivalent && e.construct == "integer arithmetic"));
    assert_eq!(a.fidelity.summary.unsupported, 0);
}

#[test]
fn c_constructs_without_a_neuron_equivalent_are_ne1002() {
    let good = "uint32_t good(uint32_t x) { return x + 1; }\n";
    let cases: &[(&str, &str, usize)] = &[
        ("pointer", "uint32_t first(const uint32_t *p) { return *p; }", 1),
        ("array", "uint32_t table[4];", 1),
        ("local array", "uint32_t f(void) { uint32_t a[3]; return 0; }", 1),
        ("struct", "struct P { int x; int y; };", 1),
        ("goto", "uint32_t f(uint32_t x) { goto end; end: return x; }", 1),
        ("switch", "uint32_t f(uint32_t x) { switch (x) { case 1: return 2; default: return 3; } }", 1),
        ("float", "float half(float x) { return x; }", 1),
        ("unbounded while", "uint32_t f(uint32_t x) { while (x > 3) { x = x - 1; } return x; }", 1),
        ("recursion", "uint32_t f(uint32_t n) { return n == 0 ? 0 : f(n - 1); }", 1),
        ("void function", "void f(void) { }", 1),
        ("narrowing cast", "uint8_t f(uint32_t x) { return (uint8_t)x; }", 1),
        ("mutable global", "int counter = 0;", 1),
        ("function-like macro", "#define SQ(x) ((x) * (x))", 1),
        ("conditional compilation", "#ifdef DEBUG\n#endif", 2),
        ("do while", "uint32_t f(uint32_t x) { do { x = x + 1; } while (x < 5); return x; }", 1),
        ("unknown call", "uint32_t f(uint32_t x) { return abs(x); }", 1),
        ("mixed signedness", "int32_t f(int32_t a, uint32_t b) { return a + b; }", 1),
        (
            "caller of a function that failed",
            "uint32_t a(uint32_t x) { goto L; L: return x; }\nuint32_t b(uint32_t x) { return a(x); }",
            2,
        ),
        ("for without a bound", "uint32_t f(uint32_t n) { uint32_t s = 0; for (uint32_t i = 0; i < n; i++) { s = s + i; } return s; }", 1),
    ];
    for (label, bad, count) in cases {
        let text = format!("#include <stdint.h>\n{bad}\n{good}");
        let imp = imported("c", "m", &text);
        assert_eq!(imp.diags.len(), *count, "{label}: {:?}", imp.diags);
        assert!(imp.diags.iter().all(|d| d.code == "NE1002"), "{label}");
        assert!(
            imp.diags.iter().all(|d| d.span[1] > d.span[0]),
            "{label}: spans"
        );
        assert_eq!(
            imp.fidelity.summary.unsupported as usize, *count,
            "{label}: fidelity"
        );
        assert!(imp
            .fidelity
            .entries
            .iter()
            .any(|e| e.class == Class::Unsupported));
        assert!(
            imp.source.contains("fn good("),
            "{label}: the rest is imported:\n{}",
            imp.source
        );
        nir_of("m", &imp.source);
    }
}

#[test]
fn c_syntax_errors_are_ne1001() {
    for (label, text) in [
        ("missing parameter list", "uint32_t f( { return 1; }"),
        ("missing semicolon", "uint32_t f(uint32_t a) { return a }"),
        (
            "unterminated comment",
            "uint32_t f(void) { return 1; } /* open",
        ),
        ("unclosed brace", "uint32_t f(uint32_t a) { return a;"),
        ("stray character", "uint32_t f(void) { return 1 @ 2; }"),
    ] {
        let err = import("c", "m", text).unwrap_err();
        assert_eq!(err[0].code, "NE1001", "{label}");
    }
}

#[test]
fn the_c_that_neuron_emits_is_firmware_outside_the_subset() {
    // `neuron emit-c` writes a whole firmware image (runtime tables, structs, pointers, an
    // `#include "nrt.h"`), and there is no C emitter for plain `core` functions yet (the C99
    // kernel of task C6 is not written), so the corpus `core` programs have no C counterpart
    // to round trip. The importer must still read such a file without failing: everything
    // outside the subset is `NE1002`.
    let text = std::fs::read_to_string(root().join("conformance/golden/c/blink_mcu.c")).unwrap();
    let imp = imported("c", "blink_mcu", &text.replace("\r\n", "\n"));
    assert!(imp.diags.len() >= 10, "{}", imp.diags.len());
    assert!(imp.diags.iter().all(|d| d.code == "NE1002"));
    assert!(!imp.source.contains("fn main"));
    nir_of("blink_mcu", &imp.source);
}

// ---------------------------------------------------------------------------------------
// Verilog
// ---------------------------------------------------------------------------------------

/// Width of a written type, as the Verilog backend reads it.
fn type_width(t: &n::TypeExpr) -> u32 {
    let n::TypeExprKind::Named { path, args } = &t.kind else {
        panic!("unexpected type");
    };
    match path.as_str() {
        "bool" => 1,
        _ => match args.first() {
            None => 1,
            Some(n::TypeArg::Expr { expr }) => match &expr.e {
                n::ExprKind::Lit {
                    lit: n::Lit::Int { v, .. },
                } => v.parse().unwrap(),
                other => panic!("width {other:?}"),
            },
            Some(other) => panic!("type argument {other:?}"),
        },
    }
}

fn bits_value(text: &str) -> i128 {
    let t = text.replace('_', "");
    let (_, rest) = t.split_once('\'').unwrap();
    let (base, digits) = rest.split_at(1);
    let radix = match base {
        "b" | "B" => 2,
        "o" | "O" => 8,
        "d" | "D" => 10,
        _ => 16,
    };
    i128::from_str_radix(digits, radix).unwrap()
}

fn wid(e: &n::Expr) -> u32 {
    match &e.ty {
        n::Type::Bits { width, .. } => *width,
        n::Type::Bool => 1,
        _ => 0,
    }
}

fn mask(v: i128, w: u32) -> i128 {
    if w == 0 || w >= 120 {
        v
    } else {
        v & ((1i128 << w) - 1)
    }
}

fn place_name(e: &n::Expr) -> String {
    match &e.e {
        n::ExprKind::Local { name, .. }
        | n::ExprKind::Ref { name, .. }
        | n::ExprKind::Global { name } => name.clone(),
        other => panic!("place {other:?}"),
    }
}

fn written(b: &n::Block, out: &mut BTreeSet<String>) {
    for s in &b.stmts {
        match &s.s {
            n::StmtKind::Assign { place, .. } => {
                out.insert(place_name(place));
            }
            n::StmtKind::If {
                branches,
                otherwise,
            } => {
                for br in branches {
                    written(&br.body, out);
                }
                if let Some(o) = otherwise {
                    written(o, out);
                }
            }
            _ => {}
        }
    }
}

/// A cycle simulator of one NIR circuit. It follows the Verilog backend: a circuit with a
/// 1-bit input called `rst` or `reset` has a synchronous reset of the registers that its
/// clocked blocks write.
struct Sim<'a> {
    c: &'a n::Circuit,
    widths: BTreeMap<String, u32>,
    vals: BTreeMap<String, i128>,
    consts: BTreeMap<String, i128>,
    inits: BTreeMap<String, i128>,
}

impl<'a> Sim<'a> {
    fn new(c: &'a n::Circuit) -> Sim<'a> {
        let mut s = Sim {
            c,
            widths: BTreeMap::new(),
            vals: BTreeMap::new(),
            consts: BTreeMap::new(),
            inits: BTreeMap::new(),
        };
        for p in &c.ports {
            s.widths.insert(p.name.clone(), type_width(&p.ty));
            s.vals.insert(p.name.clone(), 0);
        }
        for m in &c.members {
            match &m.def {
                n::CircuitDef::Reg { name, ty, init, .. } => {
                    s.widths.insert(name.clone(), type_width(ty));
                    let v = init.as_ref().map_or(0, |e| s.ev(e));
                    if init.is_some() {
                        s.inits.insert(name.clone(), v);
                    }
                    s.vals.insert(name.clone(), v);
                }
                n::CircuitDef::Wire { name, ty, .. } => {
                    s.widths.insert(name.clone(), type_width(ty));
                    s.vals.insert(name.clone(), 0);
                }
                n::CircuitDef::Const { name, value, .. } => {
                    let v = s.ev(value);
                    s.consts.insert(name.clone(), v);
                }
                _ => {}
            }
        }
        s
    }

    fn get(&self, name: &str) -> i128 {
        self.consts
            .get(name)
            .or_else(|| self.vals.get(name))
            .copied()
            .unwrap_or_else(|| panic!("sim: unknown name {name}"))
    }

    fn ev(&self, e: &n::Expr) -> i128 {
        use n::ExprKind as X;
        let r = match &e.e {
            X::Lit { lit } => match lit {
                n::Lit::Int { v, .. } => v.parse().unwrap(),
                n::Lit::Bits { text } => bits_value(text),
                n::Lit::Bool { v } => i128::from(*v),
                other => panic!("sim: literal {other:?}"),
            },
            X::Local { name, .. } | X::Ref { name, .. } | X::Global { name } => self.get(name),
            X::Unary { op, operand } => {
                let x = self.ev(operand);
                match op.as_str() {
                    "~" => !x,
                    "not" => {
                        if matches!(operand.ty, n::Type::Bool) {
                            i128::from(x == 0)
                        } else {
                            !x
                        }
                    }
                    "-" => -x,
                    other => panic!("sim: unary {other}"),
                }
            }
            X::Binary { op, lhs, rhs } => {
                let (a, b) = (self.ev(lhs), self.ev(rhs));
                let wide = wid(lhs) > 1 || wid(rhs) > 1;
                match op.as_str() {
                    "+" | "+%" => a + b,
                    "-" | "-%" => a - b,
                    "*" | "*%" => a * b,
                    "/" => {
                        if b == 0 {
                            0
                        } else {
                            a / b
                        }
                    }
                    "%" => {
                        if b == 0 {
                            0
                        } else {
                            a % b
                        }
                    }
                    "&" => a & b,
                    "|" => a | b,
                    "^" | "xor" => a ^ b,
                    "<<" => a << b,
                    ">>" => a >> b,
                    "==" => i128::from(a == b),
                    "!=" => i128::from(a != b),
                    "<" => i128::from(a < b),
                    "<=" => i128::from(a <= b),
                    ">" => i128::from(a > b),
                    ">=" => i128::from(a >= b),
                    "and" => {
                        if wide {
                            a & b
                        } else {
                            i128::from(a != 0 && b != 0)
                        }
                    }
                    "or" => {
                        if wide {
                            a | b
                        } else {
                            i128::from(a != 0 || b != 0)
                        }
                    }
                    "++" => (a << wid(rhs)) | b,
                    other => panic!("sim: binary {other}"),
                }
            }
            X::Compare { first, rest } => {
                let mut prev = self.ev(first);
                let mut ok = true;
                for r in rest {
                    let x = self.ev(&r.rhs);
                    ok &= match r.op.as_str() {
                        "==" => prev == x,
                        "!=" => prev != x,
                        "<" => prev < x,
                        "<=" => prev <= x,
                        ">" => prev > x,
                        ">=" => prev >= x,
                        other => panic!("sim: compare {other}"),
                    };
                    prev = x;
                }
                i128::from(ok)
            }
            X::Index { base, index, end } => {
                let b = self.ev(base);
                let i = self.ev(index);
                match end {
                    Some(lo) => {
                        let l = self.ev(lo);
                        (b >> l) & ((1i128 << (i - l + 1)) - 1)
                    }
                    None => (b >> i) & 1,
                }
            }
            X::IfElse {
                cond,
                then,
                otherwise,
            } => {
                if self.ev(cond) != 0 {
                    self.ev(then)
                } else {
                    self.ev(otherwise)
                }
            }
            X::Cast { operand, .. } => self.ev(operand),
            other => panic!("sim: expression {other:?}"),
        };
        mask(r, wid(e))
    }

    fn exec(&mut self, b: &n::Block, nonblocking: bool, pend: &mut Vec<(String, i128)>) {
        for s in &b.stmts {
            match &s.s {
                n::StmtKind::Pass => {}
                n::StmtKind::Assign {
                    place,
                    op,
                    value,
                    when,
                } => {
                    assert_eq!(op, "=", "sim: compound assignment");
                    if let Some(c) = when {
                        if self.ev(c) == 0 {
                            continue;
                        }
                    }
                    let name = place_name(place);
                    let v = mask(self.ev(value), self.widths[&name]);
                    if nonblocking {
                        pend.push((name, v));
                    } else {
                        self.vals.insert(name, v);
                    }
                }
                n::StmtKind::If {
                    branches,
                    otherwise,
                } => {
                    let mut done = false;
                    for br in branches {
                        if self.ev(&br.cond) != 0 {
                            self.exec(&br.body, nonblocking, pend);
                            done = true;
                            break;
                        }
                    }
                    if !done {
                        if let Some(o) = otherwise {
                            self.exec(o, nonblocking, pend);
                        }
                    }
                }
                other => panic!("sim: statement {other:?}"),
            }
        }
    }

    fn comb(&mut self) {
        let c = self.c;
        for _ in 0..4 {
            for m in &c.members {
                if let n::CircuitDef::Comb { body } = &m.def {
                    self.exec(body, false, &mut Vec::new());
                }
            }
        }
    }

    fn reset_port(&self) -> Option<String> {
        self.c
            .ports
            .iter()
            .find(|p| {
                p.dir == "in"
                    && self.widths[&p.name] == 1
                    && matches!(p.name.as_str(), "rst" | "reset")
            })
            .map(|p| p.name.clone())
    }

    /// One clock cycle: inputs, settle, sample the outputs, clock edge.
    fn cycle(&mut self, inputs: &BTreeMap<String, i128>, outs: &[String]) -> Vec<i128> {
        for (k, v) in inputs {
            if self.widths.contains_key(k) {
                self.vals.insert(k.clone(), mask(*v, self.widths[k]));
            }
        }
        self.comb();
        let sample = outs.iter().map(|o| self.get(o)).collect();
        let reset = self.reset_port().is_some_and(|r| self.vals[&r] == 1);
        let mut pend = Vec::new();
        let c = self.c;
        for m in &c.members {
            if let n::CircuitDef::OnEdge { body, .. } = &m.def {
                if reset {
                    let mut w = BTreeSet::new();
                    written(body, &mut w);
                    for name in w {
                        if let Some(v) = self.inits.get(&name) {
                            pend.push((name, *v));
                        }
                    }
                } else {
                    self.exec(body, true, &mut pend);
                }
            }
        }
        for (k, v) in pend {
            self.vals.insert(k, v);
        }
        self.comb();
        sample
    }
}

fn first_circuit(nir: &n::Nir) -> &n::Circuit {
    for m in &nir.modules {
        for i in &m.items {
            if let n::ItemDef::Circuit { circuit } = &i.def {
                return circuit;
            }
        }
    }
    panic!("no circuit");
}

/// Random stimulus for the input ports of a circuit.
fn stimulus(c: &n::Circuit, cycles: usize, seed: u64) -> Vec<BTreeMap<String, i128>> {
    let mut rng = Lcg(seed);
    (0..cycles)
        .map(|_| {
            c.ports
                .iter()
                .filter(|p| p.dir == "in")
                .map(|p| {
                    let w = type_width(&p.ty);
                    let raw = i128::from(rng.draw());
                    // One-bit inputs are mostly low, so that pulses and frames happen.
                    let v = if w == 1 {
                        i128::from(raw % 4 == 0)
                    } else {
                        mask(raw, w)
                    };
                    (p.name.clone(), v)
                })
                .collect()
        })
        .collect()
}

fn waveform(nir: &n::Nir, stim: &[BTreeMap<String, i128>]) -> Vec<Vec<i128>> {
    let c = first_circuit(nir);
    let outs: Vec<String> = c
        .ports
        .iter()
        .filter(|p| p.dir == "out")
        .map(|p| p.name.clone())
        .collect();
    let mut sim = Sim::new(c);
    stim.iter().map(|s| sim.cycle(s, &outs)).collect()
}

fn hdl_corpus() -> Vec<(String, String)> {
    let mut out = vec![(
        "pwm_hdl".to_string(),
        std::fs::read_to_string(root().join("examples/pwm_hdl.neuron")).unwrap(),
    )];
    let mut files: Vec<PathBuf> = std::fs::read_dir(root().join("examples/corpus/hdl"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    files.sort();
    for f in files {
        out.push((
            f.file_stem().unwrap().to_string_lossy().into_owned(),
            std::fs::read_to_string(f).unwrap(),
        ));
    }
    out.into_iter()
        .map(|(n, t)| (n, t.replace("\r\n", "\n")))
        // A short divisor and a short half period make the programs move within a few cycles.
        .map(|(n, t)| {
            (
                n,
                t.replace("clk.freq / 115200 Bd", "4")
                    .replace("50_000_000", "5"),
            )
        })
        .collect()
}

fn verilog_of(nir: &n::Nir) -> String {
    match emit_verilog(nir) {
        Ok(v) => v.text,
        Err(d) => panic!("{d:?}"),
    }
}

/// The Verilog without its comment lines (they name the source file).
fn code_only(v: &str) -> String {
    v.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_hdl_program_round_trips_through_verilog() {
    let mut total_cycles = 0;
    let mut total_values = 0;
    let programs = hdl_corpus();
    assert_eq!(programs.len(), 6);
    for (name, src) in &programs {
        let original = nir_of(name, src);
        let v1 = verilog_of(&original);
        let imp = imported("verilog", name, &v1);
        assert!(imp.complete(), "{name}: {:?}\n{}", imp.diags, imp.source);
        let back = nir_of(name, &imp.source);
        let v2 = verilog_of(&back);
        assert_eq!(
            code_only(&v2),
            code_only(&v1),
            "{name}: the re-emitted Verilog differs\n{}",
            imp.source
        );
        // The same stimulus gives the same waveform on both circuits.
        let stim = stimulus(first_circuit(&original), 600, 0xC0FFEE);
        let a = waveform(&original, &stim);
        let b = waveform(&back, &stim);
        assert_eq!(a, b, "{name}: the waveforms differ");
        assert!(
            a.iter().any(|row| row != &a[0]) || name == "d03_shift_register",
            "{name}: the outputs never move, the stimulus is too weak"
        );
        total_cycles += a.len();
        total_values += a.iter().map(Vec::len).sum::<usize>();
    }
    eprintln!(
        "Verilog round trip: {} programs, {total_cycles} cycles, {total_values} output values compared",
        programs.len()
    );
}

const COUNTER_V: &str = r"// A 4-bit counter with enable and synchronous reset.
`timescale 1ns/1ps
module counter_en (
  input clk,
  input rst,
  input en,
  output reg [3:0] count
);
  always @(posedge clk) begin
    if (rst)
      count <= 4'd0;
    else if (en)
      count <= count + 1;
  end
endmodule
";

const EDGE_V: &str = r"module rise_detect (input clk, input sig, output pulse);
  reg prev = 1'b0;
  always @(posedge clk) prev <= sig;
  assign pulse = sig & ~prev;
endmodule
";

const ALU_V: &str = r"module alu (
  input clk,
  input [1:0] op,
  input [7:0] a,
  input [7:0] b,
  output reg [7:0] y,
  output zero
);
  localparam WIDTH = 8;
  wire [7:0] masked = a & 8'h0f;
  always @* begin
    case (op)
      2'd0: y = a + b;
      2'd1: y = a - b;
      2'd2, 2'd3: y = masked | {4'b0000, b[3:0]};
      default: y = 8'd0;
    endcase
  end
  assign zero = (y == 0);
endmodule
";

fn wave_of(nir: &n::Nir, stim: &[BTreeMap<String, i128>]) -> Vec<Vec<i128>> {
    waveform(nir, stim)
}

fn map(pairs: &[(&str, i128)]) -> BTreeMap<String, i128> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

#[test]
fn a_counter_with_enable_and_synchronous_reset_is_imported() {
    let imp = imported("verilog", "counter_en", COUNTER_V);
    assert!(imp.complete(), "{:?}", imp.diags);
    let nir = nir_of("counter_en", &imp.source);
    let verilog = verilog_of(&nir);
    // The `if (rst)` branch became the reset value of the register, and the backend writes
    // the same synchronous reset again.
    assert!(!imp.source.contains("rst:") || imp.source.contains("rst: in bit"));
    assert!(verilog.contains("if (rst) begin"), "{verilog}");
    assert!(verilog.contains("count_r <= 0;") || verilog.contains("count_r <= 4'd0;"));
    let mut rng = Lcg(42);
    let stim: Vec<_> = (0..400)
        .map(|_| {
            map(&[
                ("rst", i128::from(rng.draw() % 9 == 0)),
                ("en", i128::from(rng.draw() % 3 != 0)),
            ])
        })
        .collect();
    // Output before the clock edge of each cycle.
    let mut model = 0i128;
    let want: Vec<Vec<i128>> = stim
        .iter()
        .map(|s| {
            let seen = model;
            if s["rst"] == 1 {
                model = 0;
            } else if s["en"] == 1 {
                model = (model + 1) % 16;
            }
            vec![seen]
        })
        .collect();
    assert_eq!(wave_of(&nir, &stim), want);
    assert!(want.iter().any(|w| w[0] == 15), "the counter wrapped");
    assert!(
        want.iter().filter(|w| w[0] == 0).count() > 20,
        "resets happened"
    );
}

#[test]
fn an_edge_detector_with_assign_is_imported() {
    let imp = imported("verilog", "rise_detect", EDGE_V);
    assert!(imp.complete(), "{:?}", imp.diags);
    let nir = nir_of("rise_detect", &imp.source);
    let mut rng = Lcg(7);
    let stim: Vec<_> = (0..300)
        .map(|_| map(&[("sig", i128::from(rng.draw() % 3 == 0))]))
        .collect();
    let mut prev = 0;
    let want: Vec<Vec<i128>> = stim
        .iter()
        .map(|s| {
            let p = i128::from(s["sig"] == 1 && prev == 0);
            prev = s["sig"];
            vec![p]
        })
        .collect();
    assert_eq!(wave_of(&nir, &stim), want);
    assert!(want.iter().filter(|w| w[0] == 1).count() > 20);
    // The unused clock of the module is still a port: here it is the clock of the circuit.
    assert!(imp.source.contains("clock clk:"), "{}", imp.source);
}

#[test]
fn a_combinational_alu_with_case_parameters_and_wires_is_imported() {
    let imp = imported("verilog", "alu", ALU_V);
    assert!(imp.complete(), "{:?}\n{}", imp.diags, imp.source);
    assert!(imp.source.contains("const WIDTH = 8"));
    assert!(!imp.source.contains("clock "), "no clock without edges");
    let nir = nir_of("alu", &imp.source);
    let mut rng = Lcg(99);
    let stim: Vec<_> = (0..300)
        .map(|_| {
            map(&[
                ("op", i128::from(rng.draw() % 4)),
                ("a", i128::from(rng.draw() % 256)),
                ("b", i128::from(rng.draw() % 256)),
            ])
        })
        .collect();
    let want: Vec<Vec<i128>> = stim
        .iter()
        .map(|s| {
            let (a, b) = (s["a"], s["b"]);
            let y = match s["op"] {
                0 => (a + b) & 255,
                1 => (a - b) & 255,
                _ => (a & 0x0f) | (b & 0x0f),
            };
            vec![y, i128::from(y == 0)]
        })
        .collect();
    // The outputs are `y` then `zero`.
    assert_eq!(wave_of(&nir, &stim), want);
    let verilog = verilog_of(&nir);
    assert!(verilog.contains("always @*"), "{verilog}");
}

#[test]
fn verilog_constructs_without_a_neuron_equivalent_are_ne1002() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "latch",
            "module m(input clk, input en, input [3:0] d, output reg [3:0] q);\n  always @* begin\n    if (en) q = d;\n  end\nendmodule\n",
            "latch",
        ),
        (
            "blocking assignment in a clocked block",
            "module m(input clk, input d, output reg q);\n  always @(posedge clk) begin\n    q = d;\n  end\nendmodule\n",
            "blocking assignment",
        ),
        (
            "non-blocking assignment in a combinational block",
            "module m(input a, output reg q);\n  always @* q <= a;\nendmodule\n",
            "non-blocking assignment",
        ),
        (
            "asynchronous reset",
            "module m(input clk, input rst, input d, output reg q);\n  always @(posedge clk or posedge rst) begin\n    if (rst) q <= 0; else q <= d;\n  end\nendmodule\n",
            "asynchronous reset",
        ),
        (
            "second clock",
            "module m(input c1, input c2, input d, output reg q, output reg r);\n  always @(posedge c1) q <= d;\n  always @(posedge c2) r <= d;\nendmodule\n",
            "second clock",
        ),
        (
            "initial block",
            "module m(input clk, output reg q);\n  initial q = 0;\n  always @(posedge clk) q <= ~q;\nendmodule\n",
            "initial",
        ),
        (
            "function",
            "module m(input [3:0] a, output [3:0] y);\n  function [3:0] twice;\n    input [3:0] x;\n    twice = x + x;\n  endfunction\n  assign y = a;\nendmodule\n",
            "function",
        ),
        (
            "module instance",
            "module m(input a, output y);\n  inv u1 (.a(a), .y(y));\nendmodule\n",
            "instance",
        ),
        (
            "memory",
            "module m(input clk, input [7:0] d, output reg [7:0] q);\n  reg [7:0] mem [0:15];\n  always @(posedge clk) q <= d;\nendmodule\n",
            "memory",
        ),
        (
            "lost carry",
            "module m(input clk, input [7:0] a, input [7:0] b, output reg [8:0] s);\n  always @(posedge clk) s <= a + b;\nendmodule\n",
            "carry",
        ),
        (
            "partial assignment",
            "module m(input clk, input d, output reg [3:0] q);\n  always @(posedge clk) q[0] <= d;\nendmodule\n",
            "partial assignment",
        ),
        (
            "x digits",
            "module m(input a, output y);\n  assign y = a & 1'bx;\nendmodule\n",
            "x or z",
        ),
        (
            "generate",
            "module m(input a, output y);\n  genvar i;\n  assign y = a;\nendmodule\n",
            "generate",
        ),
    ];
    for (label, text, word) in cases {
        let imp = imported("verilog", "m", text);
        assert_eq!(imp.diags.len(), 1, "{label}: {:?}", imp.diags);
        let d = &imp.diags[0];
        assert_eq!(d.code, "NE1002", "{label}");
        assert!(d.message.contains(word), "{label}: {}", d.message);
        assert!(d.span[1] > d.span[0], "{label}: span");
        assert_eq!(imp.fidelity.summary.unsupported, 1, "{label}");
        assert!(imp
            .fidelity
            .entries
            .iter()
            .any(|e| e.class == Class::Unsupported && e.span == d.span));
    }
}

#[test]
fn a_port_called_rst_that_is_not_a_plain_reset_is_renamed() {
    // The Verilog backend adds a synchronous reset to every circuit with a port called `rst`;
    // when the module does not use it that way, the port is renamed so that the circuit keeps
    // its behavior.
    let text = "module m(input clk, input rst, input d, output reg q);\n  always @(posedge clk) begin\n    if (rst) q <= d;\n  end\nendmodule\n";
    let imp = imported("verilog", "m", text);
    assert!(imp.complete(), "{:?}", imp.diags);
    assert!(imp.source.contains("rst_in: in bit"), "{}", imp.source);
    assert!(imp
        .fidelity
        .entries
        .iter()
        .any(|e| e.construct == "port renamed"));
    let nir = nir_of("m", &imp.source);
    assert!(
        !verilog_of(&nir).contains("if (rst) begin"),
        "no implicit reset"
    );
}

#[test]
fn verilog_syntax_errors_are_ne1001() {
    for (label, text) in [
        ("semicolon in the port list", "module m(input a; endmodule"),
        (
            "no endmodule",
            "module m(input a, output y);\n  assign y = a;\n",
        ),
        (
            "missing expression",
            "module m(input a, output y);\n  assign y = ;\nendmodule\n",
        ),
        (
            "unterminated comment",
            "module m(input a); /* open\nendmodule\n",
        ),
        ("not a module", "wire x;"),
        ("empty", ""),
    ] {
        let err = import("verilog", "m", text).unwrap_err();
        assert_eq!(err[0].code, "NE1001", "{label}");
    }
}

#[test]
fn the_verilog_import_is_deterministic_and_formatted() {
    for (name, text) in [
        ("counter_en", COUNTER_V),
        ("rise_detect", EDGE_V),
        ("alu", ALU_V),
    ] {
        let a = imported("verilog", name, text);
        let b = imported("verilog", name, text);
        assert_eq!(a.source, b.source, "{name}");
        let again = neuron_syntax::format::format_source(&a.source).unwrap();
        assert_eq!(again.text, a.source, "{name}: formatted");
        assert!(a.source.starts_with("## Imported from Verilog"));
        assert!(a.fidelity.summary.exact >= 1);
        assert_eq!(a.fidelity.summary.unsupported, 0);
    }
    let a = imported("verilog", "counter_en", COUNTER_V);
    assert!(a
        .fidelity
        .entries
        .iter()
        .any(|e| e.construct == "clock frequency" && e.class == Class::Equivalent));
}
