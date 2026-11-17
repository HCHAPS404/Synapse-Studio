//! A cycle-counted simulator of the ATmega328P (task B6).
//!
//! It executes the real firmware that `avr-gcc` builds from the generated C: the AVR
//! instruction set (without the FMUL family, SPM and the 3-byte-PC instructions), and the
//! peripherals that the Neuron runtime uses: GPIO ports B, C and D, external interrupts
//! INT0 and INT1, timer0 in CTC mode (the 1 ms tick), the ADC, USART0 transmit, the PWM
//! registers of timers 1 and 2, sleep and the watchdog.
//!
//! The harness drives the inputs (`schedule_pin`, `set_adc`), runs for a number of cycles and
//! reads back what the program did: the changes of the output pins with their cycle, the
//! bytes sent over the serial port and the PWM compare registers.
//!
//! ```
//! use neuron_sim_avr::Avr;
//! // ldi r16, 0x20 ; out PORTB(0x05), r16 ; rjmp .-2
//! let mut cpu = Avr::new(16_000_000);
//! cpu.load_words(&[0xE200, 0xB905, 0xCFFF]);
//! cpu.run_until(10).unwrap();
//! ```

#![forbid(unsafe_code)]

mod hex;

pub use hex::parse_hex;
use std::collections::BTreeMap;

/// Why the simulation stopped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SimError {
    /// The program uses an instruction that the simulator does not model.
    #[error("unimplemented instruction {opcode:#06x} at word address {pc:#06x}")]
    Unimplemented {
        /// Word address.
        pc: u32,
        /// The opcode.
        opcode: u16,
    },
    /// The program counter left the flash.
    #[error("the program counter {pc:#06x} is outside the flash")]
    OutOfRange {
        /// Word address.
        pc: u32,
    },
    /// The HEX file is not valid.
    #[error("invalid HEX file: {0}")]
    BadHex(String),
}

/// A change of an output pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinEvent {
    /// CPU cycle of the change.
    pub cycle: u64,
    /// Port letter (`B`, `C`, `D`).
    pub port: char,
    /// Bit in the port.
    pub bit: u8,
    /// New level.
    pub level: bool,
}

const C: u8 = 1 << 0;
const Z: u8 = 1 << 1;
const N: u8 = 1 << 2;
const V: u8 = 1 << 3;
const S: u8 = 1 << 4;
const H: u8 = 1 << 5;
const T: u8 = 1 << 6;
const I: u8 = 1 << 7;

const SPL: usize = 0x5D;
const SPH: usize = 0x5E;
const SREG: usize = 0x5F;
const DATA_SIZE: usize = 0x900;
const FLASH_WORDS: usize = 0x4000;

// I/O registers (data-space addresses) of the ATmega328P.
const PIN_BASE: usize = 0x23;
const TIFR0: usize = 0x35;
const EIFR: usize = 0x3C;
const EIMSK: usize = 0x3D;
const TCCR0A: usize = 0x44;
const TCCR0B: usize = 0x45;
const TCNT0: usize = 0x46;
const OCR0A: usize = 0x47;
const WDTCSR: usize = 0x60;
const EICRA: usize = 0x69;
const TIMSK0: usize = 0x6E;
const ADCL: usize = 0x78;
const ADCSRA: usize = 0x7A;
const ADMUX: usize = 0x7C;
const UCSR0A: usize = 0xC0;
const UCSR0B: usize = 0xC1;
const UDR0: usize = 0xC6;

/// The simulated chip.
#[derive(Debug, Clone)]
pub struct Avr {
    flash: Vec<u16>,
    data: Vec<u8>,
    pc: u32,
    cycles: u64,
    sleeping: bool,
    f_cpu: u64,
    input: [u8; 3],
    driven: [u8; 3],
    adc: [u16; 8],
    t0_acc: u64,
    wdt_timeout: Option<u64>,
    last_wdr: u64,
    watchdog_reset: bool,
    events: BTreeMap<u64, Vec<(char, u8, bool)>>,
    pin_events: Vec<PinEvent>,
    uart_tx: Vec<(u64, u8)>,
}

fn port_letter(i: usize) -> char {
    ['B', 'C', 'D'][i]
}

impl Avr {
    /// A chip in its reset state, clocked at `f_cpu` hertz.
    pub fn new(f_cpu: u64) -> Avr {
        Avr {
            flash: vec![0xFFFF; FLASH_WORDS],
            data: vec![0; DATA_SIZE],
            pc: 0,
            cycles: 0,
            sleeping: false,
            f_cpu,
            input: [0; 3],
            driven: [0; 3],
            adc: [0; 8],
            t0_acc: 0,
            wdt_timeout: None,
            last_wdr: 0,
            watchdog_reset: false,
            events: BTreeMap::new(),
            pin_events: Vec::new(),
            uart_tx: Vec::new(),
        }
    }

