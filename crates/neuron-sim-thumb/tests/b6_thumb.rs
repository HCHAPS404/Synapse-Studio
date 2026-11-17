//! Task B6: the ARMv6-M simulator runs hand-assembled programs with exact results and
//! cycle counts, takes SysTick interrupts at the programmed period, and drives the GPIO.
//! No ARM compiler is needed: the programs are encoded by the small assembler below,
//! whose encodings follow the ARMv6-M architecture.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use neuron_sim_thumb::{Config, SimError, Stop, Thumb};
use proptest::prelude::*;

/// A tiny assembler: 16-bit instructions, `BL`, labels and a literal pool.
#[derive(Default)]
struct Asm {
    code: Vec<u16>,
    /// `(index of the LDR, value)`.
    literals: Vec<(usize, u32)>,
}

const SIO: u32 = 0xd000_0000;
const SYST_CSR: u32 = 0xe000_e010;

impl Asm {
    fn here(&self) -> usize {
        self.code.len()
    }
    fn h(&mut self, op: u16) -> &mut Self {
        self.code.push(op);
        self
    }
    fn movs(&mut self, rd: u16, imm: u16) -> &mut Self {
        self.h(0x2000 | rd << 8 | imm)
    }
    fn adds_i(&mut self, rd: u16, imm: u16) -> &mut Self {
        self.h(0x3000 | rd << 8 | imm)
    }
    fn subs_i(&mut self, rd: u16, imm: u16) -> &mut Self {
        self.h(0x3800 | rd << 8 | imm)
    }
    fn adds(&mut self, rd: u16, rn: u16, rm: u16) -> &mut Self {
        self.h(0x1800 | rm << 6 | rn << 3 | rd)
    }
    fn subs(&mut self, rd: u16, rn: u16, rm: u16) -> &mut Self {
        self.h(0x1a00 | rm << 6 | rn << 3 | rd)
    }
    fn dp(&mut self, op: u16, rdn: u16, rm: u16) -> &mut Self {
        self.h(0x4000 | op << 6 | rm << 3 | rdn)
    }
    fn lsls(&mut self, rd: u16, rm: u16, imm: u16) -> &mut Self {
        self.h(imm << 6 | rm << 3 | rd)
    }
    fn str_(&mut self, rt: u16, rn: u16, imm_words: u16) -> &mut Self {
        self.h(0x6000 | imm_words << 6 | rn << 3 | rt)
    }
    fn ldr(&mut self, rt: u16, rn: u16, imm_words: u16) -> &mut Self {
        self.h(0x6800 | imm_words << 6 | rn << 3 | rt)
    }
    fn strb(&mut self, rt: u16, rn: u16, imm: u16) -> &mut Self {
        self.h(0x7000 | imm << 6 | rn << 3 | rt)
    }
    fn ldrsb_r(&mut self, rt: u16, rn: u16, rm: u16) -> &mut Self {
        self.h(0x5600 | rm << 6 | rn << 3 | rt)
    }
    fn ldrh(&mut self, rt: u16, rn: u16, imm_halves: u16) -> &mut Self {
        self.h(0x8800 | imm_halves << 6 | rn << 3 | rt)
    }
    fn push(&mut self, list: u16, lr: bool) -> &mut Self {
        self.h(0xb400 | u16::from(lr) << 8 | list)
    }
    fn pop(&mut self, list: u16, pc: bool) -> &mut Self {
        self.h(0xbc00 | u16::from(pc) << 8 | list)
    }
    fn bx_lr(&mut self) -> &mut Self {
        self.h(0x4770)
    }
    fn bkpt(&mut self, imm: u16) -> &mut Self {
        self.h(0xbe00 | imm)
    }
    fn wfi(&mut self) -> &mut Self {
        self.h(0xbf30)
    }
    fn cpsid(&mut self) -> &mut Self {
        self.h(0xb672)
    }
    /// `B<cond>` to an earlier or later instruction index.
    fn bcond_to(&mut self, cond: u16, target: usize) -> &mut Self {
        let from = self.here() as i64 + 2;
        let off = (target as i64 - from) as i16 as u16;
        self.h(0xd000 | cond << 8 | (off & 0xff))
    }
    fn b_to(&mut self, target: usize) -> &mut Self {
        let from = self.here() as i64 + 2;
        let off = (target as i64 - from) as i16 as u16;
        self.h(0xe000 | (off & 0x7ff))
    }
    /// `BL` to an instruction index.
    fn bl_to(&mut self, target: usize) -> &mut Self {
        let from = (self.here() as i64 + 2) * 2;
        let off = (target as i64 * 2 - from) as i32 as u32;
        let s = (off >> 24) & 1;
        let i1 = (off >> 23) & 1;
        let i2 = (off >> 22) & 1;
        let j1 = (!(i1 ^ s)) & 1;
        let j2 = (!(i2 ^ s)) & 1;
        let hi = 0xf000 | (s << 10) | ((off >> 12) & 0x3ff);
        let lo = 0xd000 | (j1 << 13) | (j2 << 11) | ((off >> 1) & 0x7ff);
        self.h(hi as u16).h(lo as u16)
    }
    /// `LDR rd, =value` through the literal pool.
    fn ldr_const(&mut self, rd: u16, value: u32) -> &mut Self {
        self.literals.push((self.here(), value));
        self.h(0x4800 | rd << 8)
    }
    /// The code with its literal pool appended, as halfwords at offset `base` (bytes).
    fn finish(&mut self, base: u32) -> Vec<u16> {
        let mut code = self.code.clone();
        if code.len() % 2 == 1 {
            code.push(0xbf00);
        }
        for (at, value) in self.literals.clone() {
            let lit = code.len();
            code.push(value as u16);
            code.push((value >> 16) as u16);
            let instr_addr = base + 2 * at as u32;
            let pc = (instr_addr + 4) & !3;
            let lit_addr = base + 2 * lit as u32;
            let imm = (lit_addr - pc) / 4;
            assert!(imm < 256, "literal out of range");
            code[at] |= imm as u16;
        }
        code
    }
}

