//! The C backend (tasks B2 to B5): MIR to C, the runtime sources, and the build and flash
//! drivers for AVR, the bare-metal Arm ports (RP2040, RP2350, STM32F4, STM32U5, nRF52840,
//! SAMD21) and ESP-IDF.
//!
//! ```no_run
//! # fn demo(program: &neuron_mir::Program) {
//! let emitted = neuron_codegen_c::emit_c(program);
//! println!("{}", emitted.c);
//! # }
//! ```

mod emit;
mod runtime;
mod toolchain;
pub mod uf2;

pub use emit::{emit_c, emit_c_ui, Emitted};
pub use runtime::{runtime_files, ui_files, Port, RuntimeFile};
pub use toolchain::{
    avr_chip_name, build_arm, build_avr, build_esp_idf, build_host, esp_idf_project, find_tool,
    flash_avr, flash_bossac, flash_esptool, flash_lm4flash, flash_stm32, flash_uf2, ArmBuild,
    AvrBuild, BuildError, FlashOptions,
};
