//! `neuron` command line tool.
//!
//! Implemented: `--version`, `lex <file>`, `ast <file>`, `check <file|dir>...`
//! (syntax and full semantic analysis), `emit-nir` and `print-nir` (spec 16), `fmt`,
//! `explain`, `lsp`, `run`, `test`, and the hardware path `targets`, `emit-c`, `build`
//! and `flash` (tasks B1 to B4). Planned: `doc`, `pkg`.

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
  neuron fmt [--check] [--stdin] <file|dir>...
                                    rewrite files in the canonical layout; `--check` only
                                    lists the files that would change (exit code 1)
  neuron explain <NE####>           the long explanation of a diagnostic
  neuron lsp                        language server on standard input and output
  neuron run [options] <file|dir>...
                                    run the program in simulated time and print what it does
  neuron test <file|dir>...         run the `test` blocks of the program
  neuron targets [board]            list the boards, or describe one (pins, clocks, runtime)
  neuron emit-c <file|dir>... [--board b] [-o dir]
                                    check the program against its board and print the C
                                    (with `-o`, write it with the runtime sources)
  neuron build <file|dir>... [--board b] [--out dir] [--host | --native]
                                    check, generate C and compile the firmware for the board
                                    (default build/<board>/): app.hex (AVR), app.elf/app.bin/
                                    app.uf2 (RP2040, RP2350, STM32), an idf.py project (ESP32-S3);
                                    `--host` builds an executable that runs in simulated time;
                                    `--native` uses the native AVR backend (no external compiler);
                                    for an hdl program and an FPGA board, Verilog, constraints
                                    and (with yosys/nextpnr or Vivado) the bitstream
  neuron emit-verilog <file|dir>... [--board fpga] [-o dir]
                                    print the Verilog of the hdl modules, or write top.v and the
                                    constraint file of the board
  neuron flash <file|dir>... [--board b] [--port <port> | --drive <dir>] [--native]
                                    build and write the board: avrdude (AVR, --port), the UF2
                                    drive (RP2040/RP2350, --drive or found), probe-rs/OpenOCD/
                                    st-flash (STM32), esptool (ESP32-S3, --port), openFPGALoader

run options:
  --for <time>        simulated time to run, such as `2 s` or `500ms` (default 1 s)
  --scenario <file>   scripted inputs (one `at 100 ms: button = true` per line)
  --jitter <seed>     random execution delays, to check that signals do not depend on them
  --action-time <t>   time a device action takes in the model (default 100 ms)

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

/// `neuron explain NE####`: the long explanation of a diagnostic.
fn cmd_explain(code: &str) -> ExitCode {
    match neuron_cli::explain::explain(code) {
        Some(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("error: `{code}` is not a known diagnostic code (see spec/language/10-diagnostics.md)");
            ExitCode::from(1)
        }
    }
}

