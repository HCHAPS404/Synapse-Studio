#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use neuron_sema::{analyze, Analysis, SemaOptions, SourceFile};

pub fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `(file name, text)` of every example, sorted by name.
pub fn examples() -> Vec<(String, String)> {
    let dir = root().join("examples");
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .collect();
    v.sort();
    v.into_iter()
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap().replace("\r\n", "\n"),
            )
        })
        .collect()
}

/// Text of `examples/neuron.toml`.
pub fn examples_manifest() -> String {
    std::fs::read_to_string(root().join("examples/neuron.toml")).unwrap()
}

/// Code blocks of the specification that start a module or a cell.
pub fn spec_programs() -> Vec<String> {
    let dir = root().join("spec/language");
    let mut out = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let mut cur: Option<String> = None;
        for line in text.lines() {
            if line.trim() == "```neuron" {
                cur = Some(String::new());
            } else if line.trim() == "```" {
                if let Some(b) = cur.take() {
                    if b.starts_with("module") || b.starts_with("cell") {
                        out.push(b);
                    }
                }
            } else if let Some(b) = cur.as_mut() {
                b.push_str(line);
                b.push('\n');
            }
        }
    }
    out
}

/// Changes a program a little: deletes, duplicates or truncates a line, or renames a word.
pub fn mutate(text: &str, kind: u8, at: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let i = at % lines.len();
    match kind % 4 {
        0 => lines
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, l)| format!("{l}\n"))
            .collect(),
        1 => lines
            .iter()
            .enumerate()
            .flat_map(|(j, l)| {
                if j == i {
                    vec![format!("{l}\n"), format!("{l}\n")]
                } else {
                    vec![format!("{l}\n")]
                }
            })
            .collect(),
        2 => {
            let mut cut = (at * 7) % (text.len() + 1);
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text[..cut].to_string()
        }
        _ => text.replace(lines[i].split_whitespace().next().unwrap_or("x"), "zz"),
    }
}

pub fn run_with(src: &str, opts: &SemaOptions) -> Analysis {
    analyze(
        vec![SourceFile {
            path: "t.neuron".into(),
            text: src.into(),
        }],
        None,
        opts,
    )
}

pub fn run(src: &str) -> Analysis {
    run_with(src, &SemaOptions::default())
}

pub fn pkg(files: &[(&str, &str)], manifest: Option<&str>) -> Analysis {
    analyze(
        files
            .iter()
            .map(|(p, t)| SourceFile {
                path: (*p).into(),
                text: (*t).into(),
            })
            .collect(),
        manifest,
        &SemaOptions::default(),
    )
}

/// Error codes in order.
pub fn errors(a: &Analysis) -> Vec<&'static str> {
    a.errors().map(|d| d.diag.code).collect()
}

/// All codes (errors and warnings).
pub fn all_codes(a: &Analysis) -> Vec<&'static str> {
    a.codes()
}

/// Asserts that the program has no errors.
pub fn ok(src: &str) {
    let a = run(src);
    assert!(a.is_ok(), "expected no errors, got:\n{}", a.render());
}

/// Asserts that the errors are exactly `codes`.
pub fn only(src: &str, codes: &[&str]) {
    let a = run(src);
    assert_eq!(errors(&a), codes, "unexpected diagnostics:\n{}", a.render());
}

/// Asserts that `code` is among the errors.
pub fn has(src: &str, code: &str) {
    let a = run(src);
    assert!(
        errors(&a).contains(&code),
        "expected {code}, got:\n{}",
        a.render()
    );
}

pub fn has_with(src: &str, opts: &SemaOptions, code: &str) {
    let a = run_with(src, opts);
    assert!(
        errors(&a).contains(&code),
        "expected {code}, got:\n{}",
        a.render()
    );
}
