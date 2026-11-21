//! E1: the JSON API of the compiler bindings, tested natively (no wasm-bindgen).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_syntax::LineIndex;
use neuron_wasm::api;
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const M06: &str = include_str!("../../../examples/corpus/mcu/m06_traffic_light.neuron");
const M02: &str = include_str!("../../../examples/corpus/mcu/m02_button_toggle.neuron");
const P06: &str = include_str!("../../../examples/corpus/plc/p06_pump_with_levels.neuron");
const R02: &str = include_str!("../../../examples/corpus/robot/r02_palletize.neuron");
const H01: &str = include_str!("../../../examples/corpus/host/h01_gateway_counter.neuron");
const C01: &str = include_str!("../../../examples/corpus/core/c01_units.neuron");

fn files(name: &str, text: &str) -> String {
    json!({ name: text }).to_string()
}

fn parse(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("reply is not JSON ({e}): {s}"))
}

fn nir_of(name: &str, text: &str) -> neuron_nir::Nir {
    analyze(
        vec![SourceFile {
            path: name.into(),
            text: text.into(),
        }],
        None,
        &SemaOptions::default(),
    )
    .nir
    .expect("the program has no errors")
}

// ---- check ------------------------------------------------------------------------------

#[test]
fn check_accepts_a_valid_program() {
    let v = parse(&api::check(&files("m06.neuron", M06)));
    assert_eq!(v["ok"], true);
    assert_eq!(v["has_errors"], false);
    assert_eq!(v["files"], json!(["m06.neuron"]));
    let errors = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["severity"] == "error")
        .count();
    assert_eq!(errors, 0);
}

#[test]
fn check_reports_positions_of_an_invalid_program() {
    let src = "module t\nfn f() -> u8:\n  return nope\n";
    let v = parse(&api::check(&files("bad.neuron", src)));
    assert_eq!(v["ok"], true);
    assert_eq!(v["has_errors"], true);
    let d = &v["diagnostics"][0];
    assert_eq!(d["severity"], "error");
    assert_eq!(d["file"], "bad.neuron");
    // The same analyzer as `neuron check`: same first diagnostic, same span.
    let direct = analyze(
        vec![SourceFile {
            path: "bad.neuron".into(),
            text: src.into(),
        }],
        None,
        &SemaOptions::default(),
    );
    let first = &direct.diagnostics[0].diag;
    assert_eq!(d["code"], first.code);
    assert_eq!(d["message"], first.message.as_str());
    assert_eq!(d["span"], json!([first.span.start, first.span.end]));
    // And the position is where `nope` is: line 3, column 10.
    let at = src.find("nope").unwrap();
    assert_eq!(d["span"][0], at);
    assert_eq!(d["start"], json!({"line": 3, "col": 10}));
    let (line, col) = LineIndex::new(src).line_col(src, first.span.end);
    assert_eq!(d["end"], json!({"line": line, "col": col}));
    assert!(d["rendered"].as_str().unwrap().contains("bad.neuron:3:10"));
}

#[test]
fn check_takes_arrays_and_a_manifest() {
    let arr = json!([{"path": "m06.neuron", "text": M06}]).to_string();
    let v = parse(&api::check(&arr));
    assert_eq!(v["has_errors"], false);
    let bad_manifest = json!({"m06.neuron": M06, "neuron.toml": "this is [not toml"}).to_string();
    let v = parse(&api::check(&bad_manifest));
    assert_eq!(v["has_errors"], true);
    let codes: Vec<&Value> = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| &d["code"])
        .collect();
    assert!(codes.contains(&&json!("NE0108")), "{codes:?}");
}

// ---- format -----------------------------------------------------------------------------

#[test]
fn format_is_idempotent_and_matches_the_formatter() {
    let messy = "module m for mcu\nio led:   gpio.output   @ D13\ntask t every  500 ms:\n    led.toggle( )\n";
    let once = parse(&api::format(messy));
    assert_eq!(once["ok"], true);
    assert_eq!(once["changed"], true);
    let text = once["text"].as_str().unwrap();
    assert_eq!(
        text,
        neuron_syntax::format::format_source(messy).unwrap().text
    );
    let twice = parse(&api::format(text));
    assert_eq!(twice["text"], text);
    assert_eq!(twice["changed"], false);
}

