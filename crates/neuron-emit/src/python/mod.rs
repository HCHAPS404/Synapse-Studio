//! The `python` emitter: `host` modules to Python 3 programs that run in simulated time.
//!
//! One `<module>.py` per `host` module plus `neuron_rt.py`, the runtime they share (an
//! event queue with the scheduling rules of the reference interpreter: logical execution
//! time for signals, periodic and event tasks, `when` rules, faults and the safe state).
//! `python app.py --test` runs the `test` blocks, `python app.py --trace 3s` prints the
//! trace in the interpreter's text format. Standard library only, Python 3.10 or newer.
//!
//! Translated: constants, variables, signals, functions, tasks (`every` and `on`), `when`
//! rules, `safety:`, `test` blocks, integers (checked like the interpreter), reals (with
//! `f32` rounding), quantities (stored in base units), arrays, tuples and the function
//! blocks `moving_average`, `hysteresis`, `debounce`, `ramp`, `lowpass`, `highpass`.
//! Everything else is `NE0610` with its location.

use crate::{nirx, Class, Diag, EmitOptions, Emitter, Fidelity, Manifest, Package, Writer};
use neuron_nir::{
    Block, Expr, ExprKind, FnBody, FnDecl, ItemDef, Lit, Module, Nir, Param, Pattern, PatternKind,
    SafetyDef, Stmt, StmtKind, TaskAttr, Trigger, Type,
};
use std::collections::{BTreeSet, HashMap};

const RUNTIME: &str = include_str!("neuron_rt.py");

const CONSTRUCTS: &[&str] = &[
    "const",
    "var",
    "signal",
    "fn",
    "task every",
    "task on",
    "when",
    "safety",
    "test",
    "emit",
    "fault",
    "assert",
    "if",
    "for",
    "while",
    "loop",
    "moving_average",
    "hysteresis",
    "debounce",
    "ramp",
    "lowpass",
    "highpass",
];

/// The `python` emitter.
pub struct Python;

/// Python class and parameters of a function block.
fn block_class(name: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match name {
        "moving_average" => ("rt.MovingAverage", &["window"]),
        "hysteresis" => ("rt.Hysteresis", &["low", "high"]),
        "debounce" => ("rt.Debounce", &["time"]),
        "ramp" => ("rt.Ramp", &["rate"]),
        "lowpass" => ("rt.LowPass", &["cutoff"]),
        "highpass" => ("rt.HighPass", &["cutoff"]),
        _ => return None,
    })
}

fn int_range(kind: &str) -> Option<(i128, i128)> {
    Some(match kind {
        "u8" => (0, 255),
        "u16" => (0, 65_535),
        "u32" => (0, i128::from(u32::MAX)),
        "u64" | "index" => (0, i128::from(u64::MAX)),
        "i8" => (-128, 127),
        "i16" => (-32_768, 32_767),
        "i32" => (i128::from(i32::MIN), i128::from(i32::MAX)),
        "i64" => (i128::from(i64::MIN), i128::from(i64::MAX)),
        _ => return None,
    })
}

/// `Some(range)` for integer types (`None` range for untyped literals), `None` otherwise.
fn int_class(ty: &Type) -> Option<Option<(i128, i128)>> {
    match ty {
        Type::Num { kind, unit: None } => match kind.as_str() {
            "integer" => Some(None),
            k => int_range(k).map(Some),
        },
        _ => None,
    }
}

fn is_float(ty: &Type) -> bool {
    match ty {
        Type::Num { kind, unit } => {
            unit.is_some() || matches!(kind.as_str(), "f32" | "f64" | "real" | "duration")
        }
        _ => false,
    }
}

fn is_f32(ty: &Type) -> bool {
    matches!(ty, Type::Num { kind, unit: None } if kind == "f32")
}

fn needs_deep(ty: &Type) -> bool {
    matches!(ty, Type::Array { .. } | Type::Tuple { .. })
}

/// A Python float literal.
fn py_float(x: f64) -> String {
    if x.is_nan() {
        "float('nan')".into()
    } else if x.is_infinite() {
        if x > 0.0 {
            "float('inf')"
        } else {
            "float('-inf')"
        }
        .into()
    } else {
        format!("{x:?}")
    }
}

/// A Python string literal (ASCII only).
fn py_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (' '..='~').contains(&c) => out.push(c),
            c if (c as u32) <= 0xFFFF => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push_str(&format!("\\U{:08x}", c as u32)),
        }
    }
    out.push('"');
    out
}

fn ident(s: &str) -> String {
    let mut o: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if o.is_empty() {
        o.push('_');
    }
    o
}

/// The text of a string literal as the interpreter reads it.
fn unescape(text: &str) -> String {
    let t = text.trim();
    let inner = t
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(t);
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Dotted text of a name or field chain.
fn dotted(e: &Expr) -> String {
    match &e.e {
        ExprKind::Global { name } | ExprKind::Ref { name, .. } | ExprKind::Local { name, .. } => {
            name.clone()
        }
        ExprKind::Module { name, .. } => name.clone(),
        ExprKind::Field { base, name, .. } => {
            let b = dotted(base);
            if b.is_empty() {
                name.clone()
            } else {
                format!("{b}.{name}")
            }
        }
        ExprKind::Call { callee, .. } => dotted(callee),
        _ => String::new(),
    }
}

/// `true` when the chain starts at a `Global` (an input from outside the package).
fn global_root(e: &Expr) -> bool {
    match &e.e {
        ExprKind::Global { .. } => true,
        ExprKind::Field { base, .. } => global_root(base),
        _ => false,
    }
}

struct Lower<'a> {
    nir: &'a Nir,
    file: String,
    defs: HashMap<String, &'a ItemDef>,
    blocks: HashMap<String, &'static str>,
    diags: Vec<Diag>,
    fid: Fidelity,
    assigned: BTreeSet<String>,
    tmp: usize,
    libm: bool,
    watchdog: Option<[usize; 2]>,
    module_vars: Vec<String>,
}

