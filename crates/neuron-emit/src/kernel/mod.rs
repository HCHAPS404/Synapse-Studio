//! The micro-kernel (task C6): state tables and the interpreters that run them.
//!
//! A `fsm` or a `sequence` is compiled to a [`Table`]: states, transitions and small
//! postfix programs (guards and actions). The table has two forms:
//!
//! * `.nst.json`, the readable form ([`Table::to_json`]);
//! * the word encoding ([`Machine::encode`]), an array of `u16` words that a device
//!   downloads, also written as text (`.nst.hex`, [`to_hex`]).
//!
//! Three table-independent interpreters read the words: `neuron_kernel.st` (an IEC
//! function block over an `ARRAY OF INT`), `nrt_kernel.c` (C99) and `neuron_kernel.spel`
//! (SPEL+ functions). [`Exec`] is the Rust reference executor of the same algorithm.
//!
//! # Word layout
//!
//! | Words | Content |
//! |---|---|
//! | 0 to 11 | header: magic `0x4E53`, version `1`, flags (bit 0 sequence, bit 1 publish every output at the first scan), inputs, outputs, variables, states, transitions, initial state, program length, total length, checksum |
//! | 12.. | variable initial values, one word each |
//! | next | one record per state, 5 words: entry, exit, `do` (offsets into the program, `0xFFFF` for none), index of its first transition, number of transitions |
//! | next | one record per transition, 3 words: target state, guard offset (`0xFFFF`: always), action offset (`0xFFFF`: none) |
//! | rest | program: blocks of instructions, each ended by `END` (`0`) |
//!
//! The checksum is the sum of `word[i] * (i + 1)` over every word except the checksum
//! itself, modulo 65521: any single flipped bit changes it.
//!
//! # Instructions
//!
//! Values are 16-bit signed integers; booleans are `0` and `1`. Every result wraps to 16
//! bits. A block has at most 16 values on its stack. `Push v`, `Ldi i` (input), `Ldv i`
//! (variable), `Ldo i` (output), `Timer` (scans since the state was entered), `Sto i`
//! (pop into output `i`), `Stv i`, `Not`, `And`, `Or`, `Eq`, `Ne`, `Lt`, `Le`, `Gt`,
//! `Ge`, `Add`, `Sub`, `Mul`, `Neg`, `Tclr` (reset the state timer).
//!
//! # One scan
//!
//! The first scan loads the initial values and enters the initial state. Each scan then
//! runs rounds: in a round the state's `do` block runs, then its transitions are tried in
//! order and the first whose guard is not zero fires (exit, action, new state, timer to
//! zero, entry of the new state). A `fsm` runs one round per scan, like the reference
//! interpreter; a `sequence` runs rounds until nothing fires (at most 64), because a
//! sequence checks a wait at once. At the end of the scan the timer grows by one and the
//! outputs that changed since the last report are listed.

mod compile;
mod emit;

pub use compile::{compile_nir, Compiled, Note};
pub use emit::Kernel;

use serde::{Deserialize, Serialize};

/// First word of a table: the letters `NS`.
pub const MAGIC: u16 = 0x4E53;
/// Version of the encoding.
pub const VERSION: u16 = 1;
/// Number of header words.
pub const HEADER_WORDS: usize = 12;
/// Marks "no block" in a state or transition record.
pub const NONE: u16 = 0xFFFF;
/// Limit of states.
pub const MAX_STATES: usize = 64;
/// Limit of transitions.
pub const MAX_TRANSITIONS: usize = 128;
/// Limit of program words.
pub const MAX_PROGRAM_WORDS: usize = 256;
/// Limit of variables.
pub const MAX_VARS: usize = 32;
/// Limit of inputs and of outputs (each).
pub const MAX_IO: usize = 32;
/// Depth of the value stack.
pub const MAX_STACK: usize = 16;
/// Largest value of the state timer, in scans.
pub const MAX_TIMER: i16 = i16::MAX;
/// Rounds of one scan of a sequence.
pub const MAX_ROUNDS: usize = 64;
/// Modulus of the checksum.
const CHECK_MOD: u64 = 65_521;

