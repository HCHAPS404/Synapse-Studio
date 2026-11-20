//! Evaluation of expressions, statements and patterns.

use crate::value::{parse_num, round9, Closure, Geo, UnitInfo, Value};
use crate::{fault, int_kind, Interp, Stop, R};
use neuron_nir::{Block, Expr, ExprKind, ItemDef, Lit, Pattern, PatternKind, Stmt, StmtKind, Type};
use std::rc::Rc;

/// Largest array or iteration count the interpreter builds.
pub(crate) const MAX_ARRAY: usize = 1 << 20;

/// Statements one activation may execute before it is stopped as a runaway loop
/// (`fault LoopBound`).
const STEP_BUDGET: u64 = 5_000_000;

/// Applies the unit written after a geometry value (`xyz(1, 2, 3) m`): the components
/// were taken as written, so they are scaled to mm (lengths) or deg (angles).
fn rescale_geo(g: &Geo, unit: &str) -> Geo {
    let scale = neuron_sema::units::lookup_unit(unit).map_or(1.0, |u| u.scale.to_f64());
    let len = |v: &[f64]| -> Vec<f64> { v.iter().map(|x| round9(x * scale * 1000.0)).collect() };
    let ang =
        |v: &[f64]| -> Vec<f64> { v.iter().map(|x| round9((x * scale).to_degrees())).collect() };
    match g {
        Geo::Vec { kind, v } => Geo::Vec {
            kind: kind.clone(),
            v: if kind == "rpy" { ang(v) } else { len(v) },
        },
        Geo::Joints(v) => Geo::Joints(ang(v)),
        Geo::Point { frame, v } => {
            let l = len(v);
            Geo::Point {
                frame: frame.clone(),
                v: [l[0], l[1], l[2]],
            }
        }
        other => other.clone(),
    }
}

