//! Semantic types (spec 03).

use crate::units::Unit;

/// Reference to a user type definition: `(module index, name)`.
pub type DefRef = (usize, String);

/// Kind of a numeric type.
#[derive(Debug, Clone, PartialEq)]
pub enum NumKind {
    /// Integer literal without context.
    LitInt,
    /// Real literal without context.
    LitReal,
    /// Fixed-width integer.
    Int { signed: bool, bits: u8 },
    /// Natural word size of the target.
    Index,
    /// IEEE 754 float of 32 or 64 bits.
    Float(u8),
    /// Fixed point `fixed<int, frac>`.
    Fixed { signed: bool, int: u8, frac: u8 },
    /// Signed time span with nanosecond resolution.
    Duration,
}

impl NumKind {
    /// `true` for the two literal kinds.
    pub fn is_lit(&self) -> bool {
        matches!(self, NumKind::LitInt | NumKind::LitReal)
    }

    /// `true` for integer kinds (including `index` and integer literals).
    pub fn is_integer(&self) -> bool {
        matches!(self, NumKind::LitInt | NumKind::Int { .. } | NumKind::Index)
    }

    /// Source name of the kind.
    pub fn name(&self) -> String {
        match self {
            NumKind::LitInt => "integer".into(),
            NumKind::LitReal => "real".into(),
            NumKind::Int { signed, bits } => format!("{}{bits}", if *signed { "i" } else { "u" }),
            NumKind::Index => "index".into(),
            NumKind::Float(b) => format!("f{b}"),
            NumKind::Fixed { signed, int, frac } => {
                format!("{}fixed<{int}, {frac}>", if *signed { "" } else { "u" })
            }
            NumKind::Duration => "duration".into(),
        }
    }

    /// Inclusive value range of an integer kind (`index` is treated as unbounded above).
    pub fn int_range(&self) -> Option<(i128, i128)> {
        match self {
            NumKind::Int { signed, bits } => {
                let b = u32::from(*bits);
                if *signed {
                    Some((-(1i128 << (b - 1)), (1i128 << (b - 1)) - 1))
                } else {
                    Some((0, (1i128 << b) - 1))
                }
            }
            NumKind::Index => Some((0, i128::from(u64::MAX))),
            _ => None,
        }
    }
}

/// Signature of a function or method.
#[derive(Debug, Clone, PartialEq)]
pub struct FnTy {
    /// Parameters after `self`.
    pub params: Vec<ParamTy>,
    /// Return type (`Void` when absent).
    pub ret: Ty,
    /// Generic type parameters with their bounds.
    pub generics: Vec<(String, Vec<String>)>,
    /// `Some(mutable)` for methods that take `self`.
    pub self_mode: Option<bool>,
}

/// A parameter of a function signature.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamTy {
    /// Name.
    pub name: String,
    /// Type.
    pub ty: Ty,
    /// `mut` (exclusive).
    pub mutable: bool,
    /// Has a default value.
    pub has_default: bool,
}

/// A semantic type.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    /// Not yet typed external vocabulary. Compatible with everything.
    Unknown,
    /// Type of diverging expressions (reserved; compatible with everything).
    #[allow(dead_code)]
    Never,
    /// No value.
    Void,
    /// `bool`.
    Bool,
    /// Numeric type with a physical unit (dimensionless when plain).
    Num(NumKind, Unit),
    /// `bit<N>` / `sbit<N>`.
    Bits { signed: bool, width: u32 },
    /// `instant`.
    Instant,
    /// `str` or `str<N>`.
    Str(Option<u32>),
    /// `[T; N]`.
    Array(Box<Ty>, Option<u64>),
    /// `[T]` (parameters only).
    Slice(Box<Ty>),
    /// `(A, B, ...)`.
    Tuple(Vec<Ty>),
    /// `range<T>`.
    Range(Box<Ty>),
    /// `T?`.
    Option(Box<Ty>),
    /// `none` without context.
    NoneLit,
    /// `Result<T, E>`.
    Result(Box<Ty>, Box<Ty>),
    /// User struct, enum or class (with generic arguments).
    Adt { def: DefRef, args: Vec<Ty> },
    /// User interface used as a bound or a type.
    Interface(DefRef),
    /// Generic type parameter.
    Param(String),
    /// `channel<T, N>`.
    Channel(Box<Ty>, Option<u64>),
    /// `pool<T, N>`.
    Pool(Box<Ty>, Option<u64>),
    /// `Handle<T>`.
    Handle(Box<Ty>),
    /// A callable.
    Fn(Box<FnTy>),
    /// An `io` binding of the given kind (`gpio.output`, `pwm`, ...).
    Io(String),
    /// Named external type (`point`, `frame`, `xyz`).
    Opaque(String),
    /// The name of a type used as a value (`Mode` in `Mode.Manual`).
    TypeRef(Box<Ty>),
    /// A package module used as a value.
    Module(usize),
    /// A hardware or protocol event (`btn.falling`).
    Event,
}

