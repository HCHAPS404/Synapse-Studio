//! NIR to MIR (task B2).
//!
//! The lowering covers the subset that the C backend supports (spec: `.kiro/specs/neuron-codegen-c`).
//! Anything outside it is reported as `NE0610`, naming the construct, never dropped.

use crate::mir::*;
use neuron_nir as n;
use neuron_syntax::LineIndex;
use neuron_target::{Diag, HwReport, IoBinding, IoKind as HwKind};
use std::collections::{BTreeMap, BTreeSet};

/// A source file, so that statements can carry their line.
#[derive(Debug, Clone)]
pub struct Source {
    /// Path as it appears in the NIR (`Module::file`).
    pub file: String,
    /// Text.
    pub text: String,
}

/// Result of lowering.
#[derive(Debug, Clone, Default)]
pub struct Lowered {
    /// The program (complete only when `diags` is empty).
    pub program: Program,
    /// `NE0610` for every construct outside the supported subset.
    pub diags: Vec<Diag>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cv {
    I(i64),
    F(f64),
    B(bool),
}

#[derive(Debug, Clone)]
enum Ent {
    Const(Cv),
    Var(String, Ty),
    Signal(String, Ty),
    Func(String, Vec<Ty>, Option<Ty>),
    Io(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Task,
    Isr,
    Other,
}

const BUILTIN_FAULTS: [&str; 7] = [
    "Overflow",
    "DivideByZero",
    "ContractViolation",
    "StackOverflow",
    "LoopBound",
    "DeadlineMiss",
    "Watchdog",
];

fn ty_of(t: &n::Type) -> Option<Ty> {
    match t {
        n::Type::Bool => Some(Ty::Bool),
        n::Type::Instant => Some(Ty::F32),
        n::Type::Num { kind, unit } => {
            if unit.is_some() {
                return Some(Ty::F32);
            }
            match kind.as_str() {
                "u8" => Some(Ty::U8),
                "i8" => Some(Ty::I8),
                "u16" | "index" => Some(Ty::U16),
                "i16" => Some(Ty::I16),
                "u32" => Some(Ty::U32),
                "i32" => Some(Ty::I32),
                "f32" | "f64" | "duration" => Some(Ty::F32),
                _ => None,
            }
        }
        _ => None,
    }
}

fn ty_of_texpr(t: &n::TypeExpr) -> Option<Ty> {
    if t.optional {
        return None;
    }
    if t.unit.is_some() {
        return Some(Ty::F32);
    }
    match &t.kind {
        n::TypeExprKind::Named { path, args } if args.is_empty() => match path.as_str() {
            "bool" => Some(Ty::Bool),
            "u8" => Some(Ty::U8),
            "i8" => Some(Ty::I8),
            "u16" | "index" => Some(Ty::U16),
            "i16" => Some(Ty::I16),
            "u32" => Some(Ty::U32),
            "i32" => Some(Ty::I32),
            "f32" | "f64" | "duration" => Some(Ty::F32),
            _ => None,
        },
        n::TypeExprKind::Unit => Some(Ty::F32),
        _ => None,
    }
}

fn concrete(e: &n::Expr) -> Option<Ty> {
    ty_of(&e.ty)
}

fn c_ident(prefix: &str, name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{prefix}_{clean}")
}

fn bin_op(op: &str) -> Option<BinOp> {
    Some(match op {
        "+" => BinOp::Add,
        "-" => BinOp::Sub,
        "*" => BinOp::Mul,
        "/" => BinOp::Div,
        "%" => BinOp::Rem,
        "+%" => BinOp::AddW,
        "-%" => BinOp::SubW,
        "*%" => BinOp::MulW,
        "&" => BinOp::BitAnd,
        "|" => BinOp::BitOr,
        "^" => BinOp::BitXor,
        "<<" => BinOp::Shl,
        ">>" => BinOp::Shr,
        "and" => BinOp::And,
        "or" => BinOp::Or,
        "==" => BinOp::Eq,
        "!=" => BinOp::Ne,
        "<" => BinOp::Lt,
        "<=" => BinOp::Le,
        ">" => BinOp::Gt,
        ">=" => BinOp::Ge,
        _ => return None,
    })
}

struct Lower<'a> {
    nir: &'a n::Nir,
    hw: &'a HwReport,
    lines: Vec<LineIndex>,
    sources: &'a [Source],
    ents: BTreeMap<String, Ent>,
    names: BTreeMap<String, String>,
    used: BTreeSet<String>,
    prog: Program,
    diags: Vec<Diag>,
    file_idx: u32,
    file_name: String,
    act: Act,
    reads: BTreeSet<String>,
    writes: BTreeSet<String>,
    touches: BTreeSet<String>,
    calls: BTreeSet<String>,
    fn_ret: Option<Ty>,
    isr_touch: BTreeSet<String>,
    isr_calls: BTreeSet<String>,
    other_touch: BTreeSet<String>,
    other_calls: BTreeSet<String>,
}

/// Lowers the `mcu` and `core` modules of a package.
pub fn lower(nir: &n::Nir, hw: &HwReport, sources: &[Source]) -> Lowered {
    let mut l = Lower {
        nir,
        hw,
        lines: Vec::new(),
        sources,
        ents: BTreeMap::new(),
        names: BTreeMap::new(),
        used: BTreeSet::new(),
        prog: Program::default(),
        diags: Vec::new(),
        file_idx: 0,
        file_name: String::new(),
        act: Act::Other,
        reads: BTreeSet::new(),
        writes: BTreeSet::new(),
        touches: BTreeSet::new(),
        calls: BTreeSet::new(),
        fn_ret: None,
        isr_touch: BTreeSet::new(),
        isr_calls: BTreeSet::new(),
        other_touch: BTreeSet::new(),
        other_calls: BTreeSet::new(),
    };
    l.run();
    Lowered {
        program: l.prog,
        diags: l.diags,
    }
}

impl<'a> Lower<'a> {
    // ---- diagnostics and positions ----------------------------------------------------

    fn unsupported(&mut self, what: &str, span: n::Span) {
        self.diags.push(Diag {
            code: "NE0610",
            message: format!("{what} is not supported by the C backend yet"),
            help: Some("the supported subset is listed in docs/targets.md".to_string()),
            file: self.file_name.clone(),
            span,
        });
    }

    fn loc(&self, span: n::Span) -> Loc {
        let line = match (self.lines.get(self.file_idx as usize), self.source_text()) {
            (Some(idx), Some(text)) => idx.line_col(text, span[0]).0 as u32,
            _ => 0,
        };
        Loc {
            file: self.file_idx,
            line,
        }
    }

    fn source_text(&self) -> Option<&'a str> {
        self.sources
            .iter()
            .find(|s| s.file == self.file_name)
            .map(|s| s.text.as_str())
    }

    fn unique(&mut self, base: String) -> String {
        let mut name = base.clone();
        let mut k = 2;
        while !self.used.insert(name.clone()) {
            name = format!("{base}_{k}");
            k += 1;
        }
        name
    }

    // ---- constants ---------------------------------------------------------------------