    /// Loads an Intel HEX image.
    pub fn load_hex(&mut self, text: &str) -> Result<(), SimError> {
        let bytes = parse_hex(text)?;
        self.load_image(&bytes);
        Ok(())
    }

    /// Loads a flat byte image at address 0.
    pub fn load_image(&mut self, bytes: &[u8]) {
        for (i, pair) in bytes.chunks(2).enumerate() {
            if i < self.flash.len() {
                let lo = u16::from(pair[0]);
                let hi = u16::from(*pair.get(1).unwrap_or(&0xFF));
                self.flash[i] = lo | (hi << 8);
            }
        }
    }

    /// Loads instruction words at address 0.
    pub fn load_words(&mut self, words: &[u16]) {
        for (i, w) in words.iter().enumerate() {
            if i < self.flash.len() {
                self.flash[i] = *w;
            }
        }
    }

    /// Schedules a change of an input pin at a CPU cycle.
    pub fn schedule_pin(&mut self, cycle: u64, port: char, bit: u8, level: bool) {
        self.events
            .entry(cycle)
            .or_default()
            .push((port, bit, level));
    }

    /// Sets the value that an ADC conversion of `channel` returns (0 to 1023).
    pub fn set_adc(&mut self, channel: u8, value: u16) {
        if let Some(c) = self.adc.get_mut(usize::from(channel)) {
            *c = value.min(1023);
        }
    }

    /// The CPU cycles elapsed.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Converts a cycle count to milliseconds.
    pub fn ms(&self, cycles: u64) -> f64 {
        cycles as f64 * 1000.0 / self.f_cpu as f64
    }

    /// The cycle at which `ms` milliseconds have passed.
    pub fn cycle_of_ms(&self, ms: f64) -> u64 {
        (ms * self.f_cpu as f64 / 1000.0).round() as u64
    }

    /// Changes of the output pins so far.
    pub fn pin_events(&self) -> &[PinEvent] {
        &self.pin_events
    }

    /// Bytes written to USART0, with the cycle of each.
    pub fn uart_output(&self) -> &[(u64, u8)] {
        &self.uart_tx
    }

    /// The compare register of a PWM channel (`timer` 1 or 2, `chan` `a` or `b`).
    pub fn pwm_compare(&self, timer: u8, chan: char) -> Option<u8> {
        let addr = match (timer, chan) {
            (1, 'a') => 0x88,
            (1, 'b') => 0x8A,
            (2, 'a') => 0xB3,
            (2, 'b') => 0xB4,
            _ => return None,
        };
        Some(self.data[addr])
    }

    /// `true` when the watchdog reset the chip (the run stopped there).
    pub fn watchdog_reset(&self) -> bool {
        self.watchdog_reset
    }

    /// A byte of the data space (for tests).
    pub fn peek(&self, addr: u16) -> u8 {
        self.data.get(usize::from(addr)).copied().unwrap_or(0)
    }

    /// The program counter, as a word address.
    pub fn pc(&self) -> u32 {
        self.pc
    }

    // ---- data space -------------------------------------------------------------------------

    fn rd(&self, addr: usize) -> u8 {
        match addr {
            a if (PIN_BASE..PIN_BASE + 9).contains(&a) && (a - PIN_BASE) % 3 == 0 => {
                let i = (a - PIN_BASE) / 3;
                let ddr = self.data[a + 1];
                let port = self.data[a + 2];
                (self.input[i] & !ddr) | (port & ddr)
            }
            UCSR0A => self.data[UCSR0A] | 0x20, // the data register is always empty
            a if a < self.data.len() => self.data[a],
            _ => 0,
        }
    }

    fn drive_changed(&mut self, i: usize) {
        let ddr = self.data[PIN_BASE + 1 + 3 * i];
        let port = self.data[PIN_BASE + 2 + 3 * i];
        let now = port & ddr;
        let was = self.driven[i];
        let diff = now ^ was;
        self.driven[i] = now;
        for bit in 0..8u8 {
            if diff & (1 << bit) != 0 {
                self.pin_events.push(PinEvent {
                    cycle: self.cycles,
                    port: port_letter(i),
                    bit,
                    level: now & (1 << bit) != 0,
                });
            }
        }
    }

