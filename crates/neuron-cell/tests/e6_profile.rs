//! Task E6.3: calibration profiles, the timing overlay and `NE1112`.
//!
//! A profile overrides the manifest timing of a device on a medium. These tests prove a
//! synthetic profile changes an `NE0701` result both ways (a failing link passes, a passing
//! link fails) and that an invalid profile is `NE1112`. Numbers are synthetic; real numbers
//! come from session I1.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{
    link_table, link_table_with, partition, CellDiag, Library, LinkReport, TimingOverlay,
    TimingProfile, Trust, MIN_MEASURED_SAMPLES,
};
use neuron_sema::{analyze, SemaOptions, SourceFile};

fn cell(text: &str) -> Vec<SourceFile> {
    vec![SourceFile {
        path: "c.neuron".into(),
        text: text.into(),
    }]
}

fn table_with(text: &str, overlay: &TimingOverlay) -> Result<Vec<LinkReport>, Vec<CellDiag>> {
    let a = analyze(cell(text), None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    link_table_with(&nir, &plan, &lib, overlay)
}

// Two NUCLEO boards over `zero_hop` (event-driven, no polling). Default worst case is
// 10us+5us + 20us + 10us+5us (transport) + 100us (receiver scan) = 150 us.
const LINK_130: &str =
    "cell C\n  device a: nucleo_f446re @ usb\n  device b: nucleo_f446re @ usb\n  link a.x -> b.y via zero_hop within 130 us\n";

fn profile(trust: &str, samples: u64, transport: &str, jitter: &str) -> String {
    format!(
        "[profile]\ndevice = \"nucleo_f446re\"\nmedium = \"zero_hop\"\ntrust = \"{trust}\"\nsamples = {samples}\ntransport = \"{transport}\"\njitter = \"{jitter}\"\n"
    )
}

#[test]
fn a_profile_makes_a_failing_link_pass() {
    // No profile: 150 us > 130 us -> NE0701.
    let e = table_with(LINK_130, &TimingOverlay::new()).unwrap_err();
    assert_eq!(e[0].code, "NE0701", "{e:?}");

    // A measured profile with fast transport/jitter: 1us+1us + 20us + 1us+1us + 100us = 124 us.
    let mut overlay = TimingOverlay::new();
    overlay
        .load("fast.toml", &profile("measured", 2000, "1 us", "1 us"))
        .unwrap();
    let rows = table_with(LINK_130, &overlay).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].meets_budget(), "{:?}", rows[0]);
    assert_eq!(rows[0].latency.total_ns, 124_000);
    // The link's trust is the lowest of every number used: the device ends are now `measured`
    // but the `zero_hop` medium number is still `generic`, so the link reports `generic`. A
    // link reports `measured` only when the medium is measured too (session I1 data commit).
    assert_eq!(rows[0].trust, Trust::Generic);
}

#[test]
fn a_profile_makes_a_passing_link_fail() {
    // A roomy budget the default (150 us) meets.
    let link = LINK_130.replace("within 130 us", "within 160 us");
    assert!(table_with(&link, &TimingOverlay::new()).unwrap()[0].meets_budget());

    // A slow measured profile pushes it over: 20us+10us + 20us + 20us+10us + 100us = 180 us.
    let mut overlay = TimingOverlay::new();
    overlay
        .load("slow.toml", &profile("measured", 500, "20 us", "10 us"))
        .unwrap();
    let e = table_with(&link, &overlay).unwrap_err();
    assert_eq!(e[0].code, "NE0701", "{e:?}");
    assert!(
        e[0].message.contains("exceeds `within 160 us`"),
        "{}",
        e[0].message
    );
}

#[test]
fn an_empty_overlay_matches_link_table() {
    let a = analyze(cell(LINK_130), None, &SemaOptions::default());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    let plain = link_table(&nir, &plan, &lib);
    let overlaid = link_table_with(&nir, &plan, &lib, &TimingOverlay::new());
    assert_eq!(plain, overlaid);
}

#[test]
fn a_generic_profile_keeps_generic_trust() {
    let p = TimingProfile::parse("g.toml", &profile("generic", 0, "1 us", "1 us")).unwrap();
    assert_eq!(p.trust, Trust::Generic);
    let mut overlay = TimingOverlay::new();
    overlay.insert(p);
    let rows = table_with(LINK_130, &overlay).unwrap();
    assert_eq!(rows[0].trust, Trust::Generic);
}

#[test]
fn an_unknown_unit_is_ne1112() {
    let e = TimingProfile::parse("p.toml", &profile("measured", 2000, "2 furlongs", "1 us"))
        .unwrap_err();
    assert_eq!(e.code, "NE1112");
    assert!(e.message.contains("transport"), "{}", e.message);
}

#[test]
fn a_non_positive_value_is_ne1112() {
    let e = TimingProfile::parse("p.toml", &profile("measured", 2000, "0 us", "1 us")).unwrap_err();
    assert_eq!(e.code, "NE1112");
    assert!(e.message.contains("positive"), "{}", e.message);
}

#[test]
fn too_few_samples_for_measured_is_ne1112() {
    let n = MIN_MEASURED_SAMPLES - 1;
    let e = TimingProfile::parse("p.toml", &profile("measured", n, "1 us", "1 us")).unwrap_err();
    assert_eq!(e.code, "NE1112");
    assert!(e.message.contains("samples"), "{}", e.message);
}

#[test]
fn an_unknown_field_is_ne1112() {
    let text = format!("{}oops = 1\n", profile("measured", 2000, "1 us", "1 us"));
    let e = TimingProfile::parse("p.toml", &text).unwrap_err();
    assert_eq!(e.code, "NE1112");
}

#[test]
fn a_generic_profile_may_omit_samples() {
    // A generic profile is not held to the measured-sample floor.
    let p = TimingProfile::parse("g.toml", &profile("generic", 0, "3 ms", "1 ms")).unwrap();
    assert_eq!(p.transport_ns, 3_000_000);
    assert_eq!(p.jitter_ns, 1_000_000);
}
