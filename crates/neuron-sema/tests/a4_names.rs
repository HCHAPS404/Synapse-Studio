//! Task A4: names, modules and packages (NE0101 to NE0105, NE0108).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

#[test]
fn unknown_name_suggests_a_fix() {
    let src = "module t\nfn area(width: f32 mm) -> f32 mm:\n  return widht\n";
    let a = run(src);
    assert_eq!(errors(&a), ["NE0101"], "{}", a.render());
    assert!(
        a.render().contains("did you mean `width`?"),
        "{}",
        a.render()
    );
}

#[test]
fn known_names_resolve() {
    ok("module t\nconst limit = 10\nfn f(x: u8) -> u8:\n  let y = x\n  return y\n");
}

#[test]
fn duplicate_definitions_are_reported() {
    only(
        "module t\nfn a() -> bool:\n  return true\nfn a() -> bool:\n  return false\n",
        &["NE0102"],
    );
    only(
        "module t\nfn f(x: u8) -> u8:\n  let y = 1\n  let y = 2\n  return x\n",
        &["NE0102"],
    );
    only(
        "module t\nfn f(x: u8, x: u8) -> u8:\n  return x\n",
        &["NE0102"],
    );
}

#[test]
fn shadowing_in_a_nested_scope_is_allowed() {
    ok("module t\nfn f(x: u8) -> u8:\n  if x > 1:\n    let x: u8 = 5\n    return x\n  return x\n");
}

const MODE: &str = "module t\nenum Mode:\n  A\n  B\n  C\n";

#[test]
fn non_exhaustive_enum_match() {
    let src = format!("{MODE}fn f(m: Mode):\n  match m:\n    Mode.A => pass\n    Mode.B => pass\n");
    let a = run(&src);
    assert_eq!(errors(&a), ["NE0103"], "{}", a.render());
    assert!(a.render().contains("Mode.C"), "{}", a.render());
}

#[test]
fn exhaustive_matches_pass() {
    let all = format!("{MODE}fn f(m: Mode):\n  match m:\n    Mode.A => pass\n    Mode.B => pass\n    Mode.C => pass\n");
    ok(&all);
    let wild = format!("{MODE}fn f(m: Mode):\n  match m:\n    Mode.A => pass\n    _ => pass\n");
    ok(&wild);
    let alt = format!(
        "{MODE}fn f(m: Mode):\n  match m:\n    Mode.A | Mode.B => pass\n    Mode.C => pass\n"
    );
    ok(&alt);
}

#[test]
fn guarded_arms_do_not_count_for_exhaustiveness() {
    let src = format!(
        "{MODE}fn f(m: Mode, ok: bool):\n  match m:\n    Mode.A => pass\n    Mode.B => pass\n    Mode.C if ok => pass\n"
    );
    only(&src, &["NE0103"]);
}

#[test]
fn integer_ranges_must_cover_the_type() {
    ok("module t\nfn f(x: u8):\n  match x:\n    0..=127 => pass\n    128..=255 => pass\n");
    only(
        "module t\nfn f(x: u8):\n  match x:\n    0..=9 => pass\n",
        &["NE0103"],
    );
}

#[test]
fn bool_match_needs_both_values() {
    ok("module t\nfn f(x: bool):\n  match x:\n    true => pass\n    false => pass\n");
    only(
        "module t\nfn f(x: bool):\n  match x:\n    true => pass\n",
        &["NE0103"],
    );
}

#[test]
fn circular_imports_are_reported() {
    let a = pkg(
        &[
            (
                "a.neuron",
                "module a\nuse b.{g}\npub fn f() -> bool:\n  return true\n",
            ),
            (
                "b.neuron",
                "module b\nuse a.{f}\npub fn g() -> bool:\n  return true\n",
            ),
        ],
        None,
    );
    assert!(errors(&a).contains(&"NE0104"), "{}", a.render());
    assert!(a.render().contains("a -> b -> a") || a.render().contains("b -> a -> b"));
}

#[test]
fn private_items_cannot_be_imported() {
    let a = pkg(
        &[
            (
                "a.neuron",
                "module a\nuse b.{secret}\nfn f() -> u8:\n  return secret()\n",
            ),
            ("b.neuron", "module b\nfn secret() -> u8:\n  return 1\n"),
        ],
        None,
    );
    assert_eq!(errors(&a), ["NE0105"], "{}", a.render());
}

#[test]
fn private_items_cannot_be_used_through_a_module() {
    let a = pkg(
        &[
            (
                "a.neuron",
                "module a\nuse b\nfn f() -> u8:\n  return b.secret()\n",
            ),
            (
                "b.neuron",
                "module b\nfn secret() -> u8:\n  return 1\npub fn open() -> u8:\n  return 2\n",
            ),
        ],
        None,
    );
    assert_eq!(errors(&a), ["NE0105"], "{}", a.render());
}