fn run(code: &[u16], limit: u64) -> (Thumb, Stop) {
    let cfg = Config::rp2040();
    let mut cpu = Thumb::new(cfg);
    cpu.load_halfwords(0, code).unwrap();
    cpu.set_pc(cfg.flash_base);
    let stop = cpu.run_until(limit).unwrap();
    (cpu, stop)
}

#[test]
fn a_counted_loop_gives_the_result_and_the_cycles() {
    let mut a = Asm::default();
    a.movs(0, 10).movs(1, 0);
    let top = a.here();
    a.adds_i(1, 3).subs_i(0, 1).bcond_to(1, top).bkpt(7);
    let (cpu, stop) = run(&a.finish(0), 10_000);
    assert_eq!(stop, Stop::Breakpoint(7));
    assert_eq!(cpu.reg(1), 30);
    // 2 moves, 10 x (adds + subs), 9 taken branches (2 cycles) and 1 not taken, the BKPT.
    assert_eq!(cpu.cycles(), 2 + 20 + 9 * 2 + 1 + 1);
}

#[test]
fn a_call_saves_and_restores_through_the_stack() {
    let mut a = Asm::default();
    a.movs(0, 5).movs(1, 7);
    let call_at = a.here();
    a.h(0).h(0); // BL placeholder
    a.bkpt(0);
    let func = a.here();
    a.push(1 << 4, true)
        .adds(4, 0, 1)
        .movs(2, 0)
        .adds(0, 4, 2)
        .pop(1 << 4, true);
    // Patch the BL now that the target is known.
    let mut b = Asm {
        code: a.code[..call_at].to_vec(),
        ..Asm::default()
    };
    b.bl_to(func);
    a.code[call_at] = b.code[call_at];
    a.code[call_at + 1] = b.code[call_at + 1];
    let (cpu, stop) = run(&a.finish(0), 10_000);
    assert_eq!(stop, Stop::Breakpoint(0));
    assert_eq!(cpu.reg(0), 12);
    let cfg = Config::rp2040();
    assert_eq!(
        cpu.reg(13),
        cfg.ram_base + cfg.ram_size,
        "the stack is balanced"
    );
}

#[test]
fn loads_and_stores_of_every_width() {
    let mut a = Asm::default();
    a.ldr_const(0, 0x2000_0100)
        .ldr_const(1, 0x8081_f2f3)
        .str_(1, 0, 0) // word at 0x100
        .ldrh(2, 0, 0) // 0xf2f3
        .movs(3, 1)
        .ldrsb_r(4, 0, 3) // byte 0xf2 sign-extended
        .movs(5, 0x5a)
        .strb(5, 0, 3) // byte 3 becomes 0x5a
        .ldr(6, 0, 0)
        .bkpt(0);
    let (cpu, _) = run(&a.finish(0), 10_000);
    assert_eq!(cpu.reg(2), 0xf2f3);
    assert_eq!(cpu.reg(4), 0xffff_fff2);
    assert_eq!(cpu.reg(6), 0x5a81_f2f3);
}

