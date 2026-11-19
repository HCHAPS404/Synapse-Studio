//! Small readers over NIR shared by the emitters.

use neuron_nir::{CallTarget, Expr, ExprKind, Lit, Nir, Type};

/// An exact NIR number (`"0.01"`, `"1/3"`) as `f64`.
pub fn parse_num(s: &str) -> f64 {
    match s.split_once('/') {
        Some((n, d)) => {
            let d: f64 = d.trim().parse().unwrap_or(1.0);
            if d == 0.0 {
                0.0
            } else {
                n.trim().parse::<f64>().unwrap_or(0.0) / d
            }
        }
        None => s.trim().parse().unwrap_or(0.0),
    }
}

/// The value of a constant in base SI units (a literal number, or a quantity).
pub fn si(e: &Expr) -> Option<f64> {
    match &e.e {
        ExprKind::Lit {
            lit: Lit::Int { v, .. } | Lit::Real { v, .. },
        } => Some(parse_num(v)),
        ExprKind::Quantity { si: Some(s), .. } => Some(parse_num(s)),
        ExprKind::Quantity { value, unit, .. } => {
            let n = si(value)?;
            let scale = match &e.ty {
                Type::Num { unit: Some(u), .. } => parse_num(&u.scale),
                _ => 1.0,
            };
            let _ = unit;
            Some(n * scale)
        }
        ExprKind::Unary { op, operand } if op == "-" => si(operand).map(|v| -v),
        _ => None,
    }
}

/// A constant duration in nanoseconds.
pub fn dur_ns(e: &Expr) -> Option<i64> {
    if !is_time(&e.ty) {
        return None;
    }
    si(e).map(|s| (s * 1e9).round() as i64)
}

/// `true` when the type is a duration (seconds to the power 1, nothing else).
pub fn is_time(t: &Type) -> bool {
    match t {
        Type::Num { kind, unit } => {
            kind == "duration"
                || unit
                    .as_ref()
                    .is_some_and(|u| u.dim == [1, 0, 0, 0, 0, 0, 0, 0, 0])
        }
        _ => false,
    }
}

/// A constant integer.
pub fn int(e: &Expr) -> Option<i64> {
    match &e.e {
        ExprKind::Lit {
            lit: Lit::Int { v, .. },
        } => v.parse().ok(),
        ExprKind::Unary { op, operand } if op == "-" => int(operand).map(|v| -v),
        _ => None,
    }
}

/// A constant boolean.
pub fn boolean(e: &Expr) -> Option<bool> {
    match &e.e {
        ExprKind::Lit {
            lit: Lit::Bool { v },
        } => Some(*v),
        _ => None,
    }
}

/// IEC duration literal: `T#3S`, `T#500MS`, `T#1500US`.
pub fn iec_time(ns: i64) -> String {
    let (sign, ns) = if ns < 0 { ("-", -ns) } else { ("", ns) };
    if ns % 1_000_000_000 == 0 {
        format!("T#{sign}{}S", ns / 1_000_000_000)
    } else if ns % 1_000_000 == 0 {
        format!("T#{sign}{}MS", ns / 1_000_000)
    } else if ns % 1_000 == 0 {
        format!("T#{sign}{}US", ns / 1_000)
    } else {
        format!("T#{sign}{ns}NS")
    }
}

/// Name of the prelude function or block a call names (`ton`, `abs`, `xyz`).
pub fn builtin(e: &Expr) -> Option<&str> {
    let ExprKind::Call { callee, target, .. } = &e.e else {
        return None;
    };
    if let Some(CallTarget::Builtin { name }) = target {
        return Some(name);
    }
    match &callee.e {
        ExprKind::Global { name } => Some(name),
        _ => None,
    }
}

/// Argument `name`, or the positional argument `pos`.
pub fn arg<'a>(args: &'a [neuron_nir::Arg], name: &str, pos: usize) -> Option<&'a Expr> {
    if let Some(a) = args.iter().find(|a| a.name.as_deref() == Some(name)) {
        return Some(&a.value);
    }
    args.iter()
        .filter(|a| a.name.is_none())
        .nth(pos)
        .map(|a| &a.value)
}

/// The source text of a NIR string literal without its quotes.
pub fn unquote(s: &str) -> String {
    s.trim_matches('"').to_string()
}

/// File of module `m`.
pub fn file(nir: &Nir, m: usize) -> String {
    nir.modules
        .get(m)
        .map(|x| x.file.clone())
        .unwrap_or_default()
}

/// Display unit text of a type, when it has one.
pub fn unit_text(t: &Type) -> Option<&str> {
    match t {
        Type::Num { unit: Some(u), .. } => Some(&u.text),
        _ => None,
    }
}

/// Converts a value in base units to the display unit of `t`.
pub fn from_si(t: &Type, si: f64) -> f64 {
    match t {
        Type::Num { unit: Some(u), .. } => {
            let s = parse_num(&u.scale);
            let o = parse_num(&u.offset);
            if s == 0.0 {
                si
            } else {
                (si - o) / s
            }
        }
        _ => si,
    }
}

/// A number as a short decimal (`3`, `0.5`, `120`).
pub fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.9}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}
