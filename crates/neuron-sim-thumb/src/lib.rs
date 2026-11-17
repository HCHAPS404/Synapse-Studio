//! A simulator of an ARMv6-M core (Cortex-M0+, the core of the RP2040) for task B6.
//!
//! It executes the whole ARMv6-M Thumb instruction set (the 16-bit instructions, `BL`,
//! `MRS`, `MSR`, `DMB`, `DSB`, `ISB`), counts cycles with the Cortex-M0+ timings of the
//! common cases, and models what a scheduler tick needs:
//!
//! - flash (read only) and RAM regions, with faults on unmapped or unaligned accesses;
//! - exception entry and return with the 8-word stack frame, `PRIMASK`, `VTOR`;
//! - SysTick (exception 15) and 32 external interrupts behind a minimal NVIC;
//! - a GPIO block laid out like the RP2040 `SIO` (`GPIO_IN`, `GPIO_OUT` and its
//!   `SET`/`CLR`/`XOR` aliases, `GPIO_OE`), with every output change recorded;
//! - `WFI` that skips idle time up to the next event.
//!
//! - the RP2040 peripherals the neuron-rt port waits on: RESETS (`RESET_DONE` follows the
//!   released resets), the crystal oscillator (`STATUS.STABLE`), the clock muxes
//!   (`*_SELECTED`), the edge interrupts of IO_BANK0 (IRQ 13) and the UART0 data register.
//!
//! Other peripheral registers (`0x4000_0000` to `0x5fff_ffff`) keep what is written to them,
//! so start-up code that writes configuration runs; their hardware behavior is not modelled.
//! A RAM image (as the RP2040 boot ROM loads from a UF2 file) is loaded with `load_ram` and
//! started with `reset_at`.
//!
//! ```
//! use neuron_sim_thumb::{Config, Thumb, Stop};
//! let mut cpu = Thumb::new(Config::rp2040());
//! // movs r0, #42 ; bkpt #0
//! cpu.load_halfwords(0, &[0x202a, 0xbe00]).unwrap();
//! cpu.set_pc(Config::rp2040().flash_base);
//! assert_eq!(cpu.run_until(1_000).unwrap(), Stop::Breakpoint(0));
//! assert_eq!(cpu.reg(0), 42);
//! ```

/// Memory layout and clock of the simulated chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// First address of the program memory.
    pub flash_base: u32,
    /// Size of the program memory in bytes.
    pub flash_size: u32,
    /// First address of the RAM.
    pub ram_base: u32,
    /// Size of the RAM in bytes.
    pub ram_size: u32,
    /// Core clock in hertz (to convert cycles to time).
    pub hz: u64,
}

impl Config {
    /// The RP2040 layout: 2 MiB of external flash at `0x1000_0000` (XIP), 264 KiB of RAM,
    /// 125 MHz.
    pub fn rp2040() -> Config {
        Config {
            flash_base: 0x1000_0000,
            flash_size: 2 * 1024 * 1024,
            ram_base: 0x2000_0000,
            ram_size: 264 * 1024,
            hz: 125_000_000,
        }
    }
}

/// Why the simulation stopped with an error (a HardFault on the real core).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SimError {
    /// An access outside every region.
    #[error("access to unmapped address {addr:#010x} at pc {pc:#010x}")]
    Unmapped {
        /// Address.
        addr: u32,
        /// Instruction.
        pc: u32,
    },
    /// A write to the program memory.
    #[error("write to read-only address {addr:#010x} at pc {pc:#010x}")]
    ReadOnly {
        /// Address.
        addr: u32,
        /// Instruction.
        pc: u32,
    },
    /// A halfword or word access that is not aligned (ARMv6-M faults on those).
    #[error("unaligned access to {addr:#010x} at pc {pc:#010x}")]
    Unaligned {
        /// Address.
        addr: u32,
        /// Instruction.
        pc: u32,
    },
    /// An encoding that ARMv6-M does not define, or `UDF`.
    #[error("undefined instruction {opcode:#06x} at pc {pc:#010x}")]
    Undefined {
        /// The first halfword.
        opcode: u32,
        /// Instruction.
        pc: u32,
    },
    /// `SVC` is not modelled (no supervisor calls in generated code).
    #[error("supervisor call {imm} at pc {pc:#010x} is not modelled")]
    Svc {
        /// Immediate.
        imm: u32,
        /// Instruction.
        pc: u32,
    },
}

/// Why `run_until` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The cycle limit was reached.
    Reached,
    /// A `BKPT` instruction executed (the program asked to stop).
    Breakpoint(u8),
}

/// A change of a GPIO output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinEvent {
    /// Cycle of the store that changed the pin.
    pub cycle: u64,
    /// GPIO number.
    pub pin: u8,
    /// New level.
    pub level: bool,
}

const SIO_BASE: u32 = 0xd000_0000;
const SCS_BASE: u32 = 0xe000_e000;
const EXC_SYSTICK: u32 = 15;
const EXC_RETURN_THREAD: u32 = 0xffff_fff9;

#[derive(Debug, Clone, Default)]
struct SysTick {
    csr: u32,
    rvr: u32,
    cvr: u32,
}

/// The simulated core and its memory.
#[derive(Debug, Clone)]
pub struct Thumb {
    cfg: Config,
    r: [u32; 16],
    n: bool,
    z: bool,
    c: bool,
    v: bool,
    ipsr: u32,
    primask: bool,
    cycles: u64,
    flash: Vec<u8>,
    ram: Vec<u8>,
    vtor: u32,
    systick: SysTick,
    /// Pending exceptions, bit `n` for exception number `n` (16 + IRQ).
    pending: u64,
    nvic_enabled: u32,
    gpio_out: u32,
    gpio_oe: u32,
    gpio_in: u32,
    events: Vec<PinEvent>,
    inputs: Vec<(u64, u8, bool)>,
    sleeping: bool,
    /// Address of the instruction being executed.
    cur: u32,
    /// RP2040 peripherals that start-up code waits on, and the other registers as storage.
    rp: RpPeriph,
}