    fn wr(&mut self, addr: usize, v: u8) {
        if addr >= self.data.len() {
            return;
        }
        if (PIN_BASE..PIN_BASE + 9).contains(&addr) {
            let i = (addr - PIN_BASE) / 3;
            match (addr - PIN_BASE) % 3 {
                0 => {
                    // Writing 1 to PINx toggles PORTx.
                    self.data[addr + 2] ^= v;
                    self.drive_changed(i);
                }
                1 | 2 => {
                    self.data[addr] = v;
                    self.drive_changed(i);
                }
                _ => {}
            }
            return;
        }
        match addr {
            // Flags are cleared by writing a one.
            TIFR0 | EIFR | 0x36 | 0x37 => self.data[addr] &= !v,
            TCNT0 => self.data[addr] = v,
            WDTCSR => {
                self.data[addr] = v;
                if v & 0x08 != 0 {
                    let idx = usize::from(((v >> 2) & 8) | (v & 7));
                    let ms = [16u64, 32, 64, 125, 250, 500, 1000, 2000, 4000, 8000][idx.min(9)];
                    self.wdt_timeout = Some(ms * self.f_cpu / 1000);
                    self.last_wdr = self.cycles;
                } else {
                    self.wdt_timeout = None;
                }
            }
            ADCSRA => {
                let mut stored = v;
                if v & 0x80 != 0 && v & 0x40 != 0 {
                    let ch = usize::from(self.data[ADMUX] & 0x0F);
                    let value = self.adc.get(ch).copied().unwrap_or(0);
                    self.data[ADCL] = (value & 0xFF) as u8;
                    self.data[ADCL + 1] = (value >> 8) as u8;
                    stored = (v & !0x40) | 0x10; // conversion done, ADIF set
                }
                self.data[addr] = stored;
            }
            UDR0 => self.uart_tx.push((self.cycles, v)),
            _ => self.data[addr] = v,
        }
    }

    fn sp(&self) -> u16 {
        u16::from(self.data[SPL]) | (u16::from(self.data[SPH]) << 8)
    }

    fn set_sp(&mut self, v: u16) {
        self.data[SPL] = (v & 0xFF) as u8;
        self.data[SPH] = (v >> 8) as u8;
    }

    fn push(&mut self, v: u8) {
        let sp = self.sp();
        self.wr(usize::from(sp), v);
        self.set_sp(sp.wrapping_sub(1));
    }

    fn pop(&mut self) -> u8 {
        let sp = self.sp().wrapping_add(1);
        self.set_sp(sp);
        self.rd(usize::from(sp))
    }

    fn push_pc(&mut self) {
        let pc = self.pc;
        self.push((pc & 0xFF) as u8);
        self.push(((pc >> 8) & 0xFF) as u8);
    }

    fn pop_pc(&mut self) {
        let hi = u32::from(self.pop());
        let lo = u32::from(self.pop());
        self.pc = (hi << 8) | lo;
    }

    fn sreg(&self) -> u8 {
        self.data[SREG]
    }

    fn flag(&self, f: u8) -> bool {
        self.data[SREG] & f != 0
    }

    fn set_flag(&mut self, f: u8, on: bool) {
        if on {
            self.data[SREG] |= f;
        } else {
            self.data[SREG] &= !f;
        }
    }

    fn pair(&self, lo: usize) -> u16 {
        u16::from(self.data[lo]) | (u16::from(self.data[lo + 1]) << 8)
    }

    fn set_pair(&mut self, lo: usize, v: u16) {
        self.data[lo] = (v & 0xFF) as u8;
        self.data[lo + 1] = (v >> 8) as u8;
    }

    // ---- flags --------------------------------------------------------------------------------

    fn nzs(&mut self, r: u8, v: bool) {
        self.set_flag(N, r & 0x80 != 0);
        self.set_flag(Z, r == 0);
        self.set_flag(V, v);
        let s = self.flag(N) != self.flag(V);
        self.set_flag(S, s);
    }

    fn add_flags(&mut self, d: u8, r: u8, res: u8) {
        let (d3, r3, s3) = (d & 8 != 0, r & 8 != 0, res & 8 != 0);
        let (d7, r7, s7) = (d & 0x80 != 0, r & 0x80 != 0, res & 0x80 != 0);
        self.set_flag(H, (d3 && r3) || (r3 && !s3) || (!s3 && d3));
        self.set_flag(C, (d7 && r7) || (r7 && !s7) || (!s7 && d7));
        let v = (d7 && r7 && !s7) || (!d7 && !r7 && s7);
        self.nzs(res, v);
    }

    /// `keep_z`: the zero flag is only cleared, never set (SBC, SBCI, CPC).
    fn sub_flags(&mut self, d: u8, r: u8, res: u8, keep_z: bool) {
        let old_z = self.flag(Z);
        let (d3, r3, s3) = (d & 8 != 0, r & 8 != 0, res & 8 != 0);
        let (d7, r7, s7) = (d & 0x80 != 0, r & 0x80 != 0, res & 0x80 != 0);
        self.set_flag(H, (!d3 && r3) || (r3 && s3) || (s3 && !d3));
        self.set_flag(C, (!d7 && r7) || (r7 && s7) || (s7 && !d7));
        let v = (d7 && !r7 && !s7) || (!d7 && r7 && s7);
        self.nzs(res, v);
        if keep_z {
            self.set_flag(Z, res == 0 && old_z);
        }
    }

