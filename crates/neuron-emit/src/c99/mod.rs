//! The `c` emitter: `core` modules to portable C99 (task C8).
//!
//! One `<module>.c` and `<module>.h` per `core` module, plus `nrt_core.h` and `nrt_core.c`
//! (the runtime they share). This is plain computation code, not firmware: `neuron
//! emit-c` is the separate command that builds a whole image for a board.
//!
//! * Naming: every function is `<module>_<name>`; `pub` functions are declared in the header,
//!   the others are `static` in the `.c`. A `const` is a `#define <MODULE>_<NAME>`. Enums are
//!   C enums with constants `<module>_<Enum>_<Variant>` and structs are typedefs
//!   `<module>_<Struct>`.
//! * Types: `bool`, `uint8_t` to `uint64_t` (`index` is `uint64_t`), `int8_t` to `int64_t`,
//!   `float` (`f32` without a unit) and `double` (`f64`, durations, anything with a unit).
//! * Units are erased: a quantity is a `double` in base SI units (a `mm` parameter takes
//!   metres, a `ms` one seconds, `degC` kelvin). The C does not check units.
//! * Faults: the interpreter stops with `Overflow` when an integer result leaves the range of
//!   its type (or a cast does), with `DivideByZero`, and with `ContractViolation` when a
//!   `requires` is false. The C sets the global `nrt_fault` (first fault wins), yields 0 for
//!   the failing operation and goes on; the caller clears `nrt_fault` before the call and reads
//!   it after. The wrapping (`+%`) and saturating (`+|`) operators never fault. See
//!   `nrt_core.h`.
//! * Loops are bounded like in the interpreter: `while` and `loop` stop after `max` (or
//!   1 000 000) iterations; `for` over a range is a C `for`.
//! * Not translated (`NE0610` with its location): classes, interfaces, module variables,
//!   options, results, arrays, tuples, strings, bit vectors, generics, `mut` parameters,
//!   `ensures`, recursion, tasks and every other dialect's construct.

use crate::{nirx, Class, Diag, EmitOptions, Emitter, Fidelity, Manifest, Package, Writer};
use neuron_nir::{
    Block, Expr, ExprKind, FnBody, FnDecl, Item, ItemDef, Lit, Module, Nir, Pattern, PatternKind,
    Stmt, StmtKind, Type, TypeExpr, TypeExprKind,
};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const RUNTIME_H: &str = include_str!("nrt_core.h");
const RUNTIME_C: &str = include_str!("nrt_core.c");

const CONSTRUCTS: &[&str] = &[
    "const",
    "fn",
    "struct",
    "enum",
    "match",
    "let",
    "assign",
    "if",
    "for",
    "while",
    "loop",
    "return",
    "requires",
    "integer arithmetic",
    "wrapping and saturating arithmetic",
    "float arithmetic",
    "cast",
    "abs",
    "min",
    "max",
    "clamp",
    "floor",
    "ceil",
    "round",
    "sqrt",
];

/// The `c` emitter.
pub struct C;

// ---------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------

/// An integer type: signedness and width (`bits == 0` is an untyped integer, read as 64 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IntTy {
    signed: bool,
    bits: u32,
}

impl IntTy {
    fn eff(self) -> u32 {
        if self.bits == 0 {
            64
        } else {
            self.bits
        }
    }

    fn cname(self) -> String {
        format!("{}int{}_t", if self.signed { "" } else { "u" }, self.eff())
    }

    fn lo(self) -> i128 {
        if self.signed {
            -(1i128 << (self.eff() - 1))
        } else {
            0
        }
    }

    fn hi(self) -> i128 {
        if self.signed {
            (1i128 << (self.eff() - 1)) - 1
        } else {
            (1i128 << self.eff()) - 1
        }
    }

    fn fam(self) -> char {
        if self.signed {
            's'
        } else {
            'u'
        }
    }

    /// Arguments that give a helper the range: `lo, hi` (signed) or `hi` (unsigned).
    fn range_args(self) -> String {
        if self.signed {
            format!("{}, {}", lit_s(self.lo()), lit_s(self.hi()))
        } else {
            lit_u(self.hi())
        }
    }

    /// Narrower than `int`: C promotes it, so results need a cast back.
    fn small(self) -> bool {
        self.eff() < 32
    }
}

/// A C type of the generated code.
#[derive(Debug, Clone, PartialEq)]
enum Ct {
    Void,
    Bool,
    Int(IntTy),
    F32,
    F64,
    /// Item id of a struct.
    Struct(String),
    /// Item id of an enum.
    Enum(String),
}

fn int_kind(kind: &str) -> Option<IntTy> {
    let (signed, bits) = match kind {
        "u8" => (false, 8),
        "u16" => (false, 16),
        "u32" => (false, 32),
        "u64" | "index" => (false, 64),
        "i8" => (true, 8),
        "i16" => (true, 16),
        "i32" => (true, 32),
        "i64" => (true, 64),
        "integer" => (true, 0),
        _ => return None,
    };
    Some(IntTy { signed, bits })
}

/// A signed C integer literal.
fn lit_s(v: i128) -> String {
    if v == i128::from(i64::MIN) {
        "(-9223372036854775807ll - 1)".to_string()
    } else if v < 0 {
        let a = -v;
        if a <= i128::from(i32::MAX) {
            format!("(-{a})")
        } else {
            format!("(-{a}ll)")
        }
    } else if v <= i128::from(i32::MAX) {
        format!("{v}")
    } else {
        format!("{v}ll")
    }
}

/// An unsigned C integer literal.
fn lit_u(v: i128) -> String {
    if v <= i128::from(u32::MAX) {
        format!("{v}u")
    } else {
        format!("{v}ull")
    }
}

/// The literal of `v` for the integer type `it`.
fn lit_int(it: IntTy, v: i128) -> String {
    if it.signed || v < 0 {
        lit_s(v)
    } else {
        lit_u(v)
    }
}

/// A C floating literal (`float` values are written as a cast of the rounded number).
fn flit(x: f64, single: bool) -> String {
    let x = if single { f64::from(x as f32) } else { x };
    let s = if x.is_nan() {
        "NAN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 {
            "INFINITY".to_string()
        } else {
            "(-INFINITY)".to_string()
        }
    } else if x.is_sign_negative() {
        format!("({x:?})")
    } else {
        format!("{x:?}")
    };
    if single {
        format!("((float){s})")
    } else {
        s
    }
}

const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while", "bool", "true", "false", "NULL", "main",
];

fn ident(s: &str) -> String {
    let mut o: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if o.is_empty() {
        o.push('_');
    }
    if o.starts_with(|c: char| c.is_ascii_digit()) {
        o.insert(0, '_');
    }
    o
}

/// A parameter name that is not a C keyword.
fn pname(s: &str) -> String {
    let o = ident(s);
    if C_KEYWORDS.contains(&o.as_str()) {
        format!("{o}_")
    } else {
        o
    }
}

/// `true` for `x` that can be used before `.` without parentheses.
fn simple(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

// ---------------------------------------------------------------------------------------
// The lowering
// ---------------------------------------------------------------------------------------

struct Lower<'a> {
    file: String,
    prefix: String,
    defs: HashMap<String, &'a ItemDef>,
    /// Struct and enum item ids by name.
    types: HashMap<String, String>,
    /// C names of parameters, by local id.
    locals: HashMap<String, String>,
    diags: Vec<Diag>,
    fid: Fidelity,
    tmp: usize,
    /// Return type of the function being written.
    ret: Ct,
    /// Locals read by the function being written.
    reads: BTreeSet<String>,
    /// Reads found by the first pass over the function now being written.
    reads_final: BTreeSet<String>,
    /// Functions called by the function being written (item ids).
    calls: BTreeSet<String>,
    // Flags of the function being written, for the fidelity report.
    chk: bool,
    neg: bool,
    untyped: bool,
    pow: bool,
    units: Cell<bool>,
}

