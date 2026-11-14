//! Abstract syntax tree (spec/grammar/neuron.ebnf).
//!
//! Every node carries the [`Span`] of the source it was parsed from, so later
//! stages can report diagnostics and build source maps.

use crate::span::Span;

/// An identifier with its span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    /// Name without escaping backticks.
    pub name: String,
    /// Location.
    pub span: Span,
}

/// A dotted path such as `std.control.pid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    /// Segments in order.
    pub segments: Vec<Ident>,
    /// Location.
    pub span: Span,
}

impl Path {
    /// The path written with dots.
    pub fn dotted(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }
}

/// Dialect named in a module header (spec 09).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// Portable code.
    Core,
    /// Microcontrollers.
    Mcu,
    /// PLCs.
    Plc,
    /// FPGAs and digital logic.
    Hdl,
    /// Manipulators.
    Robot,
    /// Displays and HMIs.
    Ui,
    /// Linux SoCs and PCs.
    Host,
    /// Whole installation (cell files).
    Cell,
}

impl Dialect {
    /// Parses a dialect name used after `for`.
    pub fn from_name(name: &str) -> Option<Dialect> {
        Some(match name {
            "core" => Dialect::Core,
            "mcu" => Dialect::Mcu,
            "plc" => Dialect::Plc,
            "hdl" => Dialect::Hdl,
            "robot" => Dialect::Robot,
            "ui" => Dialect::Ui,
            "host" => Dialect::Host,
            _ => return None,
        })
    }

    /// Name as written in source.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Core => "core",
            Dialect::Mcu => "mcu",
            Dialect::Plc => "plc",
            Dialect::Hdl => "hdl",
            Dialect::Robot => "robot",
            Dialect::Ui => "ui",
            Dialect::Host => "host",
            Dialect::Cell => "cell",
        }
    }
}

/// A parsed `.neuron` file.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    /// Header line.
    pub header: Header,
    /// Top-level items.
    pub items: Vec<Item>,
    /// Whole file.
    pub span: Span,
}

/// `module a.b for mcu on board` or `cell Name`.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// Module path or cell name.
    pub name: Path,
    /// Declared dialect (`Core` by default, `Cell` for cell files).
    pub dialect: Dialect,
    /// Target after `on`.
    pub target: Option<Path>,
    /// Location.
    pub span: Span,
}

/// An attribute such as `@inline` or `@overflow(saturate)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// Name.
    pub name: Ident,
    /// Arguments.
    pub args: Vec<Arg>,
    /// Location.
    pub span: Span,
}

/// A top-level or nested item.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Attributes preceding the item.
    pub attrs: Vec<Attribute>,
    /// `pub` visibility.
    pub public: bool,
    /// Documentation comment text (`##` lines joined).
    pub doc: Option<String>,
    /// Item kind.
    pub kind: ItemKind,
    /// Location.
    pub span: Span,
}