    fn logic_flags(&mut self, res: u8) {
        self.nzs(res, false);
    }

    // ---- execution -----------------------------------------------------------------------------

    fn is_two_word(op: u16) -> bool {
        matches!(op & 0xFE0F, 0x9000 | 0x9200) || matches!(op & 0xFE0E, 0x940C | 0x940E)
    }

    /// Skips the next instruction; returns the extra cycles.
    fn skip_next(&mut self) -> u32 {
        let next = self.flash.get(self.pc as usize).copied().unwrap_or(0xFFFF);
        if Self::is_two_word(next) {
            self.pc += 2;
            2
        } else {
            self.pc += 1;
            1
        }
    }

    fn branch(&mut self, op: u16, set: bool) -> u32 {
        let bit = (op & 7) as u8;
        let taken = (self.sreg() >> bit) & 1 == u8::from(set);
        if taken {
            let k = ((op >> 3) & 0x7F) as i16;
            let k = if k & 0x40 != 0 { k - 128 } else { k };
            self.pc = (self.pc as i64 + i64::from(k)) as u32;
            2
        } else {
            1
        }
    }

    fn ptr_access(&mut self, op: u16, store: bool) -> u32 {
        let d = usize::from((op >> 4) & 0x1F);
        let (base, mode) = match op & 0x000F {
            0x1 => (30, 1), // Z+
            0x2 => (30, 2), // -Z
            0x9 => (28, 1), // Y+
            0xA => (28, 2), // -Y
            0xC => (26, 0), // X
            0xD => (26, 1), // X+
            _ => (26, 2),   // -X
        };
        let mut p = self.pair(base);
        if mode == 2 {
            p = p.wrapping_sub(1);
            self.set_pair(base, p);
        }
        if store {
            let v = self.data[d];
            self.wr(usize::from(p), v);
        } else {
            self.data[d] = self.rd(usize::from(p));
        }
        if mode == 1 {
            self.set_pair(base, p.wrapping_add(1));
        }
        2
    }

