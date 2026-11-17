//! MIR to AVR machine code for the ATmega328P.
//!
//! The generated program reproduces the scheduling rules of the C runtime
//! (`runtime/neuron-rt/src/nrt_core.c`): a 1 ms tick on timer 0 in CTC mode; in each
//! millisecond the signal commits, then the releases, then the ready activities by priority,
//! run to completion; `fsm` blocks are periodic activities of priority 1; a fault enters the
//! safe state and stops every activity.
//!
//! Register use: `r24:r25` is the accumulator, `r22:r23` the second operand, `r16`..`r21`
//! are scratch, `r20` carries a fault code, `r1` is always zero, `r0` is a temporary. Every
//! variable lives at a fixed SRAM address (there is no recursion, so no stack frames). The
//! stack holds return addresses and the left operands of binary operations.

use crate::asm::{Flag, Op};
use neuron_mir::{
    BinOp, Edge, Expr, Fsm, IoK, Place, Program, SigAccess, Stmt, StmtKind, Ty, UnOp,
};
use neuron_target::Diag;
use std::collections::BTreeMap;

// I/O addresses (I/O space, for IN/OUT/SBI) and data addresses (for LDS/STS) of the ATmega328P.
const SREG: u8 = 0x3f;
const SPL: u8 = 0x3d;
const SPH: u8 = 0x3e;
const SMCR: u8 = 0x33;
const EIFR: u8 = 0x1c;
const EIMSK: u8 = 0x1d;
const TCCR0A: u8 = 0x24;
const TCCR0B: u8 = 0x25;
const OCR0A: u8 = 0x27;
const TIMSK0_DATA: u16 = 0x6e;
const EICRA_DATA: u16 = 0x69;
const RAMEND: u16 = 0x08ff;
const SRAM_START: u16 = 0x0100;

const F_OVERFLOW: u8 = 1;
const F_DIVZERO: u8 = 2;
const F_LOOP: u8 = 5;
const F_DEADLINE: u8 = 6;
const F_USER: u8 = 16;

/// `(PIN, DDR, PORT)` I/O addresses of a port letter.
fn port_regs(port: char) -> Option<(u8, u8, u8)> {
    match port {
        'B' => Some((0x03, 0x04, 0x05)),
        'C' => Some((0x06, 0x07, 0x08)),
        'D' => Some((0x09, 0x0a, 0x0b)),
        _ => None,
    }
}

fn size(t: Ty) -> u16 {
    match t {
        Ty::Bool | Ty::U8 | Ty::I8 => 1,
        Ty::U16 | Ty::I16 => 2,
        Ty::U32 | Ty::I32 | Ty::F32 => 4,
    }
}

fn signed(t: Ty) -> bool {
    matches!(t, Ty::I8 | Ty::I16)
}

/// One schedulable activity: a task or a state machine.
struct Sched {
    label: String,
    task: Option<String>,
    period: u16,
    offset: u16,
    prio: u32,
    deadline: u16,
    reads: Vec<String>,
    writes: Vec<String>,
    // SRAM
    next: u16,
    pending: u16,
    release: u16,
    commit_at: u16,
    commit_pending: u16,
}

/// The state of one activity being compiled (for the trap path).
struct Act {
    name: String,
    isr: bool,
    sp_slot: u16,
    task: Option<String>,
}

pub(crate) struct Gen<'a> {
    p: &'a Program,
    pub(crate) ops: Vec<Op>,
    pub(crate) diags: Vec<Diag>,
    next_ram: u16,
    vars: BTreeMap<String, (u16, Ty)>,
    next_buf: BTreeMap<String, u16>,
    dirty: BTreeMap<String, u16>,
    snaps: BTreeMap<(String, String), u16>,
    shared: BTreeMap<String, bool>,
    labels: u32,
    act: Option<Act>,
    loops: Vec<(String, String)>,
    // runtime state
    ms: u16,
    now: u16,
    last: u16,
    faulted: u16,
    fault_code: u16,
    scheds: Vec<Sched>,
}

