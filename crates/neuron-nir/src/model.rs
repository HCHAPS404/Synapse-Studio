//! The NIR data model (spec 11 §11.1, spec 16).
//!
//! The model mirrors the source structure so that it can be printed back to `.neuron`
//! (property P-RT), and adds what the compiler knows: stable ids for every definition,
//! resolved references, and the type and unit of every expression.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Byte range `[start, end)` into the source file of the module.
pub type Span = [usize; 2];

fn is_false(b: &bool) -> bool {
    !*b
}

/// A whole package lowered to NIR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Nir {
    /// Version of this format (`1.0`).
    pub nir_version: String,
    /// Language edition (`2026`).
    pub edition: String,
    /// Package name from `neuron.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// One entry per source file.
    pub modules: Vec<Module>,
}

/// File kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModuleKind {
    /// `module a.b for dialect`.
    Module,
    /// `cell Name`.
    Cell,
}

/// One source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Module {
    /// Stable id (`m0`).
    pub id: String,
    /// Module path or cell name.
    pub name: String,
    /// Path of the source file.
    pub file: String,
    /// Module or cell.
    pub kind: ModuleKind,
    /// Dialect name.
    pub dialect: String,
    /// Target after `on`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Items in source order.
    pub items: Vec<Item>,
}

/// A top-level item (or a method of a class).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Item {
    /// Stable id (`m0.i3`).
    pub id: String,
    /// Location.
    pub span: Span,
    /// `pub`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub public: bool,
    /// Documentation comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// Attributes such as `@min_interval(5 ms)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attrs: Vec<Attr>,
    /// The definition.
    pub def: ItemDef,
}

/// Attribute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Attr {
    /// Name without `@`.
    pub name: String,
    /// Arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
}

/// Call or construction argument.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Arg {
    /// `mut` at the call site.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mutable: bool,
    /// Name of a named argument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Value.
    pub value: Expr,
}

