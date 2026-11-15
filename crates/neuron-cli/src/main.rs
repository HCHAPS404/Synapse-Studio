//! `neuron` command line tool.
//!
//! Implemented: `--version`, `lex <file>`, `ast <file>`, `check <file|dir>...`
//! (syntax and full semantic analysis), `emit-nir` and `print-nir` (spec 16).
//! Planned: `fmt`, `build`, `flash`, `test`, `doc`, `pkg`, `explain`.

use neuron_sema::{analyze, Reports, SemaOptions, SourceFile};
use neuron_syntax::{lex, parse, LineIndex};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "neuron - the Neuron language toolchain

usage:
  neuron --version
  neuron lex <file.neuron>          print the token stream
  neuron ast <file.neuron>          print the syntax tree
  neuron check [options] <file|dir>...
                                    analyze files, or a package (a directory with neuron.toml);
                                    exit code 1 on errors
  neuron emit-nir <file|dir>... [-o out.nir.json]
                                    analyze and write the program as canonical NIR JSON
                                    (stdout by default); nothing is written on errors
  neuron print-nir <file.nir.json>  print NIR back as Neuron source

check options:
  --cpu-hz <n>     CPU clock in Hz: enables worst-case execution time analysis
  --ram <bytes>    RAM of the target: enables the memory budget check
  --report         print timing, resources, memory and counterexamples";

fn cmd_ast(path: &str) -> ExitCode {
    let src = match read(path) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let out = parse(&src);
    println!("{:#?}", out.file);
    for d in &out.diagnostics {
        eprint!("{}", d.render(path, &src));
    }
    if out.diagnostics.iter().any(|d| d.is_error()) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn read(path: &str) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|e| {
        eprintln!("error: cannot read {path}: {e}");
        ExitCode::from(2)
    })
}

fn cmd_lex(path: &str) -> ExitCode {
    let src = match read(path) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let index = LineIndex::new(&src);
    let out = lex(&src);
    for t in out.significant() {
        let (line, col) = index.line_col(&src, t.span.start);
        let text = if t.kind.is_layout() {
            String::new()
        } else {
            format!("{:?}", t.text(&src))
        };
        println!("{line}:{col}\t{}\t{text}", t.kind.describe());
    }
    for d in &out.diagnostics {
        eprint!("{}", d.render(path, &src));
    }
    if out.diagnostics.iter().any(|d| d.is_error()) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "neuron") {
            out.push(p);
        }
    }
    Ok(())
}

/// Reads the files and directories given on the command line. A directory is a package:
/// its `neuron.toml` is used as the manifest. With `relative`, the paths of files found
/// in a directory are relative to it and use `/`, so the output does not depend on where
/// the command runs or on the operating system.
fn load(paths: &[String], relative: bool) -> Result<(Vec<SourceFile>, Option<String>), ExitCode> {
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut manifest: Option<String> = None;
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            if let Ok(m) = std::fs::read_to_string(path.join("neuron.toml")) {
                manifest = Some(m);
            }
            let mut found = Vec::new();
            if let Err(e) = collect(path, &mut found) {
                eprintln!("error: cannot read {p}: {e}");
                return Err(ExitCode::from(2));
            }
            for f in found {
                let shown = if relative {
                    f.strip_prefix(path)
                        .unwrap_or(&f)
                        .to_string_lossy()
                        .replace('\\', "/")
                } else {
                    f.to_string_lossy().into_owned()
                };
                files.push((f, shown));
            }
        } else {
            let shown = if relative {
                path.file_name()
                    .map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned())
            } else {
                p.clone()
            };
            files.push((path.to_path_buf(), shown));
        }
    }
    let mut sources = Vec::new();
    for (f, shown) in files {
        let text = read(&f.to_string_lossy())?;
        sources.push(SourceFile { path: shown, text });
    }
    Ok((sources, manifest))
}

/// `neuron emit-nir <file|dir>... [-o out.nir.json]`: analyzes and writes canonical NIR.
fn cmd_emit_nir(args: &[String]) -> ExitCode {
    let mut out: Option<String> = None;
    let mut paths: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-o" || a == "--output" {
            let Some(v) = it.next() else {
                eprintln!("error: `{a}` needs a file name\n\n{USAGE}");
                return ExitCode::from(2);
            };
            out = Some(v.clone());
        } else {
            paths.push(a.clone());
        }
    }
    if paths.is_empty() {
        eprintln!("error: `emit-nir` needs at least one file or directory\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let (sources, manifest) = match load(&paths, true) {
        Ok(x) => x,
        Err(code) => return code,
    };
    let analysis = analyze(sources, manifest.as_deref(), &SemaOptions::default());
    eprint!("{}", analysis.render());
    let Some(nir) = analysis.nir.as_ref() else {
        eprintln!("error: no NIR was written because the program has errors");
        return ExitCode::from(1);
    };
    let text = neuron_sema::nir::to_json(nir);
    match out {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, text) {
                eprintln!("error: cannot write {path}: {e}");
                return ExitCode::from(2);
            }
            eprintln!("wrote {path}");
        }
        None => print!("{text}"),
    }
    ExitCode::SUCCESS
}

