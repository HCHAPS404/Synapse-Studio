//! Task A5: types, units, conversions, generics, classes and interfaces
//! (NE0201 to NE0217).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use proptest::prelude::*;

fn body(stmts: &str) -> String {
    format!(
        "module t\nfn f():\n{}",
        stmts
            .lines()
            .map(|l| format!("  {l}\n"))
            .collect::<String>()
    )
}

#[test]
fn exact_unit_arithmetic_is_accepted() {
    ok("module t\nfn f() -> f32 s:\n  return 1 s + 250 ms\n");
    ok("module t\nfn f() -> f32 mm/s:\n  return 10 mm / 2 s\n");
    ok("module t\nfn f(a: f32 mm, b: f32 mm/s, t: f32 s) -> f32 mm:\n  return a + b * t\n");
    ok("module t\nfn f() -> u16 mV:\n  return 3 V\n");
}

#[test]
fn incompatible_dimensions_ne0201() {
    only(&body("let x = 1 mm + 1 s"), &["NE0201"]);
    only(
        "module t\nfn f(a: f32 mm, b: f32 s) -> f32 mm:\n  return a + b\n",
        &["NE0201"],
    );
    only(
        "module t\nfn f(a: f32 mm, b: f32 s) -> bool:\n  return a < b\n",
        &["NE0201"],
    );
    only("module t\nfn f() -> f32 mm:\n  return 5 s\n", &["NE0201"]);
    // A plain number is not a length, except for zero.
    only(
        "module t\nfn f(a: f32 mm) -> bool:\n  return a > 5\n",
        &["NE0201"],
    );
    ok("module t\nfn f(a: f32 mm) -> bool:\n  return a > 0\n");
}

#[test]
fn units_of_products_and_quotients() {
    ok("module t\nfn f(a: f32 m, t: f32 s) -> f32 m/s:\n  return a / t\n");
    only(
        "module t\nfn f(a: f32 m, t: f32 s) -> f32 m:\n  return a / t\n",
        &["NE0201"],
    );
    ok("module t\nfn f(v: f32 V, r: f32 Ohm) -> f32 A:\n  return v / r\n");
}

#[test]
fn affine_units_ne0203() {
    let two = "module t\nfn f(a: f32 degC) -> f32 degC:\n  return a + a\n";
    only(two, &["NE0203"]);
    ok("module t\nfn f(a: f32 degC, b: f32 degC) -> f32 K:\n  return a - b\n");
    ok("module t\nfn f(a: f32 degC, d: f32 K) -> f32 degC:\n  return a + d\n");
    ok("module t\nfn f(a: f32 degC) -> bool:\n  return a > 90 degC\n");
}

#[test]
fn lossy_conversions_ne0204() {
    only(&body("let y: u16 = 7\nlet z = y as u8"), &["NE0204"]);
    only(&body("let x: f32 = 1.5\nlet i = x as i32"), &["NE0204"]);
    ok(&body("let y: u8 = 7\nlet z = y as u16"));
    ok(&body("let y: u16 = 7\nlet z = y as f32"));
    // Inexact unit conversion for integers.
    only(
        "module t\nfn f(a: u16 mV) -> u16 V:\n  return a\n",
        &["NE0204"],
    );
    ok("module t\nfn f(a: u16 V) -> u16 mV:\n  return a\n");
    only(&body("let x: u16 V = 500 mV"), &["NE0204"]);
}

#[test]
fn literal_out_of_range_ne0205() {
    only(&body("let x: u8 = 256"), &["NE0205"]);
    only(&body("let x: u8 = -1"), &["NE0205"]);
    ok(&body("let x: u8 = 255"));
    ok(&body("let x: i8 = -128"));
}

#[test]
fn overflow_of_constants_ne0207() {
    only(&body("let x: u8 = 200 + 100"), &["NE0207"]);
    ok(&body("let x: u8 = 100 + 100"));
}

#[test]
fn unit_after_a_call_ne0202() {
    only(&body("let x = abs(3) mm"), &["NE0202"]);
    ok("module t for robot\npoint home = joints(0, -30, 60, 0, 60, 0) deg\n");
}