/// Item definitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "item", rename_all = "snake_case")]
pub enum ItemDef {
    /// `use a.b.{c, d} as e`.
    Use {
        /// Path segments.
        path: Vec<String>,
        /// Selected names.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        names: Vec<String>,
        /// Rename.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alias: Option<String>,
    },
    /// `const`.
    Const {
        /// Name.
        name: String,
        /// Declared type.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ty: Option<TypeExpr>,
        /// Value.
        value: Expr,
    },
    /// `var` / `let` / `retain var` at module level.
    Var {
        /// The variable.
        var: VarDecl,
    },
    /// `signal`.
    Signal {
        /// Name.
        name: String,
        /// Type.
        ty: TypeExpr,
        /// Initial value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        init: Option<Expr>,
    },
    /// `unit name = expr`.
    Unit {
        /// Name.
        name: String,
        /// Definition.
        def: UnitRef,
    },
    /// `event`.
    Event {
        /// Name.
        name: String,
        /// Payload fields.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<EventField>,
        /// Source device.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    /// `struct`.
    Struct {
        /// Name.
        name: String,
        /// Generic parameters.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        generics: Vec<Generic>,
        /// Fields.
        fields: Vec<Field>,
    },
    /// `enum`.
    Enum {
        /// Name.
        name: String,
        /// Generic parameters.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        generics: Vec<Generic>,
        /// Representation type.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repr: Option<TypeExpr>,
        /// Variants.
        variants: Vec<Variant>,
    },
    /// `class`.
    Class {
        /// Name.
        name: String,
        /// Generic parameters.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        generics: Vec<Generic>,
        /// Interfaces after `is`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        interfaces: Vec<String>,
        /// State fields.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        vars: Vec<VarDecl>,
        /// `invariant` clauses.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        invariants: Vec<Expr>,
        /// Methods (items whose definition is `fn`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        methods: Vec<Item>,
    },
    /// `interface`.
    Interface {
        /// Name.
        name: String,
        /// Generic parameters.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        generics: Vec<Generic>,
        /// Method signatures.
        methods: Vec<FnSig>,
    },
    /// `type Name = T`.
    Alias {
        /// Name.
        name: String,
        /// Generic parameters.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        generics: Vec<Generic>,
        /// Aliased type.
        ty: TypeExpr,
    },
    /// `fn`.
    Fn {
        /// The function.
        func: FnDecl,
    },
    /// `task`.
    Task {
        /// Name.
        name: String,
        /// Release trigger.
        trigger: Trigger,
        /// Attributes in parentheses.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attrs: Vec<TaskAttr>,
        /// Body.
        body: Block,
    },
    /// `on interrupt`.
    Interrupt {
        /// Hardware event.
        source: Expr,
        /// Attributes in parentheses.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attrs: Vec<TaskAttr>,
        /// Body.
        body: Block,
    },
    /// `when` rule.
    Rule {
        /// The rule.
        rule: Rule,
    },
    /// `fsm`.
    Fsm {
        /// The state machine.
        fsm: Fsm,
    },
    /// `sequence`.
    Sequence {
        /// Name.
        name: String,
        /// Parameters.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        params: Vec<Param>,
        /// Device (cell files).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<String>,
        /// Body.
        items: Vec<SeqItem>,
    },
    /// `safety:`.
    Safety {
        /// Members in source order.
        members: Vec<SafetyMember>,
    },
    /// `verify:`.
    Verify {
        /// Properties.
        props: Vec<Prop>,
    },
    /// `test "name":`.
    Test {
        /// Name, with its quotes.
        name: String,
        /// Body.
        body: Block,
    },
    /// `io name: kind @ location`.
    Io {
        /// Binding name.
        name: String,
        /// Peripheral kind and configuration.
        kind: Expr,
        /// Location.
        at: Expr,
    },
    /// `clocks:`.
    Clocks {
        /// Clock definitions.
        clocks: Vec<ClockDef>,
    },
    /// `uses regs.X`.
    Uses {
        /// Dotted paths.
        paths: Vec<String>,
    },
    /// `trusted extern "C":`.
    ExternBlock {
        /// ABI string with quotes.
        abi: String,
        /// Signatures.
        fns: Vec<FnSig>,
    },
    /// `extern "C" fn`.
    ExternFn {
        /// ABI string with quotes.
        abi: String,
        /// Function.
        func: FnDecl,
    },
    /// `trusted asm`.
    Asm {
        /// The block.
        asm: Asm,
    },
    /// `scan P` (plc).
    Scan {
        /// Scan period.
        period: Expr,
    },
    /// `inputs:` / `outputs:` / `memory:` (plc).
    PlcSection {
        /// `inputs`, `outputs` or `memory`.
        section: String,
        /// Variables.
        vars: Vec<VarDecl>,
    },
    /// `rung` (plc).
    Rung {
        /// Name.
        name: String,
        /// Body.
        body: Block,
    },
    /// `clock` (hdl).
    Clock {
        /// Name.
        name: String,
        /// Frequency.
        freq: Expr,
        /// Pin.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pin: Option<Expr>,
    },
    /// `circuit` (hdl).
    Circuit {
        /// The circuit.
        circuit: Circuit,
    },
    /// `sync a -> b via m` (hdl).
    Sync {
        /// Source.
        from: Expr,
        /// Destination.
        to: Expr,
        /// Method.
        via: Expr,
    },
    /// `frame` / `tool` / `point` (robot).
    Geometry {
        /// `frame`, `tool` or `point`.
        kind: String,
        /// Name.
        name: String,
        /// Declared type.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ty: Option<TypeExpr>,
        /// Value.
        value: Expr,
    },
    /// `display` (ui).
    Display {
        /// Name.
        name: String,
        /// Driver.
        driver: Expr,
    },
    /// `screen` (ui).
    Screen {
        /// Name.
        name: String,
        /// Display.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<String>,
        /// Widgets.
        widgets: Vec<Widget>,
    },
    /// `theme` (ui).
    Theme {
        /// Name.
        name: String,
        /// Body.
        body: Block,
    },
    /// `device` (cell).
    Device {
        /// Name.
        name: String,
        /// Device type (dotted path).
        ty: String,
        /// Address.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address: Option<Expr>,
    },
    /// `place module on device` (cell).
    Place {
        /// Module path.
        module: String,
        /// Device.
        device: String,
    },
    /// `link` (cell).
    Link {
        /// Source signal.
        from: Expr,
        /// Destination signal.
        to: Expr,
        /// Medium.
        via: Expr,
        /// Period.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        every: Option<Expr>,
        /// Latency budget.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        within: Option<Expr>,
    },
    /// `tag` (cell).
    Tag {
        /// Name.
        name: String,
        /// Type.
        ty: TypeExpr,
        /// Locations on devices.
        locations: Vec<Expr>,
    },
    /// `zone` (cell).
    Zone {
        /// Name.
        name: String,
        /// Shape.
        shape: Expr,
    },
    /// `mutex zone between a, b` (cell).
    Mutex {
        /// Zone.
        zone: String,
        /// Participants.
        parties: Vec<Expr>,
    },
}

/// A variable declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VarDecl {
    /// Stable id (item id, local id `l3`, or field id).
    pub id: String,
    /// `var` (true) or `let` (false).
    pub mutable: bool,
    /// `retain` (plc).
    #[serde(default, skip_serializing_if = "is_false")]
    pub retain: bool,
    /// Name.
    pub name: String,
    /// Declared type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<TypeExpr>,
    /// Resolved type.
    pub vty: Type,
    /// Initial value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<Expr>,
    /// Placement (`@ %I0.0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Expr>,
    /// Location.
    pub span: Span,
}

