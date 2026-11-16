//! The committed JSON Schema (`spec/schemas/nir.schema.json`) matches the data model.
//! Regenerate it with `tools/rust-check.ps1 -Golden` (sets `UPDATE_GOLDEN=1`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

#[test]
fn committed_schema_matches_the_model() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/schemas/nir.schema.json");
    let got = neuron_nir::schema_json();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .expect("missing spec/schemas/nir.schema.json; run tools/rust-check.ps1 -Golden")
        .replace("\r\n", "\n");
    assert!(
        want == got,
        "the NIR schema changed; review it and run tools/rust-check.ps1 -Golden"
    );
}

#[test]
fn schema_is_valid_json_and_names_the_root() {
    let v: serde_json::Value = serde_json::from_str(&neuron_nir::schema_json()).unwrap();
    assert_eq!(v["title"], "Nir");
    assert!(v["$defs"]["Expr"].is_object());
    assert!(v["$defs"]["Lit"].is_object());
}
