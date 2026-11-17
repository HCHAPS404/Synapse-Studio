//! The MIR data model (task B2).
//!
//! MIR is NIR after checking: names are resolved to C-safe identifiers, every expression has
//! a machine type, units are gone (a quantity is a `f32` in base SI units), constants are
//! folded, compound assignments are expanded and `signal` accesses say whether they read the
//! snapshot of a task or the committed value. It is a structured tree, not SSA: SSA arrives
//! with the native backends (task B8), which read the same tree.

use std::collections::BTreeSet;

/// Machine type of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ty {
    /// `bool`.
    Bool,
    /// 8-bit unsigned.
    U8,
    /// 8-bit signed.
    I8,
    /// 16-bit unsigned.
    U16,
    /// 16-bit signed.
    I16,
    /// 32-bit unsigned.
    U32,
    /// 32-bit signed.
    I32,
    /// Single-precision float. Also the representation of every quantity.
    F32,
}

impl Ty {
    /// `true` for the integer types.
    pub fn is_int(self) -> bool {
        !matches!(self, Ty::Bool | Ty::F32)
    }

    /// `true` for `f32`.
    pub fn is_float(self) -> bool {
        self == Ty::F32
    }

    /// The C type.
    pub fn c_type(self) -> &'static str {
        match self {
            Ty::Bool => "bool",
            Ty::U8 => "uint8_t",
            Ty::I8 => "int8_t",
            Ty::U16 => "uint16_t",
            Ty::I16 => "int16_t",
            Ty::U32 => "uint32_t",
            Ty::I32 => "int32_t",
            Ty::F32 => "float",
        }
    }

    /// Suffix of the runtime helpers (`nrt_fit_u16`, `nrt_ld_u16`).
    pub fn tag(self) -> &'static str {
        match self {
            Ty::Bool => "b",
            Ty::U8 => "u8",
            Ty::I8 => "i8",
            Ty::U16 => "u16",
            Ty::I16 => "i16",
            Ty::U32 => "u32",
            Ty::I32 => "i32",
            Ty::F32 => "f32",
        }
    }

    /// Name as written in Neuron.
    pub fn name(self) -> &'static str {
        match self {
            Ty::Bool => "bool",
            Ty::F32 => "f32",
            other => other.tag(),
        }
    }
}