#[test]
fn logic_shifts_and_multiply() {
    let mut a = Asm::default();
    a.movs(0, 0xf0)
        .movs(1, 0x3c)
        .movs(2, 0)
        .adds(2, 2, 0)
        .dp(0, 2, 1) // AND -> 0x30
        .movs(3, 0)
        .adds(3, 3, 0)
        .dp(1, 3, 1) // EOR -> 0xcc
        .movs(4, 6)
        .movs(5, 7)
        .dp(13, 4, 5) // MUL -> 42
        .movs(6, 1)
        .lsls(6, 6, 31) // 0x8000_0000, N set
        .bkpt(0);
    let (cpu, _) = run(&a.finish(0), 10_000);
    assert_eq!(cpu.reg(2), 0x30);
    assert_eq!(cpu.reg(3), 0xcc);
    assert_eq!(cpu.reg(4), 42);
    assert_eq!(cpu.reg(6), 0x8000_0000);
    assert!(cpu.flags().0, "N");
}

/// Vector table, then a reset handler that starts SysTick and sleeps, and a SysTick
/// handler that toggles GPIO 25: the RP2040 board LED.
fn blinky(reload: u32) -> Vec<u16> {
    let cfg = Config::rp2040();
    let vectors = 16usize; // words
    let reset_at = 2 * vectors; // halfword index
    let mut a = Asm {
        code: vec![0; reset_at],
        ..Asm::default()
    };
    // reset:
    a.ldr_const(0, SIO)
        .ldr_const(1, 1 << 25)
        .str_(1, 0, 0x24 / 4) // GPIO_OE_SET
        .ldr_const(2, SYST_CSR)
        .ldr_const(3, reload)
        .str_(3, 2, 1) // RVR
        .movs(3, 0)
        .str_(3, 2, 2) // CVR
        .movs(3, 7)
        .str_(3, 2, 0); // CSR: enable, interrupt, core clock
    let idle = a.here();
    a.wfi().b_to(idle);
    let handler = a.here();
    a.ldr_const(0, SIO)
        .ldr_const(1, 1 << 25)
        .str_(1, 0, 0x1c / 4)
        .bx_lr(); // GPIO_OUT_XOR
    let mut code = a.finish(0);
    let word = |code: &mut Vec<u16>, i: usize, v: u32| {
        code[2 * i] = v as u16;
        code[2 * i + 1] = (v >> 16) as u16;
    };
    word(&mut code, 0, cfg.ram_base + cfg.ram_size);
    word(&mut code, 1, cfg.flash_base + 2 * reset_at as u32 + 1);
    word(&mut code, 15, cfg.flash_base + 2 * handler as u32 + 1);
    code
}

#[test]
fn systick_toggles_the_led_at_the_programmed_period() {
    let cfg = Config::rp2040();
    let mut cpu = Thumb::new(cfg);
    // 1 ms at 125 MHz.
    cpu.load_halfwords(0, &blinky(125_000 - 1)).unwrap();
    cpu.reset().unwrap();
    let end = cpu.cycle_of_ms(10.5);
    assert_eq!(cpu.run_until(end).unwrap(), Stop::Reached);
    let ev = cpu.pin_events();
    assert_eq!(ev.len(), 10, "{ev:?}");
    assert!(ev.iter().all(|e| e.pin == 25));
    for (i, e) in ev.iter().enumerate() {
        assert_eq!(e.level, i % 2 == 0);
    }
    for w in ev.windows(2) {
        assert_eq!(w[1].cycle - w[0].cycle, 125_000, "the period is exact");
    }
    // The first toggle is one period after the start, plus start-up and entry latency.
    let first_ms = cpu.ms(ev[0].cycle);
    assert!((1.0..1.001).contains(&first_ms), "{first_ms}");
}

