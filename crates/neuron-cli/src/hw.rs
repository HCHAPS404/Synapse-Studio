//! The hardware path of the tool (task B4): `neuron targets`, `emit-c`, `build` and `flash`.
//!
//! The pipeline is: semantic analysis (NIR) → hardware checks against the board
//! (`neuron-target`) → lowering to MIR (`neuron-mir`) → C (`neuron-codegen-c`) → the
//! C compiler of the board. Every step reports its problems with a code and a source
//! location, and nothing is built when one step fails.
//!
//! ```
//! let reg = neuron_target::Registry::builtin().expect("embedded targets");
//! let table = neuron_cli::hw::targets_table(&reg);
//! assert!(table.contains("arduino_uno_r3"));
//! ```

use neuron_codegen_c::{emit_c, Emitted};
use neuron_mir::{lower, verify, Program, Source};
use neuron_nir::Nir;
use neuron_syntax::LineIndex;
use neuron_target::{check, Diag, Registry, Target, Tier, Trust};

/// A program that passed every check, ready to be compiled.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The resolved board.
    pub target: Target,
    /// The MIR program.
    pub program: Program,
    /// The generated C and its source map.
    pub emitted: Emitted,
    /// The screen, when the package has a `ui` module.
    pub ui: Option<neuron_ui::UiApp>,
}

fn tier_name(t: Tier) -> &'static str {
    match t {
        Tier::Runtime => "runtime",
        Tier::Described => "described",
        Tier::Sdk => "sdk",
    }
}

fn trust_name(t: Trust) -> &'static str {
    match t {
        Trust::Verified => "verified",
        Trust::Community => "community",
        Trust::AiGenerated => "ai-generated",
    }
}

fn mhz(hz: u64) -> String {
    let v = hz as f64 / 1e6;
    if (v - v.round()).abs() < 1e-9 {
        format!("{v:.0} MHz")
    } else {
        format!("{v} MHz")
    }
}

/// The table of `neuron targets`: one line per board.
pub fn targets_table(reg: &Registry) -> String {
    let mut out = String::from(
        "board                     chip         arch        clock     tier       trust\n",
    );
    for b in reg.boards() {
        let (chip, arch) = match reg.resolve(&b.name) {
            Ok(t) => (t.chip.name.clone(), t.arch.name.clone()),
            Err(_) => (b.chip.clone(), "?".to_string()),
        };
        out.push_str(&format!(
            "{:<25} {:<12} {:<11} {:<9} {:<10} {}\n",
            b.name,
            chip,
            arch,
            mhz(b.sysclk_hz.unwrap_or(b.crystal_hz)),
            tier_name(b.tier),
            trust_name(b.trust)
        ));
    }
    out.push_str(
        "\n`runtime` boards build and flash with `neuron build` / `neuron flash`; `sdk` boards get a\n\
         vendor-SDK project that `neuron build` compiles when the SDK is installed; `described` boards\n\
         are checked (pins, clocks, rates) but have no runtime port yet. See docs/targets.md.\n",
    );
    out.push_str(
        "\nFPGA boards (hdl modules, `neuron emit-verilog` and `neuron build --board <fpga>`)\n",
    );
    for name in neuron_codegen_hdl::FpgaBoard::names() {
        if let Some(b) = neuron_codegen_hdl::FpgaBoard::builtin(name) {
            out.push_str(&format!(
                "{:<25} {:<12} {:<11} {}\n",
                b.name, b.part, b.package, b.trust
            ));
        }
    }
    out
}

/// The details of one board for `neuron targets <board>`.
pub fn target_detail(reg: &Registry, board: &str) -> Result<String, String> {
    let t = reg
        .resolve(board)
        .map_err(|e| format!("error[NE0607]: {e}"))?;
    let mut out = String::new();
    out.push_str(&format!(
        "{}  ({} tier, {} trust)\n",
        t.board.name,
        tier_name(t.board.tier),
        trust_name(t.board.trust)
    ));
    if !t.board.note.is_empty() {
        out.push_str(&format!("  {}\n", t.board.note));
    }
    out.push_str(&format!(
        "  chip      {} ({} flash, {} bytes RAM, {:.1} V)\n",
        t.chip.name,
        if t.chip.flash_bytes == 0 {
            "external".to_string()
        } else {
            format!("{} bytes", t.chip.flash_bytes)
        },
        t.chip.ram_bytes,
        f64::from(t.board.voltage_mv) / 1000.0
    ));
    out.push_str(&format!(
        "  arch      {} ({}-bit, {}{})\n",
        t.arch.name,
        t.arch.word_bits,
        t.arch.isa,
        if t.arch.fpu { ", FPU" } else { "" }
    ));
    out.push_str(&format!(
        "  clock     crystal {}, system clock at reset {}, max {}\n",
        mhz(t.board.crystal_hz),
        mhz(t.board.sysclk_hz.unwrap_or(t.board.crystal_hz)),
        mhz(t.chip.clock.max_hz)
    ));
    out.push_str(&format!(
        "  runtime   {} (tick {} Hz{})\n",
        t.runtime.name,
        t.runtime.tick_hz,
        if t.runtime.reserves.is_empty() {
            String::new()
        } else {
            format!(", owns {}", t.runtime.reserves.join(", "))
        }
    ));
    if let Some(p) = &t.board.programmer {
        out.push_str(&format!(
            "  flash     {}{}{}\n",
            p.tool,
            p.protocol
                .as_ref()
                .map_or(String::new(), |x| format!(" -c {x}")),
            p.baud.map_or(String::new(), |b| format!(" -b {b}"))
        ));
    }
    out.push_str("  pins\n");
    for (alias, chip_pin) in &t.board.pins {
        let funcs = t
            .chip
            .pins
            .iter()
            .find(|p| &p.name == chip_pin)
            .map(|p| p.functions.join(" "))
            .unwrap_or_default();
        out.push_str(&format!("    {alias:<6} {chip_pin:<8} {funcs}\n"));
    }
    for (what, map) in [("led", &t.board.leds), ("button", &t.board.buttons)] {
        for (name, pin) in map {
            out.push_str(&format!("  {what:<9} {name} = {pin}\n"));
        }
    }
    out.push_str(&format!("  source    {}\n", t.board.source));
    Ok(out)
}