    fn cv(&self, e: &n::Expr) -> Option<Cv> {
        match &e.e {
            n::ExprKind::Lit { lit } => match lit {
                n::Lit::Int { v, .. } => v.parse::<i64>().ok().map(Cv::I),
                n::Lit::Real { v, .. } => v.parse::<f64>().ok().map(Cv::F),
                n::Lit::Bool { v } => Some(Cv::B(*v)),
                _ => None,
            },
            n::ExprKind::Quantity { value, si, .. } => match si {
                Some(s) => s.parse::<f64>().ok().map(Cv::F),
                None => self.cv(value),
            },
            n::ExprKind::Ref { id, .. } => match self.ents.get(id) {
                Some(Ent::Const(c)) => Some(*c),
                _ => None,
            },
            n::ExprKind::Unary { op, operand } => {
                let v = self.cv(operand)?;
                match (op.as_str(), v) {
                    ("-", Cv::I(i)) => i.checked_neg().map(Cv::I),
                    ("-", Cv::F(f)) => Some(Cv::F(-f)),
                    ("not", Cv::B(b)) => Some(Cv::B(!b)),
                    ("~", Cv::I(i)) => Some(Cv::I(!i)),
                    _ => None,
                }
            }
            n::ExprKind::Binary { op, lhs, rhs } => {
                let (a, b) = (self.cv(lhs)?, self.cv(rhs)?);
                match (a, b) {
                    (Cv::I(x), Cv::I(y)) => match op.as_str() {
                        "+" => x.checked_add(y).map(Cv::I),
                        "-" => x.checked_sub(y).map(Cv::I),
                        "*" => x.checked_mul(y).map(Cv::I),
                        "/" if y != 0 => Some(Cv::I(x / y)),
                        "%" if y != 0 => Some(Cv::I(x % y)),
                        _ => None,
                    },
                    (Cv::B(x), Cv::B(y)) => match op.as_str() {
                        "and" => Some(Cv::B(x && y)),
                        "or" => Some(Cv::B(x || y)),
                        _ => None,
                    },
                    (x, y) => {
                        let (x, y) = (cv_f(x)?, cv_f(y)?);
                        match op.as_str() {
                            "+" => Some(Cv::F(x + y)),
                            "-" => Some(Cv::F(x - y)),
                            "*" => Some(Cv::F(x * y)),
                            "/" if y != 0.0 => Some(Cv::F(x / y)),
                            _ => None,
                        }
                    }
                }
            }
            n::ExprKind::Cast { operand, .. } => {
                let v = self.cv(operand)?;
                match (concrete(e), v) {
                    (Some(t), Cv::F(f)) if t.is_int() => Some(Cv::I(f as i64)),
                    (Some(Ty::F32), Cv::I(i)) => Some(Cv::F(i as f64)),
                    _ => Some(v),
                }
            }
            _ => None,
        }
    }

    fn cv_expr(&self, v: Cv, ty: Ty) -> Expr {
        match (v, ty) {
            (Cv::B(b), _) => Expr::Bool(b),
            (Cv::I(i), Ty::F32) => Expr::Float(i as f64),
            (Cv::I(i), Ty::Bool) => Expr::Bool(i != 0),
            (Cv::I(i), t) => Expr::Int(i, t),
            (Cv::F(f), t) if t.is_int() => Expr::Int(f as i64, t),
            (Cv::F(f), _) => Expr::Float(f),
        }
    }

    /// Milliseconds of a constant duration, or `None` unless it is a whole number of them.
    fn ms_of(&self, e: &n::Expr) -> Option<u32> {
        let secs = match self.cv(e)? {
            Cv::F(f) => f,
            Cv::I(i) => i as f64,
            Cv::B(_) => return None,
        };
        let ms = (secs * 1000.0).round();
        if (secs * 1000.0 - ms).abs() > 1e-6 || ms < 0.0 || ms > f64::from(u32::MAX) {
            return None;
        }
        Some(ms as u32)
    }

    fn int_of(&self, e: &n::Expr) -> Option<i64> {
        match self.cv(e)? {
            Cv::I(i) => Some(i),
            Cv::F(f) if f.fract() == 0.0 => Some(f as i64),
            _ => None,
        }
    }

    // ---- driver --------------------------------------------------------------------------

    fn run(&mut self) {
        let nir = self.nir;
        self.prog.name = nir
            .package
            .clone()
            .or_else(|| nir.modules.first().map(|m| m.name.clone()))
            .unwrap_or_else(|| "app".to_string());
        if let Some(t) = &self.hw.target {
            self.prog.board = Some(t.board.name.clone());
        }
        self.prog.sysclk_hz = self.hw.sysclk_hz;
        let wanted: Vec<usize> = nir
            .modules
            .iter()
            .enumerate()
            .filter(|(_, m)| matches!(m.dialect.as_str(), "mcu" | "core"))
            .map(|(i, _)| i)
            .collect();
        for &mi in &wanted {
            let text = self
                .sources
                .iter()
                .find(|s| s.file == nir.modules[mi].file)
                .map_or("", |s| s.text.as_str());
            self.lines.push(LineIndex::new(text));
            self.prog.files.push(nir.modules[mi].file.clone());
        }
        self.build_ios();
        for (k, &mi) in wanted.iter().enumerate() {
            self.file_idx = k as u32;
            self.file_name = nir.modules[mi].file.clone();
            self.collect(mi);
        }
        for (k, &mi) in wanted.iter().enumerate() {
            self.file_idx = k as u32;
            self.file_name = nir.modules[mi].file.clone();
            self.activities(mi);
        }
        self.finish();
    }

    fn build_ios(&mut self) {
        let hw = self.hw;
        let isa = hw
            .target
            .as_ref()
            .map_or(String::new(), |t| t.arch.isa.clone());
        for b in &hw.bindings {
            let def = self.io_def(b, &isa);
            self.prog.ios.push(def);
        }
    }

    fn io_def(&mut self, b: &IoBinding, isa: &str) -> IoDef {
        let mut def = IoDef {
            name: b.name.clone(),
            kind: IoK::GpioOut,
            port: b.port,
            bit: b.bit,
            aux: 255,
            cfg: 0,
        };
        let module_file = self
            .nir
            .modules
            .get(b.module)
            .map(|m| m.file.clone())
            .unwrap_or_default();
        let mut bad = |what: String| {
            self.diags.push(Diag {
                code: "NE0610",
                message: format!("{what} is not supported by the C backend yet"),
                help: Some("the supported subset is listed in docs/targets.md".to_string()),
                file: module_file.clone(),
                span: b.span,
            });
        };
        match &b.kind {
            HwKind::GpioInput => {
                def.kind = IoK::GpioIn;
                def.aux = b.int_num.map_or(255, |n| n as u8);
                def.cfg = u16::from(b.pull_up);
            }
            HwKind::GpioOutput | HwKind::GpioOpenDrain => def.kind = IoK::GpioOut,
            HwKind::Pwm => {
                def.kind = IoK::Pwm;
                if let Some(p) = &b.pwm {
                    let timer: u8 = p.timer.trim_start_matches("timer").parse().unwrap_or(0);
                    let chan = match p.channel {
                        'a' => 0,
                        'b' => 1,
                        _ => 2,
                    };
                    if isa == "avr" && !(timer == 1 || timer == 2) || chan > 1 {
                        bad(format!("PWM on `{}` channel `{}` (the AVR port drives timers 1 and 2, channels a and b)", p.timer, p.channel));
                    }
                    def.aux = timer * 4 + chan;
                    def.cfg = p.prescaler as u16;
                }
            }
            HwKind::Adc => {
                def.kind = IoK::Adc;
                def.aux = b.adc_channel.map_or(0, |c| c as u8);
            }
            HwKind::Uart => {
                def.kind = IoK::Uart;
                if let Some(u) = &b.uart {
                    if u.index != 0 {
                        bad(format!("UART {} (the AVR port drives UART 0)", u.index));
                    }
                    def.aux = u.index as u8;
                    def.cfg = (u.ubrr as u16 & 0x0FFF) | if u.double_speed { 0x8000 } else { 0 };
                }
            }
            HwKind::Timer | HwKind::Other(_) => bad(format!("the `io` kind of `{}`", b.name)),
        }
        def
    }