#[test]
fn primask_holds_the_tick_until_interrupts_are_enabled() {
    let cfg = Config::rp2040();
    let mut a = Asm::default();
    a.cpsid()
        .ldr_const(2, SYST_CSR)
        .ldr_const(3, 99)
        .str_(3, 2, 1)
        .movs(3, 7)
        .str_(3, 2, 0)
        .ldr_const(0, 1000);
    let spin = a.here();
    a.subs_i(0, 1).bcond_to(1, spin).bkpt(1);
    let (cpu, stop) = {
        let mut cpu = Thumb::new(cfg);
        cpu.load_halfwords(0, &a.finish(0)).unwrap();
        cpu.set_pc(cfg.flash_base);
        let s = cpu.run_until(100_000).unwrap();
        (cpu, s)
    };
    // The vector table is empty, so taking the interrupt would jump to address 0 and
    // fault; reaching the breakpoint shows it was held.
    assert_eq!(stop, Stop::Breakpoint(1));
    assert_eq!(cpu.reg(0), 0);
}

#[test]
fn an_input_pin_is_copied_to_the_led() {
    let cfg = Config::rp2040();
    let mut a = Asm::default();
    a.ldr_const(0, SIO)
        .ldr_const(1, 1 << 25)
        .str_(1, 0, 0x24 / 4);
    let top = a.here();
    a.ldr(2, 0, 1) // GPIO_IN
        .lsls(2, 2, 25) // GPIO 0 -> bit 25 (other inputs are low)
        .str_(2, 0, 0x10 / 4) // GPIO_OUT
        .b_to(top);
    let mut cpu = Thumb::new(cfg);
    cpu.load_halfwords(0, &a.finish(0)).unwrap();
    cpu.set_pc(cfg.flash_base);
    for (ms, level) in [(1.0, true), (2.0, false), (3.0, true)] {
        let c = cpu.cycle_of_ms(ms);
        cpu.schedule_input(c, 0, level);
    }
    cpu.run_until(cpu.cycle_of_ms(4.0)).unwrap();
    let ev: Vec<(u64, bool)> = cpu
        .pin_events()
        .iter()
        .map(|e| (e.cycle, e.level))
        .collect();
    assert_eq!(ev.len(), 3, "{ev:?}");
    for ((cycle, level), (ms, want)) in ev.iter().zip([(1.0, true), (2.0, false), (3.0, true)]) {
        let lag = cycle - cpu.cycle_of_ms(ms);
        assert_eq!(*level, want);
        assert!(
            lag <= 10,
            "the loop is 7 cycles, so the LED follows within 10 ({lag})"
        );
    }
}

#[test]
fn faults_are_reported_with_the_address() {
    let mut a = Asm::default();
    a.ldr_const(0, 0x2000_0001).ldr(1, 0, 0).bkpt(0);
    let cfg = Config::rp2040();
    let mut cpu = Thumb::new(cfg);
    cpu.load_halfwords(0, &a.finish(0)).unwrap();
    cpu.set_pc(cfg.flash_base);
    assert!(matches!(
        cpu.run_until(1000),
        Err(SimError::Unaligned {
            addr: 0x2000_0001,
            ..
        })
    ));

    let mut a = Asm::default();
    a.ldr_const(0, cfg.flash_base).str_(0, 0, 0).bkpt(0);
    let mut cpu = Thumb::new(cfg);
    cpu.load_halfwords(0, &a.finish(0)).unwrap();
    cpu.set_pc(cfg.flash_base);
    assert!(matches!(
        cpu.run_until(1000),
        Err(SimError::ReadOnly { .. })
    ));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, failure_persistence: None, ..ProptestConfig::default() })]

    /// The flags of ADDS, SUBS and CMP agree with a reference computed in 64 bits.
    #[test]
    fn add_and_subtract_flags_match_the_reference(x in any::<u32>(), y in any::<u32>()) {
        for sub in [false, true] {
            let mut a = Asm::default();
            a.ldr_const(0, x).ldr_const(1, y);
            if sub { a.subs(2, 0, 1); } else { a.adds(2, 0, 1); }
            a.bkpt(0);
            let (cpu, _) = run(&a.finish(0), 1000);
            let (want, c, v) = if sub {
                let r = x.wrapping_sub(y);
                (r, x >= y, ((x as i32) as i64 - (y as i32) as i64) != i64::from(r as i32))
            } else {
                let r = x.wrapping_add(y);
                (r, u64::from(x) + u64::from(y) > u64::from(u32::MAX), ((x as i32) as i64 + (y as i32) as i64) != i64::from(r as i32))
            };
            prop_assert_eq!(cpu.reg(2), want);
            prop_assert_eq!(cpu.flags(), (want >> 31 == 1, want == 0, c, v));
        }
    }
}
