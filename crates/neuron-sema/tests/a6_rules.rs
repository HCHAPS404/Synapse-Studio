//! Task A6: dialect rules and effects (NE0501, NE0302, NE0303, NE0305, NE0403,
//! NE0404, NE0405).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

#[test]
fn constructs_outside_their_dialect_ne0501() {
    only("module t for core\nvar x: u8 = 0\n", &["NE0501"]);
    only(
        "module t for core\ntask a every 10 ms:\n  pass\n",
        &["NE0501"],
    );
    only(
        "module t for robot\nio b: gpio.input @ din(1)\non interrupt b.falling (prio 1):\n  pass\n",
        &["NE0501"],
    );
    only("module t for mcu\nscan 10 ms\n", &["NE0501"]);
    has(
        "module t for plc\nscan 10 ms\nio a: gpio.output @ D1\n",
        "NE0501",
    );
    only(
        "module t for mcu\nfn f(p: point):\n  move ptp to p\n",
        &["NE0501"],
    );
    only(
        "module t for mcu\nvar a: bool = false\nfn f():\n  set a when true\n",
        &["NE0501"],
    );
}

#[test]
fn each_dialect_accepts_its_own_constructs() {
    ok("module t for mcu\nvar n: u8 = 0\ntask a every 10 ms:\n  n += 1\n");
    ok("module t for plc\nscan 10 ms\ninputs:\n  a: bool @ %I0.0\noutputs:\n  b: bool @ %Q0.0\nrung r:\n  b = a\nsafety:\n  safe_state:\n    b = false\n");
    ok("module t for hdl\nclock c: 100 MHz\ncircuit f(a: in bit<8>, y: out bit<8>) on c:\n  comb:\n    y = a\n");
    ok("module t for robot\npoint home = joints(0, 0, 0, 0, 0, 0) deg\nsequence s():\n  move ptp to home speed 50 %\n");
    ok("module t for host\nvar n: u8 = 0\ntask a every 10 ms:\n  n += 1\n");
}

#[test]
fn floats_are_not_synthesizable_in_hdl() {
    let a = run("module t for hdl\nclock c: 100 MHz\ncircuit f(a: in bit<8>, y: out f32) on c:\n  comb:\n    y = 1.5\n");
    assert!(errors(&a).contains(&"NE0501"), "{}", a.render());
    let b = run("module t for hdl\nconst x: f32 = 1.5\n");
    assert!(errors(&b).contains(&"NE0501"), "{}", b.render());
}

#[test]
fn heap_types_only_in_host() {
    has("module t for mcu\nfn f(x: list<u8>):\n  pass\n", "NE0501");
    ok("module t for host\nfn f(x: list<u8>):\n  pass\n");
}

#[test]
fn rungs_hold_only_boolean_networks() {
    let head =
        "module t for plc\nscan 10 ms\ninputs:\n  a: bool @ %I0.0\noutputs:\n  b: bool @ %Q0.0\n";
    let safety = "safety:\n  safe_state:\n    b = false\n";
    only(
        &format!("{head}rung r:\n  if a:\n    b = true\n{safety}"),
        &["NE0501"],
    );
    ok(&format!("{head}rung r:\n  b = a and not b\n{safety}"));
}

#[test]
fn recursion_is_rejected_outside_host_ne0302() {
    only("module t\nfn f(n: u8) -> u8:\n  return f(n)\n", &["NE0302"]);
    let mutual = "module t\nfn a(n: u8) -> u8:\n  return b(n)\nfn b(n: u8) -> u8:\n  return a(n)\n";
    let r = run(mutual);
    assert_eq!(errors(&r), ["NE0302", "NE0302"], "{}", r.render());
    ok("module t for host\nfn f(n: u8) -> u8:\n  return f(n)\n");
    ok("module t\nfn f(n: u8) -> u8:\n  return n\nfn g(n: u8) -> u8:\n  return f(n)\n");
}

