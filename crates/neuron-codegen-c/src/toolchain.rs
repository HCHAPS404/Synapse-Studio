//! Drivers for the external tools: the host C compiler, `avr-gcc`, `avr-objcopy` and `avrdude`.
//!
//! Tools are looked up in the environment first (`NEURON_CC`, `NEURON_AVR_GCC`,
//! `NEURON_AVRDUDE`) and then in `PATH`. Nothing is downloaded or installed here.

use crate::emit::Emitted;
use crate::runtime::{runtime_files, Port};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Why a build or a flash did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// A tool is not installed.
    #[error("`{tool}` was not found; install it or set {env} (see docs/phase-b-prerequisites.md)")]
    ToolMissing {
        /// Tool name.
        tool: String,
        /// Environment variable that points to it.
        env: &'static str,
    },
    /// A tool ran and failed.
    #[error("`{tool}` failed:\n{output}")]
    Failed {
        /// Tool name.
        tool: String,
        /// Its output.
        output: String,
    },
    /// A file could not be written or read.
    #[error("{0}")]
    Io(String),
}

/// Finds a tool: the environment variable first, then `PATH`.
pub fn find_tool(env: &str, names: &[&str]) -> Option<PathBuf> {
    if let Some(v) = std::env::var_os(env) {
        let p = PathBuf::from(v);
        if p.components().count() > 1 && p.exists() {
            return Some(p);
        }
        if let Some(found) = search_path(&p.to_string_lossy()) {
            return Some(found);
        }
    }
    names.iter().find_map(|n| search_path(n))
}

fn search_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    for dir in std::env::split_paths(&path) {
        for candidate in [dir.join(&exe), dir.join(name)] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn io_err(e: impl std::fmt::Display) -> BuildError {
    BuildError::Io(e.to_string())
}

fn run(tool: &str, cmd: &mut Command) -> Result<String, BuildError> {
    let out = cmd.output().map_err(io_err)?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(BuildError::Failed {
            tool: tool.to_string(),
            output: text,
        })
    }
}

fn write_sources(e: &Emitted, port: Port, dir: &Path) -> Result<Vec<PathBuf>, BuildError> {
    std::fs::create_dir_all(dir).map_err(io_err)?;
    let app = dir.join("app.c");
    std::fs::write(&app, &e.c).map_err(io_err)?;
    std::fs::write(dir.join("app.map.json"), &e.map_json).map_err(io_err)?;
    let mut sources = vec![app];
    for f in runtime_files(port) {
        let path = dir.join(f.name);
        std::fs::write(&path, f.text).map_err(io_err)?;
        if f.name.ends_with(".c") {
            sources.push(path);
        }
    }
    Ok(sources)
}

/// Builds the host executable (`-DNRT_HOST`) with strict warnings.
pub fn build_host(e: &Emitted, dir: &Path) -> Result<PathBuf, BuildError> {
    let cc = find_tool("NEURON_CC", &["gcc", "cc", "clang"]).ok_or(BuildError::ToolMissing {
        tool: "gcc".into(),
        env: "NEURON_CC",
    })?;
    let sources = write_sources(e, Port::Host, dir)?;
    let exe = dir.join(format!("app_host{}", std::env::consts::EXE_SUFFIX));
    let mut cmd = Command::new(&cc);
    cmd.args([
        "-std=c99",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-O1",
        "-DNRT_HOST",
        "-I",
    ])
    .arg(dir)
    .arg("-o")
    .arg(&exe)
    .args(&sources);
    run("gcc", &mut cmd)?;
    Ok(exe)
}

/// The files of an AVR build.
#[derive(Debug, Clone)]
pub struct AvrBuild {
    /// ELF image.
    pub elf: PathBuf,
    /// Intel HEX image, as `avrdude` wants it.
    pub hex: PathBuf,
    /// Output of `avr-size`.
    pub size_report: String,
}

/// Builds the firmware for an AVR chip with `-Wall -Wextra -Werror`.
pub fn build_avr(e: &Emitted, mcu: &str, f_cpu: u64, dir: &Path) -> Result<AvrBuild, BuildError> {
    let gcc = find_tool("NEURON_AVR_GCC", &["avr-gcc"]).ok_or(BuildError::ToolMissing {
        tool: "avr-gcc".into(),
        env: "NEURON_AVR_GCC",
    })?;
    let sources = write_sources(e, Port::Avr, dir)?;
    let elf = dir.join("app.elf");
    let hex = dir.join("app.hex");
    let mut cmd = Command::new(&gcc);
    cmd.args([
        "-std=c99",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-Os",
        "-ffunction-sections",
        "-fdata-sections",
        "-Wl,--gc-sections",
    ])
    .arg(format!("-mmcu={mcu}"))
    .arg(format!("-DF_CPU={f_cpu}UL"))
    .arg("-DNRT_PORT_AVR")
    .arg("-I")
    .arg(dir)
    .arg("-o")
    .arg(&elf)
    .args(&sources);
    run("avr-gcc", &mut cmd)?;
    let beside = |name: &str| -> PathBuf {
        let sibling = gcc.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        if sibling.is_file() {
            sibling
        } else {
            PathBuf::from(name)
        }
    };
    run(
        "avr-objcopy",
        Command::new(beside("avr-objcopy"))
            .args(["-O", "ihex", "-R", ".eeprom"])
            .arg(&elf)
            .arg(&hex),
    )?;
    let size_report = run(
        "avr-size",
        Command::new(beside("avr-size"))
            .arg("--mcu")
            .arg(mcu)
            .arg("-C")
            .arg(&elf),
    )
    .or_else(|_| run("avr-size", Command::new(beside("avr-size")).arg(&elf)))
    .unwrap_or_default();
    Ok(AvrBuild {
        elf,
        hex,
        size_report,
    })
}

/// The part name `avrdude` uses for a chip.
pub fn avr_chip_name(mcu: &str) -> &str {
    match mcu {
        "atmega328p" => "m328p",
        "atmega2560" => "m2560",
        other => other,
    }
}

/// How to reach the board with `avrdude`.
#[derive(Debug, Clone)]
pub struct FlashOptions {
    /// Chip (`atmega328p`).
    pub mcu: String,
    /// Programmer protocol (`arduino`, `wiring`).
    pub programmer: String,
    /// Serial port (`COM3`, `/dev/ttyACM0`).
    pub port: String,
    /// Bootloader speed.
    pub baud: u32,
}

/// Writes a HEX image to the board.
pub fn flash_avr(hex: &Path, opts: &FlashOptions) -> Result<String, BuildError> {
    let avrdude = find_tool("NEURON_AVRDUDE", &["avrdude"]).ok_or(BuildError::ToolMissing {
        tool: "avrdude".into(),
        env: "NEURON_AVRDUDE",
    })?;
    let mut cmd = Command::new(&avrdude);
    // The Arduino package keeps `avrdude.conf` beside the binary.
    if let Some(dir) = avrdude.parent() {
        for rel in ["../etc/avrdude.conf", "avrdude.conf", "etc/avrdude.conf"] {
            let conf = dir.join(rel);
            if conf.is_file() {
                cmd.arg("-C").arg(conf);
                break;
            }
        }
    }
    cmd.arg("-p")
        .arg(avr_chip_name(&opts.mcu))
        .arg("-c")
        .arg(&opts.programmer)
        .arg("-P")
        .arg(&opts.port)
        .arg("-b")
        .arg(opts.baud.to_string())
        .arg("-D")
        .arg(format!("-Uflash:w:{}:i", hex.display()));
    run("avrdude", &mut cmd)
}