    /// First pass: names, types and constants of every item.
    fn collect(&mut self, mi: usize) {
        let nir = self.nir;
        let module = &nir.modules[mi];
        for item in &module.items {
            match &item.def {
                n::ItemDef::Const { name, value, .. } => {
                    self.names.insert(name.clone(), item.id.clone());
                    match self.cv(value) {
                        Some(c) => {
                            self.ents.insert(item.id.clone(), Ent::Const(c));
                        }
                        None => self
                            .unsupported("a `const` that is not a compile-time value", item.span),
                    }
                }
                n::ItemDef::Var { var } => {
                    self.names.insert(var.name.clone(), item.id.clone());
                    let Some(ty) = ty_of(&var.vty) else {
                        self.unsupported(
                            &format!(
                                "the type of `{}` (function blocks, arrays and structs)",
                                var.name
                            ),
                            item.span,
                        );
                        continue;
                    };
                    let init = var.init.as_ref().and_then(|e| self.cv(e));
                    if !var.mutable {
                        match init {
                            Some(c) => {
                                self.ents.insert(item.id.clone(), Ent::Const(c));
                            }
                            None => self.unsupported(
                                &format!("`let {}` with a value that is not constant", var.name),
                                item.span,
                            ),
                        }
                        continue;
                    }
                    if var.init.is_some() && init.is_none() {
                        self.unsupported(
                            &format!(
                                "the initial value of `{}` (it must be a constant)",
                                var.name
                            ),
                            item.span,
                        );
                    }
                    let cname = self.unique(c_ident("g", &var.name));
                    self.prog.globals.push(Global {
                        name: cname.clone(),
                        ty,
                        init: init.map(|c| self.cv_expr(c, ty)),
                        signal: false,
                        shared: false,
                        source_name: var.name.clone(),
                    });
                    self.ents.insert(item.id.clone(), Ent::Var(cname, ty));
                }
                n::ItemDef::Signal {
                    name, vty, init, ..
                } => {
                    self.names.insert(name.clone(), item.id.clone());
                    let Some(ty) = vty.as_ref().and_then(ty_of) else {
                        self.unsupported(&format!("the type of signal `{name}`"), item.span);
                        continue;
                    };
                    let cname = self.unique(c_ident("s", name));
                    let init = init.as_ref().and_then(|e| self.cv(e));
                    self.prog.globals.push(Global {
                        name: cname.clone(),
                        ty,
                        init: init.map(|c| self.cv_expr(c, ty)),
                        signal: true,
                        shared: false,
                        source_name: name.clone(),
                    });
                    self.ents.insert(item.id.clone(), Ent::Signal(cname, ty));
                }
                n::ItemDef::Fn { func } => {
                    let mut tys = Vec::new();
                    for p in &func.sig.params {
                        match p.ty.as_ref().and_then(ty_of_texpr) {
                            Some(t) if p.name != "self" => tys.push(t),
                            _ => self.unsupported(
                                &format!("the parameter `{}` of `{}`", p.name, func.sig.name),
                                item.span,
                            ),
                        }
                    }
                    let ret = func.sig.ret.as_ref().and_then(ty_of_texpr);
                    if func.sig.ret.is_some() && ret.is_none() {
                        self.unsupported(
                            &format!("the return type of `{}`", func.sig.name),
                            item.span,
                        );
                    }
                    let cname = self.unique(c_ident("f", &func.sig.name));
                    self.names.insert(func.sig.name.clone(), item.id.clone());
                    self.ents
                        .insert(item.id.clone(), Ent::Func(cname, tys, ret));
                }
                n::ItemDef::Io { name, .. } => {
                    self.names.insert(name.clone(), item.id.clone());
                    match self
                        .hw
                        .bindings
                        .iter()
                        .position(|b| b.module == mi && &b.name == name)
                    {
                        Some(bi) => {
                            self.ents.insert(item.id.clone(), Ent::Io(bi));
                        }
                        None => self.unsupported(
                            &format!(
                                "`io {name}` without a board (add `on <board>` to the module)"
                            ),
                            item.span,
                        ),
                    }
                }
                n::ItemDef::Event { name, fields, .. } => {
                    if !fields.is_empty() {
                        self.unsupported("an event with a payload", item.span);
                    }
                    self.event_index(name);
                }
                n::ItemDef::Use { .. }
                | n::ItemDef::Unit { .. }
                | n::ItemDef::Clocks { .. }
                | n::ItemDef::Uses { .. }
                | n::ItemDef::Verify { .. }
                | n::ItemDef::Test { .. }
                | n::ItemDef::Task { .. }
                | n::ItemDef::Interrupt { .. }
                | n::ItemDef::Fsm { .. }
                | n::ItemDef::Safety { .. }
                | n::ItemDef::Rule { .. } => {}
                other => {
                    let what = item_kind(other);
                    self.unsupported(&format!("`{what}`"), item.span);
                }
            }
        }
    }

    fn event_index(&mut self, name: &str) -> usize {
        if let Some(i) = self.prog.events.iter().position(|e| e == name) {
            return i;
        }
        self.prog.events.push(name.to_string());
        self.prog.events.len() - 1
    }

    fn fault_name(&mut self, name: &str) -> String {
        if !BUILTIN_FAULTS.contains(&name) && !self.prog.faults.iter().any(|f| f == name) {
            self.prog.faults.push(name.to_string());
        }
        name.to_string()
    }

    // ---- activities ---------------------------------------------------------------------

    fn begin(&mut self, act: Act) {
        self.act = act;
        self.reads.clear();
        self.writes.clear();
        self.touches.clear();
        self.calls.clear();
    }

    fn end(&mut self) {
        let t = std::mem::take(&mut self.touches);
        let c = std::mem::take(&mut self.calls);
        if self.act == Act::Isr {
            self.isr_touch.extend(t);
            self.isr_calls.extend(c);
        } else {
            self.other_touch.extend(t);
            self.other_calls.extend(c);
        }
    }

    fn activities(&mut self, mi: usize) {
        let nir = self.nir;
        let module = &nir.modules[mi];
        for item in &module.items {
            match &item.def {
                n::ItemDef::Fn { func } => self.function(item, func),
                n::ItemDef::Task {
                    name,
                    trigger,
                    attrs,
                    body,
                } => {
                    self.task(item, name, trigger, attrs, body);
                }
                n::ItemDef::Interrupt {
                    source,
                    attrs,
                    body,
                } => self.interrupt(item, source, attrs, body),
                n::ItemDef::Fsm { fsm } => self.fsm(item, fsm),
                n::ItemDef::Rule { rule } => {
                    self.begin(Act::Other);
                    let r = self.rule(rule);
                    self.end();
                    if let Some(r) = r {
                        self.prog.rules.push(r);
                    }
                }
                n::ItemDef::Safety { members } => self.safety(item, members),
                _ => {}
            }
        }
    }

    fn function(&mut self, item: &n::Item, func: &n::FnDecl) {
        let Some(Ent::Func(cname, tys, ret)) = self.ents.get(&item.id).cloned() else {
            return;
        };
        if tys.len() != func.sig.params.len() {
            return;
        }
        self.begin(Act::Other);
        self.fn_ret = ret;
        let params: Vec<(String, Ty)> = func
            .sig
            .params
            .iter()
            .zip(&tys)
            .map(|(p, t)| (c_ident("l", &p.name), *t))
            .collect();
        let body = match &func.body {
            n::FnBody::Block { block } => self.block(block),
            n::FnBody::Expr { expr } => match self.expr(expr, ret) {
                Some(e) => vec![Stmt {
                    loc: self.loc(expr.span),
                    kind: StmtKind::Return(Some(e)),
                }],
                None => Vec::new(),
            },
        };
        let touches = std::mem::take(&mut self.touches);
        let calls = std::mem::take(&mut self.calls);
        self.fn_ret = None;
        self.prog.funcs.push(Func {
            name: cname,
            params,
            ret,
            body,
            atomic: false,
            touches,
            calls,
        });
    }

