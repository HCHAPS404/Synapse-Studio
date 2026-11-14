//! Compile-time evaluation of constant expressions (literals, `const` items,
//! arithmetic and quantities).

use crate::model::{key, Program, Res, SpanKey, SymKind};
use crate::units::{to_base, Dim, Rat};
use neuron_syntax::ast::{BinOp, Expr, ExprKind, ItemKind, Lit, UnOp};
use std::collections::BTreeMap;

/// A constant value in base units together with its dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Qty {
    /// Value in the base unit of the dimension.
    pub value: Rat,
    /// Dimension.
    pub dim: Dim,
}

/// Evaluator bound to a program and its name resolution.
pub struct Consts<'a> {
    prog: &'a Program,
    res: &'a BTreeMap<SpanKey, Res>,
}

const MAX_DEPTH: u8 = 12;

impl<'a> Consts<'a> {
    /// Creates an evaluator.
    pub fn new(prog: &'a Program, res: &'a BTreeMap<SpanKey, Res>) -> Self {
        Consts { prog, res }
    }

    /// Evaluates `e`, which lives in `file`.
    pub fn eval(&self, file: usize, e: &Expr) -> Option<Qty> {
        self.go(file, e, 0)
    }

    /// An integer constant (dimensionless).
    pub fn int(&self, file: usize, e: &Expr) -> Option<i128> {
        let q = self.eval(file, e)?;
        if q.dim.is_none() && q.value.is_integer() {
            Some(q.value.numer())
        } else {
            None
        }
    }

    /// A duration in nanoseconds.
    pub fn duration_ns(&self, file: usize, e: &Expr) -> Option<i128> {
        let q = self.eval(file, e)?;
        if q.dim != Dim::base(0, 1) {
            return None;
        }
        let ns = q.value.mul(Rat::int(1_000_000_000))?;
        if ns.is_integer() {
            Some(ns.numer())
        } else {
            None
        }
    }

    fn lit(l: &Lit) -> Option<Rat> {
        match l {
            Lit::Int(t) | Lit::Real(t) => Rat::parse(t),
            Lit::Bool(b) => Some(Rat::int(i128::from(*b))),
            _ => None,
        }
    }

    fn go(&self, file: usize, e: &Expr, depth: u8) -> Option<Qty> {
        if depth > MAX_DEPTH {
            return None;
        }
        match &e.kind {
            ExprKind::Lit(l) => Some(Qty {
                value: Self::lit(l)?,
                dim: Dim::NONE,
            }),
            ExprKind::Quantity(inner, unit) => {
                let v = self.go(file, inner, depth + 1)?;
                if !v.dim.is_none() {
                    return None;
                }
                let u = self.prog.units.resolve(unit).ok()?;
                Some(Qty {
                    value: to_base(v.value, &u)?,
                    dim: u.dim,
                })
            }
            ExprKind::Unary(UnOp::Neg, x) => {
                let q = self.go(file, x, depth + 1)?;
                Some(Qty {
                    value: Rat::ZERO.sub(q.value)?,
                    dim: q.dim,
                })
            }
            ExprKind::Binary(op, a, b) => {
                let (x, y) = (self.go(file, a, depth + 1)?, self.go(file, b, depth + 1)?);
                match op {
                    BinOp::Add | BinOp::Sub if x.dim == y.dim => Some(Qty {
                        value: if *op == BinOp::Add {
                            x.value.add(y.value)?
                        } else {
                            x.value.sub(y.value)?
                        },
                        dim: x.dim,
                    }),
                    BinOp::Mul => Some(Qty {
                        value: x.value.mul(y.value)?,
                        dim: x.dim.mul(&y.dim)?,
                    }),
                    BinOp::Div => Some(Qty {
                        value: x.value.div(y.value)?,
                        dim: x.dim.div(&y.dim)?,
                    }),
                    _ => None,
                }
            }
            ExprKind::Name(id) => match self.res.get(&key(file, id.span))? {
                Res::Item { module, name } => {
                    let sym = self.prog.symbol(*module, name)?;
                    if sym.kind != SymKind::Const {
                        return None;
                    }
                    let item = self.prog.files.get(*module)?.ast.items.get(sym.item)?;
                    if let ItemKind::Const { value, .. } = &item.kind {
                        self.go(*module, value, depth + 1)
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }
}
