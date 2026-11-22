//! Task D3.2: worst-case latency per link, `NE0701` with alternatives, link table.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{fmt_ns, latency, link_table, partition, CellDiag, Library, LinkReport, Trust};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sources(dir: &Path, only: Option<&str>) -> Vec<SourceFile> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .filter(|p| only.is_none_or(|o| p.file_name().is_some_and(|n| n == o)))
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| SourceFile {
            path: f.file_name().unwrap().to_string_lossy().into_owned(),
            text: std::fs::read_to_string(f).unwrap(),
        })
        .collect()
}

fn table(src: Vec<SourceFile>) -> Result<Vec<LinkReport>, Vec<CellDiag>> {
    let a = analyze(src, None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    link_table(&nir, &plan, &lib)
}

fn cell(text: &str) -> Vec<SourceFile> {
    vec![SourceFile {
        path: "c.neuron".into(),
        text: text.into(),
    }]
}

#[test]
fn every_corpus_link_reports_its_budget() {
    let corpus = root().join("examples/corpus/cell");
    let mut rows = 0;
    for name in [
        "x01_two_devices.neuron",
        "x02_robot_zone.neuron",
        "x04_event_from_fpga.neuron",
    ] {
        let t = table(sources(&corpus, Some(name))).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(t.len(), 1, "{name}");
        rows += t.len();
        for r in &t {
            assert!(r.latency.total_ns > 0 && r.meets_budget(), "{name}: {r:?}");
            assert_eq!(r.trust, Trust::Generic, "generic numbers are labeled");
        }
    }
    let bottling = table(sources(&root().join("examples"), None)).unwrap();
    assert_eq!(bottling.len(), 2);
    rows += bottling.len();
    assert_eq!(rows, 5);
    // The FPGA edge is event-driven: no polling term.
    let edge = &bottling[0];
    assert_eq!(
        (edge.sender.as_str(), edge.medium.as_str()),
        ("fpga", "zero_hop")
    );
    assert_eq!(edge.latency.polling_ns, 0);
    assert_eq!(bottling[1].latency.polling_ns, 10_000_000);
}

#[test]
fn a_200_us_link_over_modbus_is_ne0701_with_a_medium_that_fits() {
    let e = table(cell(
        "cell C\n  device a: nucleo_f446re @ usb\n  device b: nucleo_f446re @ usb\n  link a.x -> b.y via modbus_tcp within 200 us\n",
    ))
    .unwrap_err();
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].code, "NE0701");
    assert!(e[0].message.contains("exceeds `within 200 us`"), "{}", e[0]);
    let alt = e[0].help.iter().find(|h| h.starts_with("via zero_hop"));
    assert!(alt.is_some(), "{:?}", e[0].help);
    assert!(e[0].help[0].contains("generic numbers"));
}

#[test]
fn a_budget_no_medium_meets_says_so() {
    // A PLC input cannot see anything in 100 us: its scan alone is 1 ms.
    let e = table(cell(
        "cell C\n  device f: generic_artix7_board @ usb\n  device p: generic_s7_1200 @ usb\n  link f.x -> p.y via zero_hop(gpio, opto_24v) within 100 us\n",
    ))
    .unwrap_err();
    assert_eq!(e[0].code, "NE0701");
    assert!(
        e[0].help.iter().any(|h| h.contains("no medium")),
        "{:?}",
        e[0].help
    );
}

#[test]
fn a_medium_one_end_does_not_speak_is_ne0607() {
    let e = table(cell(
        "cell C\n  device f: generic_artix7_board @ usb\n  device p: generic_s7_1200 @ usb\n  link f.x -> p.y via modbus_tcp within 10 ms\n",
    ))
    .unwrap_err();
    assert_eq!(e[0].code, "NE0607");
    assert!(e[0]
        .help
        .iter()
        .any(|h| h.contains("`f` (generic_artix7_board) speaks zero_hop")));
}

#[test]
fn durations_print_with_their_unit() {
    assert_eq!(fmt_ns(150_000), "150 us");
    assert_eq!(fmt_ns(2_500_000), "2.5 ms");
    assert_eq!(fmt_ns(999), "999 ns");
    assert_eq!(fmt_ns(1_000_000_000), "1 s");
}

proptest! {
    /// The latency is the sum of its terms, grows one for one with the period, and
    /// `NE0701` fires exactly when it exceeds the budget.
    #[test]
    fn latency_is_additive_and_the_check_is_exact(p_us in 1u64..1_000_000, w_us in 1u64..1_000_000) {
        let lib = Library::builtin().unwrap();
        let (s, r) = (lib.device("generic_s7_1200").unwrap(), lib.device("generic_scara").unwrap());
        let m = lib.medium("modbus_tcp").unwrap();
        let l = latency(s, r, m, Some(p_us * 1_000)).unwrap();
        prop_assert_eq!(l.total_ns, l.serialize_ns + l.transport_ns + l.receiver_phase_ns + l.polling_ns);
        let l2 = latency(s, r, m, Some(p_us * 1_000 + 1_000)).unwrap();
        prop_assert_eq!(l2.total_ns, l.total_ns + 1_000);
        let text = format!(
            "cell C\n  device p: generic_s7_1200 @ usb\n  device a: generic_scara @ usb\n  link p.x -> a.y via modbus_tcp every {p_us} us within {w_us} us\n"
        );
        let got = table(cell(&text));
        prop_assert_eq!(got.is_err(), l.total_ns > w_us * 1_000);
    }
}
