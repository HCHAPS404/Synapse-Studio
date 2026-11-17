//! Drivers for the open FPGA tools: iverilog (simulation), yosys (synthesis), nextpnr and
//! icepack (iCE40 bitstreams), Vivado in batch mode (7-series, when installed) and
//! openFPGALoader (flashing). Tools are found through an environment variable or `PATH`;
//! nothing is downloaded.

use crate::board::{Family, FpgaBoard};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Why a tool run did not happen or failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolError {
    /// The tool is not installed.
    #[error("`{tool}` was not found; install it or set {env} (see docs/hdl.md)")]
    Missing {
        /// Tool.
        tool: String,
        /// Environment variable that points to it.
        env: String,
    },
    /// The tool ran and failed.
    #[error("`{tool}` failed:\n{output}")]
    Failed {
        /// Tool.
        tool: String,
        /// Output.
        output: String,
    },
    /// A file could not be written.
    #[error("{0}")]
    Io(String),
}

/// Finds a tool: `NEURON_<NAME>` first, then `PATH`.
pub fn find(name: &str) -> Option<PathBuf> {
    let env = format!("NEURON_{}", name.to_ascii_uppercase().replace('-', "_"));
    if let Some(v) = std::env::var_os(&env) {
        let p = PathBuf::from(v);
        if p.is_file() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&path)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

fn need(name: &str) -> Result<PathBuf, ToolError> {
    find(name).ok_or_else(|| ToolError::Missing {
        tool: name.to_string(),
        env: format!("NEURON_{}", name.to_ascii_uppercase().replace('-', "_")),
    })
}

fn io(e: impl std::fmt::Display) -> ToolError {
    ToolError::Io(e.to_string())
}

fn run(tool: &str, cmd: &mut Command) -> Result<String, ToolError> {
    let out = cmd.output().map_err(io)?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(ToolError::Failed {
            tool: tool.into(),
            output: text,
        })
    }
}

/// Compiles `design` with a `testbench` in iverilog and runs it; returns what it printed.
pub fn simulate_iverilog(design: &str, testbench: &str, dir: &Path) -> Result<String, ToolError> {
    let iverilog = need("iverilog")?;
    let vvp = need("vvp")?;
    std::fs::create_dir_all(dir).map_err(io)?;
    std::fs::write(dir.join("design.v"), design).map_err(io)?;
    std::fs::write(dir.join("tb.v"), testbench).map_err(io)?;
    run(
        "iverilog",
        Command::new(iverilog)
            .current_dir(dir)
            .args(["-g2005", "-Wall", "-o", "sim.vvp", "tb.v", "design.v"]),
    )?;
    run(
        "vvp",
        Command::new(vvp).current_dir(dir).args(["-n", "sim.vvp"]),
    )
}

/// Synthesizes `top` with yosys for the family and returns the number of cells.
pub fn synth_check_yosys(
    design: &str,
    top: &str,
    family: Family,
    dir: &Path,
) -> Result<u64, ToolError> {
    let yosys = need("yosys")?;
    std::fs::create_dir_all(dir).map_err(io)?;
    std::fs::write(dir.join("design.v"), design).map_err(io)?;
    let synth = match family {
        Family::Ice40 => "synth_ice40",
        Family::Xc7 => "synth_xilinx",
    };
    let script = format!("read_verilog design.v; {synth} -top {top}; stat");
    let log = run(
        "yosys",
        Command::new(yosys).current_dir(dir).args(["-p", &script]),
    )?;
    Ok(log
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Number of cells:"))
        .filter_map(|n| n.trim().parse::<u64>().ok())
        .next_back()
        .unwrap_or(0))
}

/// The files of a bitstream build.
#[derive(Debug, Clone)]
pub struct Bitstream {
    /// The bitstream (`.bin` for iCE40, `.bit` for 7-series).
    pub file: PathBuf,
    /// Tool output.
    pub log: String,
}

/// Builds a bitstream: yosys, nextpnr-ice40 and icepack for iCE40; Vivado in batch mode
/// for 7-series.
pub fn build_bitstream(
    design: &str,
    constraints: &str,
    top: &str,
    board: &FpgaBoard,
    dir: &Path,
) -> Result<Bitstream, ToolError> {
    std::fs::create_dir_all(dir).map_err(io)?;
    std::fs::write(dir.join("top.v"), design).map_err(io)?;
    match board.family {
        Family::Ice40 => {
            std::fs::write(dir.join("top.pcf"), constraints).map_err(io)?;
            let mut log = run(
                "yosys",
                Command::new(need("yosys")?).current_dir(dir).args([
                    "-q",
                    "-p",
                    &format!("read_verilog top.v; synth_ice40 -top {top} -json top.json"),
                ]),
            )?;
            log.push_str(&run(
                "nextpnr-ice40",
                Command::new(need("nextpnr-ice40")?).current_dir(dir).args([
                    &format!("--{}", board.part),
                    "--package",
                    &board.package,
                    "--json",
                    "top.json",
                    "--pcf",
                    "top.pcf",
                    "--asc",
                    "top.asc",
                ]),
            )?);
            log.push_str(&run(
                "icepack",
                Command::new(need("icepack")?)
                    .current_dir(dir)
                    .args(["top.asc", "top.bin"]),
            )?);
            Ok(Bitstream {
                file: dir.join("top.bin"),
                log,
            })
        }
        Family::Xc7 => {
            std::fs::write(dir.join("top.xdc"), constraints).map_err(io)?;
            let tcl = format!(
                "read_verilog top.v\nread_xdc top.xdc\nsynth_design -top {top} -part {}\nopt_design\nplace_design\nroute_design\nwrite_bitstream -force top.bit\n",
                board.part
            );
            std::fs::write(dir.join("build.tcl"), tcl).map_err(io)?;
            let log = run(
                "vivado",
                Command::new(need("vivado")?).current_dir(dir).args([
                    "-mode",
                    "batch",
                    "-nojournal",
                    "-nolog",
                    "-source",
                    "build.tcl",
                ]),
            )?;
            Ok(Bitstream {
                file: dir.join("top.bit"),
                log,
            })
        }
    }
}

/// Loads a bitstream into the board with openFPGALoader.
pub fn flash_fpga(bitstream: &Path, board: &FpgaBoard) -> Result<String, ToolError> {
    let loader = need("openFPGALoader")?;
    run(
        "openFPGALoader",
        Command::new(loader).args(&board.loader_args).arg(bitstream),
    )
}
