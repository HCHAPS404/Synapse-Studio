//! Task A9: lowering to NIR, canonical JSON and the printer (spec 16).
//!
//! Properties: NIR-1 (same input, same bytes) and P-RT (printing NIR back to source
//! and analyzing it again gives the same NIR, apart from source positions).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use neuron_sema::nir::{self, Nir};
use neuron_sema::{analyze, Analysis, SemaOptions, SourceFile};
use proptest::prelude::*;

fn analyze_files(files: &[(String, String)], manifest: Option<&str>) -> Analysis {
    analyze(
        files
            .iter()
            .map(|(p, t)| SourceFile {
                path: p.clone(),
                text: t.clone(),
            })
            .collect(),
        manifest,
        &SemaOptions::default(),
    )
}

fn lower(files: &[(String, String)], manifest: Option<&str>) -> Nir {
    let a = analyze_files(files, manifest);
    match a.nir {
        Some(n) => n,
        None => panic!("expected NIR, got errors:\n{}", a.render()),
    }
}

/// The NIR as JSON with every `span` removed (positions change when text is reprinted).
fn without_spans(n: &Nir) -> serde_json::Value {
    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(m) => {
                m.remove("span");
                m.values_mut().for_each(strip);
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(n).unwrap();
    strip(&mut v);
    v
}

/// Checks P-RT for one package and returns a description of the first failure.
fn round_trip(files: &[(String, String)], manifest: Option<&str>) -> Result<(), String> {
    let first = lower(files, manifest);
    let printed = nir::print(&first);
    let again = analyze_files(&printed, manifest);
    let Some(second) = again.nir.as_ref() else {
        return Err(format!(
            "printed program has errors:\n{}\n--- printed ---\n{}",
            again.render(),
            printed.iter().map(|(_, t)| t.as_str()).collect::<String>()
        ));
    };
    if without_spans(&first) != without_spans(second) {
        let a = serde_json::to_string_pretty(&without_spans(&first)).unwrap();
        let b = serde_json::to_string_pretty(&without_spans(second)).unwrap();
        let line = a
            .lines()
            .zip(b.lines())
            .position(|(x, y)| x != y)
            .unwrap_or(0);
        return Err(format!(
            "NIR differs after printing (line {line}):\n  {}\n  {}\n--- printed ---\n{}",
            a.lines().nth(line).unwrap_or(""),
            b.lines().nth(line).unwrap_or(""),
            printed.iter().map(|(_, t)| t.as_str()).collect::<String>()
        ));
    }
    if nir::print(second) != printed {
        return Err("the printer is not idempotent".into());
    }
    Ok(())
}

#[test]
fn examples_package_lowers_to_nir() {
    let n = lower(&examples(), Some(&examples_manifest()));
    assert_eq!(n.nir_version, nir::NIR_VERSION);
    assert_eq!(n.package.as_deref(), Some("examples"));
    assert_eq!(n.modules.len(), examples().len());
    for (i, m) in n.modules.iter().enumerate() {
        assert_eq!(m.id, format!("m{i}"));
    }
}

#[test]
fn ip_addresses_are_literals_in_nir() {
    let n = lower(&examples(), Some(&examples_manifest()));
    let json = nir::to_json(&n);
    assert!(json.contains("\"lit\": \"ip\""), "no ip literal in the NIR");
    assert!(json.contains("\"text\": \"192.168.0.10\""));
}

#[test]
fn programs_with_errors_have_no_nir() {
    let a = run("module t\nfn f() -> u8:\n  return nope\n");
    assert!(!a.is_ok());
    assert!(a.nir.is_none());
}

#[test]
fn json_round_trip() {
    let n = lower(&examples(), Some(&examples_manifest()));
    let text = nir::to_json(&n);
    let back = nir::from_json(&text).unwrap();
    if back != n {
        let a = format!("{n:#?}");
        let b = format!("{back:#?}");
        let line = a.lines().zip(b.lines()).position(|(x, y)| x != y);
        let line = line.unwrap_or(0);
        let ctx: Vec<&str> = a.lines().skip(line.saturating_sub(12)).take(14).collect();
        panic!(
            "JSON round trip changed the NIR at line {line}:\n{}\n  read back: {}",
            ctx.join("\n"),
            b.lines().nth(line).unwrap_or("")
        );
    }
    for p in spec_programs() {
        if let Some(n) = run(&p).nir {
            assert_eq!(nir::from_json(&nir::to_json(&n)).unwrap(), n);
        }
    }
}

#[test]
fn unknown_major_versions_are_rejected() {
    let mut n = lower(&examples(), Some(&examples_manifest()));
    n.nir_version = "2.0".into();
    assert!(nir::from_json(&nir::to_json(&n)).is_err());
}

#[test]
fn nir_1_output_is_byte_identical_across_runs() {
    let a = nir::to_json(&lower(&examples(), Some(&examples_manifest())));
    let b = nir::to_json(&lower(&examples(), Some(&examples_manifest())));
    assert_eq!(a, b);
    assert!(a.ends_with('\n') && !a.contains('\r'));
}

#[test]
fn examples_match_the_golden_nir() {
    let path = root().join("conformance/golden/nir/examples.nir.json");
    let got = nir::to_json(&lower(&examples(), Some(&examples_manifest())));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .expect("missing golden file; run tools/rust-check.ps1 -Golden")
        .replace("\r\n", "\n");
    assert!(
        want == got,
        "NIR of the examples changed; review and run tools/rust-check.ps1 -Golden"
    );
}

#[test]
fn p_rt_holds_for_the_examples_package() {
    round_trip(&examples(), Some(&examples_manifest())).unwrap();
}

#[test]
fn p_rt_holds_for_every_example_alone() {
    for (name, text) in examples() {
        let files = vec![(name.clone(), text)];
        if analyze_files(&files, None).nir.is_some() {
            if let Err(e) = round_trip(&files, None) {
                panic!("{name}: {e}");
            }
        }
    }
}

#[test]
fn p_rt_holds_for_every_valid_spec_program() {
    let mut checked = 0;
    for (i, p) in spec_programs().into_iter().enumerate() {
        let files = vec![("t.neuron".to_string(), p)];
        if analyze_files(&files, None).nir.is_some() {
            if let Err(e) = round_trip(&files, None) {
                panic!("spec program {i}: {e}");
            }
            checked += 1;
        }
    }
    // Most spec blocks are fragments that refer to names defined elsewhere.
    assert!(checked >= 8, "only {checked} spec programs were valid");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// P-RT also holds for valid variations of the examples.
    #[test]
    fn p_rt_holds_for_mutated_examples(idx in 0usize..16, kind in 0u8..4, at in 0usize..4000) {
        let ex = examples();
        let (name, text) = &ex[idx % ex.len()];
        let files = vec![(name.clone(), mutate(text, kind, at))];
        if analyze_files(&files, None).nir.is_some() {
            let r = round_trip(&files, None);
            prop_assert!(r.is_ok(), "{}", r.unwrap_err());
        }
    }
}
