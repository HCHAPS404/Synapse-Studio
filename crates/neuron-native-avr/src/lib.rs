//! Native AVR backend (task B8): MIR to ATmega328P machine code and Intel HEX, without an
//! external compiler.
//!
//! [`compile`] generates the vector table, the start-up code, a scheduler that follows the
//! rules of the C runtime, the activities of the program and the interrupt handlers, then
//! assembles them with the two-pass assembler of [`asm`]. The image runs on an Arduino UNO
//! R3 and in `neuron-sim-avr`. Constructs outside the native subset are `NE0610`; the C
//! backend covers more (see `docs/native-backend.md`).
//!
//! ```
//! let p = neuron_mir::Program { sysclk_hz: 16_000_000, board: Some("arduino_uno_r3".into()), ..Default::default() };
//! let n = neuron_native_avr::compile(&p).expect("an empty program compiles");
//! assert!(n.hex.ends_with(":00000001FF\n"));
//! ```

pub mod asm;
mod codegen;

use neuron_mir::Program;
use neuron_target::Diag;

/// A native firmware image.
#[derive(Debug, Clone)]
pub struct Native {
    /// Intel HEX, as `avrdude` and the simulator read it.
    pub hex: String,
    /// Program size in 16-bit words.
    pub words: usize,
    /// SRAM used by variables and the runtime, in bytes (the stack comes on top).
    pub ram_bytes: u16,
    /// Assembly listing with addresses and source lines.
    pub listing: String,
}

/// Compiles a program for the ATmega328P (Arduino UNO R3).
pub fn compile(p: &Program) -> Result<Native, Vec<Diag>> {
    if let Some(b) = &p.board {
        if b != "arduino_uno_r3" {
            return Err(vec![Diag {
                code: "NE0610",
                message: format!(
                    "the native backend supports the ATmega328P (arduino_uno_r3) only, not `{b}`"
                ),
                help: Some("use the C backend for this board".into()),
                file: p.files.first().cloned().unwrap_or_default(),
                span: [0, 0],
            }]);
        }
    }
    let mut g = codegen::Gen::new(p);
    g.program();
    if !g.diags.is_empty() {
        return Err(g.diags);
    }
    let file = p.files.first().cloned().unwrap_or_default();
    let assembled = asm::assemble(&g.ops).map_err(|e| {
        vec![Diag {
            code: "NE0610",
            message: format!("internal assembler error: {e}"),
            help: Some("please report it".into()),
            file: file.clone(),
            span: [0, 0],
        }]
    })?;
    if assembled.words.len() > 16 * 1024 {
        return Err(vec![Diag {
            code: "NE0604",
            message: format!(
                "the program needs {} bytes of flash; the ATmega328P has 32768",
                assembled.words.len() * 2
            ),
            help: None,
            file,
            span: [0, 0],
        }]);
    }
    let ram = g.ram_bytes();
    Ok(Native {
        hex: asm::intel_hex(&assembled.words),
        words: assembled.words.len(),
        ram_bytes: ram,
        listing: assembled.listing,
    })
}
