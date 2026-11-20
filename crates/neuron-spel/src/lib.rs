//! A subset of SPEL+-style robot programs and a virtual controller that runs it (task C3).
//!
//! The subset is what `neuron emit spel` writes, plus hand-written programs that use the
//! same statements:
//!
//! | Statement | Meaning |
//! |---|---|
//! | `Function name` ... `Fend`, `Call name`, `Exit Function` | functions (variables are global) |
//! | `Integer a, b`, `Real x`, `Global Real g` | declarations (every variable starts at 0) |
//! | `Integer t(100)`, `t(i) = x`, `t(i)` | global arrays of numbers (one index, every element starts at 0) |
//! | `v = expr`, `For v = a To b [Step s]` ... `Next v` | arithmetic and loops |
//! | `If c Then` ... `ElseIf` ... `Else` ... `EndIf`, `If c Then stmt` | conditions |
//! | `label:`, `GoTo label` | jumps inside a function |
//! | `P3 = JA(j1, ..)`, `P3 = XY(x, y, z, u, v, w) /L1`, `P4 = P3 +Z(50)` | points |
//! | `Local k, XY(..)`, `TLSet k, XY(..)`, `Tool k`, `Weight kg`, `PLabel n, "name"` | frames and tools |
//! | `Speed p`, `SpeedS v`, `CP On`, `CP Off` | motion settings |
//! | `Go P3 [+Z(d)]`, `Move P3 [+Z(d)]`, `Arc P2, P3` | motions: each takes a fixed time |
//! | `On b`, `Off b`, `Sw(b)`, `Out(b)` | outputs and inputs |
//! | `OutW w, v`, `InW(w)` | output and input word registers (integers). Style of `On`/`Sw`; the real word I/O statements must be confirmed in session I6 |
//! | `Wait seconds`, `Wait cond`, `Wait cond, seconds` and `TW` | waits polled on a 1 ms grid |
//! | `Error n`, `OnErr GoTo label`, `Quit All`, `Halt` | errors |
//! | `Call NeuronCheck(n)` | workspace check against [`Limits`] |
//!
//! The [`Controller`] runs `main` in simulated time. A motion takes
//! [`Options::move_ns`] (1 s by default, like the reference interpreter); a wait on a
//! condition is polled every [`Options::tick_ns`] (1 ms). It records every motion with its
//! instant and resolved target, and every change of an output.
//!
//! ```
//! let prog = neuron_spel::parse("Function main\n  P1 = JA(0, 0, 0, 0, 0, 0)\n  Go P1\n  On 3\nFend\n").unwrap();
//! let mut c = neuron_spel::Controller::new(prog, neuron_spel::Options::default());
//! c.run();
//! assert_eq!(c.motions().len(), 1);
//! assert_eq!(c.outputs()[0].t_ns, 1_000_000_000);
//! ```

#![forbid(unsafe_code)]

mod parse;
mod vm;

pub use parse::{parse, ParseError};
pub use vm::{
    Controller, Limits, Motion, MotionKind, Options, OutChange, Outcome, Target, WordChange,
    DEFAULT_MOVE_NS, WORKSPACE_ERROR,
};

/// A parsed program.
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub(crate) funcs: Vec<Func>,
    pub(crate) globals: Vec<String>,
    /// Names of the arrays (`Integer a(10)` or an assignment `a(i) = x`).
    pub(crate) arrays: Vec<String>,
}

impl Program {
    /// Names of the functions, lowercase, in source order.
    pub fn functions(&self) -> Vec<&str> {
        self.funcs.iter().map(|f| f.name.as_str()).collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Func {
    pub name: String,
    pub code: Vec<Ins>,
    pub labels: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone)]
pub(crate) enum Ex {
    Num(f64),
    Var(String),
    Neg(Box<Ex>),
    Not(Box<Ex>),
    Bin(Op, Box<Ex>, Box<Ex>),
    Call(String, Vec<Ex>),
}

impl Ex {
    /// Whether the expression is a condition (a wait on it is a wait on an input).
    pub(crate) fn is_cond(&self) -> bool {
        match self {
            Ex::Not(_) => true,
            Ex::Bin(op, ..) => matches!(
                op,
                Op::Eq | Op::Ne | Op::Lt | Op::Gt | Op::Le | Op::Ge | Op::And | Op::Or
            ),
            Ex::Call(n, _) => n == "sw" || n == "out",
            Ex::Var(n) => n == "tw" || n == "true" || n == "false",
            _ => false,
        }
    }
}

/// A point reference of a motion: `P3` or `P3 +Z(d)`.
#[derive(Debug, Clone)]
pub(crate) struct PRef {
    pub n: u32,
    pub dz: Option<Ex>,
}

#[derive(Debug, Clone)]
pub(crate) enum PointDef {
    Ja(Vec<Ex>),
    Xy(Vec<Ex>, Option<u32>),
    Copy(PRef),
}

#[derive(Debug, Clone)]
pub(crate) enum Ins {
    Set(String, Ex),
    /// `name(index) = value`: an element of an array (global, every element starts at 0).
    SetIdx(String, Ex, Ex),
    SetPoint(u32, PointDef),
    Jmp(usize),
    JmpIfNot(Ex, usize),
    ForTest {
        var: String,
        to: Ex,
        step: Ex,
        exit: usize,
    },
    GoTo(String),
    Call(String, Vec<Ex>),
    Exit,
    Speed(Ex),
    SpeedS(Ex),
    Cp(bool),
    Go(PRef),
    Move(PRef),
    Arc(PRef, PRef),
    On(Ex),
    Off(Ex),
    /// `OutW word, value`: writes an output word register.
    OutW(Ex, Ex),
    Wait(Ex),
    WaitCond(Ex, Option<Ex>),
    Error(Ex),
    OnErr(String),
    Quit,
    Local(u32, Vec<Ex>),
    TlSet(u32, Vec<Ex>),
    Tool(u32),
    Weight(Ex),
    PLabel(u32, String),
}
