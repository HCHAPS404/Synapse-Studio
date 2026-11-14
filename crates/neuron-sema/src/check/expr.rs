//! Expression typing, coercion and calls.

use super::{default_ty, CallTarget, Checker, DefInfo, FnKey};
use crate::model::{key, Res, SymKind};
use crate::prelude;
use crate::resolve::LocalKind;
use crate::ty::{DefRef, FnTy, NumKind, ParamTy, Ty};
use crate::units::{convert, lookup_unit, Dim, Rat, Unit};
use neuron_syntax::ast::{Arg, BinOp, Expr, ExprKind, Ident, ItemKind, Lit, UnOp, UnitExpr};
use neuron_syntax::Span;

/// `true` when the literal is a zero (which adopts any unit).
fn is_zero_lit(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Lit(Lit::Int(t) | Lit::Real(t)) => Rat::parse(t).is_some_and(|r| r.is_zero()),
        _ => false,
    }
}

fn contains_param(t: &Ty) -> bool {
    match t {
        Ty::Param(_) => true,
        Ty::Array(a, _) | Ty::Slice(a) | Ty::Range(a) | Ty::Option(a) | Ty::Handle(a) => {
            contains_param(a)
        }
        Ty::Channel(a, _) | Ty::Pool(a, _) => contains_param(a),
        Ty::Tuple(v) => v.iter().any(contains_param),
        Ty::Result(a, b) => contains_param(a) || contains_param(b),
        Ty::Adt { args, .. } => args.iter().any(contains_param),
        _ => false,
    }
}

/// Binds generic parameters of `p` by matching it against `a`.
fn unify(p: &Ty, a: &Ty, gens: &[String], subst: &mut Vec<(String, Ty)>) {
    match (p, a) {
        (Ty::Param(n), _) if gens.contains(n) => match subst.iter().position(|(s, _)| s == n) {
            None => subst.push((n.clone(), a.clone())),
            Some(i) => {
                let cur_is_lit = matches!(&subst[i].1, Ty::Num(k, _) if k.is_lit());
                let new_is_lit = matches!(a, Ty::Num(k, _) if k.is_lit());
                if cur_is_lit && !new_is_lit && !a.is_unknown() {
                    subst[i].1 = a.clone();
                }
            }
        },
        (Ty::Array(x, _), Ty::Array(y, _))
        | (Ty::Slice(x), Ty::Array(y, _))
        | (Ty::Slice(x), Ty::Slice(y))
        | (Ty::Range(x), Ty::Range(y))
        | (Ty::Option(x), Ty::Option(y))
        | (Ty::Handle(x), Ty::Handle(y)) => unify(x, y, gens, subst),
        (Ty::Channel(x, _), Ty::Channel(y, _)) | (Ty::Pool(x, _), Ty::Pool(y, _)) => {
            unify(x, y, gens, subst);
        }
        (Ty::Tuple(x), Ty::Tuple(y)) if x.len() == y.len() => {
            for (i, j) in x.iter().zip(y) {
                unify(i, j, gens, subst);
            }
        }
        (Ty::Result(a1, b1), Ty::Result(a2, b2)) => {
            unify(a1, a2, gens, subst);
            unify(b1, b2, gens, subst);
        }
        (Ty::Adt { def: d1, args: x }, Ty::Adt { def: d2, args: y }) if d1 == d2 => {
            for (i, j) in x.iter().zip(y) {
                unify(i, j, gens, subst);
            }
        }
        _ => {}
    }
}

/// Joins the kinds of two numeric operands.
fn unify_kinds(a: &NumKind, b: &NumKind) -> Option<NumKind> {
    use NumKind::*;
    if a == b {
        return Some(a.clone());
    }
    match (a, b) {
        (LitInt, LitReal) | (LitReal, LitInt) => Some(LitReal),
        (LitInt, k) | (k, LitInt) => Some(k.clone()),
        (LitReal, k @ (Float(_) | Fixed { .. } | Duration))
        | (k @ (Float(_) | Fixed { .. } | Duration), LitReal) => Some(k.clone()),
        _ => None,
    }
}

fn kind_accepts(exp: &NumKind, act: &NumKind) -> bool {
    use NumKind::*;
    if exp == act || matches!(exp, LitInt | LitReal) {
        return true;
    }
    matches!(
        (exp, act),
        (
            Int { .. } | Index | Float(_) | Fixed { .. } | Duration,
            LitInt
        ) | (Float(_) | Fixed { .. } | Duration, LitReal)
    )
}

fn is_ord(t: &Ty) -> bool {
    match t {
        Ty::Num(k, _) => !matches!(k, NumKind::Float(_) | NumKind::LitReal),
        Ty::Bits { .. } | Ty::Instant | Ty::Str(_) | Ty::Param(_) | Ty::Unknown | Ty::Never => true,
        _ => false,
    }
}

fn diff_unit(u: &Unit) -> Unit {
    let mut d = u.interval();
    if d.scale == Rat::ONE {
        d.label = "K".to_string();
    }
    d
}

fn io_member(kind: &str, m: &str) -> Ty {
    let f = |params: Vec<(&str, Ty)>, ret: Ty| {
        Ty::Fn(Box::new(FnTy {
            params: params
                .into_iter()
                .map(|(n, ty)| ParamTy {
                    name: n.to_string(),
                    ty,
                    mutable: false,
                    has_default: false,
                })
                .collect(),
            ret,
            generics: Vec::new(),
            self_mode: Some(true),
        }))
    };
    let percent = lookup_unit("%").map_or(Ty::Unknown, |u| Ty::Num(NumKind::Float(32), u));
    let hz = lookup_unit("Hz").map_or(Ty::Unknown, |u| Ty::Num(NumKind::Float(32), u));
    match (kind, m) {
        ("gpio.output" | "gpio.open_drain", "write") => f(vec![("level", Ty::Bool)], Ty::Void),
        ("gpio.output" | "gpio.open_drain", "toggle") => f(vec![], Ty::Void),
        ("gpio.output" | "gpio.open_drain" | "gpio.input", "read") => f(vec![], Ty::Bool),
        ("gpio.input", "rising" | "falling" | "changed") => Ty::Bool,
        ("pwm", "duty") => percent,
        ("pwm", "freq") => hz,
        ("adc", "read") => f(
            vec![],
            Ty::num(NumKind::Int {
                signed: false,
                bits: 16,
            }),
        ),
        ("uart", "print") => f(vec![("text", Ty::Str(None))], Ty::Bool),
        ("uart", "ready") => f(vec![], Ty::Bool),
        ("uart", "byte") => f(
            vec![],
            Ty::num(NumKind::Int {
                signed: false,
                bits: 8,
            }),
        ),
        _ => Ty::Unknown,
    }
}

fn block_member(block: &str, m: &str) -> Ty {
    let seconds = lookup_unit("s").map_or(Ty::Unknown, |u| Ty::Num(NumKind::Duration, u));
    match (block, m) {
        ("ton" | "tof" | "tp", "input" | "q") => Ty::Bool,
        ("ton" | "tof" | "tp", "et") => seconds,
        ("r_trig" | "f_trig", "input" | "q") => Ty::Bool,
        ("sr" | "rs", "set" | "reset" | "q") => Ty::Bool,
        ("ctu" | "ctd" | "ctud", "q" | "reset" | "cu" | "cd" | "load") => Ty::Bool,
        _ => Ty::Unknown,
    }
}

fn parse_bits_lit(text: &str) -> Ty {
    let width = text
        .split('\'')
        .next()
        .and_then(|w| w.parse::<u32>().ok())
        .unwrap_or(1);
    Ty::Bits {
        signed: false,
        width,
    }
}

fn io_kind_name(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Call(c, _) => io_kind_name(c),
        ExprKind::Field(b, id) => format!("{}.{}", io_kind_name(b), id.name),
        ExprKind::Name(id) => id.name.clone(),
        _ => String::new(),
    }
}

