//! Parser rules clarified in spec v0.2.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_syntax::ast::*;
use neuron_syntax::parse;

fn ok(src: &str) -> File {
    let out = parse(src);
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    out.file
}

fn codes(src: &str) -> Vec<&'static str> {
    parse(src).diagnostics.iter().map(|d| d.code).collect()
}

fn const_value(f: &File, i: usize) -> &Expr {
    let ItemKind::Const { value, .. } = &f.items[i].kind else {
        panic!("not a const")
    };
    value
}

#[test]
fn unit_after_a_range_applies_to_both_bounds() {
    let f = ok("module m\nconst r = 0..120 degC\nconst s = -40..40 mm\n");
    for (i, unit) in [(0, "degC"), (1, "mm")] {
        let ExprKind::Range { lo, hi, .. } = &const_value(&f, i).kind else {
            panic!("not a range")
        };
        for end in [lo, hi] {
            assert!(matches!(&end.kind, ExprKind::Quantity(_, u) if u.display() == unit));
        }
    }
}

#[test]
fn linear_map_arguments_get_units_on_the_target_range() {
    let f = ok("module m\nconst v = scale(0..4095 -> 0..120 degC)\n");
    let ExprKind::Call(_, args) = &const_value(&f, 0).kind else {
        panic!()
    };
    let ExprKind::Map(from, to) = &args[0].value.kind else {
        panic!("not a map")
    };
    assert!(
        matches!(&from.kind, ExprKind::Range { lo, .. } if matches!(lo.kind, ExprKind::Lit(_)))
    );
    assert!(
        matches!(&to.kind, ExprKind::Range { lo, .. } if matches!(lo.kind, ExprKind::Quantity(..)))
    );
}

#[test]
fn percent_is_remainder_between_operands_and_a_unit_otherwise() {
    let f = ok("module m\nconst a = 7 % 2\nconst b = 50 %\nconst c = f(2) % 3\n");
    assert!(matches!(
        &const_value(&f, 0).kind,
        ExprKind::Binary(BinOp::Rem, ..)
    ));
    assert!(matches!(&const_value(&f, 1).kind, ExprKind::Quantity(_, u) if u.display() == "%"));
    assert!(matches!(
        &const_value(&f, 2).kind,
        ExprKind::Binary(BinOp::Rem, ..)
    ));
}

#[test]
fn max_is_a_normal_name_and_a_loop_bound() {
    ok("module m\nfn big(a: i32, b: i32) -> i32 = max(a, b)\n");
    ok("module m for mcu\ntask t every 1 ms:\n  while x max 3:\n    pass\n  loop max 2:\n    break\n");
}

#[test]
fn robot_io_locations_are_ordinary_calls() {
    ok("module r for robot\nio a: gpio.input @ din(5)\nio b: gpio.output @ dout(3)\n");
}

#[test]
fn contracts_are_only_accepted_at_the_start_of_a_function() {
    ok("module m\nfn f(x: i32):\n  requires x > 0\n  ensures result > 0\n  return x\n");
    assert_eq!(
        codes("module m\nfn f(x: i32):\n  if x > 0:\n    requires x > 1\n    pass\n"),
        vec!["NE0012"]
    );
    assert_eq!(
        codes("module m\nfn g(x: i32):\n  pass\n  requires x > 0\n"),
        vec!["NE0012"]
    );
    assert_eq!(
        codes("module m\nfn h(x: i32):\n  invariant x > 0\n  pass\n"),
        vec!["NE0012"]
    );
}

#[test]
fn non_ascii_names_are_rejected_but_unit_characters_are_fine() {
    assert_eq!(codes("module m\nconst \u{b5}x = 1\n"), vec!["NE0015"]);
    ok("module m\nconst t = 5 \u{b5}s\nconst r = 2 k\u{3a9}\n");
}

#[test]
fn unit_spacing_is_checked_but_recovered() {
    let out = parse("module m\nconst t = 5  ms\n");
    assert_eq!(out.diagnostics.len(), 1);
    assert_eq!(out.diagnostics[0].code, "NE0014");
    assert!(matches!(
        &const_value(&out.file, 0).kind,
        ExprKind::Quantity(..)
    ));
}

#[test]
fn semicolons_in_sequences_give_one_item_per_statement() {
    let f = ok("module m for plc\nsequence s:\n  a.on(); b.on()\n  c.on()\n");
    let ItemKind::Sequence(s) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(s.items.len(), 3);
    assert!(s.items.iter().all(|i| matches!(i, SeqItem::Stmt(_))));
}

#[test]
fn cell_items_must_be_indented_under_the_header() {
    assert!(codes("cell C\ndevice a: x\n").contains(&"NE0005"));
    assert!(codes("cell C\n  device a: x\nzone z: box()\n").contains(&"NE0013"));
    ok("cell C\n  device a: x\n  zone z: box(x: 0..1 mm)\n");
}

#[test]
fn nodes_know_their_location() {
    let src = "module m\nstruct P:\n  x: f32\n  y: f32\nfn f(mut self, n: i32) = g(a, b: 2)\n";
    let f = ok(src);
    let ItemKind::Struct(s) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(s.members[0].span.text(src), "x: f32");
    assert_eq!(s.members[1].span.text(src), "y: f32");
    let ItemKind::Fn(func) = &f.items[1].kind else {
        panic!()
    };
    assert_eq!(func.sig.params[0].span.text(src), "mut self");
    assert_eq!(func.sig.params[1].span.text(src), "n: i32");
    let FnBody::Expr(e) = &func.body else {
        panic!()
    };
    let ExprKind::Call(_, args) = &e.kind else {
        panic!()
    };
    assert_eq!(args[0].span.text(src), "a");
    assert_eq!(args[1].span.text(src), "b: 2");
}