/// Item kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    /// `use a.b.{c, d} as e`
    Use {
        /// Imported module path.
        path: Path,
        /// Selected names (`{c, d}`).
        names: Vec<Ident>,
        /// Rename.
        alias: Option<Ident>,
    },
    /// `const name: T = expr`
    Const {
        /// Name.
        name: Ident,
        /// Optional type.
        ty: Option<Type>,
        /// Value.
        value: Expr,
    },
    /// `var` / `let` at module level, `retain var`, and plc variables.
    Var(VarDecl),
    /// `signal name: T = expr`
    Signal {
        /// Name.
        name: Ident,
        /// Type.
        ty: Type,
        /// Initial value.
        init: Option<Expr>,
    },
    /// `unit name = unit_expr`
    Unit {
        /// Name.
        name: Ident,
        /// Definition.
        def: UnitExpr,
    },
    /// `event name: fields from device`
    Event {
        /// Name.
        name: Ident,
        /// Fields (`timestamp` is a field named `timestamp` without type).
        fields: Vec<(Ident, Option<Type>)>,
        /// Source device.
        from: Option<Ident>,
    },
    /// `struct`
    Struct(TypeDef<Field>),
    /// `enum`
    Enum(TypeDef<Variant>),
    /// `class`
    Class(ClassDecl),
    /// `interface`
    Interface {
        /// Name.
        name: Ident,
        /// Generic parameters.
        generics: Vec<GenericParam>,
        /// Method signatures.
        methods: Vec<FnSig>,
    },
    /// `type Name = T`
    Alias {
        /// Name.
        name: Ident,
        /// Generic parameters.
        generics: Vec<GenericParam>,
        /// Aliased type.
        ty: Type,
    },
    /// `fn`
    Fn(FnDecl),
    /// `task name every P (...)`
    Task(TaskDecl),
    /// `on interrupt source (...)`
    Interrupt {
        /// Hardware event.
        source: Expr,
        /// Attributes in parentheses.
        attrs: Vec<TaskAttr>,
        /// Body.
        body: Block,
    },
    /// `when cond for D:`
    Rule(Rule),
    /// `fsm`
    Fsm(FsmDecl),
    /// `sequence`
    Sequence(SequenceDecl),
    /// `safety:`
    Safety(Vec<SafetyMember>),
    /// `verify:`
    Verify(Vec<Property>),
    /// `test "name":`
    Test {
        /// Test name.
        name: String,
        /// Body.
        body: Block,
    },
    /// `io name: kind(config) @ location`
    Io {
        /// Binding name.
        name: Ident,
        /// Peripheral kind and configuration.
        kind: Expr,
        /// Location expression.
        location: Expr,
    },
    /// `clocks:`
    Clocks(Vec<ClockDef>),
    /// `uses regs.X, regs.Y`
    Uses(Vec<Path>),
    /// `trusted extern "C":` block of foreign signatures.
    ExternBlock {
        /// ABI string.
        abi: String,
        /// Signatures.
        fns: Vec<FnSig>,
    },
    /// `extern "C" fn` exported function.
    ExternFn {
        /// ABI string.
        abi: String,
        /// Function.
        func: FnDecl,
    },
    /// `trusted asm arch(...)`
    Asm(AsmBlock),
    /// `scan P` (plc)
    Scan(Expr),
    /// `inputs:` / `outputs:` / `memory:` (plc)
    PlcSection {
        /// Which section.
        section: PlcSection,
        /// Variables.
        vars: Vec<VarDecl>,
    },
    /// `rung name:` (plc)
    Rung {
        /// Name.
        name: Ident,
        /// Body.
        body: Block,
    },
    /// `clock name: freq @ pin` (hdl)
    Clock {
        /// Name.
        name: Ident,
        /// Frequency.
        freq: Expr,
        /// Pin.
        pin: Option<Expr>,
    },
    /// `circuit` (hdl)
    Circuit(CircuitDecl),
    /// `sync a -> b via method` (hdl)
    Sync {
        /// Source.
        from: Expr,
        /// Destination.
        to: Expr,
        /// Method.
        via: Expr,
    },
    /// `frame` / `tool` / `point` (robot)
    Geometry {
        /// Which kind.
        kind: GeometryKind,
        /// Name.
        name: Ident,
        /// Optional type.
        ty: Option<Type>,
        /// Value.
        value: Expr,
    },
    /// `display name: driver(...)` (ui)
    Display {
        /// Name.
        name: Ident,
        /// Driver.
        driver: Expr,
    },
    /// `screen Name on display:` (ui)
    Screen {
        /// Name.
        name: Ident,
        /// Display.
        display: Option<Ident>,
        /// Widgets.
        widgets: Vec<Widget>,
    },
    /// `theme Name:` (ui)
    Theme {
        /// Name.
        name: Ident,
        /// Body.
        body: Block,
    },
    /// `device name: type @ address` (cell)
    Device {
        /// Name.
        name: Ident,
        /// Device type.
        ty: Path,
        /// Address.
        address: Option<Expr>,
    },
    /// `place module on device` (cell)
    Place {
        /// Module.
        module: Path,
        /// Device.
        device: Ident,
    },
    /// `link a -> b via m every P within L` (cell)
    Link(LinkDecl),
    /// `tag name: T @ loc, loc` (cell)
    Tag {
        /// Name.
        name: Ident,
        /// Type.
        ty: Type,
        /// Locations on devices.
        locations: Vec<Expr>,
    },
    /// `zone name: shape` (cell)
    Zone {
        /// Name.
        name: Ident,
        /// Shape.
        shape: Expr,
    },
    /// `mutex zone between a, b` (cell)
    Mutex {
        /// Zone.
        zone: Ident,
        /// Participants.
        parties: Vec<Expr>,
    },
}