    fn exec(&mut self) -> Result<u32, SimError> {
        let pc0 = self.pc;
        let op = *self
            .flash
            .get(pc0 as usize)
            .ok_or(SimError::OutOfRange { pc: pc0 })?;
        self.pc += 1;
        let d = usize::from((op >> 4) & 0x1F);
        let r = usize::from(((op >> 5) & 0x10) | (op & 0x0F));
        let kk = (((op >> 4) & 0xF0) | (op & 0x0F)) as u8;
        let dh = 16 + usize::from((op >> 4) & 0x0F);
        let unimpl = |pc: u32| SimError::Unimplemented { pc, opcode: op };

        if op == 0 {
            return Ok(1);
        }
        match op & 0xFF00 {
            0x0100 => {
                let (dd, rr) = (usize::from((op >> 4) & 0xF) * 2, usize::from(op & 0xF) * 2);
                self.data[dd] = self.data[rr];
                self.data[dd + 1] = self.data[rr + 1];
                return Ok(1);
            }
            0x0200 => {
                let (a, b) = (
                    usize::from((op >> 4) & 0xF) + 16,
                    usize::from(op & 0xF) + 16,
                );
                let p = i16::from(self.data[a] as i8) * i16::from(self.data[b] as i8);
                self.mul_result(p as u16);
                return Ok(2);
            }
            0x0300 => {
                if op & 0x0088 == 0 {
                    let (a, b) = (usize::from((op >> 4) & 7) + 16, usize::from(op & 7) + 16);
                    let p = i16::from(self.data[a] as i8) * i16::from(self.data[b]);
                    self.mul_result(p as u16);
                    return Ok(2);
                }
                return Err(unimpl(pc0));
            }
            _ => {}
        }
        match op & 0xFC00 {
            0x0400 | 0x1400 | 0x1800 | 0x0800 => {
                let (a, b) = (self.data[d], self.data[r]);
                let sbc = op & 0xFC00 == 0x0800 || op & 0xFC00 == 0x0400;
                let carry = u8::from(sbc && self.flag(C));
                let res = a.wrapping_sub(b).wrapping_sub(carry);
                // The flags use the original operand and the result, carry-in included.
                self.sub_flags(a, b, res, sbc);
                if op & 0xFC00 == 0x1800 || op & 0xFC00 == 0x0800 {
                    self.data[d] = res;
                }
                return Ok(1);
            }
            0x0C00 | 0x1C00 => {
                let (a, b) = (self.data[d], self.data[r]);
                let carry = u8::from(op & 0xFC00 == 0x1C00 && self.flag(C));
                let res = a.wrapping_add(b).wrapping_add(carry);
                self.add_flags(a, b, res);
                self.data[d] = res;
                return Ok(1);
            }
            0x1000 => {
                // CPSE
                if self.data[d] == self.data[r] {
                    return Ok(1 + self.skip_next());
                }
                return Ok(1);
            }
            0x2000 => {
                let res = self.data[d] & self.data[r];
                self.data[d] = res;
                self.logic_flags(res);
                return Ok(1);
            }
            0x2400 => {
                let res = self.data[d] ^ self.data[r];
                self.data[d] = res;
                self.logic_flags(res);
                return Ok(1);
            }
            0x2800 => {
                let res = self.data[d] | self.data[r];
                self.data[d] = res;
                self.logic_flags(res);
                return Ok(1);
            }
            0x2C00 => {
                self.data[d] = self.data[r];
                return Ok(1);
            }
            0x9C00 => {
                let p = u16::from(self.data[d]) * u16::from(self.data[r]);
                self.mul_result(p);
                return Ok(2);
            }
            _ => {}
        }
        match op & 0xF000 {
            0x3000 | 0x5000 | 0x4000 => {
                let a = self.data[dh];
                let sbc = op & 0xF000 == 0x4000;
                let carry = u8::from(sbc && self.flag(C));
                let res = a.wrapping_sub(kk).wrapping_sub(carry);
                self.sub_flags(a, kk, res, sbc);
                if op & 0xF000 != 0x3000 {
                    self.data[dh] = res;
                }
                return Ok(1);
            }
            0x6000 => {
                let res = self.data[dh] | kk;
                self.data[dh] = res;
                self.logic_flags(res);
                return Ok(1);
            }
            0x7000 => {
                let res = self.data[dh] & kk;
                self.data[dh] = res;
                self.logic_flags(res);
                return Ok(1);
            }
            0xC000 | 0xD000 => {
                let k = (op & 0x0FFF) as i32;
                let k = if k & 0x800 != 0 { k - 0x1000 } else { k };
                let call = op & 0xF000 == 0xD000;
                if call {
                    self.push_pc();
                }
                self.pc = (self.pc as i64 + i64::from(k)) as u32;
                return Ok(if call { 3 } else { 2 });
            }
            0xE000 => {
                self.data[dh] = kk;
                return Ok(1);
            }
            _ => {}
        }
        // LDD/STD with Y or Z and a displacement.
        if op & 0xD000 == 0x8000 {
            let q = u16::from(((op >> 8) & 0x20) as u8 | ((op >> 7) & 0x18) as u8 | (op & 7) as u8);
            let base = if op & 0x0008 != 0 { 28 } else { 30 };
            let addr = usize::from(self.pair(base).wrapping_add(q));
            if op & 0x0200 != 0 {
                let v = self.data[d];
                self.wr(addr, v);
            } else {
                self.data[d] = self.rd(addr);
            }
            return Ok(2);
        }
        match op & 0xFE0F {
            0x9000 | 0x9200 => {
                let addr = usize::from(self.flash.get(self.pc as usize).copied().unwrap_or(0));
                self.pc += 1;
                if op & 0x0200 != 0 {
                    let v = self.data[d];
                    self.wr(addr, v);
                } else {
                    self.data[d] = self.rd(addr);
                }
                return Ok(2);
            }
            0x9001 | 0x9002 | 0x9009 | 0x900A | 0x900C | 0x900D | 0x900E => {
                return Ok(self.ptr_access(op, false))
            }
            0x9201 | 0x9202 | 0x9209 | 0x920A | 0x920C | 0x920D | 0x920E => {
                return Ok(self.ptr_access(op, true))
            }
            0x9004 | 0x9005 => {
                let z = self.pair(30);
                let word = self
                    .flash
                    .get(usize::from(z >> 1))
                    .copied()
                    .unwrap_or(0xFFFF);
                self.data[d] = if z & 1 == 0 {
                    (word & 0xFF) as u8
                } else {
                    (word >> 8) as u8
                };
                if op & 1 == 1 {
                    self.set_pair(30, z.wrapping_add(1));
                }
                return Ok(3);
            }
            0x900F => {
                self.data[d] = self.pop();
                return Ok(2);
            }
            0x920F => {
                let v = self.data[d];
                self.push(v);
                return Ok(2);
            }
            0x9400 => {
                let res = !self.data[d];
                self.data[d] = res;
                self.nzs(res, false);
                self.set_flag(C, true);
                return Ok(1);
            }
            0x9401 => {
                let a = self.data[d];
                let res = 0u8.wrapping_sub(a);
                self.data[d] = res;
                self.nzs(res, res == 0x80);
                self.set_flag(H, (res & 8 != 0) || (a & 8 != 0));
                self.set_flag(C, res != 0);
                return Ok(1);
            }
            0x9402 => {
                self.data[d] = self.data[d].rotate_right(4);
                return Ok(1);
            }
            0x9403 => {
                let res = self.data[d].wrapping_add(1);
                self.data[d] = res;
                self.nzs(res, res == 0x80);
                return Ok(1);
            }
            0x940A => {
                let res = self.data[d].wrapping_sub(1);
                self.data[d] = res;
                self.nzs(res, res == 0x7F);
                return Ok(1);
            }
            0x9405..=0x9407 => {
                let a = self.data[d];
                let c_in = self.flag(C);
                let res = match op & 0xF {
                    5 => (a >> 1) | (a & 0x80),
                    6 => a >> 1,
                    _ => (a >> 1) | (u8::from(c_in) << 7),
                };
                self.data[d] = res;
                let c = a & 1 != 0;
                let n = res & 0x80 != 0;
                self.set_flag(C, c);
                self.nzs(res, n != c);
                return Ok(1);
            }
            _ => {}
        }
        match op {
            0x9409 => {
                self.pc = u32::from(self.pair(30));
                return Ok(2);
            }
            0x9509 => {
                self.push_pc();
                self.pc = u32::from(self.pair(30));
                return Ok(3);
            }
            0x9508 => {
                self.pop_pc();
                return Ok(4);
            }
            0x9518 => {
                self.pop_pc();
                self.set_flag(I, true);
                return Ok(4);
            }
            0x9588 => {
                self.sleeping = true;
                return Ok(1);
            }
            0x9598 => return Ok(1),
            0x95A8 => {
                self.last_wdr = self.cycles;
                return Ok(1);
            }
            0x95C8 => {
                let z = self.pair(30);
                let word = self
                    .flash
                    .get(usize::from(z >> 1))
                    .copied()
                    .unwrap_or(0xFFFF);
                self.data[0] = if z & 1 == 0 {
                    (word & 0xFF) as u8
                } else {
                    (word >> 8) as u8
                };
                return Ok(3);
            }
            _ => {}
        }
        match op & 0xFF8F {
            0x9408 => {
                self.data[SREG] |= 1 << ((op >> 4) & 7);
                return Ok(1);
            }
            0x9488 => {
                self.data[SREG] &= !(1 << ((op >> 4) & 7));
                return Ok(1);
            }
            _ => {}
        }
        if op & 0xFE0E == 0x940C || op & 0xFE0E == 0x940E {
            let hi = u32::from(((op >> 3) & 0x3E) | (op & 1));
            let lo = u32::from(self.flash.get(self.pc as usize).copied().unwrap_or(0));
            self.pc += 1;
            let target = (hi << 16) | lo;
            if op & 0x0002 != 0 {
                self.push_pc();
                self.pc = target;
                return Ok(4);
            }
            self.pc = target;
            return Ok(3);
        }
        match op & 0xFF00 {
            0x9600 | 0x9700 => {
                let lo = 24 + 2 * usize::from((op >> 4) & 3);
                let k = ((op >> 2) & 0x30) | (op & 0x0F);
                let a = self.pair(lo);
                let res = if op & 0x0100 == 0 {
                    a.wrapping_add(k)
                } else {
                    a.wrapping_sub(k)
                };
                self.set_pair(lo, res);
                let (r15, d15) = (res & 0x8000 != 0, a & 0x8000 != 0);
                if op & 0x0100 == 0 {
                    self.set_flag(V, !d15 && r15);
                    self.set_flag(C, !r15 && d15);
                } else {
                    self.set_flag(V, d15 && !r15);
                    self.set_flag(C, r15 && !d15);
                }
                self.set_flag(N, r15);
                self.set_flag(Z, res == 0);
                let s = self.flag(N) != self.flag(V);
                self.set_flag(S, s);
                return Ok(2);
            }
            0x9800 | 0x9A00 => {
                let a = usize::from((op >> 3) & 0x1F) + 0x20;
                let bit = (op & 7) as u8;
                let v = self.rd(a);
                let nv = if op & 0x0200 != 0 {
                    v | (1 << bit)
                } else {
                    v & !(1 << bit)
                };
                self.wr(a, nv);
                return Ok(2);
            }
            0x9900 | 0x9B00 => {
                let a = usize::from((op >> 3) & 0x1F) + 0x20;
                let bit = (op & 7) as u8;
                let set = self.rd(a) & (1 << bit) != 0;
                let want = op & 0x0200 != 0;
                if set == want {
                    return Ok(1 + self.skip_next());
                }
                return Ok(1);
            }
            _ => {}
        }
        match op & 0xF800 {
            0xB000 => {
                let a = usize::from(((op >> 5) & 0x30) | (op & 0x0F)) + 0x20;
                self.data[d] = self.rd(a);
                return Ok(1);
            }
            0xB800 => {
                let a = usize::from(((op >> 5) & 0x30) | (op & 0x0F)) + 0x20;
                let v = self.data[d];
                self.wr(a, v);
                return Ok(1);
            }
            _ => {}
        }
        match op & 0xFC00 {
            0xF000 => return Ok(self.branch(op, true)),
            0xF400 => return Ok(self.branch(op, false)),
            _ => {}
        }
        match op & 0xFE08 {
            0xF800 => {
                let bit = (op & 7) as u8;
                let t = self.flag(T);
                self.data[d] = (self.data[d] & !(1u8 << bit)) | (u8::from(t) << bit);
                return Ok(1);
            }
            0xFA00 => {
                let on = self.data[d] & (1 << (op & 7)) != 0;
                self.set_flag(T, on);
                return Ok(1);
            }
            0xFC00 | 0xFE00 => {
                let set = self.data[d] & (1 << (op & 7)) != 0;
                let want = op & 0x0200 != 0;
                if set == want {
                    return Ok(1 + self.skip_next());
                }
                return Ok(1);
            }
            _ => {}
        }
        Err(unimpl(pc0))
    }