/// One instruction of a guard or action block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Insn {
    /// Pushes a constant.
    Push(i16),
    /// Pushes input `i`.
    Ldi(u8),
    /// Pushes variable `i`.
    Ldv(u8),
    /// Pushes the current value of output `i`.
    Ldo(u8),
    /// Pushes the state timer (scans since entry).
    Timer,
    /// Pops into output `i`.
    Sto(u8),
    /// Pops into variable `i`.
    Stv(u8),
    /// Logical not: `x == 0`.
    Not,
    /// Logical and of two values.
    And,
    /// Logical or of two values.
    Or,
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Less than.
    Lt,
    /// Less or equal.
    Le,
    /// Greater than.
    Gt,
    /// Greater or equal.
    Ge,
    /// Wrapping sum.
    Add,
    /// Wrapping difference.
    Sub,
    /// Wrapping product.
    Mul,
    /// Wrapping negation.
    Neg,
    /// Sets the state timer to zero.
    Tclr,
}

impl Insn {
    fn opcode(self) -> u16 {
        match self {
            Insn::Push(_) => 1,
            Insn::Ldi(_) => 2,
            Insn::Ldv(_) => 3,
            Insn::Ldo(_) => 4,
            Insn::Timer => 5,
            Insn::Sto(_) => 6,
            Insn::Stv(_) => 7,
            Insn::Not => 8,
            Insn::And => 9,
            Insn::Or => 10,
            Insn::Eq => 11,
            Insn::Ne => 12,
            Insn::Lt => 13,
            Insn::Le => 14,
            Insn::Gt => 15,
            Insn::Ge => 16,
            Insn::Add => 17,
            Insn::Sub => 18,
            Insn::Mul => 19,
            Insn::Neg => 20,
            Insn::Tclr => 21,
        }
    }

    fn operand(self) -> Option<u16> {
        match self {
            Insn::Push(v) => Some(v as u16),
            Insn::Ldi(i) | Insn::Ldv(i) | Insn::Ldo(i) | Insn::Sto(i) | Insn::Stv(i) => {
                Some(u16::from(i))
            }
            _ => None,
        }
    }

    /// Number of words the instruction takes.
    pub fn words(self) -> usize {
        1 + usize::from(self.operand().is_some())
    }

    fn from_words(op: u16, arg: u16) -> Option<Insn> {
        let idx = u8::try_from(arg).ok();
        Some(match op {
            1 => Insn::Push(arg as i16),
            2 => Insn::Ldi(idx?),
            3 => Insn::Ldv(idx?),
            4 => Insn::Ldo(idx?),
            5 => Insn::Timer,
            6 => Insn::Sto(idx?),
            7 => Insn::Stv(idx?),
            8 => Insn::Not,
            9 => Insn::And,
            10 => Insn::Or,
            11 => Insn::Eq,
            12 => Insn::Ne,
            13 => Insn::Lt,
            14 => Insn::Le,
            15 => Insn::Gt,
            16 => Insn::Ge,
            17 => Insn::Add,
            18 => Insn::Sub,
            19 => Insn::Mul,
            20 => Insn::Neg,
            21 => Insn::Tclr,
            _ => return None,
        })
    }

    /// Values popped and pushed.
    fn effect(self) -> (usize, usize) {
        match self {
            Insn::Push(_) | Insn::Ldi(_) | Insn::Ldv(_) | Insn::Ldo(_) | Insn::Timer => (0, 1),
            Insn::Sto(_) | Insn::Stv(_) => (1, 0),
            Insn::Not | Insn::Neg => (1, 1),
            Insn::Tclr => (0, 0),
            _ => (2, 1),
        }
    }
}

/// A transition of a state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trans {
    /// Destination state.
    pub target: usize,
    /// Guard; `None` fires always. A guard leaves one value; it fires when that is not 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<Vec<Insn>>,
    /// Action, run after the exit block of the source state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action: Vec<Insn>,
}

/// A state with its blocks and its transitions (tried in order).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StateCode {
    /// Run when the state is entered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entry: Vec<Insn>,
    /// Run when the state is left.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exit: Vec<Insn>,
    /// Run in every round while the state is active (`do`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub during: Vec<Insn>,
    /// Transitions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<Trans>,
}

/// What the word encoding holds: the behaviour of a state machine, without names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    /// Run rounds until nothing fires (a sequence) instead of one round (a state machine).
    pub sequence: bool,
    /// Report every output at the first scan, as a PLC does; otherwise an output is
    /// reported from its first write.
    pub publish_initial: bool,
    /// Number of inputs.
    pub inputs: usize,
    /// Number of outputs.
    pub outputs: usize,
    /// Initial value of each variable.
    pub vars: Vec<i16>,
    /// Initial state.
    pub initial: usize,
    /// The states.
    pub states: Vec<StateCode>,
}

