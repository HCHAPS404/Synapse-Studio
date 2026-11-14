//! Generic traversal helpers over statements and expressions.

use neuron_syntax::ast::{Arg, Expr, ExprKind, SeqItem, Stmt, StmtKind};

/// Visits `stmts` and every statement nested in them, in source order.
pub fn each_stmt<'a>(stmts: &'a [Stmt], f: &mut impl FnMut(&'a Stmt)) {
    for s in stmts {
        f(s);
        match &s.kind {
            StmtKind::If {
                branches,
                otherwise,
            } => {
                for (_, b) in branches {
                    each_stmt(&b.stmts, f);
                }
                if let Some(o) = otherwise {
                    each_stmt(&o.stmts, f);
                }
            }
            StmtKind::Match { arms, .. } => {
                for a in arms {
                    each_stmt(&a.body.stmts, f);
                }
            }
            StmtKind::For { body, .. }
            | StmtKind::While { body, .. }
            | StmtKind::Loop { body, .. } => each_stmt(&body.stmts, f),
            StmtKind::Atomic(b) => each_stmt(&b.stmts, f),
            StmtKind::Rule(r) => each_stmt(&r.body.stmts, f),
            _ => {}
        }
    }
}

/// Collects the statements of a sequence body (through steps and branches).
pub fn seq_stmts<'a>(items: &'a [SeqItem], out: &mut Vec<&'a Stmt>) {
    for it in items {
        match it {
            SeqItem::Step { body, .. } => seq_stmts(body, out),
            SeqItem::Parallel { branches, .. } => {
                for (_, b) in branches {
                    seq_stmts(b, out);
                }
            }
            SeqItem::Stmt(s) => out.push(s),
        }
    }
}

fn args<'a>(a: &'a [Arg], f: &mut impl FnMut(&'a Expr)) {
    for x in a {
        f(&x.value);
    }
}

/// Visits the expressions that belong directly to `s` (not to nested statements).
pub fn stmt_exprs<'a>(s: &'a Stmt, f: &mut impl FnMut(&'a Expr)) {
    match &s.kind {
        StmtKind::Let { value, .. } => f(value),
        StmtKind::Assign {
            place, value, when, ..
        } => {
            f(place);
            f(value);
            if let Some(w) = when {
                f(w);
            }
        }
        StmtKind::Expr(e) | StmtKind::Wait(e) => f(e),
        StmtKind::Return(e) => {
            if let Some(e) = e {
                f(e);
            }
        }
        StmtKind::Fault { args: a, .. } | StmtKind::Emit { args: a, .. } => args(a, f),
        StmtKind::Assert { cond, .. } => f(cond),
        StmtKind::Coil { place, when, .. } => {
            f(place);
            f(when);
        }
        StmtKind::Until { cond, timeout, .. } => {
            f(cond);
            if let Some(t) = timeout {
                f(t);
            }
        }
        StmtKind::Move(m) => {
            f(&m.to);
            if let Some(v) = &m.via {
                f(v);
            }
            for (_, e) in &m.options {
                f(e);
            }
        }
        StmtKind::If { branches, .. } => {
            for (c, _) in branches {
                f(c);
            }
        }
        StmtKind::Match { value, arms } => {
            f(value);
            for a in arms {
                if let Some(g) = &a.guard {
                    f(g);
                }
            }
        }
        StmtKind::For { iter, .. } => f(iter),
        StmtKind::While { cond, max, .. } => {
            f(cond);
            if let Some(m) = max {
                f(m);
            }
        }
        StmtKind::Loop { max, .. } => {
            if let Some(m) = max {
                f(m);
            }
        }
        StmtKind::Rule(r) => {
            f(&r.cond);
            if let Some(h) = &r.hold {
                f(h);
            }
        }
        StmtKind::Pass
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Jump(_)
        | StmtKind::Atomic(_)
        | StmtKind::Asm(_) => {}
    }
}

/// Visits `e` and all its sub-expressions, parents before children.
pub fn each_expr<'a>(e: &'a Expr, f: &mut impl FnMut(&'a Expr)) {
    f(e);
    match &e.kind {
        ExprKind::Lit(_)
        | ExprKind::Name(_)
        | ExprKind::SelfValue
        | ExprKind::Result
        | ExprKind::Error => {}
        ExprKind::Quantity(x, _)
        | ExprKind::Unary(_, x)
        | ExprKind::Try(x)
        | ExprKind::Old(x)
        | ExprKind::Run(x)
        | ExprKind::Field(x, _)
        | ExprKind::Cast(x, _)
        | ExprKind::Lambda(_, x) => each_expr(x, f),
        ExprKind::Binary(_, a, b)
        | ExprKind::Map(a, b)
        | ExprKind::Pipe(a, b)
        | ExprKind::ArrayRepeat(a, b) => {
            each_expr(a, f);
            each_expr(b, f);
        }
        ExprKind::Range { lo, hi, .. } => {
            each_expr(lo, f);
            each_expr(hi, f);
        }
        ExprKind::Compare(first, rest) => {
            each_expr(first, f);
            for (_, x) in rest {
                each_expr(x, f);
            }
        }
        ExprKind::IfElse(c, a, b) => {
            each_expr(c, f);
            each_expr(a, f);
            each_expr(b, f);
        }
        ExprKind::Call(callee, a) => {
            each_expr(callee, f);
            for x in a {
                each_expr(&x.value, f);
            }
        }
        ExprKind::Index(b, i, end) => {
            each_expr(b, f);
            each_expr(i, f);
            if let Some(x) = end {
                each_expr(x, f);
            }
        }
        ExprKind::Tuple(xs) | ExprKind::Array(xs) => {
            for x in xs {
                each_expr(x, f);
            }
        }
    }
}