/// `neuron fmt [--check] [--stdin] <file|dir>...`: the canonical formatter.
fn cmd_fmt(args: &[String]) -> ExitCode {
    let mut check = false;
    let mut stdin = false;
    let mut paths: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            "--stdin" => stdin = true,
            other => paths.push(other.to_string()),
        }
    }
    if stdin {
        let mut src = String::new();
        if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut src) {
            eprintln!("error: cannot read standard input: {e}");
            return ExitCode::from(2);
        }
        return match neuron_syntax::format::format_source(&src) {
            Ok(f) => {
                print!("{}", f.text);
                ExitCode::SUCCESS
            }
            Err(diags) => {
                for d in &diags {
                    eprint!("{}", d.render("<stdin>", &src));
                }
                ExitCode::from(1)
            }
        };
    }
    if paths.is_empty() {
        eprintln!("error: `fmt` needs at least one file or directory\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for p in &paths {
        let path = Path::new(p);
        if path.is_dir() {
            if let Err(e) = collect(path, &mut files) {
                eprintln!("error: cannot read {p}: {e}");
                return ExitCode::from(2);
            }
        } else {
            files.push(path.to_path_buf());
        }
    }
    let (mut changed, mut same, mut broken) = (0usize, 0usize, 0usize);
    for f in &files {
        let shown = f.to_string_lossy().into_owned();
        let src = match read(&shown) {
            Ok(s) => s,
            Err(code) => return code,
        };
        match neuron_syntax::format::format_source(&src) {
            Err(diags) => {
                broken += 1;
                for d in &diags {
                    eprint!("{}", d.render(&shown, &src));
                }
            }
            Ok(out) if out.text == src => same += 1,
            Ok(out) => {
                changed += 1;
                if check {
                    println!("would reformat {shown}");
                } else if let Err(e) = std::fs::write(f, out.text) {
                    eprintln!("error: cannot write {shown}: {e}");
                    return ExitCode::from(2);
                } else {
                    println!("formatted {shown}");
                }
            }
        }
    }
    eprintln!("{changed} changed, {same} unchanged, {broken} with errors");
    if broken > 0 || (check && changed > 0) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Analyzes the given files and packages. Prints the diagnostics; returns the analysis
/// only when there are no errors.
fn analyze_or_exit(paths: &[String]) -> Result<neuron_sema::Analysis, ExitCode> {
    let (sources, manifest) = load(paths, true)?;
    let analysis = analyze(sources, manifest.as_deref(), &SemaOptions::default());
    eprint!("{}", analysis.render());
    if analysis.nir.is_none() {
        eprintln!("error: the program has errors, so it cannot run");
        return Err(ExitCode::from(1));
    }
    Ok(analysis)
}

/// `neuron run <file|dir>... [--for 2s] [--scenario file] [--jitter seed]`
fn cmd_run(args: &[String]) -> ExitCode {
    let mut paths: Vec<String> = Vec::new();
    let mut opts = neuron_interp::Options::default();
    let mut scenario: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--for" | "--scenario" | "--jitter" | "--action-time" => {
                let Some(v) = it.next() else {
                    eprintln!("error: `{a}` needs a value\n\n{USAGE}");
                    return ExitCode::from(2);
                };
                match a.as_str() {
                    "--for" => match neuron_interp::parse_duration(v) {
                        Ok(ns) => opts.end_ns = ns,
                        Err(e) => {
                            eprintln!("error: --for: {e}");
                            return ExitCode::from(2);
                        }
                    },
                    "--action-time" => match neuron_interp::parse_duration(v) {
                        Ok(ns) => opts.action_ns = ns,
                        Err(e) => {
                            eprintln!("error: --action-time: {e}");
                            return ExitCode::from(2);
                        }
                    },
                    "--jitter" => {
                        let Ok(seed) = v.parse::<u64>() else {
                            eprintln!("error: --jitter needs a number");
                            return ExitCode::from(2);
                        };
                        opts.jitter = Some(neuron_interp::Jitter {
                            seed,
                            max_ns: opts.base_tick_ns,
                        });
                    }
                    _ => scenario = Some(v.clone()),
                }
            }
            other => paths.push(other.to_string()),
        }
    }
    if paths.is_empty() {
        eprintln!("error: `run` needs at least one file or directory\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let analysis = match analyze_or_exit(&paths) {
        Ok(a) => a,
        Err(code) => return code,
    };
    let Some(nir) = analysis.nir else {
        return ExitCode::from(1);
    };
    let mut interp = match neuron_interp::Interp::new(nir, opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    if let Some(file) = scenario {
        let text = match read(&file) {
            Ok(t) => t,
            Err(code) => return code,
        };
        match neuron_interp::parse_scenario(&text) {
            Ok(sc) => interp.load_scenario(&sc),
            Err(e) => {
                eprintln!("error: {file}: {e}");
                return ExitCode::from(2);
            }
        }
    }
    interp.run();
    print!("{}", neuron_interp::render(interp.trace()));
    eprintln!(
        "ran {} of simulated time, {} observations{}",
        neuron_interp::fmt_time(interp.now_ns()),
        interp.trace().len(),
        interp
            .current_fault()
            .map_or(String::new(), |f| format!(", in fault {f}"))
    );
    ExitCode::SUCCESS
}

/// `neuron test <file|dir>...`: runs every `test` block.
fn cmd_test(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("error: `test` needs at least one file or directory\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let analysis = match analyze_or_exit(args) {
        Ok(a) => a,
        Err(code) => return code,
    };
    let Some(nir) = &analysis.nir else {
        return ExitCode::from(1);
    };
    let reports = neuron_interp::run_tests(nir, &neuron_interp::Options::default());
    let mut failed = 0usize;
    for r in &reports {
        let file = &analysis.paths[r.module];
        if r.passed {
            println!("PASS  {file}: {}", r.name);
        } else {
            failed += 1;
            let place = r.at.map_or(String::new(), |(m, span)| {
                let text = &analysis.texts[m];
                let (line, col) = LineIndex::new(text).line_col(text, span[0]);
                format!(" ({}:{line}:{col})", analysis.paths[m])
            });
            println!("FAIL  {file}: {}: {}{place}", r.name, r.message);
        }
    }
    println!(
        "{} test(s), {} passed, {} failed",
        reports.len(),
        reports.len() - failed,
        failed
    );
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn registry() -> Result<neuron_target::Registry, ExitCode> {
    neuron_target::Registry::builtin().map_err(|e| {
        eprintln!("error[NE0607]: the embedded target descriptions are invalid: {e}");
        ExitCode::from(2)
    })
}

/// `neuron targets [board]`: the boards, or the details of one.
fn cmd_targets(args: &[String]) -> ExitCode {
    let reg = match registry() {
        Ok(r) => r,
        Err(code) => return code,
    };
    match args.first() {
        None => {
            print!("{}", neuron_cli::hw::targets_table(&reg));
            ExitCode::SUCCESS
        }
        Some(board) => match neuron_cli::hw::target_detail(&reg, board) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
    }
}

/// Options shared by `emit-c`, `build` and `flash`.
#[derive(Default)]
struct HwArgs {
    paths: Vec<String>,
    board: Option<String>,
    out: Option<String>,
    port: Option<String>,
    drive: Option<String>,
    host: bool,
    native: bool,
}

fn parse_hw_args(cmd: &str, args: &[String]) -> Result<HwArgs, ExitCode> {
    let mut h = HwArgs::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--board" | "-o" | "--out" | "--port" | "--drive" => {
                let Some(v) = it.next() else {
                    eprintln!("error: `{a}` needs a value\n\n{USAGE}");
                    return Err(ExitCode::from(2));
                };
                match a.as_str() {
                    "--board" => h.board = Some(v.clone()),
                    "--port" => h.port = Some(v.clone()),
                    "--drive" => h.drive = Some(v.clone()),
                    _ => h.out = Some(v.clone()),
                }
            }
            "--host" => h.host = true,
            "--native" => h.native = true,
            other => h.paths.push(other.to_string()),
        }
    }
    if h.paths.is_empty() {
        eprintln!("error: `{cmd}` needs at least one file or directory\n\n{USAGE}");
        return Err(ExitCode::from(2));
    }
    Ok(h)
}