/// The RP2040 peripheral behavior that the neuron-rt port needs: resets that complete,
/// a crystal oscillator that becomes stable, clock muxes that report their selection, edge
/// interrupts of IO_BANK0 and the UART0 transmit register.
#[derive(Debug, Clone)]
struct RpPeriph {
    regs: std::collections::BTreeMap<u32, u32>,
    resets: u32,
    xosc_on: bool,
    clk_ref: u32,
    clk_sys: u32,
    intr: [u32; 4],
    inte: [u32; 4],
    uart: Vec<(u64, u8)>,
}

impl Default for RpPeriph {
    fn default() -> RpPeriph {
        RpPeriph {
            regs: std::collections::BTreeMap::new(),
            resets: 0x01ff_ffff,
            xosc_on: false,
            clk_ref: 0,
            clk_sys: 0,
            intr: [0; 4],
            inte: [0; 4],
            uart: Vec::new(),
        }
    }
}

const RP_RESETS: u32 = 0x4000_c000;
const RP_CLOCKS: u32 = 0x4000_8000;
const RP_XOSC: u32 = 0x4002_4000;
const RP_IO_BANK0: u32 = 0x4001_4000;
const RP_UART0: u32 = 0x4003_4000;
/// NVIC number of IO_IRQ_BANK0 on the RP2040.
const RP_IO_IRQ: u32 = 13;

fn add_with_carry(x: u32, y: u32, carry: bool) -> (u32, bool, bool) {
    let unsigned = u64::from(x) + u64::from(y) + u64::from(carry);
    let result = unsigned as u32;
    let signed = i64::from(x as i32) + i64::from(y as i32) + i64::from(carry);
    (
        result,
        unsigned >> 32 != 0,
        signed != i64::from(result as i32),
    )
}

fn sign_extend(v: u32, bits: u32) -> u32 {
    let shift = 32 - bits;
    (((v << shift) as i32) >> shift) as u32
}

impl Thumb {
    /// A core with empty memory, stopped at address 0.
    pub fn new(cfg: Config) -> Thumb {
        Thumb {
            cfg,
            r: [0; 16],
            n: false,
            z: false,
            c: false,
            v: false,
            ipsr: 0,
            primask: false,
            cycles: 0,
            flash: vec![0; cfg.flash_size as usize],
            ram: vec![0; cfg.ram_size as usize],
            vtor: cfg.flash_base,
            systick: SysTick::default(),
            pending: 0,
            nvic_enabled: 0,
            gpio_out: 0,
            gpio_oe: 0,
            gpio_in: 0,
            events: Vec::new(),
            inputs: Vec::new(),
            sleeping: false,
            cur: 0,
            rp: RpPeriph::default(),
        }
    }

    /// Copies bytes into the RAM at `offset` from its base (a RAM image, as the RP2040 boot
    /// ROM loads one from a UF2 file).
    pub fn load_ram(&mut self, offset: u32, bytes: &[u8]) -> Result<(), SimError> {
        let start = offset as usize;
        let end = start + bytes.len();
        if end > self.ram.len() {
            return Err(SimError::Unmapped {
                addr: self.cfg.ram_base.wrapping_add(end as u32),
                pc: 0,
            });
        }
        self.ram[start..end].copy_from_slice(bytes);
        Ok(())
    }

    /// Starts an image whose vector table is at `base`: VTOR, SP and PC from the table.
    pub fn reset_at(&mut self, base: u32) -> Result<(), SimError> {
        self.vtor = base;
        self.r[13] = self.read(base, 4)? & !3;
        self.r[15] = self.read(base + 4, 4)? & !1;
        self.r[14] = 0xffff_ffff;
        self.ipsr = 0;
        self.primask = false;
        Ok(())
    }

    /// Bytes written to the UART0 data register, with the cycle of each write.
    pub fn uart_output(&self) -> &[(u64, u8)] {
        &self.rp.uart
    }

    fn read_periph(&mut self, addr: u32) -> u32 {
        match addr {
            a if a == RP_RESETS => self.rp.resets,
            a if a == RP_RESETS + 8 => !self.rp.resets & 0x01ff_ffff,
            a if a == RP_XOSC + 4 => {
                if self.rp.xosc_on {
                    0x8000_0000
                } else {
                    0
                }
            }
            a if a == RP_CLOCKS + 0x38 => 1 << (self.rp.clk_ref & 3),
            a if a == RP_CLOCKS + 0x44 => 1 << (self.rp.clk_sys & 1),
            a if (RP_IO_BANK0 + 0xf0..RP_IO_BANK0 + 0x100).contains(&a) => {
                self.rp.intr[((a - RP_IO_BANK0 - 0xf0) / 4) as usize]
            }
            a if (RP_IO_BANK0 + 0x100..RP_IO_BANK0 + 0x110).contains(&a) => {
                self.rp.inte[((a - RP_IO_BANK0 - 0x100) / 4) as usize]
            }
            a if (RP_IO_BANK0 + 0x120..RP_IO_BANK0 + 0x130).contains(&a) => {
                let i = ((a - RP_IO_BANK0 - 0x120) / 4) as usize;
                self.rp.intr[i] & self.rp.inte[i]
            }
            a if a == RP_UART0 + 0x18 => 0, // transmit FIFO never full
            a => self.rp.regs.get(&a).copied().unwrap_or(0),
        }
    }