/// Range of an integer type.
pub(crate) fn int_range(kind: &str) -> Option<(i128, i128)> {
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

fn wrap_to(kind: &str, v: i128) -> i128 {
    match int_range(kind) {
        Some((lo, hi)) => (v - lo).rem_euclid(hi - lo + 1) + lo,
        None => v,
    }
}

fn saturate_to(kind: &str, v: i128) -> i128 {
    match int_range(kind) {
        Some((lo, hi)) => v.clamp(lo, hi),
        None => v,
    }
}

fn mask_bits(v: i128, width: u32, signed: bool) -> i128 {
    if width == 0 || width >= 127 {
        return v;
    }
    let m = (1i128 << width) - 1;
    let u = v & m;
    if signed && ((u >> (width - 1)) & 1) == 1 {
        u - (1i128 << width)
    } else {
        u
    }
}

pub(crate) fn bits_width(ty: &Type) -> Option<u32> {
    match ty {
        Type::Bits { width, .. } => Some(*width),
        _ => None,
    }
}

/// Dotted text of a name or field chain (`button.falling`), empty for anything else.
pub(crate) fn dotted(e: &Expr) -> String {
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

fn unescape(text: &str) -> String {
    let t = text.trim();
    let t = t.strip_prefix('b').unwrap_or(t);
    let t = t.strip_prefix('r').unwrap_or(t);
    let inner = t
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| t.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
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

/// Value of a bit-vector literal such as `8'b1010_0000` or `16'hBEEF`.
fn bits_literal(text: &str) -> Value {
    let t: String = text.chars().filter(|c| *c != '_').collect();
    let (digits, radix) = match t.split_once('\'') {
        Some((_, rest)) => {
            let mut cs = rest.chars();
            let r = match cs.next() {
                Some('b') | Some('B') => 2,
                Some('h') | Some('H') => 16,
                Some('o') | Some('O') => 8,
                _ => 10,
            };
            (cs.as_str().to_string(), r)
        }
        None => (t.clone(), 10),
    };
    Value::Int(i128::from_str_radix(&digits, radix).unwrap_or(0))
}

impl Interp {
    pub(crate) fn unit_info(&mut self, u: &neuron_nir::Unit) -> Rc<UnitInfo> {
        let key = format!("{}|{}|{}", u.text, u.scale, u.offset);
        Rc::clone(
            self.unit_cache
                .entry(key)
                .or_insert_with(|| Rc::new(UnitInfo::from_nir(u))),
        )
    }

    /// Gives a value the representation its type asks for.
    pub(crate) fn fit(&mut self, v: Value, ty: &Type) -> Value {
        match ty {
            Type::Num { unit: Some(u), .. } => match v {
                Value::Int(_) | Value::Real(_) | Value::Qty(..) => {
                    let si = v.as_f64().unwrap_or(0.0);
                    let info = self.unit_info(u);
                    Value::Qty(si, info)
                }
                other => other,
            },
            Type::Num { kind, unit: None } => match v {
                Value::Qty(si, _) if kind != "unknown" => Value::Real(si),
                Value::Int(i) if matches!(kind.as_str(), "f32" | "f64" | "real") => {
                    Value::Real(i as f64)
                }
                Value::Real(r) if kind == "f32" => Value::Real(f64::from(r as f32)),
                other => other,
            },
            _ => v,
        }
    }

    /// Evaluates an expression used as a value (operand, condition, right-hand side): an
    /// `io` input binding reads the input's current value, like a bare `until x`. Method
    /// calls and properties go through [`Self::ev`] and keep the binding.
    pub(crate) fn ev_val(&mut self, e: &Expr) -> R<Value> {
        match self.ev(e)? {
            Value::Io(k) => {
                let kind = self.io.get(&k).map(|s| s.kind.clone()).unwrap_or_default();
                if Self::io_is_input(&kind) {
                    Ok(self.io_read(&k))
                } else {
                    Ok(Value::Io(k))
                }
            }
            v => Ok(v),
        }
    }

    pub(crate) fn ev(&mut self, e: &Expr) -> R<Value> {
        let v = self.ev_raw(e)?;
        Ok(self.fit(v, &e.ty))
    }

    fn ev_raw(&mut self, e: &Expr) -> R<Value> {
        match &e.e {
            ExprKind::Lit { lit } => Ok(self.lit(lit)),
            ExprKind::Quantity { value, si, unit } => {
                if si.is_none() && matches!(value.e, ExprKind::Call { .. }) {
                    let v = self.ev(value)?;
                    if let Value::Geo(g) = &v {
                        return Ok(Value::Geo(Rc::new(rescale_geo(g, &unit.text))));
                    }
                    if let (Some(x), Type::Num { unit: Some(u), .. }) = (v.as_f64(), &e.ty) {
                        let info = self.unit_info(u);
                        let si = info.to_si(x);
                        return Ok(Value::Qty(si, info));
                    }
                    return Ok(v);
                }
                if let (Some(s), Type::Num { unit: Some(u), .. }) = (si, &e.ty) {
                    let info = self.unit_info(u);
                    return Ok(Value::Qty(parse_num(s), info));
                }
                if let Some(s) = si {
                    return Ok(Value::Real(parse_num(s)));
                }
                let v = self.ev(value)?;
                if let (Some(x), Type::Num { unit: Some(u), .. }) = (v.as_f64(), &e.ty) {
                    let info = self.unit_info(u);
                    let si = info.to_si(x);
                    return Ok(Value::Qty(si, info));
                }
                Ok(v)
            }
            ExprKind::Ref { id, name } => self.ev_ref(id, name),
            ExprKind::Local { id, .. } => Ok(self.locals.get(id).cloned().unwrap_or(Value::Unit)),
            ExprKind::Global { name } => Ok(self
                .input_get(name)
                .unwrap_or_else(|| Value::Opaque(name.clone()))),
            ExprKind::Module { id, .. } => Ok(Value::Module(id.clone())),
            ExprKind::State { state, .. } => Ok(Value::Str(state.clone())),
            ExprKind::SelfValue => Ok(self.locals.get("self").cloned().unwrap_or(Value::Unit)),
            ExprKind::Result => Ok(self.locals.get("result").cloned().unwrap_or(Value::Unit)),
            ExprKind::Binary { op, lhs, rhs } => self.ev_binary(e, op, lhs, rhs),
            ExprKind::Compare { first, rest } => {
                let mut left = self.ev_val(first)?;
                for c in rest {
                    let right = self.ev_val(&c.rhs)?;
                    let ok = self.binop(&c.op, left.clone(), right.clone(), &Type::Bool)?;
                    if !ok.truthy() {
                        return Ok(Value::Bool(false));
                    }
                    left = right;
                }
                Ok(Value::Bool(true))
            }
            ExprKind::Unary { op, operand } => {
                let v = self.ev_val(operand)?;
                self.unary(op, v, &e.ty)
            }
            ExprKind::Range { lo, hi, inclusive } => {
                let a = self.ev(lo)?;
                let b = self.ev(hi)?;
                Ok(Value::Range(Box::new(a), Box::new(b), *inclusive))
            }
            ExprKind::Map { from, to } => {
                let a = self.ev(from)?;
                let b = self.ev(to)?;
                Ok(Value::Map(Box::new(a), Box::new(b)))
            }
            ExprKind::IfElse {
                cond,
                then,
                otherwise,
            } => {
                if self.ev_val(cond)?.truthy() {
                    self.ev(then)
                } else {
                    self.ev(otherwise)
                }
            }
            ExprKind::Pipe { lhs, rhs } => {
                let v = self.ev(lhs)?;
                self.ev_pipe(rhs, v)
            }
            ExprKind::Call {
                callee,
                args,
                target,
            } => self.ev_call(e, callee, args, target.as_ref(), None),
            ExprKind::Index { base, index, end } => {
                let b = self.ev(base)?;
                let i = self.ev(index)?;
                match (b, i, end) {
                    (Value::Array(v) | Value::Tuple(v), Value::Int(k), None) => usize::try_from(k)
                        .ok()
                        .and_then(|k| v.get(k).cloned())
                        .ok_or_else(|| fault("ContractViolation")),
                    (Value::Int(x), Value::Int(hi), Some(lo_e)) => {
                        let lo = self.ev(lo_e)?.as_int().unwrap_or(0);
                        let width =
                            u32::try_from(hi.saturating_sub(lo).saturating_add(1)).unwrap_or(1);
                        let shifted = x.checked_shr(u32::try_from(lo).unwrap_or(0)).unwrap_or(0);
                        Ok(Value::Int(mask_bits(shifted, width, false)))
                    }
                    (Value::Int(x), Value::Int(k), None) => Ok(Value::Int(
                        x.checked_shr(u32::try_from(k).unwrap_or(0)).unwrap_or(0) & 1,
                    )),
                    _ => Err(fault("ContractViolation")),
                }
            }
            ExprKind::Field { base, name, .. } => self.ev_field(e, base, name),
            ExprKind::Try { operand } => match self.ev(operand)? {
                Value::Res(Ok(b)) => Ok(*b),
                Value::Res(Err(x)) => Err(Stop::Return(Value::Res(Err(x)))),
                Value::Opt(Some(b)) => Ok(*b),
                Value::Opt(None) => Err(Stop::Return(Value::Opt(None))),
                other => Ok(other),
            },
            ExprKind::Cast { operand, .. } => {
                let v = self.ev(operand)?;
                self.cast(v, &e.ty)
            }
            ExprKind::Tuple { items } => {
                let mut out = Vec::new();
                for x in items {
                    out.push(self.ev(x)?);
                }
                Ok(Value::Tuple(out))
            }
            ExprKind::Array { items } => {
                let mut out = Vec::new();
                for x in items {
                    out.push(self.ev(x)?);
                }
                Ok(Value::Array(out))
            }
            ExprKind::ArrayRepeat { value, count } => {
                let v = self.ev(value)?;
                let n = usize::try_from(self.ev(count)?.as_int().unwrap_or(0)).unwrap_or(0);
                if n > MAX_ARRAY {
                    return Err(fault("ContractViolation"));
                }
                Ok(Value::Array((0..n).map(|_| v.deep()).collect()))
            }
            ExprKind::Lambda { params, body } => Ok(Value::Lambda(Rc::new(Closure {
                params: params
                    .iter()
                    .map(|p| (p.name.clone(), p.id.clone()))
                    .collect(),
                body: (**body).clone(),
                captured: self.locals.clone(),
            }))),
            ExprKind::Old { operand } => {
                let key = format!("$old{}-{}", operand.span[0], operand.span[1]);
                match self.locals.get(&key) {
                    Some(v) => Ok(v.clone()),
                    None => self.ev(operand),
                }
            }
            ExprKind::Run { action } => {
                let saved = self.in_run;
                self.in_run = true;
                let v = self.ev(action);
                self.in_run = saved;
                match v? {
                    h @ Value::Handle(_) => Ok(h),
                    _ => Ok(self.finished_handle()),
                }
            }
        }
    }

    fn lit(&mut self, l: &Lit) -> Value {
        match l {
            Lit::Int { v, .. } => {
                Value::Int(v.parse::<i128>().unwrap_or_else(|_| parse_num(v) as i128))
            }
            Lit::Real { v, .. } => Value::Real(parse_num(v)),
            Lit::Str { text } => Value::Str(self.interpolate(&unescape(text))),
            Lit::RawStr { text } | Lit::ByteStr { text } | Lit::Char { text } => {
                Value::Str(unescape(text))
            }
            Lit::Bool { v } => Value::Bool(*v),
            Lit::None => Value::Opt(None),
            Lit::Iec { text } => Value::Opaque(text.clone()),
            Lit::Bits { text } => bits_literal(text),
            Lit::Ip { text } => Value::Str(text.clone()),
        }
    }

    /// Replaces `{name}` and `{name:.N}` in a string with the value of `name`.
    fn interpolate(&mut self, s: &str) -> String {
        if !s.contains('{') {
            return s.to_string();
        }
        let mut out = String::new();
        let mut rest = s;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            if let Some(stripped) = after.strip_prefix('{') {
                out.push('{');
                rest = stripped;
                continue;
            }
            let Some(end) = after.find('}') else {
                out.push_str(&rest[start..]);
                return out;
            };
            let spec = &after[..end];
            let (name, fmt) = spec.split_once(':').unwrap_or((spec, ""));
            match self.lookup_name(name.trim()) {
                Some(v) => out.push_str(&format_value(&v, fmt)),
                None => {
                    out.push('{');
                    out.push_str(spec);
                    out.push('}');
                }
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        out
    }

    fn lookup_name(&mut self, name: &str) -> Option<Value> {
        if name == "fault.name" {
            return Some(Value::Str(self.fault.clone().unwrap_or_default()));
        }
        let first = name.split('.').next().unwrap_or(name);
        let mut v = if let Some(id) = self.names.get(first) {
            self.locals.get(id).cloned()?
        } else {
            let &(m, i) = self
                .idx
                .by_name
                .get(&(self.cur_module, first.to_string()))?;
            let nir = Rc::clone(&self.nir);
            let id = nir.modules[m].items[i].id.clone();
            self.ev_ref(&id, first).ok()?
        };
        for part in name.split('.').skip(1) {
            v = match &v {
                Value::Obj(rc) => rc.borrow().get(part).cloned()?,
                _ => return None,
            };
        }
        Some(v)
    }

    pub(crate) fn ev_ref(&mut self, id: &str, name: &str) -> R<Value> {
        if let Some(v) = self.globals.get(id) {
            return Ok(v.clone());
        }
        if self.signals.contains_key(id) {
            return Ok(self.read_signal(id));
        }
        let Some(&(m, i)) = self.idx.items.get(id) else {
            return Ok(Value::Opaque(name.to_string()));
        };
        let nir = Rc::clone(&self.nir);
        let item = &nir.modules[m].items[i];
        match &item.def {
            ItemDef::Const { value, .. } => {
                if !self.consts_busy.insert(id.to_string()) {
                    return Err(fault("ContractViolation"));
                }
                let v = self.ev(value);
                self.consts_busy.remove(id);
                let v = v?;
                self.globals.insert(id.to_string(), v.clone());
                Ok(v)
            }
            ItemDef::Geometry {
                kind,
                name: gname,
                value,
                ..
            } => {
                if !self.consts_busy.insert(id.to_string()) {
                    return Err(fault("ContractViolation"));
                }
                let v = self.ev(value);
                self.consts_busy.remove(id);
                let mut v = v?;
                if kind == "frame" {
                    if let Value::Geo(g) = &v {
                        if let Geo::Frame { origin, rot, .. } = &**g {
                            v = Value::Geo(Rc::new(Geo::Frame {
                                name: gname.clone(),
                                origin: origin.clone(),
                                rot: rot.clone(),
                            }));
                        }
                    }
                }
                self.globals.insert(id.to_string(), v.clone());
                Ok(v)
            }
            ItemDef::Io { .. } => Ok(Value::Io(id.to_string())),
            ItemDef::Fn { .. } => Ok(Value::Func(id.to_string())),
            ItemDef::Struct { .. } | ItemDef::Enum { .. } | ItemDef::Class { .. } => {
                Ok(Value::Type(id.to_string()))
            }
            ItemDef::Fsm { .. } => Ok(Value::Fsm(id.to_string())),
            _ => Ok(Value::Opaque(name.to_string())),
        }
    }

    pub(crate) fn read_signal(&self, id: &str) -> Value {
        if let Some(s) = &self.ctx.snap_sig {
            if let Some(v) = s.get(id) {
                return v.clone();
            }
        }
        self.signals.get(id).cloned().unwrap_or(Value::Unit)
    }

    pub(crate) fn input_get(&self, key: &str) -> Option<Value> {
        if let Some(s) = &self.ctx.snap_in {
            return s.get(key).cloned();
        }
        self.inputs.get(key).cloned()
    }

    fn ev_binary(&mut self, e: &Expr, op: &str, l: &Expr, r: &Expr) -> R<Value> {
        match op {
            "and" => {
                if !self.ev_val(l)?.truthy() {
                    return Ok(Value::Bool(false));
                }
                return Ok(Value::Bool(self.ev_val(r)?.truthy()));
            }
            "or" => {
                if self.ev_val(l)?.truthy() {
                    return Ok(Value::Bool(true));
                }
                return Ok(Value::Bool(self.ev_val(r)?.truthy()));
            }
            "??" => {
                return match self.ev(l)? {
                    Value::Opt(Some(b)) => Ok(*b),
                    Value::Opt(None) => self.ev(r),
                    other => Ok(other),
                };
            }
            _ => {}
        }
        let a = self.ev_val(l)?;
        let b = self.ev_val(r)?;
        if op == "++" {
            if let (Value::Int(x), Value::Int(y)) = (&a, &b) {
                let wb = bits_width(&r.ty).unwrap_or(0);
                let low = mask_bits(*y, wb, false);
                return Ok(Value::Int(x.checked_shl(wb).unwrap_or(0) | low));
            }
        }
        self.binop(op, a, b, &e.ty)
    }

    pub(crate) fn binop(&mut self, op: &str, a: Value, b: Value, ty: &Type) -> R<Value> {
        match op {
            "==" => return Ok(Value::Bool(a.same(&b))),
            "!=" => return Ok(Value::Bool(!a.same(&b))),
            "<" | "<=" | ">" | ">=" => {
                let ord = match (&a, &b) {
                    (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
                    _ => match (a.as_f64(), b.as_f64()) {
                        (Some(x), Some(y)) => x.partial_cmp(&y),
                        _ => None,
                    },
                };
                let Some(o) = ord else {
                    return Ok(Value::Bool(false));
                };
                return Ok(Value::Bool(match op {
                    "<" => o.is_lt(),
                    "<=" => o.is_le(),
                    ">" => o.is_gt(),
                    _ => o.is_ge(),
                }));
            }
            "in" => {
                return Ok(Value::Bool(match &b {
                    Value::Range(lo, hi, inc) => match (a.as_f64(), lo.as_f64(), hi.as_f64()) {
                        (Some(x), Some(l), Some(h)) => x >= l && (x < h || (*inc && x == h)),
                        _ => false,
                    },
                    Value::Array(v) => v.iter().any(|x| x.same(&a)),
                    _ => false,
                }))
            }
            _ => {}
        }
        if let (Value::Bool(x), Value::Bool(y)) = (&a, &b) {
            return Ok(Value::Bool(match op {
                "&" => *x & *y,
                "|" => *x | *y,
                "^" => *x ^ *y,
                _ => return Err(fault("ContractViolation")),
            }));
        }
        if let (Value::Int(x), Value::Int(y)) = (&a, &b) {
            return self.int_op(op, *x, *y, ty);
        }
        match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => {
                let r = match op {
                    "+" | "+%" | "+|" => x + y,
                    "-" | "-%" | "-|" => x - y,
                    "*" | "*%" | "*|" => x * y,
                    "/" => x / y,
                    "%" => x % y,
                    "**" => x.powf(y),
                    _ => return Err(fault("ContractViolation")),
                };
                Ok(Value::Real(r))
            }
            _ => Err(fault("ContractViolation")),
        }
    }

    fn int_op(&self, op: &str, x: i128, y: i128, ty: &Type) -> R<Value> {
        let kind = int_kind(ty);
        let over = || fault("Overflow");
        let r = match op {
            "+" => x.checked_add(y).ok_or_else(over)?,
            "-" => x.checked_sub(y).ok_or_else(over)?,
            "*" => x.checked_mul(y).ok_or_else(over)?,
            "/" => {
                if y == 0 {
                    return Err(fault("DivideByZero"));
                }
                x.checked_div(y).ok_or_else(over)?
            }
            "%" => {
                if y == 0 {
                    return Err(fault("DivideByZero"));
                }
                x.checked_rem(y).ok_or_else(over)?
            }
            "**" => {
                let e = u32::try_from(y).map_err(|_| over())?;
                x.checked_pow(e).ok_or_else(over)?
            }
            "+%" | "-%" | "*%" => {
                let raw = match op {
                    "+%" => x.wrapping_add(y),
                    "-%" => x.wrapping_sub(y),
                    _ => x.wrapping_mul(y),
                };
                return Ok(Value::Int(
                    self.finish_bits(kind.map_or(raw, |k| wrap_to(k, raw)), ty),
                ));
            }
            "+|" | "-|" | "*|" => {
                let raw = match op {
                    "+|" => x.saturating_add(y),
                    "-|" => x.saturating_sub(y),
                    _ => x.saturating_mul(y),
                };
                return Ok(Value::Int(
                    self.finish_bits(kind.map_or(raw, |k| saturate_to(k, raw)), ty),
                ));
            }
            "&" => x & y,
            "|" => x | y,
            "^" => x ^ y,
            "<<" => {
                let s = u32::try_from(y).map_err(|_| over())?;
                x.checked_shl(s).ok_or_else(over)?
            }
            ">>" => {
                let s = u32::try_from(y).map_err(|_| over())?;
                x.checked_shr(s).ok_or_else(over)?
            }
            _ => return Err(fault("ContractViolation")),
        };
        if matches!(ty, Type::Bits { .. }) {
            return Ok(Value::Int(self.finish_bits(r, ty)));
        }
        if let Some(k) = kind {
            if let Some((lo, hi)) = int_range(k) {
                if r < lo || r > hi {
                    return Err(over());
                }
            }
        }
        Ok(Value::Int(r))
    }

    fn finish_bits(&self, v: i128, ty: &Type) -> i128 {
        match ty {
            Type::Bits { width, signed } => mask_bits(v, *width, *signed),
            _ => v,
        }
    }

    fn unary(&mut self, op: &str, v: Value, ty: &Type) -> R<Value> {
        Ok(match (op, v) {
            ("not", Value::Bool(b)) => Value::Bool(!b),
            ("not", other) => Value::Bool(!other.truthy()),
            ("-", Value::Int(i)) => Value::Int(i.checked_neg().ok_or_else(|| fault("Overflow"))?),
            ("-", Value::Real(r)) => Value::Real(-r),
            ("-", Value::Qty(si, u)) => Value::Qty(-si, u),
            ("~", Value::Int(i)) => {
                let n = !i;
                Value::Int(match (ty, int_kind(ty)) {
                    (Type::Bits { width, signed }, _) => mask_bits(n, *width, *signed),
                    (_, Some(k)) => wrap_to(k, n),
                    _ => n,
                })
            }
            _ => return Err(fault("ContractViolation")),
        })
    }

    fn cast(&mut self, v: Value, ty: &Type) -> R<Value> {
        match ty {
            Type::Num { kind, unit } => {
                let is_int = kind.starts_with(['u', 'i']) && kind != "integer";
                let keep_si = matches!(v, Value::Qty(..));
                let n = v.as_f64().ok_or_else(|| fault("ContractViolation"))?;
                if let Some(u) = unit {
                    let info = self.unit_info(u);
                    let si = if keep_si { n } else { info.to_si(n) };
                    return Ok(Value::Qty(si, info));
                }
                if is_int {
                    let i = if let Value::Int(i) = v {
                        i
                    } else {
                        n.trunc() as i128
                    };
                    if let Some((lo, hi)) = int_range(kind) {
                        if i < lo || i > hi {
                            return Err(fault("Overflow"));
                        }
                    }
                    Ok(Value::Int(i))
                } else if kind == "f32" {
                    Ok(Value::Real(f64::from(n as f32)))
                } else {
                    Ok(Value::Real(n))
                }
            }
            Type::Bits { width, signed } => match v {
                Value::Int(i) => Ok(Value::Int(mask_bits(i, *width, *signed))),
                Value::Bool(b) => Ok(Value::Int(i128::from(b))),
                other => Ok(other),
            },
            _ => Ok(v),
        }
    }

    fn ev_field(&mut self, e: &Expr, base: &Expr, name: &str) -> R<Value> {
        let b = self.ev(base)?;
        if matches!(name, "rising" | "falling" | "changed")
            && !matches!(b, Value::Obj(_) | Value::Variant(_) | Value::Handle(_))
        {
            return Ok(Value::Bool(self.edge(e, &b, name)));
        }
        match b {
            Value::Obj(rc) => Ok(rc.borrow().get(name).cloned().unwrap_or(Value::Unit)),
            Value::Io(k) => Ok(self.io_prop(&k, name)),
            Value::Range(lo, hi, _) => match name {
                "start" => Ok(*lo),
                "end" => Ok(*hi),
                _ => Err(fault("ContractViolation")),
            },
            Value::Type(id) => self.variant_value(&id, name),
            Value::Module(mid) => {
                let m: usize = mid.trim_start_matches('m').parse().unwrap_or(0);
                match self.idx.by_name.get(&(m, name.to_string())).copied() {
                    Some((mm, ii)) => {
                        let nir = Rc::clone(&self.nir);
                        let id = nir.modules[mm].items[ii].id.clone();
                        self.ev_ref(&id, name)
                    }
                    None => Err(fault("ContractViolation")),
                }
            }
            Value::Handle(h) => {
                let st = self.handles.get(h).cloned();
                Ok(Value::Bool(match (name, st) {
                    ("busy", Some(s)) => s.busy,
                    ("done", Some(s)) => s.done,
                    ("failed", Some(s)) => s.failed,
                    ("timeout", Some(s)) => s.timeout,
                    _ => false,
                }))
            }
            Value::Opaque(p) => {
                let key = format!("{p}.{name}");
                Ok(self.input_get(&key).unwrap_or(Value::Opaque(key)))
            }
            Value::Tuple(items) => {
                let k = match name {
                    "x" => Some(0),
                    "y" => Some(1),
                    "z" => Some(2),
                    n => n.parse::<usize>().ok(),
                };
                k.and_then(|k| items.get(k).cloned())
                    .ok_or_else(|| fault("ContractViolation"))
            }
            Value::Fsm(id) => Ok(self.fsm_field(&id, name)),
            _ => Err(fault("ContractViolation")),
        }
    }

    fn edge(&mut self, e: &Expr, v: &Value, kind: &str) -> bool {
        let cur = v.truthy();
        let site = (self.cur_module, e.span[0], e.span[1]);
        let prev = self.edge_sites.insert(site, cur).unwrap_or(false);
        match kind {
            "rising" => cur && !prev,
            "falling" => !cur && prev,
            _ => cur != prev,
        }
    }

    /// An enum variant without payload, `Mode.Auto`.
    pub(crate) fn variant_value(&mut self, id: &str, name: &str) -> R<Value> {
        self.make_variant(id, name, Vec::new())
    }

    pub(crate) fn make_variant(&mut self, id: &str, name: &str, payload: Vec<Value>) -> R<Value> {
        let &(m, i) = self
            .idx
            .items
            .get(id)
            .ok_or_else(|| fault("ContractViolation"))?;
        let nir = Rc::clone(&self.nir);
        match &nir.modules[m].items[i].def {
            ItemDef::Enum {
                name: ename,
                variants,
                ..
            } if variants.iter().any(|v| v.name == name) => {
                Ok(Value::Variant(Rc::new(crate::value::Variant {
                    enum_id: id.to_string(),
                    enum_name: ename.clone(),
                    name: name.to_string(),
                    payload,
                })))
            }
            _ => Err(fault("ContractViolation")),
        }
    }

    fn ev_pipe(&mut self, rhs: &Expr, v: Value) -> R<Value> {
        match &rhs.e {
            ExprKind::Call {
                callee,
                args,
                target,
            } => self.ev_call(rhs, callee, args, target.as_ref(), Some(v)),
            _ => {
                let c = self.ev(rhs)?;
                self.call_value(rhs, c, vec![(None, v)], &[])
            }
        }
    }

    // ---- places -----------------------------------------------------------------

    pub(crate) fn write_signal(&mut self, id: &str, v: Value) {
        if self.ctx.act.is_some() && !self.ctx.immediate && self.ctx.snap_sig.is_some() {
            self.ctx.writes.push((id.to_string(), v));
        } else {
            self.commit_signal(id, v);
        }
    }

    fn assign_ref(&mut self, id: &str, v: Value) -> R<()> {
        if self.signals.contains_key(id) {
            self.write_signal(id, v);
        } else {
            self.globals.insert(id.to_string(), v);
        }
        Ok(())
    }

    pub(crate) fn assign(&mut self, place: &Expr, v: Value) -> R<()> {
        match &place.e {
            ExprKind::Local { id, .. } => {
                self.locals.insert(id.clone(), v);
                Ok(())
            }
            ExprKind::Ref { id, .. } => self.assign_ref(id, v),
            ExprKind::Field { base, name, .. } => {
                let b = self.ev(base)?;
                match b {
                    Value::Obj(rc) => {
                        rc.borrow_mut().set(name, v);
                        Ok(())
                    }
                    Value::Io(k) => self.io_set_prop(&k, name, v),
                    Value::Opaque(p) => {
                        self.inputs.insert(format!("{p}.{name}"), v);
                        Ok(())
                    }
                    _ => {
                        let n = name.clone();
                        let mut v = Some(v);
                        self.modify_place(base, &mut |bv: &mut Value| {
                            if let (Value::Tuple(items), Some(nv)) = (bv, v.take()) {
                                let k = match n.as_str() {
                                    "x" => Some(0),
                                    "y" => Some(1),
                                    "z" => Some(2),
                                    o => o.parse::<usize>().ok(),
                                };
                                if let Some(slot) = k.and_then(|k| items.get_mut(k)) {
                                    *slot = nv;
                                }
                            }
                            Ok(())
                        })
                    }
                }
            }
            ExprKind::Index { base, index, .. } => {
                let i = self.ev(index)?.as_int().unwrap_or(-1);
                let Ok(i) = usize::try_from(i) else {
                    return Err(fault("ContractViolation"));
                };
                let mut v = Some(v);
                self.modify_place(base, &mut |bv: &mut Value| match bv {
                    Value::Array(a) => match (a.get_mut(i), v.take()) {
                        (Some(slot), Some(nv)) => {
                            *slot = nv;
                            Ok(())
                        }
                        _ => Err(fault("ContractViolation")),
                    },
                    _ => Ok(()),
                })
            }
            _ => Err(fault("ContractViolation")),
        }
    }

    fn modify_place(&mut self, place: &Expr, f: &mut dyn FnMut(&mut Value) -> R<()>) -> R<()> {
        match &place.e {
            ExprKind::Local { id, .. } => match self.locals.get_mut(id) {
                Some(slot) => f(slot),
                None => Ok(()),
            },
            ExprKind::SelfValue => match self.locals.get_mut("self") {
                Some(slot) => f(slot),
                None => Ok(()),
            },
            ExprKind::Ref { id, .. } => match self.globals.get_mut(id) {
                Some(slot) => f(slot),
                None => Ok(()),
            },
            ExprKind::Field { base, name, .. } => {
                let n = name.clone();
                self.modify_place(base, &mut |bv: &mut Value| match bv {
                    Value::Obj(rc) => {
                        let mut o = rc.borrow_mut();
                        match o.fields.iter_mut().find(|(k, _)| *k == n) {
                            Some((_, slot)) => f(slot),
                            None => Ok(()),
                        }
                    }
                    Value::Tuple(items) => {
                        let k = match n.as_str() {
                            "x" => Some(0),
                            "y" => Some(1),
                            "z" => Some(2),
                            o => o.parse::<usize>().ok(),
                        };
                        match k.and_then(|k| items.get_mut(k)) {
                            Some(slot) => f(slot),
                            None => Ok(()),
                        }
                    }
                    _ => Ok(()),
                })
            }
            ExprKind::Index { base, index, .. } => {
                let i = self.ev(index)?.as_int().unwrap_or(-1);
                let Ok(i) = usize::try_from(i) else {
                    return Err(fault("ContractViolation"));
                };
                self.modify_place(base, &mut |bv: &mut Value| match bv {
                    Value::Array(a) => match a.get_mut(i) {
                        Some(slot) => f(slot),
                        None => Err(fault("ContractViolation")),
                    },
                    _ => Ok(()),
                })
            }
            _ => Ok(()),
        }
    }

    // ---- statements -------------------------------------------------------------

    pub(crate) fn exec_block(&mut self, b: &Block) -> R<()> {
        for s in &b.stmts {
            self.exec_stmt(s)?;
        }
        Ok(())
    }

    pub(crate) fn exec_stmt(&mut self, s: &Stmt) -> R<()> {
        self.steps += 1;
        if self.steps > STEP_BUDGET {
            return Err(fault("LoopBound"));
        }
        match &s.s {
            StmtKind::Let { pattern, value, .. } => {
                let v = self.ev_val(value)?.deep();
                self.bind_pat(pattern, &v)?;
                Ok(())
            }
            StmtKind::Assign {
                place,
                op,
                value,
                when,
            } => {
                if let Some(w) = when {
                    if !self.ev_val(w)?.truthy() {
                        return Ok(());
                    }
                }
                let rhs = self.ev_val(value)?;
                let nv = if op == "=" {
                    rhs.deep()
                } else {
                    let cur = self.ev(place)?;
                    let bop = op.strip_suffix('=').unwrap_or(op);
                    let r = self.binop(bop, cur, rhs, &place.ty)?;
                    self.fit(r, &place.ty)
                };
                self.assign(place, nv)
            }
            StmtKind::Expr { expr } => {
                self.ev(expr)?;
                Ok(())
            }
            StmtKind::Pass | StmtKind::Asm { .. } | StmtKind::Rule { .. } => Ok(()),
            StmtKind::Return { value } => {
                let v = match value {
                    Some(e) => self.ev(e)?,
                    None => Value::Unit,
                };
                Err(Stop::Return(v))
            }
            StmtKind::Break => Err(Stop::Break),
            StmtKind::Continue => Err(Stop::Continue),
            StmtKind::Fault { name, .. } => Err(Stop::Fault(name.clone())),
            StmtKind::Assert { cond, msg } => {
                if self.ev_val(cond)?.truthy() {
                    return Ok(());
                }
                let text = msg
                    .as_deref()
                    .map(|m| m.trim_matches('"').to_string())
                    .unwrap_or_default();
                if self.in_test {
                    self.fail = Some((self.cur_module, s.span, text.clone()));
                    return Err(Stop::Fault(format!("AssertionFailed: {text}")));
                }
                Err(fault("ContractViolation"))
            }
            StmtKind::Emit { event, .. } => {
                self.emit_event(event);
                Ok(())
            }
            StmtKind::Coil { set, place, when } => {
                if self.ev_val(when)?.truthy() {
                    self.assign(place, Value::Bool(*set))?;
                }
                Ok(())
            }
            StmtKind::Wait { .. }
            | StmtKind::Until { .. }
            | StmtKind::Jump { .. }
            | StmtKind::Move { .. } => Ok(()),
            StmtKind::If {
                branches,
                otherwise,
            } => {
                for br in branches {
                    if self.ev_val(&br.cond)?.truthy() {
                        return self.exec_block(&br.body);
                    }
                }
                match otherwise {
                    Some(b) => self.exec_block(b),
                    None => Ok(()),
                }
            }
            StmtKind::Match { value, arms } => {
                let v = self.ev(value)?;
                for arm in arms {
                    if self.bind_pat(&arm.pattern, &v)? {
                        let pass = match &arm.guard {
                            Some(g) => self.ev_val(g)?.truthy(),
                            None => true,
                        };
                        if pass {
                            return self.exec_block(&arm.body);
                        }
                    }
                }
                Ok(())
            }
            StmtKind::For {
                pattern,
                iter,
                body,
                ..
            } => {
                let it = self.ev(iter)?;
                let items: Vec<Value> = match it {
                    Value::Range(lo, hi, inc) => {
                        let (a, b) = (lo.as_int().unwrap_or(0), hi.as_int().unwrap_or(0));
                        let end = if inc { b.saturating_add(1) } else { b };
                        if end.saturating_sub(a) > MAX_ARRAY as i128 {
                            return Err(fault("LoopBound"));
                        }
                        (a..end).map(Value::Int).collect()
                    }
                    Value::Array(v) | Value::Tuple(v) => v,
                    _ => Vec::new(),
                };
                for item in items {
                    self.bind_pat(pattern, &item)?;
                    match self.exec_block(body) {
                        Err(Stop::Break) => break,
                        Err(Stop::Continue) | Ok(()) => {}
                        Err(other) => return Err(other),
                    }
                }
                Ok(())
            }
            StmtKind::While {
                pattern,
                cond,
                max,
                body,
            } => {
                let bound = self.loop_bound(max.as_ref())?;
                let mut n = 0u64;
                while n < bound {
                    let c = self.ev(cond)?;
                    let go = match pattern {
                        Some(p) => self.bind_pat(p, &c)?,
                        None => c.truthy(),
                    };
                    if !go {
                        break;
                    }
                    n += 1;
                    match self.exec_block(body) {
                        Err(Stop::Break) => break,
                        Err(Stop::Continue) | Ok(()) => {}
                        Err(other) => return Err(other),
                    }
                }
                Ok(())
            }
            StmtKind::Loop { max, body } => {
                let bound = self.loop_bound(max.as_ref())?;
                let mut n = 0u64;
                while n < bound {
                    n += 1;
                    match self.exec_block(body) {
                        Err(Stop::Break) => break,
                        Err(Stop::Continue) | Ok(()) => {}
                        Err(other) => return Err(other),
                    }
                }
                Ok(())
            }
            StmtKind::Atomic { body } => self.exec_block(body),
        }
    }

    pub(crate) fn loop_bound(&mut self, max: Option<&Expr>) -> R<u64> {
        const CAP: u64 = 1_000_000;
        match max {
            Some(e) => {
                let n = self.ev(e)?.as_int().unwrap_or(0);
                Ok(u64::try_from(n).unwrap_or(0).min(CAP))
            }
            None => Ok(CAP),
        }
    }

    // ---- patterns ---------------------------------------------------------------

    /// Matches `v` against `p`, binding names. Returns `false` when it does not match.
    pub(crate) fn bind_pat(&mut self, p: &Pattern, v: &Value) -> R<bool> {
        match &p.p {
            PatternKind::Wildcard => Ok(true),
            PatternKind::Bind { name, id, .. } => {
                self.locals.insert(id.clone(), v.clone());
                self.names.insert(name.clone(), id.clone());
                Ok(true)
            }
            PatternKind::Lit { value } => {
                let lv = self.ev(value)?;
                Ok(lv.same(v))
            }
            PatternKind::Range { lo, hi, inclusive } => {
                let (l, h) = (self.ev(lo)?, self.ev(hi)?);
                Ok(match (v.as_f64(), l.as_f64(), h.as_f64()) {
                    (Some(x), Some(l), Some(h)) => x >= l && (x < h || (*inclusive && x == h)),
                    _ => false,
                })
            }
            PatternKind::Ctor { path, args, fields } => {
                let last = path.rsplit('.').next().unwrap_or(path);
                match (last, v) {
                    ("Some", Value::Opt(Some(b))) => match args.first() {
                        Some(a) => self.bind_pat(a, b),
                        None => Ok(true),
                    },
                    ("Ok", Value::Res(Ok(b))) | ("Err", Value::Res(Err(b))) => match args.first() {
                        Some(a) => self.bind_pat(a, b),
                        None => Ok(true),
                    },
                    (name, Value::Variant(var)) if var.name == name => {
                        for (a, pv) in args.iter().zip(&var.payload) {
                            if !self.bind_pat(a, pv)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                    (_, Value::Obj(rc)) => {
                        let o = rc.borrow();
                        let named: Vec<(String, Value)> = fields
                            .iter()
                            .filter_map(|f| o.get(&f.name).map(|x| (f.name.clone(), x.clone())))
                            .collect();
                        drop(o);
                        for (f, (_, fv)) in fields.iter().zip(named) {
                            if !self.bind_pat(&f.pattern, &fv)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            PatternKind::Path { path } => {
                let last = path.rsplit('.').next().unwrap_or(path);
                Ok(match v {
                    Value::Variant(var) => var.name == last,
                    Value::Opt(None) => last == "none",
                    _ => false,
                })
            }
            PatternKind::Tuple { items } => match v {
                Value::Tuple(vs) if vs.len() == items.len() => {
                    for (p, x) in items.iter().zip(vs) {
                        if !self.bind_pat(p, x)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                _ => Ok(false),
            },
            PatternKind::Or { alts } => {
                for a in alts {
                    if self.bind_pat(a, v)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    /// Visits `e` and every expression inside it.
    pub(crate) fn walk<'a>(e: &'a Expr, f: &mut dyn FnMut(&'a Expr)) {
        f(e);
        match &e.e {
            ExprKind::Quantity { value, .. } => Self::walk(value, f),
            ExprKind::Binary { lhs, rhs, .. } | ExprKind::Pipe { lhs, rhs } => {
                Self::walk(lhs, f);
                Self::walk(rhs, f);
            }
            ExprKind::Compare { first, rest } => {
                Self::walk(first, f);
                for c in rest {
                    Self::walk(&c.rhs, f);
                }
            }
            ExprKind::Unary { operand, .. }
            | ExprKind::Try { operand }
            | ExprKind::Cast { operand, .. }
            | ExprKind::Old { operand } => Self::walk(operand, f),
            ExprKind::Run { action } => Self::walk(action, f),
            ExprKind::Range { lo, hi, .. } => {
                Self::walk(lo, f);
                Self::walk(hi, f);
            }
            ExprKind::Map { from, to } => {
                Self::walk(from, f);
                Self::walk(to, f);
            }
            ExprKind::IfElse {
                cond,
                then,
                otherwise,
            } => {
                Self::walk(cond, f);
                Self::walk(then, f);
                Self::walk(otherwise, f);
            }
            ExprKind::Call { callee, args, .. } => {
                Self::walk(callee, f);
                for a in args {
                    Self::walk(&a.value, f);
                }
            }
            ExprKind::Index { base, index, end } => {
                Self::walk(base, f);
                Self::walk(index, f);
                if let Some(x) = end {
                    Self::walk(x, f);
                }
            }
            ExprKind::Field { base, .. } => Self::walk(base, f),
            ExprKind::Tuple { items } | ExprKind::Array { items } => {
                for x in items {
                    Self::walk(x, f);
                }
            }
            ExprKind::ArrayRepeat { value, count } => {
                Self::walk(value, f);
                Self::walk(count, f);
            }
            ExprKind::Lambda { body, .. } => Self::walk(body, f),
            _ => {}
        }
    }
}

fn format_value(v: &Value, fmt: &str) -> String {
    let places = fmt.strip_prefix('.').and_then(|d| d.parse::<usize>().ok());
    match (v, places) {
        (Value::Qty(si, u), Some(p)) => format!("{:.p$} {}", u.from_si(*si), u.text),
        (Value::Real(r), Some(p)) => format!("{r:.p$}"),
        (Value::Int(i), Some(p)) => format!("{:.p$}", *i as f64),
        _ => v.show(),
    }
}
