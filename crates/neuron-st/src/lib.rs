//! A subset of IEC 61131-3 Structured Text and a virtual PLC (Phase C, task C2).
//!
//! [`parse`] reads programs, function blocks, functions and a `CONFIGURATION`, and also
//! the Siemens-style SCL spelling (`#local`, `"global"`, `BEGIN`). [`Plc`] scans a
//! program in simulated time: it latches the inputs, runs the body, publishes the outputs
//! and records every output change. The standard blocks TON, TOF, TP, CTU, CTD, CTUD,
//! R_TRIG, F_TRIG, SR and RS are built in, with two clock modes ([`Clock`]).
//!
//! The emitters use it to check their output by running it, the importers to read
//! existing code, and the digital twin to run a virtual PLC.
//!
//! ```
//! use neuron_st::{parse, Clock, Plc, Value};
//! let src = "PROGRAM blink\nVAR lamp AT %QX0.0 : BOOL; t : TON; END_VAR\n\
//!            t(IN := NOT t.Q, PT := T#20MS);\nIF t.Q THEN lamp := NOT lamp; END_IF;\nEND_PROGRAM\n";
//! let mut plc = Plc::new(parse(src).unwrap(), None, 10_000_000, Clock::ScanCounted).unwrap();
//! plc.run(100_000_000, &[]).unwrap();
//! assert!(plc.trace().len() > 2);
//! assert_eq!(plc.get("lamp"), Some(&Value::Bool(plc.trace().len() % 2 == 0)));
//! ```

#![forbid(unsafe_code)]

pub mod ast;
mod parser;
mod vm;

pub use parser::{parse, ParseError};
pub use vm::{Change, Clock, Plc, Value, VmError};