/// `neuron print-nir <file.nir.json>`: prints NIR back as Neuron source.
fn cmd_print_nir(path: &str) -> ExitCode {
    let text = match read(path) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let nir = match neuron_sema::nir::from_json(&text) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("error: {path} is not valid NIR: {e}");
            return ExitCode::from(1);
        }
    };
    let modules = neuron_sema::nir::print(&nir);
    let many = modules.len() > 1;
    for (i, (file, src)) in modules.iter().enumerate() {
        if many {
            if i > 0 {
                println!();
            }
            println!("# ---- {file} ----");
        }
        print!("{src}");
    }
    ExitCode::SUCCESS
}

fn print_reports(r: &Reports) {
    if !r.timing.is_empty() {
        println!("\ntiming (ns)");
        println!("  module/activity            kind       prio  period   wcet  blocking  response  deadline  ok");
        for t in &r.timing {
            let n = |v: Option<u64>| v.map_or("-".to_string(), |x| x.to_string());
            println!(
                "  {:<26} {:<10} {:>4}  {:>6} {:>6} {:>9} {:>9} {:>9}  {}",
                format!("{}/{}", t.module, t.name),
                t.kind,
                t.priority,
                n(t.period_ns),
                n(t.wcet_ns),
                t.blocking_ns,
                n(t.response_ns),
                n(t.deadline_ns),
                t.schedulable.map_or("?", |b| if b { "yes" } else { "NO" })
            );
        }
    }
    if !r.resources.is_empty() {
        println!("\nshared resources");
        for x in &r.resources {
            println!(
                "  {}/{} ({}) ceiling {} used by {}{}",
                x.module,
                x.name,
                x.kind,
                x.ceiling,
                x.accessors.join(", "),
                if x.cross_core { " [cross-core]" } else { "" }
            );
        }
    }
    if !r.memory.is_empty() {
        println!("\nmemory (bytes)");
        for m in &r.memory {
            println!(
                "  {}: static {} + stack {} + runtime {} = {}{}",
                m.module,
                m.static_bytes,
                m.stack_bytes,
                m.runtime_bytes,
                m.total_bytes,
                m.ram_bytes.map_or(String::new(), |r| format!(" of {r}"))
            );
        }
    }
    for c in &r.counterexamples {
        println!("\ncounterexample for `{}` in {}", c.property, c.module);
        for s in &c.trace {
            let inputs: Vec<String> = s
                .inputs
                .iter()
                .map(|(n, v)| format!("{n}={}", u8::from(*v)))
                .collect();
            println!(
                "  tick {:>3} t={} ns  {}  inputs: {}",
                s.tick,
                s.time_ns,
                s.states.join(", "),
                inputs.join(" ")
            );
        }
    }
}

fn cmd_check(args: &[String]) -> ExitCode {
    let mut opts = SemaOptions::default();
    let mut report = false;
    let mut paths: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--report" => report = true,
            "--cpu-hz" | "--ram" => {
                let Some(v) = it.next().and_then(|v| v.parse::<u64>().ok()) else {
                    eprintln!("error: `{a}` needs a number\n\n{USAGE}");
                    return ExitCode::from(2);
                };
                if a == "--cpu-hz" {
                    opts.cpu_hz = Some(v);
                } else {
                    opts.ram_bytes = Some(v);
                }
            }
            other => paths.push(other.to_string()),
        }
    }
    if paths.is_empty() {
        eprintln!("error: `check` needs at least one file or directory\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let (sources, manifest) = match load(&paths, false) {
        Ok(x) => x,
        Err(code) => return code,
    };
    let analysis = analyze(sources, manifest.as_deref(), &opts);
    eprint!("{}", analysis.render());
    let mut per_file = vec![0usize; analysis.paths.len()];
    for d in analysis.errors() {
        per_file[d.file] += 1;
    }
    for (path, n) in analysis.paths.iter().zip(&per_file) {
        if *n == 0 {
            println!("{path}: ok");
        } else {
            println!("{path}: {n} error(s)");
        }
    }
    if report {
        print_reports(&analysis.reports);
    }
    if analysis.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!(
                "neuron {} (language edition {})",
                env!("CARGO_PKG_VERSION"),
                neuron_syntax::LANGUAGE_EDITION
            );
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") | None => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("lex") => match args.get(1) {
            Some(path) => cmd_lex(path),
            None => {
                eprintln!("error: `lex` needs a file\n\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("ast") => match args.get(1) {
            Some(path) => cmd_ast(path),
            None => {
                eprintln!("error: `ast` needs a file\n\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("check") => cmd_check(&args[1..]),
        Some("emit-nir") => cmd_emit_nir(&args[1..]),
        Some("print-nir") => match args.get(1) {
            Some(path) => cmd_print_nir(path),
            None => {
                eprintln!("error: `print-nir` needs a .nir.json file\n\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some(other) => {
            eprintln!("error: unknown command '{other}'\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