/// `var`/`let` declaration, also used for plc section variables and class members.
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    /// `true` for `var` (mutable), `false` for `let`.
    pub mutable: bool,
    /// `retain` (plc).
    pub retain: bool,
    /// Name.
    pub name: Ident,
    /// Type.
    pub ty: Option<Type>,
    /// Initial value.
    pub init: Option<Expr>,
    /// Placement (`@ %I0.0`, `@ addr`).
    pub at: Option<Expr>,
    /// Location.
    pub span: Span,
}

/// plc variable sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlcSection {
    /// `inputs:`
    Inputs,
    /// `outputs:`
    Outputs,
    /// `memory:`
    Memory,
}

/// robot geometry items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryKind {
    /// `frame`
    Frame,
    /// `tool`
    Tool,
    /// `point`
    Point,
}

/// Generic parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum GenericParam {
    /// `T: Bound + Bound`
    Type {
        /// Name.
        name: Ident,
        /// Bounds.
        bounds: Vec<Path>,
    },
    /// `const N: index`
    Const {
        /// Name.
        name: Ident,
        /// Type.
        ty: Type,
    },
}

/// A struct-like or enum-like type definition.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef<M> {
    /// Name.
    pub name: Ident,
    /// Generic parameters.
    pub generics: Vec<GenericParam>,
    /// Underlying representation (enums: `enum E: u8`).
    pub repr: Option<Type>,
    /// Members.
    pub members: Vec<M>,
}

/// Struct field.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// Name.
    pub name: Ident,
    /// Type.
    pub ty: Type,
    /// Default.
    pub default: Option<Expr>,
}

/// Enum variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    /// Name.
    pub name: Ident,
    /// Payload types.
    pub payload: Vec<Type>,
    /// Explicit discriminant.
    pub value: Option<Expr>,
}

/// `class`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    /// Name.
    pub name: Ident,
    /// Generic parameters.
    pub generics: Vec<GenericParam>,
    /// Implemented interfaces (`is A, B`).
    pub interfaces: Vec<Path>,
    /// State members.
    pub vars: Vec<VarDecl>,
    /// Methods.
    pub methods: Vec<Item>,
    /// `invariant` clauses.
    pub invariants: Vec<Expr>,
}

/// Function parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// `mut` (exclusive).
    pub mutable: bool,
    /// Name (`self` for methods).
    pub name: Ident,
    /// Type (`None` for `self`).
    pub ty: Option<Type>,
    /// Default value.
    pub default: Option<Expr>,
}

/// Function signature.
#[derive(Debug, Clone, PartialEq)]
pub struct FnSig {
    /// Name.
    pub name: Ident,
    /// Generic parameters.
    pub generics: Vec<GenericParam>,
    /// Parameters.
    pub params: Vec<Param>,
    /// Return type.
    pub ret: Option<Type>,
    /// Location.
    pub span: Span,
}

/// Function body.
#[derive(Debug, Clone, PartialEq)]
pub enum FnBody {
    /// Block body.
    Block(Block),
    /// `= expr`.
    Expr(Expr),
}

/// `fn` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    /// Signature.
    pub sig: FnSig,
    /// Contracts at the start of the body.
    pub contracts: Vec<Contract>,
    /// Body.
    pub body: FnBody,
}

/// `requires` / `ensures` / `invariant`.
#[derive(Debug, Clone, PartialEq)]
pub struct Contract {
    /// Kind.
    pub kind: ContractKind,
    /// Condition.
    pub cond: Expr,
}

/// Contract kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractKind {
    /// Precondition.
    Requires,
    /// Postcondition.
    Ensures,
    /// Class invariant.
    Invariant,
}

/// Task trigger.
#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// `every P offset O`
    Every {
        /// Period.
        period: Expr,
        /// Phase.
        offset: Option<Expr>,
    },
    /// `on E`
    On(Expr),
}

/// Attribute inside `( ... )` after a task header.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskAttr {
    /// `prio N`
    Prio(Expr),
    /// `deadline D`
    Deadline(Expr),
    /// `core C`
    Core(Expr),
    /// `stack S`
    Stack(Expr),
    /// `reads a b`
    Reads(Vec<Ident>),
    /// `writes a b`
    Writes(Vec<Ident>),
}

/// `task`.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskDecl {
    /// Name.
    pub name: Ident,
    /// Trigger.
    pub trigger: Trigger,
    /// Attributes.
    pub attrs: Vec<TaskAttr>,
    /// Body.
    pub body: Block,
}

