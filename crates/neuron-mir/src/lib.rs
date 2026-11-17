//! MIR: the typed, unit-free form that the code generators read (task B2).
//!
//! `lower` turns a checked package (NIR plus the hardware report of `neuron-target`) into a
//! [`Program`]. Constructs that the C backend does not cover yet are reported as `NE0610`,
//! naming the construct. `verify` checks the internal consistency of a program and
//! `Program::render` prints it as text, for the golden files.
//!
//! ```
//! use neuron_mir::{Expr, Ty};
//! assert_eq!(Expr::Int(3, Ty::U8).ty(), Ty::U8);
//! ```

mod lower;
mod mir;
mod print;
mod verify;

pub use lower::{lower, Lowered, Source};
pub use mir::*;
pub use verify::verify;