    fn mul_result(&mut self, p: u16) {
        self.set_pair(0, p);
        self.set_flag(C, p & 0x8000 != 0);
        self.set_flag(Z, p == 0);
    }

    // ---- peripherals -----------------------------------------------------------------------------

    fn t0_prescaler(&self) -> u64 {
        match self.data[TCCR0B] & 7 {
            1 => 1,
            2 => 8,
            3 => 64,
            4 => 256,
            5 => 1024,
            _ => 0,
        }
    }

    fn t0_ctc(&self) -> bool {
        self.data[TCCR0A] & 3 == 2 && self.data[TCCR0B] & 8 == 0
    }

    fn t0_advance(&mut self, n: u64) {
        let p = self.t0_prescaler();
        if p == 0 {
            return;
        }
        self.t0_acc += n;
        let ticks = self.t0_acc / p;
        self.t0_acc %= p;
        if ticks == 0 {
            return;
        }
        let cnt = u64::from(self.data[TCNT0]);
        if self.t0_ctc() {
            let period = u64::from(self.data[OCR0A]) + 1;
            let visits = (cnt + ticks + 1) / period - (cnt + 1) / period;
            if visits > 0 {
                self.data[TIFR0] |= 0x02; // OCF0A
            }
            self.data[TCNT0] = ((cnt + ticks) % period) as u8;
        } else {
            if cnt + ticks > 255 {
                self.data[TIFR0] |= 0x01; // TOV0
            }
            self.data[TCNT0] = ((cnt + ticks) % 256) as u8;
        }
    }