/// `when cond for D:` rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// Condition.
    pub cond: Expr,
    /// Hold duration.
    pub hold: Option<Expr>,
    /// Body.
    pub body: Block,
    /// Location.
    pub span: Span,
}

/// `fsm`.
#[derive(Debug, Clone, PartialEq)]
pub struct FsmDecl {
    /// Name.
    pub name: Ident,
    /// Evaluation trigger.
    pub trigger: Option<Trigger>,
    /// Local state variables.
    pub vars: Vec<VarDecl>,
    /// States.
    pub states: Vec<State>,
}

/// One state.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// `initial`.
    pub initial: bool,
    /// `final`.
    pub is_final: bool,
    /// Name.
    pub name: Ident,
    /// `entry:`
    pub entry: Option<Block>,
    /// `exit:`
    pub exit: Option<Block>,
    /// `do:`
    pub during: Option<Block>,
    /// Outgoing transitions in source order.
    pub transitions: Vec<Transition>,
    /// Location.
    pub span: Span,
}

/// `-> Target when cond or after D`
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    /// Destination state.
    pub target: Ident,
    /// Guard.
    pub when: Option<Expr>,
    /// Timeout since entry.
    pub after: Option<Expr>,
    /// Action executed when the transition fires.
    pub action: Option<Block>,
    /// Location.
    pub span: Span,
}

/// `sequence`.
#[derive(Debug, Clone, PartialEq)]
pub struct SequenceDecl {
    /// Name.
    pub name: Ident,
    /// Parameters.
    pub params: Vec<Param>,
    /// Device it runs on (cell).
    pub on: Option<Ident>,
    /// Body.
    pub items: Vec<SeqItem>,
}

/// Item in a sequence body.
#[derive(Debug, Clone, PartialEq)]
pub enum SeqItem {
    /// `step Name:`
    Step {
        /// `initial`.
        initial: bool,
        /// Name.
        name: Ident,
        /// Body.
        body: Vec<SeqItem>,
        /// Location.
        span: Span,
    },
    /// `parallel:` with branches.
    Parallel {
        /// Branches.
        branches: Vec<(Ident, Vec<SeqItem>)>,
        /// Location.
        span: Span,
    },
    /// Any statement (including `until`, `wait`, `->`, `move`).
    Stmt(Stmt),
}

/// Member of a `safety:` block.
#[derive(Debug, Clone, PartialEq)]
pub enum SafetyMember {
    /// `rate D`
    Rate(Expr),
    /// `watchdog D`
    Watchdog(Expr),
    /// `safe_state:`
    SafeState(Block),
    /// `when ...:`
    Rule(Rule),
    /// `on fault [Name]:`
    OnFault {
        /// Specific fault.
        fault: Option<Path>,
        /// Body.
        body: Block,
    },
    /// `reset when cond`
    Reset(Expr),
}

/// Property in a `verify:` block.
#[derive(Debug, Clone, PartialEq)]
pub enum Property {
    /// `never cond`
    Never(Expr),
    /// `always cond [implies cond]`
    Always {
        /// Condition.
        cond: Expr,
        /// Consequence.
        implies: Option<Expr>,
    },
    /// `eventually_within D after E: cond`
    EventuallyWithin {
        /// Bound.
        within: Expr,
        /// Trigger.
        after: Expr,
        /// Goal.
        goal: Expr,
    },
}

/// One line of `clocks:`.
#[derive(Debug, Clone, PartialEq)]
pub struct ClockDef {
    /// Clock name.
    pub name: Ident,
    /// Requested frequency or derivation.
    pub value: Expr,
    /// Source (`from hse(8 MHz)`).
    pub from: Option<Expr>,
}

/// Inline assembly block.
#[derive(Debug, Clone, PartialEq)]
pub struct AsmBlock {
    /// Architecture name.
    pub arch: Ident,
    /// Operand list text between the parentheses (checked by the backend).
    pub operands: String,
    /// Raw assembly lines.
    pub lines: Vec<String>,
}

/// hdl circuit port direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDir {
    /// `in`
    In,
    /// `out`
    Out,
    /// `inout`
    InOut,
}

/// hdl port.
#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    /// Name.
    pub name: Ident,
    /// Direction.
    pub dir: PortDir,
    /// Type.
    pub ty: Type,
    /// Pin.
    pub pin: Option<Expr>,
}

