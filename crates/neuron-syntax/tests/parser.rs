//! Parser tests: every program in the specification and examples parses cleanly,
//! key constructs produce the expected tree, errors are reported and recovered.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_syntax::ast::*;
use neuron_syntax::parse;
use proptest::prelude::*;

fn ok(src: &str) -> File {
    let out = parse(src);
    assert!(
        out.diagnostics.is_empty(),
        "unexpected diagnostics:\n{}",
        out.diagnostics
            .iter()
            .map(|d| d.render("test.neuron", src))
            .collect::<String>()
    );
    out.file
}

fn codes(src: &str) -> Vec<&'static str> {
    parse(src).diagnostics.iter().map(|d| d.code).collect()
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// ---- whole programs -----------------------------------------------------

#[test]
fn examples_parse_without_errors() {
    for entry in std::fs::read_dir(repo_root().join("examples")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "neuron") {
            let src = std::fs::read_to_string(&path).unwrap();
            let out = parse(&src);
            assert!(
                out.diagnostics.is_empty(),
                "{}:\n{}",
                path.display(),
                out.diagnostics
                    .iter()
                    .map(|d| d.render(&path.display().to_string(), &src))
                    .collect::<String>()
            );
        }
    }
}

/// Every ```neuron block in the specification that is a complete file parses.
#[test]
fn spec_programs_parse_without_errors() {
    let mut parsed = 0;
    for entry in std::fs::read_dir(repo_root().join("spec/language")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let md = std::fs::read_to_string(&path).unwrap();
        let mut block: Option<String> = None;
        for line in md.lines() {
            match (&mut block, line.trim_start()) {
                (None, "```neuron") => block = Some(String::new()),
                (Some(code), "```") => {
                    let first = code
                        .lines()
                        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                        .unwrap_or("");
                    if first.starts_with("module ") || first.starts_with("cell ") {
                        let out = parse(code);
                        assert!(
                            out.diagnostics.is_empty(),
                            "{}:\n{code}\n{}",
                            path.display(),
                            out.diagnostics
                                .iter()
                                .map(|d| d.render("spec", code))
                                .collect::<String>()
                        );
                        parsed += 1;
                    }
                    block = None;
                }
                (Some(code), _) => {
                    code.push_str(line);
                    code.push('\n');
                }
                _ => {}
            }
        }
    }
    assert!(
        parsed >= 11,
        "only {parsed} complete programs found in the spec"
    );
}

// ---- structure ----------------------------------------------------------

#[test]
fn header_dialect_and_target() {
    let f = ok("module blink for mcu on nucleo_f446re\n");
    assert_eq!(f.header.name.dotted(), "blink");
    assert_eq!(f.header.dialect, Dialect::Mcu);
    assert_eq!(f.header.target.unwrap().dotted(), "nucleo_f446re");
    assert_eq!(
        ok("cell Line\n  device a: x\n").header.dialect,
        Dialect::Cell
    );
    assert_eq!(codes("module m for robots\n"), vec!["NE0010"]);
}

#[test]
fn task_with_quantity_period_and_attributes() {
    let f = ok("module m for mcu\ntask blink every 500 ms offset 2 ms (prio 1, deadline 4 ms):\n  led.toggle()\n");
    let ItemKind::Task(t) = &f.items[0].kind else {
        panic!("not a task")
    };
    let Trigger::Every { period, offset } = &t.trigger else {
        panic!()
    };
    assert!(matches!(&period.kind, ExprKind::Quantity(_, u) if u.display() == "ms"));
    assert!(offset.is_some());
    assert_eq!(t.attrs.len(), 2);
    assert_eq!(t.body.stmts.len(), 1);
}

#[test]
fn compound_units() {
    let f = ok("module m\nconst g = 9.81 m/s^2\nconst v: f32 mm/s = 3 mm/s\n");
    let ItemKind::Const { value, .. } = &f.items[0].kind else {
        panic!()
    };
    let ExprKind::Quantity(_, u) = &value.kind else {
        panic!()
    };
    assert_eq!(u.display(), "m*s^-2");
    let ItemKind::Const { ty, .. } = &f.items[1].kind else {
        panic!()
    };
    assert_eq!(
        ty.as_ref().unwrap().unit.as_ref().unwrap().display(),
        "mm*s^-1"
    );
}

#[test]
fn pipeline_and_linear_map() {
    let f = ok("module m for mcu\ntask t every 10 ms:\n  temp = s.read() |> scale(0..4095 -> 0..120 degC) |> f.step()\n");
    let ItemKind::Task(t) = &f.items[0].kind else {
        panic!()
    };
    let StmtKind::Assign { value, .. } = &t.body.stmts[0].kind else {
        panic!()
    };
    let ExprKind::Pipe(inner, _) = &value.kind else {
        panic!("expected a pipeline")
    };
    let ExprKind::Pipe(_, scale) = &inner.kind else {
        panic!()
    };
    let ExprKind::Call(_, args) = &scale.kind else {
        panic!()
    };
    assert!(matches!(args[0].value.kind, ExprKind::Map(..)));
}

