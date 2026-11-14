//! `neuron` command line tool.
//!
//! Implemented: `--version`, `lex <file>`, `ast <file>`, `check <file>...`
//! (lexical and syntax checks). Planned: `fmt`, `build`, `flash`, `test`, `doc`,
//! `pkg`, `explain`.

use neuron_syntax::{lex, parse, LineIndex};
use std::process::ExitCode;

const USAGE: &str = "neuron - the Neuron language toolchain

usage:
  neuron --version
  neuron lex <file.neuron>          print the token stream
  neuron ast <file.neuron>          print the syntax tree
  neuron check <file.neuron>...     report diagnostics (exit code 1 on errors)";

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

fn cmd_check(paths: &[String]) -> ExitCode {
    if paths.is_empty() {
        eprintln!("error: `check` needs at least one file\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let mut errors = 0;
    for path in paths {
        let src = match read(path) {
            Ok(s) => s,
            Err(code) => return code,
        };
        let out = parse(&src);
        for d in &out.diagnostics {
            eprintln!("{}", d.render(path, &src));
        }
        let n = out.diagnostics.iter().filter(|d| d.is_error()).count();
        errors += n;
        if n == 0 {
            println!("{path}: ok");
        } else {
            println!("{path}: {n} error(s)");
        }
    }
    if errors > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
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