    fn write_periph(&mut self, addr: u32, value: u32) {
        match addr {
            a if a == RP_RESETS => self.rp.resets = value & 0x01ff_ffff,
            a if a == RP_XOSC => self.rp.xosc_on = (value >> 12) & 0xfff == 0xfab,
            a if a == RP_CLOCKS + 0x30 => self.rp.clk_ref = value,
            a if a == RP_CLOCKS + 0x3c => self.rp.clk_sys = value,
            a if (RP_IO_BANK0 + 0xf0..RP_IO_BANK0 + 0x100).contains(&a) => {
                self.rp.intr[((a - RP_IO_BANK0 - 0xf0) / 4) as usize] &= !value;
                self.update_io_irq();
            }
            a if (RP_IO_BANK0 + 0x100..RP_IO_BANK0 + 0x110).contains(&a) => {
                self.rp.inte[((a - RP_IO_BANK0 - 0x100) / 4) as usize] = value;
                self.update_io_irq();
            }
            a if a == RP_UART0 => self.rp.uart.push((self.cycles, value as u8)),
            a => {
                self.rp.regs.insert(a, value);
            }
        }
    }

    /// IO_IRQ_BANK0 is level sensitive: pending while an enabled edge is latched.
    fn update_io_irq(&mut self) {
        let active = (0..4).any(|i| self.rp.intr[i] & self.rp.inte[i] != 0);
        if active {
            self.pending |= 1 << (16 + RP_IO_IRQ);
        }
    }

    /// Latches the edge bits of IO_BANK0 for a change of an input pin.
    fn latch_edge(&mut self, pin: u8, level: bool) {
        let i = usize::from(pin / 8);
        let shift = 4 * u32::from(pin % 8);
        let bit = if level { 0x8 } else { 0x4 };
        self.rp.intr[i] |= bit << shift;
        self.update_io_irq();
    }

    /// Copies bytes into the program memory at `offset` from its base.
    pub fn load_bytes(&mut self, offset: u32, bytes: &[u8]) -> Result<(), SimError> {
        let start = offset as usize;
        let end = start + bytes.len();
        if end > self.flash.len() {
            return Err(SimError::Unmapped {
                addr: self.cfg.flash_base.wrapping_add(end as u32),
                pc: 0,
            });
        }
        self.flash[start..end].copy_from_slice(bytes);
        Ok(())
    }

    /// Copies little-endian halfwords (instructions) into the program memory.
    pub fn load_halfwords(&mut self, offset: u32, code: &[u16]) -> Result<(), SimError> {
        let bytes: Vec<u8> = code.iter().flat_map(|h| h.to_le_bytes()).collect();
        self.load_bytes(offset, &bytes)
    }

    /// Takes the reset vector: SP from the first word of the vector table, PC from the second.
    pub fn reset(&mut self) -> Result<(), SimError> {
        self.vtor = self.cfg.flash_base;
        self.r[13] = self.read(self.vtor, 4)? & !3;
        let pc = self.read(self.vtor + 4, 4)?;
        self.r[15] = pc & !1;
        self.r[14] = 0xffff_ffff;
        self.ipsr = 0;
        self.primask = false;
        Ok(())
    }

    /// Sets the program counter (to run code without a vector table). SP starts at the
    /// top of the RAM.
    pub fn set_pc(&mut self, pc: u32) {
        self.r[15] = pc & !1;
        if self.r[13] == 0 {
            self.r[13] = self.cfg.ram_base.wrapping_add(self.cfg.ram_size);
        }
    }

    /// Register `n` (13 is SP, 14 is LR, 15 is the address of the next instruction).
    pub fn reg(&self, n: usize) -> u32 {
        self.r[n & 15]
    }

    /// Sets register `n`.
    pub fn set_reg(&mut self, n: usize, v: u32) {
        self.r[n & 15] = v;
    }

    /// Flags as `(N, Z, C, V)`.
    pub fn flags(&self) -> (bool, bool, bool, bool) {
        (self.n, self.z, self.c, self.v)
    }

    /// Cycles executed so far.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// The cycle at a time in milliseconds.
    pub fn cycle_of_ms(&self, ms: f64) -> u64 {
        (ms * self.cfg.hz as f64 / 1000.0).round() as u64
    }

    /// The time of a cycle in milliseconds.
    pub fn ms(&self, cycle: u64) -> f64 {
        cycle as f64 * 1000.0 / self.cfg.hz as f64
    }

    /// Output changes so far.
    pub fn pin_events(&self) -> &[PinEvent] {
        &self.events
    }

    /// Current level of the GPIO outputs (bit `n` is GPIO `n`).
    pub fn gpio_out(&self) -> u32 {
        self.gpio_out & self.gpio_oe
    }

    /// Drives an input pin at a given cycle.
    pub fn schedule_input(&mut self, cycle: u64, pin: u8, level: bool) {
        let at = self.inputs.partition_point(|(c, _, _)| *c <= cycle);
        self.inputs.insert(at, (cycle, pin & 31, level));
    }

    /// Makes external interrupt `irq` pending (it is taken when the NVIC enables it).
    pub fn raise_irq(&mut self, irq: u32) {
        if irq < 32 {
            self.pending |= 1 << (16 + irq);
        }
    }

    /// Reads a word of memory (for tests and inspection).
    pub fn peek(&mut self, addr: u32) -> Result<u32, SimError> {
        self.read(addr, 4)
    }

    // ---- memory ----

    fn fault_unmapped(&self, addr: u32) -> SimError {
        SimError::Unmapped { addr, pc: self.cur }
    }