#[test]
fn format_reports_syntax_errors_without_changing_anything() {
    let v = parse(&api::format("module m for mcu\nfn (:\n"));
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["kind"], "syntax");
    assert!(!v["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(v["diagnostics"][0]["file"], "<input>");
}

// ---- hover and definition ---------------------------------------------------------------

#[test]
fn hover_and_definition_find_the_declaration() {
    let src = "module t\n## Adds one.\nfn inc(x: u8) -> u8:\n  return x + 1\nfn g() -> u8:\n  return inc(2)\n";
    let f = files("t.neuron", src);
    let at = src.rfind("inc").unwrap() + 1;
    let h = parse(&api::hover(&f, "t.neuron", at));
    assert_eq!(h["ok"], true);
    let md = h["hover"]["markdown"].as_str().unwrap();
    assert!(md.contains("fn inc(x: u8) -> u8:"), "{md}");
    assert!(md.contains("Adds one."), "{md}");
    assert_eq!(h["hover"]["range"]["start"]["line"], 6);
    let d = parse(&api::definition(&f, "t.neuron", at));
    assert_eq!(d["definition"]["file"], "t.neuron");
    assert_eq!(d["definition"]["line"], 3);
    assert_eq!(d["definition"]["header"], "fn inc(x: u8) -> u8:");
    // Nothing under the cursor, or a file that is not there.
    assert!(parse(&api::definition(&f, "t.neuron", 100_000))["definition"].is_null());
    assert_eq!(
        parse(&api::hover(&f, "other.neuron", 0))["error"]["kind"],
        "bad_request"
    );
}

// ---- run --------------------------------------------------------------------------------

#[test]
fn run_gives_the_trace_of_the_interpreter() {
    let end_ns = 6_000_000_000;
    let v = parse(&api::run(&files("m06.neuron", M06), end_ns, None));
    assert_eq!(v["ok"], true);
    let mut it = neuron_interp::Interp::new(
        nir_of("m06.neuron", M06),
        neuron_interp::Options {
            end_ns,
            ..neuron_interp::Options::default()
        },
    )
    .unwrap();
    it.run();
    assert_eq!(v["trace"], neuron_interp::render(it.trace()).as_str());
    assert_eq!(v["events"].as_array().unwrap().len(), it.trace().len());
    let first = &it.trace()[0];
    assert_eq!(v["events"][0]["t_ns"], first.t_ns);
    assert_eq!(v["events"][0]["kind"], first.kind.tag());
    assert_eq!(v["events"][0]["name"], first.name.as_str());
    assert_eq!(v["events"][0]["value"], first.value.as_str());
    assert_eq!(v["now_ns"], it.now_ns());
    assert!(v["fault"].is_null());
}

#[test]
fn run_applies_a_scenario() {
    let scenario = "100 ms: button = true\n200 ms: button = false\n";
    let end_ns = 500_000_000;
    let v = parse(&api::run(&files("m02.neuron", M02), end_ns, Some(scenario)));
    assert_eq!(v["ok"], true, "{v}");
    let mut it = neuron_interp::Interp::new(
        nir_of("m02.neuron", M02),
        neuron_interp::Options {
            end_ns,
            ..neuron_interp::Options::default()
        },
    )
    .unwrap();
    it.load_scenario(&neuron_interp::parse_scenario(scenario).unwrap());
    it.run();
    assert_eq!(v["trace"], neuron_interp::render(it.trace()).as_str());
    assert!(v["trace"].as_str().unwrap().contains("led"));
    let without = parse(&api::run(&files("m02.neuron", M02), end_ns, None));
    assert_ne!(without["trace"], v["trace"]);
}

#[test]
fn run_rejects_bad_input_with_structured_errors() {
    let f = files("m06.neuron", M06);
    let v = parse(&api::run(&f, 1_000, Some("nonsense")));
    assert_eq!(v["error"]["kind"], "bad_request");
    assert!(v["error"]["message"].as_str().unwrap().contains("line 1"));
    assert_eq!(
        parse(&api::run(&f, -1, None))["error"]["kind"],
        "bad_request"
    );
    let broken = files("bad.neuron", "module t\nfn f() -> u8:\n  return nope\n");
    let v = parse(&api::run(&broken, 1_000, None));
    assert_eq!(v["error"]["kind"], "analysis");
    assert_eq!(v["diagnostics"][0]["severity"], "error");
}

// ---- test -------------------------------------------------------------------------------

#[test]
fn test_report_equals_run_tests() {
    for (name, src) in [
        ("c01.neuron", C01),
        ("m06.neuron", M06),
        ("m02.neuron", M02),
    ] {
        let v = parse(&api::test(&files(name, src)));
        assert_eq!(v["ok"], true);
        let reports =
            neuron_interp::run_tests(&nir_of(name, src), &neuron_interp::Options::default());
        assert!(!reports.is_empty(), "{name} has tests");
        let tests = v["tests"].as_array().unwrap();
        assert_eq!(tests.len(), reports.len());
        for (t, r) in tests.iter().zip(&reports) {
            assert_eq!(t["name"], r.name.as_str());
            assert_eq!(t["passed"], r.passed);
            assert_eq!(t["message"], r.message.as_str());
            assert_eq!(t["module"], r.module);
            assert_eq!(t["file"], name);
        }
        assert_eq!(v["total"], reports.len());
        assert_eq!(v["passed"], reports.iter().filter(|r| r.passed).count());
    }
}

#[test]
fn a_failed_assertion_is_located() {
    let src = "module t for core\ntest \"fails\":\n  assert 1 == 2, \"one is not two\"\n";
    let v = parse(&api::test(&files("t.neuron", src)));
    assert_eq!(v["failed"], 1);
    let t = &v["tests"][0];
    assert_eq!(t["passed"], false);
    assert!(t["message"].as_str().unwrap().contains("one is not two"));
    assert_eq!(t["at"]["file"], "t.neuron");
    assert_eq!(t["at"]["start"]["line"], 3);
}

// ---- emit -------------------------------------------------------------------------------

fn emit_source(lang: &str) -> (&'static str, &'static str) {
    match lang {
        "st" | "scl" | "plcopen" => ("p06.neuron", P06),
        "spel" => ("r02.neuron", R02),
        "python" => ("h01.neuron", H01),
        "kernel" => ("m06.neuron", M06),
        "c" => ("c01.neuron", C01),
        other => panic!("no corpus program for the emitter `{other}`"),
    }
}

#[test]
fn emit_returns_what_neuron_emit_returns_for_every_emitter() {
    for lang in neuron_emit::names() {
        let (name, src) = emit_source(lang);
        let v = parse(&api::emit(lang, &files(name, src), None));
        let direct = neuron_emit::emitter(lang)
            .unwrap()
            .emit(&nir_of(name, src), &neuron_emit::EmitOptions::default());
        match direct {
            Ok(pkg) => {
                assert_eq!(v["ok"], true, "{lang}: {v}");
                assert_eq!(v["lang"], *lang);
                let got: BTreeMap<String, String> =
                    serde_json::from_value(v["files"].clone()).unwrap();
                assert_eq!(got, pkg.all_files(), "{lang}");
                assert!(!got.is_empty());
                assert_eq!(v["fidelity"], serde_json::to_value(&pkg.fidelity).unwrap());
                assert!(v["fidelity"]["summary"].is_object(), "{lang}");
                assert!(v["diagnostics"].is_array());
            }
            Err(diags) => {
                assert_eq!(v["ok"], false, "{lang}");
                assert_eq!(v["error"]["kind"], "emit");
                assert_eq!(v["diagnostics"].as_array().unwrap().len(), diags.len());
            }
        }
    }
}

#[test]
fn emit_takes_a_profile() {
    let f = files("p06.neuron", P06);
    let v = parse(&api::emit("st", &f, Some("yaskawa")));
    let direct = neuron_emit::emitter("st")
        .unwrap()
        .emit(
            &nir_of("p06.neuron", P06),
            &neuron_emit::EmitOptions {
                profile: "yaskawa".into(),
                robot: String::new(),
            },
        )
        .unwrap();
    let got: BTreeMap<String, String> = serde_json::from_value(v["files"].clone()).unwrap();
    assert_eq!(got, direct.all_files());
    let generic = parse(&api::emit("st", &f, None));
    assert_ne!(generic["files"], v["files"]);
}

#[test]
fn emit_errors_are_structured() {
    let f = files("p06.neuron", P06);
    let v = parse(&api::emit("cobol", &f, None));
    assert_eq!(v["error"]["kind"], "unknown_language");
    assert!(v["error"]["message"].as_str().unwrap().contains("plcopen"));
    let bad = files("bad.neuron", "module t\nfn f() -> u8:\n  return nope\n");
    assert_eq!(
        parse(&api::emit("st", &bad, None))["error"]["kind"],
        "analysis"
    );
}

// ---- import -----------------------------------------------------------------------------

#[test]
fn import_round_trips_the_emitted_structured_text() {
    let emitted = parse(&api::emit("st", &files("p06.neuron", P06), None));
    let files_map: BTreeMap<String, String> =
        serde_json::from_value(emitted["files"].clone()).unwrap();
    let st = &files_map["p06_pump_with_levels.st"];
    let v = parse(&api::import("st", "p06_pump_with_levels", st));
    assert_eq!(v["ok"], true, "{v}");
    let direct = neuron_import::import("st", "p06_pump_with_levels", st).unwrap();
    assert_eq!(v["source"], direct.source.as_str());
    assert_eq!(v["complete"], direct.complete());
    assert_eq!(
        v["fidelity"],
        serde_json::to_value(&direct.fidelity).unwrap()
    );
    // The imported source is analyzed normally.
    let checked = parse(&api::check(&files("p06_imported.neuron", &direct.source)));
    assert_eq!(checked["has_errors"], false, "{checked}");
}

#[test]
fn import_errors_are_structured() {
    let v = parse(&api::import("cobol", "x", "x"));
    assert_eq!(v["error"]["kind"], "unknown_language");
    let v = parse(&api::import("st", "x", "PROGRAM ( ; END"));
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["kind"], "import");
    assert_eq!(v["diagnostics"][0]["code"], "NE1001");
}

