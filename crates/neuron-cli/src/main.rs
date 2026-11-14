//! `neuron` command line tool.
//!
//! Implemented: `--version`, `lex <file>`, `ast <file>`, `check <file|dir>...`
//! (syntax and full semantic analysis). Planned: `fmt`, `build`, `flash`, `test`,
//! `doc`, `pkg`, `explain`.

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
    let mut files: Vec<PathBuf> = Vec::new();
    let mut manifest: Option<String> = None;
    for p in &paths {
        let path = Path::new(p);
        if path.is_dir() {
            if let Ok(m) = std::fs::read_to_string(path.join("neuron.toml")) {
                manifest = Some(m);
            }
            if let Err(e) = collect(path, &mut files) {
                eprintln!("error: cannot read {p}: {e}");
                return ExitCode::from(2);
            }
        } else {
            files.push(path.to_path_buf());
        }
    }
    let mut sources = Vec::new();
    for f in &files {
        let display = f.to_string_lossy().into_owned();
        match read(&display) {
            Ok(text) => sources.push(SourceFile {
                path: display,
                text,
            }),
            Err(code) => return code,
        }
    }
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
        Some(other) => {
            eprintln!("error: unknown command '{other}'\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
