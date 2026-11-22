//! Task D1.2: every device of the corpus cells and of the bottling example has a
//! manifest, and the same module builds for two PLC vendor classes.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::{Kind, Library};
use neuron_emit::EmitOptions;
use neuron_sema::{analyze, SemaOptions, SourceFile};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Analyzes the `.neuron` files directly inside `dir` as one package.
fn package(dir: &Path, only: Option<&str>) -> neuron_nir::Nir {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .filter(|p| only.is_none_or(|o| p.file_name().is_some_and(|n| n == o)))
        .collect();
    files.sort();
    let sources = files
        .iter()
        .map(|f| SourceFile {
            path: f.file_name().unwrap().to_string_lossy().into_owned(),
            text: std::fs::read_to_string(f).unwrap(),
        })
        .collect();
    let manifest = std::fs::read_to_string(dir.join("neuron.toml")).ok();
    let a = analyze(sources, manifest.as_deref(), &SemaOptions::default());
    assert!(a.errors().next().is_none(), "{}", a.render());
    a.nir.clone().unwrap()
}

#[test]
fn every_cell_device_has_a_manifest() {
    let lib = Library::builtin().unwrap();
    let corpus = root().join("examples/corpus/cell");
    let mut cells: Vec<(String, neuron_nir::Nir)> = std::fs::read_dir(&corpus)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "neuron"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let nir = package(&corpus, Some(&name));
            (name, nir)
        })
        .collect();
    cells.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(cells.len(), 4, "the corpus has four cells");
    // The bottling cell places modules of other files, so the whole package is loaded.
    cells.push((
        "bottling_line".into(),
        package(&root().join("examples"), None),
    ));
    let mut kinds = std::collections::BTreeSet::new();
    for (name, nir) in &cells {
        let found = lib
            .resolve_cell(nir)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(!found.devices.is_empty(), "{name} names no device");
        for d in &found.devices {
            kinds.insert(lib.device(&d.manifest).unwrap().kind);
        }
    }
    // The cells use every class except `host` (Raspberry Pi nodes come in session I4).
    let want = [Kind::Plc, Kind::Mcu, Kind::Fpga, Kind::Robot];
    assert!(want.iter().all(|k| kinds.contains(k)), "{kinds:?}");
}

#[test]
fn the_same_module_builds_for_two_plc_vendor_classes() {
    let lib = Library::builtin().unwrap();
    let plcs: Vec<_> = lib
        .device_names()
        .into_iter()
        .filter_map(|n| lib.device(&n).cloned())
        .filter(|m| m.kind == Kind::Plc)
        .collect();
    let classes: std::collections::BTreeSet<_> =
        plcs.iter().map(|m| m.vendor_class.clone()).collect();
    assert!(
        classes.len() >= 2,
        "need two PLC vendor classes: {classes:?}"
    );

    let nir = package(&root().join("examples"), Some("conveyor_plc.neuron"));
    let mut outputs = Vec::new();
    for m in &plcs {
        let emitter = neuron_emit::emitter(m.backend.name()).unwrap();
        let opts = EmitOptions {
            profile: m.profile.clone(),
            ..EmitOptions::default()
        };
        let pkg = emitter
            .emit(&nir, &opts)
            .unwrap_or_else(|d| panic!("{}: {d:?}", m.name));
        assert!(!pkg.files.is_empty(), "{} produced no files", m.name);
        outputs.push(pkg.files);
    }
    assert_ne!(
        outputs[0], outputs[1],
        "two vendor classes give two programs"
    );
}
