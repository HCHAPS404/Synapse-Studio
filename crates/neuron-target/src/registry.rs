//! Loading, validating and resolving target descriptions.
//!
//! The descriptions of `targets/` are embedded at build time, so `neuron build` works
//! from any directory. A directory on disk can be overlaid (`Registry::overlay_dir`) to
//! add or replace a device without rebuilding the tool.

use crate::model::{Arch, Board, Chip, Runtime, Target};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::path::Path;

/// Errors of the target stack. Every one of them is reported as `NE0607`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetError {
    /// A description file could not be read or parsed.
    #[error("cannot read {file}: {message}")]
    Parse {
        /// File name.
        file: String,
        /// Parser message.
        message: String,
    },
    /// A name does not exist in the registry.
    #[error("unknown {kind} `{name}`{}", known_hint(.known))]
    Unknown {
        /// `board`, `chip`, `arch` or `runtime`.
        kind: &'static str,
        /// The missing name.
        name: String,
        /// Names that exist.
        known: Vec<String>,
    },
    /// A file is well formed but inconsistent.
    #[error("{file}: {message}")]
    Invalid {
        /// File name.
        file: String,
        /// What is wrong.
        message: String,
    },
}

fn known_hint(known: &[String]) -> String {
    if known.is_empty() {
        String::new()
    } else {
        format!(" (known: {})", known.join(", "))
    }
}

/// Embedded descriptions: `(file name, text)`.
const BUILTIN: &[(&str, &str)] = &[
    (
        "avr8.arch.toml",
        include_str!("../../../targets/arch/avr8.arch.toml"),
    ),
    (
        "thumbv6m.arch.toml",
        include_str!("../../../targets/arch/thumbv6m.arch.toml"),
    ),
    (
        "thumbv7em.arch.toml",
        include_str!("../../../targets/arch/thumbv7em.arch.toml"),
    ),
    (
        "thumbv8m.arch.toml",
        include_str!("../../../targets/arch/thumbv8m.arch.toml"),
    ),
    (
        "xtensa-lx7.arch.toml",
        include_str!("../../../targets/arch/xtensa-lx7.arch.toml"),
    ),
    (
        "atmega328p.chip.toml",
        include_str!("../../../targets/chips/atmega328p.chip.toml"),
    ),
    (
        "atmega2560.chip.toml",
        include_str!("../../../targets/chips/atmega2560.chip.toml"),
    ),
    (
        "rp2040.chip.toml",
        include_str!("../../../targets/chips/rp2040.chip.toml"),
    ),
    (
        "rp2350.chip.toml",
        include_str!("../../../targets/chips/rp2350.chip.toml"),
    ),
    (
        "esp32s3.chip.toml",
        include_str!("../../../targets/chips/esp32s3.chip.toml"),
    ),
    (
        "stm32f446.chip.toml",
        include_str!("../../../targets/chips/stm32f446.chip.toml"),
    ),
    (
        "stm32n657.chip.toml",
        include_str!("../../../targets/chips/stm32n657.chip.toml"),
    ),
    (
        "nrf52840.chip.toml",
        include_str!("../../../targets/chips/nrf52840.chip.toml"),
    ),
    (
        "samd21g18.chip.toml",
        include_str!("../../../targets/chips/samd21g18.chip.toml"),
    ),
    (
        "stm32u585.chip.toml",
        include_str!("../../../targets/chips/stm32u585.chip.toml"),
    ),
    (
        "arduino_uno_r3.board.toml",
        include_str!("../../../targets/boards/arduino_uno_r3.board.toml"),
    ),
    (
        "arduino_mega_2560.board.toml",
        include_str!("../../../targets/boards/arduino_mega_2560.board.toml"),
    ),
    (
        "raspberry_pi_pico.board.toml",
        include_str!("../../../targets/boards/raspberry_pi_pico.board.toml"),
    ),
    (
        "raspberry_pi_pico_2w.board.toml",
        include_str!("../../../targets/boards/raspberry_pi_pico_2w.board.toml"),
    ),
    (
        "esp32_s3_devkit.board.toml",
        include_str!("../../../targets/boards/esp32_s3_devkit.board.toml"),
    ),
    (
        "xiao_esp32s3.board.toml",
        include_str!("../../../targets/boards/xiao_esp32s3.board.toml"),
    ),
    (
        "nucleo_f446re.board.toml",
        include_str!("../../../targets/boards/nucleo_f446re.board.toml"),
    ),
    (
        "nucleo_n657x0_q.board.toml",
        include_str!("../../../targets/boards/nucleo_n657x0_q.board.toml"),
    ),
    (
        "nano_33_ble_sense.board.toml",
        include_str!("../../../targets/boards/nano_33_ble_sense.board.toml"),
    ),
    (
        "nano_33_iot.board.toml",
        include_str!("../../../targets/boards/nano_33_iot.board.toml"),
    ),
    (
        "uno_q.board.toml",
        include_str!("../../../targets/boards/uno_q.board.toml"),
    ),
    (
        "baremetal-avr.runtime.toml",
        include_str!("../../../targets/runtimes/baremetal-avr.runtime.toml"),
    ),
    (
        "baremetal-rp.runtime.toml",
        include_str!("../../../targets/runtimes/baremetal-rp.runtime.toml"),
    ),
    (
        "baremetal-stm32.runtime.toml",
        include_str!("../../../targets/runtimes/baremetal-stm32.runtime.toml"),
    ),
    (
        "baremetal-nrf52.runtime.toml",
        include_str!("../../../targets/runtimes/baremetal-nrf52.runtime.toml"),
    ),
    (
        "baremetal-samd21.runtime.toml",
        include_str!("../../../targets/runtimes/baremetal-samd21.runtime.toml"),
    ),
    (
        "baremetal-stm32u5.runtime.toml",
        include_str!("../../../targets/runtimes/baremetal-stm32u5.runtime.toml"),
    ),
    (
        "esp-idf.runtime.toml",
        include_str!("../../../targets/runtimes/esp-idf.runtime.toml"),
    ),
];

