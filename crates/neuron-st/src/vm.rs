//! The virtual PLC: scans a program in simulated time and records its output changes.

use crate::ast::{Arg, CaseLabel, Expr, Init, IntTy, Pou, PouKind, Section, Stmt, Ty, Unit};
use std::collections::BTreeMap;
use std::fmt;

/// How the standard timers measure time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    /// A timer advances by one scan period per call, from the first call with its input
    /// true (the rule of the Neuron reference interpreter, spec 14.3).
    ScanCounted,
    /// A timer measures the time since the scan in which its input rose (IEC 61131-3).
    WallClock,
}

/// A run-time error of the PLC (division by zero, unknown name, loop limit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmError(pub String);

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VmError {}

/// A value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `BOOL`.
    Bool(bool),
    /// Integer with its type.
    Int(i64, IntTy),
    /// `REAL` / `LREAL`.
    Real(f64),
    /// `TIME` in nanoseconds.
    Time(i64),
    /// `STRING`.
    Str(String),
    /// Function block instance (index into the instance table).
    Fb(usize),
    /// Array.
    Array(Vec<Value>),
}

impl Value {
    fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(v, _) => *v != 0,
            Value::Real(v) => *v != 0.0,
            _ => false,
        }
    }
    fn as_f64(&self) -> f64 {
        match self {
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Int(v, _) => *v as f64,
            Value::Real(v) => *v,
            Value::Time(t) => *t as f64,
            _ => 0.0,
        }
    }
    fn as_i64(&self) -> i64 {
        match self {
            Value::Bool(b) => i64::from(*b),
            Value::Int(v, _) => *v,
            Value::Real(v) => v.trunc() as i64,
            Value::Time(t) => *t,
            _ => 0,
        }
    }
    /// The value as the reference interpreter prints it (`true`, `42`, `1.5`).
    pub fn show(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Int(v, _) => v.to_string(),
            Value::Real(v) => format!("{v}"),
            Value::Time(t) => format!("{} ms", *t as f64 / 1e6),
            Value::Str(s) => s.clone(),
            Value::Fb(_) => "<fb>".into(),
            Value::Array(a) => format!(
                "[{}]",
                a.iter().map(Value::show).collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

const DINT: IntTy = IntTy {
    bits: 32,
    signed: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StdFb {
    Ton,
    Tof,
    Tp,
    Ctu,
    Ctd,
    Ctud,
    RTrig,
    FTrig,
    Sr,
    Rs,
}

fn std_fb(name: &str) -> Option<StdFb> {
    Some(match name.to_ascii_uppercase().as_str() {
        "TON" | "TON_TIME" => StdFb::Ton,
        "TOF" | "TOF_TIME" => StdFb::Tof,
        "TP" | "TP_TIME" => StdFb::Tp,
        "CTU" | "CTU_INT" | "CTU_DINT" => StdFb::Ctu,
        "CTD" | "CTD_INT" | "CTD_DINT" => StdFb::Ctd,
        "CTUD" | "CTUD_INT" | "CTUD_DINT" => StdFb::Ctud,
        "R_TRIG" => StdFb::RTrig,
        "F_TRIG" => StdFb::FTrig,
        "SR" => StdFb::Sr,
        "RS" => StdFb::Rs,
        _ => return None,
    })
}

#[derive(Debug, Clone)]
enum Inst {
    Std {
        kind: StdFb,
        f: BTreeMap<String, Value>,
        start: Option<i64>,
        mem: bool,
        mem2: bool,
        running: bool,
    },
    User {
        pou: usize,
        vars: BTreeMap<String, Value>,
    },
}

enum Flow {
    Next,
    Exit,
    Return,
}

/// One recorded output change.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// Instant in nanoseconds.
    pub t_ns: i64,
    /// Variable name as declared.
    pub name: String,
    /// New value.
    pub value: Value,
}

/// A PLC running one program.
pub struct Plc {
    unit: Unit,
    prog: usize,
    /// Scan period in nanoseconds.
    pub period: i64,
    clock: Clock,
    now: i64,
    vars: BTreeMap<String, Value>,
    names: BTreeMap<String, String>,
    insts: Vec<Inst>,
    inputs: Vec<String>,
    outputs: Vec<String>,
    image: BTreeMap<String, Value>,
    last: BTreeMap<String, Value>,
    trace: Vec<Change>,
    budget: u64,
}

fn key(s: &str) -> String {
    s.trim_start_matches('#').to_ascii_lowercase()
}

impl Plc {
    /// Loads program `name` (or the first program) of a unit. The scan period comes from
    /// the `CONFIGURATION` when it has one, otherwise from `period_ns`.
    ///
    /// # Errors
    /// Fails when there is no such program or an initial value cannot be computed.
    pub fn new(
        unit: Unit,
        name: Option<&str>,
        period_ns: i64,
        clock: Clock,
    ) -> Result<Plc, VmError> {
        let prog = unit
            .pous
            .iter()
            .position(|p| {
                p.kind == PouKind::Program
                    && p.name != "GlobalVars"
                    && name.is_none_or(|n| p.name.eq_ignore_ascii_case(n))
            })
            .ok_or_else(|| VmError("no PROGRAM in the source".into()))?;
        let pname = unit.pous[prog].name.clone();
        let period = unit
            .tasks
            .iter()
            .find(|(t, _)| t.eq_ignore_ascii_case(&pname))
            .map_or(period_ns, |(_, p)| *p)
            .max(1);
        let mut plc = Plc {
            unit,
            prog,
            period,
            clock,
            now: 0,
            vars: BTreeMap::new(),
            names: BTreeMap::new(),
            insts: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            image: BTreeMap::new(),
            last: BTreeMap::new(),
            trace: Vec::new(),
            budget: 0,
        };
        // Global variable lists are part of the program's scope.
        let globals: Vec<usize> = plc
            .unit
            .pous
            .iter()
            .enumerate()
            .filter(|(_, p)| p.name == "GlobalVars")
            .map(|(i, _)| i)
            .collect();
        for g in globals.into_iter().chain(std::iter::once(prog)) {
            let decls = plc.unit.pous[g].vars.clone();
            for d in decls {
                let k = key(&d.name);
                let v = plc.init_value(&d.ty, d.init.as_ref())?;
                plc.vars.insert(k.clone(), v);
                plc.names.insert(k.clone(), d.name.clone());
                let addr = d.at.clone().unwrap_or_default();
                if addr.starts_with("%I") || d.section == Section::Input {
                    plc.inputs.push(k);
                } else if addr.starts_with("%Q") || d.section == Section::Output {
                    plc.outputs.push(k);
                }
            }
        }
        Ok(plc)
    }

    fn init_value(&mut self, ty: &Ty, init: Option<&Init>) -> Result<Value, VmError> {
        let empty = BTreeMap::new();
        let mut v = match ty {
            Ty::Bool => Value::Bool(false),
            Ty::Int(t) => Value::Int(0, *t),
            Ty::Real(_) => Value::Real(0.0),
            Ty::Time => Value::Time(0),
            Ty::Str => Value::Str(String::new()),
            Ty::Array(lo, hi, elem) => {
                let n = usize::try_from((hi - lo + 1).max(0)).unwrap_or(0);
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.init_value(elem, None)?);
                }
                Value::Array(items)
            }
            Ty::Named(n) => self.new_instance(n)?,
        };
        match init {
            Some(Init::Expr(e)) => {
                let x = self.eval(e, &empty)?;
                v = coerce(&v, x);
            }
            Some(Init::Struct(fields)) => {
                if let Value::Fb(id) = v {
                    for (f, e) in fields {
                        let x = self.eval(e, &empty)?;
                        self.set_field(id, f, x);
                    }
                }
            }
            None => {}
        }
        Ok(v)
    }

    fn new_instance(&mut self, ty: &str) -> Result<Value, VmError> {
        if let Some(kind) = std_fb(ty) {
            let mut f = BTreeMap::new();
            let b = |n: &str| (n.to_string(), Value::Bool(false));
            let t = |n: &str| (n.to_string(), Value::Time(0));
            let i = |n: &str| (n.to_string(), Value::Int(0, DINT));
            let fields: Vec<(String, Value)> = match kind {
                StdFb::Ton | StdFb::Tof | StdFb::Tp => vec![b("in"), b("q"), t("pt"), t("et")],
                StdFb::Ctu => vec![b("cu"), b("r"), i("pv"), b("q"), i("cv")],
                StdFb::Ctd => vec![b("cd"), b("ld"), i("pv"), b("q"), i("cv")],
                StdFb::Ctud => vec![
                    b("cu"),
                    b("cd"),
                    b("r"),
                    b("ld"),
                    i("pv"),
                    b("qu"),
                    b("qd"),
                    i("cv"),
                ],
                StdFb::RTrig | StdFb::FTrig => vec![b("clk"), b("q")],
                StdFb::Sr => vec![b("s1"), b("r"), b("q1")],
                StdFb::Rs => vec![b("s"), b("r1"), b("q1")],
            };
            f.extend(fields);
            self.insts.push(Inst::Std {
                kind,
                f,
                start: None,
                mem: false,
                mem2: false,
                running: false,
            });
            return Ok(Value::Fb(self.insts.len() - 1));
        }
        let pou = self
            .unit
            .pous
            .iter()
            .position(|p| p.kind == PouKind::FunctionBlock && p.name.eq_ignore_ascii_case(ty))
            .ok_or_else(|| VmError(format!("unknown type `{ty}`")))?;
        let decls = self.unit.pous[pou].vars.clone();
        let mut vars = BTreeMap::new();
        for d in decls {
            let v = self.init_value(&d.ty, d.init.as_ref())?;
            vars.insert(key(&d.name), v);
        }
        self.insts.push(Inst::User { pou, vars });
        Ok(Value::Fb(self.insts.len() - 1))
    }

    /// Sets an input for the next scans.
    pub fn set_input(&mut self, name: &str, v: Value) {
        self.image.insert(key(name), v);
    }

    /// Value of a variable of the program.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.vars.get(&key(name))
    }

    /// Current instant in nanoseconds.
    pub fn now(&self) -> i64 {
        self.now
    }

    /// Output changes so far.
    pub fn trace(&self) -> &[Change] {
        &self.trace
    }

    /// Runs one scan at the current instant and advances time by one period.
    ///
    /// # Errors
    /// A run-time error of the program.
    pub fn scan(&mut self) -> Result<(), VmError> {
        for k in self.inputs.clone() {
            if let Some(v) = self.image.get(&k).cloned() {
                let old = self.vars.get(&k).cloned().unwrap_or(Value::Bool(false));
                self.vars.insert(k, coerce(&old, v));
            }
        }
        self.budget = 0;
        let body = self.unit.pous[self.prog].body.clone();
        let mut scope = std::mem::take(&mut self.vars);
        let r = self.exec_block(&body, &mut scope);
        self.vars = scope;
        r?;
        for k in self.outputs.clone() {
            let v = self.vars.get(&k).cloned().unwrap_or(Value::Bool(false));
            if self.last.get(&k) != Some(&v) {
                self.trace.push(Change {
                    t_ns: self.now,
                    name: self.names.get(&k).cloned().unwrap_or(k.clone()),
                    value: v.clone(),
                });
                self.last.insert(k, v);
            }
        }
        self.now += self.period;
        Ok(())
    }

    /// Scans until the instant `end_ns` (exclusive), applying `inputs` (instant, name,
    /// value) as their instants pass.
    ///
    /// # Errors
    /// A run-time error of the program.
    pub fn run(&mut self, end_ns: i64, inputs: &[(i64, String, Value)]) -> Result<(), VmError> {
        while self.now < end_ns {
            for (t, n, v) in inputs {
                if *t <= self.now && *t > self.now - self.period {
                    self.set_input(n, v.clone());
                }
            }
            if self.now == 0 {
                for (t, n, v) in inputs {
                    if *t <= 0 {
                        self.set_input(n, v.clone());
                    }
                }
            }
            self.scan()?;
        }
        Ok(())
    }

    // ---- statements ---------------------------------------------------------------

    fn exec_block(
        &mut self,
        b: &[Stmt],
        scope: &mut BTreeMap<String, Value>,
    ) -> Result<Flow, VmError> {
        for s in b {
            match self.exec(s, scope)? {
                Flow::Next => {}
                f => return Ok(f),
            }
        }
        Ok(Flow::Next)
    }

    fn tick(&mut self) -> Result<(), VmError> {
        self.budget += 1;
        if self.budget > 1_000_000 {
            return Err(VmError("loop limit reached in one scan".into()));
        }
        Ok(())
    }

    fn exec(&mut self, s: &Stmt, scope: &mut BTreeMap<String, Value>) -> Result<Flow, VmError> {
        self.tick()?;
        match s {
            Stmt::Assign(place, value, _) => {
                let v = self.eval(value, scope)?;
                self.assign(place, v, scope)?;
            }
            Stmt::Call(target, args, _) => {
                let Expr::Var(name) = target else {
                    return Err(VmError("bad call".into()));
                };
                self.call_instance(name, args, scope)?;
            }
            Stmt::If(branches, otherwise, _) => {
                for (c, body) in branches {
                    if self.eval(c, scope)?.truthy() {
                        return self.exec_block(body, scope);
                    }
                }
                if let Some(b) = otherwise {
                    return self.exec_block(b, scope);
                }
            }
            Stmt::Case(e, arms, otherwise, _) => {
                let v = self.eval(e, scope)?.as_i64();
                for (labels, body) in arms {
                    let hit = labels.iter().any(|l| match l {
                        CaseLabel::Value(x) => *x == v,
                        CaseLabel::Range(a, b) => (*a..=*b).contains(&v),
                    });
                    if hit {
                        return self.exec_block(body, scope);
                    }
                }
                if let Some(b) = otherwise {
                    return self.exec_block(b, scope);
                }
            }
            Stmt::For(var, a, b, by, body, _) => {
                let from = self.eval(a, scope)?.as_i64();
                let to = self.eval(b, scope)?.as_i64();
                let step = match by {
                    Some(e) => self.eval(e, scope)?.as_i64(),
                    None => 1,
                };
                if step == 0 {
                    return Err(VmError("FOR with a step of zero".into()));
                }
                let k = key(var);
                let ty = match scope.get(&k) {
                    Some(Value::Int(_, t)) => *t,
                    _ => DINT,
                };
                let mut i = from;
                while (step > 0 && i <= to) || (step < 0 && i >= to) {
                    scope.insert(k.clone(), Value::Int(i, ty));
                    match self.exec_block(body, scope)? {
                        Flow::Exit => break,
                        Flow::Return => return Ok(Flow::Return),
                        Flow::Next => {}
                    }
                    self.tick()?;
                    i += step;
                }
            }
            Stmt::While(c, body, _) => {
                while self.eval(c, scope)?.truthy() {
                    match self.exec_block(body, scope)? {
                        Flow::Exit => break,
                        Flow::Return => return Ok(Flow::Return),
                        Flow::Next => {}
                    }
                    self.tick()?;
                }
            }
            Stmt::Repeat(body, c, _) => loop {
                match self.exec_block(body, scope)? {
                    Flow::Exit => break,
                    Flow::Return => return Ok(Flow::Return),
                    Flow::Next => {}
                }
                if self.eval(c, scope)?.truthy() {
                    break;
                }
                self.tick()?;
            },
            Stmt::Exit(_) => return Ok(Flow::Exit),
            Stmt::Return(_) => return Ok(Flow::Return),
        }
        Ok(Flow::Next)
    }

    fn lookup(&self, name: &str, scope: &BTreeMap<String, Value>) -> Option<Value> {
        let k = key(name);
        scope.get(&k).or_else(|| self.vars.get(&k)).cloned()
    }

    fn assign(
        &mut self,
        place: &Expr,
        v: Value,
        scope: &mut BTreeMap<String, Value>,
    ) -> Result<(), VmError> {
        match place {
            Expr::Var(n) => {
                let k = key(n);
                let target = if scope.contains_key(&k) {
                    Some(&mut *scope)
                } else if self.vars.contains_key(&k) {
                    Some(&mut self.vars)
                } else {
                    None
                };
                let Some(map) = target else {
                    return Err(VmError(format!("unknown variable `{n}`")));
                };
                let old = map.get(&k).cloned().unwrap_or(Value::Bool(false));
                map.insert(k, coerce(&old, v));
                Ok(())
            }
            Expr::Field(base, f) => match self.eval(base, scope)? {
                Value::Fb(id) => {
                    self.set_field(id, f, v);
                    Ok(())
                }
                _ => Err(VmError(format!("`{f}` is not a field of a function block"))),
            },
            Expr::Index(base, ix) => {
                let i = self.eval(ix, scope)?.as_i64();
                let Expr::Var(n) = &**base else {
                    return Err(VmError("unsupported array target".into()));
                };
                let k = key(n);
                let lo = self.array_lo(n);
                let map = if scope.contains_key(&k) {
                    &mut *scope
                } else {
                    &mut self.vars
                };
                match map.get_mut(&k) {
                    Some(Value::Array(items)) => {
                        let at = usize::try_from(i - lo)
                            .map_err(|_| VmError("index out of range".into()))?;
                        let slot = items
                            .get_mut(at)
                            .ok_or_else(|| VmError("index out of range".into()))?;
                        *slot = coerce(slot, v);
                        Ok(())
                    }
                    _ => Err(VmError(format!("`{n}` is not an array"))),
                }
            }
            _ => Err(VmError("cannot assign to this expression".into())),
        }
    }

    fn array_lo(&self, name: &str) -> i64 {
        for p in &self.unit.pous {
            for d in &p.vars {
                if d.name.eq_ignore_ascii_case(name.trim_start_matches('#')) {
                    if let Ty::Array(lo, _, _) = d.ty {
                        return lo;
                    }
                }
            }
        }
        0
    }

    fn set_field(&mut self, id: usize, f: &str, v: Value) {
        match &mut self.insts[id] {
            Inst::Std { f: fields, .. } => {
                let k = key(f);
                let old = fields.get(&k).cloned().unwrap_or(Value::Bool(false));
                fields.insert(k, coerce(&old, v));
            }
            Inst::User { vars, .. } => {
                let k = key(f);
                let old = vars.get(&k).cloned().unwrap_or(Value::Bool(false));
                vars.insert(k, coerce(&old, v));
            }
        }
    }

    fn get_field(&self, id: usize, f: &str) -> Option<Value> {
        match &self.insts[id] {
            Inst::Std { f: fields, .. } => fields.get(&key(f)).cloned(),
            Inst::User { vars, .. } => vars.get(&key(f)).cloned(),
        }
    }

    fn call_instance(
        &mut self,
        name: &str,
        args: &[Arg],
        scope: &mut BTreeMap<String, Value>,
    ) -> Result<(), VmError> {
        let Some(Value::Fb(id)) = self.lookup(name, scope) else {
            // A function called as a statement.
            self.call_fn(name, args, scope)?;
            return Ok(());
        };
        let mut outs = Vec::new();
        let pou_inputs: Vec<String> = match &self.insts[id] {
            Inst::User { pou, .. } => self.unit.pous[*pou]
                .vars
                .iter()
                .filter(|d| d.section == Section::Input)
                .map(|d| key(&d.name))
                .collect(),
            Inst::Std { .. } => Vec::new(),
        };
        for (k, a) in args.iter().enumerate() {
            if a.output {
                outs.push(a);
                continue;
            }
            let v = self.eval(&a.value, scope)?;
            let field = match &a.name {
                Some(n) => key(n),
                None => match pou_inputs.get(k) {
                    Some(n) => n.clone(),
                    None => return Err(VmError(format!("positional argument of `{name}`"))),
                },
            };
            self.set_field(id, &field, v);
        }
        self.run_instance(id)?;
        for a in outs {
            let Some(n) = &a.name else { continue };
            let v = self.get_field(id, n).unwrap_or(Value::Bool(false));
            self.assign(&a.value, v, scope)?;
        }
        Ok(())
    }

    fn run_instance(&mut self, id: usize) -> Result<(), VmError> {
        let (now, period, clock) = (self.now, self.period, self.clock);
        match &mut self.insts[id] {
            Inst::Std {
                kind,
                f,
                start,
                mem,
                mem2,
                running,
            } => {
                let gb = |f: &BTreeMap<String, Value>, k: &str| f.get(k).is_some_and(Value::truthy);
                let gt = |f: &BTreeMap<String, Value>, k: &str| f.get(k).map_or(0, Value::as_i64);
                match kind {
                    StdFb::Ton => {
                        let (input, pt) = (gb(f, "in"), gt(f, "pt"));
                        let et = match clock {
                            Clock::ScanCounted => {
                                if input {
                                    (gt(f, "et") + period).min(pt)
                                } else {
                                    0
                                }
                            }
                            Clock::WallClock => {
                                if input {
                                    let s = *start.get_or_insert(now);
                                    (now - s).min(pt)
                                } else {
                                    *start = None;
                                    0
                                }
                            }
                        };
                        f.insert("et".into(), Value::Time(et));
                        f.insert("q".into(), Value::Bool(input && et >= pt));
                    }
                    StdFb::Tof => {
                        let (input, pt) = (gb(f, "in"), gt(f, "pt"));
                        let mut q = gb(f, "q");
                        let mut et = gt(f, "et");
                        if input {
                            q = true;
                            et = 0;
                            *start = None;
                        } else if q {
                            et = match clock {
                                Clock::ScanCounted => et + period,
                                Clock::WallClock => now - *start.get_or_insert(now),
                            };
                            if et >= pt {
                                q = false;
                                et = pt;
                            }
                        }
                        f.insert("et".into(), Value::Time(et));
                        f.insert("q".into(), Value::Bool(q));
                    }
                    StdFb::Tp => {
                        let (input, pt) = (gb(f, "in"), gt(f, "pt"));
                        let mut q = gb(f, "q");
                        let mut et = gt(f, "et");
                        if input && !*mem && !*running {
                            *running = true;
                            *start = Some(now);
                            et = 0;
                            q = true;
                        }
                        if *running {
                            et = match clock {
                                Clock::ScanCounted => et + period,
                                Clock::WallClock => now - start.unwrap_or(now),
                            };
                            if et >= pt {
                                *running = false;
                                q = false;
                            }
                        }
                        *mem = input;
                        f.insert("et".into(), Value::Time(et));
                        f.insert("q".into(), Value::Bool(q));
                    }
                    StdFb::Ctu | StdFb::Ctd | StdFb::Ctud => {
                        let up = gb(f, "cu");
                        let down = gb(f, "cd");
                        let mut cv = gt(f, "cv");
                        let pv = gt(f, "pv");
                        match kind {
                            StdFb::Ctu => {
                                if gb(f, "r") {
                                    cv = 0;
                                } else if up && !*mem {
                                    cv += 1;
                                }
                                f.insert("q".into(), Value::Bool(cv >= pv));
                            }
                            StdFb::Ctd => {
                                if gb(f, "ld") {
                                    cv = pv;
                                } else if down && !*mem2 {
                                    cv -= 1;
                                }
                                f.insert("q".into(), Value::Bool(cv <= 0));
                            }
                            _ => {
                                if gb(f, "r") {
                                    cv = 0;
                                } else if gb(f, "ld") {
                                    cv = pv;
                                } else {
                                    if up && !*mem {
                                        cv += 1;
                                    }
                                    if down && !*mem2 {
                                        cv -= 1;
                                    }
                                }
                                f.insert("qu".into(), Value::Bool(cv >= pv));
                                f.insert("qd".into(), Value::Bool(cv <= 0));
                            }
                        }
                        *mem = up;
                        *mem2 = down;
                        f.insert("cv".into(), Value::Int(cv, DINT));
                    }
                    StdFb::RTrig | StdFb::FTrig => {
                        let clk = gb(f, "clk");
                        let q = if *kind == StdFb::RTrig {
                            clk && !*mem
                        } else {
                            !clk && *mem
                        };
                        *mem = clk;
                        f.insert("q".into(), Value::Bool(q));
                    }
                    StdFb::Sr => {
                        let q = gb(f, "s1") || (gb(f, "q1") && !gb(f, "r"));
                        f.insert("q1".into(), Value::Bool(q));
                    }
                    StdFb::Rs => {
                        let q = !gb(f, "r1") && (gb(f, "s") || gb(f, "q1"));
                        f.insert("q1".into(), Value::Bool(q));
                    }
                }
                Ok(())
            }
            Inst::User { pou, vars } => {
                let pou = *pou;
                let mut scope = std::mem::take(vars);
                let body = self.unit.pous[pou].body.clone();
                let r = self.exec_block(&body, &mut scope);
                if let Inst::User { vars, .. } = &mut self.insts[id] {
                    *vars = scope;
                }
                r.map(|_| ())
            }
        }
    }

    // ---- expressions --------------------------------------------------------------

    fn eval(&mut self, e: &Expr, scope: &BTreeMap<String, Value>) -> Result<Value, VmError> {
        Ok(match e {
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Int(v) => Value::Int(*v, DINT),
            Expr::Real(v) => Value::Real(*v),
            Expr::Time(v) => Value::Time(*v),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Var(n) => self
                .lookup(n, scope)
                .ok_or_else(|| VmError(format!("unknown variable `{n}`")))?,
            Expr::Field(base, f) => match self.eval(base, scope)? {
                Value::Fb(id) => self
                    .get_field(id, f)
                    .ok_or_else(|| VmError(format!("no field `{f}`")))?,
                _ => return Err(VmError(format!("`{f}` is not a field of a function block"))),
            },
            Expr::Index(base, ix) => {
                let i = self.eval(ix, scope)?.as_i64();
                let lo = match &**base {
                    Expr::Var(n) => self.array_lo(n),
                    _ => 0,
                };
                // Indexing a variable reads the element in place: a table of a thousand
                // words is not copied for every read.
                if let Expr::Var(n) = &**base {
                    let k = key(n);
                    if let Some(Value::Array(items)) = scope.get(&k).or_else(|| self.vars.get(&k)) {
                        return usize::try_from(i - lo)
                            .ok()
                            .and_then(|at| items.get(at).cloned())
                            .ok_or_else(|| VmError("index out of range".into()));
                    }
                }
                match self.eval(base, scope)? {
                    Value::Array(items) => usize::try_from(i - lo)
                        .ok()
                        .and_then(|k| items.get(k).cloned())
                        .ok_or_else(|| VmError("index out of range".into()))?,
                    _ => return Err(VmError("indexing a value that is not an array".into())),
                }
            }
            Expr::Unary(op, x) => {
                let v = self.eval(x, scope)?;
                match (op.as_str(), v) {
                    ("NOT", Value::Bool(b)) => Value::Bool(!b),
                    ("NOT", Value::Int(i, t)) => Value::Int(t.wrap(!i), t),
                    ("-", Value::Int(i, t)) => Value::Int(t.wrap(-i), t),
                    ("-", Value::Real(r)) => Value::Real(-r),
                    ("-", Value::Time(r)) => Value::Time(-r),
                    _ => return Err(VmError(format!("invalid operand of {op}"))),
                }
            }
            Expr::Binary(op, a, b) => {
                let x = self.eval(a, scope)?;
                let y = self.eval(b, scope)?;
                binary(op, &x, &y)?
            }
            Expr::Call(name, args) => {
                let mut sc = scope.clone();
                self.call_fn(name, args, &mut sc)?
            }
        })
    }

    fn call_fn(
        &mut self,
        name: &str,
        args: &[Arg],
        scope: &mut BTreeMap<String, Value>,
    ) -> Result<Value, VmError> {
        let mut vals = Vec::new();
        for a in args {
            vals.push(self.eval(&a.value, scope)?);
        }
        let up = name.to_ascii_uppercase();
        let f = |i: usize| vals.get(i).map_or(0.0, Value::as_f64);
        let num1 = |r: f64| Ok(Value::Real(r));
        match up.as_str() {
            "ABS" => {
                return Ok(match vals.first() {
                    Some(Value::Int(v, t)) => Value::Int(t.wrap(v.abs()), *t),
                    Some(Value::Time(v)) => Value::Time(v.abs()),
                    _ => Value::Real(f(0).abs()),
                })
            }
            "SQRT" => return num1(f(0).sqrt()),
            "SIN" => return num1(f(0).sin()),
            "COS" => return num1(f(0).cos()),
            "TAN" => return num1(f(0).tan()),
            "ASIN" => return num1(f(0).asin()),
            "ACOS" => return num1(f(0).acos()),
            "ATAN" => return num1(f(0).atan()),
            "EXP" => return num1(f(0).exp()),
            "LN" => return num1(f(0).ln()),
            "LOG" => return num1(f(0).log10()),
            "EXPT" => return num1(f(0).powf(f(1))),
            "TRUNC" => return Ok(Value::Int(f(0).trunc() as i64, DINT)),
            "MIN" | "MAX" => {
                let mut best = vals
                    .first()
                    .cloned()
                    .ok_or_else(|| VmError(format!("{up} needs arguments")))?;
                for v in vals.iter().skip(1) {
                    let better = if up == "MIN" {
                        v.as_f64() < best.as_f64()
                    } else {
                        v.as_f64() > best.as_f64()
                    };
                    if better {
                        best = v.clone();
                    }
                }
                return Ok(best);
            }
            "LIMIT" => {
                let (lo, x, hi) = (f(0), f(1), f(2));
                let pick = if x < lo {
                    0
                } else if x > hi {
                    2
                } else {
                    1
                };
                return vals
                    .get(pick)
                    .cloned()
                    .ok_or_else(|| VmError("LIMIT needs 3 arguments".into()));
            }
            "SEL" => {
                let g = vals.first().is_some_and(Value::truthy);
                return vals
                    .get(if g { 2 } else { 1 })
                    .cloned()
                    .ok_or_else(|| VmError("SEL needs 3 arguments".into()));
            }
            "SHL" | "SHR" => {
                let (x, n) = (
                    vals.first().cloned().unwrap_or(Value::Int(0, DINT)),
                    f(1) as u32,
                );
                if let Value::Int(v, t) = x {
                    let r = if up == "SHL" {
                        v.wrapping_shl(n)
                    } else {
                        ((v as u64) >> n) as i64
                    };
                    return Ok(Value::Int(t.wrap(r), t));
                }
                return Err(VmError(format!("{up} needs an integer")));
            }
            _ => {}
        }
        if let Some(target) = up
            .strip_prefix("TO_")
            .or_else(|| up.split_once("_TO_").map(|(_, t)| t))
        {
            let v = vals.first().cloned().unwrap_or(Value::Int(0, DINT));
            return convert(&v, target);
        }
        // A user function.
        if let Some(pi) = self
            .unit
            .pous
            .iter()
            .position(|p| p.kind == PouKind::Function && p.name.eq_ignore_ascii_case(name))
        {
            let pou: Pou = self.unit.pous[pi].clone();
            let mut local = BTreeMap::new();
            let inputs: Vec<_> = pou
                .vars
                .iter()
                .filter(|d| d.section == Section::Input)
                .collect();
            for d in &pou.vars {
                let v = self.init_value(&d.ty, d.init.as_ref())?;
                local.insert(key(&d.name), v);
            }
            for (k, a) in args.iter().enumerate() {
                let v = vals[k].clone();
                let field = match &a.name {
                    Some(n) => key(n),
                    None => inputs.get(k).map(|d| key(&d.name)).unwrap_or_default(),
                };
                let old = local.get(&field).cloned().unwrap_or(Value::Bool(false));
                local.insert(field, coerce(&old, v));
            }
            let ret_key = key(&pou.name);
            let ret = match &pou.ret {
                Some(t) => self.init_value(t, None)?,
                None => Value::Bool(false),
            };
            local.insert(ret_key.clone(), ret);
            self.exec_block(&pou.body, &mut local)?;
            return Ok(local.remove(&ret_key).unwrap_or(Value::Bool(false)));
        }
        Err(VmError(format!("unknown function `{name}`")))
    }
}