/// Event payload field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventField {
    /// Name (`timestamp` has no type).
    pub name: String,
    /// Type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<TypeExpr>,
}

/// Generic parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "param", rename_all = "snake_case")]
pub enum Generic {
    /// `T: Bound + Bound`.
    Type {
        /// Name.
        name: String,
        /// Bounds (dotted paths).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bounds: Vec<String>,
    },
    /// `const N: index`.
    Const {
        /// Name.
        name: String,
        /// Type.
        ty: TypeExpr,
    },
}

/// Struct field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Field {
    /// Name.
    pub name: String,
    /// Type.
    pub ty: TypeExpr,
    /// Default value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Expr>,
    /// Location.
    pub span: Span,
}

/// Enum variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Variant {
    /// Name.
    pub name: String,
    /// Payload types.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub payload: Vec<TypeExpr>,
    /// Explicit discriminant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Expr>,
    /// Location.
    pub span: Span,
}

/// Function or method parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Param {
    /// `mut`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mutable: bool,
    /// Name (`self` for the receiver).
    pub name: String,
    /// Local id (none for `self`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Type (none for `self`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<TypeExpr>,
    /// Default value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Expr>,
}

/// Function signature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FnSig {
    /// Name.
    pub name: String,
    /// Generic parameters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generics: Vec<Generic>,
    /// Parameters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<Param>,
    /// Return type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ret: Option<TypeExpr>,
    /// Location.
    pub span: Span,
}

/// Function declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FnDecl {
    /// Signature.
    pub sig: FnSig,
    /// `requires` / `ensures` clauses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contracts: Vec<Contract>,
    /// Body.
    pub body: FnBody,
}

/// Function body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "body", rename_all = "snake_case")]
pub enum FnBody {
    /// Indented block.
    Block {
        /// The block.
        block: Block,
    },
    /// `= expr`.
    Expr {
        /// The expression.
        expr: Expr,
    },
}

/// Contract clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Contract {
    /// `requires`, `ensures` or `invariant`.
    pub kind: String,
    /// Condition.
    pub cond: Expr,
    /// Location.
    pub span: Span,
}

/// Task trigger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "trigger", rename_all = "snake_case")]
pub enum Trigger {
    /// `every P offset O`.
    Every {
        /// Period.
        period: Expr,
        /// Phase.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<Expr>,
    },
    /// `on E`.
    On {
        /// Event.
        event: Expr,
    },
}

/// Task attribute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "attr", rename_all = "snake_case")]
pub enum TaskAttr {
    /// `prio N`.
    Prio {
        /// Value.
        value: Expr,
    },
    /// `deadline D`.
    Deadline {
        /// Value.
        value: Expr,
    },
    /// `core C`.
    Core {
        /// Value.
        value: Expr,
    },
    /// `stack S`.
    Stack {
        /// Value.
        value: Expr,
    },
    /// `reads a b`.
    Reads {
        /// Names.
        names: Vec<String>,
    },
    /// `writes a b`.
    Writes {
        /// Names.
        names: Vec<String>,
    },
}

/// `when cond for D:` rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    /// Condition.
    pub cond: Expr,
    /// Hold duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<Expr>,
    /// Body.
    pub body: Block,
    /// Location.
    pub span: Span,
}

/// State machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Fsm {
    /// Name.
    pub name: String,
    /// Evaluation trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Trigger>,
    /// Local variables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vars: Vec<VarDecl>,
    /// States in source order.
    pub states: Vec<State>,
}

