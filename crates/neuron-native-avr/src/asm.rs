//! A two-pass AVR assembler for the instructions the code generator uses.
//!
//! Encodings follow the public AVR instruction set summary. Every instruction has a fixed
//! size (one word, or two for `LDS`, `STS`, `JMP` and `CALL`), so one pass assigns the
//! addresses and the second encodes. A branch whose target is out of range is an error,
//! never a silent wrap.

use std::collections::BTreeMap;

/// A status register flag tested by a conditional branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// Carry.
    C = 0,
    /// Zero.
    Z = 1,
    /// Negative.
    N = 2,
    /// Two's complement overflow.
    V = 3,
    /// Sign (N xor V).
    S = 4,
}

/// One instruction or a label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// A label at the current address.
    Label(String),
    /// A comment for the listing.
    Note(String),
    /// `LDI Rd, K` (r16..r31).
    Ldi(u8, u8),
    /// `MOV Rd, Rr`.
    Mov(u8, u8),
    /// `MOVW Rd, Rr` (even registers).
    Movw(u8, u8),
    /// `ADD Rd, Rr`.
    Add(u8, u8),
    /// `ADC Rd, Rr`.
    Adc(u8, u8),
    /// `SUB Rd, Rr`.
    Sub(u8, u8),
    /// `SBC Rd, Rr`.
    Sbc(u8, u8),
    /// `AND Rd, Rr`.
    And(u8, u8),
    /// `OR Rd, Rr`.
    Or(u8, u8),
    /// `EOR Rd, Rr`.
    Eor(u8, u8),
    /// `CP Rd, Rr`.
    Cp(u8, u8),
    /// `CPC Rd, Rr`.
    Cpc(u8, u8),
    /// `SUBI Rd, K`.
    Subi(u8, u8),
    /// `SBCI Rd, K`.
    Sbci(u8, u8),
    /// `CPI Rd, K`.
    Cpi(u8, u8),
    /// `COM Rd`.
    Com(u8),
    /// `NEG Rd`.
    Neg(u8),
    /// `INC Rd`.
    Inc(u8),
    /// `DEC Rd`.
    Dec(u8),
    /// `LSR Rd`.
    Lsr(u8),
    /// `ROR Rd`.
    Ror(u8),
    /// `ASR Rd`.
    Asr(u8),
    /// `PUSH Rr`.
    Push(u8),
    /// `POP Rd`.
    Pop(u8),
    /// `LDS Rd, k`.
    Lds(u8, u16),
    /// `STS k, Rr`.
    Sts(u16, u8),
    /// `IN Rd, A`.
    In(u8, u8),
    /// `OUT A, Rr`.
    Out(u8, u8),
    /// `SBI A, b`.
    Sbi(u8, u8),
    /// `CBI A, b`.
    Cbi(u8, u8),
    /// `SBIC A, b`.
    Sbic(u8, u8),
    /// `SBIS A, b`.
    Sbis(u8, u8),
    /// `BRBS` (flag set) or `BRBC` (flag clear) to a label.
    Br(Flag, bool, String),
    /// `RJMP` to a label.
    Rjmp(String),
    /// `JMP` to a label.
    Jmp(String),
    /// `CALL` to a label.
    Call(String),
    /// `RET`.
    Ret,
    /// `RETI`.
    Reti,
    /// `SEI`.
    Sei,
    /// `CLI`.
    Cli,
    /// `SLEEP`.
    Sleep,
}

impl Op {
    fn words(&self) -> u32 {
        match self {
            Op::Label(_) | Op::Note(_) => 0,
            Op::Lds(..) | Op::Sts(..) | Op::Jmp(_) | Op::Call(_) => 2,
            _ => 1,
        }
    }
}

/// Why assembly failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AsmError {
    /// A label is used but never defined.
    #[error("undefined label `{0}`")]
    Undefined(String),
    /// A label is defined twice.
    #[error("label `{0}` is defined twice")]
    Duplicate(String),
    /// A relative jump does not reach its target.
    #[error("`{0}` is out of range of the instruction at word {1:#x}")]
    Range(String, u32),
    /// An operand is not valid for the instruction.
    #[error("invalid operand in `{0}`")]
    Operand(String),
}

fn rr(op: u16, d: u8, r: u8) -> u16 {
    let (d, r) = (u16::from(d & 31), u16::from(r & 31));
    op | ((r & 0x10) << 5) | (d << 4) | (r & 0x0f)
}

fn imm(op: u16, d: u8, k: u8) -> Result<u16, AsmError> {
    if !(16..=31).contains(&d) {
        return Err(AsmError::Operand(format!("r{d} with an immediate")));
    }
    let k = u16::from(k);
    Ok(op | ((k & 0xf0) << 4) | (u16::from(d - 16) << 4) | (k & 0x0f))
}

fn one(op: u16, d: u8) -> u16 {
    op | (u16::from(d & 31) << 4)
}

