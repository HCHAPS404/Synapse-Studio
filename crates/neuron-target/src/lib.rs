//! Target descriptions and the hardware checks of Neuron programs (task B1).
//!
//! A *target* is a stack of four small TOML files (architecture, chip, board, runtime) kept
//! in `targets/`. [`Registry`] loads and validates them, [`check`] binds the `io` items of a
//! lowered package to pins and peripherals, and [`solve`] makes the clock tree and the
//! derived rates (baud, PWM, tick).
//!
//! ```
//! use neuron_target::Registry;
//! let reg = Registry::builtin().unwrap();
//! let uno = reg.resolve("arduino_uno_r3").unwrap();
//! assert_eq!(uno.chip.name, "atmega328p");
//! assert_eq!(uno.pin("D13").unwrap().name, "PB5");
//! ```

mod check;
mod model;
mod registry;
pub mod solve;

pub use check::{check, expr_text, flat, Diag, HwReport, IoBinding, IoKind, PwmCfg, UartCfg};
pub use model::{
    Arch, Board, Chip, ClockSource, ClockSpec, Peripheral, Pin, Pll, Programmer, Runtime, Target,
    Tier, Trust,
};
pub use registry::{Registry, TargetError};