/// One state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct State {
    /// `initial`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub initial: bool,
    /// `final`.
    #[serde(rename = "final", default, skip_serializing_if = "is_false")]
    pub is_final: bool,
    /// Name.
    pub name: String,
    /// `entry:`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<Block>,
    /// `exit:`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<Block>,
    /// `do:`.
    #[serde(rename = "do", default, skip_serializing_if = "Option::is_none")]
    pub during: Option<Block>,
    /// Transitions in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<Transition>,
    /// Location.
    pub span: Span,
}

/// `-> Target when cond or after D`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Transition {
    /// Destination state.
    pub target: String,
    /// Guard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Expr>,
    /// Timeout since entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Expr>,
    /// Action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Block>,
    /// Location.
    pub span: Span,
}

/// Item of a sequence body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "seq", rename_all = "snake_case")]
pub enum SeqItem {
    /// `step Name:`.
    Step {
        /// `initial`.
        #[serde(default, skip_serializing_if = "is_false")]
        initial: bool,
        /// Name.
        name: String,
        /// Body.
        body: Vec<SeqItem>,
        /// Location.
        span: Span,
    },
    /// `parallel:`.
    Parallel {
        /// Branches.
        branches: Vec<SeqBranch>,
        /// Location.
        span: Span,
    },
    /// A statement.
    Stmt {
        /// The statement.
        stmt: Stmt,
    },
}

/// `branch name:` of a parallel block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SeqBranch {
    /// Name.
    pub name: String,
    /// Body.
    pub items: Vec<SeqItem>,
}

/// Member of `safety:`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SafetyMember {
    /// Location.
    pub span: Span,
    /// The member.
    pub def: SafetyDef,
}

/// Kinds of safety members.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "member", rename_all = "snake_case")]
pub enum SafetyDef {
    /// `rate D`.
    Rate {
        /// Value.
        value: Expr,
    },
    /// `watchdog D`.
    Watchdog {
        /// Value.
        value: Expr,
    },
    /// `safe_state:`.
    SafeState {
        /// Body.
        body: Block,
    },
    /// `when ...:`.
    Rule {
        /// The rule.
        rule: Rule,
    },
    /// `on fault [Name]:`.
    OnFault {
        /// Specific fault.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fault: Option<String>,
        /// Body.
        body: Block,
    },
    /// `reset when cond`.
    Reset {
        /// Condition.
        cond: Expr,
    },
}

/// A `verify:` property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Prop {
    /// Location.
    pub span: Span,
    /// The property.
    pub def: PropDef,
}

/// Property kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "prop", rename_all = "snake_case")]
pub enum PropDef {
    /// `never cond`.
    Never {
        /// Condition.
        cond: Expr,
    },
    /// `always cond [implies cond]`.
    Always {
        /// Condition.
        cond: Expr,
        /// Consequence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        implies: Option<Expr>,
    },
    /// `eventually_within D after E: G`.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClockDef {
    /// Stable id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Requested frequency or derivation.
    pub value: Expr,
    /// Source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Expr>,
    /// Location.
    pub span: Span,
}

/// Inline assembly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Asm {
    /// Architecture.
    pub arch: String,
    /// Operand list as written.
    pub operands: String,
    /// Raw lines.
    pub lines: Vec<String>,
}

/// hdl circuit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Circuit {
    /// Name.
    pub name: String,
    /// Ports.
    pub ports: Vec<Port>,
    /// Clock domain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<String>,
    /// Members.
    pub members: Vec<CircuitMember>,
}

/// hdl port.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Port {
    /// Local id.
    pub id: String,
    /// Name.
    pub name: String,
    /// `in`, `out` or `inout`.
    pub dir: String,
    /// Type.
    pub ty: TypeExpr,
    /// Pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<Expr>,
    /// Location.
    pub span: Span,
}

/// Member of a circuit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CircuitMember {
    /// Location.
    pub span: Span,
    /// The member.
    pub def: CircuitDef,
}

/// Circuit member kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "member", rename_all = "snake_case")]
pub enum CircuitDef {
    /// `reg name: T = reset`.
    Reg {
        /// Local id.
        id: String,
        /// Name.
        name: String,
        /// Type.
        ty: TypeExpr,
        /// Reset value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        init: Option<Expr>,
    },
    /// `wire name: T`.
    Wire {
        /// Local id.
        id: String,
        /// Name.
        name: String,
        /// Type.
        ty: TypeExpr,
    },
    /// `const`.
    Const {
        /// Local id.
        id: String,
        /// Name.
        name: String,
        /// Type.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ty: Option<TypeExpr>,
        /// Value.
        value: Expr,
    },
    /// `instance u = circuit(args)`.
    Instance {
        /// Local id.
        id: String,
        /// Name.
        name: String,
        /// Instantiation.
        of: Expr,
    },
    /// `comb:`.
    Comb {
        /// Body.
        body: Block,
    },
    /// `on rising(clk):` / `on falling(clk):`.
    OnEdge {
        /// Rising edge.
        rising: bool,
        /// Clock.
        clock: String,
        /// Body.
        body: Block,
    },
    /// Nested state machine.
    Fsm {
        /// The state machine.
        fsm: Fsm,
    },
}