    fn task(
        &mut self,
        item: &n::Item,
        name: &str,
        trigger: &n::Trigger,
        attrs: &[n::TaskAttr],
        body: &n::Block,
    ) {
        let (mut period_ms, mut offset_ms, mut event) = (0u32, 0u32, None);
        match trigger {
            n::Trigger::Every { period, offset } => {
                match self.ms_of(period) {
                    Some(ms) if ms >= 1 => period_ms = ms,
                    _ => self.unsupported(&format!("the period of task `{name}` (it must be a whole number of milliseconds)"), item.span),
                }
                if let Some(o) = offset {
                    match self.ms_of(o) {
                        Some(ms) => offset_ms = ms,
                        None => {
                            self.unsupported(&format!("the offset of task `{name}`"), item.span)
                        }
                    }
                }
            }
            n::Trigger::On { event: ev } => match crate_flat(ev) {
                Some(e) => event = Some(self.event_index(&e)),
                None => self.unsupported(&format!("the trigger of task `{name}`"), item.span),
            },
        }
        let (mut prio, mut deadline) = (1u32, None);
        for a in attrs {
            match a {
                n::TaskAttr::Prio { value } => match self.int_of(value) {
                    Some(p) if p >= 0 => prio = p as u32,
                    _ => self.unsupported("a priority that is not a constant", item.span),
                },
                n::TaskAttr::Deadline { value } => match self.ms_of(value) {
                    Some(ms) => deadline = Some(ms),
                    None => self.unsupported(
                        "a deadline that is not a whole number of milliseconds",
                        item.span,
                    ),
                },
                _ => {}
            }
        }
        self.begin(Act::Task);
        let stmts = self.block(body);
        let reads: Vec<String> = self.reads.iter().cloned().collect();
        let writes: Vec<String> = self.writes.iter().cloned().collect();
        self.end();
        let cname = self.unique(c_ident("task", name));
        let loc = self.loc(item.span);
        self.prog.tasks.push(Task {
            name: cname,
            period_ms,
            offset_ms,
            prio,
            event,
            deadline_ms: deadline.unwrap_or(if event.is_some() { 0 } else { period_ms }),
            reads,
            writes,
            body: stmts,
            loc,
        });
    }

    fn interrupt(
        &mut self,
        item: &n::Item,
        source: &n::Expr,
        attrs: &[n::TaskAttr],
        body: &n::Block,
    ) {
        let n::ExprKind::Field {
            base, name: edge, ..
        } = &source.e
        else {
            self.unsupported("an interrupt source that is not `<io>.<edge>`", item.span);
            return;
        };
        let edge = match edge.as_str() {
            "falling" => Edge::Falling,
            "rising" => Edge::Rising,
            "change" => Edge::Change,
            other => {
                self.unsupported(&format!("the interrupt event `{other}`"), item.span);
                return;
            }
        };
        let io = match &base.e {
            n::ExprKind::Ref { id, .. } => match self.ents.get(id) {
                Some(Ent::Io(i)) => *i,
                _ => {
                    self.unsupported("an interrupt source that is not an input", item.span);
                    return;
                }
            },
            _ => {
                self.unsupported("an interrupt source that is not an input", item.span);
                return;
            }
        };
        let mut prio = 1;
        for a in attrs {
            if let n::TaskAttr::Prio { value } = a {
                prio = self.int_of(value).unwrap_or(1).max(0) as u32;
            }
        }
        self.begin(Act::Isr);
        let stmts = self.block(body);
        self.end();
        let name = self.unique(format!(
            "isr_{}_{}",
            self.prog.ios[io].name,
            edge_name(edge)
        ));
        self.prog.irqs.push(Irq {
            name,
            io,
            edge,
            prio,
            body: stmts,
        });
    }