/// Renders a hardware or lowering diagnostic like the compiler renders its own:
/// `error[NE0601]: message` and ` --> file:line:col`.
pub fn render_diag(d: &Diag, paths: &[String], texts: &[String]) -> String {
    let mut out = format!("error[{}]: {}\n", d.code, d.message);
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

/// Sets the board of every `mcu` module, for `--board`.
pub fn override_board(nir: &mut Nir, board: &str) {
    for m in &mut nir.modules {
        if m.dialect == "mcu" {
            m.target = Some(board.to_string());
        }
    }
}

/// Runs the hardware checks, the lowering and the C emitter. On failure the error is the
/// rendered list of diagnostics.
pub fn prepare(
    reg: &Registry,
    nir: &Nir,
    paths: &[String],
    texts: &[String],
) -> Result<Prepared, String> {
    let mut hw = check(reg, nir);
    // A package with a screen only: the board comes from the ui module.
    if hw.target.is_none() {
        if let Some(board) = nir
            .modules
            .iter()
            .find(|m| m.dialect == "ui")
            .and_then(|m| m.target.clone())
        {
            match reg.resolve(&board) {
                Ok(t) => {
                    hw.sysclk_hz = t.board.sysclk_hz.unwrap_or(t.board.crystal_hz);
                    hw.target = Some(t);
                }
                Err(e) => return Err(format!("error[NE0607]: {e}")),
            }
        }
    }
    let render_all = |diags: &[Diag]| -> String {
        let mut s = String::new();
        for d in diags {
            s.push_str(&render_diag(d, paths, texts));
            s.push('\n');
        }
        s
    };
    if !hw.diags.is_empty() {
        return Err(format!(
            "{}error: {} hardware problem(s); nothing was built",
            render_all(&hw.diags),
            hw.diags.len()
        ));
    }
    let Some(target) = hw.target.clone() else {
        return Err("error: no `mcu` module names a board; write `module m for mcu on <board>` or pass `--board`\n\
             (`neuron targets` lists the boards)"
            .to_string());
    };
    let sources: Vec<Source> = paths
        .iter()
        .zip(texts)
        .map(|(p, t)| Source {
            file: p.clone(),
            text: t.clone(),
        })
        .collect();
    let mut lowered = lower(nir, &hw, &sources);
    if lowered.program.board.is_none() {
        lowered.program.board = Some(target.board.name.clone());
        lowered.program.sysclk_hz = hw.sysclk_hz;
        if lowered.program.name.is_empty() {
            lowered.program.name = nir
                .modules
                .first()
                .map(|m| m.name.clone())
                .unwrap_or_default();
        }
    }
    if !lowered.diags.is_empty() {
        return Err(format!(
            "{}error: {} construct(s) outside the subset of the C backend; nothing was built\n\
             (`neuron run` runs the program in the interpreter)",
            render_all(&lowered.diags),
            lowered.diags.len()
        ));
    }
    let problems = verify(&lowered.program);
    if !problems.is_empty() {
        return Err(format!(
            "internal error: the MIR verifier rejected the program (please report it):\n  {}",
            problems.join("\n  ")
        ));
    }
    let ui = neuron_ui::lower_ui(nir, target.chip.ram_bytes, Some(&lowered.program)).map_err(
        |diags| {
            format!(
                "{}error: {} problem(s) in the ui module; nothing was built",
                render_all(&diags),
                diags.len()
            )
        },
    )?;
    let emitted = match &ui {
        Some(app) => {
            neuron_codegen_c::emit_c_ui(&lowered.program, &neuron_ui::emit_ui_c(app), app.period_ms)
        }
        None => emit_c(&lowered.program),
    };
    Ok(Prepared {
        target,
        program: lowered.program,
        emitted,
        ui,
    })
}