#[test]
fn type_mismatch_ne0208() {
    only(
        &body("let a: u8 = 1\nlet b: u16 = 2\nlet c = a + b"),
        &["NE0208"],
    );
    only(&body("let a: bool = 1"), &["NE0208"]);
    only(&body("let a: u8 = 1.5"), &["NE0208"]);
    only(&body("if 3:\n  pass"), &["NE0208"]);
    only(&body("let a: u8 = true"), &["NE0208"]);
}

#[test]
fn call_arguments_ne0209() {
    let base = "module t\nfn g(a: u8, b: u8 = 2) -> u8:\n  return a + b\n";
    ok(&format!(
        "{base}fn f() -> u8:\n  return g(1) + g(1, 3) + g(1, b: 3)\n"
    ));
    only(&format!("{base}fn f() -> u8:\n  return g()\n"), &["NE0209"]);
    only(
        &format!("{base}fn f() -> u8:\n  return g(1, 2, 3)\n"),
        &["NE0209"],
    );
    only(
        &format!("{base}fn f() -> u8:\n  return g(1, c: 3)\n"),
        &["NE0209"],
    );
    only(
        &format!("{base}fn f() -> u8:\n  return g(a: 1, 2)\n"),
        &["NE0209"],
    );
}

#[test]
fn immutable_bindings_ne0210() {
    only(&body("let x = 1\nx = 2"), &["NE0210"]);
    only("module t\nfn f(x: u8):\n  x = 2\n", &["NE0210"]);
    ok("module t\nfn f(mut x: u8):\n  x = 2\n");
    ok(&body("var x = 1\nx = 2\nx += 1"));
    only("module t\nconst c = 1\nfn f():\n  c = 2\n", &["NE0210"]);
}

const POINT: &str = "module t\nstruct Point:\n  x: f32 mm\n  y: f32 mm\n";

#[test]
fn struct_fields_and_construction() {
    ok(&format!(
        "{POINT}fn f() -> f32 mm:\n  let p = Point(x: 10 mm, y: 5 mm)\n  return p.x + p.y\n"
    ));
    only(
        &format!("{POINT}fn f() -> f32 mm:\n  let p = Point(x: 10 mm)\n  return p.x\n"),
        &["NE0209"],
    );
    only(
        &format!("{POINT}fn f() -> f32 mm:\n  let p = Point(x: 10 mm, y: 5 mm)\n  return p.z\n"),
        &["NE0212"],
    );
    only(
        &format!("{POINT}fn f() -> f32 mm:\n  let p = Point(x: 10 s, y: 5 mm)\n  return p.x\n"),
        &["NE0201"],
    );
    only(
        &format!("{POINT}fn f(p: Point):\n  p.x = 1 mm\n"),
        &["NE0210"],
    );
    ok(&format!("{POINT}fn f(mut p: Point):\n  p.x = 1 mm\n"));
}

#[test]
fn enums_and_variants() {
    let src = "module t\nenum Command:\n  Stop\n  Move(f32 mm, f32 mm/s)\n  Home\n";
    ok(&format!(
        "{src}fn f() -> Command:\n  return Command.Move(10 mm, 5 mm/s)\n"
    ));
    ok(&format!("{src}fn f() -> Command:\n  return Command.Stop\n"));
    only(
        &format!("{src}fn f() -> Command:\n  return Command.Fly\n"),
        &["NE0212"],
    );
    only(
        &format!("{src}fn f() -> Command:\n  return Command.Move(10 mm)\n"),
        &["NE0209"],
    );
    ok(&format!(
        "{src}fn f(c: Command):\n  match c:\n    Command.Stop => pass\n    Command.Move(p, v) if v > 0 mm/s => pass\n    Command.Move(p, v) => pass\n    Command.Home => pass\n"
    ));
}

const LOWPASS: &str = "module t\npub class LowPass:\n  var state: f32 = 0.0\n  let alpha: f32\n\n  pub fn step(mut self, x: f32) -> f32:\n    requires 0.0 <= self.alpha <= 1.0\n    self.state = self.state + self.alpha * (x - self.state)\n    return self.state\n";

