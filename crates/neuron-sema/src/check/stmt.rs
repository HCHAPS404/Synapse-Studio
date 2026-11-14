//! Statement checking.

use super::{default_ty, Checker, DefInfo};
use crate::ty::{NumKind, Ty};
use neuron_syntax::ast::{AssignOp, Block, Expr, ExprKind, Rule, Stmt, StmtKind, VarDecl};

fn diverges(s: &Stmt) -> bool {
    match &s.kind {
        StmtKind::Return(_) | StmtKind::Fault { .. } => true,
        StmtKind::If {
            branches,
            otherwise: Some(o),
        } => branches.iter().all(|(_, b)| always_returns(&b.stmts)) && always_returns(&o.stmts),
        StmtKind::Match { arms, .. } => {
            !arms.is_empty() && arms.iter().all(|a| always_returns(&a.body.stmts))
        }
        StmtKind::Loop { max: None, body } => !contains_break(&body.stmts),
        _ => false,
    }
}

fn contains_break(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match &s.kind {
        StmtKind::Break => true,
        StmtKind::If {
            branches,
            otherwise,
        } => {
            branches.iter().any(|(_, b)| contains_break(&b.stmts))
                || otherwise.as_ref().is_some_and(|o| contains_break(&o.stmts))
        }
        StmtKind::Match { arms, .. } => arms.iter().any(|a| contains_break(&a.body.stmts)),
        StmtKind::Atomic(b) => contains_break(&b.stmts),
        _ => false,
    })
}

/// `true` when execution cannot fall off the end of `stmts`.
pub(super) fn always_returns(stmts: &[Stmt]) -> bool {
    stmts.iter().any(diverges)
}

