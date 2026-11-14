//! `neuron` command line tool (scaffold). Subcommands (`check`, `fmt`, `build`,
//! `flash`, `test`, `doc`, `pkg`) arrive with the compiler tasks.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!(
                "neuron {} (language edition {})",
                env!("CARGO_PKG_VERSION"),
                neuron_syntax::LANGUAGE_EDITION
            );
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") | None => {
            println!("neuron - the Neuron language toolchain (scaffold)");
            println!("usage: neuron --version | --help");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown command '{other}'");
            ExitCode::from(2)
        }
    }
}