fn block_words(code: &[Insn]) -> usize {
    if code.is_empty() {
        0
    } else {
        code.iter().map(|i| i.words()).sum::<usize>() + 1
    }
}

fn put_block(prog: &mut Vec<u16>, code: &[Insn]) -> u16 {
    if code.is_empty() {
        return NONE;
    }
    let at = prog.len() as u16;
    for i in code {
        prog.push(i.opcode());
        if let Some(a) = i.operand() {
            prog.push(a);
        }
    }
    prog.push(0);
    at
}

fn checksum(words: &[u16]) -> u16 {
    let mut acc: u64 = 0;
    for (i, w) in words.iter().enumerate() {
        if i != 11 {
            acc = (acc + u64::from(*w) * (i as u64 + 1)) % CHECK_MOD;
        }
    }
    acc as u16
}

impl Machine {
    /// Total number of transitions.
    pub fn transitions(&self) -> usize {
        self.states.iter().map(|s| s.transitions.len()).sum()
    }

    /// Words of the program part.
    pub fn program_words(&self) -> usize {
        self.states
            .iter()
            .map(|s| {
                block_words(&s.entry)
                    + block_words(&s.exit)
                    + block_words(&s.during)
                    + s.transitions
                        .iter()
                        .map(|t| t.guard.as_deref().map_or(0, block_words) + block_words(&t.action))
                        .sum::<usize>()
            })
            .sum()
    }

    /// Total words of the encoding.
    pub fn total_words(&self) -> usize {
        HEADER_WORDS
            + self.vars.len()
            + 5 * self.states.len()
            + 3 * self.transitions()
            + self.program_words()
    }

    fn blocks(&self) -> impl Iterator<Item = (&[Insn], bool)> {
        self.states.iter().flat_map(|s| {
            let mut v: Vec<(&[Insn], bool)> = vec![
                (s.entry.as_slice(), false),
                (s.exit.as_slice(), false),
                (s.during.as_slice(), false),
            ];
            for t in &s.transitions {
                if let Some(g) = &t.guard {
                    v.push((g.as_slice(), true));
                }
                v.push((t.action.as_slice(), false));
            }
            v
        })
    }

    /// The limits of the micro-kernel that this machine exceeds (`NE1004`), one text each.
    pub fn limits(&self) -> Vec<String> {
        let mut v = Vec::new();
        let mut chk = |what: &str, n: usize, max: usize| {
            if n > max {
                v.push(format!("{n} {what}, the limit is {max}"));
            }
        };
        chk("states", self.states.len(), MAX_STATES);
        chk("transitions", self.transitions(), MAX_TRANSITIONS);
        chk("program words", self.program_words(), MAX_PROGRAM_WORDS);
        chk("variables", self.vars.len(), MAX_VARS);
        chk("inputs", self.inputs, MAX_IO);
        chk("outputs", self.outputs, MAX_IO);
        let deepest = self
            .blocks()
            .filter_map(|(b, r)| self.analyse(b, r).ok())
            .max()
            .unwrap_or(0);
        chk("values on the stack", deepest, MAX_STACK);
        v
    }

    /// Stack discipline and index ranges of one block. Returns its deepest stack.
    fn analyse(&self, code: &[Insn], result: bool) -> Result<usize, String> {
        let (mut depth, mut deepest) = (0usize, 0usize);
        for i in code {
            let (pop, push) = i.effect();
            if depth < pop {
                return Err(format!("`{i:?}` pops from an empty stack"));
            }
            depth = depth - pop + push;
            deepest = deepest.max(depth);
            let bad = match *i {
                Insn::Ldi(k) => usize::from(k) >= self.inputs,
                Insn::Ldv(k) | Insn::Stv(k) => usize::from(k) >= self.vars.len(),
                Insn::Ldo(k) | Insn::Sto(k) => usize::from(k) >= self.outputs,
                _ => false,
            };
            if bad {
                return Err(format!("`{i:?}` is out of range"));
            }
        }
        let want = usize::from(result);
        if depth != want {
            return Err(format!("a block leaves {depth} values, expected {want}"));
        }
        Ok(deepest)
    }

    /// Structural check: indices, targets and stack use. Limits are [`Machine::limits`].
    ///
    /// # Errors
    /// The first problem found.
    pub fn check(&self) -> Result<(), String> {
        if self.states.is_empty() {
            return Err("no states".into());
        }
        if self.initial >= self.states.len() {
            return Err("the initial state does not exist".into());
        }
        for s in &self.states {
            for t in &s.transitions {
                if t.target >= self.states.len() {
                    return Err("a transition goes to a state that does not exist".into());
                }
            }
        }
        for (b, r) in self.blocks() {
            self.analyse(b, r)?;
        }
        Ok(())
    }

