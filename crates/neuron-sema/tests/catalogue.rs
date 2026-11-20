//! Every diagnostic code used by the compiler is in the catalogue
//! (`spec/language/10-diagnostics.md`) and documented in `docs/errors/`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            files(&p, ext, out);
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

/// Codes written as string literals, such as `"NE0201"`.
fn codes_in(text: &str) -> BTreeSet<String> {
    let b = text.as_bytes();
    let mut out = BTreeSet::new();
    for i in 0..b.len().saturating_sub(7) {
        if b[i] == b'"'
            && b[i + 1] == b'N'
            && (b[i + 2] == b'E' || b[i + 2] == b'L')
            && b[i + 3..i + 7].iter().all(u8::is_ascii_digit)
            && b[i + 7] == b'"'
        {
            out.insert(text[i + 1..i + 7].to_string());
        }
    }
    out
}

fn all_mentions(text: &str) -> BTreeSet<String> {
    let b = text.as_bytes();
    let mut out = BTreeSet::new();
    for i in 0..b.len().saturating_sub(5) {
        if b[i] == b'N'
            && (b[i + 1] == b'E' || b[i + 1] == b'L')
            && b[i + 2..i + 6].iter().all(u8::is_ascii_digit)
        {
            out.insert(text[i..i + 6].to_string());
        }
    }
    out
}

#[test]
fn emitted_codes_are_catalogued_and_documented() {
    let r = root();
    let mut sources = Vec::new();
    files(&r.join("crates/neuron-syntax/src"), "rs", &mut sources);
    files(&r.join("crates/neuron-sema/src"), "rs", &mut sources);
    files(&r.join("crates/neuron-target/src"), "rs", &mut sources);
    files(&r.join("crates/neuron-mir/src"), "rs", &mut sources);
    files(&r.join("crates/neuron-codegen-hdl/src"), "rs", &mut sources);
    files(&r.join("crates/neuron-native-avr/src"), "rs", &mut sources);
    files(&r.join("crates/neuron-ui/src"), "rs", &mut sources);
    files(&r.join("crates/neuron-emit/src"), "rs", &mut sources);
    files(&r.join("crates/neuron-st/src"), "rs", &mut sources);
    files(&r.join("crates/neuron-import/src"), "rs", &mut sources);
    let mut emitted = BTreeSet::new();
    for f in &sources {
        emitted.extend(codes_in(&std::fs::read_to_string(f).unwrap()));
    }
    assert!(emitted.len() > 40, "only {} codes found", emitted.len());

    let catalogue = std::fs::read_to_string(r.join("spec/language/10-diagnostics.md")).unwrap();
    let catalogued = all_mentions(&catalogue);

    let mut docs = String::new();
    let mut doc_files = Vec::new();
    files(&r.join("docs/errors"), "md", &mut doc_files);
    for f in doc_files {
        docs.push_str(&std::fs::read_to_string(f).unwrap());
    }
    let documented = all_mentions(&docs);

    let missing_cat: Vec<_> = emitted.difference(&catalogued).collect();
    let missing_doc: Vec<_> = emitted.difference(&documented).collect();
    assert!(
        missing_cat.is_empty(),
        "not in 10-diagnostics.md: {missing_cat:?}"
    );
    assert!(
        missing_doc.is_empty(),
        "not documented in docs/errors: {missing_doc:?}"
    );
}

#[test]
fn catalogued_codes_are_documented() {
    let r = root();
    let catalogue = std::fs::read_to_string(r.join("spec/language/10-diagnostics.md")).unwrap();
    let mut docs = String::new();
    let mut doc_files = Vec::new();
    files(&r.join("docs/errors"), "md", &mut doc_files);
    for f in doc_files {
        docs.push_str(&std::fs::read_to_string(f).unwrap());
    }
    let documented = all_mentions(&docs);
    let missing: Vec<_> = all_mentions(&catalogue)
        .into_iter()
        .filter(|c| !documented.contains(c))
        .collect();
    assert!(
        missing.is_empty(),
        "catalogued but not documented in docs/errors: {missing:?}"
    );
}