    fn read(&mut self, addr: u32, size: u32) -> Result<u32, SimError> {
        if addr % size != 0 {
            return Err(SimError::Unaligned { addr, pc: self.cur });
        }
        let get = |mem: &[u8], off: u32| -> Option<u32> {
            let o = off as usize;
            let s = mem.get(o..o + size as usize)?;
            Some(
                s.iter()
                    .rev()
                    .fold(0u32, |acc, b| (acc << 8) | u32::from(*b)),
            )
        };
        if addr >= self.cfg.flash_base && addr - self.cfg.flash_base < self.cfg.flash_size {
            return get(&self.flash, addr - self.cfg.flash_base)
                .ok_or_else(|| self.fault_unmapped(addr));
        }
        if addr >= self.cfg.ram_base && addr - self.cfg.ram_base < self.cfg.ram_size {
            return get(&self.ram, addr - self.cfg.ram_base)
                .ok_or_else(|| self.fault_unmapped(addr));
        }
        if (SIO_BASE..SIO_BASE + 0x100).contains(&addr) {
            return Ok(match addr - SIO_BASE {
                0x004 => self.gpio_in,
                0x010 => self.gpio_out,
                0x020 => self.gpio_oe,
                _ => 0,
            });
        }
        if (SCS_BASE..SCS_BASE + 0x1000).contains(&addr) {
            return Ok(self.read_scs(addr - SCS_BASE));
        }
        if (0x4000_0000..0x6000_0000).contains(&addr) {
            return Ok(self.read_periph(addr));
        }
        Err(self.fault_unmapped(addr))
    }

    fn read_scs(&mut self, off: u32) -> u32 {
        match off {
            0x010 => {
                let v = self.systick.csr;
                self.systick.csr &= !(1 << 16);
                v | (1 << 2)
            }
            0x014 => self.systick.rvr,
            0x018 => self.systick.cvr,
            0x100 | 0x180 => self.nvic_enabled,
            0x200 | 0x280 => (self.pending >> 16) as u32,
            0xd00 => 0x410c_c601, // CPUID of a Cortex-M0+ r0p1
            0xd04 => self.ipsr & 0x1ff,
            0xd08 => self.vtor,
            _ => 0,
        }
    }

    fn write(&mut self, addr: u32, size: u32, value: u32) -> Result<(), SimError> {
        if addr % size != 0 {
            return Err(SimError::Unaligned { addr, pc: self.cur });
        }
        if addr >= self.cfg.flash_base && addr - self.cfg.flash_base < self.cfg.flash_size {
            return Err(SimError::ReadOnly { addr, pc: self.cur });
        }
        if addr >= self.cfg.ram_base && addr - self.cfg.ram_base < self.cfg.ram_size {
            let o = (addr - self.cfg.ram_base) as usize;
            let bytes = value.to_le_bytes();
            let dst = self
                .ram
                .get_mut(o..o + size as usize)
                .ok_or(SimError::Unmapped { addr, pc: self.cur })?;
            dst.copy_from_slice(&bytes[..size as usize]);
            return Ok(());
        }
        if (SIO_BASE..SIO_BASE + 0x100).contains(&addr) {
            let before = self.gpio_out();
            match addr - SIO_BASE {
                0x010 => self.gpio_out = value,
                0x014 => self.gpio_out |= value,
                0x018 => self.gpio_out &= !value,
                0x01c => self.gpio_out ^= value,
                0x020 => self.gpio_oe = value,
                0x024 => self.gpio_oe |= value,
                0x028 => self.gpio_oe &= !value,
                0x02c => self.gpio_oe ^= value,
                _ => {}
            }
            self.record(before);
            return Ok(());
        }
        if (SCS_BASE..SCS_BASE + 0x1000).contains(&addr) {
            self.write_scs(addr - SCS_BASE, value);
            return Ok(());
        }
        if (0x4000_0000..0x6000_0000).contains(&addr) {
            self.write_periph(addr, value);
            return Ok(());
        }
        Err(self.fault_unmapped(addr))
    }

    fn write_scs(&mut self, off: u32, value: u32) {
        match off {
            0x010 => self.systick.csr = (self.systick.csr & (1 << 16)) | (value & 0x7),
            0x014 => self.systick.rvr = value & 0x00ff_ffff,
            0x018 => {
                self.systick.cvr = 0;
                self.systick.csr &= !(1 << 16);
            }
            0x100 => self.nvic_enabled |= value,
            0x180 => self.nvic_enabled &= !value,
            0x200 => self.pending |= u64::from(value) << 16,
            0x280 => self.pending &= !(u64::from(value) << 16),
            0xd04 => {
                // ICSR: PENDSTSET (26) and PENDSTCLR (25).
                if value & (1 << 26) != 0 {
                    self.pending |= 1 << EXC_SYSTICK;
                }
                if value & (1 << 25) != 0 {
                    self.pending &= !(1 << EXC_SYSTICK);
                }
            }
            0xd08 => self.vtor = value & !0x7f,
            _ => {}
        }
    }

    fn record(&mut self, before: u32) {
        let after = self.gpio_out();
        let changed = before ^ after;
        for pin in 0..32u8 {
            if changed & (1 << pin) != 0 {
                self.events.push(PinEvent {
                    cycle: self.cycles,
                    pin,
                    level: after & (1 << pin) != 0,
                });
            }
        }
    }

    // ---- time ----

    /// Advances the clock by `n` cycles: SysTick and scheduled inputs.
    fn tick(&mut self, n: u64) {
        self.cycles += n;
        if self.systick.csr & 1 != 0 {
            let mut left = n;
            while left > 0 {
                if self.systick.cvr == 0 {
                    if self.systick.rvr == 0 {
                        break;
                    }
                    self.systick.cvr = self.systick.rvr;
                    left -= 1;
                } else if left >= u64::from(self.systick.cvr) {
                    left -= u64::from(self.systick.cvr);
                    self.systick.cvr = 0;
                    self.systick.csr |= 1 << 16;
                    if self.systick.csr & 2 != 0 {
                        self.pending |= 1 << EXC_SYSTICK;
                    }
                } else {
                    self.systick.cvr -= left as u32;
                    left = 0;
                }
            }
        }
        while let Some(&(at, pin, level)) = self.inputs.first() {
            if at > self.cycles {
                break;
            }
            self.inputs.remove(0);
            let before = self.gpio_in & (1 << pin) != 0;
            if level {
                self.gpio_in |= 1 << pin;
            } else {
                self.gpio_in &= !(1 << pin);
            }
            if before != level {
                self.latch_edge(pin, level);
            }
        }
    }