/// Analysis, hardware checks, lowering and C, for the hardware commands.
fn prepare_hw(h: &HwArgs) -> Result<neuron_cli::hw::Prepared, ExitCode> {
    let reg = registry()?;
    let (sources, manifest) = load(&h.paths, true)?;
    let analysis = analyze(sources, manifest.as_deref(), &SemaOptions::default());
    eprint!("{}", analysis.render());
    let Some(mut nir) = analysis.nir.clone() else {
        eprintln!("error: the program has errors, so nothing was built");
        return Err(ExitCode::from(1));
    };
    if let Some(b) = &h.board {
        neuron_cli::hw::override_board(&mut nir, b);
    }
    neuron_cli::hw::prepare(&reg, &nir, &analysis.paths, &analysis.texts).map_err(|e| {
        eprintln!("{e}");
        ExitCode::from(1)
    })
}

fn out_dir(h: &HwArgs, p: &neuron_cli::hw::Prepared) -> PathBuf {
    match &h.out {
        Some(o) => PathBuf::from(o),
        None => Path::new("build").join(&p.target.board.name),
    }
}

/// `neuron emit-c <file|dir>... [--board b] [-o dir]`: the generated C, without compiling it.
fn cmd_emit_c(args: &[String]) -> ExitCode {
    let h = match parse_hw_args("emit-c", args) {
        Ok(h) => h,
        Err(code) => return code,
    };
    let p = match prepare_hw(&h) {
        Ok(p) => p,
        Err(code) => return code,
    };
    let Some(dir) = &h.out else {
        print!("{}", p.emitted.c);
        return ExitCode::SUCCESS;
    };
    let dir = PathBuf::from(dir);
    let port = match neuron_codegen_c::Port::for_target(&p.target.runtime.port, &p.target.chip.name)
    {
        Some(port) if !h.host => port,
        _ => neuron_codegen_c::Port::Host,
    };
    let mut files: Vec<(String, String)> = vec![
        ("app.c".into(), p.emitted.c.clone()),
        ("app.map.json".into(), p.emitted.map_json.clone()),
    ];
    for f in neuron_codegen_c::runtime_files(port) {
        files.push((f.name.to_string(), f.text.to_string()));
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("error: cannot create {}: {e}", dir.display());
        return ExitCode::from(2);
    }
    for (name, text) in files {
        if let Err(e) = std::fs::write(dir.join(&name), text) {
            eprintln!("error: cannot write {name}: {e}");
            return ExitCode::from(2);
        }
    }
    eprintln!("wrote the C sources and the runtime to {}", dir.display());
    ExitCode::SUCCESS
}