/// ui widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Widget {
    /// Kind (`label`, `button`, ...).
    pub kind: String,
    /// Instance name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Text with its quotes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Options.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<NamedExpr>,
    /// Children.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Widget>,
    /// Event handlers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<WidgetEvent>,
    /// Location.
    pub span: Span,
}

/// `name value` pair (widget and move options).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NamedExpr {
    /// Name.
    pub name: String,
    /// Value.
    pub value: Expr,
}

/// `on press:` handler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WidgetEvent {
    /// Event name.
    pub name: String,
    /// Body.
    pub body: Block,
}

// ---- types --------------------------------------------------------------

/// A type as written in the source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TypeExpr {
    /// Kind.
    pub kind: TypeExprKind,
    /// Unit suffix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<UnitRef>,
    /// `?` suffix.
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
    /// Location.
    pub span: Span,
}

/// Kinds of written types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum TypeExprKind {
    /// `path<args>`.
    Named {
        /// Dotted path.
        path: String,
        /// Generic arguments.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<TypeArg>,
    },
    /// `[T; N]`.
    Array {
        /// Element.
        elem: Box<TypeExpr>,
        /// Length.
        len: Box<Expr>,
    },
    /// `[T]`.
    Slice {
        /// Element.
        elem: Box<TypeExpr>,
    },
    /// `(A, B)`.
    Tuple {
        /// Elements.
        items: Vec<TypeExpr>,
    },
    /// A bare unit (`degC` means `f32 degC`).
    Unit,
}

/// Generic argument.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "arg", rename_all = "snake_case")]
pub enum TypeArg {
    /// Type argument.
    Type {
        /// The type.
        ty: TypeExpr,
    },
    /// Constant argument.
    Expr {
        /// The expression.
        expr: Expr,
    },
}

/// A unit expression as written (`mm/s`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UnitRef {
    /// Canonical text.
    pub text: String,
    /// Factors in source order with exponents (division gives negative exponents).
    pub factors: Vec<UnitFactor>,
}

/// One factor of a unit expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UnitFactor {
    /// Unit name.
    pub name: String,
    /// Exponent.
    pub exp: i32,
}

/// A resolved unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Unit {
    /// Display text (`mm/s`, or the dimension name for derived units).
    pub text: String,
    /// Exponents of the base dimensions in the order
    /// `s m kg A K mol cd rad count`.
    pub dim: [i8; 9],
    /// Exact factor to the base unit, as a decimal or `n/d` string.
    pub scale: String,
    /// Exact offset to the base unit (non-zero only for `degC` and `degF`).
    #[serde(default = "zero", skip_serializing_if = "is_zero")]
    pub offset: String,
}

fn is_zero(s: &str) -> bool {
    s == "0"
}

fn zero() -> String {
    "0".into()
}

