//! Calls: user functions, methods, constructors, lambdas and actions of other devices.

use crate::builtin::geo_comp;
use crate::value::{round9, Closure, Geo, Obj, Value};
use crate::{fault, Ev, HandleState, Interp, Stop, TraceKind, R};
use neuron_nir::{Arg, CallTarget, Expr, ExprKind, FnBody, FnDecl, ItemDef, Param, Type, TypeExpr};
use std::cell::RefCell;
use std::rc::Rc;

/// Evaluated arguments: optional name and value.
pub(crate) type Vals = Vec<(Option<String>, Value)>;

fn positional(vals: &Vals) -> Vec<Value> {
    vals.iter()
        .filter(|(n, _)| n.is_none())
        .map(|(_, v)| v.clone())
        .collect()
}

fn named(vals: &Vals, name: &str) -> Option<Value> {
    vals.iter()
        .find(|(n, _)| n.as_deref() == Some(name))
        .map(|(_, v)| v.clone())
}

pub(crate) fn new_obj(def: &str, name: &str, native: bool, fields: Vec<(String, Value)>) -> Value {
    Value::Obj(Rc::new(RefCell::new(Obj {
        def: def.to_string(),
        name: name.to_string(),
        native,
        fields,
    })))
}

/// The value a variable of this type has before it is assigned.
pub(crate) fn default_for(ty: &Type) -> Value {
    match ty {
        Type::Bool => Value::Bool(false),
        Type::Num { kind, .. } if matches!(kind.as_str(), "f32" | "f64" | "real" | "duration") => {
            Value::Real(0.0)
        }
        Type::Num { .. } | Type::Bits { .. } => Value::Int(0),
        Type::Str { .. } => Value::Str(String::new()),
        Type::Array { elem, len } => {
            let n = usize::try_from(len.unwrap_or(0)).unwrap_or(0).min(1 << 20);
            Value::Array((0..n).map(|_| default_for(elem)).collect())
        }
        Type::Tuple { items } => Value::Tuple(items.iter().map(default_for).collect()),
        Type::Option { .. } | Type::NoneLit => Value::Opt(None),
        Type::Instant => Value::Real(0.0),
        _ => Value::Unit,
    }
}

fn default_for_expr(t: &TypeExpr) -> Value {
    use neuron_nir::TypeExprKind as K;
    match &t.kind {
        _ if t.optional => Value::Opt(None),
        K::Named { path, .. } => match path.as_str() {
            "bool" => Value::Bool(false),
            "f32" | "f64" | "duration" | "instant" => Value::Real(0.0),
            "str" => Value::Str(String::new()),
            p if p.starts_with(['u', 'i']) || p.starts_with("bit") || p.starts_with("sbit") => {
                Value::Int(0)
            }
            _ if t.unit.is_some() => Value::Real(0.0),
            _ => Value::Unit,
        },
        K::Tuple { items } => Value::Tuple(items.iter().map(default_for_expr).collect()),
        K::Unit => Value::Real(0.0),
        _ => Value::Unit,
    }
}

impl Interp {
    /// The value of a variable of type `ty` before it is assigned. A quantity starts at
    /// zero *in its unit* (so `0 degC` is 273.15 K).
    pub(crate) fn default_value(&mut self, ty: &Type) -> Value {
        match ty {
            Type::Num { unit: Some(u), .. } => {
                let info = self.unit_info(u);
                Value::Qty(info.to_si(0.0), info)
            }
            other => default_for(other),
        }
    }

    pub(crate) fn finished_handle(&mut self) -> Value {
        let h = self.handles.len();
        self.handles.push(HandleState {
            busy: false,
            done: true,
            failed: false,
            timeout: false,
            path: String::new(),
        });
        Value::Handle(h)
    }

    pub(crate) fn ev_call(
        &mut self,
        e: &Expr,
        callee: &Expr,
        args: &[Arg],
        target: Option<&CallTarget>,
        pre: Option<Value>,
    ) -> R<Value> {
        let _ = target;
        let saved = self.in_run;
        self.in_run = false;
        let mut vals: Vals = Vec::new();
        if let Some(p) = pre {
            vals.push((None, p));
        }
        let mut failed: Option<Stop> = None;
        for a in args {
            match self.ev(&a.value) {
                Ok(v) => vals.push((a.name.clone(), v)),
                Err(s) => {
                    failed = Some(s);
                    break;
                }
            }
        }
        self.in_run = saved;
        if let Some(s) = failed {
            return Err(s);
        }
        match &callee.e {
            ExprKind::Field { base, name, .. } => {
                let b = self.ev(base)?;
                self.call_method(e, b, name, vals, args)
            }
            _ => {
                let c = self.ev(callee)?;
                self.call_value(e, c, vals, args)
            }
        }
    }