impl<'a> Lower<'a> {
    fn new(nir: &'a Nir, m: usize) -> Lower<'a> {
        let module = &nir.modules[m];
        let mut defs = HashMap::new();
        let mut blocks = HashMap::new();
        for it in &module.items {
            defs.insert(it.id.clone(), &it.def);
            if let ItemDef::Var { var } = &it.def {
                if let Some(init) = &var.init {
                    if let Some(n) = nirx::builtin(init) {
                        if let Some((class, _)) = block_class(n) {
                            blocks.insert(var.id.clone(), class);
                        }
                    }
                }
            }
        }
        Lower {
            nir,
            file: module.file.clone(),
            defs,
            blocks,
            diags: Vec::new(),
            fid: Fidelity::new("emit-python", "host.python"),
            assigned: BTreeSet::new(),
            tmp: 0,
            libm: false,
            watchdog: None,
            module_vars: Vec::new(),
        }
    }

    fn unsupported(&mut self, span: [usize; 2], what: &str) {
        self.diags.push(Diag {
            code: "NE0610",
            message: format!("{what} is not translated by the python emitter"),
            help: Some(
                "the python emitter translates `host` modules with tasks, rules, safety, functions and tests; translate that part by hand"
                    .into(),
            ),
            file: self.file.clone(),
            span,
        });
    }

    fn lname(id: &str, name: &str) -> String {
        format!("{}_{}", ident(name), ident(id))
    }

    fn next_tmp(&mut self) -> usize {
        self.tmp += 1;
        self.tmp
    }

    // ---- types and defaults ---------------------------------------------------------

    fn default_py(&mut self, ty: &Type, span: [usize; 2]) -> String {
        match ty {
            Type::Bool => "False".into(),
            Type::Num { unit: Some(u), .. } => py_float(nirx::parse_num(&u.offset)),
            Type::Num { kind, unit: None } => {
                if is_float(ty) || kind == "duration" {
                    "0.0".into()
                } else if int_class(ty).is_some() {
                    "0".into()
                } else {
                    self.unsupported(span, &format!("the type `{kind}`"));
                    "None".into()
                }
            }
            Type::Instant => "0.0".into(),
            Type::Str { .. } => "\"\"".into(),
            Type::Array { elem, len } => {
                let d = self.default_py(elem, span);
                format!("[{d} for _ in range({})]", len.unwrap_or(0).min(1 << 20))
            }
            Type::Tuple { items } => {
                let v: Vec<String> = items.iter().map(|t| self.default_py(t, span)).collect();
                if v.len() == 1 {
                    format!("({},)", v[0])
                } else {
                    format!("({})", v.join(", "))
                }
            }
            _ => {
                self.unsupported(span, "a variable of this type");
                "None".into()
            }
        }
    }

    // ---- expressions ------------------------------------------------------------------

    fn ex(&mut self, e: &Expr) -> String {
        let s = self.ex_raw(e);
        let wrap = is_f32(&e.ty)
            && matches!(
                e.e,
                ExprKind::Binary { .. }
                    | ExprKind::Unary { .. }
                    | ExprKind::Cast { .. }
                    | ExprKind::Pipe { .. }
                    | ExprKind::Call { .. }
            );
        if wrap {
            format!("rt.f32({s})")
        } else {
            s
        }
    }

    fn ex_raw(&mut self, e: &Expr) -> String {
        match &e.e {
            ExprKind::Lit { lit } => self.lit(e, lit),
            ExprKind::Quantity { value, unit, si } => {
                if let Some(s) = si {
                    return py_float(nirx::parse_num(s));
                }
                let v = self.ex(value);
                match &e.ty {
                    Type::Num { unit: Some(u), .. } => {
                        let (sc, of) = (nirx::parse_num(&u.scale), nirx::parse_num(&u.offset));
                        let _ = unit;
                        format!("({v} * {} + {})", py_float(sc), py_float(of))
                    }
                    _ => v,
                }
            }
            ExprKind::Ref { id, name } => match self.defs.get(id).copied() {
                Some(ItemDef::Var { .. } | ItemDef::Const { .. }) => {
                    format!("g_{}", ident(name))
                }
                Some(ItemDef::Signal { name, .. }) => format!("R.rd({})", py_str(name)),
                _ => {
                    self.unsupported(e.span, &format!("the reference to `{name}`"));
                    "None".into()
                }
            },
            ExprKind::Local { id, name } => Self::lname(id, name),
            ExprKind::Global { name } => format!("R.inp({})", py_str(name)),
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, op, lhs, rhs),
            ExprKind::Compare { first, rest } => {
                let mut s = format!("({}", self.ex(first));
                for c in rest {
                    if !matches!(c.op.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=") {
                        self.unsupported(e.span, &format!("the comparison `{}`", c.op));
                    }
                    s.push_str(&format!(" {} {}", c.op, self.ex(&c.rhs)));
                }
                s.push(')');
                s
            }
            ExprKind::Unary { op, operand } => {
                let v = self.ex(operand);
                match op.as_str() {
                    "not" => format!("(not {v})"),
                    "-" => format!("(-{v})"),
                    "~" => match int_class(&e.ty) {
                        Some(Some((lo, hi))) => format!("rt.wrap(~{v}, {lo}, {hi})"),
                        _ => {
                            self.unsupported(e.span, "bitwise not on this type");
                            "None".into()
                        }
                    },
                    _ => {
                        self.unsupported(e.span, &format!("the operator `{op}`"));
                        "None".into()
                    }
                }
            }
            ExprKind::IfElse {
                cond,
                then,
                otherwise,
            } => format!(
                "({} if {} else {})",
                self.ex(then),
                self.ex(cond),
                self.ex(otherwise)
            ),
            ExprKind::Pipe { lhs, rhs } => {
                let v = self.ex(lhs);
                match &rhs.e {
                    ExprKind::Call { callee, args, .. } => {
                        self.call(rhs, callee, args, Some((v, &lhs.ty)))
                    }
                    _ => {
                        self.unsupported(e.span, "a pipe into something that is not a call");
                        "None".into()
                    }
                }
            }
            ExprKind::Call { callee, args, .. } => self.call(e, callee, args, None),
            ExprKind::Index { base, index, end } => {
                if end.is_some() {
                    self.unsupported(e.span, "a bit slice");
                    return "None".into();
                }
                format!("rt.at({}, {})", self.ex(base), self.ex(index))
            }
            ExprKind::Field { base, name, .. } => {
                if global_root(e) {
                    return format!("R.inp({})", py_str(&dotted(e)));
                }
                if let Type::Tuple { .. } = &base.ty {
                    let k = match name.as_str() {
                        "x" => Some(0),
                        "y" => Some(1),
                        "z" => Some(2),
                        n => n.parse::<usize>().ok(),
                    };
                    if let Some(k) = k {
                        return format!("rt.at({}, {k})", self.ex(base));
                    }
                }
                self.unsupported(e.span, &format!("the field `{name}`"));
                "None".into()
            }
            ExprKind::Cast { operand, .. } => self.cast(e, operand),
            ExprKind::Tuple { items } => {
                let v: Vec<String> = items.iter().map(|x| self.ex(x)).collect();
                if v.len() == 1 {
                    format!("({},)", v[0])
                } else {
                    format!("({})", v.join(", "))
                }
            }
            ExprKind::Array { items } => {
                let v: Vec<String> = items.iter().map(|x| self.ex(x)).collect();
                format!("[{}]", v.join(", "))
            }
            ExprKind::ArrayRepeat { value, count } => {
                format!("rt.repeat({}, {})", self.ex(value), self.ex(count))
            }
            _ => {
                self.unsupported(e.span, "this expression");
                "None".into()
            }
        }
    }