fn convert(v: &Value, target: &str) -> Result<Value, VmError> {
    let int = |bits: u8, signed: bool| {
        let t = IntTy { bits, signed };
        let x = match v {
            Value::Real(r) => r.round() as i64,
            other => other.as_i64(),
        };
        Ok(Value::Int(t.wrap(x), t))
    };
    match target {
        "BOOL" => Ok(Value::Bool(v.truthy())),
        "SINT" => int(8, true),
        "INT" => int(16, true),
        "DINT" => int(32, true),
        "LINT" => int(64, true),
        "USINT" | "BYTE" => int(8, false),
        "UINT" | "WORD" => int(16, false),
        "UDINT" | "DWORD" => int(32, false),
        "ULINT" | "LWORD" => int(64, false),
        "REAL" | "LREAL" => Ok(Value::Real(v.as_f64())),
        "TIME" => Ok(Value::Time(v.as_i64())),
        _ => Err(VmError(format!("unknown conversion to {target}"))),
    }
}

/// Converts `v` to the type of `like` (assignment conversion).
fn coerce(like: &Value, v: Value) -> Value {
    match (like, v) {
        (Value::Int(_, t), Value::Int(x, _)) => Value::Int(t.wrap(x), *t),
        (Value::Int(_, t), Value::Bool(b)) => Value::Int(i64::from(b), *t),
        (Value::Int(_, t), Value::Real(r)) => Value::Int(t.wrap(r.round() as i64), *t),
        (Value::Real(_), Value::Int(x, _)) => Value::Real(x as f64),
        (Value::Bool(_), Value::Int(x, _)) => Value::Bool(x != 0),
        (_, v) => v,
    }
}