/// A resolved type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Type {
    /// External vocabulary that is not typed yet.
    Unknown,
    /// No value.
    Void,
    /// Diverges.
    Never,
    /// `bool`.
    Bool,
    /// Number with a unit.
    Num {
        /// `u8` ... `i64`, `index`, `f32`, `f64`, `fixed<I, F>`, `ufixed<I, F>`,
        /// `duration`, or `integer` / `real` for literals without context.
        kind: String,
        /// Unit (absent for plain numbers).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<Unit>,
    },
    /// `bit<N>` / `sbit<N>`.
    Bits {
        /// Signed.
        signed: bool,
        /// Width.
        width: u32,
    },
    /// `instant`.
    Instant,
    /// `str` / `str<N>`.
    Str {
        /// Capacity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cap: Option<u32>,
    },
    /// `[T; N]`.
    Array {
        /// Element.
        elem: Box<Type>,
        /// Length.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        len: Option<u64>,
    },
    /// `[T]`.
    Slice {
        /// Element.
        elem: Box<Type>,
    },
    /// `(A, B)`.
    Tuple {
        /// Elements.
        items: Vec<Type>,
    },
    /// `range<T>`.
    Range {
        /// Element.
        elem: Box<Type>,
    },
    /// `T?`.
    Option {
        /// Inner.
        inner: Box<Type>,
    },
    /// `none` without context.
    NoneLit,
    /// `Result<T, E>`.
    Result {
        /// Success.
        ok: Box<Type>,
        /// Error.
        err: Box<Type>,
    },
    /// User struct, enum or class.
    Adt {
        /// Item id of the definition.
        def: String,
        /// Name.
        name: String,
        /// Generic arguments.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<Type>,
    },
    /// User interface.
    Interface {
        /// Item id.
        def: String,
        /// Name.
        name: String,
    },
    /// Generic parameter.
    Param {
        /// Name.
        name: String,
    },
    /// `channel<T, N>`.
    Channel {
        /// Element.
        elem: Box<Type>,
        /// Capacity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cap: Option<u64>,
    },
    /// `pool<T, N>`.
    Pool {
        /// Element.
        elem: Box<Type>,
        /// Capacity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cap: Option<u64>,
    },
    /// `Handle<T>`.
    Handle {
        /// Element.
        elem: Box<Type>,
    },
    /// A function value.
    Fn,
    /// An `io` binding.
    Io {
        /// Peripheral kind.
        kind: String,
    },
    /// Named external type (`point`, `ton`, ...).
    Opaque {
        /// Name.
        name: String,
    },
    /// A type used as a value.
    TypeRef {
        /// The type.
        of: Box<Type>,
    },
    /// A package module used as a value.
    Module {
        /// Module id.
        id: String,
    },
    /// A hardware or protocol event.
    Event,
}

// ---- statements ---------------------------------------------------------

/// Block of statements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Block {
    /// Contracts at the start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contracts: Vec<Contract>,
    /// Statements.
    pub stmts: Vec<Stmt>,
}

/// A statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stmt {
    /// Location.
    pub span: Span,
    /// Kind.
    pub s: StmtKind,
}

/// `if` / `elif` branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IfBranch {
    /// Condition.
    pub cond: Expr,
    /// Body.
    pub body: Block,
}

/// `match` arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Arm {
    /// Pattern.
    pub pattern: Pattern,
    /// Guard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<Expr>,
    /// Body.
    pub body: Block,
    /// Location.
    pub span: Span,
}

/// Statement kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "s", rename_all = "snake_case")]
pub enum StmtKind {
    /// `let` / `var`.
    Let {
        /// `var`.
        mutable: bool,
        /// Pattern.
        pattern: Pattern,
        /// Declared type.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ty: Option<TypeExpr>,
        /// Value.
        value: Expr,
    },
    /// `place op value [when cond]`.
    Assign {
        /// Target.
        place: Expr,
        /// `=`, `+=`, ...
        op: String,
        /// Value.
        value: Expr,
        /// Conditional assignment.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        when: Option<Expr>,
    },
    /// Expression statement.
    Expr {
        /// The expression.
        expr: Expr,
    },
    /// `pass`.
    Pass,
    /// `return [expr]`.
    Return {
        /// Value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Expr>,
    },
    /// `break`.
    Break,
    /// `continue`.
    Continue,
    /// `fault Name(args)`.
    Fault {
        /// Fault name.
        name: String,
        /// Arguments.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<Arg>,
    },
    /// `assert cond, "msg"`.
    Assert {
        /// Condition.
        cond: Expr,
        /// Message with its quotes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        msg: Option<String>,
    },
    /// `emit Event(args)`.
    Emit {
        /// Event.
        event: String,
        /// Payload.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<Arg>,
    },
    /// `set x when c` / `reset x when c`.
    Coil {
        /// `set`.
        set: bool,
        /// Target.
        place: Expr,
        /// Condition.
        when: Expr,
    },
    /// `wait D`.
    Wait {
        /// Duration.
        duration: Expr,
    },
    /// `until cond timeout D -> Target`.
    Until {
        /// Condition.
        cond: Expr,
        /// Timeout.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<Expr>,
        /// Step to jump to on timeout.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_timeout: Option<String>,
    },
    /// `-> Target`.
    Jump {
        /// Step.
        target: String,
    },
    /// robot `move`.
    Move {
        /// `ptp`, `linear` or `arc`.
        kind: String,
        /// Target.
        to: Expr,
        /// Intermediate point.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        via: Option<Expr>,
        /// Options.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<NamedExpr>,
    },
    /// `if` / `elif` / `else`.
    If {
        /// Branches.
        branches: Vec<IfBranch>,
        /// `else`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        otherwise: Option<Block>,
    },
    /// `match`.
    Match {
        /// Scrutinee.
        value: Expr,
        /// Arms.
        arms: Vec<Arm>,
    },
    /// `for pattern in expr:`.
    For {
        /// Pattern.
        pattern: Pattern,
        /// Iterated expression.
        iter: Expr,
        /// Static number of iterations, when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u64>,
        /// Body.
        body: Block,
    },
    /// `while cond max N:`.
    While {
        /// Pattern of `while let`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<Pattern>,
        /// Condition or scrutinee.
        cond: Expr,
        /// Bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<Expr>,
        /// Body.
        body: Block,
    },
    /// `loop max N:`.
    Loop {
        /// Bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<Expr>,
        /// Body.
        body: Block,
    },
    /// `atomic:`.
    Atomic {
        /// Body.
        body: Block,
    },
    /// Nested rule.
    Rule {
        /// The rule.
        rule: Rule,
    },
    /// Inline assembly.
    Asm {
        /// The block.
        asm: Asm,
    },
}