    fn fsm(&mut self, item: &n::Item, fsm: &n::Fsm) {
        if !fsm.vars.is_empty() {
            self.unsupported(&format!("variables inside `fsm {}`", fsm.name), item.span);
        }
        let period_ms = match &fsm.trigger {
            None => 10,
            Some(n::Trigger::Every { period, .. }) => {
                self.ms_of(period).filter(|m| *m >= 1).unwrap_or_else(|| {
                    self.unsupported(
                        "a state-machine period that is not a whole number of milliseconds",
                        item.span,
                    );
                    10
                })
            }
            Some(n::Trigger::On { .. }) => {
                self.unsupported("a state machine released by an event", item.span);
                10
            }
        };
        self.begin(Act::Other);
        let mut states = Vec::new();
        let index: BTreeMap<&str, usize> = fsm
            .states
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.as_str(), i))
            .collect();
        for st in &fsm.states {
            let entry = st.entry.as_ref().map(|b| self.block(b)).unwrap_or_default();
            let exit = st.exit.as_ref().map(|b| self.block(b)).unwrap_or_default();
            let during = st
                .during
                .as_ref()
                .map(|b| self.block(b))
                .unwrap_or_default();
            let mut trans = Vec::new();
            for t in &st.transitions {
                let when = t.when.as_ref().and_then(|w| self.expr(w, Some(Ty::Bool)));
                let after_ms = match &t.after {
                    Some(a) => self.ms_of(a).or_else(|| {
                        self.unsupported(
                            "an `after` that is not a whole number of milliseconds",
                            t.span,
                        );
                        None
                    }),
                    None => None,
                };
                let action = t.action.as_ref().map(|b| self.block(b)).unwrap_or_default();
                match index.get(t.target.as_str()) {
                    Some(&target) => trans.push(Trans {
                        target,
                        when,
                        after_ms,
                        action,
                    }),
                    None => self.unsupported(
                        &format!("a transition to the unknown state `{}`", t.target),
                        t.span,
                    ),
                }
            }
            states.push(State {
                name: st.name.clone(),
                entry,
                exit,
                during,
                trans,
            });
        }
        self.end();
        let initial = fsm.states.iter().position(|s| s.initial).unwrap_or(0);
        self.prog.fsms.push(Fsm {
            name: fsm.name.clone(),
            period_ms,
            initial,
            states,
        });
    }

    fn rule(&mut self, rule: &n::Rule) -> Option<Rule> {
        let cond = self.expr(&rule.cond, Some(Ty::Bool))?;
        let hold_ms = match &rule.hold {
            Some(h) => match self.ms_of(h) {
                Some(m) => Some(m),
                None => {
                    self.unsupported(
                        "a rule hold time that is not a whole number of milliseconds",
                        rule.span,
                    );
                    None
                }
            },
            None => None,
        };
        let body = self.block(&rule.body);
        Some(Rule {
            cond,
            hold_ms,
            body,
        })
    }

    fn safety(&mut self, item: &n::Item, members: &[n::SafetyMember]) {
        let mut rate = None;
        self.begin(Act::Other);
        for m in members {
            match &m.def {
                n::SafetyDef::Rate { value } => rate = self.ms_of(value).filter(|r| *r >= 1),
                n::SafetyDef::Watchdog { value } => match self.ms_of(value) {
                    Some(ms) => self.prog.safety.watchdog_ms = Some(ms),
                    None => self.unsupported(
                        "a watchdog time that is not a whole number of milliseconds",
                        m.span,
                    ),
                },
                n::SafetyDef::SafeState { body } => {
                    let mut s = self.block(body);
                    self.prog.safety.safe_state.append(&mut s);
                }
                n::SafetyDef::Rule { rule } => {
                    if let Some(r) = self.rule(rule) {
                        self.prog.safety.rules.push(r);
                    }
                }
                n::SafetyDef::OnFault { fault, body } => {
                    let f = fault.as_ref().map(|f| self.fault_name(f));
                    let b = self.block(body);
                    self.prog.safety.on_fault.push((f, b));
                }
                n::SafetyDef::Reset { cond } => {
                    self.prog.safety.reset = self.expr(cond, Some(Ty::Bool));
                }
            }
        }
        self.end();
        if let Some(r) = rate {
            self.prog.safety.rate_ms = r;
        }
        let _ = item;
    }

    /// Closes the program: edge bits, shared data and the safety rate.
    fn finish(&mut self) {
        // Edge configuration of the inputs that raise interrupts.
        let mut edges: BTreeMap<usize, (bool, bool)> = BTreeMap::new();
        for irq in &self.prog.irqs {
            let e = edges.entry(irq.io).or_insert((false, false));
            match irq.edge {
                Edge::Falling => e.0 = true,
                Edge::Rising => e.1 = true,
                Edge::Change => {
                    e.0 = true;
                    e.1 = true;
                }
            }
        }
        for (io, (f, r)) in edges {
            let bits: u16 = match (f, r) {
                (true, true) => 3,
                (true, false) => 1,
                (false, true) => 2,
                _ => 0,
            };
            self.prog.ios[io].cfg |= bits << 1;
        }
        // Data touched both by an interrupt and by other code is shared.
        let mut eff: BTreeMap<String, BTreeSet<String>> = self
            .prog
            .funcs
            .iter()
            .map(|f| (f.name.clone(), f.touches.clone()))
            .collect();
        loop {
            let mut changed = false;
            for f in &self.prog.funcs {
                for c in &f.calls {
                    if let Some(ct) = eff.get(c).cloned() {
                        if let Some(e) = eff.get_mut(&f.name) {
                            for x in ct {
                                changed |= e.insert(x);
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let expand = |touch: &BTreeSet<String>, calls: &BTreeSet<String>| -> BTreeSet<String> {
            let mut s = touch.clone();
            for c in calls {
                if let Some(t) = eff.get(c) {
                    s.extend(t.iter().cloned());
                }
            }
            s
        };
        let isr = expand(&self.isr_touch, &self.isr_calls);
        let other = expand(&self.other_touch, &self.other_calls);
        let shared: BTreeSet<String> = isr.intersection(&other).cloned().collect();
        for g in &mut self.prog.globals {
            g.shared = shared.contains(&g.name);
        }
        for f in &mut self.prog.funcs {
            f.atomic = eff
                .get(&f.name)
                .is_some_and(|t| t.iter().any(|x| shared.contains(x)));
        }
        // The safety logic runs at its `rate`, 10 ms by default (the interpreter's default rate).
        if self.prog.safety.rate_ms == 0 {
            self.prog.safety.rate_ms = 10;
        }
    }

    // ---- statements -----------------------------------------------------------------------

    fn block(&mut self, b: &n::Block) -> Vec<Stmt> {
        let mut out = Vec::new();
        for s in &b.stmts {
            self.stmt(s, &mut out);
        }
        out
    }

    fn push(&self, out: &mut Vec<Stmt>, span: n::Span, kind: StmtKind) {
        out.push(Stmt {
            loc: self.loc(span),
            kind,
        });
    }

    fn stmt(&mut self, s: &n::Stmt, out: &mut Vec<Stmt>) {
        match &s.s {
            n::StmtKind::Pass => {}
            n::StmtKind::Let {
                pattern, ty, value, ..
            } => {
                let n::PatternKind::Bind { name, ty: pty, .. } = &pattern.p else {
                    self.unsupported("a `let` with a pattern", s.span);
                    return;
                };
                let t = ty_of(pty)
                    .or_else(|| ty.as_ref().and_then(ty_of_texpr))
                    .or_else(|| concrete(value));
                let Some(t) = t else {
                    self.unsupported(&format!("the type of `{name}`"), s.span);
                    return;
                };
                if let Some(v) = self.expr(value, Some(t)) {
                    let v = coerce(v, t);
                    self.push(
                        out,
                        s.span,
                        StmtKind::Let {
                            name: c_ident("l", name),
                            ty: t,
                            value: v,
                        },
                    );
                }
            }
            n::StmtKind::Assign {
                place,
                op,
                value,
                when,
            } => {
                if when.is_some() {
                    self.unsupported("a conditional assignment (`when`)", s.span);
                    return;
                }
                self.assign(s.span, place, op, value, out);
            }
            n::StmtKind::Expr { expr } => self.expr_stmt(s.span, expr, out),
            n::StmtKind::Return { value } => {
                let ret = self.fn_ret;
                let v = match value {
                    Some(v) => match self.expr(v, ret) {
                        Some(e) => Some(match ret {
                            Some(t) => coerce(e, t),
                            None => e,
                        }),
                        None => return,
                    },
                    None => None,
                };
                self.push(out, s.span, StmtKind::Return(v));
            }
            n::StmtKind::Break => self.push(out, s.span, StmtKind::Break),
            n::StmtKind::Continue => self.push(out, s.span, StmtKind::Continue),
            n::StmtKind::Fault { name, args } => {
                if !args.is_empty() {
                    self.unsupported("a fault with arguments", s.span);
                }
                let f = self.fault_name(name);
                self.push(out, s.span, StmtKind::Fault(f));
            }
            n::StmtKind::Assert { cond, .. } => {
                if let Some(c) = self.expr(cond, Some(Ty::Bool)) {
                    let fault = vec![Stmt {
                        loc: self.loc(s.span),
                        kind: StmtKind::Fault("ContractViolation".to_string()),
                    }];
                    self.push(
                        out,
                        s.span,
                        StmtKind::If {
                            cond: Expr::Unary(UnOp::Not, Box::new(c), Ty::Bool),
                            then: fault,
                            otherwise: Vec::new(),
                        },
                    );
                }
            }
            n::StmtKind::Emit { event, args } => {
                if !args.is_empty() {
                    self.unsupported("an event with a payload", s.span);
                }
                let i = self.event_index(event);
                self.push(out, s.span, StmtKind::Emit(i));
            }
            n::StmtKind::If {
                branches,
                otherwise,
            } => {
                let stmt = self.if_chain(s.span, branches, otherwise.as_ref());
                if let Some(st) = stmt {
                    out.push(st);
                }
            }
            n::StmtKind::For {
                pattern,
                iter,
                body,
                ..
            } => {
                let n::PatternKind::Bind { name, ty, .. } = &pattern.p else {
                    self.unsupported("a `for` with a pattern", s.span);
                    return;
                };
                let n::ExprKind::Range { lo, hi, inclusive } = &iter.e else {
                    self.unsupported("a `for` over something other than a constant range", s.span);
                    return;
                };
                let (Some(l), Some(h)) = (self.int_of(lo), self.int_of(hi)) else {
                    self.unsupported("a `for` range with bounds that are not constants", s.span);
                    return;
                };
                let hi_excl = if *inclusive { h + 1 } else { h };
                let t = ty_of(ty).unwrap_or(Ty::U16);
                let b = self.block(body);
                self.push(
                    out,
                    s.span,
                    StmtKind::For {
                        var: c_ident("l", name),
                        ty: t,
                        lo: l,
                        hi: hi_excl,
                        body: b,
                    },
                );
            }
            n::StmtKind::While {
                pattern,
                cond,
                max,
                body,
            } => {
                if pattern.is_some() {
                    self.unsupported("`while let`", s.span);
                    return;
                }
                let Some(m) = max.as_ref().and_then(|m| self.int_of(m)) else {
                    self.unsupported("a `while` without a constant `max` bound", s.span);
                    return;
                };
                if let Some(c) = self.expr(cond, Some(Ty::Bool)) {
                    let b = self.block(body);
                    self.push(
                        out,
                        s.span,
                        StmtKind::While {
                            cond: c,
                            max: m,
                            body: b,
                        },
                    );
                }
            }
            n::StmtKind::Loop { max, body } => {
                let Some(m) = max.as_ref().and_then(|m| self.int_of(m)) else {
                    self.unsupported("a `loop` without a constant `max` bound", s.span);
                    return;
                };
                let b = self.block(body);
                self.push(
                    out,
                    s.span,
                    StmtKind::While {
                        cond: Expr::Bool(true),
                        max: m,
                        body: b,
                    },
                );
            }
            n::StmtKind::Atomic { body } => {
                let mut b = self.block(body);
                out.append(&mut b);
            }
            n::StmtKind::Match { .. } => self.unsupported("`match`", s.span),
            n::StmtKind::Coil { .. } => self.unsupported("`set` and `reset` coils", s.span),
            n::StmtKind::Wait { .. } => self.unsupported("`wait`", s.span),
            n::StmtKind::Until { .. } => self.unsupported("`until`", s.span),
            n::StmtKind::Jump { .. } => self.unsupported("a step jump", s.span),
            n::StmtKind::Move { .. } => self.unsupported("a robot `move`", s.span),
            n::StmtKind::Rule { .. } => self.unsupported("a `when` rule inside a body", s.span),
            n::StmtKind::Asm { .. } => self.unsupported("inline assembly", s.span),
        }
    }

    fn if_chain(
        &mut self,
        span: n::Span,
        branches: &[n::IfBranch],
        otherwise: Option<&n::Block>,
    ) -> Option<Stmt> {
        let mut tail: Vec<Stmt> = otherwise.map(|b| self.block(b)).unwrap_or_default();
        let mut result: Option<Stmt> = None;
        for br in branches.iter().rev() {
            let cond = self.expr(&br.cond, Some(Ty::Bool));
            let then = self.block(&br.body);
            let cond = cond?;
            let st = Stmt {
                loc: self.loc(span),
                kind: StmtKind::If {
                    cond,
                    then,
                    otherwise: std::mem::take(&mut tail),
                },
            };
            tail = vec![st.clone()];
            result = Some(st);
        }
        result
    }

    fn place_ty(&mut self, p: &n::Expr) -> Option<(Place, Ty, Expr)> {
        match &p.e {
            n::ExprKind::Local { name, .. } => {
                let t = concrete(p)?;
                let c = c_ident("l", name);
                Some((Place::Local(c.clone()), t, Expr::Local(c, t)))
            }
            n::ExprKind::Ref { id, name } => match self.ents.get(id).cloned() {
                Some(Ent::Var(c, t)) => {
                    self.touches.insert(c.clone());
                    Some((Place::Global(c.clone()), t, Expr::Global(c, t)))
                }
                Some(Ent::Signal(c, t)) => {
                    self.touches.insert(c.clone());
                    let task = self.act == Act::Task;
                    if task {
                        self.writes.insert(c.clone());
                        self.reads.insert(c.clone());
                    }
                    let access = if task {
                        SigAccess::Snapshot
                    } else {
                        SigAccess::Current
                    };
                    Some((
                        Place::Signal {
                            name: c.clone(),
                            deferred: task,
                        },
                        t,
                        Expr::Signal(c, t, access),
                    ))
                }
                _ => {
                    self.unsupported(&format!("assignment to `{name}`"), p.span);
                    None
                }
            },
            _ => {
                self.unsupported("this assignment target", p.span);
                None
            }
        }
    }

    fn assign(
        &mut self,
        span: n::Span,
        place: &n::Expr,
        op: &str,
        value: &n::Expr,
        out: &mut Vec<Stmt>,
    ) {
        // `pwm.duty = 35 %`
        if let n::ExprKind::Field { base, name, .. } = &place.e {
            if let (n::ExprKind::Ref { id, .. }, "duty") = (&base.e, name.as_str()) {
                if let Some(Ent::Io(io)) = self.ents.get(id).cloned() {
                    if op != "=" {
                        self.unsupported("a compound assignment to `duty`", span);
                        return;
                    }
                    if let Some(v) = self.expr(value, Some(Ty::F32)) {
                        let v = coerce(v, Ty::F32);
                        self.push(out, span, StmtKind::IoPwm { io, value: v });
                    }
                    return;
                }
            }
            self.unsupported("assignment to a field", span);
            return;
        }
        let Some((pl, ty, read)) = self.place_ty(place) else {
            return;
        };
        let Some(v) = self.expr(value, Some(ty)) else {
            return;
        };
        let v = coerce(v, ty);
        let value = if op == "=" {
            v
        } else {
            let Some(bop) = bin_op(op.trim_end_matches('=')) else {
                self.unsupported(&format!("the operator `{op}`"), span);
                return;
            };
            Expr::Binary(bop, Box::new(read), Box::new(v), ty)
        };
        self.push(out, span, StmtKind::Assign { place: pl, value });
    }

    fn expr_stmt(&mut self, span: n::Span, e: &n::Expr, out: &mut Vec<Stmt>) {
        let n::ExprKind::Call {
            callee,
            args,
            target,
        } = &e.e
        else {
            self.unsupported("an expression used as a statement", span);
            return;
        };
        match target {
            Some(n::CallTarget::Io { binding, method }) => {
                let io = match self.hw.bindings.iter().position(|b| {
                    &b.name == binding
                        && self
                            .nir
                            .modules
                            .get(b.module)
                            .is_some_and(|m| m.file == self.file_name)
                }) {
                    Some(i) => i,
                    None => {
                        self.unsupported(&format!("the `io` `{binding}`"), span);
                        return;
                    }
                };
                match method.as_str() {
                    "write" | "set" => {
                        let Some(a) = args.first() else {
                            self.unsupported("`write` without a value", span);
                            return;
                        };
                        if let Some(v) = self.expr(&a.value, Some(Ty::Bool)) {
                            self.push(out, span, StmtKind::IoPut { io, value: v });
                        }
                    }
                    "toggle" => self.push(out, span, StmtKind::IoFlip { io }),
                    "set_duty" => {
                        if let Some(a) = args.first() {
                            if let Some(v) = self.expr(&a.value, Some(Ty::F32)) {
                                self.push(
                                    out,
                                    span,
                                    StmtKind::IoPwm {
                                        io,
                                        value: coerce(v, Ty::F32),
                                    },
                                );
                            }
                        }
                    }
                    "print" | "println" | "send" | "write_str" => {
                        let mut parts = Vec::new();
                        for a in args {
                            match &a.value.e {
                                n::ExprKind::Lit {
                                    lit: n::Lit::Str { text },
                                } => {
                                    if let Some(p) = self.print_parts(text, a.value.span) {
                                        parts.extend(p);
                                    }
                                }
                                n::ExprKind::Lit {
                                    lit: n::Lit::RawStr { text },
                                } => {
                                    let t =
                                        text.trim_start_matches('r').trim_matches('"').to_string();
                                    parts.push(PrintPart::Text(t));
                                }
                                _ => {
                                    if let Some(v) = self.expr(&a.value, None) {
                                        parts.push(PrintPart::Value(v));
                                    }
                                }
                            }
                        }
                        self.push(
                            out,
                            span,
                            StmtKind::Print {
                                io,
                                parts,
                                newline: method == "println",
                            },
                        );
                    }
                    "read" | "get" => {}
                    other => self.unsupported(&format!("the operation `{other}` of an `io`"), span),
                }
            }
            Some(n::CallTarget::Fn { id }) => {
                let Some(Ent::Func(cname, tys, _)) = self.ents.get(id).cloned() else {
                    self.unsupported("a call to this function", span);
                    return;
                };
                let mut argv = Vec::new();
                for (a, t) in args.iter().zip(&tys) {
                    match self.expr(&a.value, Some(*t)) {
                        Some(v) => argv.push(coerce(v, *t)),
                        None => return,
                    }
                }
                self.calls.insert(cname.clone());
                self.push(
                    out,
                    span,
                    StmtKind::CallVoid {
                        name: cname,
                        args: argv,
                    },
                );
            }
            _ => {
                let what = crate_flat(callee).unwrap_or_else(|| "this call".to_string());
                self.unsupported(&format!("the call `{what}`"), span);
            }
        }
    }

    /// Splits a string literal into text and `{name}` values.
    fn print_parts(&mut self, lit: &str, span: n::Span) -> Option<Vec<PrintPart>> {
        let inner = lit
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(lit);
        let mut parts = Vec::new();
        let mut text = String::new();
        let mut chars = inner.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some('n') => text.push('\n'),
                    Some('t') => text.push('\t'),
                    Some('r') => text.push('\r'),
                    Some('0') => text.push('\0'),
                    Some(o) => text.push(o),
                    None => {}
                },
                '{' => {
                    let mut name = String::new();
                    for d in chars.by_ref() {
                        if d == '}' {
                            break;
                        }
                        name.push(d);
                    }
                    let var = name.split(':').next().unwrap_or("").trim().to_string();
                    if !text.is_empty() {
                        parts.push(PrintPart::Text(std::mem::take(&mut text)));
                    }
                    let e = self.named_value(&var, span)?;
                    parts.push(PrintPart::Value(e));
                }
                other => text.push(other),
            }
        }
        if !text.is_empty() {
            parts.push(PrintPart::Text(text));
        }
        Some(parts)
    }

    fn named_value(&mut self, name: &str, span: n::Span) -> Option<Expr> {
        let id = self.names.get(name).cloned();
        match id.as_ref().and_then(|i| self.ents.get(i).cloned()) {
            Some(Ent::Var(c, t)) => {
                self.touches.insert(c.clone());
                Some(Expr::Global(c, t))
            }
            Some(Ent::Signal(c, t)) => {
                self.touches.insert(c.clone());
                Some(Expr::Signal(c, t, SigAccess::Current))
            }
            Some(Ent::Const(c)) => Some(self.cv_expr(
                c,
                match c {
                    Cv::B(_) => Ty::Bool,
                    Cv::I(_) => Ty::I32,
                    Cv::F(_) => Ty::F32,
                },
            )),
            _ => {
                self.unsupported(
                    &format!(
                        "`{{{name}}}` in a string (only module variables, signals and constants)"
                    ),
                    span,
                );
                None
            }
        }
    }

    // ---- expressions ----------------------------------------------------------------------

    fn expr(&mut self, e: &n::Expr, want: Option<Ty>) -> Option<Expr> {
        if let Some(c) = self.cv(e) {
            let ty = match c {
                Cv::B(_) => Ty::Bool,
                Cv::F(_) => match concrete(e).or(want) {
                    Some(t) if t.is_int() => Ty::F32,
                    Some(t) => t,
                    None => Ty::F32,
                },
                Cv::I(_) => concrete(e).or(want).unwrap_or(Ty::I32),
            };
            return Some(self.cv_expr(c, ty));
        }
        match &e.e {
            n::ExprKind::Local { name, .. } => {
                let t = concrete(e).or(want)?;
                Some(Expr::Local(c_ident("l", name), t))
            }
            n::ExprKind::Ref { id, name } => match self.ents.get(id).cloned() {
                Some(Ent::Var(c, t)) => {
                    self.touches.insert(c.clone());
                    Some(Expr::Global(c, t))
                }
                Some(Ent::Signal(c, t)) => {
                    self.touches.insert(c.clone());
                    let access = if self.act == Act::Task {
                        self.reads.insert(c.clone());
                        SigAccess::Snapshot
                    } else {
                        SigAccess::Current
                    };
                    Some(Expr::Signal(c, t, access))
                }
                _ => {
                    self.unsupported(&format!("the name `{name}` in an expression"), e.span);
                    None
                }
            },
            n::ExprKind::Unary { op, operand } => {
                let (uop, w) = match op.as_str() {
                    "-" => (UnOp::Neg, concrete(e).or(want)),
                    "not" => (UnOp::Not, Some(Ty::Bool)),
                    "~" => (UnOp::BitNot, concrete(e).or(want)),
                    other => {
                        self.unsupported(&format!("the operator `{other}`"), e.span);
                        return None;
                    }
                };
                let o = self.expr(operand, w)?;
                let t = if uop == UnOp::Not { Ty::Bool } else { o.ty() };
                Some(Expr::Unary(uop, Box::new(o), t))
            }
            n::ExprKind::Binary { op, lhs, rhs } => self.binary(e, op, lhs, rhs, want),
            n::ExprKind::Compare { first, rest } => {
                let mut acc: Option<Expr> = None;
                let mut left = first.as_ref();
                for r in rest {
                    let bop = bin_op(&r.op).filter(|b| b.is_compare());
                    let Some(bop) = bop else {
                        self.unsupported(&format!("the comparison `{}`", r.op), e.span);
                        return None;
                    };
                    let t = concrete(left).or_else(|| concrete(&r.rhs)).or_else(|| {
                        if matches!(self.cv(left), Some(Cv::F(_)))
                            || matches!(self.cv(&r.rhs), Some(Cv::F(_)))
                        {
                            Some(Ty::F32)
                        } else {
                            Some(Ty::I32)
                        }
                    })?;
                    let a = coerce(self.expr(left, Some(t))?, t);
                    let b = coerce(self.expr(&r.rhs, Some(t))?, t);
                    let cmp = Expr::Binary(bop, Box::new(a), Box::new(b), Ty::Bool);
                    acc = Some(match acc {
                        None => cmp,
                        Some(prev) => {
                            Expr::Binary(BinOp::And, Box::new(prev), Box::new(cmp), Ty::Bool)
                        }
                    });
                    left = &r.rhs;
                }
                acc
            }
            n::ExprKind::IfElse {
                cond,
                then,
                otherwise,
            } => {
                let c = self.expr(cond, Some(Ty::Bool))?;
                let t = concrete(e).or(want);
                let a = self.expr(then, t)?;
                let b = self.expr(otherwise, Some(a.ty()))?;
                let ty = a.ty();
                Some(Expr::IfElse(
                    Box::new(c),
                    Box::new(a),
                    Box::new(coerce(b, ty)),
                    ty,
                ))
            }
            n::ExprKind::Cast { operand, .. } => {
                let t = concrete(e).or(want)?;
                let o = self.expr(operand, None)?;
                Some(Expr::Cast(Box::new(o), t))
            }
            n::ExprKind::Call {
                callee,
                args,
                target,
            } => self.call_expr(e, callee, args, target.as_ref(), want),
            n::ExprKind::Pipe { lhs, rhs } => self.pipe(e, lhs, rhs),
            _ => {
                self.unsupported("this expression", e.span);
                None
            }
        }
    }

    fn binary(
        &mut self,
        e: &n::Expr,
        op: &str,
        lhs: &n::Expr,
        rhs: &n::Expr,
        want: Option<Ty>,
    ) -> Option<Expr> {
        let Some(bop) = bin_op(op) else {
            self.unsupported(&format!("the operator `{op}`"), e.span);
            return None;
        };
        if matches!(bop, BinOp::And | BinOp::Or) {
            let a = self.expr(lhs, Some(Ty::Bool))?;
            let b = self.expr(rhs, Some(Ty::Bool))?;
            return Some(Expr::Binary(bop, Box::new(a), Box::new(b), Ty::Bool));
        }
        let literal_default = || {
            if matches!(self.cv(lhs), Some(Cv::F(_))) || matches!(self.cv(rhs), Some(Cv::F(_))) {
                Ty::F32
            } else {
                Ty::I32
            }
        };
        let t = concrete(lhs)
            .or_else(|| concrete(rhs))
            .or_else(|| {
                if bop.is_compare() {
                    None
                } else {
                    concrete(e).or(want)
                }
            })
            .unwrap_or_else(literal_default);
        if t == Ty::Bool
            || (t.is_float()
                && matches!(
                    bop,
                    BinOp::Rem
                        | BinOp::BitAnd
                        | BinOp::BitOr
                        | BinOp::BitXor
                        | BinOp::Shl
                        | BinOp::Shr
                        | BinOp::AddW
                        | BinOp::SubW
                        | BinOp::MulW
                ))
        {
            self.unsupported(&format!("the operator `{op}` on `{}`", t.name()), e.span);
            return None;
        }
        let a = coerce(self.expr(lhs, Some(t))?, t);
        // A shift count keeps its own type.
        let b = if matches!(bop, BinOp::Shl | BinOp::Shr) {
            self.expr(rhs, Some(Ty::U8))?
        } else {
            coerce(self.expr(rhs, Some(t))?, t)
        };
        let res = if bop.is_compare() { Ty::Bool } else { t };
        Some(Expr::Binary(bop, Box::new(a), Box::new(b), res))
    }

    fn call_expr(
        &mut self,
        e: &n::Expr,
        callee: &n::Expr,
        args: &[n::Arg],
        target: Option<&n::CallTarget>,
        want: Option<Ty>,
    ) -> Option<Expr> {
        match target {
            Some(n::CallTarget::Io { binding, method })
                if matches!(method.as_str(), "read" | "get") =>
            {
                let io = self.hw.bindings.iter().position(|b| {
                    &b.name == binding
                        && self
                            .nir
                            .modules
                            .get(b.module)
                            .is_some_and(|m| m.file == self.file_name)
                })?;
                let t = match self.prog.ios[io].kind {
                    IoK::Adc => Ty::U16,
                    _ => Ty::Bool,
                };
                Some(Expr::IoGet(io, t))
            }
            Some(n::CallTarget::Fn { id }) => {
                let Some(Ent::Func(cname, tys, ret)) = self.ents.get(id).cloned() else {
                    self.unsupported("a call to this function", e.span);
                    return None;
                };
                let Some(ret) = ret else {
                    self.unsupported("a function without a result used as a value", e.span);
                    return None;
                };
                let mut argv = Vec::new();
                for (a, t) in args.iter().zip(&tys) {
                    argv.push(coerce(self.expr(&a.value, Some(*t))?, *t));
                }
                self.calls.insert(cname.clone());
                Some(Expr::Call(cname, argv, ret))
            }
            Some(n::CallTarget::Builtin { name })
                if matches!(name.as_str(), "abs" | "min" | "max" | "clamp") =>
            {
                let t = args
                    .iter()
                    .find_map(|a| concrete(&a.value))
                    .or_else(|| concrete(e))
                    .or(want)
                    .unwrap_or(Ty::F32);
                let mut argv = Vec::new();
                for a in args {
                    argv.push(coerce(self.expr(&a.value, Some(t))?, t));
                }
                Some(Expr::Builtin(name.clone(), argv, t))
            }
            _ => {
                let what = crate_flat(callee).unwrap_or_else(|| "this call".to_string());
                self.unsupported(&format!("the call `{what}`"), e.span);
                None
            }
        }
    }

    fn pipe(&mut self, e: &n::Expr, lhs: &n::Expr, rhs: &n::Expr) -> Option<Expr> {
        let n::ExprKind::Call {
            args,
            target,
            callee,
        } = &rhs.e
        else {
            self.unsupported("a pipeline into something other than a call", e.span);
            return None;
        };
        match target {
            Some(n::CallTarget::Builtin { name }) if name == "scale" => {
                let Some(first) = args.first() else {
                    self.unsupported("`scale` without a mapping", e.span);
                    return None;
                };
                let n::ExprKind::Map { from, to } = &first.value.e else {
                    self.unsupported("`scale` with something other than `a..b -> c..d`", e.span);
                    return None;
                };
                let (
                    n::ExprKind::Range { lo: a, hi: b, .. },
                    n::ExprKind::Range { lo: c, hi: d, .. },
                ) = (&from.e, &to.e)
                else {
                    self.unsupported("`scale` with something other than ranges", e.span);
                    return None;
                };
                let x = coerce(self.expr(lhs, None)?, Ty::F32);
                let mut v = vec![x];
                for part in [a, b, c, d] {
                    v.push(coerce(self.expr(part, Some(Ty::F32))?, Ty::F32));
                }
                Some(Expr::Scale(v))
            }
            Some(n::CallTarget::Fn { id }) => {
                let Some(Ent::Func(cname, tys, ret)) = self.ents.get(id).cloned() else {
                    self.unsupported("a pipeline into this function", e.span);
                    return None;
                };
                let ret = ret?;
                let mut argv = vec![coerce(self.expr(lhs, tys.first().copied())?, *tys.first()?)];
                for (a, t) in args.iter().zip(tys.iter().skip(1)) {
                    argv.push(coerce(self.expr(&a.value, Some(*t))?, *t));
                }
                self.calls.insert(cname.clone());
                Some(Expr::Call(cname, argv, ret))
            }
            _ => {
                let what = crate_flat(callee).unwrap_or_else(|| "this call".to_string());
                self.unsupported(&format!("a pipeline into `{what}`"), e.span);
                None
            }
        }
    }
}

fn cv_f(c: Cv) -> Option<f64> {
    match c {
        Cv::I(i) => Some(i as f64),
        Cv::F(f) => Some(f),
        Cv::B(_) => None,
    }
}

fn edge_name(e: Edge) -> &'static str {
    match e {
        Edge::Falling => "falling",
        Edge::Rising => "rising",
        Edge::Change => "change",
    }
}

/// Converts a numeric expression to `t` when its type differs (constants are re-typed).
fn coerce(e: Expr, t: Ty) -> Expr {
    if e.ty() == t || t == Ty::Bool || e.ty() == Ty::Bool {
        return e;
    }
    match e {
        Expr::Int(v, _) if t.is_int() => Expr::Int(v, t),
        Expr::Int(v, _) => Expr::Float(v as f64),
        other => Expr::Cast(Box::new(other), t),
    }
}

fn crate_flat(e: &n::Expr) -> Option<String> {
    neuron_target::flat(e)
}

fn item_kind(d: &n::ItemDef) -> &'static str {
    match d {
        n::ItemDef::Struct { .. } => "struct",
        n::ItemDef::Enum { .. } => "enum",
        n::ItemDef::Class { .. } => "class",
        n::ItemDef::Interface { .. } => "interface",
        n::ItemDef::Alias { .. } => "type alias",
        n::ItemDef::Sequence { .. } => "sequence",
        n::ItemDef::ExternBlock { .. } | n::ItemDef::ExternFn { .. } => "extern",
        _ => "this item",
    }
}
