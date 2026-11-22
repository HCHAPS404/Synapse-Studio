//! Task D4.3: the model checker over the product of several modules joined by
//! bounded-delay channels (`verify_cell`).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_sema::{
    verify_cell, CellChannel, CellMember, CellModel, CellVerification, ChannelEnd, SemaOptions,
    SourceFile,
};
use proptest::prelude::*;

fn src(path: &str, text: &str) -> SourceFile {
    SourceFile {
        path: path.into(),
        text: text.into(),
    }
}

/// A sender and a receiver in two modules; the cell holds the property.
fn sources(property: &str) -> Vec<SourceFile> {
    vec![
        src(
            "tx.neuron",
            "module tx for mcu\nsignal go: bool\nfsm Tx every 10 ms:\n  initial state Idle:\n    -> Sent when go\n  state Sent:\n    -> Sent when true\n",
        ),
        src(
            "rx.neuron",
            "module rx for mcu\nsignal seen: bool\nfsm Rx every 10 ms:\n  initial state Idle:\n    -> Got when seen\n  state Got:\n    -> Got when true\n",
        ),
        src(
            "cellm.neuron",
            &format!("module cellm for mcu\nverify:\n  {property}\n"),
        ),
    ]
}

fn model(delay_ns: u64) -> CellModel {
    CellModel {
        owner: "cellm".into(),
        members: vec![
            CellMember {
                module: "tx".into(),
                device: "a".into(),
            },
            CellMember {
                module: "rx".into(),
                device: "b".into(),
            },
        ],
        channels: vec![CellChannel {
            from: ChannelEnd::InState(vec!["tx.Tx.Sent".into()]),
            to: "b.seen".into(),
            delay_ns,
        }],
        tick_ns: Some(10_000_000),
    }
}

fn run(property: &str, delay_ns: u64) -> CellVerification {
    verify_cell(sources(property), &model(delay_ns), &SemaOptions::default()).unwrap()
}

#[test]
fn a_value_is_seen_only_after_it_was_written() {
    // The receiver cannot reach `Got` unless the sender was in `Sent` before: the
    // channel never invents a value.
    let v = run("never rx.Rx.Got and not tx.Tx.Sent", 30_000_000);
    assert!(v.counterexamples.is_empty(), "{:?}", v.diagnostics);
    assert_eq!(v.proved.len(), 1);
}

#[test]
fn the_shortest_counterexample_is_found_and_is_plain_data() {
    let v = run("never rx.Rx.Got", 30_000_000);
    assert_eq!(v.counterexamples.len(), 1);
    let c = &v.counterexamples[0];
    // tick 0: go rises; tick 1: Sent and seen at once (delay 0 is allowed); tick 2: Got.
    assert_eq!(c.trace.len(), 3, "{c:?}");
    assert!(c.trace[0].inputs.iter().any(|(k, v)| k == "a.go" && *v));
    assert!(c.trace[1].states.contains(&"tx.Tx.Sent".to_string()));
    assert!(c.trace[2].states.contains(&"rx.Rx.Got".to_string()));
    assert_eq!(c.trace[2].time_ns, 2 * 10_000_000);
    assert!(v.diagnostics.iter().any(|d| d.diag.code == "NE0901"));
}

#[test]
fn the_delay_bound_decides_eventually_within() {
    // Sent at tick 1; with a 30 ms delay the receiver may see it up to 3 ticks late,
    // so `Got` is not certain within 20 ms but is within 100 ms.
    let bad = run(
        "eventually_within 20 ms after tx.Tx.Sent: rx.Rx.Got",
        30_000_000,
    );
    assert_eq!(bad.counterexamples.len(), 1, "{:?}", bad.diagnostics);
    let ok = run(
        "eventually_within 100 ms after tx.Tx.Sent: rx.Rx.Got",
        30_000_000,
    );
    assert!(ok.counterexamples.is_empty(), "{:?}", ok.diagnostics);
    // With a 10 ms delay the 20 ms bound holds.
    let fast = run(
        "eventually_within 30 ms after tx.Tx.Sent: rx.Rx.Got",
        10_000_000,
    );
    assert!(fast.counterexamples.is_empty(), "{:?}", fast.diagnostics);
}

#[test]
fn a_one_tick_pulse_is_not_lost_by_the_channel() {
    let sources = vec![
        src(
            "tx.neuron",
            "module tx for mcu\nsignal go: bool\nfsm Tx every 10 ms:\n  initial state Idle:\n    -> Pulse when go\n  state Pulse:\n    -> Idle when true\n",
        ),
        src(
            "rx.neuron",
            "module rx for mcu\nsignal seen: bool\nfsm Rx every 10 ms:\n  initial state Idle:\n    -> Got when seen\n  state Got:\n    -> Got when true\n",
        ),
        src(
            "cellm.neuron",
            "module cellm for mcu\nverify:\n  eventually_within 100 ms after tx.Tx.Pulse: rx.Rx.Got\n",
        ),
    ];
    let mut m = model(30_000_000);
    m.channels[0].from = ChannelEnd::InState(vec!["tx.Tx.Pulse".into()]);
    let v = verify_cell(sources, &m, &SemaOptions::default()).unwrap();
    assert!(v.counterexamples.is_empty(), "{:?}", v.diagnostics);
    assert_eq!(v.proved.len(), 1);
}

#[test]
fn unknown_modules_are_reported() {
    let mut m = model(0);
    m.owner = "nope".into();
    assert!(verify_cell(sources("never tx.Tx.Sent"), &m, &SemaOptions::default()).is_err());
}

proptest! {
    /// CEX-1: a chain of `n` one-tick states reaches the last one at tick `n`, so the
    /// shortest counterexample of `never Last` has exactly `n + 1` steps, whatever the
    /// delay of an unrelated channel.
    #[test]
    fn counterexamples_are_shortest(n in 1usize..12, delay in 0u64..6) {
        let mut text = String::from("module chain for mcu\nsignal x: bool\nfsm Chain every 10 ms:\n  initial state S0:\n    -> S1 after 10 ms\n");
        for i in 1..n {
            text.push_str(&format!("  state S{i}:\n    -> S{} after 10 ms\n", i + 1));
        }
        text.push_str(&format!("  state S{n}:\n    -> S{n} when true\nverify:\n  never Chain.S{n}\n"));
        let m = CellModel {
            owner: "chain".into(),
            members: vec![],
            channels: vec![CellChannel {
                from: ChannelEnd::Atom("x".into()),
                to: "y".into(),
                delay_ns: delay * 10_000_000,
            }],
            tick_ns: Some(10_000_000),
        };
        let v = verify_cell(vec![src("chain.neuron", &text)], &m, &SemaOptions::default()).unwrap();
        prop_assert_eq!(v.counterexamples.len(), 1);
        prop_assert_eq!(v.counterexamples[0].trace.len(), n + 1);
        // Ticks are consecutive and states change only along the chain.
        for (i, s) in v.counterexamples[0].trace.iter().enumerate() {
            prop_assert_eq!(s.tick, i as u64);
            prop_assert_eq!(&s.states[0], &format!("Chain.S{i}"));
        }
    }
}
