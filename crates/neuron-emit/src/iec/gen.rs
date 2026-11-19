//! Lowering of one `plc` module to Structured Text, shared by the ST and SCL emitters.
//!
//! The order inside one scan follows spec 14.3 and the reference interpreter: the safety
//! logic, then rungs, rules, sequences and state machines in source order, then every
//! standard function block is called once (timers, counters, edge detectors, latches),
//! then the time accumulators of rules, sequences and state machines advance by one scan.

use crate::fidelity::{Class, Fidelity};
use crate::nirx::{arg, builtin, dur_ns, fmt_num, iec_time, int, is_time, si, unquote};
use crate::package::Writer;
use crate::Diag;
use neuron_nir::{
    Arg, Expr, ExprKind, Fsm, ItemDef, Lit, Nir, Rule, SafetyDef, SeqItem, Stmt, StmtKind, Type,
};
use std::collections::{BTreeMap, BTreeSet};

/// Spelling of the generated code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    /// IEC 61131-3 Structured Text.
    Iec,
    /// Siemens-style SCL: `#` before every local name, `TON_TIME`-style block types.
    Scl,
}

/// Function blocks of IEC 61131-3 that the interpreter advances at the end of a scan.
pub(crate) const STD_BLOCKS: &[&str] = &[
    "ton", "tof", "tp", "ctu", "ctd", "ctud", "r_trig", "f_trig", "sr", "rs",
];
/// Blocks of `std.control` that the emitter writes as helper function blocks.
pub(crate) const HELPER_BLOCKS: &[&str] = &[
    "hysteresis",
    "debounce",
    "ramp",
    "lowpass",
    "highpass",
    "moving_average",
    "pid",
];

const IEC_KEYWORDS: &[&str] = &[
    "ABS",
    "AND",
    "ANY",
    "ARRAY",
    "AT",
    "BOOL",
    "BY",
    "BYTE",
    "CASE",
    "CONFIGURATION",
    "CONSTANT",
    "CTD",
    "CTU",
    "CTUD",
    "DATE",
    "DINT",
    "DO",
    "DT",
    "DWORD",
    "ELSE",
    "ELSIF",
    "END_CASE",
    "END_FOR",
    "END_IF",
    "END_REPEAT",
    "END_VAR",
    "END_WHILE",
    "EXIT",
    "FALSE",
    "FOR",
    "FUNCTION",
    "FUNCTION_BLOCK",
    "F_TRIG",
    "IF",
    "INT",
    "LINT",
    "LREAL",
    "MOD",
    "NOT",
    "OF",
    "ON",
    "OR",
    "PROGRAM",
    "REAL",
    "REPEAT",
    "RESOURCE",
    "RETAIN",
    "RETURN",
    "RS",
    "R_TRIG",
    "SINT",
    "SR",
    "STRING",
    "TASK",
    "THEN",
    "TIME",
    "TO",
    "TOF",
    "TON",
    "TP",
    "TRUE",
    "TYPE",
    "UDINT",
    "UINT",
    "ULINT",
    "UNTIL",
    "USINT",
    "VAR",
    "WHILE",
    "WITH",
    "WORD",
    "XOR",
    "BEGIN",
    "VERSION",
    "REGION",
];

/// An IEC identifier for a Neuron name (keywords get a `_v` suffix).
pub(crate) fn ident(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, 'n');
    }
    if IEC_KEYWORDS.contains(&s.to_ascii_uppercase().as_str()) {
        s.push_str("_v");
    }
    s
}

/// `%I0.0` -> `%IX0.0`; word and double-word addresses are kept.
pub(crate) fn iec_addr(text: &str) -> String {
    let t = text.to_ascii_uppercase();
    let b = t.as_bytes();
    if b.len() > 2 && b[0] == b'%' && matches!(b[1], b'I' | b'Q' | b'M') && b[2].is_ascii_digit() {
        format!("%{}X{}", b[1] as char, &t[2..])
    } else {
        t
    }
}

/// IEC elementary type of a Neuron type.
pub(crate) fn iec_type(t: &Type) -> Option<&'static str> {
    if is_time(t) {
        return Some("TIME");
    }
    Some(match t {
        Type::Bool => "BOOL",
        Type::Num { kind, .. } => match kind.as_str() {
            "u8" => "USINT",
            "u16" => "UINT",
            "u32" => "UDINT",
            "u64" => "ULINT",
            "i8" => "SINT",
            "i16" => "INT",
            "i32" | "index" | "integer" => "DINT",
            "i64" => "LINT",
            "f32" => "REAL",
            "f64" | "real" => "LREAL",
            _ => return None,
        },
        Type::Str { .. } => "STRING",
        _ => return None,
    })
}

/// Neuron field of a block and its IEC name, and whether it is an input.
pub(crate) fn fb_fields(kind: &str) -> &'static [(&'static str, &'static str, bool)] {
    match kind {
        "ton" | "tof" | "tp" => &[
            ("input", "IN", true),
            ("q", "Q", false),
            ("et", "ET", false),
        ],
        "ctu" => &[
            ("input", "CU", true),
            ("reset", "R", true),
            ("q", "Q", false),
            ("cv", "CV", false),
        ],
        "ctd" => &[
            ("input", "CD", true),
            ("load", "LD", true),
            ("q", "Q", false),
            ("cv", "CV", false),
        ],
        "ctud" => &[
            ("input", "CU", true),
            ("down", "CD", true),
            ("reset", "R", true),
            ("load", "LD", true),
            ("qu", "QU", false),
            ("qd", "QD", false),
            ("cv", "CV", false),
        ],
        "r_trig" | "f_trig" => &[("input", "CLK", true), ("q", "Q", false)],
        "sr" => &[
            ("set", "S1", true),
            ("reset", "R", true),
            ("q", "Q1", false),
        ],
        "rs" => &[
            ("set", "S", true),
            ("reset", "R1", true),
            ("q", "Q1", false),
        ],
        "hysteresis" => &[("q", "Q", false)],
        "debounce" => &[("stable", "Q", false)],
        "ramp" => &[("value", "OUT", false)],
        "lowpass" | "highpass" => &[("y", "OUT", false)],
        _ => &[],
    }
}

/// A function block instance declared at module level.
#[derive(Debug, Clone)]
pub(crate) struct Block {
    /// Item id.
    pub id: String,
    /// IEC identifier.
    pub name: String,
    /// Neuron block name (`ton`, `hysteresis`).
    pub kind: String,
    /// Constructor arguments.
    pub args: Vec<Arg>,
    /// Location.
    pub span: [usize; 2],
}

/// A declared variable.
#[derive(Debug, Clone)]
pub(crate) struct PVar {
    /// IEC identifier.
    pub name: String,
    /// IEC type.
    pub ty: String,
    /// IEC direct address.
    pub at: Option<String>,
    /// `RETAIN`.
    pub retain: bool,
    /// Initial value.
    pub init: Option<String>,
    /// `CONSTANT`.
    pub constant: bool,
}