/// hdl `circuit`.
#[derive(Debug, Clone, PartialEq)]
pub struct CircuitDecl {
    /// Name.
    pub name: Ident,
    /// Ports.
    pub ports: Vec<Port>,
    /// Clock domain.
    pub clock: Option<Ident>,
    /// Members.
    pub members: Vec<CircuitMember>,
}

/// Member of a circuit.
#[derive(Debug, Clone, PartialEq)]
pub enum CircuitMember {
    /// `reg name: T = init`
    Reg {
        /// Name.
        name: Ident,
        /// Type.
        ty: Type,
        /// Reset value.
        init: Option<Expr>,
    },
    /// `wire name: T`
    Wire {
        /// Name.
        name: Ident,
        /// Type.
        ty: Type,
    },
    /// `const`
    Const {
        /// Name.
        name: Ident,
        /// Type.
        ty: Option<Type>,
        /// Value.
        value: Expr,
    },
    /// `instance u = circuit(args)`
    Instance {
        /// Instance name.
        name: Ident,
        /// Instantiation.
        of: Expr,
    },
    /// `comb:`
    Comb(Block),
    /// `on rising(clk):` / `on falling(clk):`
    OnEdge {
        /// `true` for rising.
        rising: bool,
        /// Clock.
        clock: Ident,
        /// Body.
        body: Block,
    },
    /// Nested `fsm`.
    Fsm(FsmDecl),
}

/// ui widget.
#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    /// Widget kind (`label`, `button`, ...).
    pub kind: Ident,
    /// Optional instance name.
    pub name: Option<Ident>,
    /// Optional text.
    pub text: Option<String>,
    /// Options (`at`, `bind`, `range`, ...).
    pub options: Vec<(Ident, Expr)>,
    /// Children.
    pub children: Vec<Widget>,
    /// Event handlers.
    pub events: Vec<(Ident, Block)>,
    /// Location.
    pub span: Span,
}

/// cell `link`.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkDecl {
    /// Source signal.
    pub from: Expr,
    /// Destination signal.
    pub to: Expr,
    /// Medium.
    pub via: Expr,
    /// Period.
    pub every: Option<Expr>,
    /// Latency budget.
    pub within: Option<Expr>,
}

// ---- types --------------------------------------------------------------

/// A type expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    /// Kind.
    pub kind: TypeKind,
    /// Physical unit.
    pub unit: Option<UnitExpr>,
    /// `?` suffix.
    pub optional: bool,
    /// Location.
    pub span: Span,
}

/// Type kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// Named type with generic arguments: `u8`, `fixed<16, 16>`, `Result<T, E>`.
    Named {
        /// Path.
        path: Path,
        /// Arguments.
        args: Vec<TypeArg>,
    },
    /// `[T; N]`
    Array(Box<Type>, Box<Expr>),
    /// `[T]` (parameters only)
    Slice(Box<Type>),
    /// `(A, B)`
    Tuple(Vec<Type>),
    /// A bare unit used as a type (`degC` means `f32 degC`).
    Unit,
}

/// Generic argument.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeArg {
    /// Type argument.
    Type(Type),
    /// Constant argument.
    Expr(Expr),
}

/// Unit expression: `mm/s`, `m/s^2`, `N*m`, `%`.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitExpr {
    /// Factors with exponents (division gives negative exponents).
    pub factors: Vec<(Ident, i32)>,
    /// Location.
    pub span: Span,
}

impl UnitExpr {
    /// Canonical text (`m*s^-2`).
    pub fn display(&self) -> String {
        self.factors
            .iter()
            .map(|(u, e)| {
                if *e == 1 {
                    u.name.clone()
                } else {
                    format!("{}^{e}", u.name)
                }
            })
            .collect::<Vec<_>>()
            .join("*")
    }
}

// ---- statements ---------------------------------------------------------

/// Indented block of statements.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Contracts at the start of the block (functions only).
    pub contracts: Vec<Contract>,
    /// Statements.
    pub stmts: Vec<Stmt>,
    /// Location.
    pub span: Span,
}

/// A statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    /// Kind.
    pub kind: StmtKind,
    /// Location.
    pub span: Span,
}

/// Assignment operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    /// `=`
    Set,
    /// `+=` and the other compound forms.
    Compound(BinOp),
}