/// What a build produced.
enum Built {
    /// Intel HEX for avrdude.
    Hex(PathBuf),
    /// A bare-metal Arm image.
    Arm(neuron_codegen_c::ArmBuild),
    /// The build folder of an ESP-IDF project.
    Idf(PathBuf),
    /// A host executable.
    Host,
}

fn fail(e: impl std::fmt::Display) -> ExitCode {
    eprintln!("error: {e}");
    ExitCode::from(1)
}

/// Builds for the board (or for the host with `--host`, or natively with `--native`).
fn build(h: &HwArgs) -> Result<(neuron_cli::hw::Prepared, Built), ExitCode> {
    use neuron_codegen_c::Port;
    let p = prepare_hw(h)?;
    let dir = out_dir(h, &p);
    let board = p.target.board.name.clone();
    if h.host {
        let exe = neuron_codegen_c::build_host(&p.emitted, &dir).map_err(fail)?;
        eprintln!("built {} for the host (simulated time)", exe.display());
        eprintln!("run it with: {} --for 2000 [scenario-file]", exe.display());
        return Ok((p, Built::Host));
    }
    if h.native {
        let n = neuron_native_avr::compile(&p.program).map_err(|diags| {
            for d in &diags {
                eprintln!("error[{}]: {}", d.code, d.message);
                if let Some(help) = &d.help {
                    eprintln!("  = help: {help}");
                }
            }
            ExitCode::from(1)
        })?;
        std::fs::create_dir_all(&dir).map_err(fail)?;
        let hex = dir.join("app.hex");
        std::fs::write(&hex, &n.hex).map_err(fail)?;
        std::fs::write(dir.join("app.lst"), &n.listing).map_err(fail)?;
        eprintln!(
            "built {} for {board} with the native backend: {} bytes of flash, {} bytes of SRAM for data (listing in app.lst)",
            hex.display(),
            n.words * 2,
            n.ram_bytes
        );
        return Ok((p, Built::Hex(hex)));
    }
    let port = Port::for_target(&p.target.runtime.port, &p.target.chip.name);
    match (p.target.board.tier, port) {
        (neuron_target::Tier::Described, _) | (_, None) => {
            eprintln!(
                "error: `{board}` is a `described` board: its pins, clocks and rates were checked, but the runtime \
                 has no port for `{}` yet.\nUse `neuron emit-c` for the C, `--host` for a host build, or \
                 `neuron run` for the interpreter. Boards that build: `neuron targets`.",
                p.target.chip.name
            );
            Err(ExitCode::from(1))
        }
        (_, Some(Port::Avr)) => {
            let b = neuron_codegen_c::build_avr(
                &p.emitted,
                &p.target.chip.mcu,
                p.program.sysclk_hz,
                &dir,
            )
            .map_err(fail)?;
            eprintln!(
                "built {} for {board} ({} at {} Hz)",
                b.hex.display(),
                p.target.chip.mcu,
                p.program.sysclk_hz
            );
            if !b.size_report.trim().is_empty() {
                eprintln!("{}", b.size_report.trim_end());
            }
            Ok((p, Built::Hex(b.hex)))
        }
        (_, Some(Port::EspIdf)) => {
            match neuron_codegen_c::build_esp_idf(&p.emitted, &p.program.name, &dir) {
                Ok(build_dir) => {
                    eprintln!("built the ESP-IDF project in {}", dir.display());
                    Ok((p, Built::Idf(build_dir)))
                }
                Err(neuron_codegen_c::BuildError::ToolMissing { .. }) => {
                    eprintln!(
                    "wrote the ESP-IDF project to {}; ESP-IDF is not installed here, so it was not compiled.\n\
                     With ESP-IDF: cd {} && idf.py set-target esp32s3 && idf.py build",
                    dir.display(),
                    dir.display()
                );
                    Err(ExitCode::from(1))
                }
                Err(e) => Err(fail(e)),
            }
        }
        (_, Some(port)) => {
            if let Some(io) = p
                .program
                .ios
                .iter()
                .find(|io| matches!(io.kind, neuron_mir::IoK::Pwm | neuron_mir::IoK::Adc))
            {
                eprintln!(
                    "error[NE0610]: `{}` ({:?}) is not supported by the `{}` runtime port yet; gpio, interrupts and uart are",
                    io.name, io.kind, p.target.runtime.name
                );
                return Err(ExitCode::from(1));
            }
            let b = neuron_codegen_c::build_arm(&p.emitted, port, p.program.sysclk_hz, &dir)
                .map_err(fail)?;
            eprintln!(
                "built {} for {board} ({} at {} Hz)",
                b.elf.display(),
                p.target.chip.name,
                p.program.sysclk_hz
            );
            if let Some(u) = &b.uf2 {
                eprintln!("UF2 image: {}", u.display());
            }
            if !b.size_report.trim().is_empty() {
                eprintln!("{}", b.size_report.trim_end());
            }
            Ok((p, Built::Arm(b)))
        }
    }
}

