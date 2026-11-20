//! The part of the standard library the interpreter implements itself: math functions,
//! function blocks (timers, counters, edge detectors, filters, PID) and `io` bindings.

use crate::calls::{new_obj, Vals};
use crate::value::{round9, Geo, Obj, Value};
use crate::{fault, Interp, TraceKind, R};
use neuron_nir::Expr;
use std::cell::RefCell;
use std::rc::Rc;

fn arg(vals: &Vals, name: &str, pos: usize) -> Option<Value> {
    if let Some((_, v)) = vals.iter().find(|(n, _)| n.as_deref() == Some(name)) {
        return Some(v.clone());
    }
    vals.iter()
        .filter(|(n, _)| n.is_none())
        .nth(pos)
        .map(|(_, v)| v.clone())
}

fn num(v: &Option<Value>) -> f64 {
    v.as_ref().and_then(Value::as_f64).unwrap_or(0.0)
}

/// A number with the unit of `template`.
fn like(template: &Value, si: f64) -> Value {
    match template {
        Value::Qty(_, u) => Value::Qty(si, Rc::clone(u)),
        _ => Value::Real(si),
    }
}

/// A geometry component in mm (or deg when `angle`). A quantity is converted from base
/// units; a plain number is taken as written (the unit that follows scales it later).
pub(crate) fn geo_comp(v: &Value, angle: bool) -> f64 {
    match v {
        Value::Qty(si, _) => round9(if angle { si.to_degrees() } else { si * 1000.0 }),
        other => other.as_f64().unwrap_or(0.0),
    }
}

/// The components of an `xyz`/`rpy` value, or `n` zeros.
fn geo_vec(v: &Option<Value>, n: usize) -> Vec<f64> {
    match v {
        Some(Value::Geo(g)) => match &**g {
            Geo::Vec { v, .. } | Geo::Joints(v) => v.clone(),
            _ => vec![0.0; n],
        },
        _ => vec![0.0; n],
    }
}