#[test]
fn chained_comparison() {
    let f = ok("module m\nfn ok(x: f32) -> bool = 0.0 <= x < 1.0\n");
    let ItemKind::Fn(func) = &f.items[0].kind else {
        panic!()
    };
    let FnBody::Expr(e) = &func.body else {
        panic!()
    };
    assert!(matches!(&e.kind, ExprKind::Compare(_, rest) if rest.len() == 2));
}

#[test]
fn contracts_are_collected() {
    let f = ok("module m\nfn clamp(x: i32, lo: i32, hi: i32) -> i32:\n  requires lo <= hi\n  ensures lo <= result <= hi\n  return x\n");
    let ItemKind::Fn(func) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(func.contracts.len(), 2);
    assert_eq!(func.contracts[0].kind, ContractKind::Requires);
}

#[test]
fn call_needs_adjacent_parenthesis() {
    // `on report (prio 1)`: the parenthesis is the attribute list, not a call
    let f = ok("module m for mcu\ntask r on report (prio 1):\n  pass\n");
    let ItemKind::Task(t) = &f.items[0].kind else {
        panic!()
    };
    assert!(matches!(&t.trigger, Trigger::On(e) if matches!(e.kind, ExprKind::Name(_))));
    assert_eq!(t.attrs.len(), 1);
}

#[test]
fn fsm_transitions() {
    let src = "module m for mcu\nfsm Door every 10 ms:\n  initial state Closed:\n    entry: lock.engage()\n    -> Opening when cmd\n  state Opening:\n    -> Open when done\n    -> Closed when stop or after 5 s\n  state Open:\n    -> Closed after 30 s\n";
    let f = ok(src);
    let ItemKind::Fsm(m) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(m.states.len(), 3);
    assert!(m.states[0].initial && m.states[0].entry.is_some());
    let t = &m.states[1].transitions[1];
    assert!(t.when.is_some() && t.after.is_some());
    assert!(m.states[2].transitions[0].when.is_none());
}

#[test]
fn sequence_with_parallel_and_until_timeout() {
    let src = "module m for plc\nsequence Cycle:\n  initial step Idle:\n    until start\n  parallel:\n    branch a:\n      step A:\n        run drill.cycle()\n        until drill.done\n    branch b:\n      step B: wait 500 ms\n  step Clamp:\n    until clamp.closed or timeout 2 s -> Fault\n    -> Idle\n";
    let f = ok(src);
    let ItemKind::Sequence(s) = &f.items[0].kind else {
        panic!()
    };
    assert_eq!(s.items.len(), 3);
    assert!(matches!(&s.items[1], SeqItem::Parallel { branches, .. } if branches.len() == 2));
    let SeqItem::Step { body, .. } = &s.items[2] else {
        panic!()
    };
    assert!(matches!(
        &body[0],
        SeqItem::Stmt(Stmt {
            kind: StmtKind::Until {
                timeout: Some(_),
                on_timeout: Some(_),
                ..
            },
            ..
        })
    ));
    assert!(matches!(
        &body[1],
        SeqItem::Stmt(Stmt {
            kind: StmtKind::Jump(_),
            ..
        })
    ));
}

#[test]
fn plc_sections_rungs_and_coils() {
    let src = "module p for plc\nscan 10 ms\ninputs:\n  start: bool @ %I0.0\noutputs:\n  valve: bool @ %Q0.0\nmemory:\n  retain cycles: u32 @ %MD10\nrung r:\n  set valve when start\n  reset valve when not start\n  cycles += 1 when start.rising\n";
    let f = ok(src);
    assert!(
        matches!(&f.items[1].kind, ItemKind::PlcSection { section: PlcSection::Inputs, vars } if vars[0].at.is_some())
    );
    let ItemKind::PlcSection { vars, .. } = &f.items[3].kind else {
        panic!()
    };
    assert!(vars[0].retain);
    let ItemKind::Rung { body, .. } = &f.items[4].kind else {
        panic!()
    };
    assert!(matches!(
        body.stmts[0].kind,
        StmtKind::Coil { set: true, .. }
    ));
    assert!(matches!(
        body.stmts[2].kind,
        StmtKind::Assign { when: Some(_), .. }
    ));
}