    /// Cycles until the next compare match of timer0, when it is running in CTC mode.
    fn t0_cycles_to_match(&self) -> Option<u64> {
        let p = self.t0_prescaler();
        if p == 0 || !self.t0_ctc() {
            return None;
        }
        let top = u64::from(self.data[OCR0A]);
        let cnt = u64::from(self.data[TCNT0]);
        let ticks = if cnt < top { top - cnt } else { top + 1 };
        Some((ticks * p).saturating_sub(self.t0_acc).max(1))
    }

    fn advance(&mut self, n: u64) {
        self.cycles += n;
        self.t0_advance(n);
        if let Some(t) = self.wdt_timeout {
            if self.cycles.saturating_sub(self.last_wdr) > t {
                self.watchdog_reset = true;
            }
        }
    }

    fn apply_pin(&mut self, port: char, bit: u8, level: bool) {
        let Some(i) = ['B', 'C', 'D'].iter().position(|p| *p == port) else {
            return;
        };
        let old = self.input[i] & (1 << bit) != 0;
        if level {
            self.input[i] |= 1 << bit;
        } else {
            self.input[i] &= !(1 << bit);
        }
        if old == level || i != 2 || !(bit == 2 || bit == 3) {
            return;
        }
        // INT0 is PD2 and INT1 is PD3; the sense bits are in EICRA.
        let n = bit - 2;
        let isc = (self.data[EICRA] >> (2 * n)) & 3;
        let fire = match isc {
            1 => true,
            2 => old && !level,
            3 => !old && level,
            _ => false,
        };
        if fire {
            self.data[EIFR] |= 1 << n;
        }
    }

    fn apply_due_events(&mut self) {
        let due: Vec<u64> = self.events.range(..=self.cycles).map(|(k, _)| *k).collect();
        for k in due {
            if let Some(list) = self.events.remove(&k) {
                for (port, bit, level) in list {
                    self.apply_pin(port, bit, level);
                }
            }
        }
    }