    /// The word encoding.
    ///
    /// # Errors
    /// The machine exceeds a limit or is not well formed.
    pub fn encode(&self) -> Result<Vec<u16>, String> {
        let l = self.limits();
        if !l.is_empty() {
            return Err(l.join("; "));
        }
        self.check()?;
        let mut head = vec![0u16; HEADER_WORDS];
        let mut vars = Vec::new();
        let mut states = Vec::new();
        let mut trans = Vec::new();
        let mut prog = Vec::new();
        for v in &self.vars {
            vars.push(*v as u16);
        }
        let mut first = 0u16;
        for s in &self.states {
            states.push(put_block(&mut prog, &s.entry));
            states.push(put_block(&mut prog, &s.exit));
            states.push(put_block(&mut prog, &s.during));
            states.push(first);
            states.push(s.transitions.len() as u16);
            first += s.transitions.len() as u16;
            for t in &s.transitions {
                trans.push(t.target as u16);
                trans.push(t.guard.as_deref().map_or(NONE, |g| put_block(&mut prog, g)));
                trans.push(put_block(&mut prog, &t.action));
            }
        }
        head[0] = MAGIC;
        head[1] = VERSION;
        head[2] = u16::from(self.sequence) | (u16::from(self.publish_initial) << 1);
        head[3] = self.inputs as u16;
        head[4] = self.outputs as u16;
        head[5] = self.vars.len() as u16;
        head[6] = self.states.len() as u16;
        head[7] = self.transitions() as u16;
        head[8] = self.initial as u16;
        head[9] = prog.len() as u16;
        let mut w = head;
        w.extend(vars);
        w.extend(states);
        w.extend(trans);
        w.extend(prog);
        w[10] = w.len() as u16;
        w[11] = checksum(&w);
        Ok(w)
    }

    /// Reads a word encoding.
    ///
    /// # Errors
    /// The magic, version, length or checksum is wrong, or the content is malformed.
    pub fn decode(w: &[u16]) -> Result<Machine, String> {
        if w.len() < HEADER_WORDS {
            return Err("shorter than the header".into());
        }
        if w[0] != MAGIC {
            return Err("bad magic".into());
        }
        if w[1] != VERSION {
            return Err(format!("unsupported version {}", w[1]));
        }
        if usize::from(w[10]) != w.len() {
            return Err("length does not match the header".into());
        }
        if w[11] != checksum(w) {
            return Err("bad checksum".into());
        }
        let (nv, ns, nt, np) = (
            usize::from(w[5]),
            usize::from(w[6]),
            usize::from(w[7]),
            usize::from(w[9]),
        );
        if HEADER_WORDS + nv + 5 * ns + 3 * nt + np != w.len() {
            return Err("the sections do not fill the table".into());
        }
        let o_var = HEADER_WORDS;
        let o_st = o_var + nv;
        let o_tr = o_st + 5 * ns;
        let o_pr = o_tr + 3 * nt;
        let prog = &w[o_pr..];
        let block = |off: u16| -> Result<Vec<Insn>, String> {
            if off == NONE {
                return Ok(Vec::new());
            }
            let mut pc = usize::from(off);
            let mut out = Vec::new();
            loop {
                let op = *prog.get(pc).ok_or("a block runs past the program")?;
                pc += 1;
                if op == 0 {
                    return Ok(out);
                }
                let arg = if matches!(op, 1..=4 | 6 | 7) {
                    let a = *prog.get(pc).ok_or("an operand is missing")?;
                    pc += 1;
                    a
                } else {
                    0
                };
                out.push(Insn::from_words(op, arg).ok_or("unknown instruction")?);
            }
        };
        let mut states = Vec::new();
        for s in 0..ns {
            let r = &w[o_st + 5 * s..o_st + 5 * s + 5];
            let (first, count) = (usize::from(r[3]), usize::from(r[4]));
            if first + count > nt {
                return Err("transitions out of range".into());
            }
            let mut transitions = Vec::new();
            for t in first..first + count {
                let r = &w[o_tr + 3 * t..o_tr + 3 * t + 3];
                transitions.push(Trans {
                    target: usize::from(r[0]),
                    guard: if r[1] == NONE {
                        None
                    } else {
                        Some(block(r[1])?)
                    },
                    action: block(r[2])?,
                });
            }
            states.push(StateCode {
                entry: block(r[0])?,
                exit: block(r[1])?,
                during: block(r[2])?,
                transitions,
            });
        }
        let m = Machine {
            sequence: w[2] & 1 != 0,
            publish_initial: w[2] & 2 != 0,
            inputs: usize::from(w[3]),
            outputs: usize::from(w[4]),
            vars: w[o_var..o_st].iter().map(|v| *v as i16).collect(),
            initial: usize::from(w[8]),
            states,
        };
        m.check()?;
        Ok(m)
    }
}