    /// Cycles until the next thing that can wake the core, if any.
    fn next_wake(&self) -> Option<u64> {
        let mut best: Option<u64> = self
            .inputs
            .first()
            .map(|(c, _, _)| c.saturating_sub(self.cycles).max(1));
        if self.systick.csr & 3 == 3 && (self.systick.rvr != 0 || self.systick.cvr != 0) {
            let d = if self.systick.cvr == 0 {
                1 + u64::from(self.systick.rvr)
            } else {
                u64::from(self.systick.cvr)
            };
            best = Some(best.map_or(d, |b| b.min(d)));
        }
        best
    }

    // ---- exceptions ----

    fn takeable(&self) -> Option<u32> {
        if self.primask || self.ipsr != 0 {
            return None;
        }
        let enabled_irqs = u64::from(self.nvic_enabled) << 16;
        let mask = self.pending & ((1 << EXC_SYSTICK) | enabled_irqs);
        (mask != 0).then(|| mask.trailing_zeros())
    }

    fn xpsr(&self) -> u32 {
        (u32::from(self.n) << 31)
            | (u32::from(self.z) << 30)
            | (u32::from(self.c) << 29)
            | (u32::from(self.v) << 28)
            | (1 << 24)
            | (self.ipsr & 0x3f)
    }

    fn enter(&mut self, exc: u32) -> Result<(), SimError> {
        self.pending &= !(1u64 << exc);
        let mut sp = self.r[13].wrapping_sub(32);
        let mut psr = self.xpsr();
        if sp & 4 != 0 {
            sp = sp.wrapping_sub(4);
            psr |= 1 << 9;
        }
        let frame = [
            self.r[0], self.r[1], self.r[2], self.r[3], self.r[12], self.r[14], self.r[15], psr,
        ];
        for (i, w) in frame.iter().enumerate() {
            self.write(sp + 4 * i as u32, 4, *w)?;
        }
        self.r[13] = sp;
        self.r[14] = EXC_RETURN_THREAD;
        self.ipsr = exc;
        self.r[15] = self.read(self.vtor + 4 * exc, 4)? & !1;
        self.sleeping = false;
        self.tick(15);
        Ok(())
    }

    fn exit(&mut self) -> Result<(), SimError> {
        let sp = self.r[13];
        let mut frame = [0u32; 8];
        for (i, w) in frame.iter_mut().enumerate() {
            *w = self.read(sp + 4 * i as u32, 4)?;
        }
        let [r0, r1, r2, r3, r12, lr, pc, psr] = frame;
        self.r[0] = r0;
        self.r[1] = r1;
        self.r[2] = r2;
        self.r[3] = r3;
        self.r[12] = r12;
        self.r[14] = lr;
        self.r[15] = pc & !1;
        self.n = psr & (1 << 31) != 0;
        self.z = psr & (1 << 30) != 0;
        self.c = psr & (1 << 29) != 0;
        self.v = psr & (1 << 28) != 0;
        self.r[13] = sp + 32 + if psr & (1 << 9) != 0 { 4 } else { 0 };
        self.ipsr = psr & 0x3f;
        self.tick(10);
        Ok(())
    }

    /// A write to PC by `BX`, `BLX`, `POP` or `LDM`: an exception return or a branch.
    fn branch_exchange(&mut self, target: u32) -> Result<(), SimError> {
        if self.ipsr != 0 && target >= 0xffff_fff0 {
            self.exit()
        } else {
            self.r[15] = target & !1;
            Ok(())
        }
    }

    // ---- execution ----

    /// Runs until the cycle counter reaches `cycle` or a `BKPT` executes.
    pub fn run_until(&mut self, cycle: u64) -> Result<Stop, SimError> {
        while self.cycles < cycle {
            if let Some(exc) = self.takeable() {
                self.enter(exc)?;
                continue;
            }
            if self.sleeping {
                // Skip the idle time up to the next event (or to the limit).
                let left = cycle - self.cycles;
                let step = self.next_wake().map_or(left, |w| w.min(left)).max(1);
                self.tick(step);
                continue;
            }
            if let Some(imm) = self.step()? {
                return Ok(Stop::Breakpoint(imm));
            }
        }
        Ok(Stop::Reached)
    }

    fn set_nz(&mut self, v: u32) {
        self.n = v & (1 << 31) != 0;
        self.z = v == 0;
    }

    fn cond(&self, c: u32) -> bool {
        match c {
            0 => self.z,
            1 => !self.z,
            2 => self.c,
            3 => !self.c,
            4 => self.n,
            5 => !self.n,
            6 => self.v,
            7 => !self.v,
            8 => self.c && !self.z,
            9 => !self.c || self.z,
            10 => self.n == self.v,
            11 => self.n != self.v,
            12 => !self.z && self.n == self.v,
            13 => self.z || self.n != self.v,
            _ => true,
        }
    }

    fn undefined(&self, op: u32) -> SimError {
        SimError::Undefined {
            opcode: op,
            pc: self.cur,
        }
    }