fn binary(op: &str, x: &Value, y: &Value) -> Result<Value, VmError> {
    use Value::{Bool, Int, Real, Time};
    Ok(match op {
        "AND" | "OR" | "XOR" => match (x, y) {
            (Int(a, t), Int(b, _)) => {
                let r = match op {
                    "AND" => a & b,
                    "OR" => a | b,
                    _ => a ^ b,
                };
                Int(t.wrap(r), *t)
            }
            _ => {
                let (a, b) = (x.truthy(), y.truthy());
                Bool(match op {
                    "AND" => a && b,
                    "OR" => a || b,
                    _ => a ^ b,
                })
            }
        },
        "=" | "<>" | "<" | ">" | "<=" | ">=" => {
            let ord = match (x, y) {
                (Int(a, _), Int(b, _)) => a.cmp(b),
                (Time(a), Time(b)) => a.cmp(b),
                (Bool(a), Bool(b)) => a.cmp(b),
                (Value::Str(a), Value::Str(b)) => a.cmp(b),
                _ => x
                    .as_f64()
                    .partial_cmp(&y.as_f64())
                    .unwrap_or(std::cmp::Ordering::Equal),
            };
            use std::cmp::Ordering::{Equal, Greater, Less};
            Bool(match op {
                "=" => ord == Equal,
                "<>" => ord != Equal,
                "<" => ord == Less,
                ">" => ord == Greater,
                "<=" => ord != Greater,
                _ => ord != Less,
            })
        }
        "+" | "-" | "*" | "/" | "MOD" | "**" => match (x, y) {
            (Int(a, t), Int(b, tb)) => {
                let t = if tb.bits > t.bits { *tb } else { *t };
                let r = match op {
                    "+" => a.wrapping_add(*b),
                    "-" => a.wrapping_sub(*b),
                    "*" => a.wrapping_mul(*b),
                    "/" => {
                        if *b == 0 {
                            return Err(VmError("division by zero".into()));
                        }
                        a / b
                    }
                    "MOD" => {
                        if *b == 0 {
                            return Err(VmError("division by zero".into()));
                        }
                        a % b
                    }
                    _ => return Ok(Real((*a as f64).powf(*b as f64))),
                };
                Int(t.wrap(r), t)
            }
            (Time(a), Time(b)) => match op {
                "+" => Time(a + b),
                "-" => Time(a - b),
                _ => return Err(VmError(format!("invalid TIME operation {op}"))),
            },
            (Time(a), Int(b, _)) => match op {
                "*" => Time(a * b),
                "/" if *b != 0 => Time(a / b),
                _ => return Err(VmError(format!("invalid TIME operation {op}"))),
            },
            _ => {
                let (a, b) = (x.as_f64(), y.as_f64());
                Real(match op {
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" => a / b,
                    "MOD" => a % b,
                    _ => a.powf(b),
                })
            }
        },
        _ => return Err(VmError(format!("unknown operator {op}"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_wrap_like_a_plc() {
        let t = IntTy {
            bits: 16,
            signed: false,
        };
        assert_eq!(t.wrap(65_536), 0);
        let s = IntTy {
            bits: 8,
            signed: true,
        };
        assert_eq!(s.wrap(128), -128);
    }
}