/// Words as text: four hex digits each, sixteen words per line.
pub fn to_hex(words: &[u16]) -> String {
    let mut s = String::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            s.push(if i % 16 == 0 { '\n' } else { ' ' });
        }
        s.push_str(&format!("{w:04X}"));
    }
    s.push('\n');
    s
}

/// Reads the text of [`to_hex`].
///
/// # Errors
/// A token that is not a 16-bit hexadecimal number.
pub fn from_hex(text: &str) -> Result<Vec<u16>, String> {
    text.split_whitespace()
        .map(|t| u16::from_str_radix(t, 16).map_err(|_| format!("`{t}` is not a word")))
        .collect()
}

/// An input or an output of a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Port {
    /// Name in the Neuron program (the `io` binding or the PLC variable).
    pub name: String,
    /// A boolean (reported as `true` and `false`) or an integer.
    pub boolean: bool,
}

impl Port {
    /// Text of a value as the reference interpreter prints it.
    pub fn show(&self, v: i16) -> String {
        if self.boolean {
            (v != 0).to_string()
        } else {
            v.to_string()
        }
    }
}

/// A compiled state machine or sequence with its names: the `.nst.json` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    /// Format of this file (1).
    pub nst: u16,
    /// Name of the `fsm` or `sequence`.
    pub name: String,
    /// `fsm` or `sequence`.
    pub kind: String,
    /// Source file.
    pub file: String,
    /// Scan period in nanoseconds.
    pub scan_ns: i64,
    /// Inputs in order.
    pub inputs: Vec<Port>,
    /// Outputs in order.
    pub outputs: Vec<Port>,
    /// Names of the variables.
    pub var_names: Vec<String>,
    /// Names of the states.
    pub state_names: Vec<String>,
    /// The behaviour.
    pub machine: Machine,
}

impl Table {
    /// The word encoding.
    ///
    /// # Errors
    /// See [`Machine::encode`].
    pub fn words(&self) -> Result<Vec<u16>, String> {
        self.machine.encode()
    }

    /// Canonical JSON text.
    pub fn to_json(&self) -> String {
        crate::to_json(self)
    }
}

/// One reported change of an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    /// Scan number (from 0).
    pub scan: u64,
    /// Output index.
    pub output: usize,
    /// New value.
    pub value: i16,
}

/// What one scan did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanOut {
    /// Outputs reported this scan, by index.
    pub changes: Vec<(usize, i16)>,
    /// States entered this scan, in order.
    pub entered: Vec<usize>,
}

/// The reference executor of a [`Machine`]: the algorithm the three kernels implement.
pub struct Exec<'a> {
    m: &'a Machine,
    started: bool,
    state: usize,
    timer: i16,
    vars: Vec<i16>,
    ov: Vec<i16>,
    ok: Vec<bool>,
    pv: Vec<i16>,
    pk: Vec<bool>,
}