impl Checker<'_> {
    // ---- module-level items --------------------------------------------

    pub(crate) fn item_ty(&mut self, module: usize, name: &str) -> Ty {
        let k = (module, name.to_string());
        if let Some(t) = self.out.item_ty.get(&k) {
            return t.clone();
        }
        if !self.in_progress.insert(k.clone()) {
            return Ty::Unknown;
        }
        let t = self.compute_item_ty(module, name);
        self.in_progress.remove(&k);
        self.out.item_ty.insert(k, t.clone());
        t
    }

    fn infer_in(&mut self, module: usize, e: &Expr, exp: Option<&Ty>) -> Ty {
        let saved = std::mem::replace(&mut self.cur, super::Ctx::new(module));
        self.silent += 1;
        let t = self.expr(e, exp);
        self.silent -= 1;
        self.cur = saved;
        t
    }

    fn lower_in_module(&mut self, module: usize, t: &neuron_syntax::ast::Type) -> Ty {
        let saved = std::mem::replace(&mut self.cur, super::Ctx::new(module));
        let ty = self.lower(t);
        self.cur = saved;
        ty
    }

    fn var_decl_ty(&mut self, module: usize, v: &neuron_syntax::ast::VarDecl) -> Ty {
        if let Some(t) = &v.ty {
            return self.lower_in_module(module, t);
        }
        match &v.init {
            Some(e) => default_ty(&self.infer_in(module, e, None)),
            None => Ty::Unknown,
        }
    }

    fn compute_item_ty(&mut self, module: usize, name: &str) -> Ty {
        let prog = self.prog;
        let Some(sym) = prog.symbol(module, name) else {
            return Ty::Unknown;
        };
        let Some(item) = prog
            .files
            .get(module)
            .and_then(|f| f.ast.items.get(sym.item))
        else {
            return Ty::Unknown;
        };
        match (&sym.kind, &item.kind) {
            (SymKind::Const, ItemKind::Const { ty, value, .. }) => match ty {
                Some(t) => self.lower_in_module(module, t),
                None => self.infer_in(module, value, None),
            },
            (SymKind::Var, ItemKind::Var(v)) => self.var_decl_ty(module, v),
            (SymKind::PlcVar(_), ItemKind::PlcSection { vars, .. }) => match vars.get(sym.sub) {
                Some(v) => self.var_decl_ty(module, v),
                None => Ty::Unknown,
            },
            (SymKind::Signal, ItemKind::Signal { ty, .. })
            | (SymKind::Tag, ItemKind::Tag { ty, .. }) => self.lower_in_module(module, ty),
            (SymKind::Fn, _) => {
                let k = FnKey {
                    module,
                    owner: None,
                    name: name.to_string(),
                };
                self.out
                    .sigs
                    .get(&k)
                    .map_or(Ty::Unknown, |s| Ty::Fn(Box::new(s.clone())))
            }
            (SymKind::Struct | SymKind::Enum | SymKind::Class, _) => {
                let n = match self.out.defs.get(&(module, name.to_string())) {
                    Some(DefInfo::Struct { generics, .. })
                    | Some(DefInfo::Enum { generics, .. })
                    | Some(DefInfo::Class { generics, .. }) => generics.len(),
                    _ => 0,
                };
                Ty::TypeRef(Box::new(Ty::Adt {
                    def: (module, name.to_string()),
                    args: vec![Ty::Unknown; n],
                }))
            }
            (SymKind::Interface, _) => {
                Ty::TypeRef(Box::new(Ty::Interface((module, name.to_string()))))
            }
            (SymKind::Io, ItemKind::Io { kind, .. }) => Ty::Io(io_kind_name(kind)),
            (SymKind::Event, _) => Ty::Event,
            (SymKind::Clock, _) => Ty::Opaque("clock".into()),
            (SymKind::Fsm, _) => Ty::Opaque("fsm".into()),
            (SymKind::Sequence, _) => Ty::Opaque("sequence".into()),
            (SymKind::Task, _) => Ty::Opaque("task".into()),
            (SymKind::Rung, _) => Ty::Opaque("rung".into()),
            (SymKind::Circuit, _) => Ty::Opaque("circuit".into()),
            (SymKind::Geometry(k), _) => Ty::Opaque(
                match k {
                    neuron_syntax::ast::GeometryKind::Frame => "frame",
                    neuron_syntax::ast::GeometryKind::Tool => "tool",
                    neuron_syntax::ast::GeometryKind::Point => "point",
                }
                .into(),
            ),
            (SymKind::Device, _) => Ty::Opaque("device".into()),
            (SymKind::Zone, _) => Ty::Opaque("zone".into()),
            _ => Ty::Unknown,
        }
    }

    // ---- main entry -----------------------------------------------------

    /// Normalizes an operand: readable `io` bindings behave as their value.
    pub(crate) fn rv(&self, t: Ty) -> Ty {
        match t {
            Ty::Io(k) if k.starts_with("gpio") => Ty::Bool,
            other => other,
        }
    }

    /// Checks that `e` is a condition.
    pub(crate) fn cond(&mut self, e: &Expr) {
        let t = self.expr(e, Some(&Ty::Bool));
        let t = self.rv(t);
        match t {
            Ty::Bool
            | Ty::Bits { width: 1, .. }
            | Ty::Unknown
            | Ty::Never
            | Ty::Opaque(_)
            | Ty::Event => {}
            other => self.err_help(
                "NE0208",
                format!("a condition must be `bool`, found `{}`", other.display()),
                e.span,
                "compare the value, for example `x != 0`",
            ),
        }
    }

    pub(crate) fn expr(&mut self, e: &Expr, exp: Option<&Ty>) -> Ty {
        match &e.kind {
            ExprKind::Lit(l) => self.lit(l),
            ExprKind::Quantity(inner, unit) => self.quantity(inner, unit, e),
            ExprKind::Name(id) => self.name(id, e),
            ExprKind::SelfValue => self.cur.self_ty.clone().unwrap_or(Ty::Unknown),
            ExprKind::Result => self.cur.result_ty.clone().unwrap_or(Ty::Unknown),
            ExprKind::Binary(op, a, b) => self.binary(*op, a, b, e),
            ExprKind::Compare(first, rest) => self.compare(first, rest),
            ExprKind::Unary(op, x) => self.unary(*op, x, e),
            ExprKind::Range { lo, hi, .. } => self.range(lo, hi, e),
            ExprKind::Map(a, b) => {
                self.expr(a, None);
                self.expr(b, None);
                Ty::Unknown
            }
            ExprKind::IfElse(c, a, b) => {
                self.cond(c);
                let ta = self.expr(a, exp);
                let tb = self.expr(b, exp);
                if self.compat(&ta, &tb) || self.compat(&tb, &ta) {
                    if matches!(&ta, Ty::Num(k, _) if k.is_lit()) {
                        tb
                    } else {
                        ta
                    }
                } else {
                    self.err(
                        "NE0208",
                        format!(
                            "the branches of `if` have different types: `{}` and `{}`",
                            ta.display(),
                            tb.display()
                        ),
                        e.span,
                    );
                    ta
                }
            }
            ExprKind::Pipe(lhs, rhs) => {
                let lt = self.expr(lhs, None);
                match &rhs.kind {
                    ExprKind::Call(callee, args) => self.call(callee, args, Some(lt), rhs, exp),
                    _ => self.call(rhs, &[], Some(lt), rhs, exp),
                }
            }
            ExprKind::Call(callee, args) => self.call(callee, args, None, e, exp),
            ExprKind::Index(base, idx, end) => self.index(base, idx, end.as_deref(), e),
            ExprKind::Field(base, id) => self.field(base, id, e),
            ExprKind::Try(x) => self.try_expr(x, e),
            ExprKind::Cast(x, t) => {
                let target = self.lower(t);
                let from = self.expr(x, None);
                self.cast(&from, &target, x, e.span)
            }
            ExprKind::Tuple(xs) => {
                let hints: Vec<Option<Ty>> = match exp {
                    Some(Ty::Tuple(ts)) if ts.len() == xs.len() => {
                        ts.iter().cloned().map(Some).collect()
                    }
                    _ => vec![None; xs.len()],
                };
                Ty::Tuple(
                    xs.iter()
                        .zip(&hints)
                        .map(|(x, h)| self.expr(x, h.as_ref()))
                        .collect(),
                )
            }
            ExprKind::Array(xs) => {
                let hint = match exp {
                    Some(Ty::Array(t, _)) => Some((**t).clone()),
                    _ => None,
                };
                let mut elem = hint.clone().unwrap_or(Ty::Unknown);
                for (i, x) in xs.iter().enumerate() {
                    let t = self.expr(x, hint.as_ref());
                    if i == 0 && hint.is_none() {
                        elem = t;
                    } else {
                        if matches!(&elem, Ty::Num(k, _) if k.is_lit())
                            && !matches!(&t, Ty::Num(k, _) if k.is_lit())
                        {
                            elem = t.clone();
                        }
                        let want = elem.clone();
                        self.coerce(&want, &t, x);
                    }
                }
                Ty::Array(Box::new(elem), Some(xs.len() as u64))
            }
            ExprKind::ArrayRepeat(v, n) => {
                let hint = match exp {
                    Some(Ty::Array(t, _)) => Some((**t).clone()),
                    _ => None,
                };
                let t = self.expr(v, hint.as_ref());
                let nt = self.expr(n, None);
                self.expect_int(&nt, n.span);
                let len = self
                    .consts
                    .int(self.cur.module, n)
                    .and_then(|x| u64::try_from(x).ok());
                Ty::Array(Box::new(t), len)
            }
            ExprKind::Lambda(params, body) => {
                for p in params {
                    self.set_local(p, Ty::Unknown);
                }
                let t = self.expr(body, None);
                Ty::Fn(Box::new(FnTy {
                    params: Vec::new(),
                    ret: t,
                    generics: Vec::new(),
                    self_mode: None,
                }))
            }
            ExprKind::Old(x) => self.expr(x, exp),
            ExprKind::Run(x) => {
                self.expr(x, None);
                Ty::Unknown
            }
            ExprKind::Error => Ty::Unknown,
        }
    }

    fn lit(&mut self, l: &Lit) -> Ty {
        match l {
            Lit::Int(_) => Ty::num(NumKind::LitInt),
            Lit::Real(_) => Ty::num(NumKind::LitReal),
            Lit::Str(_) | Lit::RawStr(_) => Ty::Str(None),
            Lit::ByteStr(_) => Ty::Array(
                Box::new(Ty::num(NumKind::Int {
                    signed: false,
                    bits: 8,
                })),
                None,
            ),
            Lit::Char(_) => Ty::num(NumKind::Int {
                signed: false,
                bits: 8,
            }),
            Lit::Bool(_) => Ty::Bool,
            Lit::None => Ty::NoneLit,
            Lit::Iec(_) => Ty::Unknown,
            Lit::Bits(t) => parse_bits_lit(t),
        }
    }

    fn name(&mut self, id: &Ident, e: &Expr) -> Ty {
        match self.res_of(id).cloned() {
            Some(Res::Local(i)) => self.out.local_ty.get(i).cloned().unwrap_or(Ty::Unknown),
            Some(Res::Item { module, name }) => {
                if self.symbol_kind(module, &name) == Some(SymKind::Fn) {
                    self.out.targets.insert(
                        key(self.cur.module, e.span),
                        CallTarget::Fn(FnKey {
                            module,
                            owner: None,
                            name: name.clone(),
                        }),
                    );
                }
                self.item_ty(module, &name)
            }
            Some(Res::Module(m)) => Ty::Module(m),
            Some(Res::Foreign(p)) => Ty::Opaque(format!(
                "foreign:{}",
                p.rsplit('.').next().unwrap_or_default()
            )),
            Some(Res::Prelude(n)) => {
                if prelude::is_value_name(&n) {
                    Ty::Opaque(format!("prelude:{n}"))
                } else {
                    Ty::Unknown
                }
            }
            Some(Res::State { .. }) => Ty::Bool,
            None => Ty::Unknown,
        }
    }

    fn quantity(&mut self, inner: &Expr, unit: &UnitExpr, whole: &Expr) -> Ty {
        let unit_val = self.prog.units.resolve(unit).ok();
        match &inner.kind {
            ExprKind::Call(callee, _) => {
                let name = match &callee.kind {
                    ExprKind::Name(id) | ExprKind::Field(_, id) => id.name.as_str(),
                    _ => "",
                };
                let t = self.expr(inner, None);
                if prelude::VECTOR_CONSTRUCTORS.contains(&name) {
                    t
                } else {
                    self.err_help(
                        "NE0202",
                        "a unit cannot follow this call",
                        whole.span,
                        "write the unit on each argument, for example `box(x: 100..220 mm)`",
                    );
                    t
                }
            }
            ExprKind::Tuple(xs) => {
                let tys = xs
                    .iter()
                    .map(|x| {
                        let t = self.expr(x, None);
                        self.attach_unit(t, unit_val.as_ref(), x)
                    })
                    .collect();
                Ty::Tuple(tys)
            }
            _ => {
                let t = self.expr(inner, None);
                self.attach_unit(t, unit_val.as_ref(), inner)
            }
        }
    }

    fn attach_unit(&mut self, t: Ty, unit: Option<&Unit>, e: &Expr) -> Ty {
        let Some(u) = unit else {
            return Ty::Unknown;
        };
        match t {
            Ty::Num(k, old) if old.is_dimensionless() => Ty::Num(k, u.clone()),
            Ty::Unknown => Ty::Unknown,
            other => {
                self.err(
                    "NE0202",
                    format!("a unit cannot follow a value of type `{}`", other.display()),
                    e.span,
                );
                other
            }
        }
    }

    // ---- operators ------------------------------------------------------

    fn range(&mut self, lo: &Expr, hi: &Expr, whole: &Expr) -> Ty {
        let a = self.expr(lo, None);
        let b = self.expr(hi, None);
        match (&a, &b) {
            (Ty::Num(ka, ua), Ty::Num(kb, ub)) => {
                if ua.dim != ub.dim && !is_zero_lit(lo) && !is_zero_lit(hi) {
                    self.dim_error("form a range from", ua, ub, whole.span);
                } else if unify_kinds(ka, kb).is_none() {
                    self.mismatch_kinds(ka, kb, whole.span);
                }
                let (k, u) = match (ka.is_lit(), kb.is_lit()) {
                    (true, false) => (kb.clone(), ub.clone()),
                    _ => (ka.clone(), ua.clone()),
                };
                let u = if u.is_dimensionless() && !ub.is_dimensionless() {
                    ub.clone()
                } else {
                    u
                };
                Ty::Range(Box::new(Ty::Num(k, u)))
            }
            (Ty::Unknown, other) | (other, Ty::Unknown) => Ty::Range(Box::new(other.clone())),
            (x, _) => Ty::Range(Box::new(x.clone())),
        }
    }

    fn dim_error(&mut self, what: &str, a: &Unit, b: &Unit, span: Span) {
        self.err_help(
            "NE0201",
            format!(
                "cannot {what} a {} ({}) and a {} ({})",
                a.dim.describe(),
                a.display(),
                b.dim.describe(),
                b.display()
            ),
            span,
            "use values with the same dimension, or divide/multiply to change it",
        );
    }

    fn mismatch_kinds(&mut self, a: &NumKind, b: &NumKind, span: Span) {
        self.err_help(
            "NE0208",
            format!("type mismatch: `{}` and `{}`", a.name(), b.name()),
            span,
            "convert one side explicitly with `as`",
        );
    }

    fn unary(&mut self, op: UnOp, x: &Expr, whole: &Expr) -> Ty {
        let t = self.expr(x, None);
        let t = self.rv(t);
        match op {
            UnOp::Not => match t {
                Ty::Bool | Ty::Unknown | Ty::Never | Ty::Opaque(_) => Ty::Bool,
                Ty::Bits { .. } => t,
                other => {
                    self.err(
                        "NE0208",
                        format!("`not` needs a `bool`, found `{}`", other.display()),
                        whole.span,
                    );
                    Ty::Bool
                }
            },
            UnOp::Neg => match &t {
                Ty::Num(NumKind::Int { signed: false, .. }, _) => {
                    self.err_help(
                        "NE0208",
                        format!("cannot negate the unsigned type `{}`", t.display()),
                        whole.span,
                        "use a signed type",
                    );
                    t
                }
                Ty::Num(..) | Ty::Unknown | Ty::Bits { signed: true, .. } => t,
                other => {
                    self.err(
                        "NE0208",
                        format!("cannot negate `{}`", other.display()),
                        whole.span,
                    );
                    t
                }
            },
            UnOp::BitNot => match &t {
                Ty::Num(k, _) if k.is_integer() => t,
                Ty::Bits { .. } | Ty::Unknown => t,
                other => {
                    self.err(
                        "NE0208",
                        format!(
                            "`~` needs an integer or bit vector, found `{}`",
                            other.display()
                        ),
                        whole.span,
                    );
                    t
                }
            },
        }
    }

    fn compare(&mut self, first: &Expr, rest: &[(BinOp, Expr)]) -> Ty {
        let mut prev_e = first;
        let mut prev_t = self.expr(first, None);
        for (op, next) in rest {
            let nt = self.expr(next, None);
            self.cmp_pair(*op, prev_e, &prev_t, next, &nt);
            prev_e = next;
            prev_t = nt;
        }
        Ty::Bool
    }

    fn cmp_pair(&mut self, op: BinOp, le: &Expr, lt: &Ty, re: &Expr, rt: &Ty) {
        let span = le.span.merge(&re.span);
        let (lt, rt) = (self.rv(lt.clone()), self.rv(rt.clone()));
        if op == BinOp::In {
            match &rt {
                Ty::Range(inner) => {
                    let inner = (**inner).clone();
                    self.cmp_pair(BinOp::Eq, le, &lt, re, &inner);
                }
                Ty::Array(inner, _) | Ty::Slice(inner) => {
                    let inner = (**inner).clone();
                    self.cmp_pair(BinOp::Eq, le, &lt, re, &inner);
                }
                Ty::Unknown | Ty::Opaque(_) => {}
                other => self.err(
                    "NE0208",
                    format!("`in` needs a range or array, found `{}`", other.display()),
                    span,
                ),
            }
            return;
        }
        match (&lt, &rt) {
            (Ty::Num(lk, lu), Ty::Num(rk, ru)) => {
                if lu.dim != ru.dim && !is_zero_lit(le) && !is_zero_lit(re) {
                    self.dim_error("compare", lu, ru, span);
                } else if unify_kinds(lk, rk).is_none() {
                    self.mismatch_kinds(lk, rk, span);
                }
            }
            _ => {
                if !(self.compat(&lt, &rt) || self.compat(&rt, &lt)) {
                    self.err(
                        "NE0208",
                        format!("cannot compare `{}` with `{}`", lt.display(), rt.display()),
                        span,
                    );
                } else if !matches!(op, BinOp::Eq | BinOp::Ne) && !self.orderable(&lt) {
                    self.err(
                        "NE0208",
                        format!("`{}` values cannot be ordered", lt.display()),
                        span,
                    );
                }
            }
        }
    }

    /// Can values of this type be ordered with `<`? Generic parameters need an `Ord` or
    /// `PartialOrd` bound.
    fn orderable(&self, t: &Ty) -> bool {
        match t {
            Ty::Num(..)
            | Ty::Bits { .. }
            | Ty::Instant
            | Ty::Unknown
            | Ty::Str(_)
            | Ty::Opaque(_)
            | Ty::Never => true,
            Ty::Param(n) => self.cur.generics.iter().any(|(g, bounds)| {
                g == n && bounds.iter().any(|b| b == "Ord" || b == "PartialOrd")
            }),
            _ => false,
        }
    }

    fn binary(&mut self, op: BinOp, a: &Expr, b: &Expr, whole: &Expr) -> Ty {
        let hint = match op {
            BinOp::Or | BinOp::And => Some(Ty::Bool),
            _ => None,
        };
        let lt = self.expr(a, hint.as_ref());
        let rt = self.expr(b, hint.as_ref());
        let (lt, rt) = (self.rv(lt), self.rv(rt));
        self.binary_tys(op, a, b, &lt, &rt, whole.span)
    }

    /// Result type of `a op b` given the operand types.
    pub(crate) fn binary_tys(
        &mut self,
        op: BinOp,
        a: &Expr,
        b: &Expr,
        lt: &Ty,
        rt: &Ty,
        span: Span,
    ) -> Ty {
        let (lt, rt) = (lt.clone(), rt.clone());
        match op {
            BinOp::Or | BinOp::And => {
                for (t, e) in [(&lt, a), (&rt, b)] {
                    if !matches!(
                        t,
                        Ty::Bool
                            | Ty::Bits { width: 1, .. }
                            | Ty::Unknown
                            | Ty::Never
                            | Ty::Opaque(_)
                            | Ty::Event
                    ) {
                        self.err(
                            "NE0208",
                            format!(
                                "`{}` needs `bool` operands, found `{}`",
                                op.as_str(),
                                t.display()
                            ),
                            e.span,
                        );
                    }
                }
                Ty::Bool
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In => {
                self.cmp_pair(op, a, &lt, b, &rt);
                Ty::Bool
            }
            BinOp::Coalesce => match (&lt, &rt) {
                (Ty::Option(inner), _) => {
                    let inner = (**inner).clone();
                    self.coerce(&inner, &rt, b);
                    inner
                }
                (Ty::Unknown, _) => rt,
                _ => {
                    self.err(
                        "NE0208",
                        format!(
                            "`??` needs an optional value on the left, found `{}`",
                            lt.display()
                        ),
                        a.span,
                    );
                    rt
                }
            },
            BinOp::Concat => match (&lt, &rt) {
                (Ty::Bits { width: w1, .. }, Ty::Bits { width: w2, .. }) => Ty::Bits {
                    signed: false,
                    width: w1 + w2,
                },
                (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
                _ => {
                    self.err(
                        "NE0208",
                        format!(
                            "`++` joins bit vectors, found `{}` and `{}`",
                            lt.display(),
                            rt.display()
                        ),
                        span,
                    );
                    Ty::Unknown
                }
            },
            BinOp::Shl | BinOp::Shr => {
                self.expect_int_like(&rt, b.span);
                match &lt {
                    Ty::Num(k, _) if k.is_integer() => lt,
                    Ty::Bits { .. } | Ty::Unknown => lt,
                    other => {
                        self.err(
                            "NE0208",
                            format!("cannot shift `{}`", other.display()),
                            a.span,
                        );
                        lt
                    }
                }
            }
            BinOp::BitOr | BinOp::BitAnd | BinOp::BitXor => match (&lt, &rt) {
                (Ty::Bool, Ty::Bool) => Ty::Bool,
                (Ty::Bits { width: w1, .. }, Ty::Bits { width: w2, .. }) => {
                    if w1 != w2 {
                        self.err(
                            "NE0208",
                            format!("bit vectors of different widths: {w1} and {w2}"),
                            span,
                        );
                    }
                    lt
                }
                (Ty::Bits { .. }, Ty::Num(k, _)) if k.is_lit() => lt,
                (Ty::Num(k1, u1), Ty::Num(k2, u2)) if k1.is_integer() && k2.is_integer() => {
                    match unify_kinds(k1, k2) {
                        Some(k) => Ty::Num(k, if k1.is_lit() { u2.clone() } else { u1.clone() }),
                        None => {
                            self.mismatch_kinds(k1, k2, span);
                            lt
                        }
                    }
                }
                (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
                _ => {
                    self.err(
                        "NE0208",
                        format!(
                            "`{}` needs integers, bit vectors or booleans, found `{}` and `{}`",
                            op.as_str(),
                            lt.display(),
                            rt.display()
                        ),
                        span,
                    );
                    lt
                }
            },
            _ => self.arith(op, a, b, &lt, &rt, span),
        }
    }

    fn expect_int_like(&mut self, t: &Ty, span: Span) {
        match t {
            Ty::Num(k, _) if k.is_integer() => {}
            Ty::Bits { .. } | Ty::Unknown => {}
            other => self.err(
                "NE0208",
                format!("expected an integer, found `{}`", other.display()),
                span,
            ),
        }
    }

    fn arith(&mut self, op: BinOp, a: &Expr, b: &Expr, lt: &Ty, rt: &Ty, span: Span) -> Ty {
        let additive = matches!(
            op,
            BinOp::Add
                | BinOp::Sub
                | BinOp::AddWrap
                | BinOp::SubWrap
                | BinOp::AddSat
                | BinOp::SubSat
        );
        let sub = matches!(op, BinOp::Sub | BinOp::SubWrap | BinOp::SubSat);
        match (lt, rt) {
            (Ty::Unknown, _) | (_, Ty::Unknown) | (Ty::Opaque(_), _) | (_, Ty::Opaque(_)) => {
                Ty::Unknown
            }
            (Ty::Instant, Ty::Instant) if sub => {
                lookup_unit("s").map_or(Ty::Unknown, |u| Ty::Num(NumKind::Duration, u))
            }
            (Ty::Instant, Ty::Num(k, u))
                if additive
                    && matches!(
                        k,
                        NumKind::Duration
                            | NumKind::LitInt
                            | NumKind::LitReal
                            | NumKind::Float(_)
                            | NumKind::Int { .. }
                    )
                    && u.dim == Dim::base(0, 1) =>
            {
                Ty::Instant
            }
            (Ty::Bits { width: w1, .. }, Ty::Bits { width: w2, .. }) => {
                if w1 != w2 {
                    self.err(
                        "NE0208",
                        format!("bit vectors of different widths: {w1} and {w2}"),
                        span,
                    );
                }
                lt.clone()
            }
            (Ty::Bits { .. }, Ty::Num(k, _)) if k.is_lit() => lt.clone(),
            (Ty::Num(k, _), Ty::Bits { .. }) if k.is_lit() => rt.clone(),
            (Ty::Num(lk, lu), Ty::Num(rk, ru)) => {
                self.num_arith(op, additive, sub, (lk, lu), (rk, ru), a, b, span)
            }
            _ => {
                self.err(
                    "NE0208",
                    format!(
                        "`{}` is not defined for `{}` and `{}`",
                        op.as_str(),
                        lt.display(),
                        rt.display()
                    ),
                    span,
                );
                Ty::Unknown
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn num_arith(
        &mut self,
        op: BinOp,
        additive: bool,
        sub: bool,
        l: (&NumKind, &Unit),
        r: (&NumKind, &Unit),
        a: &Expr,
        b: &Expr,
        span: Span,
    ) -> Ty {
        let (lk, lu) = l;
        let (rk, ru) = r;
        let Some(kind) = unify_kinds(lk, rk) else {
            self.mismatch_kinds(lk, rk, span);
            return Ty::Num(lk.clone(), lu.clone());
        };
        let wrapping = matches!(
            op,
            BinOp::AddWrap
                | BinOp::SubWrap
                | BinOp::AddSat
                | BinOp::SubSat
                | BinOp::MulWrap
                | BinOp::MulSat
        );
        if wrapping && !matches!(kind, NumKind::Int { .. } | NumKind::Index | NumKind::LitInt) {
            self.err(
                "NE0208",
                format!(
                    "`{}` works on integers, found `{}`",
                    op.as_str(),
                    kind.name()
                ),
                span,
            );
        }
        if additive {
            let unit = if lu.dim != ru.dim {
                if is_zero_lit(a) {
                    ru.clone()
                } else if is_zero_lit(b) {
                    lu.clone()
                } else {
                    self.dim_error(if sub { "subtract" } else { "add" }, lu, ru, span);
                    lu.clone()
                }
            } else {
                match (lu.is_affine(), ru.is_affine()) {
                    (true, true) if sub => diff_unit(lu),
                    (true, true) => {
                        self.err_help(
                            "NE0203",
                            format!(
                                "cannot add two absolute temperatures ({} + {})",
                                lu.display(),
                                ru.display()
                            ),
                            span,
                            "add a temperature difference (K) instead",
                        );
                        lu.clone()
                    }
                    (true, false) => lu.clone(),
                    (false, true) if sub => {
                        self.err(
                            "NE0203",
                            "cannot subtract an absolute temperature from a difference",
                            span,
                        );
                        ru.clone()
                    }
                    (false, true) => ru.clone(),
                    (false, false) => {
                        if lk.is_lit() && !rk.is_lit() {
                            ru.clone()
                        } else {
                            lu.clone()
                        }
                    }
                }
            };
            return Ty::Num(kind, unit);
        }
        match op {
            BinOp::Mul | BinOp::MulWrap | BinOp::MulSat | BinOp::Div => {
                let unit = if op == BinOp::Div {
                    lu.div(ru)
                } else {
                    lu.mul(ru)
                };
                let Some(unit) = unit else {
                    return Ty::Unknown;
                };
                let kind = if kind == NumKind::Duration && unit.is_dimensionless() {
                    NumKind::LitInt
                } else {
                    kind
                };
                Ty::Num(kind, unit)
            }
            BinOp::Rem => {
                if lu.dim != ru.dim && !ru.is_dimensionless() {
                    self.dim_error("take the remainder of", lu, ru, span);
                }
                Ty::Num(kind, lu.clone())
            }
            BinOp::Pow => {
                if !ru.is_dimensionless() {
                    self.dim_error("raise to a power with", lu, ru, span);
                    return Ty::Num(kind, lu.clone());
                }
                match self
                    .consts
                    .int(self.cur.module, b)
                    .and_then(|n| i32::try_from(n).ok())
                {
                    Some(n) => {
                        let scale = lu.scale.pow(n);
                        let dim = lu.dim.pow(n);
                        match (scale, dim) {
                            (Some(scale), Some(dim)) => Ty::Num(
                                kind,
                                Unit {
                                    dim,
                                    scale,
                                    offset: Rat::ZERO,
                                    label: String::new(),
                                },
                            ),
                            _ => Ty::Unknown,
                        }
                    }
                    None => {
                        if !lu.is_dimensionless() {
                            self.err_help(
                                "NE0201",
                                "the exponent of a quantity must be a constant integer",
                                span,
                                "use a literal exponent such as `x ** 2`",
                            );
                        }
                        Ty::Num(kind, lu.clone())
                    }
                }
            }
            _ => Ty::Num(kind, lu.clone()),
        }
    }

    // ---- indexing, fields, try, casts -------------------------------------

    fn index(&mut self, base: &Expr, idx: &Expr, end: Option<&Expr>, whole: &Expr) -> Ty {
        let bt = self.expr(base, None);
        let it = self.expr(idx, None);
        let et = end.map(|e| self.expr(e, None));
        self.expect_int_like(&it, idx.span);
        if let Some(t) = &et {
            self.expect_int_like(t, end.map_or(whole.span, |e| e.span));
        }
        match &bt {
            Ty::Array(elem, len) => {
                if let (Some(n), Some(i)) = (len, self.consts.int(self.cur.module, idx)) {
                    if i < 0 || i >= i128::from(*n) {
                        self.err(
                            "NE0216",
                            format!("index {i} is out of bounds for `{}`", bt.display()),
                            idx.span,
                        );
                    }
                }
                (**elem).clone()
            }
            Ty::Slice(elem) => (**elem).clone(),
            Ty::Bits { signed, width } => match end {
                Some(e) => {
                    let hi = self.consts.int(self.cur.module, idx);
                    let lo = self.consts.int(self.cur.module, e);
                    match (hi, lo) {
                        (Some(h), Some(l)) if h >= l && h < i128::from(*width) && l >= 0 => {
                            Ty::Bits {
                                signed: *signed,
                                width: u32::try_from(h - l + 1).unwrap_or(1),
                            }
                        }
                        (Some(_), Some(_)) => {
                            self.err(
                                "NE0216",
                                format!("bit slice is out of range for `{}`", bt.display()),
                                whole.span,
                            );
                            Ty::Unknown
                        }
                        _ => Ty::Unknown,
                    }
                }
                None => {
                    if let Some(i) = self.consts.int(self.cur.module, idx) {
                        if i < 0 || i >= i128::from(*width) {
                            self.err(
                                "NE0216",
                                format!("bit {i} is out of range for `{}`", bt.display()),
                                idx.span,
                            );
                        }
                    }
                    Ty::Bits {
                        signed: false,
                        width: 1,
                    }
                }
            },
            Ty::Pool(elem, _) => (**elem).clone(),
            Ty::Unknown | Ty::Opaque(_) | Ty::Never => Ty::Unknown,
            other => {
                self.err(
                    "NE0208",
                    format!("cannot index a value of type `{}`", other.display()),
                    base.span,
                );
                Ty::Unknown
            }
        }
    }

    fn try_expr(&mut self, x: &Expr, whole: &Expr) -> Ty {
        let t = self.expr(x, None);
        let ret = self.cur.ret.clone();
        match &t {
            Ty::Result(ok, _) => {
                if !matches!(ret, Some(Ty::Result(..) | Ty::Unknown)) {
                    self.err_help(
                        "NE0208",
                        "`?` needs the function to return a `Result`",
                        whole.span,
                        "declare `-> Result<T, E>` or handle the error with `match`",
                    );
                }
                (**ok).clone()
            }
            Ty::Option(inner) => {
                if !matches!(ret, Some(Ty::Option(_) | Ty::Unknown)) {
                    self.err_help(
                        "NE0208",
                        "`?` on an optional value needs the function to return an optional",
                        whole.span,
                        "declare `-> T?` or use `??` to give a default",
                    );
                }
                (**inner).clone()
            }
            Ty::Unknown | Ty::Opaque(_) => Ty::Unknown,
            other => {
                self.err(
                    "NE0208",
                    format!(
                        "`?` needs a `Result` or an optional value, found `{}`",
                        other.display()
                    ),
                    x.span,
                );
                Ty::Unknown
            }
        }
    }

    pub(crate) fn field(&mut self, base: &Expr, id: &Ident, whole: &Expr) -> Ty {
        let bt = self.expr(base, None);
        let name = id.name.as_str();
        let cur = self.cur.module;
        match bt {
            Ty::Module(m) => {
                if self.symbol_kind(m, name) == Some(SymKind::Fn) {
                    self.out.targets.insert(
                        key(cur, whole.span),
                        CallTarget::Fn(FnKey {
                            module: m,
                            owner: None,
                            name: name.to_string(),
                        }),
                    );
                }
                self.item_ty(m, name)
            }
            Ty::TypeRef(inner) => match *inner {
                Ty::Adt { def, args } => self.variant(&def, &args, name, id, whole),
                _ => Ty::Unknown,
            },
            Ty::Adt { def, args } => self.member(&def, &args, name, id, whole),
            Ty::Io(kind) => {
                if let Ty::Fn(_) = io_member(&kind, name) {
                    if let ExprKind::Name(b) = &base.kind {
                        self.out.targets.insert(
                            key(cur, whole.span),
                            CallTarget::Io {
                                binding: b.name.clone(),
                                method: name.to_string(),
                            },
                        );
                    }
                }
                io_member(&kind, name)
            }
            Ty::Opaque(n) => block_member(&n, name),
            Ty::Bool => match name {
                "rising" | "falling" | "changed" => Ty::Bool,
                _ => Ty::Unknown,
            },
            Ty::Tuple(ts) => match name.parse::<usize>().ok().and_then(|i| ts.get(i)) {
                Some(t) => t.clone(),
                None => {
                    self.err(
                        "NE0212",
                        format!(
                            "tuple `{}` has no element `{name}`",
                            Ty::Tuple(ts.clone()).display()
                        ),
                        id.span,
                    );
                    Ty::Unknown
                }
            },
            Ty::Range(t) => match name {
                "start" | "end" => *t,
                "contains" => Ty::Unknown,
                _ => {
                    self.err("NE0212", format!("`range` has no member `{name}`"), id.span);
                    Ty::Unknown
                }
            },
            _ => Ty::Unknown,
        }
    }

    fn variant(&mut self, def: &DefRef, args: &[Ty], name: &str, id: &Ident, whole: &Expr) -> Ty {
        match self.out.defs.get(def).cloned() {
            Some(DefInfo::Enum { generics, variants }) => {
                let Some((_, payload)) = variants.iter().find(|(v, _)| v == name) else {
                    let list = variants
                        .iter()
                        .map(|(v, _)| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.err_help(
                        "NE0212",
                        format!("enum `{}` has no variant `{name}`", def.1),
                        id.span,
                        format!("variants: {list}"),
                    );
                    return Ty::Unknown;
                };
                let ret = Ty::Adt {
                    def: def.clone(),
                    args: if args.len() == generics.len() {
                        args.to_vec()
                    } else {
                        vec![Ty::Unknown; generics.len()]
                    },
                };
                self.out.targets.insert(
                    key(self.cur.module, whole.span),
                    CallTarget::Ctor(def.clone()),
                );
                if payload.is_empty() {
                    ret
                } else {
                    Ty::Fn(Box::new(FnTy {
                        params: payload
                            .iter()
                            .enumerate()
                            .map(|(i, t)| ParamTy {
                                name: i.to_string(),
                                ty: t.clone(),
                                mutable: false,
                                has_default: false,
                            })
                            .collect(),
                        ret,
                        generics: generics.iter().map(|g| (g.clone(), Vec::new())).collect(),
                        self_mode: None,
                    }))
                }
            }
            _ => {
                self.err(
                    "NE0212",
                    format!("`{}` has no associated item `{name}`", def.1),
                    id.span,
                );
                Ty::Unknown
            }
        }
    }

    fn member(&mut self, def: &DefRef, args: &[Ty], name: &str, id: &Ident, whole: &Expr) -> Ty {
        let info = self.out.defs.get(def).cloned();
        let (generics, fields) = match &info {
            Some(DefInfo::Struct { generics, fields }) => (generics.clone(), fields.clone()),
            Some(DefInfo::Class {
                generics, fields, ..
            }) => (generics.clone(), fields.clone()),
            _ => return Ty::Unknown,
        };
        let subst: Vec<(String, Ty)> = generics
            .iter()
            .cloned()
            .zip(args.iter().cloned().chain(std::iter::repeat(Ty::Unknown)))
            .collect();
        if let Some(f) = fields.iter().find(|f| f.name == name) {
            return f.ty.subst(&subst);
        }
        let fk = FnKey {
            module: def.0,
            owner: Some(def.1.clone()),
            name: name.to_string(),
        };
        if let Some(sig) = self.out.sigs.get(&fk).cloned() {
            let ft = Ty::Fn(Box::new(sig.clone())).subst(&subst);
            self.out.targets.insert(
                key(self.cur.module, whole.span),
                CallTarget::Method {
                    key: fk,
                    mut_self: sig.self_mode == Some(true),
                },
            );
            return ft;
        }
        let known: Vec<String> = fields
            .iter()
            .map(|f| f.name.clone())
            .chain(
                self.out
                    .sigs
                    .keys()
                    .filter(|k| k.module == def.0 && k.owner.as_deref() == Some(def.1.as_str()))
                    .map(|k| k.name.clone()),
            )
            .collect();
        self.err_help(
            "NE0212",
            format!("`{}` has no field or method `{name}`", def.1),
            id.span,
            format!("available: {}", known.join(", ")),
        );
        Ty::Unknown
    }

    fn cast(&mut self, from: &Ty, to: &Ty, x: &Expr, span: Span) -> Ty {
        match (from, to) {
            (Ty::Unknown | Ty::Opaque(_), _) => to.clone(),
            (Ty::Num(fk, fu), Ty::Num(tk, tu)) => {
                if tu.is_dimensionless() && !fu.is_dimensionless() {
                    self.err_help(
                        "NE0201",
                        format!(
                            "cannot drop the unit of a {} ({}) with `as`",
                            fu.dim.describe(),
                            fu.display()
                        ),
                        span,
                        "divide by a unit first, for example `(x / 1 ms) as u32`",
                    );
                } else if !tu.is_dimensionless() && !fu.is_dimensionless() && fu.dim != tu.dim {
                    self.dim_error("convert between", fu, tu, span);
                }
                self.cast_kind(fk, tk, x, span);
                to.clone()
            }
            (Ty::Num(..), Ty::Bits { .. })
            | (Ty::Bits { .. }, Ty::Num(..))
            | (Ty::Bits { .. }, Ty::Bits { .. }) => to.clone(),
            (a, b) => {
                if !(self.compat(b, a) || self.compat(a, b)) {
                    self.err(
                        "NE0208",
                        format!(
                            "cannot convert `{}` to `{}` with `as`",
                            a.display(),
                            b.display()
                        ),
                        span,
                    );
                }
                to.clone()
            }
        }
    }

    fn cast_kind(&mut self, from: &NumKind, to: &NumKind, x: &Expr, span: Span) {
        use NumKind::*;
        let lossless = match (from, to) {
            (LitInt | LitReal, _) => {
                if let (Some(v), Some((lo, hi))) =
                    (self.consts.eval(self.cur.module, x), to.int_range())
                {
                    if v.dim.is_none()
                        && v.value.is_integer()
                        && (v.value.numer() < lo || v.value.numer() > hi)
                    {
                        self.err(
                            "NE0205",
                            format!("literal {} does not fit `{}`", v.value, to.name()),
                            span,
                        );
                    }
                }
                !(matches!(from, LitReal) && matches!(to, Int { .. } | Index))
            }
            (Index, _) => true,
            (
                Int {
                    signed: s1,
                    bits: b1,
                },
                Int {
                    signed: s2,
                    bits: b2,
                },
            ) => {
                if s1 == s2 {
                    b2 >= b1
                } else {
                    !*s1 && *s2 && b2 > b1
                }
            }
            (Int { signed, bits }, Index) => !*signed && *bits <= 32,
            (Int { bits, .. }, Float(f)) => (*f == 32 && *bits <= 16) || (*f == 64 && *bits <= 32),
            (Float(a), Float(b)) => b >= a,
            (Float(_), Int { .. } | Index) => false,
            (Duration, _) | (_, Duration) => true,
            (Fixed { .. }, _) | (_, Fixed { .. }) => true,
            (_, LitInt | LitReal) => true,
        };
        if !lossless {
            let help = match (from, to) {
                (Float(_), Int { .. } | Index) => "round first: `x.round()`, then convert",
                _ => "use `.wrap()`, `.saturate()` or `.try_into()`",
            };
            self.err_help(
                "NE0204",
                format!(
                    "`as` would lose information converting `{}` to `{}`",
                    from.name(),
                    to.name()
                ),
                span,
                help,
            );
        }
    }

    // ---- coercion ---------------------------------------------------------

    /// Pure compatibility test (no diagnostics).
    pub(crate) fn compat(&self, exp: &Ty, act: &Ty) -> bool {
        match (exp, act) {
            (Ty::Unknown | Ty::Opaque(_), _) | (_, Ty::Unknown | Ty::Never | Ty::Opaque(_)) => true,
            (Ty::Option(_), Ty::NoneLit) => true,
            (Ty::Option(a), Ty::Option(b)) => self.compat(a, b),
            (Ty::Option(a), b) => self.compat(a, b),
            (Ty::Bool, Ty::Bool) | (Ty::Void, Ty::Void) | (Ty::Instant, Ty::Instant) => true,
            (Ty::Bool, Ty::Bits { width: 1, .. }) | (Ty::Bits { width: 1, .. }, Ty::Bool) => true,
            (Ty::Bool, Ty::Io(k)) => k.starts_with("gpio"),
            (Ty::Num(ek, eu), Ty::Num(ak, au)) => eu.dim == au.dim && kind_accepts(ek, ak),
            (Ty::Bits { width: w1, .. }, Ty::Bits { width: w2, .. }) => w1 == w2,
            (Ty::Bits { .. }, Ty::Num(k, u)) => k.is_lit() && u.is_dimensionless(),
            (Ty::Str(_), Ty::Str(_)) => true,
            (Ty::Array(a, n), Ty::Array(b, m)) => {
                self.compat(a, b) && (n.is_none() || m.is_none() || n == m)
            }
            (Ty::Slice(a), Ty::Array(b, _) | Ty::Slice(b)) => self.compat(a, b),
            (Ty::Tuple(a), Ty::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| self.compat(x, y))
            }
            (Ty::Range(a), Ty::Range(b)) => self.compat(a, b),
            (Ty::Result(a, b), Ty::Result(c, d)) => self.compat(a, c) && self.compat(b, d),
            (Ty::Adt { def: d1, args: a1 }, Ty::Adt { def: d2, args: a2 }) => {
                d1 == d2 && a1.iter().zip(a2).all(|(x, y)| self.compat(x, y))
            }
            (Ty::Interface(i), Ty::Adt { def, .. }) => self.implements(def, i),
            (Ty::Interface(a), Ty::Interface(b)) => a == b,
            (Ty::Param(a), Ty::Param(b)) => a == b,
            (Ty::Handle(a), Ty::Handle(b)) => self.compat(a, b),
            (Ty::Channel(a, n), Ty::Channel(b, m)) | (Ty::Pool(a, n), Ty::Pool(b, m)) => {
                self.compat(a, b) && (n.is_none() || m.is_none() || n == m)
            }
            (Ty::Io(a), Ty::Io(b)) => a == b,
            (Ty::Event, _) | (_, Ty::Event) => true,
            (Ty::Fn(_), Ty::Fn(_)) => true,
            (Ty::TypeRef(a), Ty::TypeRef(b)) => a == b,
            (Ty::Module(a), Ty::Module(b)) => a == b,
            _ => false,
        }
    }

    /// Reports an error when a value of type `act` cannot be used where `exp` is expected.
    pub(crate) fn coerce(&mut self, exp: &Ty, act: &Ty, e: &Expr) {
        self.coerce_at(exp, act, e.span, Some(e));
    }

    pub(crate) fn coerce_at(&mut self, exp: &Ty, act: &Ty, span: Span, e: Option<&Expr>) {
        match (exp, act) {
            (Ty::Unknown | Ty::Opaque(_), _) | (_, Ty::Unknown | Ty::Never | Ty::Opaque(_)) => {}
            (Ty::Option(a), Ty::Option(b)) => self.coerce_at(a, b, span, e),
            (Ty::Option(_), Ty::NoneLit) => {}
            (Ty::Option(a), b) => self.coerce_at(a, b, span, e),
            (Ty::Num(ek, eu), Ty::Num(ak, au)) => self.coerce_num((ek, eu), (ak, au), span, e),
            (Ty::Bits { width, .. }, Ty::Num(k, u)) if k.is_lit() && u.is_dimensionless() => {
                if let Some(v) = e.and_then(|x| self.consts.int(self.cur.module, x)) {
                    if v < 0 || (*width < 127 && v >= (1i128 << *width)) {
                        self.err(
                            "NE0205",
                            format!("literal {v} does not fit `{}`", exp.display()),
                            span,
                        );
                    }
                }
            }
            (Ty::Array(a, n), Ty::Array(b, m)) => {
                if let (Some(n), Some(m)) = (n, m) {
                    if n != m {
                        self.err(
                            "NE0208",
                            format!("expected `{}`, found `{}`", exp.display(), act.display()),
                            span,
                        );
                        return;
                    }
                }
                self.coerce_at(a, b, span, None);
            }
            (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
                let elems: Vec<&Expr> = match e.map(|x| &x.kind) {
                    Some(ExprKind::Tuple(xs)) if xs.len() == a.len() => xs.iter().collect(),
                    _ => Vec::new(),
                };
                for (i, (x, y)) in a.iter().zip(b).enumerate() {
                    match elems.get(i) {
                        Some(el) => self.coerce_at(x, y, el.span, Some(el)),
                        None => self.coerce_at(x, y, span, None),
                    }
                }
            }
            (Ty::Result(a, b), Ty::Result(c, d)) => {
                self.coerce_at(a, c, span, None);
                self.coerce_at(b, d, span, None);
            }
            _ => {
                if !self.compat(exp, act) {
                    let code = match (exp, act) {
                        (Ty::Interface(_), Ty::Adt { .. }) => "NE0211",
                        _ => "NE0208",
                    };
                    self.err(
                        code,
                        format!("expected `{}`, found `{}`", exp.display(), act.display()),
                        span,
                    );
                }
            }
        }
    }

    fn coerce_num(
        &mut self,
        exp: (&NumKind, &Unit),
        act: (&NumKind, &Unit),
        span: Span,
        e: Option<&Expr>,
    ) {
        let (ek, eu) = exp;
        let (ak, au) = act;
        if eu.dim != au.dim {
            if e.is_some_and(is_zero_lit) {
                return;
            }
            self.err_help(
                "NE0201",
                format!(
                    "expected a {} ({}), found a {} ({})",
                    eu.dim.describe(),
                    eu.display(),
                    au.dim.describe(),
                    au.display()
                ),
                span,
                "write the value with the right unit",
            );
            return;
        }
        if !kind_accepts(ek, ak) {
            let help = if matches!(ak, NumKind::LitReal) && ek.is_integer() {
                "an integer type cannot hold a real literal; write an integer or change the type"
            } else {
                "convert explicitly with `as`"
            };
            self.err_help(
                "NE0208",
                format!("expected `{}`, found `{}`", ek.name(), ak.name()),
                span,
                help,
            );
            return;
        }
        if !ek.is_integer() || ek.is_lit() {
            return;
        }
        // Integer target: literals are range checked after unit conversion; other
        // values must convert exactly.
        if ak.is_lit() {
            if let Some(q) = e.and_then(|x| self.consts.eval(self.cur.module, x)) {
                let in_unit = convert(q.value, &base_unit(&q), eu);
                if let Some(v) = in_unit {
                    if !v.is_integer() {
                        self.err_help(
                            "NE0204",
                            format!("{v} is not a whole number of {}", eu.display()),
                            span,
                            "use a unit that divides the value exactly",
                        );
                    } else if let Some((lo, hi)) = ek.int_range() {
                        if v.numer() < lo || v.numer() > hi {
                            let plain = matches!(
                                e.map(|x| &x.kind),
                                Some(
                                    ExprKind::Lit(_)
                                        | ExprKind::Quantity(..)
                                        | ExprKind::Unary(UnOp::Neg, _)
                                )
                            );
                            if plain {
                                self.err(
                                    "NE0205",
                                    format!("literal {} does not fit `{}`", v.numer(), ek.name()),
                                    span,
                                );
                            } else {
                                self.err_help(
                                    "NE0207",
                                    format!(
                                        "this constant expression is {} and overflows `{}`",
                                        v.numer(),
                                        ek.name()
                                    ),
                                    span,
                                    "use a wider type, or `+%`/`+|` to wrap or saturate explicitly",
                                );
                            }
                        }
                    }
                }
            }
        } else if eu.scale != au.scale {
            if let Some(r) = au.scale.div(eu.scale) {
                if !r.is_integer() {
                    self.err_help(
                        "NE0204",
                        format!(
                            "converting `{}` to `{}` is not exact for integers",
                            au.display(),
                            eu.display()
                        ),
                        span,
                        "round explicitly or use a finer unit",
                    );
                }
            }
        }
    }

    // ---- calls ------------------------------------------------------------

    pub(crate) fn call(
        &mut self,
        callee: &Expr,
        args: &[Arg],
        first: Option<Ty>,
        whole: &Expr,
        exp: Option<&Ty>,
    ) -> Ty {
        let ct = self.expr(callee, None);
        let cur = self.cur.module;
        let target = self.out.targets.remove(&key(cur, callee.span));
        match ct {
            Ty::Fn(f) => {
                if let Some(t) = &target {
                    self.out.calls.insert(key(cur, whole.span), t.clone());
                    if let CallTarget::Method { mut_self: true, .. } = t {
                        if let ExprKind::Field(recv, _) = &callee.kind {
                            self.check_mutable_place(recv, callee.span, true);
                        }
                    }
                }
                if matches!(target, Some(CallTarget::Io { .. })) {
                    self.call_loose(&f, args, first)
                } else {
                    self.call_fn(&f, args, first, whole, callee)
                }
            }
            Ty::TypeRef(inner) => {
                if let Some(CallTarget::Ctor(d)) = &target {
                    self.out
                        .calls
                        .insert(key(cur, whole.span), CallTarget::Ctor(d.clone()));
                }
                self.ctor(&inner, args, whole)
            }
            Ty::Opaque(n) if n.starts_with("prelude:") => {
                let name = n.trim_start_matches("prelude:").to_string();
                self.out
                    .calls
                    .insert(key(cur, whole.span), CallTarget::Builtin(name.clone()));
                self.prelude_call(&name, args, first, whole, exp)
            }
            _ => {
                self.out
                    .calls
                    .insert(key(cur, whole.span), CallTarget::Foreign);
                for a in args {
                    self.expr(&a.value, None);
                }
                Ty::Unknown
            }
        }
    }

    /// Calls on `io` bindings: parameters are typed but arity errors are not reported
    /// (the peripheral tables of B1 define the exact signatures).
    fn call_loose(&mut self, f: &FnTy, args: &[Arg], first: Option<Ty>) -> Ty {
        let _ = first;
        for (i, a) in args.iter().enumerate() {
            let hint = f.params.get(i).map(|p| p.ty.clone());
            let t = self.expr(&a.value, hint.as_ref());
            if let Some(h) = &hint {
                self.coerce(h, &t, &a.value);
            }
        }
        f.ret.clone()
    }

    fn check_bound(&mut self, t: &Ty, bound: &str, span: Span) {
        let ok = match bound {
            "Ord" => is_ord(t),
            "PartialOrd" => matches!(
                t,
                Ty::Num(..)
                    | Ty::Bits { .. }
                    | Ty::Instant
                    | Ty::Param(_)
                    | Ty::Unknown
                    | Ty::Never
            ),
            "Num" => matches!(t, Ty::Num(..) | Ty::Param(_) | Ty::Unknown | Ty::Never),
            "Eq" | "Copy" => true,
            other => match (self.prog.lookup(self.cur.module, other), t) {
                (Some(Res::Item { module, name }), Ty::Adt { def, .. }) => {
                    self.implements(def, &(module, name))
                }
                (Some(Res::Item { .. }), Ty::Param(_) | Ty::Unknown | Ty::Never) => true,
                (Some(Res::Item { .. }), _) => false,
                _ => true,
            },
        };
        if !ok {
            let help = if bound == "Ord"
                && matches!(t, Ty::Num(NumKind::Float(_) | NumKind::LitReal, _))
            {
                "floats are only `PartialOrd`; order them with an explicit comparison"
            } else {
                "use a type that satisfies the bound"
            };
            self.err_help(
                "NE0208",
                format!("`{}` does not satisfy the bound `{bound}`", t.display()),
                span,
                help,
            );
        }
    }

    #[allow(clippy::needless_range_loop)]
    fn call_fn(
        &mut self,
        f: &FnTy,
        args: &[Arg],
        first: Option<Ty>,
        whole: &Expr,
        callee: &Expr,
    ) -> Ty {
        let n = f.params.len();
        let mut slot: Vec<Option<usize>> = vec![None; n];
        let piped = first.is_some() && n > 0;
        if first.is_some() && n == 0 {
            self.err(
                "NE0209",
                "cannot pipe a value into a function without parameters",
                callee.span,
            );
        }
        let mut next = usize::from(piped);
        let mut named_seen = false;
        let mut extra: Vec<usize> = Vec::new();
        for (ai, a) in args.iter().enumerate() {
            match &a.name {
                None => {
                    if named_seen {
                        self.err(
                            "NE0209",
                            "positional arguments must come before named arguments",
                            a.span,
                        );
                    }
                    if next < n {
                        slot[next] = Some(ai);
                        next += 1;
                    } else {
                        self.err(
                            "NE0209",
                            format!("too many arguments: expected at most {n}"),
                            a.span,
                        );
                        extra.push(ai);
                    }
                }
                Some(nm) => {
                    named_seen = true;
                    match f.params.iter().position(|p| p.name == nm.name) {
                        Some(pi) if slot[pi].is_none() && !(piped && pi == 0) => {
                            slot[pi] = Some(ai)
                        }
                        Some(_) => {
                            self.err(
                                "NE0209",
                                format!("argument `{}` is given twice", nm.name),
                                nm.span,
                            );
                            extra.push(ai);
                        }
                        None => {
                            let names = f
                                .params
                                .iter()
                                .map(|p| p.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            self.err_help(
                                "NE0209",
                                format!("unknown parameter `{}`", nm.name),
                                nm.span,
                                format!("parameters: {names}"),
                            );
                            extra.push(ai);
                        }
                    }
                }
            }
        }
        for (pi, p) in f.params.iter().enumerate() {
            if slot[pi].is_none() && !(piped && pi == 0) && !p.has_default {
                self.err(
                    "NE0209",
                    format!("missing argument `{}`", p.name),
                    whole.span,
                );
            }
        }
        for ai in extra {
            self.expr(&args[ai].value, None);
        }
        let gen_names: Vec<String> = f.generics.iter().map(|(g, _)| g.clone()).collect();
        let mut subst: Vec<(String, Ty)> = Vec::new();
        let mut typed: Vec<(usize, Ty, Option<usize>)> = Vec::new();
        for pi in 0..n {
            let pty = &f.params[pi].ty;
            if piped && pi == 0 {
                if let Some(t) = &first {
                    unify(pty, t, &gen_names, &mut subst);
                    typed.push((pi, t.clone(), None));
                }
                continue;
            }
            if let Some(ai) = slot[pi] {
                let hint = if contains_param(pty) {
                    None
                } else {
                    Some(pty.clone())
                };
                let t = self.expr(&args[ai].value, hint.as_ref());
                unify(pty, &t, &gen_names, &mut subst);
                typed.push((pi, t, Some(ai)));
            }
        }
        for (g, bounds) in &f.generics {
            if let Some((_, t)) = subst.iter().find(|(s, _)| s == g) {
                let t = t.clone();
                for b in bounds {
                    self.check_bound(&t, b, whole.span);
                }
            }
        }
        for (pi, t, ai) in typed {
            let p = &f.params[pi];
            let want = p.ty.subst(&subst);
            match ai {
                Some(ai) => {
                    let a = &args[ai];
                    self.coerce(&want, &t, &a.value);
                    if p.mutable != a.mutable {
                        let msg = if p.mutable {
                            format!(
                                "parameter `{}` is `mut`: pass the argument as `mut`",
                                p.name
                            )
                        } else {
                            format!(
                                "parameter `{}` is not `mut`: remove `mut` from the argument",
                                p.name
                            )
                        };
                        self.err("NE0215", msg, a.span);
                    } else if a.mutable {
                        self.check_mutable_place(&a.value, a.span, false);
                    }
                }
                None => self.coerce_at(&want, &t, callee.span, None),
            }
        }
        let leftover: Vec<(String, Ty)> =
            gen_names.iter().map(|g| (g.clone(), Ty::Unknown)).collect();
        f.ret.subst(&subst).subst(&leftover)
    }

    fn ctor(&mut self, inner: &Ty, args: &[Arg], whole: &Expr) -> Ty {
        let (def, targs) = match inner {
            Ty::Adt { def, args } => (def.clone(), args.clone()),
            Ty::Interface(d) => {
                self.err(
                    "NE0208",
                    format!("interface `{}` cannot be constructed", d.1),
                    whole.span,
                );
                return Ty::Unknown;
            }
            _ => return Ty::Unknown,
        };
        let (generics, fields) = match self.out.defs.get(&def).cloned() {
            Some(DefInfo::Struct { generics, fields })
            | Some(DefInfo::Class {
                generics, fields, ..
            }) => (generics, fields),
            Some(DefInfo::Enum { .. }) => {
                self.err_help(
                    "NE0209",
                    format!("enum `{}` cannot be called", def.1),
                    whole.span,
                    "write a variant: `Type.Variant`",
                );
                return Ty::Unknown;
            }
            _ => return Ty::Unknown,
        };
        self.out.calls.insert(
            key(self.cur.module, whole.span),
            CallTarget::Ctor(def.clone()),
        );
        let mut subst: Vec<(String, Ty)> = Vec::new();
        for (g, t) in generics.iter().zip(&targs) {
            if !t.is_unknown() {
                subst.push((g.clone(), t.clone()));
            }
        }
        let mut given: Vec<String> = Vec::new();
        for a in args {
            let Some(nm) = &a.name else {
                self.err_help(
                    "NE0209",
                    "constructor arguments must be named",
                    a.span,
                    "write `field: value`",
                );
                self.expr(&a.value, None);
                continue;
            };
            let Some(f) = fields.iter().find(|f| f.name == nm.name) else {
                let names = fields
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                self.err_help(
                    "NE0212",
                    format!("`{}` has no field `{}`", def.1, nm.name),
                    nm.span,
                    format!("fields: {names}"),
                );
                self.expr(&a.value, None);
                continue;
            };
            if given.contains(&nm.name) {
                self.err(
                    "NE0209",
                    format!("field `{}` is given twice", nm.name),
                    nm.span,
                );
            }
            given.push(nm.name.clone());
            let hint = if contains_param(&f.ty) {
                None
            } else {
                Some(f.ty.clone())
            };
            let t = self.expr(&a.value, hint.as_ref());
            unify(&f.ty, &t, &generics, &mut subst);
            let want = f.ty.subst(&subst);
            self.coerce(&want, &t, &a.value);
        }
        for f in &fields {
            if !f.has_default && !given.contains(&f.name) {
                self.err(
                    "NE0209",
                    format!("missing field `{}` in `{}`", f.name, def.1),
                    whole.span,
                );
            }
        }
        Ty::Adt {
            def,
            args: generics
                .iter()
                .map(|g| {
                    subst
                        .iter()
                        .find(|(n, _)| n == g)
                        .map_or(Ty::Unknown, |(_, t)| t.clone())
                })
                .collect(),
        }
    }

    fn prelude_call(
        &mut self,
        name: &str,
        args: &[Arg],
        first: Option<Ty>,
        whole: &Expr,
        exp: Option<&Ty>,
    ) -> Ty {
        let _ = exp;
        let mut tys: Vec<Ty> = Vec::new();
        if let Some(t) = &first {
            tys.push(t.clone());
        }
        for a in args {
            tys.push(self.expr(&a.value, None));
        }
        let need = |n: usize, this: &mut Self| -> bool {
            if tys.len() < n {
                this.err(
                    "NE0209",
                    format!("`{name}` needs {n} argument(s), found {}", tys.len()),
                    whole.span,
                );
                false
            } else {
                true
            }
        };
        let span = whole.span;
        match name {
            "abs" | "floor" | "ceil" | "round" => {
                if need(1, self) {
                    tys[0].clone()
                } else {
                    Ty::Unknown
                }
            }
            "sign" => Ty::num(NumKind::Int {
                signed: true,
                bits: 8,
            }),
            "min" | "max" | "hypot" => {
                if !need(2, self) {
                    return Ty::Unknown;
                }
                self.same_dim(&tys[0], &tys[1], span);
                match (&tys[0], &tys[1]) {
                    (Ty::Num(k, _), other @ Ty::Num(..)) if k.is_lit() => other.clone(),
                    (a, _) => a.clone(),
                }
            }
            "clamp" => {
                if !need(3, self) {
                    return Ty::Unknown;
                }
                self.same_dim(&tys[0], &tys[1], span);
                self.same_dim(&tys[0], &tys[2], span);
                tys[0].clone()
            }
            "sin" | "cos" | "tan" => {
                if !need(1, self) {
                    return Ty::Unknown;
                }
                if let Ty::Num(k, u) = &tys[0] {
                    if !u.is_dimensionless() && u.dim != Dim::base(7, 1) {
                        self.err_help(
                            "NE0201",
                            format!(
                                "`{name}` needs an angle or a plain number, found a {}",
                                u.dim.describe()
                            ),
                            span,
                            "convert with `/ 1 rad` or pass an angle such as `90 deg`",
                        );
                    }
                    let kind = if k.is_lit() {
                        NumKind::Float(32)
                    } else {
                        k.clone()
                    };
                    return Ty::num(kind);
                }
                Ty::Unknown
            }
            "asin" | "acos" | "atan" | "atan2" => {
                if name == "atan2" {
                    if !need(2, self) {
                        return Ty::Unknown;
                    }
                    self.same_dim(&tys[0], &tys[1], span);
                } else if !need(1, self) {
                    return Ty::Unknown;
                } else {
                    self.expect_plain(&tys[0], name, span);
                }
                lookup_unit("rad").map_or(Ty::Unknown, |u| Ty::Num(NumKind::Float(32), u))
            }
            "exp" | "log" | "log10" => {
                if !need(1, self) {
                    return Ty::Unknown;
                }
                self.expect_plain(&tys[0], name, span);
                match &tys[0] {
                    Ty::Num(k, _) if !k.is_lit() => Ty::num(k.clone()),
                    _ => Ty::num(NumKind::Float(32)),
                }
            }
            "sqrt" => {
                if !need(1, self) {
                    return Ty::Unknown;
                }
                match &tys[0] {
                    Ty::Num(k, u) => match u.dim.sqrt() {
                        Some(dim) if u.scale == Rat::ONE || u.is_dimensionless() => Ty::Num(
                            if k.is_lit() {
                                NumKind::Float(32)
                            } else {
                                k.clone()
                            },
                            Unit {
                                dim,
                                scale: Rat::ONE,
                                offset: Rat::ZERO,
                                label: String::new(),
                            },
                        ),
                        Some(_) => Ty::Num(NumKind::Float(32), Unit::one()),
                        None => {
                            self.err(
                                "NE0201",
                                format!("cannot take the square root of a {}", u.dim.describe()),
                                span,
                            );
                            Ty::Unknown
                        }
                    },
                    _ => Ty::Unknown,
                }
            }
            "now" => Ty::Instant,
            "Ok" => Ty::Result(
                Box::new(tys.first().cloned().unwrap_or(Ty::Unknown)),
                Box::new(Ty::Unknown),
            ),
            "Err" => Ty::Result(
                Box::new(Ty::Unknown),
                Box::new(tys.first().cloned().unwrap_or(Ty::Unknown)),
            ),
            "ton" | "tof" | "tp" => {
                for (a, t) in args
                    .iter()
                    .zip(tys.iter().skip(usize::from(first.is_some())))
                {
                    if a.name.as_ref().is_some_and(|n| n.name == "preset") {
                        self.expect_dim(t, "time", a.value.span, |d| *d == Dim::base(0, 1));
                    }
                }
                Ty::Opaque(name.to_string())
            }
            "xyz" | "xy" | "rpy" | "joints" | "frame" | "tool" => Ty::Opaque(name.to_string()),
            n if prelude::STD_BLOCKS.contains(&n) => Ty::Opaque(n.to_string()),
            _ => Ty::Unknown,
        }
    }

    fn same_dim(&mut self, a: &Ty, b: &Ty, span: Span) {
        if let (Ty::Num(_, ua), Ty::Num(_, ub)) = (a, b) {
            if ua.dim != ub.dim && !ua.is_dimensionless() && !ub.is_dimensionless() {
                self.dim_error("combine", ua, ub, span);
            }
        }
    }

    fn expect_plain(&mut self, t: &Ty, name: &str, span: Span) {
        if let Ty::Num(_, u) = t {
            if !u.is_dimensionless() {
                self.err_help(
                    "NE0201",
                    format!(
                        "`{name}` needs a plain number, found a {}",
                        u.dim.describe()
                    ),
                    span,
                    "divide by a unit first, for example `x / 1 mm`",
                );
            }
        }
    }

    // ---- mutability -------------------------------------------------------

    /// Checks that the place `e` may be modified (assigned, passed `mut`, or used as the
    /// receiver of a `mut self` method).
    pub(crate) fn check_mutable_place(&mut self, e: &Expr, span: Span, receiver: bool) {
        let mut projected = false;
        let mut cur = e;
        while let ExprKind::Field(b, _) | ExprKind::Index(b, _, _) = &cur.kind {
            projected = true;
            cur = b;
        }
        let what = if receiver {
            "call a `mut self` method on"
        } else {
            "modify"
        };
        match &cur.kind {
            ExprKind::SelfValue => {
                if !self.cur.self_mut {
                    self.err_help(
                        "NE0210",
                        format!("cannot {what} `self` here"),
                        span,
                        "declare the method with `mut self`",
                    );
                }
            }
            ExprKind::Name(id) => {
                let t = self.name(id, cur);
                let opaque = matches!(t, Ty::Unknown | Ty::Opaque(_) | Ty::Io(_));
                match self.res_of(id).cloned() {
                    Some(Res::Local(i)) => {
                        let kind = self.resolved.locals.get(i).map(|l| l.kind);
                        let immutable = matches!(
                            kind,
                            Some(
                                LocalKind::Param { mutable: false }
                                    | LocalKind::Let
                                    | LocalKind::Pattern
                                    | LocalKind::For
                                    | LocalKind::Lambda
                                    | LocalKind::InPort
                            )
                        );
                        if immutable && (!projected || !opaque) {
                            self.err_help(
                                "NE0210",
                                format!("cannot {what} `{}`: it is not mutable", id.name),
                                span,
                                "declare it with `var`, or as a `mut` parameter",
                            );
                        }
                    }
                    Some(Res::Item { module, name }) => {
                        let Some(sym) = self.prog.symbol(module, &name) else {
                            return;
                        };
                        let immutable = match sym.kind {
                            SymKind::Const => true,
                            SymKind::PlcVar(neuron_syntax::ast::PlcSection::Inputs) => true,
                            SymKind::Var => self
                                .prog
                                .files
                                .get(module)
                                .and_then(|f| f.ast.items.get(sym.item))
                                .is_some_and(
                                    |it| matches!(&it.kind, ItemKind::Var(v) if !v.mutable),
                                ),
                            _ => false,
                        };
                        if immutable && (!projected || !opaque) {
                            self.err_help(
                                "NE0210",
                                format!("cannot {what} `{name}`: it is not mutable"),
                                span,
                                "declare it with `var`",
                            );
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn base_unit(q: &crate::consteval::Qty) -> Unit {
    Unit {
        dim: q.dim,
        scale: Rat::ONE,
        offset: Rat::ZERO,
        label: String::new(),
    }
}