/// The analysis of the paths, when every module is an `hdl` module.
fn hdl_program(h: &HwArgs) -> Result<Option<neuron_nir::Nir>, ExitCode> {
    let (sources, manifest) = load(&h.paths, true)?;
    let analysis = analyze(sources, manifest.as_deref(), &SemaOptions::default());
    let Some(nir) = analysis.nir else {
        eprint!("{}", analysis.render());
        eprintln!("error: the program has errors, so nothing was built");
        return Err(ExitCode::from(1));
    };
    let hdl = !nir.modules.is_empty() && nir.modules.iter().all(|m| m.dialect == "hdl");
    Ok(hdl.then_some(nir))
}

/// The Verilog flow: Verilog, constraints and, when asked and possible, the bitstream.
fn hdl_flow(
    h: &HwArgs,
    nir: &neuron_nir::Nir,
    bitstream: bool,
) -> Result<Option<(PathBuf, neuron_codegen_hdl::FpgaBoard)>, ExitCode> {
    let v = neuron_codegen_hdl::emit_verilog(nir).map_err(|diags| {
        for d in &diags {
            eprintln!("error[{}]: {} ({})", d.code, d.message, d.file);
            if let Some(help) = &d.help {
                eprintln!("  = help: {help}");
            }
        }
        ExitCode::from(1)
    })?;
    let board = match &h.board {
        Some(b) => match neuron_codegen_hdl::FpgaBoard::builtin(b) {
            Some(board) => Some(board),
            None => {
                eprintln!(
                    "error[NE0607]: `{b}` is not an FPGA board (FPGA boards: {})",
                    neuron_codegen_hdl::FpgaBoard::names().join(", ")
                );
                return Err(ExitCode::from(1));
            }
        },
        None => None,
    };
    let top = v.modules.last().cloned().unwrap_or_default();
    let Some(out) = h
        .out
        .clone()
        .map(PathBuf::from)
        .or_else(|| board.as_ref().map(|b| Path::new("build").join(&b.name)))
    else {
        print!("{}", v.text);
        return Ok(None);
    };
    std::fs::create_dir_all(&out).map_err(fail)?;
    std::fs::write(out.join("top.v"), &v.text).map_err(fail)?;
    let Some(board) = board else {
        eprintln!(
            "wrote {} (top module `{top}`); pass `--board` for the constraints",
            out.join("top.v").display()
        );
        return Ok(None);
    };
    let constraints = neuron_codegen_hdl::emit_constraints(&v, &top, &board).map_err(|diags| {
        for d in &diags {
            eprintln!("error[{}]: {}", d.code, d.message);
        }
        ExitCode::from(1)
    })?;
    let ext = match board.family {
        neuron_codegen_hdl::Family::Ice40 => "pcf",
        neuron_codegen_hdl::Family::Xc7 => "xdc",
    };
    std::fs::write(out.join(format!("top.{ext}")), &constraints).map_err(fail)?;
    eprintln!(
        "wrote top.v and top.{ext} for `{top}` on {} to {}",
        board.name,
        out.display()
    );
    if !bitstream {
        return Ok(None);
    }
    match neuron_codegen_hdl::tools::build_bitstream(&v.text, &constraints, &top, &board, &out) {
        Ok(b) => {
            eprintln!("built the bitstream {}", b.file.display());
            Ok(Some((b.file, board)))
        }
        Err(e @ neuron_codegen_hdl::tools::ToolError::Missing { .. }) => {
            eprintln!(
                "{e}\nThe Verilog and the constraints are ready in {}.",
                out.display()
            );
            Err(ExitCode::from(1))
        }
        Err(e) => Err(fail(e)),
    }
}