#[test]
fn classes_construction_and_methods() {
    ok(&format!(
        "{LOWPASS}fn f() -> f32:\n  var g = LowPass(alpha: 0.2)\n  return g.step(1.0)\n"
    ));
    only(
        &format!("{LOWPASS}fn f() -> f32:\n  let g = LowPass(alpha: 0.2)\n  return g.step(1.0)\n"),
        &["NE0210"],
    );
    only(
        &format!("{LOWPASS}fn f() -> f32:\n  var g = LowPass()\n  return g.step(1.0)\n"),
        &["NE0209"],
    );
    only(
        &format!("{LOWPASS}fn f() -> f32:\n  var g = LowPass(alpha: 0.2)\n  return g.reset()\n"),
        &["NE0212"],
    );
    only(&format!("{LOWPASS}fn f() -> f32:\n  var g = LowPass(alpha: 0.2)\n  g.alpha = 0.5\n  return 0.0\n"), &["NE0210"]);
}

#[test]
fn methods_cannot_change_state_without_mut_self() {
    let src = "module t\nclass C:\n  var n: u8 = 0\n\n  fn bump(self):\n    self.n = self.n + 1\n";
    only(src, &["NE0210"]);
}

#[test]
fn interfaces_are_checked_against_classes() {
    let iface = "module t\ninterface Sensor:\n  fn read(mut self) -> f32\n";
    ok(&format!("{iface}class Ok1 is Sensor:\n  var v: f32 = 0.0\n\n  fn read(mut self) -> f32:\n    return self.v\n"));
    only(
        &format!("{iface}class Bad is Sensor:\n  var v: f32 = 0.0\n"),
        &["NE0211"],
    );
    let user = format!(
        "{iface}class A is Sensor:\n  var v: f32 = 0.0\n\n  fn read(mut self) -> f32:\n    return self.v\nclass B:\n  var v: f32 = 0.0\nfn use_it(mut s: Sensor) -> f32:\n  return s.read()\nfn f() -> f32:\n  var a = A()\n  return use_it(mut a)\n"
    );
    ok(&user);
    let bad = user.replace(
        "var a = A()\n  return use_it(mut a)",
        "var b = B()\n  return use_it(mut b)",
    );
    only(&bad, &["NE0211"]);
}

#[test]
fn generic_functions_and_bounds() {
    let clamp = "module t\nfn clamp<T: Ord>(x: T, lo: T, hi: T) -> T:\n  requires lo <= hi\n  ensures lo <= result <= hi\n  return if x < lo then lo else if x > hi then hi else x\n";
    ok(&format!(
        "{clamp}fn f(x: i32) -> i32:\n  return clamp(x, 0, 10)\n"
    ));
    ok(&format!(
        "{clamp}fn f(x: u8) -> u8:\n  return clamp(x, 1, 9)\n"
    ));
    only(
        &format!("{clamp}fn f(x: f32) -> f32:\n  return clamp(x, 0.0, 1.0)\n"),
        &["NE0208"],
    );
}

#[test]
fn generic_structs() {
    let src = "module t\nstruct Pair<T>:\n  a: T\n  b: T\n";
    ok(&format!(
        "{src}fn f() -> u8:\n  let p = Pair(a: 1, b: 2)\n  return p.a\n"
    ));
}

#[test]
fn results_options_and_propagation() {
    let read = "module t\nfn read(x: u16) -> Result<u16, IoError>:\n  return Ok(x)\n";
    ok(&format!(
        "{read}fn g() -> Result<u16, IoError>:\n  let v = read(1)?\n  return Ok(v)\n"
    ));
    only(
        &format!("{read}fn g() -> u16:\n  let v = read(1)?\n  return v\n"),
        &["NE0208"],
    );
    ok("module t\nfn f(x: u8?) -> u8:\n  return x ?? 3\n");
    ok("module t\nfn f() -> u8?:\n  return none\n");
}