/// Statement kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// `let` / `var` binding.
    Let {
        /// `var`.
        mutable: bool,
        /// Pattern.
        pattern: Pattern,
        /// Type.
        ty: Option<Type>,
        /// Value.
        value: Expr,
    },
    /// `place op= value [when cond]`
    Assign {
        /// Target.
        place: Expr,
        /// Operator.
        op: AssignOp,
        /// Value.
        value: Expr,
        /// Conditional assignment (plc `count += 1 when edge`).
        when: Option<Expr>,
    },
    /// Expression statement.
    Expr(Expr),
    /// `pass`
    Pass,
    /// `return [expr]`
    Return(Option<Expr>),
    /// `break`
    Break,
    /// `continue`
    Continue,
    /// `fault Name(args)`
    Fault {
        /// Fault name.
        name: Path,
        /// Arguments.
        args: Vec<Arg>,
    },
    /// `assert cond, "msg"`
    Assert {
        /// Condition.
        cond: Expr,
        /// Message.
        msg: Option<String>,
    },
    /// `emit Event(args)`
    Emit {
        /// Event.
        event: Path,
        /// Payload.
        args: Vec<Arg>,
    },
    /// plc `set x when c` / `reset x when c`
    Coil {
        /// `true` for `set`.
        set: bool,
        /// Target.
        place: Expr,
        /// Condition.
        when: Expr,
    },
    /// `wait D` (sequences)
    Wait(Expr),
    /// `until cond timeout D -> Target`
    Until {
        /// Condition.
        cond: Expr,
        /// Timeout.
        timeout: Option<Expr>,
        /// Jump on timeout.
        on_timeout: Option<Ident>,
    },
    /// `-> Target`
    Jump(Ident),
    /// robot motion.
    Move(Move),
    /// `if`
    If {
        /// `(condition, block)` for `if` and each `elif`.
        branches: Vec<(Expr, Block)>,
        /// `else`.
        otherwise: Option<Block>,
    },
    /// `match`
    Match {
        /// Scrutinee.
        value: Expr,
        /// Arms.
        arms: Vec<Arm>,
    },
    /// `for pattern in expr:`
    For {
        /// Pattern.
        pattern: Pattern,
        /// Iterated expression.
        iter: Expr,
        /// Body.
        body: Block,
    },
    /// `while cond max N:` / `while let p = e max N:`
    While {
        /// Pattern for `while let`.
        pattern: Option<Pattern>,
        /// Condition or scrutinee.
        cond: Expr,
        /// Bound.
        max: Option<Expr>,
        /// Body.
        body: Block,
    },
    /// `loop max N:`
    Loop {
        /// Bound.
        max: Option<Expr>,
        /// Body.
        body: Block,
    },
    /// `atomic:`
    Atomic(Block),
    /// Nested rule.
    Rule(Rule),
    /// Inline assembly.
    Asm(AsmBlock),
}

/// robot `move`.
#[derive(Debug, Clone, PartialEq)]
pub struct Move {
    /// Interpolation.
    pub kind: MoveKind,
    /// Target.
    pub to: Expr,
    /// Intermediate point for arcs.
    pub via: Option<Expr>,
    /// Options (`speed`, `accel`, `blend`, `tool`, `frame`).
    pub options: Vec<(Ident, Expr)>,
}

/// Interpolation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    /// Point to point (joint).
    Ptp,
    /// Straight line.
    Linear,
    /// Circular arc.
    Arc,
}

/// `match` arm.
#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    /// Pattern.
    pub pattern: Pattern,
    /// Guard.
    pub guard: Option<Expr>,
    /// Body.
    pub body: Block,
    /// Location.
    pub span: Span,
}

/// Patterns.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    /// Kind.
    pub kind: PatternKind,
    /// Location.
    pub span: Span,
}

/// Pattern kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind {
    /// `_`
    Wildcard,
    /// Binding.
    Bind(Ident),
    /// Literal.
    Lit(Expr),
    /// `a..b` / `a..=b`
    Range {
        /// Start.
        lo: Expr,
        /// End.
        hi: Expr,
        /// `..=`
        inclusive: bool,
    },
    /// `Path(p, q)` enum variant or `Path(x: p)` struct.
    Constructor {
        /// Path.
        path: Path,
        /// Positional sub-patterns.
        args: Vec<Pattern>,
        /// Named sub-patterns.
        fields: Vec<(Ident, Pattern)>,
    },
    /// Bare path (enum variant without payload, constant).
    Path(Path),
    /// `(a, b)`
    Tuple(Vec<Pattern>),
    /// `A | B`
    Or(Vec<Pattern>),
}