impl Ty {
    /// A plain number type without unit.
    pub fn num(kind: NumKind) -> Ty {
        Ty::Num(kind, Unit::one())
    }

    /// `true` for [`Ty::Unknown`].
    pub fn is_unknown(&self) -> bool {
        matches!(self, Ty::Unknown)
    }

    /// Replaces the generic parameters named in `subst`.
    pub fn subst(&self, subst: &[(String, Ty)]) -> Ty {
        let s = |t: &Ty| t.subst(subst);
        match self {
            Ty::Param(n) => subst
                .iter()
                .find(|(p, _)| p == n)
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| self.clone()),
            Ty::Array(t, n) => Ty::Array(Box::new(s(t)), *n),
            Ty::Slice(t) => Ty::Slice(Box::new(s(t))),
            Ty::Tuple(v) => Ty::Tuple(v.iter().map(s).collect()),
            Ty::Range(t) => Ty::Range(Box::new(s(t))),
            Ty::Option(t) => Ty::Option(Box::new(s(t))),
            Ty::Result(a, b) => Ty::Result(Box::new(s(a)), Box::new(s(b))),
            Ty::Adt { def, args } => Ty::Adt {
                def: def.clone(),
                args: args.iter().map(s).collect(),
            },
            Ty::Channel(t, n) => Ty::Channel(Box::new(s(t)), *n),
            Ty::Pool(t, n) => Ty::Pool(Box::new(s(t)), *n),
            Ty::Handle(t) => Ty::Handle(Box::new(s(t))),
            Ty::Fn(f) => Ty::Fn(Box::new(FnTy {
                params: f
                    .params
                    .iter()
                    .map(|p| ParamTy {
                        ty: s(&p.ty),
                        ..p.clone()
                    })
                    .collect(),
                ret: s(&f.ret),
                generics: f.generics.clone(),
                self_mode: f.self_mode,
            })),
            other => other.clone(),
        }
    }

    /// Text used in diagnostics.
    pub fn display(&self) -> String {
        match self {
            Ty::Unknown => "_".into(),
            Ty::Never => "never".into(),
            Ty::Void => "()".into(),
            Ty::Bool => "bool".into(),
            Ty::Num(k, u) => {
                let unit = if u.is_dimensionless() && u.label.is_empty() {
                    String::new()
                } else {
                    format!(" {}", u.display())
                };
                format!("{}{unit}", k.name())
            }
            Ty::Bits { signed, width } => {
                format!("{}bit<{width}>", if *signed { "s" } else { "" })
            }
            Ty::Instant => "instant".into(),
            Ty::Str(None) => "str".into(),
            Ty::Str(Some(n)) => format!("str<{n}>"),
            Ty::Array(t, Some(n)) => format!("[{}; {n}]", t.display()),
            Ty::Array(t, None) => format!("[{}; _]", t.display()),
            Ty::Slice(t) => format!("[{}]", t.display()),
            Ty::Tuple(v) => format!(
                "({})",
                v.iter().map(Ty::display).collect::<Vec<_>>().join(", ")
            ),
            Ty::Range(t) => format!("range<{}>", t.display()),
            Ty::Option(t) => format!("{}?", t.display()),
            Ty::NoneLit => "none".into(),
            Ty::Result(a, b) => format!("Result<{}, {}>", a.display(), b.display()),
            Ty::Adt { def, args } => {
                if args.is_empty() {
                    def.1.clone()
                } else {
                    format!(
                        "{}<{}>",
                        def.1,
                        args.iter().map(Ty::display).collect::<Vec<_>>().join(", ")
                    )
                }
            }
            Ty::Interface(d) => d.1.clone(),
            Ty::Param(n) => n.clone(),
            Ty::Channel(t, n) => format!("channel<{}, {}>", t.display(), opt_n(*n)),
            Ty::Pool(t, n) => format!("pool<{}, {}>", t.display(), opt_n(*n)),
            Ty::Handle(t) => format!("Handle<{}>", t.display()),
            Ty::Fn(_) => "fn".into(),
            Ty::Io(k) => format!("io {k}"),
            Ty::Opaque(n) => n.clone(),
            Ty::TypeRef(t) => format!("type {}", t.display()),
            Ty::Module(_) => "module".into(),
            Ty::Event => "event".into(),
        }
    }
}

fn opt_n(n: Option<u64>) -> String {
    n.map_or_else(|| "_".to_string(), |v| v.to_string())
}
