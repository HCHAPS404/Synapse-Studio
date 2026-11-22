//! Task E2.2: `--format json` of `neuron check`, `build` and `emit`, run as the Core API
//! runs them. The human output stays the default and is checked to be unchanged.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn neuron(args: &[&str], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_neuron"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn scratch(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("neuron-e2-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (n, t) in files {
        std::fs::write(dir.join(n), t).unwrap();
    }
    dir
}

/// Runs the command and parses standard output as the one JSON document.
fn json(args: &[&str], dir: &Path) -> (Value, Option<i32>, String) {
    let o = neuron(args, dir);
    let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
    let doc: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}):\n{stdout}"));
    (
        doc,
        o.status.code(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

const BLINK: &str = include_str!("../../../examples/corpus/mcu/m01_blink.neuron");
const BAD: &str = "module bad for host\nlet x: int = true\n";
const P01: &str = "examples/corpus/plc/p01_start_stop.neuron";

fn diag_codes(doc: &Value) -> Vec<String> {
    doc["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["code"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn check_ok_has_no_diagnostics_and_the_exit_code_is_zero() {
    let (doc, code, _) = json(&["check", "--format", "json", P01], &root());
    assert_eq!(code, Some(0));
    assert_eq!(doc["format"], 1);
    assert_eq!(doc["command"], "check");
    assert_eq!(doc["ok"], true);
    assert!(doc["diagnostics"].as_array().unwrap().is_empty());
    assert!(doc["neuron_version"].is_string());
}

#[test]
fn check_errors_carry_code_position_and_help() {
    let dir = scratch("bad", &[("bad.neuron", BAD)]);
    let (doc, code, _) = json(&["check", "--format=json", "bad.neuron"], &dir);
    assert_eq!(code, Some(1));
    assert_eq!(doc["ok"], false);
    let d = &doc["diagnostics"][0];
    assert_eq!(d["severity"], "error");
    assert!(d["code"].as_str().unwrap().starts_with("NE"));
    assert_eq!(d["file"], "bad.neuron");
    assert!(d["line"].as_u64().unwrap() >= 1);
    assert!(d["column"].as_u64().unwrap() >= 1);
    assert!(!d["message"].as_str().unwrap().is_empty());
}

#[test]
fn json_and_human_agree_on_the_diagnostics() {
    let dir = scratch("agree", &[("bad.neuron", BAD)]);
    let human = neuron(&["check", "bad.neuron"], &dir);
    let human_err = String::from_utf8_lossy(&human.stderr).into_owned();
    let (doc, _, _) = json(&["check", "--format", "json", "bad.neuron"], &dir);
    for code in diag_codes(&doc) {
        assert!(
            human_err.contains(&format!("error[{code}]")),
            "{code}\n{human_err}"
        );
    }
    let n = human_err.matches("error[").count();
    let from_json = doc["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"].is_string() && d["severity"] == "error")
        .count();
    assert_eq!(n, from_json);
}

#[test]
fn human_output_is_the_default_and_has_no_json() {
    let o = neuron(&["check", P01], &root());
    assert!(o.status.success());
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains(": ok"), "{out}");
    assert!(serde_json::from_str::<Value>(&out).is_err());
    let o = neuron(&["check", "--format", "human", P01], &root());
    assert!(String::from_utf8_lossy(&o.stdout).contains(": ok"));
}

#[test]
fn an_unknown_format_is_a_usage_error() {
    let o = neuron(&["check", "--format", "xml", P01], &root());
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--format"));
    let o = neuron(&["build", "--format"], &root());
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn usage_errors_are_reported_as_json_too() {
    let (doc, code, _) = json(&["check", "--format", "json"], &root());
    assert_eq!(code, Some(2));
    assert_eq!(doc["ok"], false);
    assert!(doc["diagnostics"][0]["message"]
        .as_str()
        .unwrap()
        .contains("needs"));
    assert!(doc["diagnostics"][0]["code"].is_null());
    // The usage text is not repeated as messages.
    assert!(doc["messages"].as_array().unwrap().is_empty());
}

#[test]
fn a_native_build_lists_its_artifacts() {
    let dir = scratch("native", &[("blink.neuron", BLINK)]);
    let (doc, code, _) = json(
        &[
            "build",
            "blink.neuron",
            "--native",
            "--out",
            "n",
            "--format",
            "json",
        ],
        &dir,
    );
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["output_dir"], "n");
    let arts: Vec<(String, u64)> = doc["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["path"].as_str().unwrap().to_string(),
                a["size"].as_u64().unwrap(),
            )
        })
        .collect();
    let names: Vec<&str> = arts.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(names, ["app.hex", "app.lst"]);
    let hex_len = std::fs::metadata(dir.join("n/app.hex")).unwrap().len();
    assert_eq!(arts[0].1, hex_len);
    assert!(doc["messages"].to_string().contains("native backend"));
}