// ---- expressions --------------------------------------------------------

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    /// `or`
    Or,
    /// `and`
    And,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `in`
    In,
    /// `|`
    BitOr,
    /// `^`
    BitXor,
    /// `&`
    BitAnd,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `+%`
    AddWrap,
    /// `-%`
    SubWrap,
    /// `+|`
    AddSat,
    /// `-|`
    SubSat,
    /// `++` (bit concatenation)
    Concat,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `*%`
    MulWrap,
    /// `*|`
    MulSat,
    /// `**`
    Pow,
    /// `??`
    Coalesce,
}

impl BinOp {
    /// Source text.
    pub fn as_str(self) -> &'static str {
        match self {
            BinOp::Or => "or",
            BinOp::And => "and",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::In => "in",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::BitAnd => "&",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::AddWrap => "+%",
            BinOp::SubWrap => "-%",
            BinOp::AddSat => "+|",
            BinOp::SubSat => "-|",
            BinOp::Concat => "++",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::MulWrap => "*%",
            BinOp::MulSat => "*|",
            BinOp::Pow => "**",
            BinOp::Coalesce => "??",
        }
    }
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `-`
    Neg,
    /// `~`
    BitNot,
    /// `not`
    Not,
}

/// Literal values (kept as source text; evaluated by the semantic phase).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lit {
    /// Integer.
    Int(String),
    /// Real.
    Real(String),
    /// String (with quotes and escapes as written).
    Str(String),
    /// Raw string.
    RawStr(String),
    /// Byte string.
    ByteStr(String),
    /// Character.
    Char(String),
    /// `true` / `false`.
    Bool(bool),
    /// `none`.
    None,
    /// IEC address (`%I0.0`).
    Iec(String),
    /// Bit vector (`8'b1010`).
    Bits(String),
}

/// Call or construction argument.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// `mut` at call site.
    pub mutable: bool,
    /// Name for named arguments.
    pub name: Option<Ident>,
    /// Value.
    pub value: Expr,
}

/// An expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    /// Kind.
    pub kind: ExprKind,
    /// Location.
    pub span: Span,
}

/// Expression kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// Literal.
    Lit(Lit),
    /// Number with unit: `5 ms`, `xyz(1, 2, 3) mm`.
    Quantity(Box<Expr>, UnitExpr),
    /// Name.
    Name(Ident),
    /// `self`.
    SelfValue,
    /// `result` in postconditions.
    Result,
    /// Binary operation.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Chained comparison `a <= x < b` (operators and operands in order).
    Compare(Box<Expr>, Vec<(BinOp, Expr)>),
    /// Unary operation.
    Unary(UnOp, Box<Expr>),
    /// `a..b` / `a..=b`
    Range {
        /// Start.
        lo: Box<Expr>,
        /// End.
        hi: Box<Expr>,
        /// `..=`
        inclusive: bool,
    },
    /// Linear map `a..b -> c..d` (arguments only).
    Map(Box<Expr>, Box<Expr>),
    /// `if c then a else b`
    IfElse(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `x |> f(a)` (kept for faithful printing; equals `f(x, a)`).
    Pipe(Box<Expr>, Box<Expr>),
    /// `f(args)`
    Call(Box<Expr>, Vec<Arg>),
    /// `x[i]` and hdl slice `x[7:0]`.
    Index(Box<Expr>, Box<Expr>, Option<Box<Expr>>),
    /// `x.name` or `t.0`.
    Field(Box<Expr>, Ident),
    /// `x?`
    Try(Box<Expr>),
    /// `x as T`
    Cast(Box<Expr>, Type),
    /// `(a, b)`
    Tuple(Vec<Expr>),
    /// `[a, b]`
    Array(Vec<Expr>),
    /// `[v; n]`
    ArrayRepeat(Box<Expr>, Box<Expr>),
    /// `fn(x) => body`
    Lambda(Vec<Ident>, Box<Expr>),
    /// `old(x)` in postconditions.
    Old(Box<Expr>),
    /// `run action` (async action handle).
    Run(Box<Expr>),
    /// Placeholder after a syntax error.
    Error,
}

impl Expr {
    /// Creates an expression.
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}