    /// The vector number of the highest-priority pending interrupt that is enabled.
    fn pending_vector(&self) -> Option<u32> {
        if !self.flag(I) {
            return None;
        }
        let eifr = self.data[EIFR];
        let eimsk = self.data[EIMSK];
        if eifr & eimsk & 1 != 0 {
            return Some(1);
        }
        if eifr & eimsk & 2 != 0 {
            return Some(2);
        }
        if self.data[TIFR0] & self.data[TIMSK0] & 0x02 != 0 {
            return Some(14);
        }
        if self.data[TIFR0] & self.data[TIMSK0] & 0x01 != 0 {
            return Some(16);
        }
        if self.data[UCSR0B] & 0x20 != 0 {
            return Some(19);
        }
        None
    }

    fn service(&mut self, vector: u32) {
        match vector {
            1 => self.data[EIFR] &= !1,
            2 => self.data[EIFR] &= !2,
            14 => self.data[TIFR0] &= !0x02,
            16 => self.data[TIFR0] &= !0x01,
            _ => {}
        }
        self.set_flag(I, false);
        self.push_pc();
        self.pc = vector * 2;
        self.sleeping = false;
        self.advance(4);
    }

    /// Runs until the cycle counter reaches `target`, a fault stops it, or the watchdog resets
    /// the chip.
    pub fn run_until(&mut self, target: u64) -> Result<(), SimError> {
        while self.cycles < target && !self.watchdog_reset {
            self.apply_due_events();
            if let Some(v) = self.pending_vector() {
                self.service(v);
                continue;
            }
            if self.sleeping {
                let mut jump = target - self.cycles;
                if let Some(next) = self.events.keys().next() {
                    jump = jump.min(next.saturating_sub(self.cycles).max(1));
                }
                if let Some(t) = self.t0_cycles_to_match() {
                    jump = jump.min(t);
                }
                self.advance(jump.max(1));
                continue;
            }
            let cyc = self.exec()?;
            self.advance(u64::from(cyc));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(words: &[u16], cycles: u64) -> Avr {
        let mut cpu = Avr::new(16_000_000);
        cpu.load_words(words);
        assert!(cpu.run_until(cycles).is_ok(), "run_until failed");
        cpu
    }

    #[test]
    fn ldi_add_and_flags() {
        // ldi r16,200 ; ldi r17,100 ; add r16,r17 ; (r16 = 44, C set)
        let cpu = run(&[0xEC08, 0xE614, 0x0F01, 0xCFFF], 6);
        assert_eq!(cpu.peek(16), 44);
        assert_ne!(cpu.peek(0x5F) & C, 0);
    }

    #[test]
    fn out_to_port_traces_a_pin() {
        // sbi DDRB,5 ; sbi PORTB,5 ; cbi PORTB,5 ; rjmp .-2
        let cpu = run(&[0x9A25, 0x9A2D, 0x982D, 0xCFFF], 12);
        let levels: Vec<bool> = cpu.pin_events().iter().map(|e| e.level).collect();
        assert_eq!(levels, [true, false]);
        assert_eq!(cpu.pin_events()[0].port, 'B');
        assert_eq!(cpu.pin_events()[0].bit, 5);
    }

    #[test]
    fn a_loop_with_dec_and_brne_counts_cycles() {
        // ldi r16,10 ; loop: dec r16 ; brne loop ; rjmp .-2
        let cpu = run(&[0xE00A, 0x950A, 0xF7F1, 0xCFFF], 200);
        assert_eq!(cpu.peek(16), 0);
    }

    #[test]
    fn call_and_ret_use_the_stack() {
        // ldi r16,0x08 ; out SPH,r16 ; ldi r16,0xFF ; out SPL,r16 ; rcall f ; ldi r17,7 ; rjmp . ; f: ldi r18,9 ; ret
        let prog = [
            0xE008, 0xBF0E, 0xEF0F, 0xBF0D, 0xD003, 0xE017, 0xCFFF, 0x0000, 0xE029, 0x9508,
        ];
        let cpu = run(&prog, 60);
        assert_eq!(cpu.peek(18), 9);
        assert_eq!(cpu.peek(17), 7);
    }

    #[test]
    fn the_timer_raises_its_flag_every_period() {
        // ldi r16,2 ; out TCCR0A(0x24),r16 ; ldi r16,1 ; out TCCR0B(0x25),r16 ; ldi r16,99 ; out OCR0A(0x27),r16 ; rjmp .
        let prog = [0xE002, 0xBD04, 0xE001, 0xBD05, 0xE603, 0xBD07, 0xCFFF];
        let cpu = run(&prog, 500);
        assert_ne!(cpu.peek(0x35) & 0x02, 0, "OCF0A after 100 timer clocks");
    }

    #[test]
    fn hex_files_are_checked() {
        assert!(parse_hex(":020000000102FB\n:00000001FF\n").is_ok());
        assert!(parse_hex(":020000000102FA\n").is_err());
    }
}