impl Checker<'_> {
    pub(crate) fn block(&mut self, b: &Block) {
        for c in &b.contracts {
            self.cond(&c.cond);
        }
        for s in &b.stmts {
            self.stmt(s);
        }
    }

    pub(crate) fn local_var(&mut self, v: &VarDecl) {
        let declared = v.ty.as_ref().map(|t| self.lower(t));
        let t = match (&v.init, declared) {
            (Some(e), Some(d)) => {
                let a = self.expr(e, Some(&d));
                self.coerce(&d, &a, e);
                d
            }
            (Some(e), None) => default_ty(&self.expr(e, None)),
            (None, Some(d)) => d,
            (None, None) => Ty::Unknown,
        };
        self.set_local(&v.name, t);
    }

    pub(crate) fn rule(&mut self, r: &Rule) {
        self.cond(&r.cond);
        if let Some(h) = &r.hold {
            self.duration(h);
        }
        self.block(&r.body);
    }

    fn loop_body(&mut self, b: &Block) {
        self.cur.loop_depth += 1;
        self.block(b);
        self.cur.loop_depth -= 1;
    }

    fn check_field_mutability(&mut self, place: &Expr) {
        let ExprKind::Field(base, id) = &place.kind else {
            return;
        };
        self.silent += 1;
        let bt = self.expr(base, None);
        self.silent -= 1;
        if let Ty::Adt { def, .. } = bt {
            let fields = match self.out.defs.get(&def) {
                Some(DefInfo::Struct { fields, .. }) | Some(DefInfo::Class { fields, .. }) => {
                    fields.clone()
                }
                _ => return,
            };
            if let Some(f) = fields.iter().find(|f| f.name == id.name) {
                if !f.mutable {
                    self.err_help(
                        "NE0210",
                        format!(
                            "field `{}` of `{}` is declared with `let` and cannot be assigned",
                            id.name, def.1
                        ),
                        place.span,
                        "declare the field with `var`",
                    );
                }
            }
        }
    }

    pub(crate) fn stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Let {
                pattern, ty, value, ..
            } => {
                let declared = ty.as_ref().map(|t| self.lower(t));
                let a = self.expr(value, declared.as_ref());
                let t = match declared {
                    Some(d) => {
                        self.coerce(&d, &a, value);
                        d
                    }
                    None => {
                        if matches!(a, Ty::Slice(_)) {
                            a
                        } else {
                            default_ty(&a)
                        }
                    }
                };
                self.pattern(pattern, &t);
            }
            StmtKind::Assign {
                place,
                op,
                value,
                when,
            } => {
                let pt = self.expr(place, None);
                self.check_mutable_place(place, place.span, false);
                self.check_field_mutability(place);
                let vt = self.expr(value, Some(&pt));
                match op {
                    AssignOp::Set => self.coerce(&pt, &vt, value),
                    AssignOp::Compound(bop) => {
                        let (l, r) = (self.rv(pt.clone()), self.rv(vt));
                        let res = self.binary_tys(*bop, place, value, &l, &r, s.span);
                        self.coerce(&pt, &res, value);
                    }
                }
                if let Some(w) = when {
                    self.cond(w);
                }
            }
            StmtKind::Expr(e) => {
                self.expr(e, None);
            }
            StmtKind::Pass | StmtKind::Asm(_) | StmtKind::Jump(_) => {}
            StmtKind::Return(e) => {
                let ret = self.cur.ret.clone();
                match (ret, e) {
                    (Some(rt), Some(e)) => {
                        let a = self.expr(e, Some(&rt));
                        self.coerce(&rt, &a, e);
                    }
                    (Some(rt), None) => {
                        if rt != Ty::Void && !rt.is_unknown() {
                            self.err(
                                "NE0208",
                                format!("expected a return value of type `{}`", rt.display()),
                                s.span,
                            );
                        }
                    }
                    (None, Some(e)) => {
                        self.expr(e, None);
                        self.err(
                            "NE0214",
                            "`return` with a value is only allowed inside a function",
                            s.span,
                        );
                    }
                    (None, None) => {}
                }
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.cur.loop_depth == 0 {
                    self.err(
                        "NE0214",
                        "`break` and `continue` are only allowed inside a loop",
                        s.span,
                    );
                }
            }
            StmtKind::Fault { args, .. } | StmtKind::Emit { args, .. } => {
                for a in args {
                    self.expr(&a.value, None);
                }
            }
            StmtKind::Assert { cond, .. } => self.cond(cond),
            StmtKind::Coil { place, when, .. } => {
                let pt = self.expr(place, None);
                self.check_mutable_place(place, place.span, false);
                self.coerce(&Ty::Bool, &self.rv(pt), place);
                self.cond(when);
            }
            StmtKind::Wait(e) => self.duration(e),
            StmtKind::Until { cond, timeout, .. } => {
                self.cond(cond);
                if let Some(t) = timeout {
                    self.duration(t);
                }
            }
            StmtKind::Move(m) => {
                self.expr(&m.to, None);
                if let Some(v) = &m.via {
                    self.expr(v, None);
                }
                for (_, e) in &m.options {
                    self.expr(e, None);
                }
            }
            StmtKind::If {
                branches,
                otherwise,
            } => {
                for (c, b) in branches {
                    self.cond(c);
                    self.block(b);
                }
                if let Some(b) = otherwise {
                    self.block(b);
                }
            }
            StmtKind::Match { value, arms } => {
                let st = self.expr(value, None);
                let st = self.rv(st);
                for a in arms {
                    self.pattern(&a.pattern, &st);
                    if let Some(g) = &a.guard {
                        self.cond(g);
                    }
                    self.block(&a.body);
                }
                self.check_exhaustive(&st, arms, s.span);
            }
            StmtKind::For {
                pattern,
                iter,
                body,
            } => {
                let it = self.expr(iter, None);
                let (elem, len) = match &it {
                    Ty::Range(t) => {
                        let len = match &iter.kind {
                            ExprKind::Range { lo, hi, inclusive } => {
                                let m = self.cur.module;
                                match (self.consts.int(m, lo), self.consts.int(m, hi)) {
                                    (Some(a), Some(b)) if b >= a => {
                                        u64::try_from(b - a + i128::from(*inclusive)).ok()
                                    }
                                    (Some(_), Some(_)) => Some(0),
                                    _ => None,
                                }
                            }
                            _ => None,
                        };
                        let elem = match &**t {
                            Ty::Num(NumKind::LitInt, u) => Ty::Num(NumKind::Index, u.clone()),
                            other => other.clone(),
                        };
                        (elem, len)
                    }
                    Ty::Array(t, n) => ((**t).clone(), *n),
                    Ty::Slice(t) => ((**t).clone(), None),
                    Ty::Unknown | Ty::Opaque(_) | Ty::Never => (Ty::Unknown, None),
                    other => {
                        self.err(
                            "NE0208",
                            format!("`{}` cannot be iterated", other.display()),
                            iter.span,
                        );
                        (Ty::Unknown, None)
                    }
                };
                self.out
                    .iter_len
                    .insert(crate::model::key(self.cur.module, iter.span), len);
                self.pattern(pattern, &elem);
                self.loop_body(body);
            }
            StmtKind::While {
                pattern,
                cond,
                max,
                body,
            } => {
                if let Some(p) = pattern {
                    let t = self.expr(cond, None);
                    self.pattern(p, &t);
                } else {
                    self.cond(cond);
                }
                if let Some(m) = max {
                    let t = self.expr(m, None);
                    self.expect_int(&t, m.span);
                }
                self.loop_body(body);
            }
            StmtKind::Loop { max, body } => {
                if let Some(m) = max {
                    let t = self.expr(m, None);
                    self.expect_int(&t, m.span);
                }
                self.loop_body(body);
            }
            StmtKind::Atomic(b) => self.block(b),
            StmtKind::Rule(r) => self.rule(r),
        }
    }
}
