//! Task D3.4: tags across devices, `NE0703`, single writer.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{
    address_fits, check_tags, parse_direct_address, partition, type_bits, CellDiag, Library,
};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn text(path: &str, text: &str) -> SourceFile {
    SourceFile {
        path: path.into(),
        text: text.into(),
    }
}
fn check(sources: Vec<SourceFile>) -> Vec<CellDiag> {
    let a = analyze(sources, None, &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    let nir = a.nir.clone().unwrap();
    let lib = Library::builtin().unwrap();
    let plan = partition(&nir, &lib).unwrap().remove(0);
    check_tags(&nir, &plan, &lib)
}
fn dir_sources(rel: &str, only: Option<&str>) -> Vec<SourceFile> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(root().join(rel))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .filter(|p| only.is_none_or(|o| p.file_name().is_some_and(|n| n == o)))
        .collect();
    files.sort();
    files
        .iter()
        .map(|p| SourceFile {
            path: p.file_name().unwrap().to_string_lossy().into_owned(),
            text: std::fs::read_to_string(p).unwrap(),
        })
        .collect()
}

const HEAD: &str = "cell C\n  device a: generic_s7_1200 @ ip(192.168.0.10)\n  device b: generic_s7_1200 @ ip(192.168.0.11)\n  device c: generic_s7_1200 @ ip(192.168.0.12)\n";
fn cell(extra: &str) -> Vec<SourceFile> {
    vec![text("c.neuron", &format!("{HEAD}{extra}"))]
}
fn plc_module(name: &str, memory: &str, rung: &str) -> SourceFile {
    text(
        &format!("{name}.neuron"),
        &format!("module {name} for plc\nscan 10 ms\nmemory:\n  {memory}\n{rung}"),
    )
}

#[test]
fn the_tags_of_the_corpus_and_of_the_bottling_package_check() {
    for name in ["x01_two_devices.neuron", "x04_event_from_fpga.neuron"] {
        let d = check(dir_sources("examples/corpus/cell", Some(name)));
        assert!(d.is_empty(), "{name}: {d:?}");
    }
    // The bottling cell places modules of other files: the whole package.
    let d = check(dir_sources("examples", None));
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn a_unit_mismatch_between_two_devices_is_ne0703() {
    let mut s = cell("  place ma on a\n  place mb on b\n  tag v: f32 mm/s @ a.speed, b.speed\n");
    s.push(plc_module("ma", "speed: f32 mm/s", ""));
    s.push(plc_module("mb", "speed: f32 m/s", ""));
    let d = check(s);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, "NE0703");
    assert!(
        d[0].message.contains("b.speed") && d[0].message.contains("m/s"),
        "{}",
        d[0]
    );
}

#[test]
fn a_type_mismatch_between_two_devices_is_ne0703() {
    let mut s = cell("  place ma on a\n  place mb on b\n  tag v: f32 mm/s @ a.speed, b.speed\n");
    s.push(plc_module("ma", "speed: f32 mm/s", ""));
    s.push(plc_module("mb", "speed: f64 mm/s", ""));
    let d = check(s);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, "NE0703");
    assert!(d[0].message.contains("f64"), "{}", d[0]);
}

#[test]
fn matching_symbols_and_an_undeclared_symbol_pass() {
    let mut s = cell("  place ma on a\n  tag v: f32 mm/s @ a.speed, b.speed\n");
    s.push(plc_module("ma", "speed: f32 mm/s", ""));
    assert!(check(s).is_empty());
}

#[test]
fn an_address_of_the_wrong_size_is_ne0703() {
    for (ty, addr) in [
        ("f32", "%MW20"),
        ("f32", "%MX0.0"),
        ("bool", "%MD20"),
        ("u16", "%MD20"),
        ("u8", "%MW4"),
        ("f64", "%MD8"),
    ] {
        let d = check(cell(&format!("  tag t: {ty} @ a.{addr}\n")));
        assert_eq!(d.len(), 1, "{ty} {addr}: {d:?}");
        assert_eq!(d[0].code, "NE0703", "{ty} {addr}");
    }
    for (ty, addr) in [
        ("f32", "%MD20"),
        ("u32", "%MD30"),
        ("i32", "%MD4"),
        ("u16", "%MW2"),
        ("u8", "%MB1"),
        ("f64", "%ML8"),
        ("bool", "%QX0.0"),
        ("bool", "%I0.1"),
    ] {
        let d = check(cell(&format!("  tag t: {ty} @ a.{addr}\n")));
        assert!(d.is_empty(), "{ty} {addr}: {d:?}");
    }
}

#[test]
fn an_address_family_the_device_does_not_have_is_ne0703() {
    let src = "cell C\n  device ctl: nucleo_f446re @ usb\n  tag t: bool @ ctl.%IX0.0\n";
    let d = check(vec![text("c.neuron", src)]);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, "NE0703");
    assert!(d[0].message.contains("address family"), "{}", d[0]);
    // `generic_iec_plc` has `%MW`, `%IX`, `%QX`: `%MD20` shares the area `%M`.
    let ok = "cell C\n  device p: generic_iec_plc @ ip(10.0.0.1)\n  tag t: f32 @ p.%MD20\n";
    assert!(check(vec![text("c.neuron", ok)]).is_empty());
}