impl<'a> Gen<'a> {
    pub(crate) fn new(p: &'a Program) -> Gen<'a> {
        Gen {
            p,
            ops: Vec::new(),
            diags: Vec::new(),
            next_ram: SRAM_START,
            vars: BTreeMap::new(),
            next_buf: BTreeMap::new(),
            dirty: BTreeMap::new(),
            snaps: BTreeMap::new(),
            shared: BTreeMap::new(),
            labels: 0,
            act: None,
            loops: Vec::new(),
            ms: 0,
            now: 0,
            last: 0,
            faulted: 0,
            fault_code: 0,
            scheds: Vec::new(),
        }
    }

    fn unsupported(&mut self, what: &str) {
        let file = self.p.files.first().cloned().unwrap_or_default();
        let message = format!("{what} is not supported by the native AVR backend yet");
        if self.diags.iter().any(|d| d.message == message) {
            return;
        }
        self.diags.push(Diag {
            code: "NE0610",
            message,
            help: Some("build with the C backend (`neuron build` without `--native`); the native subset is in docs/native-backend.md".into()),
            file,
            span: [0, 0],
        });
    }

    fn alloc(&mut self, bytes: u16) -> u16 {
        let a = self.next_ram;
        self.next_ram += bytes;
        a
    }

    fn label(&mut self, hint: &str) -> String {
        self.labels += 1;
        format!(".{hint}{}", self.labels)
    }

    fn o(&mut self, op: Op) {
        self.ops.push(op);
    }

    fn at(&mut self, l: &str) {
        self.ops.push(Op::Label(l.to_string()));
    }

    /// Jumps to `target` when the flag has the given state, at any distance.
    fn jump_if(&mut self, flag: Flag, set: bool, target: &str) {
        let skip = self.label("s");
        self.o(Op::Br(flag, !set, skip.clone()));
        self.o(Op::Jmp(target.to_string()));
        self.at(&skip);
    }

    fn ldi16(&mut self, lo_reg: u8, v: u16) {
        self.o(Op::Ldi(lo_reg, v as u8));
        self.o(Op::Ldi(lo_reg + 1, (v >> 8) as u8));
    }

    fn load16(&mut self, reg: u8, addr: u16) {
        self.o(Op::Lds(reg, addr));
        self.o(Op::Lds(reg + 1, addr + 1));
    }

    fn store16(&mut self, addr: u16, reg: u8) {
        self.o(Op::Sts(addr, reg));
        self.o(Op::Sts(addr + 1, reg + 1));
    }

    /// Makes `r25` the extension of `r24` for an 8-bit type.
    fn normalize(&mut self, t: Ty) {
        match t {
            Ty::Bool | Ty::U8 => self.o(Op::Ldi(25, 0)),
            Ty::I8 => {
                self.o(Op::Mov(25, 24));
                self.o(Op::Add(25, 25));
                self.o(Op::Sbc(25, 25));
            }
            _ => {}
        }
    }

    // ---- variables ---------------------------------------------------------------------------

    fn layout(&mut self) {
        let p = self.p;
        self.ms = self.alloc(2);
        self.now = self.alloc(2);
        self.last = self.alloc(2);
        self.faulted = self.alloc(1);
        self.fault_code = self.alloc(1);
        for g in &p.globals {
            if !matches!(size(g.ty), 1 | 2) {
                self.unsupported(&format!(
                    "the type `{}` (of `{}`)",
                    g.ty.name(),
                    g.source_name
                ));
                continue;
            }
            let a = self.alloc(size(g.ty));
            self.vars.insert(g.name.clone(), (a, g.ty));
            self.shared.insert(g.name.clone(), g.shared);
            if g.signal {
                let n = self.alloc(size(g.ty));
                self.next_buf.insert(g.name.clone(), n);
                let d = self.alloc(1);
                self.dirty.insert(g.name.clone(), d);
            }
        }
        for t in &p.tasks {
            if t.event.is_some() || t.period_ms == 0 {
                self.unsupported("a task released by an event");
            }
            if t.period_ms > 30_000 || t.offset_ms > 30_000 || t.deadline_ms > 30_000 {
                self.unsupported("a period, offset or deadline above 30 s");
            }
            for s in &t.reads {
                if let Some((_, ty)) = self.vars.get(s).copied() {
                    let a = self.alloc(size(ty));
                    self.snaps.insert((t.name.clone(), s.clone()), a);
                }
            }
            let s = Sched {
                label: format!("act_{}", t.name),
                task: Some(t.name.clone()),
                period: t.period_ms as u16,
                offset: t.offset_ms as u16,
                prio: t.prio,
                deadline: t.deadline_ms as u16,
                reads: t.reads.clone(),
                writes: t.writes.clone(),
                next: self.alloc(2),
                pending: self.alloc(1),
                release: self.alloc(2),
                commit_at: self.alloc(2),
                commit_pending: self.alloc(1),
            };
            self.scheds.push(s);
        }
        for (k, f) in p.fsms.iter().enumerate() {
            let s = Sched {
                label: format!("fsm{k}"),
                task: None,
                period: f.period_ms as u16,
                offset: 0,
                prio: 1,
                deadline: 0,
                reads: Vec::new(),
                writes: Vec::new(),
                next: self.alloc(2),
                pending: self.alloc(1),
                release: self.alloc(2),
                commit_at: self.alloc(2),
                commit_pending: self.alloc(1),
            };
            self.scheds.push(s);
        }
        if !p.rules.is_empty() {
            self.unsupported("a `when` rule");
        }
        if !p.safety.rules.is_empty() || p.safety.reset.is_some() {
            self.unsupported("safety rules and `reset when`");
        }
        if p.safety.watchdog_ms.is_some() {
            self.unsupported("the watchdog");
        }
        if !p.safety.on_fault.is_empty() {
            self.unsupported("`on fault`");
        }
        if !p.funcs.is_empty() {
            self.unsupported("user functions");
        }
        for io in &p.ios {
            if !matches!(io.kind, IoK::GpioIn | IoK::GpioOut) {
                self.unsupported("pwm, adc and uart I/O");
            }
        }
    }

    fn local(&mut self, name: &str, t: Ty) -> u16 {
        let key = format!(
            "{}::{name}",
            self.act.as_ref().map_or("", |a| a.name.as_str())
        );
        if let Some((a, _)) = self.vars.get(&key) {
            return *a;
        }
        let a = self.alloc(size(t).max(1));
        self.vars.insert(key, (a, t));
        a
    }

    fn local_addr(&self, name: &str) -> Option<(u16, Ty)> {
        let key = format!(
            "{}::{name}",
            self.act.as_ref().map_or("", |a| a.name.as_str())
        );
        self.vars.get(&key).copied()
    }

    fn in_isr(&self) -> bool {
        self.act.as_ref().is_some_and(|a| a.isr)
    }

    /// Loads a variable into the accumulator.
    fn load_var(&mut self, addr: u16, t: Ty, atomic: bool) {
        let atomic = atomic && size(t) > 1 && !self.in_isr();
        if atomic {
            self.o(Op::In(0, SREG));
            self.o(Op::Cli);
        }
        if size(t) == 1 {
            self.o(Op::Lds(24, addr));
            self.normalize(t);
        } else {
            self.load16(24, addr);
        }
        if atomic {
            self.o(Op::Out(SREG, 0));
        }
    }

    fn store_var(&mut self, addr: u16, t: Ty) {
        self.o(Op::Sts(addr, 24));
        if size(t) > 1 {
            self.o(Op::Sts(addr + 1, 25));
        }
    }

    // ---- traps ----------------------------------------------------------------------------------

    fn trap_label(&self) -> String {
        format!(
            "{}_trap",
            self.act.as_ref().map_or("none", |a| a.name.as_str())
        )
    }

    /// Faults with `code` when the flag has the given state.
    fn trap_if(&mut self, flag: Flag, set: bool, code: u8) {
        let ok = self.label("ok");
        self.o(Op::Br(flag, !set, ok.clone()));
        self.o(Op::Ldi(20, code));
        let t = self.trap_label();
        self.o(Op::Jmp(t));
        self.at(&ok);
    }

    // ---- expressions ------------------------------------------------------------------------------

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Int(v, t) => {
                if !matches!(size(*t), 1 | 2) {
                    self.unsupported(&format!("the type `{}`", t.name()));
                }
                self.ldi16(24, *v as u16);
            }
            Expr::Bool(b) => self.ldi16(24, u16::from(*b)),
            Expr::Global(name, t) => match self.vars.get(name).copied() {
                Some((a, _)) => {
                    let sh = self.shared.get(name).copied().unwrap_or(false);
                    self.load_var(a, *t, sh);
                }
                None => self.unsupported(&format!("the variable `{name}`")),
            },
            Expr::Local(name, t) => match self.local_addr(name) {
                Some((a, _)) => self.load_var(a, *t, false),
                None => self.unsupported(&format!("the local `{name}`")),
            },
            Expr::Signal(name, t, acc) => {
                let task = self.act.as_ref().and_then(|a| a.task.clone());
                let addr = match (acc, task) {
                    (SigAccess::Snapshot, Some(task)) => {
                        self.snaps.get(&(task, name.clone())).copied()
                    }
                    _ => self.vars.get(name).map(|(a, _)| *a),
                };
                match addr {
                    Some(a) => {
                        let sh = self.shared.get(name).copied().unwrap_or(false);
                        self.load_var(a, *t, sh);
                    }
                    None => self.unsupported(&format!("the signal `{name}`")),
                }
            }
            Expr::Unary(op, x, t) => {
                self.expr(x);
                match op {
                    UnOp::Not => {
                        self.o(Op::Ldi(16, 1));
                        self.o(Op::Eor(24, 16));
                    }
                    UnOp::BitNot => {
                        self.o(Op::Com(24));
                        self.o(Op::Com(25));
                        self.normalize(*t);
                    }
                    UnOp::Neg => {
                        // 0 - x, checked.
                        self.o(Op::Movw(22, 24));
                        self.o(Op::Ldi(24, 0));
                        self.o(Op::Ldi(25, 0));
                        self.arith(BinOp::Sub, *t);
                    }
                }
            }
            Expr::Binary(op, a, b, t) => {
                self.expr(a);
                self.o(Op::Push(24));
                self.o(Op::Push(25));
                self.expr(b);
                self.o(Op::Movw(22, 24));
                self.o(Op::Pop(25));
                self.o(Op::Pop(24));
                if op.is_compare() {
                    self.compare(*op, a.ty());
                } else {
                    self.arith(*op, *t);
                }
            }
            Expr::Cast(x, t) => {
                self.expr(x);
                if !matches!(size(*t), 1 | 2) {
                    self.unsupported(&format!("a conversion to `{}`", t.name()));
                }
                self.normalize(*t);
            }
            Expr::IfElse(c, a, b, _) => {
                let (other, end) = (self.label("else"), self.label("fi"));
                self.expr(c);
                self.o(Op::And(24, 24));
                self.jump_if(Flag::Z, true, &other);
                self.expr(a);
                self.o(Op::Jmp(end.clone()));
                self.at(&other);
                self.expr(b);
                self.at(&end);
            }
            Expr::IoGet(io, _) => match self
                .p
                .ios
                .get(*io)
                .and_then(|d| Some((port_regs(d.port?)?, d.bit?)))
            {
                Some(((pin, _, _), bit)) => {
                    self.o(Op::Ldi(24, 0));
                    self.o(Op::Ldi(25, 0));
                    self.o(Op::Sbic(pin, bit));
                    self.o(Op::Ldi(24, 1));
                }
                None => self.unsupported("this I/O read"),
            },
            Expr::Float(_) | Expr::Scale(_) => self.unsupported("floating point"),
            Expr::Call(..) => self.unsupported("calls of user functions"),
            Expr::Builtin(name, ..) => self.unsupported(&format!("the library function `{name}`")),
        }
    }

    /// `r24:r25 = r24:r25 op r22:r23` for a result of type `t`.
    fn arith(&mut self, op: BinOp, t: Ty) {
        if !matches!(size(t), 1 | 2) {
            self.unsupported(&format!("arithmetic on `{}`", t.name()));
            return;
        }
        let narrow = size(t) == 1;
        match op {
            BinOp::Add | BinOp::Sub => {
                if op == BinOp::Add {
                    self.o(Op::Add(24, 22));
                    self.o(Op::Adc(25, 23));
                } else {
                    self.o(Op::Sub(24, 22));
                    self.o(Op::Sbc(25, 23));
                }
                match t {
                    Ty::U16 => self.trap_if(Flag::C, true, F_OVERFLOW),
                    Ty::I16 => self.trap_if(Flag::V, true, F_OVERFLOW),
                    _ => self.check_fit8(t),
                }
            }
            BinOp::AddW | BinOp::SubW => {
                if op == BinOp::AddW {
                    self.o(Op::Add(24, 22));
                    self.o(Op::Adc(25, 23));
                } else {
                    self.o(Op::Sub(24, 22));
                    self.o(Op::Sbc(25, 23));
                }
                self.normalize(t);
            }
            BinOp::BitAnd | BinOp::And => {
                self.o(Op::And(24, 22));
                self.o(Op::And(25, 23));
            }
            BinOp::BitOr | BinOp::Or => {
                self.o(Op::Or(24, 22));
                self.o(Op::Or(25, 23));
            }
            BinOp::BitXor => {
                self.o(Op::Eor(24, 22));
                self.o(Op::Eor(25, 23));
                self.normalize(t);
            }
            BinOp::Div | BinOp::Rem => {
                if signed(t) {
                    self.unsupported("signed division");
                    return;
                }
                self.o(Op::Mov(16, 22));
                self.o(Op::Or(16, 23));
                self.trap_if(Flag::Z, true, F_DIVZERO);
                self.o(Op::Call("rt_udivmod16".into()));
                if op == BinOp::Rem {
                    self.o(Op::Movw(24, 22));
                }
                let _ = narrow;
            }
            BinOp::Shl | BinOp::Shr => {
                // The amount is in r22; shift one bit per iteration.
                let (top, done) = (self.label("sh"), self.label("shd"));
                self.at(&top);
                self.o(Op::And(22, 22));
                self.o(Op::Br(Flag::Z, true, done.clone()));
                if op == BinOp::Shl {
                    self.o(Op::Add(24, 24));
                    self.o(Op::Adc(25, 25));
                } else if signed(t) {
                    self.o(Op::Asr(25));
                    self.o(Op::Ror(24));
                } else {
                    self.o(Op::Lsr(25));
                    self.o(Op::Ror(24));
                }
                self.o(Op::Dec(22));
                self.o(Op::Rjmp(top));
                self.at(&done);
                self.normalize(t);
            }
            BinOp::Mul | BinOp::MulW => self.unsupported("multiplication"),
            _ => self.unsupported("this operator"),
        }
    }

    /// Faults when the 16-bit result in `r24:r25` does not fit the 8-bit type.
    fn check_fit8(&mut self, t: Ty) {
        match t {
            Ty::U8 | Ty::Bool => {
                self.o(Op::And(25, 25));
                self.trap_if(Flag::Z, false, F_OVERFLOW);
            }
            Ty::I8 => {
                self.o(Op::Mov(16, 24));
                self.o(Op::Add(16, 16));
                self.o(Op::Sbc(16, 16));
                self.o(Op::Cp(16, 25));
                self.trap_if(Flag::Z, false, F_OVERFLOW);
            }
            _ => {}
        }
    }

    /// A comparison of `r24:r25` with `r22:r23` as values of type `operand`; the result is 0 or 1.
    fn compare(&mut self, op: BinOp, operand: Ty) {
        let sgn = signed(operand);
        // Order the operands so that the test is `a < b`, `a >= b`, `a == b` or `a != b`.
        let (swap, flag, set) = match op {
            BinOp::Eq => (false, Flag::Z, true),
            BinOp::Ne => (false, Flag::Z, false),
            BinOp::Lt => (false, if sgn { Flag::S } else { Flag::C }, true),
            BinOp::Ge => (false, if sgn { Flag::S } else { Flag::C }, false),
            BinOp::Gt => (true, if sgn { Flag::S } else { Flag::C }, true),
            _ => (true, if sgn { Flag::S } else { Flag::C }, false), // Le
        };
        if swap {
            self.o(Op::Cp(22, 24));
            self.o(Op::Cpc(23, 25));
        } else {
            self.o(Op::Cp(24, 22));
            self.o(Op::Cpc(25, 23));
        }
        let yes = self.label("t");
        self.o(Op::Ldi(24, 1));
        self.o(Op::Br(flag, set, yes.clone()));
        self.o(Op::Ldi(24, 0));
        self.at(&yes);
        self.o(Op::Ldi(25, 0));
    }

    // ---- statements ------------------------------------------------------------------------------

    fn block(&mut self, b: &[Stmt]) {
        for s in b {
            self.stmt(s);
        }
    }

    fn place_shared(&self, p: &Place) -> bool {
        match p {
            Place::Global(n) => self.shared.get(n).copied().unwrap_or(false),
            Place::Signal { name, .. } => self.shared.get(name).copied().unwrap_or(false),
            Place::Local(_) => false,
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        let line = s.loc.line;
        self.o(Op::Note(format!("line {line}")));
        match &s.kind {
            StmtKind::Let { name, ty, value } => {
                let a = self.local(name, *ty);
                self.expr(value);
                self.store_var(a, *ty);
            }
            StmtKind::Assign { place, value } => {
                // A statement that touches data shared with an interrupt runs with interrupts off.
                let atomic = self.place_shared(place) && !self.in_isr();
                if atomic {
                    self.o(Op::Cli);
                }
                self.expr(value);
                match place {
                    Place::Local(n) => {
                        let t = value.ty();
                        let a = self.local(n, t);
                        self.store_var(a, t);
                    }
                    Place::Global(n)
                    | Place::Signal {
                        name: n,
                        deferred: false,
                    } => match self.vars.get(n).copied() {
                        Some((a, t)) => self.store_var(a, t),
                        None => self.unsupported(&format!("the variable `{n}`")),
                    },
                    Place::Signal {
                        name,
                        deferred: true,
                    } => {
                        match (
                            self.next_buf.get(name).copied(),
                            self.dirty.get(name).copied(),
                            self.vars.get(name).copied(),
                        ) {
                            (Some(nx), Some(d), Some((_, t))) => {
                                self.store_var(nx, t);
                                self.o(Op::Ldi(16, 1));
                                self.o(Op::Sts(d, 16));
                            }
                            _ => self.unsupported(&format!("the signal `{name}`")),
                        }
                    }
                }
                if atomic {
                    self.o(Op::Sei);
                }
            }
            StmtKind::IoPut { io, value } => {
                let Some(((_, _, port), bit)) = self
                    .p
                    .ios
                    .get(*io)
                    .and_then(|d| Some((port_regs(d.port?)?, d.bit?)))
                else {
                    self.unsupported("this output");
                    return;
                };
                self.expr(value);
                let (lo, end) = (self.label("lo"), self.label("pe"));
                self.o(Op::And(24, 24));
                self.o(Op::Br(Flag::Z, true, lo.clone()));
                self.o(Op::Sbi(port, bit));
                self.o(Op::Rjmp(end.clone()));
                self.at(&lo);
                self.o(Op::Cbi(port, bit));
                self.at(&end);
            }
            StmtKind::IoFlip { io } => {
                let Some(((_, _, port), bit)) = self
                    .p
                    .ios
                    .get(*io)
                    .and_then(|d| Some((port_regs(d.port?)?, d.bit?)))
                else {
                    self.unsupported("this output");
                    return;
                };
                let isr = self.in_isr();
                if !isr {
                    self.o(Op::Cli);
                }
                self.o(Op::In(24, port));
                self.o(Op::Ldi(16, 1 << bit));
                self.o(Op::Eor(24, 16));
                self.o(Op::Out(port, 24));
                if !isr {
                    self.o(Op::Sei);
                }
            }
            StmtKind::If {
                cond,
                then,
                otherwise,
            } => {
                let (other, end) = (self.label("else"), self.label("endif"));
                self.expr(cond);
                self.o(Op::And(24, 24));
                self.jump_if(Flag::Z, true, &other);
                self.block(then);
                self.o(Op::Jmp(end.clone()));
                self.at(&other);
                self.block(otherwise);
                self.at(&end);
            }
            StmtKind::For {
                var,
                ty,
                lo,
                hi,
                body,
            } => {
                let a = self.local(var, *ty);
                let (top, cont, end) =
                    (self.label("for"), self.label("next"), self.label("endfor"));
                self.ldi16(24, *lo as u16);
                self.store_var(a, *ty);
                self.at(&top);
                self.load_var(a, *ty, false);
                self.ldi16(22, *hi as u16);
                self.o(Op::Cp(24, 22));
                self.o(Op::Cpc(25, 23));
                let flag = if signed(*ty) { Flag::S } else { Flag::C };
                self.jump_if(flag, false, &end);
                self.loops.push((cont.clone(), end.clone()));
                self.block(body);
                self.loops.pop();
                self.at(&cont);
                self.load_var(a, *ty, false);
                self.o(Op::Subi(24, 0xff));
                self.o(Op::Sbci(25, 0xff));
                self.store_var(a, *ty);
                self.o(Op::Jmp(top));
                self.at(&end);
            }
            StmtKind::While { cond, max, body } => {
                let n = self.local(&format!("while{}", self.labels), Ty::U16);
                let (top, end) = (self.label("while"), self.label("endwhile"));
                self.ldi16(24, 0);
                self.store16(n, 24);
                self.at(&top);
                self.expr(cond);
                self.o(Op::And(24, 24));
                self.jump_if(Flag::Z, true, &end);
                self.load16(24, n);
                self.ldi16(22, (*max).clamp(0, 65535) as u16);
                self.o(Op::Cp(24, 22));
                self.o(Op::Cpc(25, 23));
                self.trap_if(Flag::C, false, F_LOOP);
                self.o(Op::Subi(24, 0xff));
                self.o(Op::Sbci(25, 0xff));
                self.store16(n, 24);
                self.loops.push((top.clone(), end.clone()));
                self.block(body);
                self.loops.pop();
                self.o(Op::Jmp(top));
                self.at(&end);
            }
            StmtKind::Break => match self.loops.last().cloned() {
                Some((_, end)) => self.o(Op::Jmp(end)),
                None => self.unsupported("`break` outside a loop"),
            },
            StmtKind::Continue => match self.loops.last().cloned() {
                Some((cont, _)) => self.o(Op::Jmp(cont)),
                None => self.unsupported("`continue` outside a loop"),
            },
            StmtKind::Return(None) => {
                let epi = format!(
                    "{}_end",
                    self.act.as_ref().map_or("none", |a| a.name.as_str())
                );
                self.o(Op::Jmp(epi));
            }
            StmtKind::Return(Some(_)) => self.unsupported("`return` with a value"),
            StmtKind::Fault(name) => {
                let code = self
                    .p
                    .faults
                    .iter()
                    .position(|f| f == name)
                    .map_or(F_OVERFLOW, |i| F_USER + i as u8);
                let code = match name.as_str() {
                    "Overflow" => F_OVERFLOW,
                    "DivideByZero" => F_DIVZERO,
                    "LoopBound" => F_LOOP,
                    "DeadlineMiss" => F_DEADLINE,
                    _ => code,
                };
                self.o(Op::Ldi(20, code));
                let t = self.trap_label();
                self.o(Op::Jmp(t));
            }
            StmtKind::Emit(_) => self.unsupported("`emit`"),
            StmtKind::IoPwm { .. } => self.unsupported("pwm output"),
            StmtKind::Print { .. } => self.unsupported("uart print"),
            StmtKind::CallVoid { .. } => self.unsupported("calls of user functions"),
        }
    }

    // ---- activities ---------------------------------------------------------------------------------

    /// Emits a subroutine with the trap path of an activity.
    fn activity(
        &mut self,
        name: &str,
        isr: bool,
        task: Option<String>,
        body: impl FnOnce(&mut Gen<'a>),
    ) {
        let slot = self.alloc(2);
        self.act = Some(Act {
            name: name.to_string(),
            isr,
            sp_slot: slot,
            task,
        });
        self.at(name);
        self.o(Op::In(0, SPL));
        self.o(Op::Sts(slot, 0));
        self.o(Op::In(0, SPH));
        self.o(Op::Sts(slot + 1, 0));
        body(self);
        self.at(&format!("{name}_end"));
        self.o(Op::Ret);
        // Trap path: fault with the code in r20, drop what the activity pushed, and leave.
        self.at(&format!("{name}_trap"));
        self.o(Op::Call("rt_fault".into()));
        if !isr {
            self.o(Op::Cli);
        }
        self.o(Op::Lds(0, slot));
        self.o(Op::Out(SPL, 0));
        self.o(Op::Lds(0, slot + 1));
        self.o(Op::Out(SPH, 0));
        if !isr {
            self.o(Op::Sei);
        }
        self.o(Op::Jmp(format!("{name}_end")));
        let _ = self.act.as_ref().map(|a| a.sp_slot);
        self.act = None;
    }

    fn fsm_code(&mut self, k: usize, f: &Fsm) {
        let st = self.alloc(1);
        let on = self.alloc(1);
        let t0 = self.alloc(2);
        let name = format!("fsm{k}");
        let now = self.now;
        let n_states = f.states.len();
        self.activity(&name, false, None, |g| {
            let started = g.label("on");
            g.o(Op::Lds(16, on));
            g.o(Op::And(16, 16));
            g.o(Op::Br(Flag::Z, false, started.clone()));
            g.o(Op::Ldi(16, 1));
            g.o(Op::Sts(on, 16));
            g.enter_state(f, f.initial, st, t0);
            g.o(Op::Jmp(format!("{name}_end")));
            g.at(&started);
            for (s, state) in f.states.iter().enumerate() {
                let skip = g.label("st");
                g.o(Op::Lds(16, st));
                g.o(Op::Cpi(16, s as u8));
                g.jump_if(Flag::Z, false, &skip);
                g.block(&state.during);
                for t in &state.trans {
                    let no = g.label("nofire");
                    let fire = g.label("fire");
                    if let Some(w) = &t.when {
                        g.expr(w);
                        g.o(Op::And(24, 24));
                        g.jump_if(Flag::Z, false, &fire);
                    }
                    match t.after_ms {
                        Some(a) => {
                            g.load16(24, now);
                            g.load16(22, t0);
                            g.o(Op::Sub(24, 22));
                            g.o(Op::Sbc(25, 23));
                            g.ldi16(22, a.min(65535) as u16);
                            g.o(Op::Cp(24, 22));
                            g.o(Op::Cpc(25, 23));
                            g.jump_if(Flag::C, true, &no);
                        }
                        None if t.when.is_some() => g.o(Op::Jmp(no.clone())),
                        None => {}
                    }
                    g.at(&fire);
                    g.block(&state.exit);
                    g.block(&t.action);
                    g.enter_state(f, t.target, st, t0);
                    g.o(Op::Jmp(format!("{name}_end")));
                    g.at(&no);
                }
                g.o(Op::Jmp(format!("{name}_end")));
                g.at(&skip);
            }
            let _ = n_states;
        });
    }

    fn enter_state(&mut self, f: &Fsm, s: usize, st: u16, t0: u16) {
        self.load16(24, self.now);
        self.store16(t0, 24);
        self.o(Op::Ldi(16, s as u8));
        self.o(Op::Sts(st, 16));
        if let Some(state) = f.states.get(s) {
            self.block(&state.entry);
        }
    }

    // ---- the whole program ------------------------------------------------------------------------

    pub(crate) fn program(&mut self) {
        let p = self.p;
        self.layout();
        // Vector table: reset, INT0, INT1, then TIMER0_COMPA at word 0x1c.
        self.o(Op::Jmp("reset".into()));
        let mut ints: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
        for (i, irq) in p.irqs.iter().enumerate() {
            match p.ios.get(irq.io).map(|d| d.aux) {
                Some(n @ (0 | 1)) => ints.entry(n).or_default().push(i),
                _ => self.unsupported("an interrupt on a pin other than INT0 or INT1"),
            }
        }
        for n in 0..2u8 {
            if ints.contains_key(&n) {
                self.o(Op::Jmp(format!("isr_int{n}")));
            } else {
                self.o(Op::Jmp("bad_irq".into()));
            }
        }
        for _ in 3..14 {
            self.o(Op::Jmp("bad_irq".into()));
        }
        self.o(Op::Jmp("isr_tick".into()));
        for _ in 15..26 {
            self.o(Op::Jmp("bad_irq".into()));
        }
        self.at("bad_irq");
        self.o(Op::Reti);
        let head = std::mem::take(&mut self.ops);

        // Everything else first, so that the start-up code knows all the SRAM to clear.
        self.runtime();

        // Activities.
        for t in &p.tasks {
            let label = format!("act_{}", t.name);
            self.activity(&label, false, Some(t.name.clone()), |g| g.block(&t.body));
        }
        for (k, f) in p.fsms.iter().enumerate() {
            self.fsm_code(k, f);
        }
        for (i, irq) in p.irqs.iter().enumerate() {
            self.activity(&format!("irq{i}"), true, None, |g| g.block(&irq.body));
        }
        self.activity("safe_state", true, None, |g| g.block(&p.safety.safe_state));
        self.isrs(&ints);
        let body = std::mem::take(&mut self.ops);
        self.reset(&ints);
        let start = std::mem::take(&mut self.ops);
        self.ops = head;
        self.ops.extend(start);
        self.ops.extend(body);
    }

    fn reset(&mut self, ints: &BTreeMap<u8, Vec<usize>>) {
        let p = self.p;
        self.at("reset");
        self.o(Op::Eor(1, 1));
        self.o(Op::Out(SREG, 1));
        self.ldi16(28, RAMEND);
        self.o(Op::Out(SPH, 29));
        self.o(Op::Out(SPL, 28));
        // Zero the SRAM that the program uses.
        let used = self.next_ram;
        for a in SRAM_START..used.max(SRAM_START + 1) {
            self.o(Op::Sts(a, 1));
        }
        // Initial values.
        for g in &p.globals {
            if let (Some(init), Some((a, t))) = (&g.init, self.vars.get(&g.name).copied()) {
                let v = match init {
                    Expr::Int(v, _) => *v as u16,
                    Expr::Bool(b) => u16::from(*b),
                    _ => {
                        self.unsupported("a non-constant initial value");
                        0
                    }
                };
                self.ldi16(24, v);
                self.store_var(a, t);
            }
        }
        // Pins: outputs low, inputs with their pull-up.
        for io in &p.ios {
            let Some(((_, ddr, port), bit)) = io.port.and_then(port_regs).zip(io.bit) else {
                continue;
            };
            match io.kind {
                IoK::GpioOut => {
                    self.o(Op::Cbi(port, bit));
                    self.o(Op::Sbi(ddr, bit));
                }
                IoK::GpioIn => {
                    self.o(Op::Cbi(ddr, bit));
                    if io.cfg & 1 != 0 {
                        self.o(Op::Sbi(port, bit));
                    }
                }
                _ => {}
            }
        }
        // Task start times.
        for s in &self.scheds {
            let (next, off) = (s.next, s.offset);
            self.ops.push(Op::Ldi(24, off as u8));
            self.ops.push(Op::Ldi(25, (off >> 8) as u8));
            self.ops.push(Op::Sts(next, 24));
            self.ops.push(Op::Sts(next + 1, 25));
        }
        // Timer 0, CTC, 1 kHz: OCR0A = sysclk / 64 / 1000 - 1.
        let ocr = (p.sysclk_hz / 64 / 1000).saturating_sub(1);
        if ocr == 0 || ocr > 255 {
            self.unsupported("a system clock that timer 0 cannot divide to 1 kHz");
        }
        self.o(Op::Ldi(16, 0x02));
        self.o(Op::Out(TCCR0A, 16));
        self.o(Op::Ldi(16, ocr as u8));
        self.o(Op::Out(OCR0A, 16));
        self.o(Op::Ldi(16, 0x03));
        self.o(Op::Out(TCCR0B, 16));
        self.o(Op::Ldi(16, 0x02));
        self.o(Op::Sts(TIMSK0_DATA, 16));
        // External interrupts: the edge of the handlers, or any change when they differ.
        let mut eicra = 0u8;
        let mut eimsk = 0u8;
        for (n, list) in ints {
            let edges: Vec<Edge> = list.iter().map(|i| p.irqs[*i].edge).collect();
            let mode = if edges.iter().all(|e| *e == Edge::Falling) {
                0b10
            } else if edges.iter().all(|e| *e == Edge::Rising) {
                0b11
            } else {
                0b01
            };
            eicra |= mode << (2 * n);
            eimsk |= 1 << n;
        }
        if eimsk != 0 {
            self.o(Op::Ldi(16, eicra));
            self.o(Op::Sts(EICRA_DATA, 16));
            self.o(Op::Ldi(16, eimsk));
            self.o(Op::Out(EIFR, 16));
            self.o(Op::Out(EIMSK, 16));
        }
        // Millisecond 0, then the main loop.
        self.o(Op::Call("rt_step".into()));
        self.o(Op::Sei);
        self.at("main_loop");
        self.o(Op::Cli);
        self.load16(24, self.ms);
        self.load16(22, self.last);
        self.o(Op::Cp(24, 22));
        self.o(Op::Cpc(25, 23));
        self.o(Op::Br(Flag::Z, false, "main_tick".into()));
        self.o(Op::Ldi(16, 1));
        self.o(Op::Out(SMCR, 16));
        self.o(Op::Sei);
        self.o(Op::Sleep);
        self.o(Op::Rjmp("main_loop".into()));
        self.at("main_tick");
        self.o(Op::Sei);
        self.o(Op::Subi(22, 0xff));
        self.o(Op::Sbci(23, 0xff));
        self.store16(self.last, 22);
        self.store16(self.now, 22);
        self.o(Op::Call("rt_step".into()));
        self.o(Op::Jmp("main_loop".into()));
    }

    /// The scheduler step, the fault routine and the division helper.
    fn runtime(&mut self) {
        let now = self.now;
        let faulted = self.faulted;
        // rt_step: commits, releases, then the ready activities by priority.
        self.at("rt_step");
        let n = self.scheds.len();
        for i in 0..n {
            let (has, cp, cat, label) = {
                let s = &self.scheds[i];
                (
                    !s.writes.is_empty() && s.deadline > 0,
                    s.commit_pending,
                    s.commit_at,
                    s.label.clone(),
                )
            };
            if !has {
                continue;
            }
            let skip = self.label("nocommit");
            self.o(Op::Lds(16, cp));
            self.o(Op::And(16, 16));
            self.jump_if(Flag::Z, true, &skip);
            self.load16(24, now);
            self.load16(22, cat);
            self.o(Op::Sub(24, 22));
            self.o(Op::Sbc(25, 23));
            self.jump_if(Flag::N, true, &skip);
            self.o(Op::Sts(cp, 1));
            self.o(Op::Call(format!("{label}_commit")));
            self.at(&skip);
        }
        self.o(Op::Lds(16, faulted));
        self.o(Op::And(16, 16));
        self.o(Op::Br(Flag::Z, true, "rt_go".into()));
        self.o(Op::Ret);
        self.at("rt_go");
        for i in 0..n {
            let (next, period, pending, release) = {
                let s = &self.scheds[i];
                (s.next, s.period, s.pending, s.release)
            };
            let skip = self.label("norel");
            self.load16(24, now);
            self.load16(22, next);
            self.o(Op::Sub(24, 22));
            self.o(Op::Sbc(25, 23));
            self.jump_if(Flag::N, true, &skip);
            self.load16(24, next);
            self.ldi16(22, period);
            self.o(Op::Add(24, 22));
            self.o(Op::Adc(25, 23));
            self.store16(next, 24);
            let room = self.label("room");
            self.o(Op::Lds(16, pending));
            self.o(Op::Cpi(16, 255));
            self.o(Op::Br(Flag::Z, false, room.clone()));
            self.o(Op::Ldi(20, F_DEADLINE));
            self.o(Op::Call("rt_fault".into()));
            self.o(Op::Ret);
            self.at(&room);
            self.o(Op::Inc(16));
            self.o(Op::Sts(pending, 16));
            self.load16(24, now);
            self.store16(release, 24);
            self.at(&skip);
        }
        // Highest priority first; equal priorities in table order.
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|a, b| {
            self.scheds[*b]
                .prio
                .cmp(&self.scheds[*a].prio)
                .then(a.cmp(b))
        });
        for i in order {
            let (pending, label, reads, writes, deadline, release, cat, cp, task) = {
                let s = &self.scheds[i];
                (
                    s.pending,
                    s.label.clone(),
                    s.reads.clone(),
                    s.writes.clone(),
                    s.deadline,
                    s.release,
                    s.commit_at,
                    s.commit_pending,
                    s.task.clone().unwrap_or_default(),
                )
            };
            let (again, done) = (self.label("run"), self.label("ran"));
            self.at(&again);
            self.o(Op::Lds(16, faulted));
            self.o(Op::And(16, 16));
            self.o(Op::Br(Flag::Z, true, format!("{again}_ok")));
            self.o(Op::Ret);
            self.at(&format!("{again}_ok"));
            self.o(Op::Lds(16, pending));
            self.o(Op::And(16, 16));
            self.jump_if(Flag::Z, true, &done);
            self.o(Op::Dec(16));
            self.o(Op::Sts(pending, 16));
            if !reads.is_empty() {
                self.o(Op::Cli);
                for s in &reads {
                    if let (Some(dst), Some((src, t))) = (
                        self.snaps.get(&(task.clone(), s.clone())).copied(),
                        self.vars.get(s).copied(),
                    ) {
                        for b in 0..size(t) {
                            self.o(Op::Lds(16, src + b));
                            self.o(Op::Sts(dst + b, 16));
                        }
                    }
                }
                self.o(Op::Sei);
            }
            self.o(Op::Call(label.clone()));
            if !writes.is_empty() {
                let skip = self.label("nocm");
                self.o(Op::Lds(16, faulted));
                self.o(Op::And(16, 16));
                self.jump_if(Flag::Z, false, &skip);
                if deadline == 0 {
                    self.o(Op::Call(format!("{label}_commit")));
                } else {
                    self.load16(24, release);
                    self.ldi16(22, deadline);
                    self.o(Op::Add(24, 22));
                    self.o(Op::Adc(25, 23));
                    self.store16(cat, 24);
                    self.o(Op::Ldi(16, 1));
                    self.o(Op::Sts(cp, 16));
                }
                self.at(&skip);
            }
            self.o(Op::Jmp(again));
            self.at(&done);
        }
        self.o(Op::Ret);

        // Commit routines: make the deferred writes of a task visible.
        for i in 0..n {
            let (label, writes) = (self.scheds[i].label.clone(), self.scheds[i].writes.clone());
            if writes.is_empty() {
                continue;
            }
            self.at(&format!("{label}_commit"));
            for s in &writes {
                if let (Some(d), Some(nx), Some((cur, t))) = (
                    self.dirty.get(s).copied(),
                    self.next_buf.get(s).copied(),
                    self.vars.get(s).copied(),
                ) {
                    let skip = self.label("clean");
                    self.o(Op::Lds(16, d));
                    self.o(Op::And(16, 16));
                    self.o(Op::Br(Flag::Z, true, skip.clone()));
                    self.o(Op::Sts(d, 1));
                    self.o(Op::Cli);
                    for b in 0..size(t) {
                        self.o(Op::Lds(16, nx + b));
                        self.o(Op::Sts(cur + b, 16));
                    }
                    self.o(Op::Sei);
                    self.at(&skip);
                }
            }
            self.o(Op::Ret);
        }

        // rt_fault (code in r20): the first fault wins; drop the waiting work; safe state.
        self.at("rt_fault");
        self.o(Op::Lds(16, faulted));
        self.o(Op::And(16, 16));
        self.o(Op::Br(Flag::Z, true, "rt_fault_new".into()));
        self.o(Op::Ret);
        self.at("rt_fault_new");
        self.o(Op::Ldi(16, 1));
        self.o(Op::Sts(faulted, 16));
        self.o(Op::Sts(self.fault_code, 20));
        let pendings: Vec<u16> = self.scheds.iter().map(|s| s.pending).collect();
        for p in pendings {
            self.o(Op::Sts(p, 1));
        }
        self.o(Op::Call("safe_state".into()));
        self.o(Op::Ret);

        // rt_udivmod16: r24:r25 / r22:r23 -> quotient r24:r25, remainder r22:r23.
        self.at("rt_udivmod16");
        self.o(Op::Eor(18, 18));
        self.o(Op::Eor(19, 19));
        self.o(Op::Ldi(21, 16));
        self.at("rt_div_loop");
        self.o(Op::Add(24, 24));
        self.o(Op::Adc(25, 25));
        self.o(Op::Adc(18, 18));
        self.o(Op::Adc(19, 19));
        self.o(Op::Cp(18, 22));
        self.o(Op::Cpc(19, 23));
        self.o(Op::Br(Flag::C, true, "rt_div_skip".into()));
        self.o(Op::Sub(18, 22));
        self.o(Op::Sbc(19, 23));
        self.o(Op::Inc(24));
        self.at("rt_div_skip");
        self.o(Op::Dec(21));
        self.o(Op::Br(Flag::Z, false, "rt_div_loop".into()));
        self.o(Op::Movw(22, 18));
        self.o(Op::Ret);
    }

    fn save_regs(&mut self) {
        self.o(Op::Push(0));
        self.o(Op::In(0, SREG));
        self.o(Op::Push(0));
        for r in 16..=25 {
            self.o(Op::Push(r));
        }
    }

    fn restore_regs(&mut self) {
        for r in (16..=25).rev() {
            self.o(Op::Pop(r));
        }
        self.o(Op::Pop(0));
        self.o(Op::Out(SREG, 0));
        self.o(Op::Pop(0));
    }

    fn isrs(&mut self, ints: &BTreeMap<u8, Vec<usize>>) {
        let p = self.p;
        // The millisecond tick.
        self.at("isr_tick");
        self.o(Op::Push(0));
        self.o(Op::In(0, SREG));
        self.o(Op::Push(0));
        self.o(Op::Push(24));
        self.o(Op::Push(25));
        self.load16(24, self.ms);
        self.o(Op::Subi(24, 0xff));
        self.o(Op::Sbci(25, 0xff));
        self.store16(self.ms, 24);
        self.o(Op::Pop(25));
        self.o(Op::Pop(24));
        self.o(Op::Pop(0));
        self.o(Op::Out(SREG, 0));
        self.o(Op::Pop(0));
        self.o(Op::Reti);
        for (n, list) in ints {
            self.at(&format!("isr_int{n}"));
            self.save_regs();
            let end = format!("isr_int{n}_end");
            self.o(Op::Lds(16, self.faulted));
            self.o(Op::And(16, 16));
            self.jump_if(Flag::Z, false, &end);
            for i in list {
                let irq = &p.irqs[*i];
                let Some(((pin, _, _), bit)) = p
                    .ios
                    .get(irq.io)
                    .and_then(|d| Some((port_regs(d.port?)?, d.bit?)))
                else {
                    continue;
                };
                // Run the handler only when the level matches its edge, as the C runtime does.
                let skip = self.label("edge");
                match irq.edge {
                    Edge::Rising => {
                        self.o(Op::Sbis(pin, bit));
                        self.o(Op::Rjmp(skip.clone()));
                    }
                    Edge::Falling => {
                        self.o(Op::Sbic(pin, bit));
                        self.o(Op::Rjmp(skip.clone()));
                    }
                    Edge::Change => {}
                }
                self.o(Op::Call(format!("irq{i}")));
                self.at(&skip);
            }
            self.at(&end);
            self.restore_regs();
            self.o(Op::Reti);
        }
    }

    /// Bytes of SRAM used by variables and the runtime.
    pub(crate) fn ram_bytes(&self) -> u16 {
        self.next_ram - SRAM_START
    }
}