    /// Executes one instruction. Returns the immediate of a `BKPT`.
    fn step(&mut self) -> Result<Option<u8>, SimError> {
        let pc = self.r[15];
        self.cur = pc;
        let op = self.read(pc, 2)?;
        // The value of PC that instructions see.
        let pcv = pc.wrapping_add(4);
        self.r[15] = pc.wrapping_add(2);
        let lo = |n: u32| (n & 7) as usize;
        let mut cycles = 1u64;
        match op >> 11 {
            // LSLS, LSRS, ASRS (immediate)
            0b00000..=0b00010 => {
                let imm = (op >> 6) & 31;
                let x = self.r[lo(op >> 3)];
                let kind = op >> 11;
                let (res, carry) = match (kind, imm) {
                    (0, 0) => (x, self.c),
                    (0, n) => (x << n, x & (1 << (32 - n)) != 0),
                    (1, 0) => (0, x & (1 << 31) != 0),
                    (1, n) => (x >> n, x & (1 << (n - 1)) != 0),
                    (_, 0) => (((x as i32) >> 31) as u32, x & (1 << 31) != 0),
                    (_, n) => (((x as i32) >> n) as u32, x & (1 << (n - 1)) != 0),
                };
                self.r[lo(op)] = res;
                self.set_nz(res);
                self.c = carry;
            }
            // ADDS/SUBS register or 3-bit immediate
            0b00011 => {
                let rn = self.r[lo(op >> 3)];
                let field = (op >> 6) & 7;
                let operand = if op & (1 << 10) != 0 {
                    field
                } else {
                    self.r[field as usize]
                };
                let (res, c, v) = if op & (1 << 9) != 0 {
                    add_with_carry(rn, !operand, true)
                } else {
                    add_with_carry(rn, operand, false)
                };
                self.r[lo(op)] = res;
                self.set_nz(res);
                self.c = c;
                self.v = v;
            }
            // MOVS, CMP, ADDS, SUBS with an 8-bit immediate
            0b00100..=0b00111 => {
                let rd = lo(op >> 8);
                let imm = op & 0xff;
                match (op >> 11) & 3 {
                    0 => {
                        self.r[rd] = imm;
                        self.set_nz(imm);
                    }
                    1 => {
                        let (res, c, v) = add_with_carry(self.r[rd], !imm, true);
                        self.set_nz(res);
                        self.c = c;
                        self.v = v;
                    }
                    2 => {
                        let (res, c, v) = add_with_carry(self.r[rd], imm, false);
                        self.r[rd] = res;
                        self.set_nz(res);
                        self.c = c;
                        self.v = v;
                    }
                    _ => {
                        let (res, c, v) = add_with_carry(self.r[rd], !imm, true);
                        self.r[rd] = res;
                        self.set_nz(res);
                        self.c = c;
                        self.v = v;
                    }
                }
            }
            0b01000 => {
                if op & (1 << 10) == 0 {
                    self.data_processing(op);
                } else {
                    cycles = self.special_data(op, pcv)?;
                }
            }
            // LDR (literal)
            0b01001 => {
                let addr = (pcv & !3) + 4 * (op & 0xff);
                self.r[lo(op >> 8)] = self.read(addr, 4)?;
                cycles = 2;
            }
            // Load and store with a register offset
            0b01010 | 0b01011 => {
                let addr = self.r[lo(op >> 3)].wrapping_add(self.r[lo(op >> 6)]);
                let rt = lo(op);
                match (op >> 9) & 7 {
                    0 => self.write(addr, 4, self.r[rt])?,
                    1 => self.write(addr, 2, self.r[rt] & 0xffff)?,
                    2 => self.write(addr, 1, self.r[rt] & 0xff)?,
                    3 => self.r[rt] = sign_extend(self.read(addr, 1)?, 8),
                    4 => self.r[rt] = self.read(addr, 4)?,
                    5 => self.r[rt] = self.read(addr, 2)?,
                    6 => self.r[rt] = self.read(addr, 1)?,
                    _ => self.r[rt] = sign_extend(self.read(addr, 2)?, 16),
                }
                cycles = 2;
            }
            // STR, LDR, STRB, LDRB, STRH, LDRH with a 5-bit immediate
            0b01100..=0b10001 => {
                let imm = (op >> 6) & 31;
                let base = self.r[lo(op >> 3)];
                let rt = lo(op);
                match op >> 11 {
                    0b01100 => self.write(base.wrapping_add(imm * 4), 4, self.r[rt])?,
                    0b01101 => self.r[rt] = self.read(base.wrapping_add(imm * 4), 4)?,
                    0b01110 => self.write(base.wrapping_add(imm), 1, self.r[rt] & 0xff)?,
                    0b01111 => self.r[rt] = self.read(base.wrapping_add(imm), 1)?,
                    0b10000 => self.write(base.wrapping_add(imm * 2), 2, self.r[rt] & 0xffff)?,
                    _ => self.r[rt] = self.read(base.wrapping_add(imm * 2), 2)?,
                }
                cycles = 2;
            }
            // STR, LDR relative to SP
            0b10010 | 0b10011 => {
                let addr = self.r[13].wrapping_add(4 * (op & 0xff));
                let rt = lo(op >> 8);
                if op & (1 << 11) == 0 {
                    self.write(addr, 4, self.r[rt])?;
                } else {
                    self.r[rt] = self.read(addr, 4)?;
                }
                cycles = 2;
            }
            // ADR, ADD rd, SP, #imm
            0b10100 => self.r[lo(op >> 8)] = (pcv & !3) + 4 * (op & 0xff),
            0b10101 => self.r[lo(op >> 8)] = self.r[13].wrapping_add(4 * (op & 0xff)),
            0b10110 | 0b10111 => return self.misc(op, pcv),
            // STM, LDM
            0b11000 | 0b11001 => {
                let rn = lo(op >> 8);
                let list = op & 0xff;
                let mut addr = self.r[rn];
                let load = op & (1 << 11) != 0;
                for i in 0..8 {
                    if list & (1 << i) != 0 {
                        if load {
                            self.r[i] = self.read(addr, 4)?;
                        } else {
                            self.write(addr, 4, self.r[i])?;
                        }
                        addr = addr.wrapping_add(4);
                        cycles += 1;
                    }
                }
                if !load || list & (1 << rn) == 0 {
                    self.r[rn] = addr;
                }
            }
            // B<cond>, UDF, SVC
            0b11010 | 0b11011 => {
                let c = (op >> 8) & 15;
                match c {
                    14 => return Err(self.undefined(op)),
                    15 => return Err(SimError::Svc { imm: op & 0xff, pc }),
                    _ => {
                        if self.cond(c) {
                            self.r[15] = pcv.wrapping_add(sign_extend(op & 0xff, 8) << 1);
                            cycles = 2;
                        }
                    }
                }
            }
            // B
            0b11100 => {
                self.r[15] = pcv.wrapping_add(sign_extend(op & 0x7ff, 11) << 1);
                cycles = 2;
            }
            // 32-bit instructions
            0b11110 => {
                let op2 = self.read(pc.wrapping_add(2), 2)?;
                self.r[15] = pc.wrapping_add(4);
                cycles = self.wide(op, op2, pcv)?;
            }
            _ => return Err(self.undefined(op)),
        }
        self.tick(cycles);
        Ok(None)
    }