#[test]
fn public_items_can_be_imported_and_used() {
    let a = pkg(
        &[
            (
                "a.neuron",
                "module a\nuse b.{open}\nuse b as bb\nfn f() -> u8:\n  return open() + bb.open()\n",
            ),
            ("b.neuron", "module b\npub fn open() -> u8:\n  return 2\n"),
        ],
        None,
    );
    assert!(a.is_ok(), "{}", a.render());
}

#[test]
fn missing_items_and_modules_are_unknown() {
    let a = pkg(
        &[
            ("a.neuron", "module a\nuse b.{nothing}\nuse c.{x}\n"),
            ("b.neuron", "module b\npub fn open() -> u8:\n  return 2\n"),
        ],
        None,
    );
    assert_eq!(errors(&a), ["NE0101", "NE0101"], "{}", a.render());
}

#[test]
fn standard_library_layers_are_checked() {
    ok("module t for mcu\nuse std.control.{pid, lowpass}\n");
    only("module t for mcu\nuse std.nothing.{x}\n", &["NE0101"]);
    only("module t for mcu\nuse std\n", &["NE0101"]);
}

#[test]
fn dependencies_come_from_the_manifest() {
    let manifest = "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2026\"\n\n[dependencies]\ndisplays = { git = \"https://example.org/d\", tag = \"v0.3.0\" }\n";
    let a = pkg(
        &[("a.neuron", "module a\nuse displays.panel\n")],
        Some(manifest),
    );
    assert!(a.is_ok(), "{}", a.render());
    let b = pkg(
        &[("a.neuron", "module a\nuse others.panel\n")],
        Some(manifest),
    );
    assert_eq!(errors(&b), ["NE0101"]);
}

#[test]
fn self_refers_to_the_package_root() {
    let manifest = "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2026\"\n";
    let a = pkg(
        &[
            (
                "main.neuron",
                "module app.main\nuse self.motors.{go}\nfn f() -> u8:\n  return go()\n",
            ),
            (
                "motors.neuron",
                "module app.motors\npub fn go() -> u8:\n  return 1\n",
            ),
        ],
        Some(manifest),
    );
    assert!(a.is_ok(), "{}", a.render());
}

#[test]
fn invalid_manifest_is_reported() {
    let a = pkg(
        &[("a.neuron", "module a\n")],
        Some("[package]\nedition = \"2026\"\n"),
    );
    assert_eq!(errors(&a), ["NE0108"]);
}

#[test]
fn imports_that_clash_with_local_names_are_duplicates() {
    let a = pkg(
        &[
            (
                "a.neuron",
                "module a\nuse b.{open}\nfn open() -> u8:\n  return 1\n",
            ),
            ("b.neuron", "module b\npub fn open() -> u8:\n  return 2\n"),
        ],
        None,
    );
    assert_eq!(errors(&a), ["NE0102"], "{}", a.render());
}

#[test]
fn syntax_errors_are_reported_and_stop_later_phases() {
    let a = run("module t\nfn f(:\n");
    assert!(!a.is_ok());
    assert!(
        errors(&a).iter().all(|c| c.starts_with("NE00")),
        "{}",
        a.render()
    );
}

#[test]
fn types_in_signatures_are_resolved() {
    only(
        "module t\nfn f(x: Missing) -> u8:\n  return 1\n",
        &["NE0101"],
    );
    only(
        "module t\nfn f() -> u8:\n  return 1\nfn g(x: f) -> u8:\n  return 1\n",
        &["NE0101"],
    );
}

#[test]
fn unknown_units_are_reported() {
    only(
        "module t\nfn f() -> u8:\n  let x = 5 flurb\n  return 1\n",
        &["NE0206"],
    );
}

#[test]
fn fsm_transition_targets_must_exist() {
    let src = "module t for mcu\nfsm F every 10 ms:\n  initial state A:\n    -> Missing\n";
    let a = run(src);
    assert!(errors(&a).contains(&"NE0101"), "{}", a.render());
}

#[test]
fn examples_package_has_no_errors() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    names.sort();
    for p in names {
        files.push(neuron_sema::SourceFile {
            path: p.file_name().unwrap().to_string_lossy().into_owned(),
            text: std::fs::read_to_string(&p).unwrap(),
        });
    }
    let manifest = std::fs::read_to_string(dir.join("neuron.toml")).unwrap();
    let a = neuron_sema::analyze(files, Some(&manifest), &neuron_sema::SemaOptions::default());
    assert!(a.is_ok(), "{}", a.render());
}