    pub(crate) fn call_value(&mut self, e: &Expr, c: Value, vals: Vals, args: &[Arg]) -> R<Value> {
        match c {
            Value::Func(id) => {
                let &(m, i) = self
                    .idx
                    .items
                    .get(&id)
                    .ok_or_else(|| fault("ContractViolation"))?;
                let nir = Rc::clone(&self.nir);
                match &nir.modules[m].items[i].def {
                    ItemDef::Fn { func } => self.call_user_fn(func, vals, None, args),
                    _ => Err(fault("ContractViolation")),
                }
            }
            Value::Type(id) => self.construct(&id, vals),
            Value::Lambda(cl) => self.call_lambda(&cl, &vals),
            Value::Opaque(name) => self.call_named(e, &name, vals),
            _ => Err(fault("ContractViolation")),
        }
    }

    fn call_lambda(&mut self, c: &Closure, vals: &Vals) -> R<Value> {
        let saved = std::mem::replace(&mut self.locals, c.captured.clone());
        for ((n, id), (_, v)) in c.params.iter().zip(vals) {
            self.locals.insert(id.clone(), v.clone());
            self.names.insert(n.clone(), id.clone());
        }
        let r = self.ev(&c.body);
        self.locals = saved;
        r
    }

    fn call_method(
        &mut self,
        e: &Expr,
        recv: Value,
        name: &str,
        vals: Vals,
        args: &[Arg],
    ) -> R<Value> {
        match recv {
            Value::Io(k) => self.io_call(&k, name, &vals),
            Value::Obj(rc) => {
                let (native, def) = {
                    let o = rc.borrow();
                    (o.native, o.def.clone())
                };
                if native {
                    return self.native_call(&rc, name, &vals);
                }
                let key = (def, name.to_string());
                if let Some(&(m, i, k)) = self.idx.methods.get(&key) {
                    let nir = Rc::clone(&self.nir);
                    if let ItemDef::Class { methods, .. } = &nir.modules[m].items[i].def {
                        if let ItemDef::Fn { func } = &methods[k].def {
                            return self.call_user_fn(
                                func,
                                vals,
                                Some(Value::Obj(Rc::clone(&rc))),
                                args,
                            );
                        }
                    }
                }
                Err(fault("ContractViolation"))
            }
            Value::Type(id) => {
                let payload: Vec<Value> = vals.into_iter().map(|(_, v)| v).collect();
                self.make_variant(&id, name, payload)
            }
            Value::Module(mid) => {
                let m: usize = mid.trim_start_matches('m').parse().unwrap_or(0);
                let &(mm, ii) = self
                    .idx
                    .by_name
                    .get(&(m, name.to_string()))
                    .ok_or_else(|| fault("ContractViolation"))?;
                let nir = Rc::clone(&self.nir);
                match &nir.modules[mm].items[ii].def {
                    ItemDef::Fn { func } => self.call_user_fn(func, vals, None, args),
                    ItemDef::Struct { .. } | ItemDef::Class { .. } => {
                        let id = nir.modules[mm].items[ii].id.clone();
                        self.construct(&id, vals)
                    }
                    _ => Err(fault("ContractViolation")),
                }
            }
            Value::Opaque(path) => self.foreign_call(&path, name, &vals),
            Value::Fsm(_) | Value::Handle(_) => Ok(Value::Unit),
            Value::Lambda(cl) => self.call_lambda(&cl, &vals),
            other => self.value_method(e, &other, name, &vals),
        }
    }

    /// A call on something outside the package. Inside `run` it starts an asynchronous
    /// action that takes `Options::action_ns` and then sets `<path>.done`.
    fn foreign_call(&mut self, path: &str, method: &str, vals: &Vals) -> R<Value> {
        let _ = vals;
        if !self.in_run {
            return Ok(Value::Opaque(format!("{path}.{method}")));
        }
        let h = self.handles.len();
        self.handles.push(HandleState {
            busy: true,
            done: false,
            failed: false,
            timeout: false,
            path: path.to_string(),
        });
        self.inputs
            .insert(format!("{path}.busy"), Value::Bool(true));
        self.inputs
            .insert(format!("{path}.done"), Value::Bool(false));
        self.trace_push(
            TraceKind::Event,
            format!("{path}.{method}"),
            "started".to_string(),
        );
        let t = self.now + self.opts.action_ns;
        self.push_event(t, Ev::Done(h));
        Ok(Value::Handle(h))
    }