#[test]
fn a_location_on_an_unknown_device_is_stopped_by_the_analyzer() {
    // The name check of the analyzer (`NE0101`) comes first; `check_tags` keeps `NE0607`
    // for a name that resolves but is not a device of the cell.
    let a = analyze(
        cell("  tag t: bool @ zzz.%I0.0\n"),
        None,
        &SemaOptions::default(),
    );
    assert!(a.errors().any(|e| e.diag.code == "NE0101"));
}

#[test]
fn two_output_addresses_are_two_writers() {
    let d = check(cell("  tag t: bool @ a.%Q0.0, b.%Q0.0\n"));
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, "NE0703");
    assert!(d[0].message.contains("2 writers: a, b"), "{}", d[0]);
    // An output and an input is one writer.
    assert!(check(cell("  tag t: bool @ a.%Q0.0, b.%I0.0\n")).is_empty());
}

#[test]
fn two_modules_that_assign_the_symbol_are_two_writers_and_a_reader_is_not() {
    let writer = "rung r:\n  go = true\n";
    let mut s = cell(
        "  place ma on a\n  place mb on b\n  place mc on c\n  tag go: bool @ a.go, b.go, c.go\n",
    );
    s.push(plc_module("ma", "go: bool", writer));
    s.push(plc_module("mb", "go: bool", writer));
    s.push(plc_module("mc", "go: bool", ""));
    let d = check(s);
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].message.contains("2 writers: a, b"), "{}", d[0]);
    // With one writer and two readers the tag is fine.
    let mut s = cell("  place ma on a\n  place mb on b\n  tag go: bool @ a.go, b.go\n");
    s.push(plc_module("ma", "go: bool", writer));
    s.push(plc_module("mb", "go: bool", ""));
    assert!(check(s).is_empty());
}

#[test]
fn a_memory_address_is_a_write_location_when_its_variable_is_assigned() {
    let writer = "rung r:\n  go = true\n";
    let mut s = cell("  place ma on a\n  place mb on b\n  tag go: bool @ a.%M0.0, b.%Q0.0\n");
    s.push(plc_module("ma", "go: bool @ %M0.0", writer));
    s.push(plc_module("mb", "other: bool", ""));
    let d = check(s);
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].message.contains("2 writers: a, b"), "{}", d[0]);
}

#[test]
fn the_text_of_a_diagnostic_names_the_tag() {
    let d = check(cell("  tag speed_cmd: f32 @ a.%MW2\n"));
    assert!(d[0].message.contains("speed_cmd"), "{}", d[0]);
}

fn kinds() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "bool", "u8", "i8", "u16", "i16", "u32", "i32", "f32", "u64", "i64", "f64",
    ])
}

proptest! {
    /// A type fits an address exactly when their widths are equal, and a `bool` needs a
    /// bit address.
    #[test]
    fn a_type_fits_the_address_of_its_width(
        kind in kinds(),
        area in prop::sample::select(vec!['I', 'Q', 'M']),
        size in prop::sample::select(vec!["X", "B", "W", "D", "L", ""]),
        n in 0u32..5000,
    ) {
        let address = format!("%{area}{size}{n}");
        let addr_bits = match size { "B" => 8, "W" => 16, "D" => 32, "L" => 64, _ => 1 };
        let parsed = parse_direct_address(&address).unwrap();
        prop_assert_eq!(parsed.bits, addr_bits);
        prop_assert_eq!(parsed.area, area);
        prop_assert_eq!(address_fits(kind, &address), type_bits(kind) == Some(addr_bits));
        prop_assert_eq!(address_fits("bool", &address), addr_bits == 1);
    }

    /// Text that is not an address never parses (and never panics).
    #[test]
    fn arbitrary_text_does_not_panic(s in "\\PC{0,12}") {
        let _ = parse_direct_address(&s);
        let _ = address_fits("f32", &s);
    }

    /// The diagnostics of a tag do not depend on the order of its locations.
    #[test]
    fn the_writer_rule_does_not_depend_on_the_location_order(
        order in Just(vec!["a.%Q0.0", "b.%Q0.0", "c.%I0.0", "a.%MD2"]).prop_shuffle()
    ) {
        let messages = |locs: &[&str]| -> Vec<String> {
            let mut m: Vec<String> = check(cell(&format!("  tag t: bool @ {}\n", locs.join(", "))))
                .into_iter()
                .map(|d| d.message)
                .collect();
            m.sort();
            m
        };
        let canonical = messages(&["a.%Q0.0", "b.%Q0.0", "c.%I0.0", "a.%MD2"]);
        prop_assert_eq!(messages(&order), canonical);
    }
}
