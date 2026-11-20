//! `neuron emit` and `neuron import` (Phase C, task C7): translation to and from other
//! languages. The emitters live in `neuron-emit` and the importers in `neuron-import`;
//! this module only parses the command line, runs the analyzer, applies the policy and
//! writes the results. Usage and details: `docs/translation.md`.

use crate::{load, read, USAGE};
use neuron_emit::{Class, Diag, EmitOptions, Fidelity, Policy};
use neuron_sema::{analyze, SemaOptions};
use neuron_syntax::LineIndex;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// A diagnostic of an emitter or importer with its file, line and source line, in the
/// same layout as the hardware checks.
fn render(d: &Diag, level: &str, paths: &[String], texts: &[String]) -> String {
    let mut out = format!("{level}[{}]: {}\n", d.code, d.message);
    match paths.iter().position(|p| *p == d.file) {
        Some(i) => {
            let text = &texts[i];
            let (line, col) = LineIndex::new(text).line_col(text, d.span[0]);
            out.push_str(&format!(" --> {}:{line}:{col}\n", d.file));
            if let Some(src) = text.lines().nth(line.saturating_sub(1)) {
                out.push_str(&format!("  |\n  | {src}\n"));
            }
        }
        None => out.push_str(&format!(" --> {}\n", d.file)),
    }
    if let Some(h) = &d.help {
        out.push_str(&format!("  = help: {h}\n"));
    }
    out
}

fn summary(f: &Fidelity) -> String {
    let s = &f.summary;
    format!(
        "fidelity: {} exact, {} equivalent, {} approximate, {} modeled, {} unsupported",
        s.exact, s.equivalent, s.approximate, s.modeled, s.unsupported
    )
}

fn value<'a>(it: &mut std::slice::Iter<'a, String>, flag: &str) -> Result<&'a String, ExitCode> {
    it.next().ok_or_else(|| {
        eprintln!("error: `{flag}` needs a value\n\n{USAGE}");
        ExitCode::from(2)
    })
}

/// `neuron emit <st|scl|plcopen|spel|python|kernel> <file|dir>... [-o dir] [--profile p]
/// [--robot name|path.toml] [--policy exact|equivalent|approximate]`
pub fn cmd_emit(args: &[String]) -> ExitCode {
    match emit(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn emit(args: &[String]) -> Result<(), ExitCode> {
    let Some(name) = args.first() else {
        eprintln!(
            "error: `emit` needs a language ({})\n\n{USAGE}",
            neuron_emit::names().join(", ")
        );
        return Err(ExitCode::from(2));
    };
    let Some(emitter) = neuron_emit::emitter(name) else {
        eprintln!(
            "error: unknown emitter `{name}` (the emitters are {})\n\n{USAGE}",
            neuron_emit::names().join(", ")
        );
        return Err(ExitCode::from(2));
    };
    let mut opts = EmitOptions::default();
    let mut out: Option<String> = None;
    let mut policy: Option<Policy> = None;
    let mut paths: Vec<String> = Vec::new();
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" | "--output" => out = Some(value(&mut it, a)?.clone()),
            "--profile" => opts.profile = value(&mut it, a)?.clone(),
            "--robot" => opts.robot = value(&mut it, a)?.clone(),
            "--policy" => {
                let v = value(&mut it, a)?;
                let Some(worst) = Class::parse(v) else {
                    eprintln!(
                        "error: `--policy` is exact, equivalent or approximate, not `{v}`\n\n{USAGE}"
                    );
                    return Err(ExitCode::from(2));
                };
                policy = Some(Policy { worst });
            }
            "--ros2" => {
                eprintln!(
                    "error: `--ros2` is not an option of `neuron emit`: the python emitter always writes `<module>_ros2.py`; run `python <module>.py --ros2` (see docs/translation.md)"
                );
                return Err(ExitCode::from(2));
            }
            other if other.starts_with('-') && other.len() > 1 => {
                eprintln!("error: unknown option `{other}`\n\n{USAGE}");
                return Err(ExitCode::from(2));
            }
            other => paths.push(other.to_string()),
        }
    }
    if paths.is_empty() {
        eprintln!("error: `emit {name}` needs at least one file or directory\n\n{USAGE}");
        return Err(ExitCode::from(2));
    }
    let (sources, manifest) = load(&paths, true)?;
    let analysis = analyze(sources, manifest.as_deref(), &SemaOptions::default());
    eprint!("{}", analysis.render());
    let Some(nir) = analysis.nir.as_ref() else {
        eprintln!("error: the program has errors, so nothing was emitted");
        return Err(ExitCode::from(1));
    };
    let fail = |diags: &[Diag]| {
        for d in diags {
            eprint!("{}", render(d, "error", &analysis.paths, &analysis.texts));
        }
        ExitCode::from(1)
    };
    let package = emitter.emit(nir, &opts).map_err(|d| fail(&d))?;
    if let Some(p) = policy {
        let refused = p.check(&package.fidelity);
        if !refused.is_empty() {
            let code = fail(&refused);
            eprintln!(
                "error: the translation is worse than `{}`, so nothing was written",
                p.worst
            );
            return Err(code);
        }
    }
    let dir = PathBuf::from(out.unwrap_or_else(|| format!("out/{name}")));
    let written = package.write(&dir).map_err(|e| {
        eprintln!("error: cannot write to {}: {e}", dir.display());
        ExitCode::from(2)
    })?;
    for w in &written {
        println!("{}", w.display());
    }
    println!(
        "wrote {} files to {} ({})",
        written.len(),
        dir.display(),
        summary(&package.fidelity)
    );
    Ok(())
}