fn getf(o: &Obj, k: &str) -> f64 {
    o.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

fn getb(o: &Obj, k: &str) -> bool {
    o.get(k).is_some_and(Value::truthy)
}

fn bools(fields: &[&str]) -> Vec<(String, Value)> {
    fields
        .iter()
        .map(|f| ((*f).to_string(), Value::Bool(false)))
        .collect()
}

fn reals(fields: &[&str]) -> Vec<(String, Value)> {
    fields
        .iter()
        .map(|f| ((*f).to_string(), Value::Real(0.0)))
        .collect()
}

/// Advances a block by `dt` seconds with its current inputs.
pub(crate) fn tick(o: &mut Obj, dt: f64) {
    let input = getb(o, "input");
    let prev = getb(o, "prev");
    match o.name.clone().as_str() {
        "ton" => {
            let preset = getf(o, "preset");
            let et = if input {
                (getf(o, "et") + dt).min(preset)
            } else {
                0.0
            };
            o.set("et", Value::Real(et));
            o.set("q", Value::Bool(input && et >= preset - 1e-12));
        }
        "tof" => {
            let preset = getf(o, "preset");
            let mut q = getb(o, "q");
            let mut et = getf(o, "et");
            if input {
                q = true;
                et = 0.0;
            } else if q {
                et += dt;
                if et >= preset - 1e-12 {
                    q = false;
                    et = preset;
                }
            }
            o.set("et", Value::Real(et));
            o.set("q", Value::Bool(q));
        }
        "tp" => {
            let preset = getf(o, "preset");
            let mut running = getb(o, "running");
            let mut et = getf(o, "et");
            let mut q = getb(o, "q");
            if input && !prev && !running {
                running = true;
                et = 0.0;
                q = true;
            }
            if running {
                et += dt;
                if et >= preset - 1e-12 {
                    running = false;
                    q = false;
                }
            }
            o.set("running", Value::Bool(running));
            o.set("et", Value::Real(et));
            o.set("q", Value::Bool(q));
        }
        "ctu" | "ctd" | "ctud" => {
            let rising = input && !prev;
            let mut cv = o.get("cv").and_then(Value::as_int).unwrap_or(0);
            let preset = o.get("preset").and_then(Value::as_int).unwrap_or(0);
            let down = getb(o, "down");
            let prev_down = getb(o, "prev_down");
            match o.name.clone().as_str() {
                "ctu" => {
                    if getb(o, "reset") {
                        cv = 0;
                    } else if rising {
                        cv += 1;
                    }
                    o.set("q", Value::Bool(cv >= preset));
                }
                "ctd" => {
                    if getb(o, "load") {
                        cv = preset;
                    } else if rising {
                        cv -= 1;
                    }
                    o.set("q", Value::Bool(cv <= 0));
                }
                _ => {
                    if getb(o, "reset") {
                        cv = 0;
                    } else {
                        if rising {
                            cv += 1;
                        }
                        if down && !prev_down {
                            cv -= 1;
                        }
                    }
                    o.set("qu", Value::Bool(cv >= preset));
                    o.set("qd", Value::Bool(cv <= 0));
                }
            }
            o.set("cv", Value::Int(cv));
            o.set("prev_down", Value::Bool(down));
        }
        "r_trig" => o.set("q", Value::Bool(input && !prev)),
        "f_trig" => o.set("q", Value::Bool(!input && prev)),
        "sr" => {
            let q = getb(o, "set") || (getb(o, "q") && !getb(o, "reset"));
            o.set("q", Value::Bool(q));
        }
        "rs" => {
            let q = !getb(o, "reset") && (getb(o, "set") || getb(o, "q"));
            o.set("q", Value::Bool(q));
        }
        _ => return,
    }
    o.set("prev", Value::Bool(input));
}

impl Interp {
    /// Seconds per evaluation of the running activity.
    pub(crate) fn dt(&self) -> f64 {
        let ns = if self.ctx.period > 0 {
            self.ctx.period
        } else {
            self.opts.base_tick_ns
        };
        ns as f64 / 1.0e9
    }

    // ---- named functions ----------------------------------------------------------

    pub(crate) fn call_named(&mut self, e: &Expr, name: &str, vals: Vals) -> R<Value> {
        let _ = e;
        let a0 = arg(&vals, "", 0).or_else(|| vals.first().map(|(_, v)| v.clone()));
        let a1 = vals.get(1).map(|(_, v)| v.clone());
        let a2 = vals.get(2).map(|(_, v)| v.clone());
        let x = a0.as_ref().and_then(Value::as_f64).unwrap_or(0.0);
        let r = |f: f64| Ok(Value::Real(f));
        match name {
            "abs" => match a0 {
                Some(Value::Int(i)) => Ok(Value::Int(i.saturating_abs())),
                Some(v) => Ok(like(&v, x.abs())),
                None => Err(fault("ContractViolation")),
            },
            "floor" | "ceil" | "round" => match a0 {
                Some(Value::Qty(si, u)) => {
                    let n = u.from_si(si);
                    let n = match name {
                        "floor" => n.floor(),
                        "ceil" => n.ceil(),
                        _ => n.round(),
                    };
                    Ok(Value::Qty(u.to_si(n), u))
                }
                Some(Value::Int(i)) => Ok(Value::Int(i)),
                _ => r(match name {
                    "floor" => x.floor(),
                    "ceil" => x.ceil(),
                    _ => x.round(),
                }),
            },
            "sign" => Ok(Value::Int(if x > 0.0 {
                1
            } else if x < 0.0 {
                -1
            } else {
                0
            })),
            "min" | "max" => {
                let (a, b) = match (a0, a1) {
                    (Some(a), Some(b)) => (a, b),
                    _ => return Err(fault("ContractViolation")),
                };
                let (x, y) = (a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0));
                let pick_first = if name == "min" { x <= y } else { x >= y };
                Ok(if pick_first { a } else { b })
            }
            "clamp" => {
                let (a, lo, hi) = match (a0, a1, a2) {
                    (Some(a), Some(lo), Some(hi)) => (a, lo, hi),
                    _ => return Err(fault("ContractViolation")),
                };
                let (v, l, h) = (
                    a.as_f64().unwrap_or(0.0),
                    lo.as_f64().unwrap_or(0.0),
                    hi.as_f64().unwrap_or(0.0),
                );
                Ok(if v < l {
                    lo
                } else if v > h {
                    hi
                } else {
                    a
                })
            }
            "sin" => r(x.sin()),
            "cos" => r(x.cos()),
            "tan" => r(x.tan()),
            "asin" => r(x.asin()),
            "acos" => r(x.acos()),
            "atan" => r(x.atan()),
            "atan2" => r(x.atan2(num(&a1))),
            "sqrt" => r(x.sqrt()),
            "exp" => r(x.exp()),
            "log" => r(x.ln()),
            "log10" => r(x.log10()),
            "pow" => r(x.powf(num(&a1))),
            "hypot" => r(x.hypot(num(&a1))),
            "now" => r(self.now as f64 / 1.0e9),
            "scale" => match (a0, a1) {
                (Some(v), Some(Value::Map(from, to))) => {
                    let (Value::Range(f0, f1, _), Value::Range(t0, t1, _)) = (&*from, &*to) else {
                        return Err(fault("ContractViolation"));
                    };
                    let (f0, f1) = (f0.as_f64().unwrap_or(0.0), f1.as_f64().unwrap_or(1.0));
                    let (t0v, t1v) = (t0.as_f64().unwrap_or(0.0), t1.as_f64().unwrap_or(1.0));
                    let t = if f1 == f0 {
                        0.0
                    } else {
                        (v.as_f64().unwrap_or(0.0) - f0) / (f1 - f0)
                    };
                    Ok(like(t0, t0v + t * (t1v - t0v)))
                }
                _ => Err(fault("ContractViolation")),
            },
            "Ok" => Ok(Value::Res(Ok(Box::new(a0.unwrap_or(Value::Unit))))),
            "Err" => Ok(Value::Res(Err(Box::new(a0.unwrap_or(Value::Unit))))),
            "Some" => Ok(Value::Opt(Some(Box::new(a0.unwrap_or(Value::Unit))))),
            "xyz" | "xy" | "rpy" | "joints" => {
                let angle = matches!(name, "rpy" | "joints");
                let v: Vec<f64> = vals.iter().map(|(_, v)| geo_comp(v, angle)).collect();
                Ok(Value::Geo(Rc::new(if name == "joints" {
                    Geo::Joints(v)
                } else {
                    Geo::Vec {
                        kind: name.to_string(),
                        v,
                    }
                })))
            }
            "frame" => Ok(Value::Geo(Rc::new(Geo::Frame {
                name: String::new(),
                origin: geo_vec(&arg(&vals, "origin", 0), 3),
                rot: geo_vec(&arg(&vals, "rot", 1), 3),
            }))),
            "tool" => Ok(Value::Geo(Rc::new(Geo::Tool {
                offset: geo_vec(&arg(&vals, "offset", 0), 3),
                payload_kg: round9(num(&arg(&vals, "payload", 1))),
            }))),
            "drive" => self.test_drive(&vals),
            "advance" => self.test_advance(&vals),
            "elapsed" => r(self.now as f64 / 1.0e9),
            "current_fault" => Ok(Value::Str(self.fault.clone().unwrap_or_default())),
            "state_of" => Ok(Value::Str(self.fsm_state_name(&match a0 {
                Some(v) => v.show(),
                None => String::new(),
            }))),
            other => match self.make_block(other, &vals) {
                Some(b) => Ok(b),
                None => Ok(Value::Opaque(other.to_string())),
            },
        }
    }

    // ---- function blocks ----------------------------------------------------------

    fn make_block(&mut self, name: &str, vals: &Vals) -> Option<Value> {
        let g = |n: &str, p: usize| arg(vals, n, p);
        let mut f: Vec<(String, Value)> = Vec::new();
        match name {
            "ton" | "tof" | "tp" => {
                f.push(("preset".into(), Value::Real(num(&g("preset", 0)))));
                f.extend(bools(&["input", "q", "prev", "running"]));
                f.extend(reals(&["et"]));
            }
            "ctu" | "ctd" | "ctud" => {
                f.push((
                    "preset".into(),
                    Value::Int(g("preset", 0).and_then(|v| v.as_int()).unwrap_or(0)),
                ));
                f.extend(bools(&[
                    "input",
                    "down",
                    "reset",
                    "load",
                    "q",
                    "qu",
                    "qd",
                    "prev",
                    "prev_down",
                ]));
                f.push(("cv".into(), Value::Int(0)));
            }
            "r_trig" | "f_trig" => f.extend(bools(&["input", "q", "prev"])),
            "sr" | "rs" => f.extend(bools(&["set", "reset", "q", "input", "prev"])),
            "pid" => {
                f.push(("kp".into(), Value::Real(num(&g("kp", 0)))));
                f.push(("ki".into(), Value::Real(num(&g("ki", 1)))));
                f.push(("kd".into(), Value::Real(num(&g("kd", 2)))));
                f.push((
                    "out".into(),
                    g("out", 3).unwrap_or(Value::Range(
                        Box::new(Value::Real(f64::NEG_INFINITY)),
                        Box::new(Value::Real(f64::INFINITY)),
                        true,
                    )),
                ));
                f.push((
                    "antiwindup".into(),
                    Value::Bool(g("antiwindup", 4).is_none_or(|v| v.truthy())),
                ));
                f.extend(reals(&["integral", "last"]));
            }
            "lowpass" | "highpass" => {
                f.push(("cutoff".into(), Value::Real(num(&g("cutoff", 0)))));
                f.extend(reals(&["y", "x"]));
                f.extend(bools(&["started"]));
            }
            "moving_average" => {
                let w = g("window", 0).and_then(|v| v.as_int()).unwrap_or(4).max(1);
                f.push(("window".into(), Value::Int(w)));
                f.push(("buf".into(), Value::Array(Vec::new())));
            }
            "hysteresis" => {
                f.push(("low".into(), Value::Real(num(&g("low", 0)))));
                f.push(("high".into(), Value::Real(num(&g("high", 1)))));
                f.extend(bools(&["q"]));
            }
            "debounce" => {
                f.push(("time".into(), Value::Real(num(&g("time", 0)))));
                f.extend(bools(&["stable"]));
                f.push(("since".into(), Value::Real(-1.0)));
            }
            "ramp" => {
                f.push(("rate".into(), Value::Real(num(&g("rate", 0)))));
                f.extend(reals(&["value"]));
            }
            _ => return None,
        }
        Some(new_obj("", name, true, f))
    }

    pub(crate) fn native_call(
        &mut self,
        rc: &Rc<RefCell<Obj>>,
        method: &str,
        vals: &Vals,
    ) -> R<Value> {
        let x = vals.first().map(|(_, v)| v.clone());
        let xf = x.as_ref().and_then(Value::as_f64).unwrap_or(0.0);
        let dt = self.dt();
        let now_s = self.now as f64 / 1.0e9;
        let name = rc.borrow().name.clone();
        let call = matches!(method, "step" | "update" | "call");
        let mut o = rc.borrow_mut();
        match (name.as_str(), method) {
            ("ton" | "tof" | "tp" | "r_trig" | "f_trig", _) if call => {
                o.set("input", Value::Bool(x.is_some_and(|v| v.truthy())));
                tick(&mut o, dt);
                Ok(o.get("q").cloned().unwrap_or(Value::Bool(false)))
            }
            ("ctu" | "ctd" | "ctud", _) if call => {
                o.set("input", Value::Bool(x.is_some_and(|v| v.truthy())));
                tick(&mut o, dt);
                Ok(o.get("cv").cloned().unwrap_or(Value::Int(0)))
            }
            ("sr" | "rs", _) if call => {
                o.set("set", Value::Bool(x.is_some_and(|v| v.truthy())));
                if let Some((_, r)) = vals.get(1) {
                    o.set("reset", Value::Bool(r.truthy()));
                }
                tick(&mut o, dt);
                Ok(o.get("q").cloned().unwrap_or(Value::Bool(false)))
            }
            ("pid", _) if call => {
                let (lo, hi, unit) = match o.get("out") {
                    Some(Value::Range(a, b, _)) => match (&**a, &**b) {
                        (Value::Qty(la, u), Value::Qty(hb, _)) => {
                            (u.from_si(*la), u.from_si(*hb), Some(Rc::clone(u)))
                        }
                        (a, b) => (
                            a.as_f64().unwrap_or(f64::NEG_INFINITY),
                            b.as_f64().unwrap_or(f64::INFINITY),
                            None,
                        ),
                    },
                    _ => (f64::NEG_INFINITY, f64::INFINITY, None),
                };
                let (kp, ki, kd) = (getf(&o, "kp"), getf(&o, "ki"), getf(&o, "kd"));
                let integral = getf(&o, "integral");
                let last = getf(&o, "last");
                let derivative = if dt > 0.0 { kd * (xf - last) / dt } else { 0.0 };
                let raw = kp * xf + integral + derivative;
                let out = raw.clamp(lo, hi);
                let saturated = (raw - out).abs() > f64::EPSILON;
                if !(getb(&o, "antiwindup") && saturated) {
                    o.set("integral", Value::Real(integral + ki * xf * dt));
                }
                o.set("last", Value::Real(xf));
                Ok(match unit {
                    Some(u) => Value::Qty(u.to_si(out), u),
                    None => Value::Real(out),
                })
            }
            ("lowpass" | "highpass", _) if call => {
                let fc = getf(&o, "cutoff");
                let rc_t = if fc > 0.0 {
                    1.0 / (2.0 * std::f64::consts::PI * fc)
                } else {
                    0.0
                };
                let a = if rc_t + dt > 0.0 {
                    dt / (rc_t + dt)
                } else {
                    1.0
                };
                let (y, px) = (getf(&o, "y"), getf(&o, "x"));
                let started = getb(&o, "started");
                let ny = match (name.as_str(), started) {
                    (_, false) => {
                        if name == "lowpass" {
                            xf
                        } else {
                            0.0
                        }
                    }
                    ("lowpass", true) => y + a * (xf - y),
                    _ => (1.0 - a) * (y + xf - px),
                };
                o.set("y", Value::Real(ny));
                o.set("x", Value::Real(xf));
                o.set("started", Value::Bool(true));
                Ok(x.map_or(Value::Real(ny), |t| like(&t, ny)))
            }
            ("moving_average", _) if call => {
                let w = usize::try_from(o.get("window").and_then(Value::as_int).unwrap_or(1))
                    .unwrap_or(1)
                    .max(1);
                let mut buf = match o.get("buf") {
                    Some(Value::Array(b)) => b.clone(),
                    _ => Vec::new(),
                };
                buf.push(Value::Real(xf));
                while buf.len() > w {
                    buf.remove(0);
                }
                let mean = buf.iter().filter_map(Value::as_f64).sum::<f64>() / buf.len() as f64;
                o.set("buf", Value::Array(buf));
                Ok(x.map_or(Value::Real(mean), |t| like(&t, mean)))
            }
            ("hysteresis", _) if call => {
                let (lo, hi) = (getf(&o, "low"), getf(&o, "high"));
                let mut q = getb(&o, "q");
                if xf >= hi {
                    q = true;
                } else if xf <= lo {
                    q = false;
                }
                o.set("q", Value::Bool(q));
                Ok(Value::Bool(q))
            }
            ("debounce", _) if call => {
                let raw = x.is_some_and(|v| v.truthy());
                let stable = getb(&o, "stable");
                let since = getf(&o, "since");
                if raw == stable {
                    o.set("since", Value::Real(-1.0));
                } else if since < 0.0 {
                    o.set("since", Value::Real(now_s));
                } else if now_s - since >= getf(&o, "time") - 1e-12 {
                    o.set("stable", Value::Bool(raw));
                    o.set("since", Value::Real(-1.0));
                }
                Ok(o.get("stable").cloned().unwrap_or(Value::Bool(false)))
            }
            ("ramp", _) if call => {
                let step = getf(&o, "rate").abs() * dt;
                let v = getf(&o, "value");
                let nv = v + (xf - v).clamp(-step, step);
                o.set("value", Value::Real(nv));
                Ok(x.map_or(Value::Real(nv), |t| like(&t, nv)))
            }
            (_, "reset") => {
                for k in ["integral", "last", "y", "x", "et", "value"] {
                    if o.get(k).is_some() {
                        o.set(k, Value::Real(0.0));
                    }
                }
                for k in ["q", "started", "stable", "running"] {
                    if o.get(k).is_some() {
                        o.set(k, Value::Bool(false));
                    }
                }
                if o.get("cv").is_some() {
                    o.set("cv", Value::Int(0));
                }
                Ok(Value::Unit)
            }
            _ => Err(fault("ContractViolation")),
        }
    }

    /// Advances every function block owned by module `m` (property style: `jam.input = x`).
    pub(crate) fn tick_module_blocks(&mut self, m: usize, dt: f64) {
        let ids = self.module_blocks.get(&m).cloned().unwrap_or_default();
        for id in ids {
            if let Some(Value::Obj(rc)) = self.globals.get(&id) {
                tick(&mut rc.borrow_mut(), dt);
            }
        }
    }

    // ---- io bindings --------------------------------------------------------------

    pub(crate) fn io_is_input(kind: &str) -> bool {
        !(kind.ends_with("output")
            || matches!(
                kind,
                "pwm" | "dac" | "uart" | "can" | "usb_cdc" | "watchdog"
            ))
    }

    pub(crate) fn io_read(&self, k: &str) -> Value {
        let kind = self.io.get(k).map(|s| s.kind.clone()).unwrap_or_default();
        if Self::io_is_input(&kind) {
            if let Some(v) = self.input_get(k) {
                return v;
            }
            return if kind.starts_with("adc")
                || kind.starts_with("capture")
                || kind.starts_with("encoder")
            {
                Value::Int(0)
            } else {
                Value::Bool(false)
            };
        }
        self.io
            .get(k)
            .and_then(|s| s.value.clone())
            .unwrap_or(Value::Bool(false))
    }

    pub(crate) fn io_write(&mut self, k: &str, v: Value) {
        let Some(s) = self.io.get(k) else { return };
        let changed = s.value.as_ref().is_none_or(|p| !p.same(&v));
        let name = s.name.clone();
        if let Some(s) = self.io.get_mut(k) {
            s.value = Some(v.clone());
        }
        if changed {
            self.trace_push(TraceKind::Output, name, v.show());
        }
    }

    pub(crate) fn io_set_prop(&mut self, k: &str, prop: &str, v: Value) -> R<()> {
        let Some(s) = self.io.get(k) else {
            return Ok(());
        };
        let changed = s.props.get(prop).is_none_or(|p| !p.same(&v));
        let name = format!("{}.{prop}", s.name);
        if let Some(s) = self.io.get_mut(k) {
            s.props.insert(prop.to_string(), v.clone());
        }
        if changed {
            self.trace_push(TraceKind::Output, name, v.show());
        }
        Ok(())
    }

    pub(crate) fn io_prop(&self, k: &str, name: &str) -> Value {
        match name {
            "value" => self.io_read(k),
            _ => self
                .io
                .get(k)
                .and_then(|s| s.props.get(name).cloned())
                .unwrap_or(Value::Unit),
        }
    }

    pub(crate) fn io_call(&mut self, k: &str, method: &str, vals: &Vals) -> R<Value> {
        let a0 = vals.first().map(|(_, v)| v.clone());
        match method {
            "write" | "set" => {
                self.io_write(k, a0.unwrap_or(Value::Bool(false)));
                Ok(Value::Unit)
            }
            "toggle" => {
                let cur = self
                    .io
                    .get(k)
                    .and_then(|s| s.value.clone())
                    .is_some_and(|v| v.truthy());
                self.io_write(k, Value::Bool(!cur));
                Ok(Value::Unit)
            }
            "read" | "get" => Ok(self.io_read(k)),
            "set_duty" => {
                self.io_set_prop(k, "duty", a0.unwrap_or(Value::Real(0.0)))?;
                Ok(Value::Unit)
            }
            "print" | "println" | "send" | "write_str" => {
                let name = self.io.get(k).map(|s| s.name.clone()).unwrap_or_default();
                let text = a0.map(|v| v.show()).unwrap_or_default();
                self.trace_push(TraceKind::Output, format!("{name}.{method}"), text);
                Ok(Value::Unit)
            }
            _ => Ok(Value::Unit),
        }
    }
}