    fn value_method(&mut self, e: &Expr, recv: &Value, name: &str, vals: &Vals) -> R<Value> {
        let _ = e;
        let arg0 = vals.first().map(|(_, v)| v.clone());
        match (recv, name) {
            (Value::Range(lo, hi, inc), "contains") => {
                let x = arg0.and_then(|v| v.as_f64());
                Ok(Value::Bool(match (x, lo.as_f64(), hi.as_f64()) {
                    (Some(x), Some(l), Some(h)) => x >= l && (x < h || (*inc && x == h)),
                    _ => false,
                }))
            }
            (Value::Array(v) | Value::Tuple(v), "len") => Ok(Value::Int(v.len() as i128)),
            (Value::Str(s), "len") => Ok(Value::Int(s.len() as i128)),
            (Value::Opt(o), "is_some") => Ok(Value::Bool(o.is_some())),
            (Value::Opt(o), "is_none") => Ok(Value::Bool(o.is_none())),
            (Value::Opt(o), "unwrap_or") => Ok(match o {
                Some(b) => (**b).clone(),
                None => arg0.unwrap_or(Value::Unit),
            }),
            (Value::Res(r), "is_ok") => Ok(Value::Bool(r.is_ok())),
            (Value::Res(r), "is_err") => Ok(Value::Bool(r.is_err())),
            (Value::Res(r), "unwrap_or") => Ok(match r {
                Ok(b) => (**b).clone(),
                Err(_) => arg0.unwrap_or(Value::Unit),
            }),
            (Value::Qty(si, u), "round" | "floor" | "ceil" | "trunc" | "abs") => {
                let n = u.from_si(*si);
                Ok(Value::Qty(u.to_si(round_like(name, n)), Rc::clone(u)))
            }
            (Value::Real(n), "round" | "floor" | "ceil" | "trunc" | "abs") => {
                Ok(Value::Real(round_like(name, *n)))
            }
            (Value::Int(n), "round" | "floor" | "ceil" | "trunc") => Ok(Value::Int(*n)),
            (Value::Int(n), "abs") => Ok(Value::Int(n.saturating_abs())),
            (Value::Real(n), "is_nan") => Ok(Value::Bool(n.is_nan())),
            (Value::Geo(g), "xyz") => match &**g {
                Geo::Frame { name, .. } => {
                    let mut v = [0.0; 3];
                    for (slot, (_, a)) in v.iter_mut().zip(vals) {
                        *slot = geo_comp(a, false);
                    }
                    Ok(Value::Geo(Rc::new(Geo::Point {
                        frame: name.clone(),
                        v,
                    })))
                }
                _ => Err(fault("ContractViolation")),
            },
            (Value::Geo(g), "above") => match (&**g, arg0) {
                (Geo::Point { frame, v }, Some(d)) => Ok(Value::Geo(Rc::new(Geo::Point {
                    frame: frame.clone(),
                    v: [v[0], v[1], round9(v[2] + geo_comp(&d, false))],
                }))),
                _ => Err(fault("ContractViolation")),
            },
            _ => Err(fault("ContractViolation")),
        }
    }

    // ---- user functions ---------------------------------------------------------

    pub(crate) fn call_user_fn(
        &mut self,
        func: &FnDecl,
        vals: Vals,
        selfv: Option<Value>,
        args: &[Arg],
    ) -> R<Value> {
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_names = std::mem::take(&mut self.names);
        self.depth += 1;
        let r = if self.depth > 100 {
            Err(fault("StackOverflow"))
        } else {
            self.call_body(func, &vals, selfv)
        };
        self.depth -= 1;
        self.locals = saved_locals;
        self.names = saved_names;
        let (result, finals) = r?;
        let params: Vec<&Param> = func
            .sig
            .params
            .iter()
            .filter(|p| p.name != "self")
            .collect();
        let mut pos = vals.len().saturating_sub(args.len());
        for a in args {
            let pname = match &a.name {
                Some(n) => n.clone(),
                None => {
                    let p = params.get(pos).map(|p| p.name.clone()).unwrap_or_default();
                    pos += 1;
                    p
                }
            };
            if a.mutable {
                if let Some((_, v)) = finals.iter().find(|(n, _)| *n == pname) {
                    self.assign(&a.value, v.clone())?;
                }
            }
        }
        Ok(result)
    }

