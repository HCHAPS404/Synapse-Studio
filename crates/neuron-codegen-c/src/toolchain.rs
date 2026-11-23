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
    let mut files = runtime_files(port);
    if e.ui {
        match crate::runtime::ui_files(port) {
            Some(ui) => files.extend(ui),
            None => {
                return Err(BuildError::Io(format!(
                    "displays are not supported by the {port:?} runtime port yet (NE0610); the AVR and ESP-IDF ports have the display bus"
                )))
            }
        }
    }
    for f in files {
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

/// The files of an Arm build.
#[derive(Debug, Clone)]
pub struct ArmBuild {
    /// ELF image.
    pub elf: PathBuf,
    /// Raw binary from the load address of the image.
    pub bin: PathBuf,
    /// UF2 file for the boards that load one (RP2040, RP2350).
    pub uf2: Option<PathBuf>,
    /// Output of `arm-none-eabi-size`.
    pub size_report: String,
    /// Address the binary is loaded at.
    pub load_address: u32,
}

/// CPU flags, linker script and load address of an Arm port.
fn arm_profile(port: Port) -> Option<(&'static [&'static str], &'static str, u32, Option<u32>)> {
    match port {
        Port::Rp2040 => Some((
            &["-mcpu=cortex-m0plus", "-mthumb"],
            "rp_ram.ld",
            0x2000_0000,
            Some(crate::uf2::FAMILY_RP2040),
        )),
        Port::Rp2350 => Some((
            &["-mcpu=cortex-m33", "-mthumb", "-mfloat-abi=soft"],
            "rp_ram.ld",
            0x2000_0000,
            Some(crate::uf2::FAMILY_RP2350_ARM_S),
        )),
        Port::Stm32f4 => Some((
            &["-mcpu=cortex-m4", "-mthumb", "-mfloat-abi=soft"],
            "stm32f446.ld",
            0x0800_0000,
            None,
        )),
        Port::Stm32u5 => Some((
            &["-mcpu=cortex-m33", "-mthumb", "-mfloat-abi=soft"],
            "stm32u585.ld",
            0x0800_0000,
            None,
        )),
        Port::Nrf52 => Some((
            &["-mcpu=cortex-m4", "-mthumb", "-mfloat-abi=soft"],
            "nrf52840.ld",
            0x0002_6000,
            Some(crate::uf2::FAMILY_NRF52840),
        )),
        Port::Samd21 => Some((
            &["-mcpu=cortex-m0plus", "-mthumb"],
            "samd21g18.ld",
            0x0000_2000,
            None,
        )),
        _ => None,
    }
}

/// Builds the firmware for a bare-metal Arm port (RP2040, RP2350, STM32F4) with
/// `arm-none-eabi-gcc` and `-Wall -Wextra -Werror`.
pub fn build_arm(e: &Emitted, port: Port, f_cpu: u64, dir: &Path) -> Result<ArmBuild, BuildError> {
    let Some((cpu, ld, load, family)) = arm_profile(port) else {
        return Err(BuildError::Io(format!(
            "{port:?} is not a bare-metal Arm port"
        )));
    };
    let gcc =
        find_tool("NEURON_ARM_GCC", &["arm-none-eabi-gcc"]).ok_or(BuildError::ToolMissing {
            tool: "arm-none-eabi-gcc".into(),
            env: "NEURON_ARM_GCC",
        })?;
    let sources = write_sources(e, port, dir)?;
    let elf = dir.join("app.elf");
    let bin = dir.join("app.bin");
    let mut cmd = Command::new(&gcc);
    cmd.args(cpu)
        .args([
            "-std=c99",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Os",
            "-ffunction-sections",
            "-fdata-sections",
            "-nostartfiles",
            "-Wl,--gc-sections",
        ])
        .arg(format!("-D{}", port.define()))
        .arg(format!("-DF_CPU={f_cpu}u"))
        .arg("-I")
        .arg(dir)
        .arg("-T")
        .arg(dir.join(ld))
        .arg("-o")
        .arg(&elf)
        .args(&sources)
        .arg("-lgcc");
    run("arm-none-eabi-gcc", &mut cmd)?;
    let beside = |name: &str| -> PathBuf {
        let sibling = gcc.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        if sibling.is_file() {
            sibling
        } else {
            PathBuf::from(name)
        }
    };
    run(
        "arm-none-eabi-objcopy",
        Command::new(beside("arm-none-eabi-objcopy"))
            .args(["-O", "binary"])
            .arg(&elf)
            .arg(&bin),
    )?;
    let size_report = run(
        "arm-none-eabi-size",
        Command::new(beside("arm-none-eabi-size")).arg(&elf),
    )
    .unwrap_or_default();
    let uf2 = match family {
        Some(fam) => {
            let bytes = std::fs::read(&bin).map_err(io_err)?;
            let path = dir.join("app.uf2");
            std::fs::write(&path, crate::uf2::uf2(&bytes, load, fam)).map_err(io_err)?;
            Some(path)
        }
        None => None,
    };
    Ok(ArmBuild {
        elf,
        bin,
        uf2,
        size_report,
        load_address: load,
    })
}

/// Copies a UF2 file to the mass-storage drive of a board in boot mode (`RPI-RP2`, `RP2350`).
/// `drive` is the mount point or drive letter (`E:`, `/media/user/RPI-RP2`).
pub fn flash_uf2(uf2: &Path, drive: &Path) -> Result<String, BuildError> {
    if !drive.is_dir() {
        return Err(BuildError::Io(format!(
            "{} is not a mounted drive; hold BOOTSEL while plugging the board in, then pass its drive",
            drive.display()
        )));
    }
    let dest = drive.join("app.uf2");
    std::fs::copy(uf2, &dest).map_err(io_err)?;
    Ok(format!("copied {} to {}", uf2.display(), dest.display()))
}