/// `neuron emit-verilog <file|dir>... [--board fpga] [-o dir]`
fn cmd_emit_verilog(args: &[String]) -> ExitCode {
    let h = match parse_hw_args("emit-verilog", args) {
        Ok(h) => h,
        Err(code) => return code,
    };
    match hdl_program(&h) {
        Ok(Some(nir)) => match hdl_flow(&h, &nir, false) {
            Ok(_) => ExitCode::SUCCESS,
            Err(code) => code,
        },
        Ok(None) => {
            eprintln!("error: `emit-verilog` needs `hdl` modules (`module m for hdl`)");
            ExitCode::from(1)
        }
        Err(code) => code,
    }
}

/// `neuron build <file|dir>... [--board b] [--out dir] [--host | --native]`
fn cmd_build(args: &[String]) -> ExitCode {
    let h = match parse_hw_args("build", args) {
        Ok(h) => h,
        Err(code) => return code,
    };
    match hdl_program(&h) {
        Ok(Some(nir)) => {
            return match hdl_flow(&h, &nir, h.board.is_some()) {
                Ok(_) => ExitCode::SUCCESS,
                Err(code) => code,
            }
        }
        Ok(None) => {}
        Err(code) => return code,
    }
    match build(&h) {
        Ok(_) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// The mass-storage drive of a board in UF2 boot mode, when one is mounted.
fn find_uf2_drive() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for letter in b'D'..=b'Z' {
            candidates.push(PathBuf::from(format!("{}:\\", letter as char)));
        }
    } else {
        for base in ["/media", "/run/media", "/Volumes"] {
            if let Ok(users) = std::fs::read_dir(base) {
                for u in users.flatten() {
                    candidates.push(u.path());
                    if let Ok(vols) = std::fs::read_dir(u.path()) {
                        candidates.extend(vols.flatten().map(|v| v.path()));
                    }
                }
            }
        }
    }
    candidates
        .into_iter()
        .find(|d| d.join("INFO_UF2.TXT").is_file())
}