#[test]
fn a_failed_build_reports_the_error_and_no_artifacts() {
    let dir = scratch("buildbad", &[("bad.neuron", BAD)]);
    let (doc, code, _) = json(&["build", "bad.neuron", "--format", "json"], &dir);
    assert_eq!(code, Some(1));
    assert_eq!(doc["ok"], false);
    assert!(!diag_codes(&doc).is_empty());
    assert!(doc["artifacts"].as_array().unwrap().is_empty());
    assert!(!dir.join("build").exists());
}

#[test]
fn a_hardware_error_of_build_is_a_diagnostic() {
    let src = BLINK.replace("@ D13", "@ D99");
    let dir = scratch("hw", &[("blink.neuron", &src)]);
    let (doc, code, _) = json(
        &["build", "blink.neuron", "--native", "--format", "json"],
        &dir,
    );
    assert_eq!(code, Some(1));
    let d = doc["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"].as_str().is_some_and(|c| c.starts_with("NE06")))
        .unwrap_or_else(|| panic!("no NE06xx diagnostic in {doc}"));
    assert_eq!(d["file"], "blink.neuron");
    assert!(d["line"].as_u64().is_some());
}

#[test]
fn emit_lists_the_written_files() {
    let dir = scratch("emit", &[]);
    let out = dir.join("st");
    let (doc, code, _) = json(
        &[
            "emit",
            "st",
            P01,
            "-o",
            out.to_str().unwrap(),
            "--format",
            "json",
        ],
        &root(),
    );
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["command"], "emit");
    assert_eq!(doc["ok"], true);
    let paths: Vec<&str> = doc["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"fidelity.json"), "{paths:?}");
    assert!(paths.iter().any(|p| p.ends_with(".st")), "{paths:?}");
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted);
    for a in doc["artifacts"].as_array().unwrap() {
        let size = std::fs::metadata(out.join(a["path"].as_str().unwrap()))
            .unwrap()
            .len();
        assert_eq!(a["size"].as_u64().unwrap(), size);
    }
}

#[test]
fn emit_with_a_program_error_reports_it() {
    let dir = scratch("emitbad", &[("bad.neuron", BAD)]);
    let (doc, code, _) = json(&["emit", "st", "bad.neuron", "--format", "json"], &dir);
    assert_eq!(code, Some(1));
    assert_eq!(doc["ok"], false);
    assert!(!diag_codes(&doc).is_empty());
}

#[test]
fn emit_policy_refusal_is_a_diagnostic_and_writes_nothing() {
    let dir = scratch("policy", &[]);
    let out = dir.join("o");
    let (doc, code, _) = json(
        &[
            "emit",
            "st",
            "examples/corpus/plc/p03_on_delay.neuron",
            "--policy",
            "exact",
            "-o",
            out.to_str().unwrap(),
            "--format",
            "json",
        ],
        &root(),
    );
    assert_eq!(code, Some(1), "{doc}");
    assert!(diag_codes(&doc).contains(&"NE1003".to_string()), "{doc}");
    assert!(doc["artifacts"].as_array().unwrap().is_empty());
}

#[test]
fn json_output_is_deterministic() {
    let args = ["check", "--format", "json", P01];
    let a = neuron(&args, &root());
    let b = neuron(&args, &root());
    assert_eq!(a.stdout, b.stdout);
}