#[test]
fn control_flow_outside_context_ne0214() {
    only(&body("break"), &["NE0214"]);
    only(
        "module t for mcu\ntask a every 10 ms:\n  return 3\n",
        &["NE0214"],
    );
    ok(&body("for i in 0..3:\n  if i > 1:\n    break"));
}

#[test]
fn mut_markers_at_call_sites_ne0215() {
    let g = "module t\nfn inc(mut a: u8):\n  a += 1\nfn peek(a: u8) -> u8:\n  return a\n";
    ok(&format!(
        "{g}fn f():\n  var x: u8 = 0\n  inc(mut x)\n  let y = peek(x)\n"
    ));
    only(
        &format!("{g}fn f():\n  var x: u8 = 0\n  inc(x)\n"),
        &["NE0215"],
    );
    only(
        &format!("{g}fn f():\n  var x: u8 = 0\n  let y = peek(mut x)\n"),
        &["NE0215"],
    );
    only(
        &format!("{g}fn f():\n  let x: u8 = 0\n  inc(mut x)\n"),
        &["NE0210"],
    );
}

#[test]
fn constant_indices_are_bounds_checked_ne0216() {
    only(&body("let a = [1, 2, 3]\nlet b = a[3]"), &["NE0216"]);
    ok(&body("let a = [1, 2, 3]\nlet b = a[2]"));
}

#[test]
fn functions_must_return_on_every_path_ne0217() {
    only(
        "module t\nfn f(x: bool) -> u8:\n  if x:\n    return 1\n",
        &["NE0217"],
    );
    ok("module t\nfn f(x: bool) -> u8:\n  if x:\n    return 1\n  else:\n    return 2\n");
    ok("module t\nfn f(x: bool) -> u8:\n  if x:\n    return 1\n  return 2\n");
}

#[test]
fn contracts_use_result_and_old() {
    ok("module t\nfn f(mut x: u8) -> u8:\n  requires x < 200\n  ensures result > old(x)\n  x += 1\n  return x\n");
    only(
        "module t\nfn f(x: u8) -> u8:\n  requires x\n  return x\n",
        &["NE0208"],
    );
}

#[test]
fn bit_vectors_follow_hdl_rules() {
    ok("module t\nfn f(a: bit<8>, b: bit<8>) -> bit<8>:\n  return a +% b\n");
    only(
        "module t\nfn f(a: bit<8>, b: bit<4>) -> bit<8>:\n  return a +% b\n",
        &["NE0208"],
    );
    ok("module t\nfn f(a: bit<8>, b: bit<4>) -> bit<12>:\n  return a ++ b\n");
    ok("module t\nfn f(a: bit<8>) -> bit<4>:\n  return a[7:4]\n");
}

#[test]
fn spec_examples_with_units_check() {
    ok("module t\nfn scale(raw: u16, from: range<u16>, to: range<f32 degC>) -> f32 degC:\n  let ratio = (raw - from.start) as f32 / (from.end - from.start) as f32\n  return to.start + ratio * (to.end - to.start)\n");
}

proptest! {
    /// UNIT-1 at program level: adding two quantities of the same dimension is accepted
    /// and adding different dimensions is rejected with NE0201.
    #[test]
    fn dimension_rule_holds_for_all_unit_pairs(i in 0usize..UNITS.len(), j in 0usize..UNITS.len()) {
        let (a, da) = UNITS[i];
        let (b, db) = UNITS[j];
        let src = format!("module t\nfn f(x: f32 {a}, y: f32 {b}) -> f32 {a}:\n  return x + y\n");
        let r = run(&src);
        if da == db {
            prop_assert!(r.is_ok(), "{}", r.render());
        } else {
            prop_assert_eq!(errors(&r), vec!["NE0201"], "{}", r.render());
        }
    }
}

const UNITS: [(&str, u8); 10] = [
    ("mm", 0),
    ("m", 0),
    ("um", 0),
    ("s", 1),
    ("ms", 1),
    ("min", 1),
    ("kg", 2),
    ("g", 2),
    ("V", 3),
    ("mV", 3),
];
