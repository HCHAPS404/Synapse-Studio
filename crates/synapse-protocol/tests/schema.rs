//! The committed JSON Schemas (`spec/schemas/relay/`) match the Rust types.
//! Regenerate with `cargo run -p synapse-protocol --example export_schema`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;
use synapse_protocol::{message_schema_json, replay_schema_json, Body};

fn check(file: &str, got: String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/schemas/relay")
        .join(file);
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing {file}; run the export_schema example"))
        .replace("\r\n", "\n");
    assert!(
        want == got,
        "{file} differs from the Rust types; review and run `cargo run -p synapse-protocol --example export_schema`"
    );
}

#[test]
fn committed_message_schema_matches_the_types() {
    check("message.schema.json", message_schema_json());
}

#[test]
fn committed_replay_schema_matches_the_types() {
    check("replay.schema.json", replay_schema_json());
}

#[test]
fn schema_names_every_message_type() {
    let s = message_schema_json();
    for t in Body::TYPES {
        assert!(
            s.contains(&format!("\"const\": \"{t}\"")),
            "schema lacks type `{t}`"
        );
    }
}