fn parse<T: DeserializeOwned>(file: &str, text: &str) -> Result<T, TargetError> {
    toml::from_str(text).map_err(|e| TargetError::Parse {
        file: file.to_string(),
        message: e.to_string().replace('\n', " "),
    })
}

fn stem<'a>(file: &'a str, suffix: &str) -> &'a str {
    file.strip_suffix(suffix).unwrap_or(file)
}

/// All descriptions, by kind and name.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    archs: BTreeMap<String, Arch>,
    chips: BTreeMap<String, Chip>,
    boards: BTreeMap<String, Board>,
    runtimes: BTreeMap<String, Runtime>,
}

impl Registry {
    /// The descriptions embedded in the tool.
    pub fn builtin() -> Result<Registry, TargetError> {
        let mut reg = Registry::default();
        for (file, text) in BUILTIN {
            reg.add_file(file, text)?;
        }
        reg.validate()?;
        Ok(reg)
    }

    /// Adds or replaces descriptions from `dir/{arch,chips,boards,runtimes}/*.toml`.
    pub fn overlay_dir(&mut self, dir: &Path) -> Result<(), TargetError> {
        for sub in ["arch", "chips", "boards", "runtimes"] {
            let Ok(entries) = std::fs::read_dir(dir.join(sub)) else {
                continue;
            };
            let mut files: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
            files.sort();
            for path in files {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !name.ends_with(".toml") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).map_err(|e| TargetError::Parse {
                    file: name.clone(),
                    message: e.to_string(),
                })?;
                self.add_file(&name, &text)?;
            }
        }
        self.validate()
    }

    fn add_file(&mut self, file: &str, text: &str) -> Result<(), TargetError> {
        if file.ends_with(".arch.toml") {
            let a: Arch = parse(file, text)?;
            check_name(file, ".arch.toml", &a.name, a.schema)?;
            self.archs.insert(a.name.clone(), a);
        } else if file.ends_with(".chip.toml") {
            let c: Chip = parse(file, text)?;
            check_name(file, ".chip.toml", &c.name, c.schema)?;
            self.chips.insert(c.name.clone(), c);
        } else if file.ends_with(".board.toml") {
            let b: Board = parse(file, text)?;
            check_name(file, ".board.toml", &b.name, b.schema)?;
            self.boards.insert(b.name.clone(), b);
        } else if file.ends_with(".runtime.toml") {
            let r: Runtime = parse(file, text)?;
            check_name(file, ".runtime.toml", &r.name, r.schema)?;
            self.runtimes.insert(r.name.clone(), r);
        } else {
            return Err(TargetError::Invalid {
                file: file.to_string(),
                message:
                    "the file name must end in .arch.toml, .chip.toml, .board.toml or .runtime.toml"
                        .to_string(),
            });
        }
        Ok(())
    }

    /// Names of all boards, sorted.
    pub fn board_names(&self) -> Vec<String> {
        self.boards.keys().cloned().collect()
    }

    /// All boards, sorted by name.
    pub fn boards(&self) -> impl Iterator<Item = &Board> {
        self.boards.values()
    }

    /// The architectures, chips and runtimes are reached through [`Registry::resolve`].
    pub fn chip(&self, name: &str) -> Option<&Chip> {
        self.chips.get(name)
    }

    /// Resolves a board with its chip, architecture and runtime.
    pub fn resolve(&self, board: &str) -> Result<Target, TargetError> {
        let b = self.boards.get(board).ok_or_else(|| TargetError::Unknown {
            kind: "board",
            name: board.to_string(),
            known: self.board_names(),
        })?;
        let chip = self
            .chips
            .get(&b.chip)
            .ok_or_else(|| TargetError::Unknown {
                kind: "chip",
                name: b.chip.clone(),
                known: self.chips.keys().cloned().collect(),
            })?;
        let arch = self
            .archs
            .get(&chip.arch)
            .ok_or_else(|| TargetError::Unknown {
                kind: "arch",
                name: chip.arch.clone(),
                known: self.archs.keys().cloned().collect(),
            })?;
        let runtime = self
            .runtimes
            .get(&b.runtime)
            .ok_or_else(|| TargetError::Unknown {
                kind: "runtime",
                name: b.runtime.clone(),
                known: self.runtimes.keys().cloned().collect(),
            })?;
        Ok(Target {
            board: b.clone(),
            chip: chip.clone(),
            arch: arch.clone(),
            runtime: runtime.clone(),
        })
    }

    /// Checks every description and every reference between them (property TGT-1).
    pub fn validate(&self) -> Result<(), TargetError> {
        let bad = |file: String, message: String| TargetError::Invalid { file, message };
        for b in self.boards.values() {
            let file = format!("{}.board.toml", b.name);
            let target = self.resolve(&b.name)?;
            for (what, map) in [("pin", &b.pins), ("led", &b.leds), ("button", &b.buttons)] {
                for (alias, chip_pin) in map {
                    if !target.chip.pins.iter().any(|p| &p.name == chip_pin) {
                        return Err(bad(
                            file,
                            format!(
                                "{what} `{alias}` names `{chip_pin}`, which is not a pin of `{}`",
                                target.chip.name
                            ),
                        ));
                    }
                }
            }
            if b.crystal_hz == 0 {
                return Err(bad(file, "crystal_hz must be greater than zero".into()));
            }
        }
        for c in self.chips.values() {
            let file = format!("{}.chip.toml", c.name);
            let mut seen = std::collections::BTreeSet::new();
            for p in &c.pins {
                if !seen.insert(&p.name) {
                    return Err(bad(file, format!("pin `{}` is listed twice", p.name)));
                }
                if p.functions.is_empty() {
                    return Err(bad(file, format!("pin `{}` has no function", p.name)));
                }
            }
            for per in &c.peripherals {
                for (role, pin) in &per.pins {
                    if !c.pins.iter().any(|p| &p.name == pin) {
                        return Err(bad(
                            file,
                            format!(
                                "peripheral `{}` role `{role}` names unknown pin `{pin}`",
                                per.name
                            ),
                        ));
                    }
                }
            }
            // `flash_bytes = 0` means the program lives in external flash.
            if c.ram_bytes == 0 {
                return Err(bad(file, "ram_bytes must be greater than zero".into()));
            }
            if c.clock.sources.is_empty() {
                return Err(bad(file, "the clock tree needs at least one source".into()));
            }
            if !self.archs.contains_key(&c.arch) {
                return Err(TargetError::Unknown {
                    kind: "arch",
                    name: c.arch.clone(),
                    known: self.archs.keys().cloned().collect(),
                });
            }
        }
        Ok(())
    }
}

fn check_name(file: &str, suffix: &str, name: &str, schema: u32) -> Result<(), TargetError> {
    if schema != 1 {
        return Err(TargetError::Invalid {
            file: file.to_string(),
            message: format!("unsupported schema {schema} (this tool reads schema 1)"),
        });
    }
    if stem(file, suffix) != name {
        return Err(TargetError::Invalid {
            file: file.to_string(),
            message: format!("`name = \"{name}\"` does not match the file name"),
        });
    }
    Ok(())
}