fn io(op: u16, a: u8, d: u8) -> Result<u16, AsmError> {
    if a > 63 {
        return Err(AsmError::Operand(format!("I/O address {a:#x}")));
    }
    let a = u16::from(a);
    Ok(op | ((a & 0x30) << 5) | (u16::from(d & 31) << 4) | (a & 0x0f))
}

fn bitio(op: u16, a: u8, b: u8) -> Result<u16, AsmError> {
    if a > 31 || b > 7 {
        return Err(AsmError::Operand(format!("bit {b} of I/O {a:#x}")));
    }
    Ok(op | (u16::from(a) << 3) | u16::from(b))
}

/// Assembled code: words, and the listing with addresses.
#[derive(Debug, Clone, Default)]
pub struct Assembled {
    /// Program words from address 0.
    pub words: Vec<u16>,
    /// Readable listing.
    pub listing: String,
}

/// Assembles a program that starts at word 0.
pub fn assemble(ops: &[Op]) -> Result<Assembled, AsmError> {
    let mut labels: BTreeMap<&str, u32> = BTreeMap::new();
    let mut pc = 0u32;
    for op in ops {
        if let Op::Label(l) = op {
            if labels.insert(l.as_str(), pc).is_some() {
                return Err(AsmError::Duplicate(l.clone()));
            }
        }
        pc += op.words();
    }
    let addr = |l: &str| {
        labels
            .get(l)
            .copied()
            .ok_or_else(|| AsmError::Undefined(l.to_string()))
    };
    let mut out = Assembled::default();
    for op in ops {
        let at = out.words.len() as u32;
        let start = out.words.len();
        match op {
            Op::Label(l) => {
                out.listing.push_str(&format!("{l}:\n"));
                continue;
            }
            Op::Note(n) => {
                out.listing.push_str(&format!("        ; {n}\n"));
                continue;
            }
            Op::Ldi(d, k) => out.words.push(imm(0xe000, *d, *k)?),
            Op::Subi(d, k) => out.words.push(imm(0x5000, *d, *k)?),
            Op::Sbci(d, k) => out.words.push(imm(0x4000, *d, *k)?),
            Op::Cpi(d, k) => out.words.push(imm(0x3000, *d, *k)?),
            Op::Mov(d, r) => out.words.push(rr(0x2c00, *d, *r)),
            Op::Movw(d, r) => {
                if d % 2 != 0 || r % 2 != 0 {
                    return Err(AsmError::Operand("MOVW with an odd register".into()));
                }
                out.words
                    .push(0x0100 | (u16::from(d / 2) << 4) | u16::from(r / 2));
            }
            Op::Add(d, r) => out.words.push(rr(0x0c00, *d, *r)),
            Op::Adc(d, r) => out.words.push(rr(0x1c00, *d, *r)),
            Op::Sub(d, r) => out.words.push(rr(0x1800, *d, *r)),
            Op::Sbc(d, r) => out.words.push(rr(0x0800, *d, *r)),
            Op::And(d, r) => out.words.push(rr(0x2000, *d, *r)),
            Op::Or(d, r) => out.words.push(rr(0x2800, *d, *r)),
            Op::Eor(d, r) => out.words.push(rr(0x2400, *d, *r)),
            Op::Cp(d, r) => out.words.push(rr(0x1400, *d, *r)),
            Op::Cpc(d, r) => out.words.push(rr(0x0400, *d, *r)),
            Op::Com(d) => out.words.push(one(0x9400, *d)),
            Op::Neg(d) => out.words.push(one(0x9401, *d)),
            Op::Inc(d) => out.words.push(one(0x9403, *d)),
            Op::Dec(d) => out.words.push(one(0x940a, *d)),
            Op::Lsr(d) => out.words.push(one(0x9406, *d)),
            Op::Ror(d) => out.words.push(one(0x9407, *d)),
            Op::Asr(d) => out.words.push(one(0x9405, *d)),
            Op::Push(d) => out.words.push(one(0x920f, *d)),
            Op::Pop(d) => out.words.push(one(0x900f, *d)),
            Op::Lds(d, k) => {
                out.words.push(one(0x9000, *d));
                out.words.push(*k);
            }
            Op::Sts(k, r) => {
                out.words.push(one(0x9200, *r));
                out.words.push(*k);
            }
            Op::In(d, a) => out.words.push(io(0xb000, *a, *d)?),
            Op::Out(a, r) => out.words.push(io(0xb800, *a, *r)?),
            Op::Sbi(a, b) => out.words.push(bitio(0x9a00, *a, *b)?),
            Op::Cbi(a, b) => out.words.push(bitio(0x9800, *a, *b)?),
            Op::Sbic(a, b) => out.words.push(bitio(0x9900, *a, *b)?),
            Op::Sbis(a, b) => out.words.push(bitio(0x9b00, *a, *b)?),
            Op::Br(flag, set, l) => {
                let k = i64::from(addr(l)?) - i64::from(at) - 1;
                if !(-64..=63).contains(&k) {
                    return Err(AsmError::Range(l.clone(), at));
                }
                let base = if *set { 0xf000 } else { 0xf400 };
                out.words
                    .push(base | (((k as u16) & 0x7f) << 3) | (*flag as u16));
            }
            Op::Rjmp(l) => {
                let k = i64::from(addr(l)?) - i64::from(at) - 1;
                if !(-2048..=2047).contains(&k) {
                    return Err(AsmError::Range(l.clone(), at));
                }
                out.words.push(0xc000 | ((k as u16) & 0x0fff));
            }
            Op::Jmp(l) | Op::Call(l) => {
                let k = addr(l)?;
                if k > 0xffff {
                    return Err(AsmError::Range(l.clone(), at));
                }
                out.words.push(if matches!(op, Op::Jmp(_)) {
                    0x940c
                } else {
                    0x940e
                });
                out.words.push(k as u16);
            }
            Op::Ret => out.words.push(0x9508),
            Op::Reti => out.words.push(0x9518),
            Op::Sei => out.words.push(0x9478),
            Op::Cli => out.words.push(0x94f8),
            Op::Sleep => out.words.push(0x9588),
        }
        let text: Vec<String> = out.words[start..]
            .iter()
            .map(|w| format!("{w:04x}"))
            .collect();
        out.listing.push_str(&format!(
            "{:05x}  {:<10} {}\n",
            at * 2,
            text.join(" "),
            mnemonic(op)
        ));
    }
    Ok(out)
}