/// A pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pattern {
    /// Location.
    pub span: Span,
    /// Kind.
    pub p: PatternKind,
}

/// Named sub-pattern `field: pattern`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FieldPattern {
    /// Field.
    pub name: String,
    /// Pattern.
    pub pattern: Pattern,
}

/// Pattern kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "p", rename_all = "snake_case")]
pub enum PatternKind {
    /// `_`.
    Wildcard,
    /// Binding.
    Bind {
        /// Name.
        name: String,
        /// Local id.
        id: String,
        /// Type of the bound value.
        ty: Type,
    },
    /// Literal.
    Lit {
        /// Value.
        value: Expr,
    },
    /// `a..b` / `a..=b`.
    Range {
        /// Start.
        lo: Expr,
        /// End.
        hi: Expr,
        /// `..=`.
        inclusive: bool,
    },
    /// `Path(p, q)` or `Path(x: p)`.
    Ctor {
        /// Dotted path.
        path: String,
        /// Positional sub-patterns.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<Pattern>,
        /// Named sub-patterns.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<FieldPattern>,
    },
    /// Enum variant or constant.
    Path {
        /// Dotted path.
        path: String,
    },
    /// `(a, b)`.
    Tuple {
        /// Elements.
        items: Vec<Pattern>,
    },
    /// `A | B`.
    Or {
        /// Alternatives.
        alts: Vec<Pattern>,
    },
}

// ---- expressions --------------------------------------------------------

/// An expression with its type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Expr {
    /// Location.
    pub span: Span,
    /// Type and unit.
    pub ty: Type,
    /// Kind.
    pub e: ExprKind,
}

/// Literal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "lit", rename_all = "snake_case")]
pub enum Lit {
    /// Integer. `text` as written, `v` exact decimal value.
    Int {
        /// As written.
        text: String,
        /// Exact decimal value.
        v: String,
    },
    /// Real. `text` as written, `v` exact decimal value.
    Real {
        /// As written.
        text: String,
        /// Exact decimal value.
        v: String,
    },
    /// String, with quotes and escapes as written.
    Str {
        /// As written.
        text: String,
    },
    /// Raw string.
    RawStr {
        /// As written.
        text: String,
    },
    /// Byte string.
    ByteStr {
        /// As written.
        text: String,
    },
    /// Character.
    Char {
        /// As written.
        text: String,
    },
    /// `true` / `false`.
    Bool {
        /// Value.
        v: bool,
    },
    /// `none`.
    None,
    /// IEC address (`%I0.0`).
    Iec {
        /// As written.
        text: String,
    },
    /// Bit vector (`8'b1010`).
    Bits {
        /// As written.
        text: String,
    },
}

/// Comparison in a chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CmpRest {
    /// Operator.
    pub op: String,
    /// Right operand.
    pub rhs: Expr,
}

/// Lambda parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LambdaParam {
    /// Name.
    pub name: String,
    /// Local id.
    pub id: String,
}