/// `neuron import <st|scl|plcopen|spel|c|verilog> <file> [-o out.neuron]`
pub fn cmd_import(args: &[String]) -> ExitCode {
    match import(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn import(args: &[String]) -> Result<(), ExitCode> {
    let Some(lang) = args.first() else {
        eprintln!(
            "error: `import` needs a language ({})\n\n{USAGE}",
            neuron_import::languages().join(", ")
        );
        return Err(ExitCode::from(2));
    };
    if !neuron_import::languages().contains(&lang.as_str()) {
        eprintln!(
            "error: unknown language `{lang}` (the importers read {})\n\n{USAGE}",
            neuron_import::languages().join(", ")
        );
        return Err(ExitCode::from(2));
    }
    let mut out: Option<String> = None;
    let mut files: Vec<String> = Vec::new();
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" | "--output" => out = Some(value(&mut it, a)?.clone()),
            other if other.starts_with('-') && other.len() > 1 => {
                eprintln!("error: unknown option `{other}`\n\n{USAGE}");
                return Err(ExitCode::from(2));
            }
            other => files.push(other.to_string()),
        }
    }
    let [file] = files.as_slice() else {
        eprintln!("error: `import {lang}` needs exactly one input file\n\n{USAGE}");
        return Err(ExitCode::from(2));
    };
    let text = read(file)?;
    let name = Path::new(file).file_stem().map_or_else(
        || "imported".to_string(),
        |s| s.to_string_lossy().into_owned(),
    );
    let opts = neuron_import::Options {
        scan_ns: None,
        file: file.clone(),
    };
    let paths = [file.clone()];
    let texts = [text.replace("\r\n", "\n")];
    let imported = neuron_import::import_with(lang, &name, &text, &opts).map_err(|diags| {
        for d in &diags {
            eprint!("{}", render(d, "error", &paths, &texts));
        }
        eprintln!("error: {file} could not be read as {lang}, so nothing was imported");
        ExitCode::from(1)
    })?;
    for d in &imported.diags {
        eprint!("{}", render(d, "warning", &paths, &texts));
    }
    match out {
        None => print!("{}", imported.source),
        Some(o) => {
            let path = PathBuf::from(&o);
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| {
                    eprintln!("error: cannot create {}: {e}", parent.display());
                    ExitCode::from(2)
                })?;
            }
            let report = path.with_extension("fidelity.json");
            for (p, t) in [
                (&path, imported.source.clone()),
                (&report, imported.fidelity.to_json()),
            ] {
                std::fs::write(p, t).map_err(|e| {
                    eprintln!("error: cannot write {}: {e}", p.display());
                    ExitCode::from(2)
                })?;
            }
            println!("wrote {} and {}", path.display(), report.display());
        }
    }
    eprintln!(
        "imported {file} as {lang}: {}{}",
        summary(&imported.fidelity),
        if imported.complete() {
            String::new()
        } else {
            format!(
                "; {} construct(s) not imported (NE1002)",
                imported.diags.len()
            )
        }
    );
    Ok(())
}
