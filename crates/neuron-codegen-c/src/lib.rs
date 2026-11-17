//! The C backend (tasks B2 to B4): MIR to C, the runtime sources, and the build and flash
//! drivers.
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

pub use emit::{emit_c, Emitted};
pub use runtime::{runtime_files, Port, RuntimeFile};
pub use toolchain::{
    avr_chip_name, build_avr, build_host, find_tool, flash_avr, AvrBuild, BuildError, FlashOptions,
};