#[test]
fn hdl_circuit() {
    let src = "module h for hdl\nclock sys: 100 MHz @ pin(W5)\ncircuit pwm(duty: in bit<8>, o: out bit) on sys:\n  reg counter: bit<8> = 0\n  on rising(sys):\n    counter = counter +% 1\n  comb:\n    o = counter[7:0] < duty\nsync a -> b via two_ff\n";
    let f = ok(src);
    let ItemKind::Circuit(c) = &f.items[1].kind else {
        panic!()
    };
    assert_eq!(c.ports.len(), 2);
    assert_eq!(c.ports[1].dir, PortDir::Out);
    assert_eq!(c.members.len(), 3);
    assert!(matches!(f.items[2].kind, ItemKind::Sync { .. }));
}

#[test]
fn robot_motion() {
    let src = "module r for robot\npoint home = joints(0, -30, 60, 0, 60, 0) deg\nsequence go:\n  move linear to home.above(30 mm) speed 300 mm/s blend 5 mm\n  move ptp to home\n";
    let f = ok(src);
    let ItemKind::Sequence(s) = &f.items[1].kind else {
        panic!()
    };
    let SeqItem::Stmt(Stmt {
        kind: StmtKind::Move(m),
        ..
    }) = &s.items[0]
    else {
        panic!()
    };
    assert_eq!(m.kind, MoveKind::Linear);
    assert_eq!(m.options.len(), 2);
}

#[test]
fn cell_items() {
    let src = "cell Line\n  device plc: generic_s7 @ ip(192.168.0.10)\n  place conveyor on plc\n  link a.x -> plc.y via modbus_tcp every 10 ms within 25 ms\n  tag speed: f32 mm/s @ plc.%MD20, ctl.speed\n  zone press: box(x: 0..10) mm\n  mutex press between arm, plc.cyl\n";
    let f = ok(src);
    assert_eq!(f.items.len(), 6);
    let ItemKind::Link(l) = &f.items[2].kind else {
        panic!()
    };
    assert!(l.every.is_some() && l.within.is_some());
}

#[test]
fn nested_generics_close_with_shift_token() {
    ok("module m\nvar q: Option<Result<u8, E>> = none\n");
}

#[test]
fn doc_comments_attach_to_items() {
    let f = ok("module m\n## Adds one.\n## Pure.\nfn inc(x: i32) -> i32 = x + 1\n");
    assert_eq!(f.items[0].doc.as_deref(), Some("Adds one.\nPure."));
}

// ---- errors and recovery ------------------------------------------------

#[test]
fn missing_header_is_reported() {
    assert_eq!(codes("task t every 1 ms:\n  pass\n")[0], "NE0003");
}

#[test]
fn one_error_per_bad_line_and_parsing_continues() {
    let out = parse("module m\nconst a = = 3\nconst b = 4\nfn f(x u8) -> u8 = 1\nconst c = 5\n");
    assert_eq!(out.diagnostics.len(), 2, "{:?}", out.diagnostics);
    let names: Vec<_> = out
        .file
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Const { name, .. } => Some(name.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["b", "c"]);
}

#[test]
fn unclosed_bracket_is_reported_where_it_opens() {
    let out = parse("module m\nconst a = f(1, 2\nconst b = 4\n");
    let d = out
        .diagnostics
        .iter()
        .find(|d| d.code == "NE0011")
        .expect("NE0011");
    assert_eq!(d.span.start, "module m\nconst a = f".len());
}

#[test]
fn missing_block_is_ne0005() {
    assert_eq!(codes("module m\nfn f():\nconst x = 1\n"), vec!["NE0005"]);
}

// ---- properties ---------------------------------------------------------

const FRAGMENTS: &[&str] = &[
    "module m\n",
    "cell C\n",
    "task t every 5 ms:\n",
    "  ",
    "    ",
    "\n",
    "fn f(x: u8) -> u8",
    ":",
    " = x + 1\n",
    "led.toggle()\n",
    "if a > 3 ms:\n",
    "else:\n",
    "-> S when x\n",
    "step A:\n",
    "until x timeout 1 s -> F\n",
    "(",
    ")",
    "[",
    "]",
    ",",
    "set y when z\n",
    "io a: gpio.output @ PA5\n",
    "when t > 9 degC for 2 s:\n",
    "fault Overheat\n",
    "match x:\n",
    "_ => pass\n",
    "|>",
    "->",
    "..",
    "0x1F",
    "8'b1010",
    "%I0.0",
    "\"s\"",
    "#c\n",
    "safety:\n",
    "safe_state:\n",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1500))]

    /// The parser never panics and always terminates on arbitrary input.
    #[test]
    fn parser_is_total(parts in prop::collection::vec(prop::sample::select(FRAGMENTS), 0..60)) {
        let src = parts.concat();
        let _ = parse(&src);
    }

    /// Arbitrary text never panics the parser.
    #[test]
    fn parser_is_total_on_text(src in "\\PC{0,200}") {
        let _ = parse(&src);
    }
}
