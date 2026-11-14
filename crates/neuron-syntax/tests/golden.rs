//! Golden outlines: the structure of every example program must not change
//! unintentionally. Regenerate with `UPDATE_GOLDEN=1` (tools/golden-update.ps1).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_syntax::{outline::outline, parse};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn example_outlines_match_golden_files() {
    let root = repo_root();
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let dir = root.join("conformance/golden/parser");
    let mut checked = 0;
    for entry in std::fs::read_dir(root.join("examples")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "neuron") {
            continue;
        }
        let src = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\r\n", "\n");
        let parsed = parse(&src);
        assert!(parsed.diagnostics.is_empty(), "{}", path.display());
        let got = outline(&parsed.file, &src);
        let golden = dir.join(format!(
            "{}.outline",
            path.file_stem().unwrap().to_string_lossy()
        ));
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&golden, &got).unwrap();
        } else {
            let want = std::fs::read_to_string(&golden)
                .unwrap_or_else(|_| panic!("missing golden file {}", golden.display()))
                .replace("\r\n", "\n");
            assert_eq!(got, want, "outline changed for {}", path.display());
        }
        checked += 1;
    }
    assert!(checked >= 7, "only {checked} examples found");
}