    fn data_processing(&mut self, op: u32) {
        let rdn = (op & 7) as usize;
        let m = self.r[((op >> 3) & 7) as usize];
        let x = self.r[rdn];
        let shift_amount = m & 0xff;
        match (op >> 6) & 15 {
            0 => self.logic(rdn, x & m, true),
            1 => self.logic(rdn, x ^ m, true),
            2 => {
                let (res, c) = match shift_amount {
                    0 => (x, self.c),
                    n @ 1..=31 => (x << n, x & (1 << (32 - n)) != 0),
                    32 => (0, x & 1 != 0),
                    _ => (0, false),
                };
                self.c = c;
                self.logic(rdn, res, true);
            }
            3 => {
                let (res, c) = match shift_amount {
                    0 => (x, self.c),
                    n @ 1..=31 => (x >> n, x & (1 << (n - 1)) != 0),
                    32 => (0, x & (1 << 31) != 0),
                    _ => (0, false),
                };
                self.c = c;
                self.logic(rdn, res, true);
            }
            4 => {
                let (res, c) = match shift_amount {
                    0 => (x, self.c),
                    n @ 1..=31 => (((x as i32) >> n) as u32, x & (1 << (n - 1)) != 0),
                    _ => (((x as i32) >> 31) as u32, x & (1 << 31) != 0),
                };
                self.c = c;
                self.logic(rdn, res, true);
            }
            5 => {
                let (res, c, v) = add_with_carry(x, m, self.c);
                self.arith(rdn, res, c, v, true);
            }
            6 => {
                let (res, c, v) = add_with_carry(x, !m, self.c);
                self.arith(rdn, res, c, v, true);
            }
            7 => {
                let (res, c) = if shift_amount == 0 {
                    (x, self.c)
                } else {
                    let r = x.rotate_right(shift_amount & 31);
                    (r, r & (1 << 31) != 0)
                };
                self.c = c;
                self.logic(rdn, res, true);
            }
            8 => self.logic(rdn, x & m, false),
            9 => {
                let (res, c, v) = add_with_carry(!m, 0, true);
                self.arith(rdn, res, c, v, true);
            }
            10 => {
                let (res, c, v) = add_with_carry(x, !m, true);
                self.arith(rdn, res, c, v, false);
            }
            11 => {
                let (res, c, v) = add_with_carry(x, m, false);
                self.arith(rdn, res, c, v, false);
            }
            12 => self.logic(rdn, x | m, true),
            13 => self.logic(rdn, x.wrapping_mul(m), true),
            14 => self.logic(rdn, x & !m, true),
            _ => self.logic(rdn, !m, true),
        }
    }

    fn logic(&mut self, rd: usize, res: u32, write: bool) {
        if write {
            self.r[rd] = res;
        }
        self.set_nz(res);
    }

    fn arith(&mut self, rd: usize, res: u32, c: bool, v: bool, write: bool) {
        if write {
            self.r[rd] = res;
        }
        self.set_nz(res);
        self.c = c;
        self.v = v;
    }

    /// `ADD`, `CMP`, `MOV` on high registers, `BX`, `BLX`.
    fn special_data(&mut self, op: u32, pcv: u32) -> Result<u64, SimError> {
        let rm = ((op >> 3) & 15) as usize;
        let rdn = ((op & 7) | ((op >> 4) & 8)) as usize;
        let read = |s: &Thumb, r: usize| if r == 15 { pcv } else { s.r[r] };
        match (op >> 8) & 3 {
            0 => {
                let v = read(self, rdn).wrapping_add(read(self, rm));
                if rdn == 15 {
                    self.r[15] = v & !1;
                    return Ok(2);
                }
                self.r[rdn] = v;
            }
            1 => {
                let (res, c, v) = add_with_carry(read(self, rdn), !read(self, rm), true);
                self.set_nz(res);
                self.c = c;
                self.v = v;
            }
            2 => {
                let v = read(self, rm);
                if rdn == 15 {
                    self.r[15] = v & !1;
                    return Ok(2);
                }
                self.r[rdn] = v;
            }
            _ => {
                let target = read(self, rm);
                if op & (1 << 7) != 0 {
                    self.r[14] = self.r[15] | 1;
                }
                self.branch_exchange(target)?;
                return Ok(2);
            }
        }
        Ok(1)
    }