impl<'a> Lower<'a> {
    fn new(module: &'a Module, prefix: String) -> Lower<'a> {
        let mut defs = HashMap::new();
        let mut types = HashMap::new();
        for it in &module.items {
            defs.insert(it.id.clone(), &it.def);
            match &it.def {
                ItemDef::Struct { name, .. } | ItemDef::Enum { name, .. } => {
                    types.insert(name.clone(), it.id.clone());
                }
                _ => {}
            }
        }
        Lower {
            file: module.file.clone(),
            prefix,
            defs,
            types,
            locals: HashMap::new(),
            diags: Vec::new(),
            fid: Fidelity::new("emit-c", "core.c99"),
            tmp: 0,
            ret: Ct::Void,
            reads: BTreeSet::new(),
            reads_final: BTreeSet::new(),
            calls: BTreeSet::new(),
            chk: false,
            neg: false,
            untyped: false,
            pow: false,
            units: Cell::new(false),
        }
    }

    fn unsupported(&mut self, span: [usize; 2], what: &str) {
        self.diags.push(Diag {
            code: "NE0610",
            message: format!("{what} is not translated by the c emitter"),
            help: Some(
                "the c emitter translates `core` functions over integers, booleans, reals, structs and enums; translate that part by hand"
                    .into(),
            ),
            file: self.file.clone(),
            span,
        });
    }

    fn next_tmp(&mut self) -> usize {
        self.tmp += 1;
        self.tmp
    }

    fn type_name(&self, id: &str) -> String {
        match self.defs.get(id).copied() {
            Some(ItemDef::Struct { name, .. } | ItemDef::Enum { name, .. }) => {
                format!("{}_{}", self.prefix, ident(name))
            }
            _ => "int".to_string(),
        }
    }

    fn fn_name(&self, name: &str) -> String {
        format!("{}_{}", self.prefix, ident(name))
    }

    fn const_name(&self, name: &str) -> String {
        format!(
            "{}_{}",
            self.prefix.to_uppercase(),
            ident(name).to_uppercase()
        )
    }

    fn local_name(&self, id: &str, name: &str) -> String {
        match self.locals.get(id) {
            Some(n) => n.clone(),
            None => format!("{}_{}", ident(name), ident(id)),
        }
    }

    // ---- types ----------------------------------------------------------------------

    fn ct_def(&self, id: &str) -> Option<Ct> {
        match self.defs.get(id).copied() {
            Some(ItemDef::Struct { generics, .. }) if generics.is_empty() => {
                Some(Ct::Struct(id.to_string()))
            }
            Some(ItemDef::Enum {
                generics, variants, ..
            }) if generics.is_empty() && variants.iter().all(|v| v.payload.is_empty()) => {
                Some(Ct::Enum(id.to_string()))
            }
            _ => None,
        }
    }

    fn ct_type(&self, t: &Type) -> Option<Ct> {
        match t {
            Type::Void => Some(Ct::Void),
            Type::Bool => Some(Ct::Bool),
            Type::Num { kind, unit } => {
                if unit.is_some() {
                    self.units.set(true);
                    return Some(Ct::F64);
                }
                if let Some(i) = int_kind(kind) {
                    return Some(Ct::Int(i));
                }
                match kind.as_str() {
                    "f32" => Some(Ct::F32),
                    "f64" | "real" => Some(Ct::F64),
                    "duration" => {
                        self.units.set(true);
                        Some(Ct::F64)
                    }
                    _ => None,
                }
            }
            Type::Adt { def, args, .. } if args.is_empty() => self.ct_def(def),
            _ => None,
        }
    }

    fn ct_te(&self, t: &TypeExpr) -> Option<Ct> {
        if t.optional {
            return None;
        }
        if t.unit.is_some() {
            self.units.set(true);
            return Some(Ct::F64);
        }
        match &t.kind {
            TypeExprKind::Unit => {
                self.units.set(true);
                Some(Ct::F64)
            }
            TypeExprKind::Named { path, args } if args.is_empty() => match path.as_str() {
                "bool" => Some(Ct::Bool),
                "f32" => Some(Ct::F32),
                "f64" => Some(Ct::F64),
                "duration" => {
                    self.units.set(true);
                    Some(Ct::F64)
                }
                p => {
                    if let Some(i) = int_kind(p).filter(|i| i.bits != 0) {
                        return Some(Ct::Int(i));
                    }
                    let last = p.rsplit('.').next().unwrap_or(p);
                    let id = self.types.get(last)?;
                    self.ct_def(id)
                }
            },
            _ => None,
        }
    }

    fn cty(&self, ct: &Ct) -> String {
        match ct {
            Ct::Void => "void".into(),
            Ct::Bool => "bool".into(),
            Ct::Int(i) => i.cname(),
            Ct::F32 => "float".into(),
            Ct::F64 => "double".into(),
            Ct::Struct(id) | Ct::Enum(id) => self.type_name(id),
        }
    }

    fn zero(&self, ct: &Ct) -> String {
        match ct {
            Ct::Void => String::new(),
            Ct::Bool => "false".into(),
            Ct::Int(_) | Ct::Enum(_) => "0".into(),
            Ct::F32 | Ct::F64 => "0.0".into(),
            Ct::Struct(id) => format!("({}){{0}}", self.type_name(id)),
        }
    }

    // ---- expressions ----------------------------------------------------------------

    fn ex(&mut self, e: &Expr) -> String {
        self.ex_t(e, None)
    }

    /// An expression; `want` is the type the context needs (it types integer literals).
    fn ex_t(&mut self, e: &Expr, want: Option<&Ct>) -> String {
        let own = self.ct_type(&e.ty);
        // Arithmetic keeps the type the analyzer gave it (the interpreter checks the range
        // of that type, so `(1 << n) - 1` of untyped integers does not fault in a `u32`
        // context); a literal takes the type its context needs.
        let tgt = match (&e.e, &own) {
            (ExprKind::Binary { .. } | ExprKind::Unary { .. }, Some(Ct::Int(_))) => own.clone(),
            _ => want.cloned().or_else(|| own.clone()),
        };
        if let Some(v) = nirx::int(e) {
            match &tgt {
                Some(Ct::Int(it)) => return lit_int(*it, i128::from(v)),
                Some(Ct::F32) => return flit(v as f64, true),
                Some(Ct::F64) => return flit(v as f64, false),
                _ => {}
            }
        }
        match &e.e {
            ExprKind::Lit { lit } => self.lit(e, lit, tgt.as_ref()),
            ExprKind::Quantity { value, si, .. } => {
                if let Some(s) = si {
                    return flit(nirx::parse_num(s), false);
                }
                let v = self.ex(value);
                match &e.ty {
                    Type::Num { unit: Some(u), .. } => {
                        self.units.set(true);
                        let (sc, of) = (nirx::parse_num(&u.scale), nirx::parse_num(&u.offset));
                        format!(
                            "((double)({v}) * {} + {})",
                            flit(sc, false),
                            flit(of, false)
                        )
                    }
                    _ => v,
                }
            }
            ExprKind::Ref { id, name } => match self.defs.get(id).copied() {
                Some(ItemDef::Const { name, .. }) => self.const_name(name),
                _ => {
                    self.unsupported(e.span, &format!("the reference to `{name}`"));
                    "0".into()
                }
            },
            ExprKind::Local { id, name } => {
                let n = self.local_name(id, name);
                self.reads.insert(n.clone());
                n
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, op, lhs, rhs, tgt.as_ref()),
            ExprKind::Compare { first, rest } => {
                let mut parts = Vec::new();
                let mut prev = first.as_ref();
                for c in rest {
                    if !matches!(c.op.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=") {
                        self.unsupported(e.span, &format!("the comparison `{}`", c.op));
                    }
                    let lit = |x: &Expr| nirx::int(x).is_some();
                    let ct = if lit(prev) {
                        self.ct_type(&c.rhs.ty)
                    } else {
                        self.ct_type(&prev.ty)
                    };
                    if matches!(ct, Some(Ct::Struct(_))) {
                        self.unsupported(e.span, "the comparison of structs");
                    }
                    let a = self.ex_t(prev, ct.as_ref());
                    let b = self.ex_t(&c.rhs, ct.as_ref());
                    parts.push(format!("({a} {} {b})", c.op));
                    prev = &c.rhs;
                }
                if parts.len() == 1 {
                    parts.remove(0)
                } else {
                    format!("({})", parts.join(" && "))
                }
            }
            ExprKind::Unary { op, operand } => self.unary(e, op, operand, tgt.as_ref()),
            ExprKind::IfElse {
                cond,
                then,
                otherwise,
            } => {
                let c = self.ex(cond);
                let a = self.ex_t(then, tgt.as_ref());
                let b = self.ex_t(otherwise, tgt.as_ref());
                format!("({c} ? {a} : {b})")
            }
            ExprKind::Call { callee, args, .. } => self.call(e, callee, args),
            ExprKind::Field { base, name, .. } => self.field(e, base, name),
            ExprKind::Cast { operand, .. } => self.cast(e, operand),
            _ => {
                self.unsupported(e.span, "this expression");
                "0".into()
            }
        }
    }

    fn lit(&mut self, e: &Expr, l: &Lit, tgt: Option<&Ct>) -> String {
        match l {
            Lit::Int { v, .. } => match tgt {
                Some(Ct::F32) => flit(nirx::parse_num(v), true),
                Some(Ct::F64) => flit(nirx::parse_num(v), false),
                Some(Ct::Int(it)) => lit_int(*it, v.parse::<i128>().unwrap_or(0)),
                _ => {
                    self.unsupported(e.span, "this literal");
                    "0".into()
                }
            },
            Lit::Real { v, .. } => match tgt {
                Some(Ct::F32) => flit(nirx::parse_num(v), true),
                _ => flit(nirx::parse_num(v), false),
            },
            Lit::Bool { v } => if *v { "true" } else { "false" }.into(),
            _ => {
                self.unsupported(e.span, "this literal");
                "0".into()
            }
        }
    }

    fn field(&mut self, e: &Expr, base: &Expr, name: &str) -> String {
        // An enum variant: `Mode.Auto`.
        if let ExprKind::Ref { id, .. } = &base.e {
            if let Some(ItemDef::Enum { name: en, .. }) = self.defs.get(id).copied() {
                return format!("{}_{}_{}", self.prefix, ident(en), ident(name));
            }
        }
        if matches!(self.ct_type(&base.ty), Some(Ct::Struct(_))) {
            let b = self.ex(base);
            return if simple(&b) {
                format!("{b}.{}", ident(name))
            } else {
                format!("({b}).{}", ident(name))
            };
        }
        self.unsupported(e.span, &format!("the field `{name}`"));
        "0".into()
    }

    fn unary(&mut self, e: &Expr, op: &str, operand: &Expr, tgt: Option<&Ct>) -> String {
        match op {
            "not" => format!("(!{})", self.ex(operand)),
            "-" => match tgt {
                Some(Ct::Int(it)) => {
                    let v = self.ex_t(operand, tgt);
                    self.neg = true;
                    let c = it.cname();
                    if it.signed {
                        format!("(({c})nrt_neg_s({v}, {}))", it.range_args())
                    } else {
                        format!("(({c})nrt_neg_u({v}))")
                    }
                }
                Some(Ct::F32 | Ct::F64) => format!("(-{})", self.ex_t(operand, tgt)),
                _ => {
                    self.unsupported(e.span, "negation of this type");
                    "0".into()
                }
            },
            "~" => match tgt {
                Some(Ct::Int(it)) => {
                    let v = self.ex_t(operand, tgt);
                    if it.small() {
                        format!("(({})(~{v}))", it.cname())
                    } else {
                        format!("(~{v})")
                    }
                }
                _ => {
                    self.unsupported(e.span, "bitwise not on this type");
                    "0".into()
                }
            },
            _ => {
                self.unsupported(e.span, &format!("the operator `{op}`"));
                "0".into()
            }
        }
    }

    fn binary(&mut self, e: &Expr, op: &str, l: &Expr, r: &Expr, tgt: Option<&Ct>) -> String {
        match op {
            "and" => return format!("({} && {})", self.ex(l), self.ex(r)),
            "or" => return format!("({} || {})", self.ex(l), self.ex(r)),
            "in" => {
                if let ExprKind::Range { lo, hi, inclusive } = &r.e {
                    let ct = self.ct_type(&l.ty);
                    let x = self.ex_t(l, ct.as_ref());
                    let a = self.ex_t(lo, ct.as_ref());
                    let b = self.ex_t(hi, ct.as_ref());
                    let cmp = if *inclusive { "<=" } else { "<" };
                    return format!("({a} <= {x} && {x} {cmp} {b})");
                }
                self.unsupported(e.span, "`in` over something that is not a range");
                return "0".into();
            }
            "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                let ct = if nirx::int(l).is_some() {
                    self.ct_type(&r.ty)
                } else {
                    self.ct_type(&l.ty)
                };
                let a = self.ex_t(l, ct.as_ref());
                let b = self.ex_t(r, ct.as_ref());
                return format!("({a} {op} {b})");
            }
            _ => {}
        }
        match tgt {
            Some(Ct::Int(it)) => {
                let it = *it;
                let a = self.ex_t(l, tgt);
                // The right operand of a shift or a power is a count of its own type.
                let counted = matches!(op, "<<" | ">>" | "**");
                let b = if counted {
                    self.ex(r)
                } else {
                    self.ex_t(r, tgt)
                };
                let b_unsigned = match self.ct_type(&r.ty) {
                    Some(Ct::Int(i)) => !i.signed,
                    _ => false,
                };
                self.iarith(op, &a, &b, it, b_unsigned, e.span)
            }
            Some(Ct::F32 | Ct::F64) => {
                let single = tgt == Some(&Ct::F32);
                let a = self.ex_t(l, tgt);
                let b = self.ex_t(r, tgt);
                self.farith(op, &a, &b, single, e.span)
            }
            Some(Ct::Bool) => {
                let a = self.ex(l);
                let b = self.ex(r);
                match op {
                    "&" | "|" | "^" => format!("(({a} {op} {b}) != 0)"),
                    _ => {
                        self.unsupported(e.span, &format!("the operator `{op}` on booleans"));
                        "0".into()
                    }
                }
            }
            _ => {
                self.unsupported(e.span, &format!("the operator `{op}` on this type"));
                "0".into()
            }
        }
    }

    fn farith(&mut self, op: &str, a: &str, b: &str, single: bool, span: [usize; 2]) -> String {
        match op {
            "+" | "-" | "*" | "/" => format!("({a} {op} {b})"),
            "+%" | "-%" | "*%" | "+|" | "-|" | "*|" => format!("({a} {} {b})", &op[..1]),
            "%" if single => format!("((float)fmod((double){a}, (double){b}))"),
            "%" => format!("fmod({a}, {b})"),
            "**" => {
                self.pow = true;
                if single {
                    format!("((float)pow((double){a}, (double){b}))")
                } else {
                    format!("pow({a}, {b})")
                }
            }
            _ => {
                self.unsupported(span, &format!("the operator `{op}` on reals"));
                "0".into()
            }
        }
    }

    /// `a op b` on integers of type `it`, with the semantics of the interpreter.
    fn iarith(
        &mut self,
        op: &str,
        a: &str,
        b: &str,
        it: IntTy,
        b_unsigned: bool,
        span: [usize; 2],
    ) -> String {
        if it.bits == 0 {
            self.untyped = true;
        }
        let c = it.cname();
        let f = it.fam();
        let r = it.range_args();
        let count = |b: &str| {
            if b_unsigned {
                format!("nrt_cnt_u({b})")
            } else {
                format!("nrt_cnt_s({b})")
            }
        };
        match op {
            "+" | "-" | "*" | "/" | "%" => {
                self.chk = true;
                let h = match op {
                    "+" => "add",
                    "-" => "sub",
                    "*" => "mul",
                    "/" => "div",
                    _ => "rem",
                };
                format!("(({c})nrt_{h}_{f}({a}, {b}, {r}))")
            }
            "**" => {
                self.chk = true;
                format!("(({c})nrt_pow_{f}({a}, {}, {r}))", count(b))
            }
            "<<" | ">>" => {
                self.chk = true;
                let h = if op == "<<" { "shl" } else { "shr" };
                format!("(({c})nrt_{h}_{f}({a}, {}, {r}))", count(b))
            }
            "&" | "|" | "^" => {
                if it.small() {
                    format!("(({c})({a} {op} {b}))")
                } else {
                    format!("({a} {op} {b})")
                }
            }
            "+%" | "-%" | "*%" => {
                let o = &op[..1];
                if it.signed {
                    format!(
                        "(({c})nrt_wrap_s((uint64_t)({a}) {o} (uint64_t)({b}), {}))",
                        it.eff()
                    )
                } else if it.eff() >= 32 {
                    format!("({a} {o} {b})")
                } else {
                    format!("(({c})((uint32_t)({a}) {o} (uint32_t)({b})))")
                }
            }
            "+|" | "-|" | "*|" => {
                let h = match op {
                    "+|" => "add",
                    "-|" => "sub",
                    _ => "mul",
                };
                format!("(({c})nrt_sat_{h}_{f}({a}, {b}, {r}))")
            }
            _ => {
                self.unsupported(span, &format!("the operator `{op}`"));
                "0".into()
            }
        }
    }

    fn cast(&mut self, e: &Expr, operand: &Expr) -> String {
        let (Some(to), Some(from)) = (self.ct_type(&e.ty), self.ct_type(&operand.ty)) else {
            self.unsupported(e.span, "this cast");
            return "0".into();
        };
        let v = self.ex(operand);
        match (&from, &to) {
            (Ct::Int(a), Ct::Int(b)) => {
                let c = b.cname();
                let contained = (a.signed == b.signed && a.eff() <= b.eff())
                    || (!a.signed && b.signed && a.eff() < b.eff());
                if contained && a.bits != 0 {
                    return format!("(({c})({v}))");
                }
                self.chk = true;
                format!(
                    "(({c})nrt_{}2{}({v}, {}))",
                    a.fam(),
                    b.fam(),
                    b.range_args()
                )
            }
            (Ct::Int(_) | Ct::Bool, Ct::F32) => format!("((float)({v}))"),
            (Ct::Int(_) | Ct::Bool, Ct::F64) => format!("((double)({v}))"),
            (Ct::F32 | Ct::F64, Ct::Int(b)) => {
                self.chk = true;
                let f = b.fam();
                format!(
                    "(({})nrt_f2{f}((double)({v}), {}))",
                    b.cname(),
                    b.range_args()
                )
            }
            (Ct::F32 | Ct::F64, Ct::F32) => format!("((float)({v}))"),
            (Ct::F64, Ct::F64) => v,
            (Ct::F32, Ct::F64) => {
                // A plain real cast to a quantity scales it; a quantity to a quantity keeps it.
                match &e.ty {
                    Type::Num { unit: Some(u), .. }
                        if !matches!(&operand.ty, Type::Num { unit: Some(_), .. }) =>
                    {
                        let (sc, of) = (nirx::parse_num(&u.scale), nirx::parse_num(&u.offset));
                        format!(
                            "((double)({v}) * {} + {})",
                            flit(sc, false),
                            flit(of, false)
                        )
                    }
                    _ => format!("((double)({v}))"),
                }
            }
            (Ct::Int(_), Ct::Bool) | (Ct::Bool, Ct::Int(_)) => {
                format!("(({})({v}))", self.cty(&to))
            }
            _ => {
                self.unsupported(e.span, "this cast");
                "0".into()
            }
        }
    }

    // ---- calls ----------------------------------------------------------------------

    fn call(&mut self, e: &Expr, callee: &Expr, args: &[neuron_nir::Arg]) -> String {
        for a in args {
            if a.mutable {
                self.unsupported(a.value.span, "a `mut` argument");
            }
        }
        match &callee.e {
            ExprKind::Ref { id, name } => match self.defs.get(id).copied() {
                Some(ItemDef::Fn { func }) => {
                    self.calls.insert(id.clone());
                    self.user_call(func, args)
                }
                Some(ItemDef::Struct { name, fields, .. }) => {
                    self.struct_ctor(id, name, fields, args)
                }
                _ => {
                    self.unsupported(e.span, &format!("the call to `{name}`"));
                    "0".into()
                }
            },
            ExprKind::Global { name } => self.builtin(e, name, args),
            _ => {
                self.unsupported(e.span, "this call");
                "0".into()
            }
        }
    }

    fn user_call(&mut self, func: &FnDecl, args: &[neuron_nir::Arg]) -> String {
        let mut out = Vec::new();
        let mut pos = args.iter().filter(|a| a.name.is_none()).map(|a| &a.value);
        for p in func.sig.params.iter().filter(|p| p.name != "self") {
            let want = p.ty.as_ref().and_then(|t| self.ct_te(t));
            let named = args
                .iter()
                .find(|a| a.name.as_deref() == Some(p.name.as_str()))
                .map(|a| &a.value);
            let v = match named.or_else(|| pos.next()) {
                Some(v) => self.ex_t(v, want.as_ref()),
                None => match &p.default {
                    Some(d) => self.ex_t(d, want.as_ref()),
                    None => "0".into(),
                },
            };
            out.push(v);
        }
        format!("{}({})", self.fn_name(&func.sig.name), out.join(", "))
    }

    fn struct_ctor(
        &mut self,
        id: &str,
        _name: &str,
        fields: &[neuron_nir::Field],
        args: &[neuron_nir::Arg],
    ) -> String {
        let mut pos = args.iter().filter(|a| a.name.is_none()).map(|a| &a.value);
        let mut out = Vec::new();
        for f in fields {
            let want = self.ct_te(&f.ty);
            let named = args
                .iter()
                .find(|a| a.name.as_deref() == Some(f.name.as_str()))
                .map(|a| &a.value);
            let v = match named.or_else(|| pos.next()) {
                Some(v) => self.ex_t(v, want.as_ref()),
                None => match &f.default {
                    Some(d) => self.ex_t(d, want.as_ref()),
                    None => want.as_ref().map_or("0".into(), |w| self.zero(w)),
                },
            };
            out.push(format!(".{} = {v}", ident(&f.name)));
        }
        let t = self.type_name(id);
        if out.is_empty() {
            format!("({t}){{0}}")
        } else {
            format!("({t}){{ {} }}", out.join(", "))
        }
    }

    fn builtin(&mut self, e: &Expr, name: &str, args: &[neuron_nir::Arg]) -> String {
        let n = args.len();
        let arity = match name {
            "abs" | "floor" | "ceil" | "round" | "sqrt" => 1,
            "min" | "max" => 2,
            "clamp" => 3,
            _ => {
                self.unsupported(e.span, &format!("the call to `{name}`"));
                return "0".into();
            }
        };
        if n != arity {
            self.unsupported(e.span, &format!("`{name}` with {n} arguments"));
            return "0".into();
        }
        let Some(ct) = self.ct_type(&e.ty) else {
            self.unsupported(e.span, &format!("`{name}` on this type"));
            return "0".into();
        };
        let vals: Vec<String> = args
            .iter()
            .map(|a| self.ex_t(&a.value, Some(&ct)))
            .collect();
        match (&ct, name) {
            (Ct::Int(it), "abs") => {
                if it.signed {
                    self.neg = true;
                    format!(
                        "(({})nrt_abs_s({}, {}))",
                        it.cname(),
                        vals[0],
                        it.range_args()
                    )
                } else {
                    vals[0].clone()
                }
            }
            (Ct::Int(it), "min" | "max") => format!(
                "(({})nrt_{name}_{}({}, {}))",
                it.cname(),
                it.fam(),
                vals[0],
                vals[1]
            ),
            (Ct::Int(it), "clamp") => format!(
                "(({})nrt_clamp_{}({}, {}, {}))",
                it.cname(),
                it.fam(),
                vals[0],
                vals[1],
                vals[2]
            ),
            (Ct::Int(_), "floor" | "ceil" | "round") => vals[0].clone(),
            (Ct::F32, "abs" | "floor" | "ceil" | "round" | "sqrt") => {
                let f = if name == "abs" { "fabs" } else { name };
                format!("{f}f({})", vals[0])
            }
            (Ct::F64, "abs" | "floor" | "ceil" | "round" | "sqrt") => {
                let f = if name == "abs" { "fabs" } else { name };
                format!("{f}({})", vals[0])
            }
            (Ct::F32, "min" | "max" | "clamp") => {
                format!("((float)nrt_{name}_f({}))", vals.join(", "))
            }
            (Ct::F64, "min" | "max" | "clamp") => format!("nrt_{name}_f({})", vals.join(", ")),
            _ => {
                self.unsupported(e.span, &format!("`{name}` on this type"));
                "0".into()
            }
        }
    }

    // ---- statements -----------------------------------------------------------------

    fn declare_unused(&mut self, w: &mut Writer, cname: &str) {
        if !self.reads_final.contains(cname) {
            w.line(&format!("(void){cname};"));
        }
    }

    fn block(&mut self, w: &mut Writer, b: &Block) {
        if !b.contracts.is_empty() {
            self.unsupported(b.contracts[0].span, "a contract inside a block");
        }
        for s in &b.stmts {
            self.stmt(w, s);
        }
    }

    fn place(&mut self, p: &Expr) -> String {
        match &p.e {
            ExprKind::Local { id, name } => self.local_name(id, name),
            ExprKind::Field { base, name, .. }
                if matches!(self.ct_type(&base.ty), Some(Ct::Struct(_))) =>
            {
                let b = self.place(base);
                format!("{b}.{}", ident(name))
            }
            _ => {
                self.unsupported(p.span, "an assignment to this place");
                "_".into()
            }
        }
    }

    /// The text of a loop bound and the number to write in the `max` comment.
    fn bound(&mut self, max: Option<&Expr>) -> (String, Option<u64>) {
        match max {
            None => ("NRT_LOOP_CAP".into(), Some(1_000_000)),
            Some(m) => match nirx::int(m) {
                Some(v) if v >= 0 => (format!("{v}u"), u64::try_from(v).ok()),
                _ => {
                    let ct = self.ct_type(&m.ty);
                    let v = self.ex_t(m, ct.as_ref());
                    match ct {
                        Some(Ct::Int(it)) if it.signed => (format!("nrt_bound_s({v})"), None),
                        Some(Ct::Int(_)) => (format!("((uint64_t)({v}))"), None),
                        _ => {
                            self.unsupported(m.span, "a loop bound of this type");
                            ("0u".into(), None)
                        }
                    }
                }
            },
        }
    }

    fn stmt(&mut self, w: &mut Writer, s: &Stmt) {
        let mark = w.mark();
        match &s.s {
            StmtKind::Let { pattern, value, .. } => match &pattern.p {
                PatternKind::Wildcard => {
                    let v = self.ex(value);
                    w.line(&format!("(void)({v});"));
                }
                PatternKind::Bind { name, id, ty } => {
                    let ct = self.ct_type(ty).or_else(|| self.ct_type(&value.ty));
                    match ct {
                        Some(ct) if ct != Ct::Void => {
                            let v = self.ex_t(value, Some(&ct));
                            let n = self.local_name(id, name);
                            w.line(&format!("{} {n} = {v};", self.cty(&ct)));
                            self.declare_unused(w, &n);
                        }
                        _ => self.unsupported(pattern.span, "a variable of this type"),
                    }
                }
                _ => self.unsupported(pattern.span, "this pattern"),
            },
            StmtKind::Assign {
                place,
                op,
                value,
                when,
            } => {
                if let Some(c) = when {
                    let c = self.ex(c);
                    w.line(&format!("if ({c}) {{"));
                    w.indent();
                }
                let ct = self.ct_type(&place.ty);
                let rhs = if op == "=" {
                    self.ex_t(value, ct.as_ref())
                } else {
                    let cur = self.ex(place);
                    let bop = op.strip_suffix('=').unwrap_or(op);
                    match &ct {
                        Some(Ct::Int(it)) => {
                            let counted = matches!(bop, "<<" | ">>" | "**");
                            let b = if counted {
                                self.ex(value)
                            } else {
                                self.ex_t(value, ct.as_ref())
                            };
                            let bu =
                                matches!(self.ct_type(&value.ty), Some(Ct::Int(i)) if !i.signed);
                            self.iarith(bop, &cur, &b, *it, bu, s.span)
                        }
                        Some(Ct::F32 | Ct::F64) => {
                            let b = self.ex_t(value, ct.as_ref());
                            self.farith(bop, &cur, &b, ct == Some(Ct::F32), s.span)
                        }
                        _ => {
                            self.unsupported(s.span, "this compound assignment");
                            "0".into()
                        }
                    }
                };
                let p = self.place(place);
                w.line(&format!("{p} = {rhs};"));
                if when.is_some() {
                    w.dedent();
                    w.line("}");
                }
            }
            StmtKind::Expr { expr } => {
                let v = self.ex(expr);
                let is_call = matches!(&expr.e, ExprKind::Call { .. });
                if is_call {
                    w.line(&format!("{v};"));
                } else {
                    w.line(&format!("(void)({v});"));
                }
            }
            StmtKind::Pass => {}
            StmtKind::Return { value } => match value {
                Some(v) => {
                    let ret = self.ret.clone();
                    let t = self.ex_t(v, Some(&ret));
                    w.line(&format!("return {t};"));
                }
                None => w.line("return;"),
            },
            StmtKind::Break => w.line("break;"),
            StmtKind::Continue => w.line("continue;"),
            StmtKind::If {
                branches,
                otherwise,
            } => {
                for (k, br) in branches.iter().enumerate() {
                    let c = self.ex(&br.cond);
                    if k == 0 {
                        w.line(&format!("if ({c}) {{"));
                    } else {
                        w.line(&format!("}} else if ({c}) {{"));
                    }
                    w.indent();
                    self.block(w, &br.body);
                    w.dedent();
                }
                if let Some(o) = otherwise {
                    w.line("} else {");
                    w.indent();
                    self.block(w, o);
                    w.dedent();
                }
                w.line("}");
            }
            StmtKind::For {
                pattern,
                iter,
                body,
                ..
            } => self.for_stmt(w, s, pattern, iter, body),
            StmtKind::While {
                pattern,
                cond,
                max,
                body,
            } => {
                if pattern.is_some() {
                    self.unsupported(s.span, "`while let`");
                } else {
                    let k = self.next_tmp();
                    let (b, comment) = self.bound(max.as_ref());
                    let c = self.ex(cond);
                    w.line(&format!("uint64_t n{k} = 0u;"));
                    let note = comment.map_or(String::new(), |n| format!(" /* max {n} */"));
                    w.line(&format!("while (({c}) && n{k} < {b}){note} {{"));
                    w.indent();
                    w.line(&format!("n{k} += 1u;"));
                    self.block(w, body);
                    w.dedent();
                    w.line("}");
                }
            }
            StmtKind::Loop { max, body } => {
                let k = self.next_tmp();
                let (b, _) = self.bound(max.as_ref());
                w.line(&format!("for (uint64_t n{k} = 0u; n{k} < {b}; n{k}++) {{"));
                w.indent();
                self.block(w, body);
                w.dedent();
                w.line("}");
            }
            StmtKind::Atomic { body } => self.block(w, body),
            StmtKind::Match { value, arms } => self.match_stmt(w, s, value, arms),
            _ => self.unsupported(s.span, "this statement"),
        }
        let file = self.file.clone();
        w.map(mark, &file, s.span, None);
    }

    fn for_stmt(&mut self, w: &mut Writer, s: &Stmt, pattern: &Pattern, iter: &Expr, body: &Block) {
        let ExprKind::Range { lo, hi, inclusive } = &iter.e else {
            self.unsupported(s.span, "a `for` over something that is not a range");
            return;
        };
        let (var, ct) = match &pattern.p {
            PatternKind::Bind { name, id, ty } => {
                let ct = self
                    .ct_type(ty)
                    .or_else(|| self.ct_type(&lo.ty))
                    .filter(|c| matches!(c, Ct::Int(_)));
                (Some(self.local_name(id, name)), ct)
            }
            PatternKind::Wildcard => (
                None,
                self.ct_type(&lo.ty).filter(|c| matches!(c, Ct::Int(_))),
            ),
            _ => (None, None),
        };
        let Some(Ct::Int(it)) = ct else {
            self.unsupported(s.span, "a `for` over a range of this type");
            return;
        };
        let cmp = if *inclusive { "<=" } else { "<" };
        let c = it.cname();
        // Constant bounds that fit the type: a plain C `for`.
        if let (Some(a), Some(b)) = (nirx::int(lo), nirx::int(hi)) {
            let max = it.hi();
            let fits = i128::from(a) >= it.lo()
                && if *inclusive {
                    i128::from(b) < max
                } else {
                    i128::from(b) <= max
                };
            if fits {
                let k = self.next_tmp();
                let v = var.clone().unwrap_or_else(|| format!("_i{k}"));
                w.line(&format!(
                    "for ({c} {v} = {}; {v} {cmp} {}; {v}++) {{",
                    lit_int(it, i128::from(a)),
                    lit_int(it, i128::from(b))
                ));
                w.indent();
                self.block(w, body);
                w.dedent();
                w.line("}");
                return;
            }
        }
        // General case: 64-bit counters, then the loop variable.
        let k = self.next_tmp();
        let wide = if it.signed { "int64_t" } else { "uint64_t" };
        let a = self.ex_t(lo, Some(&Ct::Int(it)));
        let b = self.ex_t(hi, Some(&Ct::Int(it)));
        w.line("{");
        w.indent();
        w.line(&format!("{wide} lo{k} = ({wide})({a});"));
        w.line(&format!("{wide} hi{k} = ({wide})({b});"));
        w.line(&format!("{wide} k{k};"));
        w.line(&format!("for (k{k} = lo{k}; k{k} {cmp} hi{k}; k{k}++) {{"));
        w.indent();
        if let Some(v) = &var {
            w.line(&format!("{c} {v} = ({c})k{k};"));
            self.declare_unused(w, v);
        }
        self.block(w, body);
        w.dedent();
        w.line("}");
        w.dedent();
        w.line("}");
    }

    /// The enum constant a `Mode.Auto` path names, for a subject of enum type `id`.
    fn variant_of(&self, id: &str, path: &str) -> Option<String> {
        let ItemDef::Enum { name, variants, .. } = self.defs.get(id).copied()? else {
            return None;
        };
        let last = path.rsplit('.').next().unwrap_or(path);
        variants
            .iter()
            .any(|v| v.name == last)
            .then(|| format!("{}_{}_{}", self.prefix, ident(name), ident(last)))
    }

    /// The condition under which `p` matches the C value `s`; names it binds are appended.
    fn pat(
        &mut self,
        p: &Pattern,
        s: &str,
        sct: &Ct,
        binds: &mut Vec<(String, String)>,
    ) -> Option<String> {
        match &p.p {
            PatternKind::Wildcard => Some("1".into()),
            PatternKind::Bind { name, id, .. } => {
                binds.push((self.local_name(id, name), s.to_string()));
                Some("1".into())
            }
            PatternKind::Lit { value } => {
                let v = self.ex_t(value, Some(sct));
                Some(format!("({s} == {v})"))
            }
            PatternKind::Range { lo, hi, inclusive } => {
                let a = self.ex_t(lo, Some(sct));
                let b = self.ex_t(hi, Some(sct));
                let cmp = if *inclusive { "<=" } else { "<" };
                Some(format!("({a} <= {s} && {s} {cmp} {b})"))
            }
            PatternKind::Path { path } => {
                if let Ct::Enum(id) = sct {
                    if let Some(v) = self.variant_of(id, path) {
                        return Some(format!("({s} == {v})"));
                    }
                }
                self.unsupported(p.span, "this pattern");
                None
            }
            PatternKind::Or { alts } => {
                let mut parts = Vec::new();
                for a in alts {
                    let mut inner = Vec::new();
                    let c = self.pat(a, s, sct, &mut inner)?;
                    if !inner.is_empty() {
                        self.unsupported(a.span, "a binding inside an alternative pattern");
                        return None;
                    }
                    parts.push(c);
                }
                Some(format!("({})", parts.join(" || ")))
            }
            _ => {
                self.unsupported(p.span, "this pattern");
                None
            }
        }
    }

    fn match_stmt(&mut self, w: &mut Writer, s: &Stmt, value: &Expr, arms: &[neuron_nir::Arm]) {
        let Some(sct) = self
            .ct_type(&value.ty)
            .filter(|c| matches!(c, Ct::Int(_) | Ct::Bool | Ct::Enum(_)))
        else {
            self.unsupported(s.span, "a `match` on a value of this type");
            return;
        };
        let k = self.next_tmp();
        let subject = format!("m{k}");
        let v = self.ex_t(value, Some(&sct));
        let guarded = arms.iter().any(|a| a.guard.is_some());
        w.line("{");
        w.indent();
        w.line(&format!("{} {subject} = {v};", self.cty(&sct)));
        w.line(&format!("(void){subject};"));
        if guarded {
            w.line(&format!("int done{k} = 0;"));
        }
        let mut open = 0usize;
        for arm in arms {
            let mut binds = Vec::new();
            let Some(cond) = self.pat(&arm.pattern, &subject, &sct, &mut binds) else {
                continue;
            };
            let decl = |me: &mut Self, w: &mut Writer, binds: &[(String, String)]| {
                for (n, src) in binds {
                    w.line(&format!("{} {n} = {src};", me.cty(&sct)));
                    me.declare_unused(w, n);
                }
            };
            if guarded {
                w.line(&format!("if (!done{k} && {cond}) {{"));
                w.indent();
                decl(self, w, &binds);
                match &arm.guard {
                    Some(g) => {
                        let g = self.ex(g);
                        w.line(&format!("if ({g}) {{"));
                        w.indent();
                        w.line(&format!("done{k} = 1;"));
                        self.block(w, &arm.body);
                        w.dedent();
                        w.line("}");
                    }
                    None => {
                        w.line(&format!("done{k} = 1;"));
                        self.block(w, &arm.body);
                    }
                }
                w.dedent();
                w.line("}");
            } else {
                let head = if open == 0 {
                    format!("if ({cond}) {{")
                } else {
                    format!("}} else if ({cond}) {{")
                };
                w.line(&head);
                open += 1;
                w.indent();
                decl(self, w, &binds);
                self.block(w, &arm.body);
                w.dedent();
            }
        }
        if open > 0 {
            w.line("}");
        }
        w.dedent();
        w.line("}");
    }

    // ---- definitions ----------------------------------------------------------------

    /// The C signature of a function, or `None` after reporting what cannot be written.
    fn signature(&mut self, func: &FnDecl) -> Option<Sig> {
        let mut ok = true;
        if !func.sig.generics.is_empty() {
            self.unsupported(func.sig.span, "a generic function");
            ok = false;
        }
        let ret = match &func.sig.ret {
            None => Some(Ct::Void),
            Some(t) => self.ct_te(t),
        };
        let ret = match ret {
            Some(r) => r,
            None => {
                self.unsupported(func.sig.span, "the return type");
                ok = false;
                Ct::Void
            }
        };
        let mut params = Vec::new();
        for p in &func.sig.params {
            if p.name == "self" {
                self.unsupported(func.sig.span, "a method");
                ok = false;
                continue;
            }
            if p.mutable {
                self.unsupported(func.sig.span, "a `mut` parameter");
                ok = false;
            }
            match p.ty.as_ref().and_then(|t| self.ct_te(t)) {
                Some(ct) if ct != Ct::Void => {
                    params.push(Param {
                        ty: self.cty(&ct),
                        name: pname(&p.name),
                        id: p.id.clone(),
                    });
                }
                _ => {
                    self.unsupported(
                        func.sig.span,
                        &format!("the type of parameter `{}`", p.name),
                    );
                    ok = false;
                }
            }
        }
        if func.contracts.iter().any(|c| c.kind != "requires") {
            let c = func.contracts.iter().find(|c| c.kind != "requires");
            if let Some(c) = c {
                let k = c.kind.clone();
                self.unsupported(c.span, &format!("a `{k}` contract"));
            }
            ok = false;
        }
        ok.then(|| Sig {
            retty: self.cty(&ret),
            ret,
            params,
        })
    }

    fn params_text(params: &[Param]) -> String {
        if params.is_empty() {
            "void".to_string()
        } else {
            params
                .iter()
                .map(|p| format!("{} {}", p.ty, p.name))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    /// Writes the body of a function (between the braces).
    fn body(&mut self, w: &mut Writer, func: &FnDecl, sig: &Sig) {
        let ret = &sig.ret;
        self.ret = ret.clone();
        for p in &sig.params {
            if let Some(id) = &p.id {
                self.locals.insert(id.clone(), p.name.clone());
            }
        }
        for p in &sig.params {
            if !self.reads_final.contains(&p.name) {
                w.line(&format!("(void){};", p.name));
            }
        }
        for c in func.contracts.iter().filter(|c| c.kind == "requires") {
            let cond = self.ex(&c.cond);
            w.line(&format!("if (!{cond}) {{"));
            w.indent();
            w.line("nrt_raise(NRT_CONTRACT_VIOLATION);");
            if *ret == Ct::Void {
                w.line("return;");
            } else {
                w.line(&format!("return {};", self.zero(ret)));
            }
            w.dedent();
            w.line("}");
        }
        match &func.body {
            FnBody::Block { block } => self.block(w, block),
            FnBody::Expr { expr } => {
                let v = self.ex_t(expr, Some(ret));
                if *ret == Ct::Void {
                    w.line(&format!("(void)({v});"));
                } else {
                    w.line(&format!("return {v};"));
                }
            }
        }
        if *ret != Ct::Void {
            w.line(&format!("return {};", self.zero(ret)));
        }
    }

    fn reset_flags(&mut self) {
        self.chk = false;
        self.neg = false;
        self.untyped = false;
        self.pow = false;
        self.units.set(false);
    }

    /// Records the fidelity entries of the function just written.
    fn record(&mut self, func: &FnDecl) {
        let span = func.sig.span;
        let file = self.file.clone();
        if self.chk {
            self.fid.record(
                &file,
                span,
                Class::Equivalent,
                "checked integer arithmetic",
                "the interpreter stops the program with a fault (Overflow, DivideByZero); the C code records the first one in `nrt_fault`, yields 0 for that operation and goes on until the function returns, so callers must clear `nrt_fault` before the call and test it after",
            );
        }
        if self.neg {
            self.fid.record(
                &file,
                span,
                Class::Equivalent,
                "negation and abs of integers",
                "the C code faults when the result leaves the range of the type; the interpreter negates exactly and checks the range only when the value is stored or used",
            );
        }
        if self.untyped {
            self.fid.record(
                &file,
                span,
                Class::Equivalent,
                "untyped integer",
                "an integer without a declared type is computed in 64 bits; the interpreter uses 128 bits",
            );
        }
        if self.pow {
            self.fid.record(
                &file,
                span,
                Class::Approximate,
                "real power",
                "`pow` comes from the C library of the target and may differ from the interpreter in the last bit",
            );
        }
        if self.units.get() {
            self.fid.record(
                &file,
                span,
                Class::Equivalent,
                "units",
                "quantities are plain doubles in base SI units (mm in metres, ms in seconds, degC in kelvin); the C code does not check units",
            );
        }
    }
}

/// A parameter of a generated function.
struct Param {
    ty: String,
    name: String,
    id: Option<String>,
}

/// The C signature of a function.
struct Sig {
    retty: String,
    ret: Ct,
    params: Vec<Param>,
}

/// The locals a function reads and the functions it calls, found by a first pass.
struct Pre {
    reads: BTreeSet<String>,
    calls: BTreeSet<String>,
}

fn is_c_module(m: &Module) -> bool {
    m.dialect == "core"
}

fn guard(name: &str) -> String {
    format!("{}_H", ident(name).to_uppercase())
}

impl Emitter for C {
    fn manifest(&self) -> Manifest {
        Manifest {
            name: "c",
            language: "C99",
            dialects: &["core"],
            profiles: &[],
            constructs: CONSTRUCTS,
        }
    }

    fn emit(&self, nir: &Nir, _opts: &EmitOptions) -> Result<Package, Vec<Diag>> {
        let mods: Vec<usize> = nir
            .modules
            .iter()
            .enumerate()
            .filter(|(_, m)| is_c_module(m))
            .map(|(i, _)| i)
            .collect();
        if mods.is_empty() {
            return Err(vec![Diag {
                code: "NE0610",
                message: "the program has no `core` module to translate".into(),
                help: Some("the c emitter reads modules declared `module name for core`".into()),
                file: nir
                    .modules
                    .first()
                    .map(|m| m.file.clone())
                    .unwrap_or_default(),
                span: [0, 0],
            }]);
        }
        let mut fid = Fidelity::new("emit-c", "core.c99");
        let mut pkg = Package::new(fid.clone());
        let mut diags = Vec::new();
        let mut wiring = String::from(
            "# Using the generated C\n\nPlain C99 (`-std=c99 -pedantic` clean). Compile the `.c` files together with `nrt_core.c`, keep `nrt_core.h` on the include path and link `-lm`.\n\nFaults: the interpreter stops the program on integer overflow, a zero divisor or a false `requires`. The C sets the global `nrt_fault` (`NRT_OVERFLOW`, `NRT_DIVIDE_BY_ZERO`, `NRT_CONTRACT_VIOLATION`; the first one wins) and keeps going with 0 for the failing operation. Set `nrt_fault = 0` before a call and read it after; when it is not 0 the returned value is meaningless. The code is not thread safe.\n\nUnits are erased: a quantity is a `double` in base SI units (mm in metres, ms in seconds, degC in kelvin).\n\n`test` blocks are not translated; run them with `neuron test`.\n\n",
        );
        for m in mods {
            let module = &nir.modules[m];
            match emit_module(module) {
                Ok(out) => {
                    wiring.push_str(&format!(
                        "## `{0}.c` and `{0}.h`\n\n{1}\n",
                        ident(&module.name),
                        out.wiring
                    ));
                    pkg.add(&format!("{}.c", ident(&module.name)), out.c);
                    pkg.add_text(&format!("{}.h", ident(&module.name)), out.h);
                    fid.merge(out.fid);
                }
                Err(mut d) => diags.append(&mut d),
            }
        }
        if !diags.is_empty() {
            return Err(diags);
        }
        pkg.add_text("nrt_core.h", RUNTIME_H.to_string());
        pkg.add_text("nrt_core.c", RUNTIME_C.to_string());
        pkg.wiring = wiring;
        pkg.fidelity = fid;
        Ok(pkg)
    }
}

struct Out {
    c: Writer,
    h: String,
    fid: Fidelity,
    wiring: String,
}

fn const_text(l: &mut Lower<'_>, item: &Item, name: &str, value: &Expr) -> Option<String> {
    let ct = l.ct_type(&value.ty)?;
    let v = match &ct {
        Ct::Bool => nirx::boolean(value).map(|b| b.to_string()),
        Ct::Int(it) => nirx::int(value).map(|v| {
            let lit = lit_int(*it, i128::from(v));
            if it.small() {
                format!("(({}){lit})", it.cname())
            } else {
                lit
            }
        }),
        Ct::F32 => nirx::si(value).map(|x| flit(x, true)),
        Ct::F64 => nirx::si(value).map(|x| flit(x, false)),
        _ => None,
    };
    match v {
        Some(v) => Some(format!("#define {} {v}", l.const_name(name))),
        None => {
            l.unsupported(item.span, &format!("the constant `{name}` (only literals)"));
            None
        }
    }
}

fn emit_module(module: &Module) -> Result<Out, Vec<Diag>> {
    let prefix = ident(&module.name);
    let mut l = Lower::new(module, prefix.clone());
    let mut header_types: Vec<String> = Vec::new();
    let mut pub_consts: Vec<String> = Vec::new();
    let mut priv_consts: Vec<String> = Vec::new();
    let mut fns: Vec<(&Item, &FnDecl)> = Vec::new();

    for item in &module.items {
        match &item.def {
            ItemDef::Use { .. } | ItemDef::Test { .. } | ItemDef::Unit { .. } => {}
            ItemDef::Const { name, value, .. } => {
                if let Some(t) = const_text(&mut l, item, name, value) {
                    l.fid.exact();
                    if item.public {
                        pub_consts.push(t);
                    } else {
                        priv_consts.push(t);
                    }
                }
            }
            ItemDef::Struct {
                name,
                generics,
                fields,
            } => {
                if !generics.is_empty() {
                    l.unsupported(item.span, "a generic struct");
                    continue;
                }
                let mut lines = vec!["typedef struct {".to_string()];
                let mut ok = true;
                for f in fields {
                    match l.ct_te(&f.ty) {
                        Some(ct) if ct != Ct::Void => {
                            lines.push(format!("  {} {};", l.cty(&ct), ident(&f.name)));
                        }
                        _ => {
                            l.unsupported(f.span, &format!("the type of field `{}`", f.name));
                            ok = false;
                        }
                    }
                }
                if fields.is_empty() {
                    l.unsupported(item.span, "a struct without fields");
                    ok = false;
                }
                if ok {
                    lines.push(format!("}} {}_{};", prefix, ident(name)));
                    header_types.push(lines.join("\n"));
                    l.fid.exact();
                }
            }
            ItemDef::Enum {
                name,
                generics,
                variants,
                ..
            } => {
                if !generics.is_empty() {
                    l.unsupported(item.span, "a generic enum");
                    continue;
                }
                if variants.iter().any(|v| !v.payload.is_empty()) {
                    l.unsupported(item.span, "an enum variant with a payload");
                    continue;
                }
                if variants.is_empty() {
                    l.unsupported(item.span, "an enum without variants");
                    continue;
                }
                let mut lines = vec!["typedef enum {".to_string()];
                let mut next: i128 = 0;
                for v in variants {
                    let value = match &v.value {
                        Some(e) => match nirx::int(e) {
                            Some(n) => i128::from(n),
                            None => {
                                l.unsupported(v.span, "a computed enum value");
                                next
                            }
                        },
                        None => next,
                    };
                    next = value + 1;
                    lines.push(format!(
                        "  {prefix}_{}_{} = {value},",
                        ident(name),
                        ident(&v.name)
                    ));
                }
                lines.push(format!("}} {}_{};", prefix, ident(name)));
                header_types.push(lines.join("\n"));
                l.fid.exact();
            }
            ItemDef::Fn { func } => fns.push((item, func)),
            ItemDef::Class { .. } => l.unsupported(item.span, "a class"),
            ItemDef::Interface { .. } => l.unsupported(item.span, "an interface"),
            ItemDef::Var { .. } => l.unsupported(item.span, "a module variable"),
            ItemDef::Alias { .. } => l.unsupported(item.span, "a type alias"),
            _ => l.unsupported(item.span, "this item"),
        }
    }

    // Signatures first: a function without one is reported and skipped.
    let mut sigs: BTreeMap<String, Sig> = BTreeMap::new();
    for (item, func) in &fns {
        if let Some(sig) = l.signature(func) {
            sigs.insert(item.id.clone(), sig);
        }
    }

    // First pass: the locals each function reads and the functions it calls.
    let mut pre: BTreeMap<String, Pre> = BTreeMap::new();
    for (item, func) in &fns {
        let Some(sig) = sigs.get(&item.id) else {
            continue;
        };
        l.reads.clear();
        l.calls.clear();
        l.locals.clear();
        let mut scratch = Writer::new("  ");
        l.body(&mut scratch, func, sig);
        pre.insert(
            item.id.clone(),
            Pre {
                reads: std::mem::take(&mut l.reads),
                calls: std::mem::take(&mut l.calls),
            },
        );
    }
    // The first pass reports a construct that the second one reports again: keep one copy.
    l.tmp = 0;

    // Recursion: the interpreter faults at depth 100, plain C does not.
    let mut order: Vec<String> = Vec::new();
    let mut state: BTreeMap<String, u8> = BTreeMap::new();
    let ids: Vec<String> = fns.iter().map(|(i, _)| i.id.clone()).collect();
    fn visit(
        id: &str,
        pre: &BTreeMap<String, Pre>,
        state: &mut BTreeMap<String, u8>,
        order: &mut Vec<String>,
        cyclic: &mut BTreeSet<String>,
    ) {
        match state.get(id) {
            Some(2) => return,
            Some(1) => {
                cyclic.insert(id.to_string());
                return;
            }
            _ => {}
        }
        state.insert(id.to_string(), 1);
        if let Some(p) = pre.get(id) {
            for c in &p.calls {
                visit(c, pre, state, order, cyclic);
            }
        }
        state.insert(id.to_string(), 2);
        order.push(id.to_string());
    }
    let mut cyclic = BTreeSet::new();
    for id in &ids {
        visit(id, &pre, &mut state, &mut order, &mut cyclic);
    }
    for (item, func) in &fns {
        if cyclic.contains(&item.id) {
            l.unsupported(
                func.sig.span,
                "a recursive function (the interpreter faults at depth 100, C has no such limit)",
            );
        }
    }

    // Second pass: the real output.
    let mut w = Writer::new("  ");
    w.line(&format!(
        "/* Generated by `neuron emit c` from {}. Do not edit. */",
        module.file.replace('\\', "/")
    ));
    w.line(&format!("#include \"{prefix}.h\""));
    w.line("");
    for c in &priv_consts {
        w.line(c);
    }
    if !priv_consts.is_empty() {
        w.line("");
    }
    let mut protos: Vec<String> = Vec::new();
    let by_id: BTreeMap<&str, (&Item, &FnDecl)> =
        fns.iter().map(|(i, f)| (i.id.as_str(), (*i, *f))).collect();
    let called: BTreeSet<&String> = pre.values().flat_map(|p| p.calls.iter()).collect();
    for id in &order {
        let (Some((item, func)), Some(sig)) = (by_id.get(id.as_str()), sigs.get(id)) else {
            continue;
        };
        let (item, func) = (*item, *func);
        l.reset_flags();
        // The types in the signature count for the units note.
        let _ = l.signature(func);
        l.locals.clear();
        l.calls.clear();
        l.reads_final = pre.get(id).map(|p| p.reads.clone()).unwrap_or_default();
        let name = l.fn_name(&func.sig.name);
        let text = Lower::params_text(&sig.params);
        let retty = &sig.retty;
        let mark = w.mark();
        if item.public {
            protos.push(format!("{retty} {name}({text});"));
            w.line(&format!("{retty} {name}({text}) {{"));
        } else if called.contains(id) {
            w.line(&format!("static {retty} {name}({text}) {{"));
        } else {
            w.line(&format!(
                "static NRT_MAYBE_UNUSED {retty} {name}({text}) {{"
            ));
        }
        w.indent();
        l.body(&mut w, func, sig);
        w.dedent();
        w.line("}");
        let file = l.file.clone();
        w.map(mark, &file, item.span, None);
        w.line("");
        l.fid.exact();
        l.record(func);
    }

    let mut diags: Vec<Diag> = Vec::new();
    for d in std::mem::take(&mut l.diags) {
        if !diags
            .iter()
            .any(|x| x.span == d.span && x.message == d.message)
        {
            diags.push(d);
        }
    }
    if !diags.is_empty() {
        diags.sort_by_key(|d| d.span);
        return Err(diags);
    }

    let mut h = String::new();
    h.push_str(&format!(
        "/* Generated by `neuron emit c` from {}. Do not edit. */\n",
        module.file.replace('\\', "/")
    ));
    let g = guard(&module.name);
    h.push_str(&format!(
        "#ifndef {g}\n#define {g}\n\n#include \"nrt_core.h\"\n\n"
    ));
    h.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");
    for t in &header_types {
        h.push_str(t);
        h.push_str("\n\n");
    }
    for c in &pub_consts {
        h.push_str(c);
        h.push('\n');
    }
    if !pub_consts.is_empty() {
        h.push('\n');
    }
    for p in &protos {
        h.push_str(p);
        h.push('\n');
    }
    h.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n");
    h.push_str(&format!("#endif /* {g} */\n"));

    let mut wiring = String::new();
    for p in &protos {
        wiring.push_str(&format!("- `{p}`\n"));
    }
    Ok(Out {
        c: w,
        h,
        fid: l.fid,
        wiring,
    })
}
