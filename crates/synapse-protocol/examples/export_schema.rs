//! Regenerates the committed JSON Schemas in `spec/schemas/relay/`.
//!
//! Run: `cargo run -p synapse-protocol --example export_schema`
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/schemas/relay");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("message.schema.json"),
        synapse_protocol::message_schema_json(),
    )
    .unwrap();
    std::fs::write(
        dir.join("replay.schema.json"),
        synapse_protocol::replay_schema_json(),
    )
    .unwrap();
    println!("wrote {}", dir.display());
}