/// What a call calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum CallTarget {
    /// A function item.
    Fn {
        /// Item id.
        id: String,
    },
    /// A method of a class.
    Method {
        /// Item id of the class.
        class: String,
        /// Method name.
        name: String,
        /// The method takes `mut self`.
        #[serde(default, skip_serializing_if = "is_false")]
        mut_self: bool,
    },
    /// A struct, class or enum variant constructor.
    Ctor {
        /// Item id of the type.
        def: String,
    },
    /// An operation on an `io` binding.
    Io {
        /// Binding name.
        binding: String,
        /// Operation.
        method: String,
    },
    /// A prelude function or function block.
    Builtin {
        /// Name.
        name: String,
    },
    /// External vocabulary.
    Foreign,
}

/// Expression kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "e", rename_all = "snake_case")]
pub enum ExprKind {
    /// Literal.
    Lit {
        /// The literal.
        lit: Lit,
    },
    /// Number with a unit.
    Quantity {
        /// Number, tuple or call.
        value: Box<Expr>,
        /// Unit.
        unit: UnitRef,
        /// Exact value in base SI units, when the quantity is a constant.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        si: Option<String>,
    },
    /// Reference to a module-level definition.
    Ref {
        /// Item id.
        id: String,
        /// Name as written.
        name: String,
    },
    /// Reference to a local binding.
    Local {
        /// Local id.
        id: String,
        /// Name as written.
        name: String,
    },
    /// Name from the standard library, a dependency or a device description.
    Global {
        /// Name or dotted path.
        name: String,
    },
    /// A package module.
    Module {
        /// Module id.
        id: String,
        /// Name as written.
        name: String,
    },
    /// A state of a state machine (in `verify`).
    State {
        /// State machine.
        fsm: String,
        /// State.
        state: String,
    },
    /// `self`.
    SelfValue,
    /// `result`.
    Result,
    /// Binary operation.
    Binary {
        /// Operator as written.
        op: String,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// Chained comparison.
    Compare {
        /// First operand.
        first: Box<Expr>,
        /// Following comparisons.
        rest: Vec<CmpRest>,
    },
    /// Unary operation (`-`, `~`, `not`).
    Unary {
        /// Operator.
        op: String,
        /// Operand.
        operand: Box<Expr>,
    },
    /// `a..b` / `a..=b`.
    Range {
        /// Start.
        lo: Box<Expr>,
        /// End.
        hi: Box<Expr>,
        /// `..=`.
        inclusive: bool,
    },
    /// Linear map `a..b -> c..d`.
    Map {
        /// Source range.
        from: Box<Expr>,
        /// Target range.
        to: Box<Expr>,
    },
    /// `if c then a else b`.
    IfElse {
        /// Condition.
        cond: Box<Expr>,
        /// Value when true.
        then: Box<Expr>,
        /// Value when false.
        #[serde(rename = "else")]
        otherwise: Box<Expr>,
    },
    /// `x |> f(a)`.
    Pipe {
        /// Value.
        lhs: Box<Expr>,
        /// Function.
        rhs: Box<Expr>,
    },
    /// Call or construction.
    Call {
        /// Callee.
        callee: Box<Expr>,
        /// Arguments.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<Arg>,
        /// Resolved target.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<CallTarget>,
    },
    /// `x[i]` or `x[hi:lo]`.
    Index {
        /// Indexed value.
        base: Box<Expr>,
        /// Index (or upper bit).
        index: Box<Expr>,
        /// Lower bit of a slice.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<Box<Expr>>,
    },
    /// `x.name`.
    Field {
        /// Value.
        base: Box<Expr>,
        /// Field, method or member name.
        name: String,
        /// Item id when the field names a module item, `state:Fsm.State` for states.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    /// `x?`.
    Try {
        /// Operand.
        operand: Box<Expr>,
    },
    /// `x as T`.
    Cast {
        /// Operand.
        operand: Box<Expr>,
        /// Target type as written.
        to: TypeExpr,
    },
    /// `(a, b)`.
    Tuple {
        /// Elements.
        items: Vec<Expr>,
    },
    /// `[a, b]`.
    Array {
        /// Elements.
        items: Vec<Expr>,
    },
    /// `[v; n]`.
    ArrayRepeat {
        /// Value.
        value: Box<Expr>,
        /// Count.
        count: Box<Expr>,
    },
    /// `fn(x) => body`.
    Lambda {
        /// Parameters.
        params: Vec<LambdaParam>,
        /// Body.
        body: Box<Expr>,
    },
    /// `old(x)`.
    Old {
        /// Operand.
        operand: Box<Expr>,
    },
    /// `run action`.
    Run {
        /// Action.
        action: Box<Expr>,
    },
}