/// `neuron flash <file|dir>... --port COM3 [--board b]`: builds and writes the board.
fn cmd_flash(args: &[String]) -> ExitCode {
    let h = match parse_hw_args("flash", args) {
        Ok(h) => h,
        Err(code) => return code,
    };
    if h.host {
        eprintln!("error: `--host` builds cannot be flashed");
        return ExitCode::from(2);
    }
    // FPGA: build the bitstream, then openFPGALoader.
    match hdl_program(&h) {
        Ok(Some(nir)) => {
            if h.board.is_none() {
                eprintln!("error: flashing an FPGA needs `--board` (`neuron targets` lists the FPGA boards)");
                return ExitCode::from(2);
            }
            return match hdl_flow(&h, &nir, true) {
                Ok(Some((bit, board))) => match neuron_codegen_hdl::tools::flash_fpga(&bit, &board)
                {
                    Ok(log) => {
                        eprintln!("{}\ndone", log.trim_end());
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(e),
                },
                Ok(None) => ExitCode::from(1),
                Err(code) => code,
            };
        }
        Ok(None) => {}
        Err(code) => return code,
    }
    let tool = {
        // Decide before building, so that a missing port is reported at once.
        let reg = match registry() {
            Ok(r) => r,
            Err(code) => return code,
        };
        let (sources, manifest) = match load(&h.paths, true) {
            Ok(x) => x,
            Err(code) => return code,
        };
        let nir = analyze(sources, manifest.as_deref(), &SemaOptions::default()).nir;
        let board = h.board.clone().or_else(|| {
            nir.as_ref()
                .and_then(|n| n.modules.iter().find_map(|m| m.target.clone()))
        });
        board
            .and_then(|b| reg.resolve(&b).ok())
            .and_then(|t| t.board.programmer.map(|p| p.tool))
            .unwrap_or_else(|| "avrdude".into())
    };
    let tool = if h.native {
        "avrdude".to_string()
    } else {
        tool
    };
    if matches!(tool.as_str(), "avrdude" | "esptool") && h.port.is_none() {
        eprintln!("error: `flash` needs `--port` (such as COM3 or /dev/ttyACM0)\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let (p, built) = match build(&h) {
        Ok(x) => x,
        Err(code) => return code,
    };
    let board = p.target.board.name.clone();
    eprintln!("flashing {board} with {tool} ...");
    let result = match (tool.as_str(), &built) {
        ("avrdude", Built::Hex(hex)) => {
            let prog = p.target.board.programmer.clone();
            let opts = neuron_codegen_c::FlashOptions {
                mcu: p.target.chip.mcu.clone(),
                programmer: prog
                    .as_ref()
                    .and_then(|x| x.protocol.clone())
                    .unwrap_or_else(|| "arduino".into()),
                port: h.port.clone().unwrap_or_default(),
                baud: prog.as_ref().and_then(|x| x.baud).unwrap_or(115_200),
            };
            neuron_codegen_c::flash_avr(hex, &opts)
        }
        ("uf2", Built::Arm(b)) => {
            let drive = h.drive.clone().map(PathBuf::from).or_else(find_uf2_drive);
            match (drive, &b.uf2) {
                (Some(d), Some(u)) => neuron_codegen_c::flash_uf2(u, &d),
                (None, _) => Err(neuron_codegen_c::BuildError::Io(
                    "no UF2 drive found: hold BOOTSEL while plugging the board in, or pass `--drive`".into(),
                )),
                (_, None) => Err(neuron_codegen_c::BuildError::Io("the build made no UF2 file".into())),
            }
        }
        ("swd", Built::Arm(b)) => {
            neuron_codegen_c::flash_stm32(&b.elf, &b.bin, &p.target.chip.name)
        }
        ("esptool", Built::Idf(dir)) => {
            neuron_codegen_c::flash_esptool(dir, &h.port.clone().unwrap_or_default())
        }
        (other, _) => Err(neuron_codegen_c::BuildError::Io(format!(
            "flashing `{board}` with `{other}` is not supported"
        ))),
    };
    match result {
        Ok(log) => {
            eprintln!("{}", log.trim_end());
            eprintln!("done");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
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
    // Hardware checks against the board of each `mcu` module (spec 07, task B1).
    let mut hw_errors = 0usize;
    if let Some(nir) = &analysis.nir {
        let reg = match registry() {
            Ok(r) => r,
            Err(code) => return code,
        };
        for d in neuron_target::check(&reg, nir).diags {
            eprintln!(
                "{}",
                neuron_cli::hw::render_diag(&d, &analysis.paths, &analysis.texts)
            );
            if let Some(i) = analysis.paths.iter().position(|p| *p == d.file) {
                per_file[i] += 1;
            }
            hw_errors += 1;
        }
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
    if analysis.is_ok() && hw_errors == 0 {
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
        Some("lsp") => {
            let stdin = std::io::stdin();
            match neuron_lsp::serve(stdin.lock(), std::io::stdout().lock()) {
                Ok(0) => ExitCode::SUCCESS,
                Ok(_) => ExitCode::from(1),
                Err(e) => {
                    eprintln!("error: language server: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Some("fmt") => cmd_fmt(&args[1..]),
        Some("explain") => match args.get(1) {
            Some(code) => cmd_explain(code),
            None => {
                eprintln!("error: `explain` needs a code such as NE0201\n\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("run") => cmd_run(&args[1..]),
        Some("targets") => cmd_targets(&args[1..]),
        Some("emit-c") => cmd_emit_c(&args[1..]),
        Some("emit-verilog") => cmd_emit_verilog(&args[1..]),
        Some("build") => cmd_build(&args[1..]),
        Some("flash") => cmd_flash(&args[1..]),
        Some("test") => cmd_test(&args[1..]),
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
