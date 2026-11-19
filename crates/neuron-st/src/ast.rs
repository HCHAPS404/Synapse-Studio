//! Syntax tree of the Structured Text subset.

/// A source file: program units and, optionally, a configuration.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Unit {
    /// Programs, function blocks and functions in source order.
    pub pous: Vec<Pou>,
    /// Task intervals found in a `CONFIGURATION`, by program instance type.
    pub tasks: Vec<(String, i64)>,
}

/// Kind of program unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PouKind {
    /// `PROGRAM` (and the SCL `ORGANIZATION_BLOCK`).
    Program,
    /// `FUNCTION_BLOCK`.
    FunctionBlock,
    /// `FUNCTION`.
    Function,
}

/// A program, function block or function.
#[derive(Debug, Clone, PartialEq)]
pub struct Pou {
    /// Kind.
    pub kind: PouKind,
    /// Name as written.
    pub name: String,
    /// Return type of a function.
    pub ret: Option<Ty>,
    /// Variable declarations in source order.
    pub vars: Vec<VarDecl>,
    /// Body.
    pub body: Vec<Stmt>,
    /// Byte offset of the header.
    pub pos: usize,
}

/// Section of a variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// `VAR`.
    Var,
    /// `VAR_INPUT`.
    Input,
    /// `VAR_OUTPUT`.
    Output,
    /// `VAR_IN_OUT`.
    InOut,
    /// `VAR_TEMP`.
    Temp,
    /// `VAR_GLOBAL`.
    Global,
    /// `VAR_EXTERNAL`.
    External,
}

/// Elementary or named type.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    /// `BOOL`.
    Bool,
    /// An integer type.
    Int(IntTy),
    /// `REAL` (32 bit) or `LREAL`.
    Real(bool),
    /// `TIME`.
    Time,
    /// `STRING`.
    Str,
    /// A function block or user type.
    Named(String),
    /// `ARRAY [lo..hi] OF T`.
    Array(i64, i64, Box<Ty>),
}

/// Width and sign of an integer type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntTy {
    /// Bits.
    pub bits: u8,
    /// Signed.
    pub signed: bool,
}

impl IntTy {
    /// Wraps `v` into the range of the type, as a PLC does.
    pub fn wrap(self, v: i64) -> i64 {
        if self.bits >= 64 {
            return v;
        }
        let m = 1i128 << self.bits;
        let mut x = i128::from(v).rem_euclid(m);
        if self.signed && x >= m / 2 {
            x -= m;
        }
        x as i64
    }
}

/// Initial value.
#[derive(Debug, Clone, PartialEq)]
pub enum Init {
    /// `:= expr`.
    Expr(Expr),
    /// `:= (PT := T#3S)` for a function block instance.
    Struct(Vec<(String, Expr)>),
}

/// One declared variable.
#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    /// Name as written.
    pub name: String,
    /// Section.
    pub section: Section,
    /// `RETAIN`.
    pub retain: bool,
    /// Direct address after `AT` (`%IX0.0`).
    pub at: Option<String>,
    /// Type.
    pub ty: Ty,
    /// Initial value.
    pub init: Option<Init>,
    /// Byte offset.
    pub pos: usize,
}

/// Call argument.
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// Parameter name for `name := value` and `name => var`.
    pub name: Option<String>,
    /// `=>` (output binding).
    pub output: bool,
    /// Value, or the variable that receives an output.
    pub value: Expr,
}

/// Expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `TRUE` / `FALSE`.
    Bool(bool),
    /// Integer literal.
    Int(i64),
    /// Real literal.
    Real(f64),
    /// Duration in nanoseconds.
    Time(i64),
    /// String literal.
    Str(String),
    /// Variable.
    Var(String),
    /// `a.b`.
    Field(Box<Expr>, String),
    /// `a[i]`.
    Index(Box<Expr>, Box<Expr>),
    /// `-x`, `NOT x`.
    Unary(String, Box<Expr>),
    /// Binary operation with the operator in upper case (`AND`, `+`, `=`).
    Binary(String, Box<Expr>, Box<Expr>),
    /// Function call.
    Call(String, Vec<Arg>),
}

/// Statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `place := value;`.
    Assign(Expr, Expr, usize),
    /// `instance(args);`.
    Call(Expr, Vec<Arg>, usize),
    /// `IF ... ELSIF ... ELSE ... END_IF`.
    If(Vec<(Expr, Vec<Stmt>)>, Option<Vec<Stmt>>, usize),
    /// `CASE e OF labels: ... ELSE ... END_CASE`.
    Case(
        Expr,
        Vec<(Vec<CaseLabel>, Vec<Stmt>)>,
        Option<Vec<Stmt>>,
        usize,
    ),
    /// `FOR v := a TO b BY c DO ... END_FOR`.
    For(String, Expr, Expr, Option<Expr>, Vec<Stmt>, usize),
    /// `WHILE c DO ... END_WHILE`.
    While(Expr, Vec<Stmt>, usize),
    /// `REPEAT ... UNTIL c END_REPEAT`.
    Repeat(Vec<Stmt>, Expr, usize),
    /// `EXIT`.
    Exit(usize),
    /// `RETURN`.
    Return(usize),
}

impl Stmt {
    /// Byte offset of the statement.
    pub fn pos(&self) -> usize {
        match self {
            Stmt::Assign(_, _, p)
            | Stmt::Call(_, _, p)
            | Stmt::If(_, _, p)
            | Stmt::Case(_, _, _, p)
            | Stmt::For(_, _, _, _, _, p)
            | Stmt::While(_, _, p)
            | Stmt::Repeat(_, _, p)
            | Stmt::Exit(p)
            | Stmt::Return(p) => *p,
        }
    }
}

/// `CASE` label.
#[derive(Debug, Clone, PartialEq)]
pub enum CaseLabel {
    /// One value.
    Value(i64),
    /// `a..b`.
    Range(i64, i64),
}