    /// The `1011` group: SP adjust, extends, PUSH, POP, CPS, REV, BKPT, hints.
    fn misc(&mut self, op: u32, _pcv: u32) -> Result<Option<u8>, SimError> {
        let rd = (op & 7) as usize;
        let rm = self.r[((op >> 3) & 7) as usize];
        let mut cycles = 1u64;
        match (op >> 8) & 15 {
            0 => {
                let imm = 4 * (op & 0x7f);
                self.r[13] = if op & 0x80 != 0 {
                    self.r[13].wrapping_sub(imm)
                } else {
                    self.r[13].wrapping_add(imm)
                };
            }
            2 => {
                self.r[rd] = match (op >> 6) & 3 {
                    0 => sign_extend(rm & 0xffff, 16),
                    1 => sign_extend(rm & 0xff, 8),
                    2 => rm & 0xffff,
                    _ => rm & 0xff,
                };
            }
            4 | 5 => {
                let list = op & 0xff;
                let lr = op & (1 << 8) != 0;
                let count = list.count_ones() + u32::from(lr);
                let mut addr = self.r[13].wrapping_sub(4 * count);
                self.r[13] = addr;
                for i in 0..8 {
                    if list & (1 << i) != 0 {
                        self.write(addr, 4, self.r[i])?;
                        addr += 4;
                    }
                }
                if lr {
                    self.write(addr, 4, self.r[14])?;
                }
                cycles = 1 + u64::from(count);
            }
            6 => {
                if op & 0xffef == 0xb662 {
                    self.primask = op & (1 << 4) != 0;
                } else {
                    return Err(self.undefined(op));
                }
            }
            10 => {
                self.r[rd] = match (op >> 6) & 3 {
                    0 => rm.swap_bytes(),
                    1 => ((rm & 0x00ff_00ff) << 8) | ((rm >> 8) & 0x00ff_00ff),
                    3 => sign_extend(u32::from((rm as u16).swap_bytes()), 16),
                    _ => return Err(self.undefined(op)),
                };
            }
            12 | 13 => {
                let list = op & 0xff;
                let pc = op & (1 << 8) != 0;
                let mut addr = self.r[13];
                for i in 0..8 {
                    if list & (1 << i) != 0 {
                        self.r[i] = self.read(addr, 4)?;
                        addr += 4;
                    }
                }
                cycles = 1 + u64::from(list.count_ones());
                if pc {
                    let target = self.read(addr, 4)?;
                    addr += 4;
                    self.r[13] = addr;
                    self.branch_exchange(target)?;
                    cycles += 3;
                } else {
                    self.r[13] = addr;
                }
            }
            14 => {
                self.tick(1);
                return Ok(Some((op & 0xff) as u8));
            }
            15 => {
                match (op >> 4) & 15 {
                    0 | 1 | 4 => {} // NOP, YIELD, SEV
                    2 | 3 => {
                        // WFE and WFI: wait for an interrupt.
                        if self.takeable().is_none() {
                            self.sleeping = true;
                        }
                    }
                    _ => return Err(self.undefined(op)),
                }
                if op & 0xf != 0 {
                    return Err(self.undefined(op));
                }
            }
            _ => return Err(self.undefined(op)),
        }
        self.tick(cycles);
        Ok(None)
    }

    /// `BL`, `MSR`, `MRS`, `DMB`, `DSB`, `ISB`.
    fn wide(&mut self, op1: u32, op2: u32, _pcv: u32) -> Result<u64, SimError> {
        if op2 & 0xd000 == 0xd000 {
            // BL
            let s = (op1 >> 10) & 1;
            let j1 = (op2 >> 13) & 1;
            let j2 = (op2 >> 11) & 1;
            let i1 = !(j1 ^ s) & 1;
            let i2 = !(j2 ^ s) & 1;
            let imm =
                (s << 24) | (i1 << 23) | (i2 << 22) | ((op1 & 0x3ff) << 12) | ((op2 & 0x7ff) << 1);
            let offset = sign_extend(imm, 25);
            let next = self.r[15];
            self.r[14] = next | 1;
            self.r[15] = next.wrapping_add(offset);
            return Ok(3);
        }
        if op1 & 0xfff0 == 0xf380 && op2 & 0xff00 == 0x8800 {
            // MSR spec_reg, Rn
            let v = self.r[(op1 & 15) as usize];
            match op2 & 0xff {
                8 => self.r[13] = v & !3,
                16 => self.primask = v & 1 != 0,
                _ => {}
            }
            return Ok(4);
        }
        if op1 == 0xf3ef && op2 & 0xf000 == 0x8000 {
            // MRS Rd, spec_reg
            let rd = ((op2 >> 8) & 15) as usize;
            self.r[rd] = match op2 & 0xff {
                0 => self.xpsr() & 0xf000_0000,
                1..=3 => self.xpsr() & 0xf000_003f,
                5 => self.ipsr & 0x3f,
                8 => self.r[13],
                16 => u32::from(self.primask),
                _ => 0,
            };
            return Ok(4);
        }
        if op1 == 0xf3bf && op2 & 0xff00 == 0x8f00 && matches!((op2 >> 4) & 15, 4..=6) {
            // DSB, DMB, ISB
            return Ok(3);
        }
        Err(self.undefined(op1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_with_carry_matches_the_architecture() {
        assert_eq!(
            add_with_carry(0x7fff_ffff, 1, false),
            (0x8000_0000, false, true)
        );
        assert_eq!(add_with_carry(0xffff_ffff, 1, false), (0, true, false));
        // 5 - 7 as 5 + !7 + 1: borrow, so C is clear.
        assert_eq!(add_with_carry(5, !7, true), (0xffff_fffe, false, false));
    }

    #[test]
    fn sign_extension() {
        assert_eq!(sign_extend(0xff, 8), 0xffff_ffff);
        assert_eq!(sign_extend(0x7f, 8), 0x7f);
    }
}