// ---- version ----------------------------------------------------------------------------

#[test]
fn version_has_the_compiler_schema_and_protocol() {
    let v = parse(&api::version());
    assert_eq!(v["ok"], true);
    assert_eq!(v["compiler"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["nir_schema"], neuron_nir::NIR_VERSION);
    assert_eq!(v["protocol"], synapse_protocol::PROTOCOL_VERSION);
    assert_eq!(v["emitters"], json!(neuron_emit::names()));
    assert_eq!(v["importers"], json!(neuron_import::languages()));
}

// ---- malformed input --------------------------------------------------------------------

#[test]
fn malformed_json_is_a_structured_error() {
    for bad in [
        "",
        "{",
        "null",
        "42",
        "[1]",
        "{\"a.neuron\": 3}",
        "{}",
        "[]",
    ] {
        for out in [
            api::check(bad),
            api::hover(bad, "a.neuron", 0),
            api::definition(bad, "a.neuron", 0),
            api::run(bad, 1, None),
            api::test(bad),
            api::emit("st", bad, None),
        ] {
            let v = parse(&out);
            assert_eq!(v["ok"], false, "{bad}: {out}");
            assert_eq!(v["error"]["kind"], "bad_request", "{bad}: {out}");
        }
    }
}

/// Every reply is JSON with an `ok` flag, and no input reaches `internal` (a caught panic).
fn assert_well_formed(out: &str) {
    let v = parse(out);
    assert!(v["ok"].is_boolean(), "{out}");
    if v["ok"] == false {
        assert_ne!(v["error"]["kind"], "internal", "{out}");
        assert!(v["error"]["message"].is_string(), "{out}");
        assert!(v["diagnostics"].is_array(), "{out}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn random_strings_as_files_json_never_panic(s in "\\PC{0,120}") {
        assert_well_formed(&api::check(&s));
        assert_well_formed(&api::hover(&s, "a.neuron", 3));
        assert_well_formed(&api::definition(&s, "a.neuron", 3));
        assert_well_formed(&api::run(&s, 1_000_000, Some(&s)));
        assert_well_formed(&api::test(&s));
        for lang in neuron_emit::names() {
            assert_well_formed(&api::emit(lang, &s, Some(&s)));
        }
    }

    #[test]
    fn random_strings_as_text_never_panic(s in "\\PC{0,200}") {
        assert_well_formed(&api::format(&s));
        for lang in neuron_import::languages() {
            assert_well_formed(&api::import(lang, &s, &s));
        }
        assert_well_formed(&api::import(&s, "x", &s));
        assert_well_formed(&api::emit(&s, &files("a.neuron", &s), None));
    }

    #[test]
    fn random_programs_never_panic(s in "\\PC{0,200}", off in 0usize..300) {
        let f = files("a.neuron", &s);
        assert_well_formed(&api::check(&f));
        assert_well_formed(&api::hover(&f, "a.neuron", off));
        assert_well_formed(&api::definition(&f, "a.neuron", off));
        assert_well_formed(&api::run(&f, 10_000_000, None));
        assert_well_formed(&api::test(&f));
        assert_well_formed(&api::emit("st", &f, None));
    }

    #[test]
    fn mutated_corpus_programs_never_panic(cut in 0usize..400, junk in "[ -~]{0,6}") {
        let mut s = M06.to_string();
        let at = cut.min(s.len());
        let at = (0..=at).rev().find(|i| s.is_char_boundary(*i)).unwrap_or(0);
        s.insert_str(at, &junk);
        let f = files("m06.neuron", &s);
        assert_well_formed(&api::check(&f));
        assert_well_formed(&api::format(&s));
        assert_well_formed(&api::run(&f, 50_000_000, None));
        assert_well_formed(&api::emit("kernel", &f, None));
    }
}
