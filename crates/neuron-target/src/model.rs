//! The data model of the target description stack (spec 07, chapter 7 first page).
//!
//! A *target* is four small TOML files: architecture, chip, board and runtime. Adding a
//! device adds files; the compiler does not change. Every file carries `schema`, `name`,
//! `source` (where the facts come from) and `trust`.

use serde::Deserialize;
use std::collections::BTreeMap;

/// Trust level of a description (spec 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trust {
    /// Checked against the device and against its public documentation.
    Verified,
    /// Contributed and not checked by the maintainers.
    Community,
    /// Produced by a model and not reviewed. Cannot deploy to hardware.
    AiGenerated,
}

/// How far the toolchain supports a board (the tier table of `docs/targets.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    /// Build, simulate and (with the hardware) flash.
    Runtime,
    /// Described and checked (pins, clocks); no runtime port yet.
    Described,
    /// The runtime port is written against the vendor SDK; the tool generates the SDK project
    /// and builds it when the SDK is installed.
    Sdk,
}

/// Instruction set architecture.
#[derive(Debug, Clone, Deserialize)]
pub struct Arch {
    /// Schema version of the file.
    pub schema: u32,
    /// Name, the file stem (`avr8`).
    pub name: String,
    /// Origin of the facts and their licence.
    pub source: String,
    /// Trust level.
    pub trust: Trust,
    /// Instruction set (`avr`, `thumb2`, `xtensa`, `rv32`).
    pub isa: String,
    /// Native word size in bits.
    pub word_bits: u32,
    /// `little` or `big`.
    pub endian: String,
    /// Separate program and data memory (spec 07 §7.7).
    #[serde(default)]
    pub harvard: bool,
    /// Hardware floating point.
    #[serde(default)]
    pub fpu: bool,
    /// Widest access that is atomic, in bits.
    pub atomic_bits: u32,
    /// How interrupt priorities map (`avr-vectors`, `nvic`, ...).
    pub interrupt_model: String,
    /// C compiler driver for this architecture.
    pub toolchain: String,
}

/// A clock source (crystal, internal oscillator).
#[derive(Debug, Clone, Deserialize)]
pub struct ClockSource {
    /// Name (`xtal`, `hse`).
    pub name: String,
    /// Lowest accepted frequency.
    pub min_hz: u64,
    /// Highest accepted frequency.
    pub max_hz: u64,
}

/// Limits of a phase-locked loop.
#[derive(Debug, Clone, Deserialize)]
pub struct Pll {
    /// Input range after the pre-divider.
    pub input_min_hz: u64,
    /// Input range after the pre-divider.
    pub input_max_hz: u64,
    /// Voltage-controlled oscillator range.
    pub vco_min_hz: u64,
    /// Voltage-controlled oscillator range.
    pub vco_max_hz: u64,
    /// Smallest multiplier.
    pub mul_min: u32,
    /// Largest multiplier.
    pub mul_max: u32,
    /// Smallest pre-divider.
    pub pre_min: u32,
    /// Largest pre-divider.
    pub pre_max: u32,
}

/// The clock tree of a chip.
#[derive(Debug, Clone, Deserialize)]
pub struct ClockSpec {
    /// Highest system clock.
    pub max_hz: u64,
    /// Possible sources.
    pub sources: Vec<ClockSource>,
    /// PLL, when the chip has one.
    #[serde(default)]
    pub pll: Option<Pll>,
    /// Post-dividers applied to the PLL output, or to the source without a PLL.
    #[serde(default = "one")]
    pub post_dividers: Vec<u32>,
}

fn one() -> Vec<u32> {
    vec![1]
}

/// A pin of the package.
#[derive(Debug, Clone, Deserialize)]
pub struct Pin {
    /// Chip name (`PB5`, `GPIO25`).
    pub name: String,
    /// Port letter, for ports of 8 bits.
    #[serde(default)]
    pub port: Option<String>,
    /// Bit in the port.
    #[serde(default)]
    pub bit: Option<u8>,
    /// Functions: `gpio`, `pwm:timer1.a`, `adc:0`, `int:0`, `uart0.tx`, `adc1.in0`, ...
    pub functions: Vec<String>,
}

/// A peripheral instance.
#[derive(Debug, Clone, Deserialize)]
pub struct Peripheral {
    /// Name (`uart0`, `timer1`).
    pub name: String,
    /// Kind (`uart`, `timer`, `adc`, `spi`, `i2c`).
    pub kind: String,
    /// Other accepted names (`usart0`).
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Instance number, for numbered peripherals.
    #[serde(default)]
    pub index: Option<u32>,
    /// Counter width of a timer.
    #[serde(default)]
    pub bits: Option<u32>,
    /// Prescaler values of a timer.
    #[serde(default)]
    pub prescalers: Vec<u32>,
    /// Default pins by role (`tx`, `rx`).
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
}