/// Writes a flash image to an STM32 board with the first tool found: `probe-rs`, `openocd`
/// or `st-flash`.
pub fn flash_stm32(elf: &Path, bin: &Path, chip: &str) -> Result<String, BuildError> {
    if let Some(p) = find_tool("NEURON_PROBE_RS", &["probe-rs"]) {
        return run(
            "probe-rs",
            Command::new(p)
                .args(["download", "--chip", &chip.to_ascii_uppercase()])
                .arg(elf),
        );
    }
    if let Some(o) = find_tool("NEURON_OPENOCD", &["openocd"]) {
        let program = format!(
            "program {} verify reset exit",
            elf.display().to_string().replace('\\', "/")
        );
        return run(
            "openocd",
            Command::new(o).args([
                "-f",
                "interface/stlink.cfg",
                "-f",
                "target/stm32f4x.cfg",
                "-c",
                &program,
            ]),
        );
    }
    if let Some(s) = find_tool("NEURON_ST_FLASH", &["st-flash"]) {
        return run(
            "st-flash",
            Command::new(s)
                .args(["--reset", "write"])
                .arg(bin)
                .arg("0x08000000"),
        );
    }
    Err(BuildError::ToolMissing {
        tool: "probe-rs, openocd or st-flash".into(),
        env: "NEURON_PROBE_RS",
    })
}

/// Writes a raw binary to a SAM-BA board (Arduino Nano 33 IoT) with `bossac` over the serial
/// port. The application lives at flash offset 0x2000, above the bootloader; `bossac`
/// auto-detects the board's bootloader port.
pub fn flash_bossac(bin: &Path) -> Result<String, BuildError> {
    let bossac = find_tool("NEURON_BOSSAC", &["bossac"]).ok_or(BuildError::ToolMissing {
        tool: "bossac".into(),
        env: "NEURON_BOSSAC",
    })?;
    run(
        "bossac",
        Command::new(bossac)
            .args(["-i", "-d", "-U", "-e", "-w", "-v", "-R", "--offset=0x2000"])
            .arg(bin),
    )
}

/// The files of an ESP-IDF project that builds the program: `(relative path, text)`.
pub fn esp_idf_project(e: &Emitted, name: &str) -> Vec<(String, String)> {
    let mut files = vec![
        (
            "CMakeLists.txt".to_string(),
            format!(
                "# Generated by neuron. Build with: idf.py set-target esp32s3 && idf.py build\ncmake_minimum_required(VERSION 3.16)\ninclude($ENV{{IDF_PATH}}/tools/cmake/project.cmake)\nproject({name})\n"
            ),
        ),
        (
            "main/CMakeLists.txt".to_string(),
            format!(
                "idf_component_register(SRCS \"app.c\" \"nrt_core.c\" \"nrt_port_esp_idf.c\"{}\n                       INCLUDE_DIRS \".\"\n                       REQUIRES driver esp_timer)\ntarget_compile_definitions(${{COMPONENT_LIB}} PRIVATE NRT_PORT_ESP_IDF)\n",
                if e.ui { " \"nrt_ui.c\" \"nrt_bus_esp_idf.c\"" } else { "" }
            ),
        ),
        ("main/app.c".to_string(), e.c.clone()),
        ("main/app.map.json".to_string(), e.map_json.clone()),
    ];
    let mut rt = runtime_files(Port::EspIdf);
    if e.ui {
        rt.extend(crate::runtime::ui_files(Port::EspIdf).unwrap_or_default());
    }
    for f in rt {
        files.push((format!("main/{}", f.name), f.text.to_string()));
    }
    files
}

/// Writes the ESP-IDF project and runs `idf.py build` when ESP-IDF is installed.
pub fn build_esp_idf(e: &Emitted, name: &str, dir: &Path) -> Result<PathBuf, BuildError> {
    for (rel, text) in esp_idf_project(e, name) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io_err)?;
        }
        std::fs::write(&path, text).map_err(io_err)?;
    }
    let idf =
        find_tool("NEURON_IDF_PY", &["idf.py"]).filter(|_| std::env::var_os("IDF_PATH").is_some());
    let Some(idf) = idf else {
        return Err(BuildError::ToolMissing {
            tool: "ESP-IDF (idf.py with IDF_PATH)".into(),
            env: "IDF_PATH",
        });
    };
    run(
        "idf.py",
        Command::new(&idf)
            .current_dir(dir)
            .args(["set-target", "esp32s3"]),
    )?;
    run("idf.py", Command::new(&idf).current_dir(dir).arg("build"))?;
    Ok(dir.join("build"))
}

/// Writes an ESP32-S3 image with `esptool`.
pub fn flash_esptool(build_dir: &Path, port: &str) -> Result<String, BuildError> {
    let tool =
        find_tool("NEURON_ESPTOOL", &["esptool", "esptool.py"]).ok_or(BuildError::ToolMissing {
            tool: "esptool".into(),
            env: "NEURON_ESPTOOL",
        })?;
    run(
        "esptool",
        Command::new(tool).current_dir(build_dir).args([
            "--chip",
            "esp32s3",
            "-p",
            port,
            "-b",
            "460800",
            "write_flash",
            "@flash_args",
        ]),
    )
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