/// Position in a source file, for `#line` directives and source maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Loc {
    /// Index into [`Program::files`].
    pub file: u32,
    /// 1-based line.
    pub line: u32,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// Arithmetic negation.
    Neg,
    /// Logical `not`.
    Not,
    /// Bitwise `~`.
    BitNot,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+` (checked).
    Add,
    /// `-` (checked).
    Sub,
    /// `*` (checked).
    Mul,
    /// `/` (a zero divisor is a fault).
    Div,
    /// `%`.
    Rem,
    /// `+%` (wrapping).
    AddW,
    /// `-%` (wrapping).
    SubW,
    /// `*%` (wrapping).
    MulW,
    /// `&`.
    BitAnd,
    /// `|`.
    BitOr,
    /// `^`.
    BitXor,
    /// `<<`.
    Shl,
    /// `>>`.
    Shr,
    /// `and`.
    And,
    /// `or`.
    Or,
    /// `==`.
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

impl BinOp {
    /// `true` for the comparison operators.
    pub fn is_compare(self) -> bool {
        matches!(
            self,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        )
    }
}

/// Which copy of a signal an expression reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigAccess {
    /// The committed value.
    Current,
    /// The copy taken when the running task was released (logical execution time).
    Snapshot,
}

/// Expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Integer constant.
    Int(i64, Ty),
    /// Real constant (base units).
    Float(f64),
    /// Boolean constant.
    Bool(bool),
    /// Local variable or parameter.
    Local(String, Ty),
    /// Module variable.
    Global(String, Ty),
    /// Signal read.
    Signal(String, Ty, SigAccess),
    /// Unary operation.
    Unary(UnOp, Box<Expr>, Ty),
    /// Binary operation; the type is the type of the result.
    Binary(BinOp, Box<Expr>, Box<Expr>, Ty),
    /// Conversion.
    Cast(Box<Expr>, Ty),
    /// `if c then a else b`.
    IfElse(Box<Expr>, Box<Expr>, Box<Expr>, Ty),
    /// Call of a user function that returns a value.
    Call(String, Vec<Expr>, Ty),
    /// Library function: `abs`, `min`, `max`, `clamp`.
    Builtin(String, Vec<Expr>, Ty),
    /// Read of an `io` (index into [`Program::ios`]): a pin level or an ADC conversion.
    IoGet(usize, Ty),
    /// Linear map of `x` from `[a, b]` to `[c, d]`.
    Scale(Vec<Expr>),
}

impl Expr {
    /// The type of the expression.
    pub fn ty(&self) -> Ty {
        match self {
            Expr::Int(_, t)
            | Expr::Local(_, t)
            | Expr::Global(_, t)
            | Expr::Signal(_, t, _)
            | Expr::Unary(_, _, t)
            | Expr::Binary(_, _, _, t)
            | Expr::Cast(_, t)
            | Expr::IfElse(_, _, _, t)
            | Expr::Call(_, _, t)
            | Expr::Builtin(_, _, t)
            | Expr::IoGet(_, t) => *t,
            Expr::Float(_) | Expr::Scale(_) => Ty::F32,
            Expr::Bool(_) => Ty::Bool,
        }
    }
}

/// Assignment targets.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    /// Local variable.
    Local(String),
    /// Module variable.
    Global(String),
    /// Signal. `deferred` writes go to the next-value buffer of the running task and become
    /// visible at its deadline; the others are immediate.
    Signal {
        /// Name.
        name: String,
        /// Written by a task.
        deferred: bool,
    },
}

/// A piece of printed text.
#[derive(Debug, Clone, PartialEq)]
pub enum PrintPart {
    /// Literal text.
    Text(String),
    /// A value, in the format of its type.
    Value(Expr),
}

/// Statements.
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    /// Source line.
    pub loc: Loc,
    /// The statement.
    pub kind: StmtKind,
}

/// Statement kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// Declaration of a local.
    Let {
        /// C name.
        name: String,
        /// Type.
        ty: Ty,
        /// Initial value.
        value: Expr,
    },
    /// Assignment.
    Assign {
        /// Target.
        place: Place,
        /// Value.
        value: Expr,
    },
    /// `io.write(v)`.
    IoPut {
        /// I/O index.
        io: usize,
        /// Level.
        value: Expr,
    },
    /// `io.toggle()`.
    IoFlip {
        /// I/O index.
        io: usize,
    },
    /// `io.duty = v` (a fraction of 1).
    IoPwm {
        /// I/O index.
        io: usize,
        /// Duty cycle.
        value: Expr,
    },
    /// `uart.print(...)`.
    Print {
        /// I/O index.
        io: usize,
        /// Pieces.
        parts: Vec<PrintPart>,
        /// Append a line feed.
        newline: bool,
    },
    /// Call of a function whose result, if any, is not used.
    CallVoid {
        /// C name.
        name: String,
        /// Arguments.
        args: Vec<Expr>,
    },
    /// `if` / `else`.
    If {
        /// Condition.
        cond: Expr,
        /// Then branch.
        then: Vec<Stmt>,
        /// Else branch (may be empty).
        otherwise: Vec<Stmt>,
    },
    /// `for v in lo..hi` with constant bounds (`hi` exclusive).
    For {
        /// C name of the loop variable.
        var: String,
        /// Its type.
        ty: Ty,
        /// First value.
        lo: i64,
        /// One past the last value.
        hi: i64,
        /// Body.
        body: Vec<Stmt>,
    },
    /// `while cond max N`.
    While {
        /// Condition.
        cond: Expr,
        /// Bound on the iterations.
        max: i64,
        /// Body.
        body: Vec<Stmt>,
    },
    /// `return`.
    Return(Option<Expr>),
    /// `break`.
    Break,
    /// `continue`.
    Continue,
    /// `fault Name`: enter the safe state and leave the activity.
    Fault(String),
    /// `emit E` (index into [`Program::events`]).
    Emit(usize),
}

/// A module variable, constant or signal.
#[derive(Debug, Clone, PartialEq)]
pub struct Global {
    /// C name.
    pub name: String,
    /// Type.
    pub ty: Ty,
    /// Initial value (a constant).
    pub init: Option<Expr>,
    /// `signal`.
    pub signal: bool,
    /// Accessed from an interrupt and from other code: reads and writes are atomic.
    pub shared: bool,
    /// Name in the Neuron source.
    pub source_name: String,
}

/// A function.
#[derive(Debug, Clone, PartialEq)]
pub struct Func {
    /// C name.
    pub name: String,
    /// Parameters.
    pub params: Vec<(String, Ty)>,
    /// Return type.
    pub ret: Option<Ty>,
    /// Body.
    pub body: Vec<Stmt>,
    /// Touches data shared with an interrupt (directly or through a call).
    pub atomic: bool,
    /// Names of the globals it touches directly.
    pub touches: BTreeSet<String>,
    /// Functions it calls.
    pub calls: BTreeSet<String>,
}

/// Kind of an `io`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoK {
    /// Digital input.
    GpioIn,
    /// Digital output.
    GpioOut,
    /// PWM output.
    Pwm,
    /// Analog input.
    Adc,
    /// Serial port.
    Uart,
}

/// An `io` bound to hardware (output of the target check).
#[derive(Debug, Clone, PartialEq)]
pub struct IoDef {
    /// Name in the source.
    pub name: String,
    /// Kind.
    pub kind: IoK,
    /// Port letter of the pin.
    pub port: Option<char>,
    /// Bit in the port.
    pub bit: Option<u8>,
    /// Interrupt number, ADC channel, PWM `timer * 4 + channel` or UART index; 255 for none.
    pub aux: u8,
    /// Pull-up, edge bits, PWM prescaler or UART divisor (see `nrt.h`).
    pub cfg: u16,
}

/// Edge of an external interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// High to low.
    Falling,
    /// Low to high.
    Rising,
    /// Either.
    Change,
}

/// A task.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    /// C name.
    pub name: String,
    /// Period in milliseconds, 0 for an event task.
    pub period_ms: u32,
    /// First release.
    pub offset_ms: u32,
    /// Priority.
    pub prio: u32,
    /// Event that releases it.
    pub event: Option<usize>,
    /// Delay from release to the visibility of its signal writes.
    pub deadline_ms: u32,
    /// Signals it reads (snapshotted at release).
    pub reads: Vec<String>,
    /// Signals it writes.
    pub writes: Vec<String>,
    /// Body.
    pub body: Vec<Stmt>,
    /// Source position.
    pub loc: Loc,
}

/// An interrupt handler.
#[derive(Debug, Clone, PartialEq)]
pub struct Irq {
    /// C name.
    pub name: String,
    /// The input that raises it.
    pub io: usize,
    /// Edge.
    pub edge: Edge,
    /// Priority (kept for the report; the hardware decides on AVR).
    pub prio: u32,
    /// Body.
    pub body: Vec<Stmt>,
}

/// A transition of a state machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Trans {
    /// Index of the target state.
    pub target: usize,
    /// Guard.
    pub when: Option<Expr>,
    /// Timeout since entry, in milliseconds.
    pub after_ms: Option<u32>,
    /// Action.
    pub action: Vec<Stmt>,
}

/// A state.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// Name.
    pub name: String,
    /// `entry:`.
    pub entry: Vec<Stmt>,
    /// `exit:`.
    pub exit: Vec<Stmt>,
    /// `do:`.
    pub during: Vec<Stmt>,
    /// Transitions in source order.
    pub trans: Vec<Trans>,
}

/// A state machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Fsm {
    /// Name.
    pub name: String,
    /// Evaluation period in milliseconds.
    pub period_ms: u32,
    /// Index of the initial state.
    pub initial: usize,
    /// States.
    pub states: Vec<State>,
}

/// A `when` rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// Condition.
    pub cond: Expr,
    /// Hold time in milliseconds.
    pub hold_ms: Option<u32>,
    /// Body.
    pub body: Vec<Stmt>,
}

/// The `safety:` block.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Safety {
    /// Hardware watchdog timeout.
    pub watchdog_ms: Option<u32>,
    /// Evaluation period of the rules.
    pub rate_ms: u32,
    /// Writes of the safe values.
    pub safe_state: Vec<Stmt>,
    /// Rules.
    pub rules: Vec<Rule>,
    /// `reset when`.
    pub reset: Option<Expr>,
    /// `on fault` handlers: the fault name (any when `None`) and the body.
    pub on_fault: Vec<(Option<String>, Vec<Stmt>)>,
}

/// A whole program.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Program {
    /// Package or module name.
    pub name: String,
    /// Board.
    pub board: Option<String>,
    /// System clock in hertz.
    pub sysclk_hz: u64,
    /// Source files, indexed by [`Loc::file`].
    pub files: Vec<String>,
    /// Variables, constants and signals.
    pub globals: Vec<Global>,
    /// Functions.
    pub funcs: Vec<Func>,
    /// I/O bindings.
    pub ios: Vec<IoDef>,
    /// Event names.
    pub events: Vec<String>,
    /// User fault names (their codes follow the built-in ones).
    pub faults: Vec<String>,
    /// Tasks.
    pub tasks: Vec<Task>,
    /// Interrupt handlers.
    pub irqs: Vec<Irq>,
    /// State machines.
    pub fsms: Vec<Fsm>,
    /// `when` rules outside `safety`.
    pub rules: Vec<Rule>,
    /// Safety logic.
    pub safety: Safety,
}