    fn call_body(
        &mut self,
        func: &FnDecl,
        vals: &Vals,
        selfv: Option<Value>,
    ) -> R<(Value, Vec<(String, Value)>)> {
        if let Some(s) = selfv {
            self.locals.insert("self".to_string(), s);
        }
        let params: Vec<&Param> = func
            .sig
            .params
            .iter()
            .filter(|p| p.name != "self")
            .collect();
        let pos_vals = positional(vals);
        let mut pos = 0usize;
        for p in &params {
            let v = match named(vals, &p.name) {
                Some(v) => v,
                None => match pos_vals.get(pos) {
                    Some(v) => {
                        pos += 1;
                        v.clone()
                    }
                    None => match &p.default {
                        Some(d) => self.ev(d)?,
                        None => Value::Unit,
                    },
                },
            };
            if let Some(id) = &p.id {
                self.locals.insert(id.clone(), v);
                self.names.insert(p.name.clone(), id.clone());
            }
        }
        for c in &func.contracts {
            if c.kind == "requires" && !self.ev(&c.cond)?.truthy() {
                return Err(fault("ContractViolation"));
            }
        }
        let mut olds: Vec<&Expr> = Vec::new();
        for c in &func.contracts {
            if c.kind == "ensures" {
                Self::walk(&c.cond, &mut |x| {
                    if let ExprKind::Old { operand } = &x.e {
                        olds.push(operand.as_ref());
                    }
                });
            }
        }
        for o in olds {
            let v = self.ev(o)?.deep();
            self.locals
                .insert(format!("$old{}-{}", o.span[0], o.span[1]), v);
        }
        let result = match &func.body {
            FnBody::Block { block } => match self.exec_block(block) {
                Ok(()) => Value::Unit,
                Err(Stop::Return(v)) => v,
                Err(other) => return Err(other),
            },
            FnBody::Expr { expr } => self.ev(expr)?,
        };
        self.locals.insert("result".to_string(), result.clone());
        for c in &func.contracts {
            if c.kind == "ensures" && !self.ev(&c.cond)?.truthy() {
                return Err(fault("ContractViolation"));
            }
        }
        let finals = params
            .iter()
            .filter(|p| p.mutable)
            .filter_map(|p| {
                let id = p.id.as_ref()?;
                Some((p.name.clone(), self.locals.get(id)?.clone()))
            })
            .collect();
        Ok((result, finals))
    }

    // ---- construction ------------------------------------------------------------

    pub(crate) fn construct(&mut self, id: &str, vals: Vals) -> R<Value> {
        let &(m, i) = self
            .idx
            .items
            .get(id)
            .ok_or_else(|| fault("ContractViolation"))?;
        let nir = Rc::clone(&self.nir);
        let pos_vals = positional(&vals);
        let mut pos = 0usize;
        match &nir.modules[m].items[i].def {
            ItemDef::Struct { name, fields, .. } => {
                let mut out = Vec::new();
                for f in fields {
                    let v = match named(&vals, &f.name) {
                        Some(v) => v,
                        None => match pos_vals.get(pos) {
                            Some(v) => {
                                pos += 1;
                                v.clone()
                            }
                            None => match &f.default {
                                Some(d) => self.ev(d)?,
                                None => default_for_expr(&f.ty),
                            },
                        },
                    };
                    out.push((f.name.clone(), v.deep()));
                }
                Ok(new_obj(id, name, false, out))
            }
            ItemDef::Class { name, vars, .. } => {
                let mut out = Vec::new();
                for v in vars {
                    let val = match named(&vals, &v.name) {
                        Some(x) => x,
                        None => match (&v.init, pos_vals.get(pos)) {
                            (None, Some(x)) => {
                                pos += 1;
                                x.clone()
                            }
                            (Some(init), _) => self.ev(init)?,
                            (None, None) => self.default_value(&v.vty),
                        },
                    };
                    out.push((v.name.clone(), val.deep()));
                }
                Ok(new_obj(id, name, false, out))
            }
            _ => Err(fault("ContractViolation")),
        }
    }
}

fn round_like(name: &str, n: f64) -> f64 {
    match name {
        "round" => n.round(),
        "floor" => n.floor(),
        "ceil" => n.ceil(),
        "trunc" => n.trunc(),
        _ => n.abs(),
    }
}