#[derive(Debug, Clone, Copy)]
enum Op<'b> {
    Step(&'b str),
    Stmt(&'b Stmt),
}

/// Lowering state of one module.
pub(crate) struct Gen<'a> {
    pub nir: &'a Nir,
    pub m: usize,
    pub file: String,
    pub style: Style,
    pub lang: &'static str,
    pub fid: Fidelity,
    pub diags: Vec<Diag>,
    pub name: String,
    pub scan_ns: i64,
    pub inputs: Vec<PVar>,
    pub outputs: Vec<PVar>,
    pub memory: Vec<PVar>,
    pub locals: Vec<PVar>,
    pub blocks: Vec<Block>,
    pub helpers: BTreeSet<&'static str>,
    pub state: Vec<PVar>,
    pub faults: Vec<String>,
    names: BTreeMap<String, String>,
    end: Vec<String>,
    safe: Vec<Stmt>,
    on_fault: Vec<(Option<String>, Vec<Stmt>)>,
    has_safety: bool,
    counters: BTreeMap<&'static str, usize>,
    noted: BTreeSet<&'static str>,
}

impl<'a> Gen<'a> {
    /// Collects the declarations of module `m`.
    pub fn new(nir: &'a Nir, m: usize, style: Style, fid: Fidelity) -> Gen<'a> {
        let module = &nir.modules[m];
        let mut g = Gen {
            nir,
            m,
            file: module.file.clone(),
            style,
            lang: if style == Style::Scl {
                "SCL"
            } else {
                "Structured Text"
            },
            fid,
            diags: Vec::new(),
            name: ident(module.name.rsplit('.').next().unwrap_or(&module.name)),
            scan_ns: 10_000_000,
            inputs: Vec::new(),
            outputs: Vec::new(),
            memory: Vec::new(),
            locals: Vec::new(),
            blocks: Vec::new(),
            helpers: BTreeSet::new(),
            state: Vec::new(),
            faults: Vec::new(),
            names: BTreeMap::new(),
            end: Vec::new(),
            safe: Vec::new(),
            on_fault: Vec::new(),
            has_safety: false,
            counters: BTreeMap::new(),
            noted: BTreeSet::new(),
        };
        for item in &module.items {
            match &item.def {
                ItemDef::Scan { period } => {
                    if let Some(ns) = dur_ns(period) {
                        g.scan_ns = ns.max(1);
                    }
                }
                ItemDef::PlcSection { section, vars } => {
                    for v in vars {
                        let name = ident(&v.name);
                        g.names.insert(v.id.clone(), name.clone());
                        let Some(ty) = iec_type(&v.vty) else {
                            g.unsupported(v.span, &format!("the type of `{}`", v.name));
                            continue;
                        };
                        let at = v.at.as_ref().and_then(|a| match &a.e {
                            ExprKind::Lit {
                                lit: Lit::Iec { text },
                            } => Some(iec_addr(text)),
                            _ => None,
                        });
                        let init = v.init.as_ref().and_then(|e| g.const_text(e));
                        let pv = PVar {
                            name,
                            ty: ty.to_string(),
                            at,
                            retain: v.retain,
                            init,
                            constant: false,
                        };
                        match section.as_str() {
                            "inputs" => g.inputs.push(pv),
                            "outputs" => g.outputs.push(pv),
                            _ => g.memory.push(pv),
                        }
                        g.fid.exact();
                    }
                }
                ItemDef::Var { var } => {
                    let name = ident(&var.name);
                    g.names.insert(var.id.clone(), name.clone());
                    let kind = var
                        .init
                        .as_ref()
                        .and_then(|e| builtin(e).map(str::to_string));
                    match kind {
                        Some(k)
                            if STD_BLOCKS.contains(&k.as_str())
                                || HELPER_BLOCKS.contains(&k.as_str()) =>
                        {
                            let args = match var.init.as_ref().map(|e| &e.e) {
                                Some(ExprKind::Call { args, .. }) => args.clone(),
                                _ => Vec::new(),
                            };
                            if HELPER_BLOCKS.contains(&k.as_str()) {
                                g.helpers.insert(helper_name(&k));
                            }
                            if matches!(k.as_str(), "ton" | "tof" | "tp") {
                                g.fid.record(
                                    &g.file.clone(),
                                    var.span,
                                    Class::Equivalent,
                                    &format!("timer `{}`", var.name),
                                    "with a wall-clock PLC timer, Q may change one scan later than in the reference interpreter, which counts scans (spec 14.3)",
                                );
                            } else {
                                g.fid.exact();
                            }
                            g.blocks.push(Block {
                                id: var.id.clone(),
                                name,
                                kind: k,
                                args,
                                span: var.span,
                            });
                        }
                        _ => {
                            let Some(ty) = iec_type(&var.vty) else {
                                g.unsupported(var.span, &format!("the type of `{}`", var.name));
                                continue;
                            };
                            let init = var.init.as_ref().and_then(|e| g.const_text(e));
                            g.locals.push(PVar {
                                name,
                                ty: ty.to_string(),
                                at: None,
                                retain: var.retain,
                                init,
                                constant: !var.mutable,
                            });
                            g.fid.exact();
                        }
                    }
                }
                ItemDef::Const { name, value, .. } => {
                    let id = ident(name);
                    g.names.insert(item.id.clone(), id.clone());
                    match (iec_type(&value.ty), g.const_text(value)) {
                        (Some(ty), Some(v)) => {
                            g.locals.push(PVar {
                                name: id,
                                ty: ty.to_string(),
                                at: None,
                                retain: false,
                                init: Some(v),
                                constant: true,
                            });
                            g.fid.exact();
                        }
                        _ => {
                            g.unsupported(item.span, &format!("the constant `{name}`"));
                        }
                    }
                }
                ItemDef::Safety { members } => {
                    g.has_safety = true;
                    for mem in members {
                        match &mem.def {
                            SafetyDef::SafeState { body } => {
                                g.safe.extend(body.stmts.iter().cloned())
                            }
                            SafetyDef::OnFault { fault, body } => {
                                g.on_fault.push((fault.clone(), body.stmts.clone()))
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        g
    }

    /// Identifier as written in the body.
    pub fn r(&self, name: &str) -> String {
        match self.style {
            Style::Iec => name.to_string(),
            Style::Scl => format!("#{name}"),
        }
    }

    /// Type name of a standard block in this style.
    pub fn std_type(&self, kind: &str) -> String {
        let iec = kind.to_ascii_uppercase();
        match (self.style, kind) {
            (Style::Scl, "ton" | "tof" | "tp") => format!("{iec}_TIME"),
            (Style::Scl, "ctu" | "ctd" | "ctud") => format!("{iec}_INT"),
            (Style::Scl, _) if HELPER_BLOCKS.contains(&kind) => {
                format!("\"{}\"", helper_name(kind))
            }
            (Style::Iec, _) if HELPER_BLOCKS.contains(&kind) => helper_name(kind).to_string(),
            _ => iec,
        }
    }

    fn unsupported(&mut self, span: [usize; 2], what: &str) -> String {
        self.diags.push(Diag {
            code: "NE0610",
            message: format!("{what} has no translation to {} yet", self.lang),
            help: Some("the reference interpreter runs it (`neuron run`); docs/translation.md lists what each emitter translates".into()),
            file: self.file.clone(),
            span,
        });
        self.fid.record(
            &self.file.clone(),
            span,
            Class::Unsupported,
            what,
            "not translated",
        );
        "0".to_string()
    }

    fn note(
        &mut self,
        key: &'static str,
        span: [usize; 2],
        class: Class,
        construct: &str,
        note: &str,
    ) {
        if self.noted.insert(key) {
            let f = self.file.clone();
            self.fid.record(&f, span, class, construct, note);
        }
    }

    fn fresh(&mut self, prefix: &'static str) -> usize {
        let c = self.counters.entry(prefix).or_insert(0);
        let n = *c;
        *c += 1;
        n
    }

    fn state_var(&mut self, name: String, ty: &str, init: Option<&str>) -> String {
        if !self.state.iter().any(|v| v.name == name) {
            self.state.push(PVar {
                name: name.clone(),
                ty: ty.to_string(),
                at: None,
                retain: false,
                init: init.map(str::to_string),
                constant: false,
            });
        }
        name
    }

    /// Constant text of an initial value.
    fn const_text(&mut self, e: &Expr) -> Option<String> {
        if is_time(&e.ty) {
            return dur_ns(e).map(iec_time);
        }
        match &e.e {
            ExprKind::Lit {
                lit: Lit::Bool { v },
            } => Some(if *v { "TRUE" } else { "FALSE" }.into()),
            ExprKind::Lit {
                lit: Lit::Int { v, .. },
            } => Some(v.clone()),
            ExprKind::Lit {
                lit: Lit::Real { v, .. },
            } => Some(real(v)),
            ExprKind::Unary { op, operand } if op == "-" => {
                self.const_text(operand).map(|t| format!("-{t}"))
            }
            ExprKind::Quantity { value, .. } => self.const_text(value),
            _ => None,
        }
    }

    pub fn block_of(&self, e: &Expr) -> Option<Block> {
        match &e.e {
            ExprKind::Ref { id, .. } => self.blocks.iter().find(|b| &b.id == id).cloned(),
            _ => None,
        }
    }

    pub fn shadow(&mut self, b: &Block, field: &str) -> String {
        let n = self.state_var(format!("{}_{field}", b.name), "BOOL", None);
        self.r(&n)
    }

    pub fn preset(&mut self, b: &Block) -> Option<String> {
        let e = arg(&b.args, "preset", 0)?.clone();
        if matches!(b.kind.as_str(), "ton" | "tof" | "tp") {
            Some(dur_ns(&e).map_or_else(|| "T#0S".to_string(), iec_time))
        } else {
            Some(int(&e).map_or_else(|| "0".to_string(), |v| v.to_string()))
        }
    }

    fn num_arg(&mut self, b: &Block, name: &str, pos: usize, default: &str) -> String {
        match arg(&b.args, name, pos) {
            Some(e) => {
                if is_time(&e.ty) {
                    return dur_ns(e).map_or_else(|| "T#0S".into(), iec_time);
                }
                match si(e) {
                    Some(v) => real(&fmt_num(crate::nirx::from_si(&e.ty, v))),
                    None => {
                        let span = e.span;
                        self.unsupported(
                            span,
                            &format!("argument `{name}` of `{}` (it must be a constant)", b.name),
                        )
                    }
                }
            }
            None => default.to_string(),
        }
    }

    /// The call of a standard block with its shadowed inputs and its preset.
    fn std_call(&mut self, b: &Block) -> String {
        let mut parts = Vec::new();
        for (_, iecf, input) in fb_fields(&b.kind) {
            if *input {
                parts.push(format!("{iecf} := {}", self.shadow(b, iecf)));
            }
        }
        if let Some(p) = self.preset(b) {
            let f = if matches!(b.kind.as_str(), "ton" | "tof" | "tp") {
                "PT"
            } else {
                "PV"
            };
            if b.kind != "r_trig" && b.kind != "f_trig" && b.kind != "sr" && b.kind != "rs" {
                parts.push(format!("{f} := {p}"));
            }
        }
        format!("{}({});", self.r(&b.name), parts.join(", "))
    }

    /// `b.step(x)`: the call written before the statement, and the value it gives.
    fn method(
        &mut self,
        b: &Block,
        method: &str,
        args: &[Arg],
        span: [usize; 2],
        pre: &mut Vec<String>,
    ) -> String {
        if !matches!(method, "step" | "update" | "call") {
            return self.unsupported(span, &format!("`{}.{method}()`", b.name));
        }
        let x = match args.first() {
            Some(a) => self.ex(&a.value, pre),
            None => "FALSE".into(),
        };
        let inst = self.r(&b.name);
        let dt = real(&fmt_num(self.scan_ns as f64 / 1e9));
        match b.kind.as_str() {
            k if STD_BLOCKS.contains(&k) => {
                let first = fb_fields(k).iter().find(|f| f.2).map_or("IN", |f| f.1);
                let sh = self.shadow(b, first);
                pre.push(format!("{sh} := {x};"));
                if matches!(k, "sr" | "rs") {
                    if let Some(r) = args.get(1) {
                        let rv = self.ex(&r.value, pre);
                        let rf = if k == "sr" { "R" } else { "R1" };
                        let rsh = self.shadow(b, rf);
                        pre.push(format!("{rsh} := {rv};"));
                    }
                }
                let call = self.std_call(b);
                pre.push(call);
                let out = match k {
                    "ctu" | "ctd" | "ctud" => "CV",
                    "sr" | "rs" => "Q1",
                    _ => "Q",
                };
                format!("{inst}.{out}")
            }
            "hysteresis" => {
                let lo = self.num_arg(b, "low", 0, "0.0");
                let hi = self.num_arg(b, "high", 1, "0.0");
                pre.push(format!("{inst}(X := {x}, LOW := {lo}, HIGH := {hi});"));
                format!("{inst}.Q")
            }
            "debounce" => {
                let t = self.num_arg(b, "time", 0, "T#0S");
                let now = self.now_var();
                pre.push(format!("{inst}(X := {x}, T := {t}, NOW := {now});"));
                format!("{inst}.Q")
            }
            "ramp" => {
                let rate = self.num_arg(b, "rate", 0, "0.0");
                pre.push(format!("{inst}(X := {x}, RATE := {rate}, DT := {dt});"));
                format!("{inst}.OUT")
            }
            "lowpass" | "highpass" => {
                let fc = self.num_arg(b, "cutoff", 0, "0.0");
                pre.push(format!("{inst}(X := {x}, CUTOFF := {fc}, DT := {dt});"));
                format!("{inst}.OUT")
            }
            "moving_average" => {
                let w = arg(&b.args, "window", 0).and_then(int).unwrap_or(4);
                if !(1..=64).contains(&w) {
                    return self.unsupported(b.span, "a moving average window outside 1..64");
                }
                pre.push(format!("{inst}(X := {x}, WINDOW := {w});"));
                format!("{inst}.OUT")
            }
            "pid" => {
                let kp = self.num_arg(b, "kp", 0, "0.0");
                let ki = self.num_arg(b, "ki", 1, "0.0");
                let kd = self.num_arg(b, "kd", 2, "0.0");
                let (lo, hi) = match arg(&b.args, "out", 3).map(|e| &e.e) {
                    Some(ExprKind::Range { lo, hi, .. }) => {
                        let (lo, hi) = ((**lo).clone(), (**hi).clone());
                        let l = si(&lo).map_or("-1.0E30".into(), |v| {
                            real(&fmt_num(crate::nirx::from_si(&lo.ty, v)))
                        });
                        let h = si(&hi).map_or("1.0E30".into(), |v| {
                            real(&fmt_num(crate::nirx::from_si(&hi.ty, v)))
                        });
                        (l, h)
                    }
                    _ => ("-1.0E30".into(), "1.0E30".into()),
                };
                let aw = match arg(&b.args, "antiwindup", 4).and_then(crate::nirx::boolean) {
                    Some(false) => "FALSE",
                    _ => "TRUE",
                };
                pre.push(format!(
                    "{inst}(E := {x}, KP := {kp}, KI := {ki}, KD := {kd}, LO := {lo}, HI := {hi}, ANTIWINDUP := {aw}, DT := {dt});"
                ));
                format!("{inst}.OUT")
            }
            other => self.unsupported(span, &format!("the block `{other}`")),
        }
    }

    fn now_var(&mut self) -> String {
        if !self.noted.contains("now") {
            self.noted.insert("now");
            let s = iec_time(self.scan_ns);
            let n = self.state_var("neuron_now".into(), "TIME", None);
            let r = self.r(&n);
            self.end.push(format!("{r} := {r} + {s};"));
        }
        self.r("neuron_now")
    }

    // ---- expressions ------------------------------------------------------------------

    fn operand(&mut self, e: &Expr, pre: &mut Vec<String>) -> String {
        let s = self.ex(e, pre);
        match &e.e {
            ExprKind::Binary { .. } | ExprKind::Compare { .. } => format!("({s})"),
            ExprKind::Unary { op, .. } if op == "-" => format!("({s})"),
            _ => s,
        }
    }

    pub fn ex(&mut self, e: &Expr, pre: &mut Vec<String>) -> String {
        match &e.e {
            ExprKind::Lit { lit } => match lit {
                Lit::Bool { v } => if *v { "TRUE" } else { "FALSE" }.into(),
                Lit::Int { v, .. } => v.clone(),
                Lit::Real { v, .. } => real(v),
                Lit::Str { text } | Lit::RawStr { text } => {
                    format!("'{}'", unquote(text).replace('\'', "$'"))
                }
                Lit::Iec { text } => iec_addr(text),
                _ => self.unsupported(e.span, "this literal"),
            },
            ExprKind::Quantity { value, unit, .. } => {
                if is_time(&e.ty) {
                    match dur_ns(e) {
                        Some(ns) => iec_time(ns),
                        None => self.unsupported(e.span, "a duration that is not a constant"),
                    }
                } else {
                    let u = unit.text.clone();
                    self.note(
                        "units",
                        e.span,
                        Class::Equivalent,
                        "quantity with a unit",
                        &format!("numbers are written in their unit ({u}); PLC variables hold values in the units they are declared with"),
                    );
                    self.ex(value, pre)
                }
            }
            ExprKind::Ref { id, name } => match self.names.get(id).cloned() {
                Some(n) => {
                    if self.blocks.iter().any(|b| &b.id == id) {
                        self.unsupported(e.span, &format!("using the block `{name}` as a value"))
                    } else {
                        self.r(&n)
                    }
                }
                None => self.unsupported(e.span, &format!("the reference to `{name}`")),
            },
            ExprKind::Local { name, .. } => {
                let n = ident(name);
                self.r(&n)
            }
            ExprKind::Field { base, name, .. } => match self.block_of(base) {
                Some(b) => match fb_fields(&b.kind).iter().find(|f| f.0 == name.as_str()) {
                    Some((_, iecf, true)) => self.shadow(&b, iecf),
                    Some((_, iecf, false)) => format!("{}.{iecf}", self.r(&b.name)),
                    None if name == "preset" => self.preset(&b).unwrap_or_else(|| "0".into()),
                    None => self.unsupported(e.span, &format!("the field `{}.{name}`", b.name)),
                },
                None => self.unsupported(e.span, &format!("the field `.{name}`")),
            },
            ExprKind::Binary { op, lhs, rhs } => {
                let a = self.operand(lhs, pre);
                let b = self.operand(rhs, pre);
                if matches!(op.as_str(), "+" | "-" | "*")
                    && matches!(e.ty, Type::Num { ref kind, .. } if kind.starts_with(['u', 'i']))
                {
                    self.note(
                        "overflow",
                        e.span,
                        Class::Equivalent,
                        "integer arithmetic",
                        "an integer overflow wraps around on the PLC; the reference interpreter enters the safe state",
                    );
                }
                let o = match op.as_str() {
                    "and" | "&&" => "AND",
                    "or" | "||" => "OR",
                    "xor" => "XOR",
                    "&" => "AND",
                    "|" => "OR",
                    "^" => "XOR",
                    "%" => "MOD",
                    "==" => "=",
                    "!=" => "<>",
                    "+" | "-" | "*" | "/" | "<" | ">" | "<=" | ">=" => op.as_str(),
                    "<<" => return format!("SHL({a}, {b})"),
                    ">>" => return format!("SHR({a}, {b})"),
                    other => return self.unsupported(e.span, &format!("the operator `{other}`")),
                };
                format!("{a} {o} {b}")
            }
            ExprKind::Compare { first, rest } => {
                let mut parts = Vec::new();
                let mut prev = self.operand(first, pre);
                for c in rest {
                    let next = self.operand(&c.rhs, pre);
                    let o = match c.op.as_str() {
                        "==" => "=",
                        "!=" => "<>",
                        other => other,
                    }
                    .to_string();
                    parts.push(format!("{prev} {o} {next}"));
                    prev = next;
                }
                if parts.len() == 1 {
                    parts.remove(0)
                } else {
                    parts
                        .iter()
                        .map(|p| format!("({p})"))
                        .collect::<Vec<_>>()
                        .join(" AND ")
                }
            }
            ExprKind::Unary { op, operand } => {
                let x = self.operand(operand, pre);
                match op.as_str() {
                    "not" | "!" | "~" => format!("NOT {x}"),
                    "-" => format!("-{x}"),
                    other => self.unsupported(e.span, &format!("the operator `{other}`")),
                }
            }
            ExprKind::IfElse {
                cond,
                then,
                otherwise,
            } => {
                let c = self.ex(cond, pre);
                let t = self.ex(then, pre);
                let f = self.ex(otherwise, pre);
                format!("SEL({c}, {f}, {t})")
            }
            ExprKind::Cast { operand, .. } => {
                let x = self.ex(operand, pre);
                match iec_type(&e.ty) {
                    Some(t) => format!("TO_{t}({x})"),
                    None => self.unsupported(e.span, "this conversion"),
                }
            }
            ExprKind::Call { callee, args, .. } => {
                if let ExprKind::Field { base, name, .. } = &callee.e {
                    if let Some(b) = self.block_of(base) {
                        return self.method(&b, name, args, e.span, pre);
                    }
                }
                let Some(f) = builtin(e).map(str::to_string) else {
                    return self.unsupported(e.span, "this call");
                };
                let mut xs = Vec::new();
                for a in args {
                    xs.push(self.ex(&a.value, pre));
                }
                let one = |n: &str, xs: &[String]| format!("{n}({})", xs.join(", "));
                match f.as_str() {
                    "abs" => one("ABS", &xs),
                    "min" => one("MIN", &xs),
                    "max" => one("MAX", &xs),
                    "sqrt" => one("SQRT", &xs),
                    "sin" => one("SIN", &xs),
                    "cos" => one("COS", &xs),
                    "tan" => one("TAN", &xs),
                    "asin" => one("ASIN", &xs),
                    "acos" => one("ACOS", &xs),
                    "atan" => one("ATAN", &xs),
                    "exp" => one("EXP", &xs),
                    "log" => one("LN", &xs),
                    "log10" => one("LOG", &xs),
                    "pow" => one("EXPT", &xs),
                    "clamp" if xs.len() == 3 => format!("LIMIT({}, {}, {})", xs[1], xs[0], xs[2]),
                    other => self.unsupported(e.span, &format!("the function `{other}`")),
                }
            }
            _ => self.unsupported(e.span, "this expression"),
        }
    }

    pub fn place(&mut self, p: &Expr) -> String {
        match &p.e {
            ExprKind::Ref { .. } | ExprKind::Local { .. } => {
                let mut pre = Vec::new();
                self.ex(p, &mut pre)
            }
            ExprKind::Field { base, name, .. } => match self.block_of(base) {
                Some(b) => match fb_fields(&b.kind).iter().find(|f| f.0 == name.as_str()) {
                    Some((_, iecf, true)) => self.shadow(&b, iecf),
                    _ => self.unsupported(p.span, &format!("assigning to `{}.{name}`", b.name)),
                },
                None => self.unsupported(p.span, &format!("assigning to `.{name}`")),
            },
            _ => self.unsupported(p.span, "this assignment target"),
        }
    }

    // ---- statements ---------------------------------------------------------------

    fn flush(w: &mut Writer, pre: Vec<String>) {
        for l in pre {
            w.line(&l);
        }
    }

    pub fn stmts(&mut self, b: &[Stmt], w: &mut Writer) {
        for s in b {
            self.stmt(s, w);
        }
    }

    pub fn stmt(&mut self, s: &Stmt, w: &mut Writer) {
        let mut pre = Vec::new();
        match &s.s {
            StmtKind::Assign {
                place,
                op,
                value,
                when,
            } => {
                let p = self.place(place);
                let rhs = match op.as_str() {
                    "=" => self.ex(value, &mut pre),
                    "+=" | "-=" | "*=" | "/=" | "%=" => {
                        if matches!(place.ty, Type::Num { ref kind, .. } if kind.starts_with(['u', 'i']))
                        {
                            self.note(
                                "overflow",
                                s.span,
                                Class::Equivalent,
                                "integer arithmetic",
                                "an integer overflow wraps around on the PLC; the reference interpreter enters the safe state",
                            );
                        }
                        let v = self.operand(value, &mut pre);
                        let o = if op == "%=" { "MOD" } else { &op[..1] };
                        format!("{p} {o} {v}")
                    }
                    other => self.unsupported(s.span, &format!("the assignment `{other}`")),
                };
                match when {
                    Some(c) => {
                        let c = self.ex(c, &mut pre);
                        Self::flush(w, pre);
                        w.line(&format!("IF {c} THEN"));
                        w.indent();
                        w.line(&format!("{p} := {rhs};"));
                        w.dedent();
                        w.line("END_IF;");
                    }
                    None => {
                        Self::flush(w, pre);
                        w.line(&format!("{p} := {rhs};"));
                    }
                }
            }
            StmtKind::Coil { set, place, when } => {
                let c = self.ex(when, &mut pre);
                let p = self.place(place);
                Self::flush(w, pre);
                w.line(&format!("IF {c} THEN"));
                w.indent();
                w.line(&format!("{p} := {};", if *set { "TRUE" } else { "FALSE" }));
                w.dedent();
                w.line("END_IF;");
            }
            StmtKind::Expr { expr } => {
                let is_method = matches!(&expr.e, ExprKind::Call { callee, .. }
                    if matches!(&callee.e, ExprKind::Field { base, .. } if self.block_of(base).is_some()));
                if is_method {
                    let _ = self.ex(expr, &mut pre);
                    Self::flush(w, pre);
                } else {
                    self.unsupported(s.span, "this expression statement");
                }
            }
            StmtKind::Pass => w.line("(* pass *)"),
            StmtKind::If {
                branches,
                otherwise,
            } => {
                for (k, br) in branches.iter().enumerate() {
                    let mut p2 = Vec::new();
                    let c = self.ex(&br.cond, &mut p2);
                    if k == 0 {
                        Self::flush(w, std::mem::take(&mut pre));
                        Self::flush(w, p2);
                        w.line(&format!("IF {c} THEN"));
                    } else {
                        if !p2.is_empty() {
                            self.unsupported(br.cond.span, "a block call in an `elif` condition");
                        }
                        w.line(&format!("ELSIF {c} THEN"));
                    }
                    w.indent();
                    self.stmts(&br.body.stmts, w);
                    w.dedent();
                }
                if let Some(o) = otherwise {
                    w.line("ELSE");
                    w.indent();
                    self.stmts(&o.stmts, w);
                    w.dedent();
                }
                w.line("END_IF;");
            }
            StmtKind::For {
                pattern,
                iter,
                body,
                ..
            } => {
                let (
                    ExprKind::Range { lo, hi, inclusive },
                    neuron_nir::PatternKind::Bind { name, .. },
                ) = (&iter.e, &pattern.p)
                else {
                    self.unsupported(s.span, "a `for` loop that is not over a constant range");
                    return;
                };
                let (Some(a), Some(b)) = (int(lo), int(hi)) else {
                    self.unsupported(s.span, "a `for` loop whose bounds are not constants");
                    return;
                };
                let v = self.state_var(ident(name), "DINT", None);
                let v = self.r(&v);
                let last = if *inclusive { b } else { b - 1 };
                w.line(&format!("FOR {v} := {a} TO {last} DO"));
                w.indent();
                self.stmts(&body.stmts, w);
                w.dedent();
                w.line("END_FOR;");
            }
            StmtKind::Fault { name, .. } => self.fault(name, w),
            _ => {
                self.unsupported(s.span, &format!("the statement `{}`", stmt_name(&s.s)));
            }
        }
    }

    fn fault_code(&mut self, name: &str) -> usize {
        match self.faults.iter().position(|f| f == name) {
            Some(i) => i + 1,
            None => {
                self.faults.push(name.to_string());
                self.faults.len()
            }
        }
    }

    fn fault(&mut self, name: &str, w: &mut Writer) {
        let code = self.fault_code(name);
        let f = self.r("neuron_fault");
        let c = self.r("neuron_fault_code");
        self.has_safety = true;
        w.line(&format!("(* fault {name} *)"));
        w.line(&format!("{f} := TRUE;"));
        w.line(&format!("{c} := {code};"));
        let safe = self.safe.clone();
        self.stmts(&safe, w);
        let handlers = self.on_fault.clone();
        for (which, body) in handlers {
            if which.as_deref().is_none_or(|x| x == name) {
                self.stmts(&body, w);
            }
        }
        w.line("RETURN;");
    }

    // ---- program body ---------------------------------------------------------------

    /// Writes the body of the scan.
    pub fn body(&mut self, unit: &'static str) -> Writer {
        let nir = self.nir;
        let module = &nir.modules[self.m];
        let mut w = Writer::new(unit);
        w.indent();
        // Safety first: rules inside `safety`, reset, then the safe state while faulted.
        let mut resets = Vec::new();
        for item in &module.items {
            if let ItemDef::Safety { members } = &item.def {
                for mem in members {
                    match &mem.def {
                        SafetyDef::Rule { rule } => {
                            let mk = w.mark();
                            self.rule(rule, &mut w);
                            w.map(mk, &self.file.clone(), mem.span, Some("safety".into()));
                        }
                        SafetyDef::Reset { cond } => resets.push((cond.clone(), mem.span)),
                        SafetyDef::Watchdog { value } => {
                            let d =
                                dur_ns(value).map_or_else(|| "the declared time".into(), iec_time);
                            let f = self.file.clone();
                            self.fid.record(
                                &f,
                                mem.span,
                                Class::Modeled,
                                "watchdog",
                                &format!("set the cycle-time monitoring of the PLC to {d}; it is part of the PLC configuration, not of the program"),
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
        if self.has_safety {
            let f = self.r("neuron_fault");
            let c = self.r("neuron_fault_code");
            for (cond, span) in resets {
                let mut pre = Vec::new();
                let cv = self.ex(&cond, &mut pre);
                let mk = w.mark();
                Self::flush(&mut w, pre);
                w.line(&format!("IF {f} AND ({cv}) THEN"));
                w.indent();
                w.line(&format!("{f} := FALSE;"));
                w.line(&format!("{c} := 0;"));
                w.dedent();
                w.line("END_IF;");
                w.map(mk, &self.file.clone(), span, Some("safety".into()));
                self.fid.exact();
            }
            w.line(&format!("IF {f} THEN"));
            w.indent();
            w.line("(* safe state: holds while the fault is active *)");
            let safe = self.safe.clone();
            self.stmts(&safe, &mut w);
            w.line("RETURN;");
            w.dedent();
            w.line("END_IF;");
            self.fid.exact();
        }
        for item in &module.items {
            match &item.def {
                ItemDef::Rung { name, body } => {
                    let mk = w.mark();
                    w.line(&format!("(* rung {name} *)"));
                    self.stmts(&body.stmts, &mut w);
                    w.map(
                        mk,
                        &self.file.clone(),
                        item.span,
                        Some(format!("rung:{name}")),
                    );
                    self.fid.exact();
                }
                ItemDef::Rule { rule } => {
                    let mk = w.mark();
                    self.rule(rule, &mut w);
                    w.map(mk, &self.file.clone(), item.span, Some("rule".into()));
                }
                ItemDef::Sequence { name, items, .. } => {
                    let mk = w.mark();
                    self.sequence(name, items, item.span, &mut w);
                    w.map(
                        mk,
                        &self.file.clone(),
                        item.span,
                        Some(format!("sequence:{name}")),
                    );
                }
                ItemDef::Fsm { fsm } => {
                    let mk = w.mark();
                    self.fsm(fsm, item.span, &mut w);
                    w.map(
                        mk,
                        &self.file.clone(),
                        item.span,
                        Some(format!("fsm:{}", fsm.name)),
                    );
                }
                ItemDef::Task { .. } | ItemDef::Interrupt { .. } => {
                    self.unsupported(
                        item.span,
                        "a task in a `plc` module (use rungs, rules or a sequence)",
                    );
                }
                _ => {}
            }
        }
        let calls: Vec<Block> = self
            .blocks
            .iter()
            .filter(|b| STD_BLOCKS.contains(&b.kind.as_str()))
            .cloned()
            .collect();
        if !calls.is_empty() || !self.end.is_empty() {
            w.line(
                "(* end of scan: function blocks and timers, evaluated as in Neuron (spec 14.3) *)",
            );
        }
        for b in calls {
            let c = self.std_call(&b);
            w.line(&c);
        }
        for l in self.end.clone() {
            w.line(&l);
        }
        w.dedent();
        w
    }

    fn rule(&mut self, rule: &Rule, w: &mut Writer) {
        let n = self.fresh("rule");
        let base = format!("neuron_rule{n}");
        let mut pre = Vec::new();
        let c = self.ex(&rule.cond, &mut pre);
        let cv = self.state_var(format!("{base}_c"), "BOOL", None);
        let cv = self.r(&cv);
        w.line("(* when *)");
        Self::flush(w, pre);
        w.line(&format!("{cv} := {c};"));
        match &rule.hold {
            None => {
                let fire = self.state_var(format!("{base}_fire"), "BOOL", None);
                let prev = self.state_var(format!("{base}_prev"), "BOOL", None);
                let (fire, prev) = (self.r(&fire), self.r(&prev));
                w.line(&format!("{fire} := {cv} AND NOT {prev};"));
                w.line(&format!("{prev} := {cv};"));
                w.line(&format!("IF {fire} THEN"));
                w.indent();
                self.stmts(&rule.body.stmts, w);
                w.dedent();
                w.line("END_IF;");
                self.fid.exact();
            }
            Some(h) => {
                let Some(d) = dur_ns(h) else {
                    self.unsupported(rule.span, "a `when ... for` whose time is not a constant");
                    return;
                };
                let active = self.state_var(format!("{base}_active"), "BOOL", None);
                let fired = self.state_var(format!("{base}_fired"), "BOOL", None);
                let el = self.state_var(format!("{base}_el"), "TIME", None);
                let (active, fired, el) = (self.r(&active), self.r(&fired), self.r(&el));
                w.line(&format!("IF {cv} THEN"));
                w.indent();
                w.line(&format!("IF NOT {active} THEN"));
                w.indent();
                w.line(&format!("{active} := TRUE;"));
                w.line(&format!("{el} := T#0S;"));
                w.dedent();
                w.line("END_IF;");
                w.line(&format!("IF NOT {fired} AND {el} >= {} THEN", iec_time(d)));
                w.indent();
                w.line(&format!("{fired} := TRUE;"));
                self.stmts(&rule.body.stmts, w);
                w.dedent();
                w.line("END_IF;");
                w.dedent();
                w.line("ELSE");
                w.indent();
                w.line(&format!("{active} := FALSE;"));
                w.line(&format!("{fired} := FALSE;"));
                w.dedent();
                w.line("END_IF;");
                let s = iec_time(self.scan_ns);
                self.end
                    .push(format!("IF {active} THEN {el} := {el} + {s}; END_IF;"));
                self.fid.exact();
            }
        }
    }

    fn sequence(&mut self, name: &str, items: &[SeqItem], span: [usize; 2], w: &mut Writer) {
        let mut ops: Vec<Op> = Vec::new();
        if !flatten(items, &mut ops) {
            self.unsupported(
                span,
                &format!("the `parallel` branches of sequence `{name}`"),
            );
            return;
        }
        let n = self.fresh("seq");
        let base = format!("neuron_seq{n}");
        let pc = self.state_var(format!("{base}_pc"), "INT", None);
        let el = self.state_var(format!("{base}_el"), "TIME", None);
        let go = self.state_var(format!("{base}_go"), "BOOL", None);
        let guard = self.state_var(format!("{base}_guard"), "INT", None);
        let (pc, el, go, guard) = (self.r(&pc), self.r(&el), self.r(&go), self.r(&guard));
        let step_index = |t: &str| ops.iter().position(|o| matches!(o, Op::Step(s) if *s == t));
        let is_wait = |o: &Op| matches!(o, Op::Stmt(s) if matches!(s.s, StmtKind::Until { .. } | StmtKind::Wait { .. }));
        let mut labels: BTreeSet<usize> = BTreeSet::from([0]);
        for (k, o) in ops.iter().enumerate() {
            if is_wait(o) {
                labels.insert(k);
                labels.insert(k + 1);
            }
            if let Op::Stmt(s) = o {
                let target = match &s.s {
                    StmtKind::Jump { target } => Some(target.as_str()),
                    StmtKind::Until {
                        on_timeout: Some(t),
                        ..
                    } => Some(t.as_str()),
                    _ => None,
                };
                if let Some(t) = target {
                    if let Some(i) = step_index(t) {
                        labels.insert(i);
                    }
                }
                if contains_wait(s) {
                    self.unsupported(
                        s.span,
                        &format!("a wait inside a compound statement of sequence `{name}`"),
                    );
                    return;
                }
            }
        }
        labels.retain(|&k| k < ops.len());
        let waits: BTreeSet<usize> = ops
            .iter()
            .enumerate()
            .filter(|(_, o)| is_wait(o))
            .map(|(k, _)| k)
            .collect();
        let goto = |k: Option<usize>| -> Vec<String> {
            match k {
                Some(k) if k < ops.len() => {
                    let mut v = Vec::new();
                    if waits.contains(&k) {
                        v.push(format!("{el} := T#0S;"));
                    }
                    v.push(format!("{pc} := {k};"));
                    v.push(format!("{go} := TRUE;"));
                    v
                }
                _ => vec![format!("{pc} := -1;")],
            }
        };
        w.line(&format!("(* sequence {name} *)"));
        w.line(&format!("{go} := TRUE;"));
        w.line(&format!("{guard} := 0;"));
        w.line(&format!("WHILE {go} AND {guard} < 1000 DO"));
        w.indent();
        w.line(&format!("{go} := FALSE;"));
        w.line(&format!("{guard} := {guard} + 1;"));
        w.line(&format!("CASE {pc} OF"));
        w.indent();
        for &l in &labels {
            w.line(&format!("{l}:"));
            w.indent();
            if waits.contains(&l) {
                let Op::Stmt(s) = ops[l] else { continue };
                match &s.s {
                    StmtKind::Until {
                        cond,
                        timeout,
                        on_timeout,
                    } => {
                        let mut pre = Vec::new();
                        let c = self.ex(cond, &mut pre);
                        Self::flush(w, pre);
                        w.line(&format!("IF {c} THEN"));
                        w.indent();
                        for x in goto(Some(l + 1)) {
                            w.line(&x);
                        }
                        w.dedent();
                        if let Some(t) = timeout {
                            let d = dur_ns(t).map_or_else(|| "T#0S".into(), iec_time);
                            w.line(&format!("ELSIF {el} >= {d} THEN"));
                            w.indent();
                            let target = match on_timeout {
                                Some(t) => step_index(t),
                                None => Some(l + 1),
                            };
                            for x in goto(target) {
                                w.line(&x);
                            }
                            w.dedent();
                        }
                        w.line("END_IF;");
                    }
                    StmtKind::Wait { duration } => {
                        let d = dur_ns(duration).map_or_else(|| "T#0S".into(), iec_time);
                        w.line(&format!("IF {el} >= {d} THEN"));
                        w.indent();
                        for x in goto(Some(l + 1)) {
                            w.line(&x);
                        }
                        w.dedent();
                        w.line("END_IF;");
                    }
                    _ => {}
                }
            } else {
                let mut k = l;
                loop {
                    if k >= ops.len() {
                        for x in goto(None) {
                            w.line(&x);
                        }
                        break;
                    }
                    if k != l && labels.contains(&k) {
                        for x in goto(Some(k)) {
                            w.line(&x);
                        }
                        break;
                    }
                    match ops[k] {
                        Op::Step(s) => w.line(&format!("(* step {s} *)")),
                        Op::Stmt(s) => match &s.s {
                            StmtKind::Jump { target } => {
                                for x in goto(step_index(target)) {
                                    w.line(&x);
                                }
                                break;
                            }
                            _ => self.stmt(s, w),
                        },
                    }
                    k += 1;
                }
            }
            w.dedent();
        }
        w.dedent();
        w.line("END_CASE;");
        w.dedent();
        w.line("END_WHILE;");
        let s = iec_time(self.scan_ns);
        self.end.push(format!("{el} := {el} + {s};"));
        let f = self.file.clone();
        self.fid.record(
            &f,
            span,
            Class::Equivalent,
            &format!("sequence `{name}`"),
            "steps advance once per scan instead of once per millisecond; a step may end up to two scans earlier or later than in the reference interpreter",
        );
    }

    fn fsm(&mut self, fsm: &Fsm, span: [usize; 2], w: &mut Writer) {
        let n = self.fresh("fsm");
        let base = format!("neuron_fsm{n}");
        let st = self.state_var(format!("{base}_state"), "INT", Some("-1"));
        let el = self.state_var(format!("{base}_el"), "TIME", None);
        let (st, el) = (self.r(&st), self.r(&el));
        let init = fsm.states.iter().position(|s| s.initial).unwrap_or(0);
        w.line(&format!("(* state machine {} *)", fsm.name));
        w.line(&format!("IF {st} < 0 THEN"));
        w.indent();
        w.line(&format!("{st} := {init};"));
        w.line(&format!("{el} := T#0S;"));
        if let Some(b) = fsm.states.get(init).and_then(|s| s.entry.as_ref()) {
            self.stmts(&b.stmts, w);
        }
        w.dedent();
        w.line("END_IF;");
        w.line(&format!("CASE {st} OF"));
        w.indent();
        for (i, state) in fsm.states.iter().enumerate() {
            w.line(&format!("{i}: (* {} *)", state.name));
            w.indent();
            if let Some(b) = &state.during {
                self.stmts(&b.stmts, w);
            }
            let mut first = true;
            for t in &state.transitions {
                let mut pre = Vec::new();
                let when = t.when.as_ref().map(|c| self.ex(c, &mut pre));
                let after = t.after.as_ref().map(|d| {
                    format!(
                        "{el} >= {}",
                        dur_ns(d).map_or_else(|| "T#0S".into(), iec_time)
                    )
                });
                let cond = match (when, after) {
                    (Some(c), Some(a)) => format!("({c}) OR ({a})"),
                    (Some(c), None) => c,
                    (None, Some(a)) => a,
                    (None, None) => "TRUE".into(),
                };
                if !pre.is_empty() {
                    if first {
                        Self::flush(w, pre);
                    } else {
                        self.unsupported(t.span, "a block call in the guard of a later transition");
                    }
                }
                w.line(&format!(
                    "{} {cond} THEN",
                    if first { "IF" } else { "ELSIF" }
                ));
                first = false;
                w.indent();
                if let Some(b) = &state.exit {
                    self.stmts(&b.stmts, w);
                }
                if let Some(b) = &t.action {
                    self.stmts(&b.stmts, w);
                }
                match fsm.states.iter().position(|s| s.name == t.target) {
                    Some(ti) => {
                        w.line(&format!("{st} := {ti};"));
                        w.line(&format!("{el} := T#0S;"));
                        if let Some(b) = &fsm.states[ti].entry {
                            self.stmts(&b.stmts, w);
                        }
                    }
                    None => {
                        self.unsupported(
                            t.span,
                            &format!("the transition to the unknown state `{}`", t.target),
                        );
                    }
                }
                w.dedent();
            }
            if !first {
                w.line("END_IF;");
            }
            w.dedent();
        }
        w.dedent();
        w.line("END_CASE;");
        let s = iec_time(self.scan_ns);
        self.end.push(format!("{el} := {el} + {s};"));
        let f = self.file.clone();
        let period = fsm.trigger.as_ref().and_then(|t| match t {
            neuron_nir::Trigger::Every { period, .. } => dur_ns(period),
            neuron_nir::Trigger::On { .. } => None,
        });
        match period {
            Some(p) if p != self.scan_ns => self.fid.record(
                &f,
                span,
                Class::Equivalent,
                &format!("state machine `{}`", fsm.name),
                &format!(
                    "evaluated once per scan ({}) instead of every {}",
                    iec_time(self.scan_ns),
                    iec_time(p)
                ),
            ),
            _ => self.fid.exact(),
        }
    }

    /// Declaration line of a variable.
    pub fn decl(&self, v: &PVar) -> String {
        let at = match (&v.at, self.style) {
            (Some(a), Style::Iec) => format!(" AT {a}"),
            _ => String::new(),
        };
        let init = v
            .init
            .as_ref()
            .map_or(String::new(), |i| format!(" := {i}"));
        format!("{}{at} : {}{init};", v.name, v.ty)
    }

    /// Declaration of a block instance.
    pub fn block_decl(&self, b: &Block) -> String {
        format!("{} : {};", b.name, self.std_type(&b.kind))
    }

    /// Declarations of the generated state, sorted for a stable output.
    pub fn state_decls(&self) -> Vec<String> {
        let mut v: Vec<String> = self.state.iter().map(|p| self.decl(p)).collect();
        if self.has_safety {
            v.push("neuron_fault : BOOL;".into());
            v.push("neuron_fault_code : INT;".into());
        }
        v
    }
}

fn helper_name(kind: &str) -> &'static str {
    match kind {
        "hysteresis" => "NEURON_HYSTERESIS",
        "debounce" => "NEURON_DEBOUNCE",
        "ramp" => "NEURON_RAMP",
        "lowpass" => "NEURON_LOWPASS",
        "highpass" => "NEURON_HIGHPASS",
        "moving_average" => "NEURON_MOVING_AVERAGE",
        _ => "NEURON_PID",
    }
}

fn real(v: &str) -> String {
    if v.contains('.') || v.contains('E') || v.contains('e') {
        v.to_string()
    } else {
        format!("{v}.0")
    }
}

/// One element of a flattened sequence: a step marker or a statement.
pub(crate) fn flatten_seq(items: &[SeqItem]) -> Option<Vec<(Option<&str>, Option<&Stmt>)>> {
    let mut ops = Vec::new();
    if !flatten(items, &mut ops) {
        return None;
    }
    Some(
        ops.into_iter()
            .map(|o| match o {
                Op::Step(s) => (Some(s), None),
                Op::Stmt(s) => (None, Some(s)),
            })
            .collect(),
    )
}

fn flatten<'b>(items: &'b [SeqItem], out: &mut Vec<Op<'b>>) -> bool {
    for it in items {
        match it {
            SeqItem::Step { name, body, .. } => {
                out.push(Op::Step(name));
                if !flatten(body, out) {
                    return false;
                }
            }
            SeqItem::Parallel { .. } => return false,
            SeqItem::Stmt { stmt } => out.push(Op::Stmt(stmt)),
        }
    }
    true
}

pub(crate) fn contains_wait(s: &Stmt) -> bool {
    let block = |b: &neuron_nir::Block| {
        b.stmts.iter().any(|x| {
            matches!(
                x.s,
                StmtKind::Until { .. } | StmtKind::Wait { .. } | StmtKind::Jump { .. }
            ) || contains_wait(x)
        })
    };
    match &s.s {
        StmtKind::If {
            branches,
            otherwise,
        } => branches.iter().any(|b| block(&b.body)) || otherwise.as_ref().is_some_and(block),
        StmtKind::For { body, .. } | StmtKind::While { body, .. } | StmtKind::Loop { body, .. } => {
            block(body)
        }
        _ => false,
    }
}

pub(crate) fn stmt_name(s: &StmtKind) -> &'static str {
    match s {
        StmtKind::Let { .. } => "let",
        StmtKind::Assign { .. } => "assignment",
        StmtKind::Expr { .. } => "expression",
        StmtKind::Pass => "pass",
        StmtKind::Return { .. } => "return",
        StmtKind::Break => "break",
        StmtKind::Continue => "continue",
        StmtKind::Fault { .. } => "fault",
        StmtKind::Assert { .. } => "assert",
        StmtKind::Emit { .. } => "emit",
        StmtKind::Coil { .. } => "set/reset",
        StmtKind::Wait { .. } => "wait",
        StmtKind::Until { .. } => "until",
        StmtKind::Jump { .. } => "->",
        StmtKind::Move { .. } => "move",
        StmtKind::If { .. } => "if",
        StmtKind::Match { .. } => "match",
        StmtKind::For { .. } => "for",
        StmtKind::While { .. } => "while",
        StmtKind::Loop { .. } => "loop",
        StmtKind::Atomic { .. } => "atomic",
        StmtKind::Rule { .. } => "when",
        StmtKind::Asm { .. } => "asm",
    }
}
