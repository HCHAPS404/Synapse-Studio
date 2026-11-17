//! The `hdl` dialect to synthesizable Verilog and FPGA constraints (task B7).
//!
//! [`emit_verilog`] turns every `circuit` of the `hdl` modules of a package into a
//! Verilog-2005 module: registers with their reset values, `on rising(clk)` blocks as
//! clocked `always` blocks with non-blocking assignments, `comb:` as `always @*`, and
//! `sync ... via two_ff` as a two-flop synchronizer. A value that crosses clock domains
//! without `sync` is `NE0702`; a construct outside the subset is `NE0610`.
//! [`emit_constraints`] writes the pin file of an FPGA board (PCF for iCE40, XDC for
//! 7-series), and [`tools`] drives iverilog, yosys, nextpnr and openFPGALoader when they
//! are installed.
//!
//! ```
//! let board = neuron_codegen_hdl::FpgaBoard::builtin("artix7_35t_board").expect("embedded");
//! assert_eq!(board.family, neuron_codegen_hdl::Family::Xc7);
//! ```

mod board;
mod emit;
pub mod tools;

pub use board::{emit_constraints, Family, FpgaBoard};
pub use emit::{emit_verilog, Verilog};

/// A diagnostic of the hdl backend (same shape as the hardware checks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HdlDiag {
    /// Code (`NE0610`, `NE0702`, `NE0603`).
    pub code: &'static str,
    /// Message.
    pub message: String,
    /// Suggested fix.
    pub help: Option<String>,
    /// Source file.
    pub file: String,
    /// Byte range in that file.
    pub span: [usize; 2],
}