impl<'a> Exec<'a> {
    /// An executor before its first scan.
    pub fn new(m: &'a Machine) -> Exec<'a> {
        Exec {
            m,
            started: false,
            state: m.initial,
            timer: 0,
            vars: m.vars.clone(),
            ov: vec![0; m.outputs],
            ok: vec![m.publish_initial; m.outputs],
            pv: vec![0; m.outputs],
            pk: vec![false; m.outputs],
        }
    }

    /// Index of the active state.
    pub fn state(&self) -> usize {
        self.state
    }

    /// Runs a block; returns the top of its stack (0 for an empty stack).
    fn run(&mut self, code: &[Insn], input: &[i16]) -> i16 {
        let mut st: Vec<i16> = Vec::with_capacity(MAX_STACK);
        for i in code {
            match *i {
                Insn::Push(v) => st.push(v),
                Insn::Ldi(k) => st.push(input.get(usize::from(k)).copied().unwrap_or(0)),
                Insn::Ldv(k) => st.push(self.vars[usize::from(k)]),
                Insn::Ldo(k) => st.push(self.ov[usize::from(k)]),
                Insn::Timer => st.push(self.timer),
                Insn::Sto(k) => {
                    self.ov[usize::from(k)] = st.pop().unwrap_or(0);
                    self.ok[usize::from(k)] = true;
                }
                Insn::Stv(k) => self.vars[usize::from(k)] = st.pop().unwrap_or(0),
                Insn::Not => {
                    let a = st.pop().unwrap_or(0);
                    st.push(i16::from(a == 0));
                }
                Insn::Neg => {
                    let a = st.pop().unwrap_or(0);
                    st.push(a.wrapping_neg());
                }
                Insn::Tclr => self.timer = 0,
                op => {
                    let b = st.pop().unwrap_or(0);
                    let a = st.pop().unwrap_or(0);
                    st.push(match op {
                        Insn::And => i16::from(a != 0 && b != 0),
                        Insn::Or => i16::from(a != 0 || b != 0),
                        Insn::Eq => i16::from(a == b),
                        Insn::Ne => i16::from(a != b),
                        Insn::Lt => i16::from(a < b),
                        Insn::Le => i16::from(a <= b),
                        Insn::Gt => i16::from(a > b),
                        Insn::Ge => i16::from(a >= b),
                        Insn::Add => a.wrapping_add(b),
                        Insn::Sub => a.wrapping_sub(b),
                        _ => a.wrapping_mul(b),
                    });
                }
            }
        }
        st.last().copied().unwrap_or(0)
    }

    /// Runs one scan with the given inputs (missing inputs read 0).
    pub fn scan(&mut self, input: &[i16]) -> ScanOut {
        let m = self.m;
        let mut out = ScanOut::default();
        if !self.started {
            self.started = true;
            self.state = m.initial;
            self.timer = 0;
            out.entered.push(self.state);
            self.run(&m.states[self.state].entry, input);
        }
        let rounds = if m.sequence { MAX_ROUNDS } else { 1 };
        for _ in 0..rounds {
            let st = &m.states[self.state];
            self.run(&st.during, input);
            let mut fired = false;
            for t in &st.transitions {
                let go = match &t.guard {
                    None => true,
                    Some(g) => self.run(g, input) != 0,
                };
                if go {
                    self.run(&st.exit, input);
                    self.run(&t.action, input);
                    self.state = t.target;
                    self.timer = 0;
                    out.entered.push(self.state);
                    self.run(&m.states[self.state].entry, input);
                    fired = true;
                    break;
                }
            }
            if !fired {
                break;
            }
        }
        if self.timer < MAX_TIMER {
            self.timer += 1;
        }
        for i in 0..m.outputs {
            if self.ok[i] {
                if !self.pk[i] || self.pv[i] != self.ov[i] {
                    out.changes.push((i, self.ov[i]));
                }
                self.pv[i] = self.ov[i];
                self.pk[i] = true;
            }
        }
        out
    }
}

/// The result of running a machine over a schedule of inputs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace {
    /// Reported output changes.
    pub changes: Vec<Change>,
    /// `(scan, state)` for every state entered.
    pub states: Vec<(u64, usize)>,
}

impl Machine {
    /// Runs `scans` scans. `inputs(k)` gives the inputs of scan `k`.
    pub fn run(&self, scans: u64, mut inputs: impl FnMut(u64) -> Vec<i16>) -> Trace {
        let mut ex = Exec::new(self);
        let mut t = Trace::default();
        for k in 0..scans {
            let i = inputs(k);
            let o = ex.scan(&i);
            for (output, value) in o.changes {
                t.changes.push(Change {
                    scan: k,
                    output,
                    value,
                });
            }
            for s in o.entered {
                t.states.push((k, s));
            }
        }
        t
    }

    /// Runs over a schedule of `(instant in ns, input, value)`: scan `k` happens at
    /// `k * scan_ns` and sees every change at or before that instant.
    pub fn run_schedule(&self, scan_ns: i64, schedule: &[(i64, usize, i16)], end_ns: i64) -> Trace {
        let scans = u64::try_from((end_ns / scan_ns.max(1)).max(0)).unwrap_or(0);
        let mut cur = vec![0i16; self.inputs];
        let mut next = 0;
        self.run(scans, |k| {
            let now = i64::try_from(k).unwrap_or(0) * scan_ns;
            while next < schedule.len() && schedule[next].0 <= now {
                if let Some(slot) = cur.get_mut(schedule[next].1) {
                    *slot = schedule[next].2;
                }
                next += 1;
            }
            cur.clone()
        })
    }
}