    fn lit(&mut self, e: &Expr, l: &Lit) -> String {
        match l {
            Lit::Int { v, .. } => {
                if is_float(&e.ty) {
                    let x = nirx::parse_num(v);
                    py_float(if is_f32(&e.ty) {
                        f64::from(x as f32)
                    } else {
                        x
                    })
                } else if matches!(e.ty, Type::Bits { .. }) {
                    self.unsupported(e.span, "a bit vector");
                    "None".into()
                } else {
                    v.clone()
                }
            }
            Lit::Real { v, .. } => {
                let x = nirx::parse_num(v);
                py_float(if is_f32(&e.ty) {
                    f64::from(x as f32)
                } else {
                    x
                })
            }
            Lit::Bool { v } => if *v { "True" } else { "False" }.into(),
            Lit::Str { text } => {
                let s = unescape(text);
                if s.contains('{') {
                    self.unsupported(e.span, "string interpolation");
                    "None".into()
                } else {
                    py_str(&s)
                }
            }
            _ => {
                self.unsupported(e.span, "this literal");
                "None".into()
            }
        }
    }

    fn binary(&mut self, e: &Expr, op: &str, l: &Expr, r: &Expr) -> String {
        match op {
            "and" | "or" => return format!("({} {op} {})", self.ex(l), self.ex(r)),
            "in" => {
                if let ExprKind::Range { lo, hi, inclusive } = &r.e {
                    let cmp = if *inclusive { "<=" } else { "<" };
                    let x = self.ex(l);
                    return format!("({} <= {x} {cmp} {})", self.ex(lo), self.ex(hi));
                }
                self.unsupported(e.span, "`in` over something that is not a range");
                return "None".into();
            }
            _ => {}
        }
        let (a, b) = (self.ex(l), self.ex(r));
        if matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=") {
            return format!("({a} {op} {b})");
        }
        self.arith(op, &a, &b, &e.ty, e.span)
    }

    /// `a op b` with the semantics of the interpreter for the result type `ty`.
    fn arith(&mut self, op: &str, a: &str, b: &str, ty: &Type, span: [usize; 2]) -> String {
        if matches!(ty, Type::Bool) {
            return match op {
                "&" | "|" | "^" => format!("({a} {op} {b})"),
                _ => {
                    self.unsupported(span, &format!("the operator `{op}` on booleans"));
                    "None".into()
                }
            };
        }
        if let Some(range) = int_class(ty) {
            let chk = |s: String| match range {
                Some((lo, hi)) => format!("rt.chk({s}, {lo}, {hi})"),
                None => s,
            };
            return match op {
                "+" | "-" | "*" => chk(format!("({a} {op} {b})")),
                "/" => chk(format!("rt.idiv({a}, {b})")),
                "%" => format!("rt.irem({a}, {b})"),
                "**" => chk(format!("rt.ipow({a}, {b})")),
                "&" | "|" | "^" => format!("({a} {op} {b})"),
                "<<" => chk(format!("rt.shl({a}, {b})")),
                ">>" => format!("rt.shr({a}, {b})"),
                "+%" | "-%" | "*%" | "+|" | "-|" | "*|" => {
                    let plain = format!("({a} {} {b})", &op[..1]);
                    match (range, op.ends_with('%')) {
                        (Some((lo, hi)), true) => format!("rt.wrap({plain}, {lo}, {hi})"),
                        (Some((lo, hi)), false) => format!("rt.sat({plain}, {lo}, {hi})"),
                        (None, _) => plain,
                    }
                }
                _ => {
                    self.unsupported(span, &format!("the operator `{op}`"));
                    "None".into()
                }
            };
        }
        if is_float(ty) {
            return match op {
                "+" | "-" | "*" | "+%" | "-%" | "*%" | "+|" | "-|" | "*|" => {
                    format!("({a} {} {b})", &op[..1])
                }
                "/" => format!("rt.fdiv({a}, {b})"),
                "%" => format!("rt.fmod({a}, {b})"),
                "**" => format!("rt.fpow({a}, {b})"),
                _ => {
                    self.unsupported(span, &format!("the operator `{op}` on reals"));
                    "None".into()
                }
            };
        }
        self.unsupported(span, &format!("the operator `{op}` on this type"));
        "None".into()
    }

    fn cast(&mut self, e: &Expr, operand: &Expr) -> String {
        let v = self.ex(operand);
        match &e.ty {
            Type::Num { unit: Some(u), .. } => {
                if matches!(&operand.ty, Type::Num { unit: Some(_), .. }) {
                    v
                } else {
                    let (sc, of) = (nirx::parse_num(&u.scale), nirx::parse_num(&u.offset));
                    format!("float({v} * {} + {})", py_float(sc), py_float(of))
                }
            }
            Type::Num { kind, unit: None } => match int_class(&e.ty) {
                Some(Some((lo, hi))) => format!("rt.cast_int({v}, {lo}, {hi})"),
                Some(None) => format!("rt.cast_int({v}, {}, {})", i128::MIN, i128::MAX),
                None if kind == "f32" => format!("rt.f32(float({v}))"),
                None if is_float(&e.ty) => format!("float({v})"),
                None => {
                    self.unsupported(e.span, &format!("a cast to `{kind}`"));
                    "None".into()
                }
            },
            _ => {
                self.unsupported(e.span, "this cast");
                "None".into()
            }
        }
    }

    // ---- calls ------------------------------------------------------------------------

    fn call(
        &mut self,
        e: &Expr,
        callee: &Expr,
        args: &[neuron_nir::Arg],
        pre: Option<(String, &Type)>,
    ) -> String {
        // `drive(name, value)` names an input, it does not read it.
        if let ExprKind::Global { name } = &callee.e {
            if name == "drive" {
                let target = match args.first().map(|a| &a.value.e) {
                    Some(ExprKind::Global { name }) => Some(py_str(name)),
                    Some(ExprKind::Lit {
                        lit: Lit::Str { text },
                    }) => Some(py_str(&unescape(text))),
                    _ => None,
                };
                return match (target, args.get(1)) {
                    (Some(t), Some(v)) => format!("R.drive({t}, {})", self.ex(&v.value)),
                    _ => {
                        self.unsupported(e.span, "`drive` with this target");
                        "None".into()
                    }
                };
            }
        }
        let mut vals: Vec<(Option<String>, String)> = Vec::new();
        let mut tys: Vec<&Type> = Vec::new();
        if let Some((p, t)) = pre {
            vals.push((None, p));
            tys.push(t);
        }
        for a in args {
            if a.mutable {
                self.unsupported(a.value.span, "a `mut` argument");
            }
            vals.push((a.name.clone(), self.ex(&a.value)));
            tys.push(&a.value.ty);
        }
        match &callee.e {
            ExprKind::Ref { id, name } => match self.defs.get(id).copied() {
                Some(ItemDef::Fn { func }) => self.user_call(e, func, &vals),
                _ => {
                    self.unsupported(e.span, &format!("the call to `{name}`"));
                    "None".into()
                }
            },
            ExprKind::Global { name } => self.builtin(e, name, &vals, &tys),
            ExprKind::Field { base, name, .. } => {
                let is_block = match &base.e {
                    ExprKind::Ref { id, .. } => self.blocks.contains_key(id),
                    _ => matches!(&base.ty, Type::Opaque { name } if block_class(name).is_some()),
                };
                let b = self.ex(base);
                match (name.as_str(), is_block) {
                    ("step" | "update" | "call", true) if vals.len() == 1 => {
                        format!("{b}.step({})", vals[0].1)
                    }
                    ("len", false) if vals.is_empty() => format!("len({b})"),
                    ("abs", false) if vals.is_empty() => format!("abs({b})"),
                    _ => {
                        self.unsupported(e.span, &format!("the method `{name}`"));
                        "None".into()
                    }
                }
            }
            _ => {
                self.unsupported(e.span, "this call");
                "None".into()
            }
        }
    }

    fn user_call(&mut self, e: &Expr, func: &FnDecl, vals: &[(Option<String>, String)]) -> String {
        let mut out = Vec::new();
        let mut pos = vals.iter().filter(|(n, _)| n.is_none()).map(|(_, v)| v);
        for p in func.sig.params.iter().filter(|p| p.name != "self") {
            let named = vals
                .iter()
                .find(|(n, _)| n.as_deref() == Some(p.name.as_str()))
                .map(|(_, v)| v.clone());
            let v = match named.or_else(|| pos.next().cloned()) {
                Some(v) => v,
                None => match &p.default {
                    Some(d) => self.ex(d),
                    None => "None".into(),
                },
            };
            out.push(v);
        }
        let _ = e;
        format!("fn_{}({})", ident(&func.sig.name), out.join(", "))
    }

    fn builtin(
        &mut self,
        e: &Expr,
        name: &str,
        vals: &[(Option<String>, String)],
        tys: &[&Type],
    ) -> String {
        let v: Vec<&str> = vals.iter().map(|(_, s)| s.as_str()).collect();
        if let Some((class, params)) = block_class(name) {
            let mut out = Vec::new();
            for (i, p) in params.iter().enumerate() {
                let named = vals
                    .iter()
                    .find(|(n, _)| n.as_deref() == Some(*p))
                    .map(|(_, s)| s.clone());
                let positional = vals
                    .iter()
                    .filter(|(n, _)| n.is_none())
                    .nth(i)
                    .map(|(_, s)| s.clone());
                match named.or(positional) {
                    Some(s) => out.push(s),
                    None => break,
                }
            }
            return format!("{class}({})", out.join(", "));
        }
        let need = |n: usize, s: &mut Self| -> bool {
            if v.len() < n {
                s.unsupported(e.span, &format!("`{name}` with {} arguments", v.len()));
                false
            } else {
                true
            }
        };
        match name {
            "abs" if need(1, self) => format!("abs({})", v[0]),
            "floor" | "ceil" | "round" if need(1, self) => {
                if matches!(tys.first(), Some(Type::Num { unit: Some(_), .. })) {
                    self.unsupported(e.span, &format!("`{name}` of a quantity"));
                    return "None".into();
                }
                if matches!(tys.first(), Some(t) if int_class(t).is_some()) {
                    return v[0].to_string();
                }
                let f = if name == "round" { "roundf" } else { name };
                format!("rt.{f}({})", v[0])
            }
            "sign" if need(1, self) => format!("rt.sign({})", v[0]),
            "min" if need(2, self) => format!("rt.vmin({}, {})", v[0], v[1]),
            "max" if need(2, self) => format!("rt.vmax({}, {})", v[0], v[1]),
            "clamp" if need(3, self) => format!("rt.clamp({}, {}, {})", v[0], v[1], v[2]),
            "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sqrt" | "exp" | "log" | "log10"
                if need(1, self) =>
            {
                self.libm = true;
                format!("rt.{name}({})", v[0])
            }
            "atan2" | "hypot" if need(2, self) => {
                self.libm = true;
                format!("rt.{name}({}, {})", v[0], v[1])
            }
            "pow" if need(2, self) => {
                self.libm = true;
                format!("rt.fpow({}, {})", v[0], v[1])
            }
            "now" | "elapsed" => "R.elapsed()".into(),
            "advance" if need(1, self) => format!("R.advance({})", v[0]),
            "current_fault" => "R.current_fault()".into(),
            _ => {
                self.unsupported(e.span, &format!("the call to `{name}`"));
                "None".into()
            }
        }
    }

    // ---- statements -------------------------------------------------------------------

    fn target(&mut self, p: &Pattern) -> Option<String> {
        match &p.p {
            PatternKind::Wildcard => Some("_".into()),
            PatternKind::Bind { name, id, .. } => Some(Self::lname(id, name)),
            PatternKind::Tuple { items } => {
                let v: Option<Vec<String>> = items.iter().map(|i| self.target(i)).collect();
                v.map(|v| {
                    if v.len() == 1 {
                        format!("({},)", v[0])
                    } else {
                        format!("({})", v.join(", "))
                    }
                })
            }
            _ => {
                self.unsupported(p.span, "this pattern");
                None
            }
        }
    }

    fn block(&mut self, w: &mut Writer, b: &Block) {
        if !b.contracts.is_empty() {
            self.unsupported(b.contracts[0].span, "a contract inside a block");
        }
        w.indent();
        for s in &b.stmts {
            self.stmt(w, s);
        }
        if b.stmts.is_empty() {
            w.line("pass");
        }
        w.dedent();
    }

    fn write_place(&mut self, w: &mut Writer, place: &Expr, v: &str) {
        match &place.e {
            ExprKind::Local { id, name } => w.line(&format!("{} = {v}", Self::lname(id, name))),
            ExprKind::Ref { id, name } => match self.defs.get(id).copied() {
                Some(ItemDef::Var { .. }) => {
                    let g = format!("g_{}", ident(name));
                    self.assigned.insert(g.clone());
                    w.line(&format!("{g} = {v}"));
                }
                Some(ItemDef::Signal { name, .. }) => {
                    w.line(&format!("R.wr({}, {v})", py_str(name)));
                }
                _ => self.unsupported(place.span, &format!("an assignment to `{name}`")),
            },
            ExprKind::Index {
                base,
                index,
                end: None,
            } => {
                let (b, i) = (self.ex(base), self.ex(index));
                w.line(&format!("rt.setat({b}, {i}, {v})"));
            }
            _ => self.unsupported(place.span, "an assignment to this place"),
        }
    }

    fn stmt(&mut self, w: &mut Writer, s: &Stmt) {
        let mark = w.mark();
        match &s.s {
            StmtKind::Let { pattern, value, .. } => {
                let mut v = self.ex(value);
                if needs_deep(&value.ty) {
                    v = format!("rt.deep({v})");
                }
                if let Some(t) = self.target(pattern) {
                    w.line(&format!("{t} = {v}"));
                }
            }
            StmtKind::Assign {
                place,
                op,
                value,
                when,
            } => {
                if let Some(c) = when {
                    let c = self.ex(c);
                    w.line(&format!("if {c}:"));
                    w.indent();
                }
                let mut rhs = self.ex(value);
                let new = if op == "=" {
                    if needs_deep(&value.ty) {
                        rhs = format!("rt.deep({rhs})");
                    }
                    rhs
                } else {
                    if rhs.contains('(') {
                        let t = self.next_tmp();
                        w.line(&format!("_t{t} = {rhs}"));
                        rhs = format!("_t{t}");
                    }
                    let cur = self.ex(place);
                    let bop = op.strip_suffix('=').unwrap_or(op);
                    let r = self.arith(bop, &cur, &rhs, &place.ty, s.span);
                    if is_f32(&place.ty) {
                        format!("rt.f32({r})")
                    } else {
                        r
                    }
                };
                self.write_place(w, place, &new);
                if when.is_some() {
                    w.dedent();
                }
            }
            StmtKind::Expr { expr } => {
                let v = self.ex(expr);
                w.line(&v);
            }
            StmtKind::Pass | StmtKind::Rule { .. } => w.line("pass"),
            StmtKind::Return { value } => match value {
                Some(v) => {
                    let v = self.ex(v);
                    w.line(&format!("return {v}"));
                }
                None => w.line("return"),
            },
            StmtKind::Break => w.line("break"),
            StmtKind::Continue => w.line("continue"),
            StmtKind::Fault { name, .. } => w.line(&format!("raise rt.Fault({})", py_str(name))),
            StmtKind::Assert { cond, msg } => {
                let c = self.ex(cond);
                let text = msg
                    .as_deref()
                    .map(|m| m.trim_matches('"').to_string())
                    .unwrap_or_default();
                w.line(&format!("R.check({c}, {})", py_str(&text)));
            }
            StmtKind::Emit { event, args } => {
                if !args.is_empty() {
                    self.unsupported(s.span, "an event with a payload");
                }
                w.line(&format!("R.emit_event({})", py_str(event)));
            }
            StmtKind::Coil { set, place, when } => {
                let c = self.ex(when);
                w.line(&format!("if {c}:"));
                w.indent();
                self.write_place(w, place, if *set { "True" } else { "False" });
                w.dedent();
            }
            StmtKind::If {
                branches,
                otherwise,
            } => {
                for (k, br) in branches.iter().enumerate() {
                    let c = self.ex(&br.cond);
                    w.line(&format!("{} {c}:", if k == 0 { "if" } else { "elif" }));
                    self.block(w, &br.body);
                }
                if let Some(o) = otherwise {
                    w.line("else:");
                    self.block(w, o);
                }
            }
            StmtKind::For {
                pattern,
                iter,
                body,
                ..
            } => {
                let it = match &iter.e {
                    ExprKind::Range { lo, hi, inclusive } => format!(
                        "rt.rng({}, {}, {})",
                        self.ex(lo),
                        self.ex(hi),
                        if *inclusive { "True" } else { "False" }
                    ),
                    _ => format!("list({})", self.ex(iter)),
                };
                let t = self.target(pattern).unwrap_or_else(|| "_".into());
                w.line(&format!("for {t} in {it}:"));
                self.block(w, body);
            }
            StmtKind::While {
                pattern,
                cond,
                max,
                body,
            } => {
                if pattern.is_some() {
                    self.unsupported(s.span, "`while let`");
                }
                let k = self.next_tmp();
                let bound = match max {
                    Some(m) => format!("rt.bound({})", self.ex(m)),
                    None => "rt.LOOP_CAP".into(),
                };
                let c = self.ex(cond);
                w.line(&format!("_n{k} = 0"));
                w.line(&format!("_b{k} = {bound}"));
                w.line(&format!("while _n{k} < _b{k}:"));
                w.indent();
                w.line(&format!("if not {c}:"));
                w.indent();
                w.line("break");
                w.dedent();
                w.line(&format!("_n{k} += 1"));
                w.dedent();
                self.body_inline(w, body);
            }
            StmtKind::Loop { max, body } => {
                let bound = match max {
                    Some(m) => format!("rt.bound({})", self.ex(m)),
                    None => "rt.LOOP_CAP".into(),
                };
                w.line(&format!("for _ in range({bound}):"));
                self.block(w, body);
            }
            StmtKind::Atomic { body } => {
                for st in &body.stmts {
                    self.stmt(w, st);
                }
            }
            StmtKind::Match { .. } => self.unsupported(s.span, "`match`"),
            _ => self.unsupported(s.span, "this statement"),
        }
        w.map(mark, &self.file.clone(), s.span, None);
    }

    /// Statements of `b` one level deeper than the current line (the body of a `while`
    /// whose header was written by the caller).
    fn body_inline(&mut self, w: &mut Writer, b: &Block) {
        w.indent();
        for st in &b.stmts {
            self.stmt(w, st);
        }
        w.dedent();
    }

    // ---- definitions ------------------------------------------------------------------

    /// Writes `header` and the body made by `f`, with the `global` line it needs.
    fn def(&mut self, w: &mut Writer, header: &str, f: impl FnOnce(&mut Self, &mut Writer)) {
        self.assigned.clear();
        let mut bw = Writer::new("    ");
        bw.indent();
        f(self, &mut bw);
        let mut hw = Writer::new("    ");
        hw.indent();
        if !self.assigned.is_empty() {
            let g: Vec<&str> = self.assigned.iter().map(String::as_str).collect();
            hw.line(&format!("global {}", g.join(", ")));
        }
        hw.append(bw);
        w.line(header);
        w.append(hw);
        w.line("");
        w.line("");
    }

    fn body_def(&mut self, w: &mut Writer, header: &str, b: &Block) {
        self.def(w, header, |s, bw| {
            if !b.contracts.is_empty() {
                s.unsupported(b.contracts[0].span, "a contract inside a block");
            }
            for st in &b.stmts {
                s.stmt(bw, st);
            }
            if b.stmts.is_empty() {
                bw.line("pass");
            }
        });
    }

    fn function(&mut self, w: &mut Writer, func: &FnDecl) {
        if !func.sig.generics.is_empty() {
            self.unsupported(func.sig.span, "a generic function");
        }
        let mut params = Vec::new();
        for p in func.sig.params.iter() {
            self.param(p, &mut params);
        }
        w.line("@rt.fn");
        let header = format!("def fn_{}({}):", ident(&func.sig.name), params.join(", "));
        self.def(w, &header, |s, bw| {
            for c in &func.contracts {
                if c.kind == "requires" {
                    let cond = s.ex(&c.cond);
                    bw.line(&format!("if not {cond}:"));
                    bw.indent();
                    bw.line("raise rt.Fault(\"ContractViolation\")");
                    bw.dedent();
                } else {
                    s.unsupported(c.span, &format!("a `{}` contract", c.kind));
                }
            }
            match &func.body {
                FnBody::Block { block } => {
                    if !block.contracts.is_empty() {
                        s.unsupported(block.contracts[0].span, "a contract inside a block");
                    }
                    for st in &block.stmts {
                        s.stmt(bw, st);
                    }
                    if block.stmts.is_empty() {
                        bw.line("pass");
                    }
                }
                FnBody::Expr { expr } => {
                    let v = s.ex(expr);
                    bw.line(&format!("return {v}"));
                }
            }
        });
    }

    fn param(&mut self, p: &Param, out: &mut Vec<String>) {
        if p.name == "self" {
            self.unsupported([0, 0], "a method");
            return;
        }
        if p.mutable {
            self.unsupported([0, 0], &format!("the `mut` parameter `{}`", p.name));
        }
        if let Some(id) = &p.id {
            out.push(Self::lname(id, &p.name));
        }
    }

    /// Python for a duration in seconds as nanoseconds.
    fn ns(&mut self, e: &Expr) -> String {
        format!("rt.ns({})", self.ex(e))
    }

    fn module(&mut self, module: &'a Module) -> Writer {
        let mut w = Writer::new("    ");
        let name = ident(&module.name);
        w.line(&format!(
            "# Generated by `neuron emit python` from {}. Do not edit.",
            module
                .file
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&module.file)
        ));
        w.line(&format!(
            "# python {name}.py --test | --trace 3s | --realtime 10s"
        ));
        w.line("import sys");
        w.line("");
        w.line("import neuron_rt as rt");
        w.line("");
        w.line("R = None  # the running program, made by _boot()");
        w.line("");
        // Module state: placeholders, filled by _boot.
        for it in &module.items {
            if let ItemDef::Var { var } = &it.def {
                self.module_vars.push(format!("g_{}", ident(&var.name)));
            }
        }
        for g in self.module_vars.clone() {
            w.line(&format!("{g} = None"));
        }
        w.line("");
        w.line("");
        // Functions first, then constants (a constant may call a function).
        for it in &module.items {
            if let ItemDef::Fn { func } = &it.def {
                let m = w.mark();
                self.function(&mut w, func);
                w.map(
                    m,
                    &self.file.clone(),
                    it.span,
                    Some(format!("fn:{}", func.sig.name)),
                );
                self.fid.exact();
            }
        }
        for it in &module.items {
            if let ItemDef::Const { name, value, .. } = &it.def {
                let v = self.ex(value);
                w.line(&format!("g_{} = {v}", ident(name)));
                self.fid.exact();
            }
        }
        w.line("");
        w.line("");

        // Activities and tests.
        let mut registrations: Vec<String> = Vec::new();
        let mut tests: Vec<(String, String)> = Vec::new();
        for (k, it) in module.items.iter().enumerate() {
            let m = w.mark();
            match &it.def {
                ItemDef::Task {
                    name,
                    trigger,
                    attrs,
                    body,
                } => {
                    let fname = format!("_task_{}", ident(name));
                    self.body_def(&mut w, &format!("def {fname}():"), body);
                    let mut extra = String::new();
                    for a in attrs {
                        match a {
                            TaskAttr::Prio { value } => {
                                extra.push_str(&format!(", prio={}", self.ex(value)));
                            }
                            TaskAttr::Deadline { value } => {
                                extra.push_str(&format!(", deadline={}", self.ns(value)));
                            }
                            _ => {}
                        }
                    }
                    match trigger {
                        Trigger::Every { period, offset } => {
                            let p = self.ns(period);
                            let o = match offset {
                                Some(o) => self.ns(o),
                                None => "0".into(),
                            };
                            registrations.push(format!(
                                "R.task({}, {fname}, {p}, {o}{extra})",
                                py_str(name)
                            ));
                        }
                        Trigger::On { event } => registrations.push(format!(
                            "R.task_on({}, {fname}, {}{extra})",
                            py_str(name),
                            py_str(&dotted(event))
                        )),
                    }
                    self.fid.exact();
                }
                ItemDef::Rule { rule } => {
                    let fname = format!("_rule_{k}");
                    self.body_def(&mut w, &format!("def {fname}():"), &rule.body);
                    let cond = self.ex(&rule.cond);
                    let hold = match &rule.hold {
                        Some(h) => format!("lambda: {}", self.ex(h)),
                        None => "None".into(),
                    };
                    registrations.push(format!("R.rule(lambda: {cond}, {hold}, {fname})"));
                    self.fid.exact();
                }
                ItemDef::Safety { members } => {
                    let mut rate = "10000000".to_string();
                    let mut ms: Vec<String> = Vec::new();
                    for (j, mem) in members.iter().enumerate() {
                        match &mem.def {
                            SafetyDef::Rate { value } => rate = self.ns(value),
                            SafetyDef::Watchdog { .. } => self.watchdog = Some(mem.span),
                            SafetyDef::SafeState { body } => {
                                let f = format!("_safe_{k}_{j}");
                                self.body_def(&mut w, &format!("def {f}():"), body);
                                ms.push(format!("(\"safe\", {f})"));
                            }
                            SafetyDef::Rule { rule } => {
                                let f = format!("_safe_{k}_{j}");
                                self.body_def(&mut w, &format!("def {f}():"), &rule.body);
                                let cond = self.ex(&rule.cond);
                                let hold = match &rule.hold {
                                    Some(h) => format!("lambda: {}", self.ex(h)),
                                    None => "None".into(),
                                };
                                ms.push(format!("(\"rule\", lambda: {cond}, {hold}, {f})"));
                            }
                            SafetyDef::OnFault { fault, body } => {
                                let f = format!("_safe_{k}_{j}");
                                self.body_def(&mut w, &format!("def {f}():"), body);
                                let n = fault.as_deref().map_or("None".into(), py_str);
                                ms.push(format!("(\"fault\", {n}, {f})"));
                            }
                            SafetyDef::Reset { cond } => {
                                let c = self.ex(cond);
                                ms.push(format!("(\"reset\", lambda: {c})"));
                            }
                        }
                    }
                    registrations.push(format!("R.safety({rate}, [{}])", ms.join(", ")));
                    self.fid.exact();
                }
                ItemDef::Test { name, body } => {
                    let fname = format!("_test_{k}");
                    self.body_def(&mut w, &format!("def {fname}():"), body);
                    tests.push((nirx::unquote(name), fname));
                    self.fid.exact();
                }
                _ => {}
            }
            w.map(m, &self.file.clone(), it.span, None);
        }

        // _boot: state first, then activities, both in source order.
        let mut boot = Writer::new("    ");
        boot.indent();
        if !self.module_vars.is_empty() {
            boot.line(&format!("global R, {}", self.module_vars.join(", ")));
        } else {
            boot.line("global R");
        }
        boot.line("R = rt.Runtime()");
        for it in &module.items {
            let m = boot.mark();
            match &it.def {
                ItemDef::Var { var } => {
                    if var.at.is_some() {
                        self.unsupported(var.span, "a variable placed at an address");
                    }
                    let v = match &var.init {
                        Some(e) => {
                            let s = self.ex(e);
                            if needs_deep(&e.ty) {
                                format!("rt.deep({s})")
                            } else {
                                s
                            }
                        }
                        None => self.default_py(&var.vty, var.span),
                    };
                    boot.line(&format!("g_{} = {v}", ident(&var.name)));
                    self.fid.exact();
                }
                ItemDef::Signal {
                    name, vty, init, ..
                } => {
                    let ty = vty.clone().unwrap_or(Type::Num {
                        kind: "f64".into(),
                        unit: None,
                    });
                    let v = match init {
                        Some(e) => self.ex(e),
                        None => self.default_py(&ty, it.span),
                    };
                    let unit = match &ty {
                        Type::Num { unit: Some(u), .. } => {
                            let sc = nirx::parse_num(&u.scale);
                            format!(
                                ", ({}, {}, {})",
                                py_str(&u.text),
                                py_float(if sc == 0.0 { 1.0 } else { sc }),
                                py_float(nirx::parse_num(&u.offset))
                            )
                        }
                        _ => String::new(),
                    };
                    let v = if unit.is_empty() {
                        v
                    } else {
                        format!("float({v})")
                    };
                    boot.line(&format!("R.signal({}, {v}{unit})", py_str(name)));
                    self.fid.exact();
                }
                _ => {}
            }
            boot.map(m, &self.file.clone(), it.span, None);
        }
        for r in &registrations {
            boot.line(r);
        }
        w.line("def _boot():");
        w.append(boot);
        w.line("");
        w.line("");
        w.line("_TESTS = [");
        for (n, f) in &tests {
            w.line(&format!("    ({}, {f}),", py_str(n)));
        }
        w.line("]");
        w.line("");
        w.line("");
        w.line("if __name__ == \"__main__\":");
        w.line("    sys.exit(rt.main(sys.modules[__name__], sys.argv[1:]))");

        // Items that have no translation.
        for it in &module.items {
            match &it.def {
                ItemDef::Use { path, .. } => {
                    if path.first().is_some_and(|p| p != "std") && !path.is_empty() {
                        self.unsupported(it.span, "a `use` of another module");
                    }
                }
                ItemDef::Const { .. }
                | ItemDef::Var { .. }
                | ItemDef::Signal { .. }
                | ItemDef::Fn { .. }
                | ItemDef::Task { .. }
                | ItemDef::Rule { .. }
                | ItemDef::Safety { .. }
                | ItemDef::Test { .. }
                | ItemDef::Unit { .. }
                | ItemDef::Alias { .. }
                | ItemDef::Event { .. } => {}
                ItemDef::Verify { .. } => self.fid.record(
                    &self.file.clone(),
                    it.span,
                    Class::Equivalent,
                    "verify",
                    "properties are checked by `neuron verify`; the Python program does not check them at run time (the interpreter does not either)",
                ),
                other => {
                    let what = match other {
                        ItemDef::Struct { .. } => "a `struct`",
                        ItemDef::Enum { .. } => "an `enum`",
                        ItemDef::Class { .. } => "a `class`",
                        ItemDef::Interface { .. } => "an `interface`",
                        ItemDef::Fsm { .. } => "a state machine",
                        ItemDef::Sequence { .. } => "a `sequence`",
                        ItemDef::Interrupt { .. } => "an interrupt handler",
                        ItemDef::Io { .. } => "an `io` binding",
                        _ => "this item",
                    };
                    self.unsupported(it.span, what);
                }
            }
        }
        if let Some(span) = self.watchdog {
            self.fid.record(
                &self.file.clone(),
                span,
                Class::Equivalent,
                "watchdog",
                "the runtime does not enforce the watchdog time (the interpreter does not either)",
            );
        }
        if self.libm {
            self.fid.record(
                &self.file.clone(),
                [0, 0],
                Class::Equivalent,
                "math functions",
                "sin, exp, pow and the like use the C library of the Python that runs the program; the last digit can differ from the interpreter",
            );
        }
        self.fid.record(
            &self.file.clone(),
            [0, 0],
            Class::Equivalent,
            "runtime",
            "simulated time is exact (integer nanoseconds); --realtime follows the wall clock and the instants hold only while the host keeps up; the interpreter's statement budget per activation (fault LoopBound) is not enforced",
        );
        let _ = self.nir;
        w
    }
}