#[test]
fn mut_arguments_must_not_alias_ne0303() {
    let g = "module t for mcu\nvar g: u8 = 0\nfn bump(mut a: u8):\n  a += 1\nfn touch(mut a: u8):\n  g += 1\n  a += 1\n";
    ok(&format!("{g}fn f():\n  var x: u8 = 0\n  bump(mut x)\n"));
    only(&format!("{g}fn f():\n  touch(mut g)\n"), &["NE0303"]);
    let two = "module t\nfn two(mut a: [u8; 4], b: [u8; 4]):\n  a[0] = b[0]\n";
    ok(&format!(
        "{two}fn f():\n  var x: [u8; 4] = [0; 4]\n  var y: [u8; 4] = [0; 4]\n  two(mut x, y)\n"
    ));
    only(
        &format!("{two}fn f():\n  var x: [u8; 4] = [0; 4]\n  two(mut x, x)\n"),
        &["NE0303"],
    );
}

#[test]
fn slices_do_not_escape_ne0305() {
    only(
        "module t\nfn f(x: [u8]) -> [u8]:\n  return x\n",
        &["NE0305"],
    );
    only(
        "module t\nfn f(x: [u8]) -> u8:\n  let y = x\n  return 1\n",
        &["NE0305"],
    );
    only("module t\nstruct S:\n  a: [u8]\n", &["NE0305"]);
    ok("module t\nfn f(x: [u8]) -> u8:\n  return x[0]\n");
}

#[test]
fn loops_need_a_static_bound_ne0403() {
    only(
        "module t\nfn f(x: u8) -> u8:\n  var i: u8 = 0\n  while i < x:\n    i += 1\n  return i\n",
        &["NE0403"],
    );
    only("module t\nfn f() -> u8:\n  var i: u8 = 0\n  loop:\n    i += 1\n    if i > 3:\n      break\n  return i\n", &["NE0403"]);
    only(
        "module t\nfn f(n: u8) -> u8:\n  var s: u8 = 0\n  for i in 0..n:\n    s += 1\n  return s\n",
        &["NE0403"],
    );
    ok("module t\nfn f(x: u8) -> u8:\n  var i: u8 = 0\n  while i < x max 10:\n    i += 1\n  return i\n");
    ok("module t\nconst n = 4\nfn f() -> u8:\n  var s: u8 = 0\n  for i in 0..n:\n    s += 1\n  return s\n");
    ok("module t\nfn f(a: [u8; 4]) -> u8:\n  var s: u8 = 0\n  for v in a:\n    s += v\n  return s\n");
    ok("module t\nfn f() -> u8:\n  var i: u8 = 0\n  loop max 3:\n    i += 1\n  return i\n");
    ok("module t for host\nfn f() -> u8:\n  var i: u8 = 0\n  loop:\n    i += 1\n    if i > 3:\n      break\n  return i\n");
    // The bound must itself be constant.
    only("module t\nfn f(n: u8) -> u8:\n  var i: u8 = 0\n  while i < 5 max n:\n    i += 1\n  return i\n", &["NE0403"]);
}

#[test]
fn waiting_in_a_task_or_interrupt_ne0404() {
    only(
        "module t for mcu\ntask a every 10 ms:\n  wait 5 ms\n",
        &["NE0404"],
    );
    ok("module t for mcu\nsequence s:\n  step a:\n    wait 5 ms\n");
    let via_fn = "module t for mcu\nsequence s:\n  step a:\n    wait 5 ms\nfn slow():\n  wait 5 ms\ntask a every 10 ms:\n  slow()\n";
    has(via_fn, "NE0404");
}

#[test]
fn hard_deadlines_on_host_ne0405() {
    only(
        "module t for host\ntask a every 10 ms (deadline 5 ms):\n  pass\n",
        &["NE0405"],
    );
    ok("module t for host\ntask a every 10 ms:\n  pass\n");
}