fn mnemonic(op: &Op) -> String {
    let s = format!("{op:?}");
    s.to_ascii_lowercase()
}

/// Intel HEX of a word image (16 data bytes per record, then the end record).
pub fn intel_hex(words: &[u16]) -> String {
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut out = String::new();
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let addr = (i * 16) as u16;
        let mut rec = vec![chunk.len() as u8, (addr >> 8) as u8, addr as u8, 0];
        rec.extend_from_slice(chunk);
        let sum = rec.iter().fold(0u8, |a, b| a.wrapping_add(*b));
        rec.push(sum.wrapping_neg());
        out.push(':');
        for b in rec {
            out.push_str(&format!("{b:02X}"));
        }
        out.push('\n');
    }
    out.push_str(":00000001FF\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(op: Op) -> Vec<u16> {
        assemble(&[op]).map(|a| a.words).unwrap_or_default()
    }

    #[test]
    fn encodings_match_the_instruction_set() {
        assert_eq!(enc(Op::Ldi(24, 0x12)), vec![0xe182]);
        assert_eq!(enc(Op::Add(24, 22)), vec![0x0f86]);
        assert_eq!(enc(Op::Adc(25, 23)), vec![0x1f97]);
        assert_eq!(enc(Op::Eor(1, 1)), vec![0x2411]);
        assert_eq!(enc(Op::Out(0x3f, 1)), vec![0xbe1f]);
        assert_eq!(enc(Op::In(0, 0x3f)), vec![0xb60f]);
        assert_eq!(enc(Op::Sbi(0x04, 5)), vec![0x9a25]);
        assert_eq!(enc(Op::Push(24)), vec![0x938f]);
        assert_eq!(enc(Op::Pop(24)), vec![0x918f]);
        assert_eq!(enc(Op::Lds(24, 0x0100)), vec![0x9180, 0x0100]);
        assert_eq!(enc(Op::Sts(0x0100, 24)), vec![0x9380, 0x0100]);
        assert_eq!(enc(Op::Ret), vec![0x9508]);
        assert_eq!(enc(Op::Movw(22, 24)), vec![0x01bc]);
    }

    #[test]
    fn branches_resolve_labels_and_check_their_range() {
        let ops = vec![
            Op::Label("top".into()),
            Op::Dec(24),
            Op::Br(Flag::Z, false, "top".into()),
        ];
        assert_eq!(
            assemble(&ops).map(|a| a.words).unwrap_or_default(),
            vec![0x958a, 0xf7f1]
        );
        let mut far = vec![Op::Br(Flag::Z, true, "end".into())];
        far.extend((0..100).map(|_| Op::Inc(24)));
        far.push(Op::Label("end".into()));
        assert!(matches!(assemble(&far), Err(AsmError::Range(..))));
        assert!(matches!(
            assemble(&[Op::Rjmp("nowhere".into())]),
            Err(AsmError::Undefined(_))
        ));
    }

    #[test]
    fn hex_records_have_checksums() {
        let h = intel_hex(&[0x940c, 0x0034]);
        // 04 + 0C + 94 + 34 = D8; the checksum is its two's complement, 28.
        assert_eq!(h, ":040000000C94340028\n:00000001FF\n");
    }
}