impl Emitter for Python {
    fn manifest(&self) -> Manifest {
        Manifest {
            name: "python",
            language: "Python 3",
            dialects: &["host"],
            profiles: &[],
            constructs: CONSTRUCTS,
        }
    }

    fn emit(&self, nir: &Nir, _opts: &EmitOptions) -> Result<Package, Vec<Diag>> {
        let mods: Vec<usize> = nir
            .modules
            .iter()
            .enumerate()
            .filter(|(_, m)| m.dialect == "host")
            .map(|(i, _)| i)
            .collect();
        if mods.is_empty() {
            return Err(vec![Diag {
                code: "NE0610",
                message: "the program has no `host` module to translate".into(),
                help: Some(
                    "the python emitter reads modules declared `module name for host`".into(),
                ),
                file: nir
                    .modules
                    .first()
                    .map(|m| m.file.clone())
                    .unwrap_or_default(),
                span: [0, 0],
            }]);
        }
        let mut fid = Fidelity::new("emit-python", "host.python");
        let mut pkg = Package::new(fid.clone());
        let mut diags = Vec::new();
        let mut wiring = String::from(
            "# Running the Python program\n\nStandard library only, Python 3.10 or newer. `neuron_rt.py` must sit next to the generated modules.\n\n",
        );
        for m in mods {
            let module = &nir.modules[m];
            let mut l = Lower::new(nir, m);
            let w = l.module(module);
            diags.append(&mut l.diags);
            let file = format!("{}.py", ident(&module.name));
            wiring.push_str(&format!(
                "## `{file}`\n\n- `python {file} --test` runs the `test` blocks in simulated time and exits nonzero when one fails.\n- `python {file} --trace 3s` prints the first 3 s of the run in the text format of `neuron run`.\n- `python {file} --realtime 10s` runs against the wall clock for 10 s and prints each event as it happens.\n- `python {file} --ros2` is **not supported**: the emitter does not write a ROS 2 wrapper, and nothing requires `rclpy`.\n\n"
            ));
            pkg.add(&file, w);
            fid.merge(l.fid);
        }
        if !diags.is_empty() {
            return Err(diags);
        }
        pkg.add_text("neuron_rt.py", RUNTIME.to_string());
        pkg.wiring = wiring;
        pkg.fidelity = fid;
        Ok(pkg)
    }
}