/// A chip.
#[derive(Debug, Clone, Deserialize)]
pub struct Chip {
    /// Schema version of the file.
    pub schema: u32,
    /// Name.
    pub name: String,
    /// Origin of the facts and their licence.
    pub source: String,
    /// Trust level.
    pub trust: Trust,
    /// Architecture name.
    pub arch: String,
    /// Name the C compiler knows (`atmega328p`).
    pub mcu: String,
    /// Program memory in bytes.
    pub flash_bytes: u64,
    /// Data memory in bytes.
    pub ram_bytes: u64,
    /// Supply voltage in millivolts.
    pub voltage_mv: u32,
    /// Clock tree.
    pub clock: ClockSpec,
    /// Pins.
    pub pins: Vec<Pin>,
    /// Peripherals.
    #[serde(default)]
    pub peripherals: Vec<Peripheral>,
}

/// How a board is programmed.
#[derive(Debug, Clone, Deserialize)]
pub struct Programmer {
    /// Tool (`avrdude`, `picotool`, `esptool`).
    pub tool: String,
    /// Protocol the tool speaks (`arduino`, `wiring`).
    #[serde(default)]
    pub protocol: Option<String>,
    /// Serial speed of the bootloader.
    #[serde(default)]
    pub baud: Option<u32>,
}

/// A board.
#[derive(Debug, Clone, Deserialize)]
pub struct Board {
    /// Schema version of the file.
    pub schema: u32,
    /// Name.
    pub name: String,
    /// Origin of the facts and their licence.
    pub source: String,
    /// Trust level.
    pub trust: Trust,
    /// Support level of the toolchain.
    pub tier: Tier,
    /// Chip name.
    pub chip: String,
    /// Runtime name.
    pub runtime: String,
    /// Crystal or oscillator frequency.
    pub crystal_hz: u64,
    /// Board supply voltage.
    pub voltage_mv: u32,
    /// System clock at reset, when it is not the crystal (defaults to `crystal_hz`).
    #[serde(default)]
    pub sysclk_hz: Option<u64>,
    /// Names printed on the board, mapped to chip pins.
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
    /// On-board LEDs, mapped to chip pins.
    #[serde(default)]
    pub leds: BTreeMap<String, String>,
    /// On-board buttons, mapped to chip pins.
    #[serde(default)]
    pub buttons: BTreeMap<String, String>,
    /// Programmer.
    #[serde(default)]
    pub programmer: Option<Programmer>,
    /// Free text for the tier table.
    #[serde(default)]
    pub note: String,
}

/// A platform runtime.
#[derive(Debug, Clone, Deserialize)]
pub struct Runtime {
    /// Schema version of the file.
    pub schema: u32,
    /// Name.
    pub name: String,
    /// Origin of the facts and their licence.
    pub source: String,
    /// Trust level.
    pub trust: Trust,
    /// `baremetal`, `vendor-sdk` or `linux`.
    pub kind: String,
    /// Port directory of `runtime/neuron-rt/port/`.
    pub port: String,
    /// Scheduler tick in hertz.
    pub tick_hz: u32,
    /// Peripherals the runtime owns (the tick timer).
    #[serde(default)]
    pub reserves: Vec<String>,
    /// Extra compiler flags.
    #[serde(default)]
    pub cflags: Vec<String>,
}

/// A fully resolved target: the board and everything it refers to.
#[derive(Debug, Clone)]
pub struct Target {
    /// The board.
    pub board: Board,
    /// Its chip.
    pub chip: Chip,
    /// The chip's architecture.
    pub arch: Arch,
    /// The runtime.
    pub runtime: Runtime,
}

impl Target {
    /// The chip pin behind a name printed on the board, or a chip pin name itself.
    pub fn pin(&self, name: &str) -> Option<&Pin> {
        let chip_name = self.board.pins.get(name).map_or(name, String::as_str);
        self.chip.pins.iter().find(|p| p.name == chip_name)
    }

    /// The peripheral called `name` (or one of its aliases), case-insensitively.
    pub fn peripheral(&self, name: &str) -> Option<&Peripheral> {
        let lower = name.to_ascii_lowercase();
        self.chip
            .peripherals
            .iter()
            .find(|p| p.name == lower || p.aliases.contains(&lower))
    }

    /// Names of the board pins (and chip pins) that have a function starting with `prefix`.
    pub fn pins_with(&self, prefix: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (alias, chip_pin) in &self.board.pins {
            if self
                .chip
                .pins
                .iter()
                .any(|p| &p.name == chip_pin && p.functions.iter().any(|f| f.starts_with(prefix)))
            {
                out.push(alias.clone());
            }
        }
        if out.is_empty() {
            for p in &self.chip.pins {
                if p.functions.iter().any(|f| f.starts_with(prefix)) {
                    out.push(p.name.clone());
                }
            }
        }
        out
    }
}
