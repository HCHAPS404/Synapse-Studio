//! Task D6.1: co-simulation of a cell on one clock (D-R8, MOC-C1, MOC-C2, partition
//! equivalence, determinism).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{
    cosimulate, cosimulate_single, is_plumbing, partition, CellEventKind, CellPlan, Cut, Drive,
    DriveValue, Library, LinkModel, SimOptions, SimRun,
};
use neuron_nir::Nir;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

const MS: u64 = 1_000_000;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn file(rel: &str) -> SourceFile {
    SourceFile {
        path: Path::new(rel)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        text: std::fs::read_to_string(root().join(rel)).unwrap(),
    }
}

fn src(name: &str, text: &str) -> SourceFile {
    SourceFile {
        path: name.into(),
        text: text.into(),
    }
}

struct Cell {
    sources: Vec<SourceFile>,
    nir: Nir,
    plan: CellPlan,
    lib: Library,
}

fn load(sources: Vec<SourceFile>) -> Cell {
    let a = analyze(sources.clone(), None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    Cell {
        sources,
        nir,
        plan,
        lib,
    }
}

impl Cell {
    fn sim(&self, opts: &SimOptions) -> SimRun {
        cosimulate(&self.plan, &self.nir, &self.sources, &self.lib, opts).unwrap()
    }
}

fn opts(end_ms: u64) -> SimOptions {
    SimOptions {
        end_ns: end_ms * MS,
        ..SimOptions::default()
    }
}

fn lab() -> Vec<SourceFile> {
    vec![
        file("examples/corpus/plc/p01_start_stop.neuron"),
        file("examples/corpus/mcu/m01_blink.neuron"),
        file("examples/corpus/robot/r01_home_and_wave.neuron"),
        file("examples/corpus/hdl/d01_counter.neuron"),
        file("examples/corpus/host/h01_gateway_counter.neuron"),
        src("lab.neuron", "cell Lab\n  device plc: generic_s7_1200 @ ip(192.168.0.10)\n  device board: arduino_uno_r3 @ usb\n  device arm: generic_6axis @ ip(192.168.0.20)\n  device fpga: generic_artix7_board @ usb\n  device pi: raspberry_pi_5 @ ip(192.168.0.30)\n  place p01_start_stop on plc\n  place m01_blink on board\n  place r01_home_and_wave on arm\n  place d01_counter on fpga\n  place h01_gateway_counter on pi\n"),
    ]
}

const PRODUCER_TOGGLE: &str =
    "module producer for host\nsignal ready: bool\ntask tick every 20 ms:\n  ready = not ready\n";
const CONSUMER: &str = "module consumer for host\nsignal start: bool\nsignal seen: u32\ntask poll every 5 ms:\n  if start:\n    seen = seen + 1\n";
const PRODUCER_PLAIN: &str = "module producer for host\nsignal ready: u32\n";

fn pair_cell(every_ms: u64) -> String {
    format!("cell Pair\n  device a: raspberry_pi_5 @ ip(192.168.0.30)\n  device b: raspberry_pi_5 @ ip(192.168.0.31)\n  place producer on a\n  place consumer on b\n  link a.ready -> b.start via modbus_tcp every {every_ms} ms within 200 ms\n")
}

fn pair(producer: &str, every_ms: u64) -> Cell {
    load(vec![
        src("producer.neuron", producer),
        src("consumer.neuron", CONSUMER),
        src("pair.neuron", &pair_cell(every_ms)),
    ])
}

#[test]
fn every_corpus_cell_runs() {
    for name in [
        "x01_two_devices",
        "x02_robot_zone",
        "x03_cycle_sequence",
        "x04_event_from_fpga",
    ] {
        let c = load(vec![file(&format!("examples/corpus/cell/{name}.neuron"))]);
        let run = c.sim(&opts(300));
        assert!(!run.devices.is_empty(), "{name}");
        assert!(
            run.events.iter().all(|e| e.kind != CellEventKind::Fault),
            "{name}: no link was cut, so no fault: {}",
            run.render()
        );
        assert!(
            run.events.windows(2).all(|w| w[0].t_ns <= w[1].t_ns),
            "{name}: time ordered"
        );
    }
}

#[test]
fn the_five_class_lab_cell_runs() {
    let c = load(lab());
    let run = c.sim(&opts(1500));
    assert_eq!(run.devices.len(), 5);
    assert!(run.devices.iter().filter(|d| d.runs).count() >= 4);
    // The PLC motor program, the board LED and the Python-class counter all ran.
    let tags: Vec<_> = run
        .events
        .iter()
        .filter(|e| !is_plumbing(&e.name))
        .map(|e| format!("{}:{}", e.device, e.name))
        .collect();
    assert!(tags.iter().any(|t| t.starts_with("board:")), "{tags:?}");
    assert!(
        tags.iter().any(|t| t.starts_with("pi:messages")),
        "{tags:?}"
    );
    assert!(run.events.iter().all(|e| e.kind != CellEventKind::Fault));
}

#[test]
fn a_value_written_by_the_sender_arrives_after_exactly_the_link_latency() {
    let c = pair(PRODUCER_PLAIN, 10);
    let drives = vec![Drive {
        t_ns: 7 * MS,
        device: "a".into(),
        name: "ready".into(),
        value: DriveValue::Int(42),
    }];
    let run = c.sim(&SimOptions {
        drives,
        ..opts(300)
    });
    let l = &run.links[0];
    assert!(l.latency_ns > 0 && l.latency_ns <= l.within_ns.unwrap());
    assert_eq!(run.deliveries.len(), 1, "{}", run.render());
    let d = &run.deliveries[0];
    assert_eq!((d.sent_ns, d.value.as_str()), (7 * MS, "42"));
    assert_eq!(d.delivered_ns - d.sent_ns, l.latency_ns);
    // The receiver's own signals show the value at the delivery instant.
    // `start` is a bool, so 42 reads as true there; the endpoint keeps the number.
    for (name, shown) in [("start", "true"), ("l0_value", "42")] {
        let e = run
            .events
            .iter()
            .find(|e| e.device == "b" && e.kind == CellEventKind::Signal && e.name == name)
            .unwrap_or_else(|| panic!("no `{name}` on b:\n{}", run.render()));
        assert_eq!(
            (e.t_ns, e.value.as_str()),
            (d.delivered_ns, shown),
            "{name}"
        );
    }
}

#[test]
fn the_endpoint_signal_of_a_corpus_cell_crosses_its_link() {
    // x01: nothing is placed, so the value is the endpoint's own `l0_value`.
    let c = load(vec![file("examples/corpus/cell/x01_two_devices.neuron")]);
    let run = c.sim(&SimOptions {
        drives: vec![Drive {
            t_ns: 3 * MS,
            device: "ctl".into(),
            name: "l0_value".into(),
            value: DriveValue::Bool(true),
        }],
        ..opts(200)
    });
    assert_eq!(run.deliveries.len(), 1, "{}", run.render());
    let d = &run.deliveries[0];
    assert_eq!(d.delivered_ns - d.sent_ns, run.links[0].latency_ns);
    assert!(d.delivered_ns - d.sent_ns <= run.links[0].within_ns.unwrap());
}

#[test]
fn cutting_a_link_puts_the_receiver_in_its_safe_state_within_the_bound() {
    let consumer = "module consumer for host\nsignal start: bool\nio led: gpio.output @ GPIO17\ntask poll every 5 ms:\n  led.write(start)\nsafety:\n  safe_state:\n    led.write(false)\n";
    let c = load(vec![
        src(
            "producer.neuron",
            "module producer for host\nsignal ready: bool\n",
        ),
        src("consumer.neuron", consumer),
        src("pair.neuron", &pair_cell(10)),
    ]);
    let run = c.sim(&SimOptions {
        drives: vec![Drive {
            t_ns: 20 * MS,
            device: "a".into(),
            name: "ready".into(),
            value: DriveValue::Bool(true),
        }],
        cuts: vec![Cut {
            link: 0,
            at_ns: 100 * MS,
        }],
        ..opts(400)
    });
    let loss = &run.losses[0];
    assert_eq!((loss.link, loss.device.as_str()), (0, "b"));
    let fault = loss.fault_ns.unwrap_or_else(|| panic!("{}", run.render()));
    assert!(fault >= 100 * MS + 50 * MS - 20 * MS, "after the watchdog");
    assert!(loss.within_bound, "{loss:?}");
    assert_eq!(loss.bound_ns, run.links[0].bound_ns);
    // The LED was on, and its last write is the safe value, no later than the bound.
    let leds: Vec<_> = run
        .events
        .iter()
        .filter(|e| e.device == "b" && e.kind == CellEventKind::Output && e.name == "led")
        .collect();
    assert!(leds.iter().any(|e| e.value == "true"), "{}", run.render());
    let last = leds.last().unwrap();
    assert_eq!(last.value, "false");
    assert!(last.t_ns <= 100 * MS + loss.bound_ns, "{}", run.render());
    // A value sent after the cut is dropped, not delivered.
    let run2 = c.sim(&SimOptions {
        drives: vec![Drive {
            t_ns: 150 * MS,
            device: "a".into(),
            name: "ready".into(),
            value: DriveValue::Bool(true),
        }],
        cuts: vec![Cut {
            link: 0,
            at_ns: 100 * MS,
        }],
        ..opts(400)
    });
    assert!(run2.deliveries.is_empty());
    assert!(run2.events.iter().any(|e| e.kind == CellEventKind::Drop));
}

#[test]
fn cutting_the_corpus_links_faults_the_plc_receivers_within_the_bound() {
    for name in ["x01_two_devices", "x04_event_from_fpga"] {
        let c = load(vec![file(&format!("examples/corpus/cell/{name}.neuron"))]);
        let run = c.sim(&SimOptions {
            cuts: vec![Cut {
                link: 0,
                at_ns: 40 * MS,
            }],
            ..opts(400)
        });
        let l = &run.losses[0];
        assert_eq!(l.device, "plc", "{name}");
        assert!(l.within_bound, "{name}: {l:?}\n{}", run.render());
        assert!(
            l.fault_ns.unwrap() >= 40 * MS + 40 * MS,
            "{name}: not early"
        );
    }
}

#[test]
fn a_robot_receiver_has_no_watchdog_logic_yet_and_the_run_says_so() {
    // x02: the SPEL+ emitter has no background task, so the robot endpoint declares
    // `alive` and `lost` without logic (task D3.3). The bound is in the plan, but the
    // interpreted robot never faults: the report must not claim otherwise.
    let c = load(vec![file("examples/corpus/cell/x02_robot_zone.neuron")]);
    let run = c.sim(&SimOptions {
        cuts: vec![Cut {
            link: 0,
            at_ns: 40 * MS,
        }],
        ..opts(400)
    });
    let l = &run.losses[0];
    assert_eq!(l.device, "arm");
    assert!(!l.within_bound && l.fault_ns.is_none(), "{l:?}");
}

#[test]
fn without_a_cut_the_watchdog_stays_quiet() {
    let c = pair(PRODUCER_TOGGLE, 10);
    let run = c.sim(&opts(500));
    assert!(run.losses.is_empty());
    assert!(run.events.iter().all(|e| e.kind != CellEventKind::Fault));
    assert!(!run.deliveries.is_empty());
}

#[test]
fn a_cut_of_a_link_that_does_not_exist_is_refused() {
    let c = pair(PRODUCER_TOGGLE, 10);
    let e = cosimulate(
        &c.plan,
        &c.nir,
        &c.sources,
        &c.lib,
        &SimOptions {
            cuts: vec![Cut { link: 5, at_ns: 0 }],
            ..opts(10)
        },
    )
    .unwrap_err();
    assert!(e.to_string().contains("no link number 5"));
}

#[test]
fn the_same_cell_gives_a_byte_identical_trace() {
    let c = pair(PRODUCER_TOGGLE, 10);
    let o = SimOptions {
        cuts: vec![Cut {
            link: 0,
            at_ns: 233 * MS,
        }],
        ..opts(600)
    };
    let (a, b) = (c.sim(&o), c.sim(&o));
    assert_eq!(a.render(), b.render());
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
    let lab = load(lab());
    assert_eq!(lab.sim(&opts(500)).render(), lab.sim(&opts(500)).render());
}

/// The observable behaviour without link plumbing, as sortable lines.
fn behaviour_lines(
    events: impl Iterator<Item = (u64, CellEventKind, String, String)>,
) -> Vec<String> {
    let mut v: Vec<String> = events
        .filter(|(_, k, n, _)| {
            !is_plumbing(n)
                && !matches!(
                    k,
                    CellEventKind::Send
                        | CellEventKind::Deliver
                        | CellEventKind::Cut
                        | CellEventKind::Drop
                )
        })
        .map(|(t, k, n, val)| format!("{t:>12} {} {n} = {val}", k.tag()))
        .collect();
    v.sort();
    v
}

fn equivalence(c: &Cell, end_ms: u64) -> (Vec<String>, Vec<String>) {
    let o = SimOptions {
        link_model: LinkModel::Ideal,
        ..opts(end_ms)
    };
    let run = c.sim(&o);
    let one = cosimulate_single(&c.plan, &c.nir, &c.lib, &o).unwrap();
    (
        behaviour_lines(
            run.events
                .iter()
                .map(|e| (e.t_ns, e.kind, e.name.clone(), e.value.clone())),
        ),
        behaviour_lines(
            one.iter()
                .map(|e| (e.t_ns, e.kind, e.name.clone(), e.value.clone())),
        ),
    )
}

#[test]
fn partitioned_programs_with_ideal_links_match_one_interpreter() {
    let c = pair(PRODUCER_TOGGLE, 10);
    let (cell, single) = equivalence(&c, 500);
    assert!(
        cell.iter().any(|l| l.contains("start = true")),
        "the link carried values: {cell:?}"
    );
    assert_eq!(cell, single);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    /// MOC-C1: every delivery takes exactly the link latency, which is within the budget.
    #[test]
    fn delivery_is_bounded_by_the_link_latency(
        every in 1u64..60,
        times in proptest::collection::btree_set(1u64..250, 1..6),
    ) {
        let c = pair(PRODUCER_PLAIN, every);
        let drives = times.iter().enumerate().map(|(i, t)| Drive {
            t_ns: t * MS,
            device: "a".into(),
            name: "ready".into(),
            value: DriveValue::Int(i64::try_from(i).unwrap() + 1),
        }).collect::<Vec<_>>();
        let n = drives.len();
        let run = c.sim(&SimOptions { drives, ..opts(400) });
        let l = &run.links[0];
        prop_assert_eq!(run.deliveries.len(), n);
        for d in &run.deliveries {
            prop_assert_eq!(d.delivered_ns - d.sent_ns, l.latency_ns);
            prop_assert!(d.delivered_ns - d.sent_ns <= l.within_ns.unwrap());
        }
        // FIFO: values arrive in the order they were sent.
        let sent: Vec<_> = run.deliveries.iter().map(|d| d.sent_ns).collect();
        prop_assert!(sent.windows(2).all(|w| w[0] < w[1]));
    }

    /// Determinism: random stimuli and cuts give a byte-identical trace twice.
    #[test]
    fn random_runs_are_deterministic(
        every in 1u64..40,
        times in proptest::collection::btree_set(1u64..250, 0..5),
        cut in proptest::option::of(1u64..300),
    ) {
        let c = pair(PRODUCER_PLAIN, every);
        let o = SimOptions {
            drives: times.iter().map(|t| Drive {
                t_ns: t * MS,
                device: "a".into(),
                name: "ready".into(),
                value: DriveValue::Int(i64::try_from(*t).unwrap()),
            }).collect(),
            cuts: cut.map(|c| Cut { link: 0, at_ns: c * MS }).into_iter().collect(),
            ..opts(350)
        };
        prop_assert_eq!(c.sim(&o).render(), c.sim(&o).render());
    }

    /// MOC-C2: whenever the link is cut, the receiver faults within watchdog + safe state.
    #[test]
    fn a_cut_always_ends_in_the_safe_state_within_the_bound(cut in 1u64..200) {
        let c = pair(PRODUCER_TOGGLE, 10);
        let run = c.sim(&SimOptions {
            cuts: vec![Cut { link: 0, at_ns: cut * MS }],
            ..opts(cut + 300)
        });
        prop_assert!(run.losses[0].within_bound, "{:?}", run.losses[0]);
    }

    /// Partition equivalence for two-module cells over random periods.
    #[test]
    fn equivalence_over_random_periods(p in 3u64..40, q in 2u64..30) {
        let producer = format!("module producer for host\nsignal ready: bool\ntask tick every {p} ms:\n  ready = not ready\n");
        let consumer = format!("module consumer for host\nsignal start: bool\nsignal seen: u32\ntask poll every {q} ms:\n  if start:\n    seen = seen + 1\n");
        let c = load(vec![
            src("producer.neuron", &producer),
            src("consumer.neuron", &consumer),
            src("pair.neuron", &pair_cell(10)),
        ]);
        let (cell, single) = equivalence(&c, 300);
        prop_assert_eq!(cell, single);
    }
}
