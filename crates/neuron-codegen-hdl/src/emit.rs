//! NIR `hdl` modules to Verilog-2005.

use crate::HdlDiag;
use neuron_nir as n;
use std::collections::{BTreeMap, BTreeSet};

/// The Verilog of a package.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Verilog {
    /// One file with every module.
    pub text: String,
    /// Module names in order (the last circuit of the last module is the usual top).
    pub modules: Vec<String>,
    /// Ports of each module: `(module, [(name, dir, width, pin locations)])`.
    pub ports: Vec<(String, Vec<PortInfo>)>,
    /// Source map: 1-based Verilog line, source file and byte range of the statement.
    pub map: Vec<MapEntry>,
}

/// A port as emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    /// Verilog name.
    pub name: String,
    /// `input`, `output` or `inout`.
    pub dir: String,
    /// Width in bits.
    pub width: u32,
    /// Package pins from `@ pin(...)`, bit 0 first. Empty when the port has none.
    pub pins: Vec<String>,
    /// Frequency in hertz when the port is a clock.
    pub clock_hz: Option<u64>,
}

/// One entry of the source map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEntry {
    /// 1-based line in [`Verilog::text`].
    pub line: usize,
    /// Source file.
    pub file: String,
    /// Byte range in the source file.
    pub span: [usize; 2],
}

const KEYWORDS: &[&str] = &[
    "always",
    "and",
    "assign",
    "begin",
    "bit",
    "buf",
    "byte",
    "case",
    "casex",
    "casez",
    "cell",
    "config",
    "deassign",
    "default",
    "defparam",
    "design",
    "disable",
    "edge",
    "else",
    "end",
    "endcase",
    "endconfig",
    "endfunction",
    "endgenerate",
    "endmodule",
    "endprimitive",
    "endspecify",
    "endtable",
    "endtask",
    "event",
    "for",
    "force",
    "forever",
    "fork",
    "function",
    "generate",
    "genvar",
    "if",
    "initial",
    "inout",
    "input",
    "int",
    "integer",
    "join",
    "library",
    "localparam",
    "logic",
    "macromodule",
    "module",
    "nand",
    "negedge",
    "nor",
    "not",
    "or",
    "output",
    "parameter",
    "posedge",
    "primitive",
    "real",
    "reg",
    "release",
    "repeat",
    "signed",
    "specify",
    "table",
    "task",
    "time",
    "tri",
    "unsigned",
    "wait",
    "wand",
    "while",
    "wire",
    "wor",
    "xnor",
    "xor",
];

/// A Verilog-safe name.
pub(crate) fn vname(name: &str) -> String {
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
    let clean = if clean.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("n_{clean}")
    } else {
        clean
    };
    if KEYWORDS.contains(&clean.as_str()) {
        format!("{clean}_n")
    } else {
        clean
    }
}

/// Width and signedness of a written type (`bit<8>`, `sbit<4>`, `bit`, `bool`).
fn type_width(t: &n::TypeExpr) -> Option<(u32, bool)> {
    let n::TypeExprKind::Named { path, args } = &t.kind else {
        return None;
    };
    let signed = path == "sbit";
    match path.as_str() {
        "bool" => Some((1, false)),
        "bit" | "sbit" => match args.first() {
            None => Some((1, signed)),
            Some(n::TypeArg::Expr { expr }) => match &expr.e {
                n::ExprKind::Lit {
                    lit: n::Lit::Int { v, .. },
                } => v
                    .parse::<u32>()
                    .ok()
                    .filter(|w| (1..=64).contains(w))
                    .map(|w| (w, signed)),
                _ => None,
            },
            Some(_) => None,
        },
        _ => None,
    }
}

/// Width of an expression from its resolved type.
fn expr_width(e: &n::Expr) -> u32 {
    match &e.ty {
        n::Type::Bits { width, .. } => *width,
        _ => 1,
    }
}

fn range(width: u32) -> String {
    if width > 1 {
        format!("[{}:0] ", width - 1)
    } else {
        String::new()
    }
}

/// The names of a `pin(A, B)` location.
fn pin_names(e: &n::Expr) -> Vec<String> {
    match &e.e {
        n::ExprKind::Call { args, .. } => args.iter().filter_map(|a| flat(&a.value)).collect(),
        _ => flat(e).into_iter().collect(),
    }
}

fn flat(e: &n::Expr) -> Option<String> {
    match &e.e {
        n::ExprKind::Global { name }
        | n::ExprKind::Ref { name, .. }
        | n::ExprKind::Local { name, .. } => Some(name.clone()),
        n::ExprKind::Lit {
            lit: n::Lit::Int { text, .. },
        } => Some(text.clone()),
        n::ExprKind::Lit {
            lit: n::Lit::Str { text },
        } => Some(text.trim_matches('"').to_string()),
        n::ExprKind::Field { base, name, .. } => Some(format!("{}.{name}", flat(base)?)),
        _ => None,
    }
}

/// Value of a sized literal such as `10'b11_1111_1111`.
fn bits_value(text: &str) -> Option<f64> {
    let t = text.replace('_', "");
    let (_, rest) = t.split_once('\'')?;
    let rest = rest.trim_start_matches(['s', 'S']);
    let (base, digits) = rest.split_at(1);
    let radix = match base {
        "b" | "B" => 2,
        "o" | "O" => 8,
        "d" | "D" => 10,
        "h" | "H" => 16,
        _ => return None,
    };
    u64::from_str_radix(digits, radix).ok().map(|v| v as f64)
}

#[derive(Default)]
struct Out {
    lines: Vec<String>,
    map: Vec<MapEntry>,
}

impl Out {
    fn line(&mut self, indent: usize, text: impl AsRef<str>) {
        self.lines
            .push(format!("{}{}", "  ".repeat(indent), text.as_ref()));
    }
}

struct ClockInfo {
    hz: u64,
    pins: Vec<String>,
}

struct Ctx<'a> {
    module: &'a n::Module,
    clocks: BTreeMap<String, ClockInfo>,
    consts: BTreeMap<String, f64>,
    diags: Vec<HdlDiag>,
}

impl Ctx<'_> {
    fn unsupported(&mut self, what: &str, span: [usize; 2]) {
        self.diags.push(HdlDiag {
            code: "NE0610",
            message: format!("{what} is not supported by the Verilog backend yet"),
            help: Some("the supported subset is listed in docs/hdl.md".into()),
            file: self.module.file.clone(),
            span,
        });
    }

    fn eval(&self, e: &n::Expr) -> Option<f64> {
        match &e.e {
            n::ExprKind::Lit { lit } => match lit {
                n::Lit::Int { v, .. } | n::Lit::Real { v, .. } => v.parse().ok(),
                n::Lit::Bits { text } => bits_value(text),
                n::Lit::Bool { v } => Some(f64::from(u8::from(*v))),
                _ => None,
            },
            n::ExprKind::Quantity { si, value, .. } => si
                .as_ref()
                .and_then(|s| s.parse().ok())
                .or_else(|| self.eval(value)),
            n::ExprKind::Local { name, .. }
            | n::ExprKind::Ref { name, .. }
            | n::ExprKind::Global { name } => self.consts.get(name).copied(),
            n::ExprKind::Field { base, name, .. } if name == "freq" => flat(base)
                .and_then(|c| self.clocks.get(&c))
                .map(|c| c.hz as f64),
            n::ExprKind::Unary { op, operand } if op == "-" => self.eval(operand).map(|v| -v),
            n::ExprKind::Binary { op, lhs, rhs } => {
                let (a, b) = (self.eval(lhs)?, self.eval(rhs)?);
                match op.as_str() {
                    "+" | "+%" => Some(a + b),
                    "-" | "-%" => Some(a - b),
                    "*" | "*%" => Some(a * b),
                    "/" if b != 0.0 => Some(a / b),
                    "%" if b != 0.0 => Some(a % b),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// An expression in Verilog. Unsupported parts are reported and replaced by `0`.
    fn ex(&mut self, e: &n::Expr) -> String {
        match &e.e {
            n::ExprKind::Lit { lit } => match lit {
                n::Lit::Int { v, .. } => v.clone(),
                n::Lit::Bits { text } => text.clone(),
                n::Lit::Bool { v } => if *v { "1'b1" } else { "1'b0" }.to_string(),
                _ => {
                    self.unsupported("this literal", e.span);
                    "0".into()
                }
            },
            n::ExprKind::Quantity { .. } => match self.eval(e) {
                Some(v) => format!("{}", v.round() as i64),
                None => {
                    self.unsupported("a quantity that is not a constant", e.span);
                    "0".into()
                }
            },
            n::ExprKind::Local { name, .. }
            | n::ExprKind::Ref { name, .. }
            | n::ExprKind::Global { name } => vname(name),
            n::ExprKind::Field { name, .. } if name == "freq" => match self.eval(e) {
                Some(v) => format!("{}", v.round() as i64),
                None => {
                    self.unsupported("this field", e.span);
                    "0".into()
                }
            },
            n::ExprKind::Unary { op, operand } => {
                let x = self.ex(operand);
                match op.as_str() {
                    "-" => format!("(-{x})"),
                    "~" => format!("(~{x})"),
                    "not" => {
                        if matches!(operand.ty, n::Type::Bool) {
                            format!("(!{x})")
                        } else {
                            format!("(~{x})")
                        }
                    }
                    other => {
                        self.unsupported(&format!("the operator `{other}`"), e.span);
                        "0".into()
                    }
                }
            }
            n::ExprKind::Binary { op, lhs, rhs } => {
                if op == "++" {
                    let mut parts = Vec::new();
                    self.concat(e, &mut parts);
                    return format!("{{{}}}", parts.join(", "));
                }
                let wide = expr_width(lhs) > 1 || expr_width(rhs) > 1;
                let v = match op.as_str() {
                    "+" | "+%" => "+",
                    "-" | "-%" => "-",
                    "*" | "*%" => "*",
                    "/" => "/",
                    "%" => "%",
                    "&" => "&",
                    "|" => "|",
                    "^" | "xor" => "^",
                    "<<" => "<<",
                    ">>" => ">>",
                    "==" => "==",
                    "!=" => "!=",
                    "<" => "<",
                    "<=" => "<=",
                    ">" => ">",
                    ">=" => ">=",
                    "and" => {
                        if wide {
                            "&"
                        } else {
                            "&&"
                        }
                    }
                    "or" => {
                        if wide {
                            "|"
                        } else {
                            "||"
                        }
                    }
                    other => {
                        self.unsupported(&format!("the operator `{other}`"), e.span);
                        return "0".into();
                    }
                };
                let (a, b) = (self.ex(lhs), self.ex(rhs));
                format!("({a} {v} {b})")
            }
            n::ExprKind::Compare { first, rest } => {
                let mut prev = self.ex(first);
                let mut terms = Vec::new();
                for r in rest {
                    let rhs = self.ex(&r.rhs);
                    terms.push(format!("({prev} {} {rhs})", r.op));
                    prev = rhs;
                }
                if terms.len() == 1 {
                    terms.remove(0)
                } else {
                    format!("({})", terms.join(" && "))
                }
            }
            n::ExprKind::Index { base, index, end } => {
                let b = self.ex(base);
                let i = self.index(index);
                match end {
                    Some(lo) => {
                        let l = self.index(lo);
                        format!("{b}[{i}:{l}]")
                    }
                    None => format!("{b}[{i}]"),
                }
            }
            n::ExprKind::IfElse {
                cond,
                then,
                otherwise,
            } => {
                let (c, a, b) = (self.ex(cond), self.ex(then), self.ex(otherwise));
                format!("({c} ? {a} : {b})")
            }
            n::ExprKind::Cast { operand, .. } => self.ex(operand),
            _ => {
                self.unsupported("this expression", e.span);
                "0".into()
            }
        }
    }

    fn index(&mut self, e: &n::Expr) -> String {
        match self.eval(e) {
            Some(v) => format!("{}", v as i64),
            None => self.ex(e),
        }
    }

    fn concat(&mut self, e: &n::Expr, out: &mut Vec<String>) {
        if let n::ExprKind::Binary { op, lhs, rhs } = &e.e {
            if op == "++" {
                self.concat(lhs, out);
                self.concat(rhs, out);
                return;
            }
        }
        let s = self.ex(e);
        out.push(s);
    }

    fn block(&mut self, b: &n::Block, indent: usize, clocked: bool, out: &mut Out) {
        if b.stmts.is_empty() {
            return;
        }
        for s in &b.stmts {
            self.stmt(s, indent, clocked, out);
        }
    }

    fn stmt(&mut self, s: &n::Stmt, indent: usize, clocked: bool, out: &mut Out) {
        out.map.push(MapEntry {
            line: out.lines.len() + 1,
            file: self.module.file.clone(),
            span: s.span,
        });
        let assign = if clocked { "<=" } else { "=" };
        match &s.s {
            n::StmtKind::Pass => {}
            n::StmtKind::Assign {
                place,
                op,
                value,
                when,
            } => {
                let lhs = self.ex(place);
                let v = self.ex(value);
                let rhs = match op.as_str() {
                    "=" => v,
                    other => match other.strip_suffix('=') {
                        Some(bop)
                            if ["+", "-", "*", "&", "|", "^", "+%", "-%", "*%", "<<", ">>"]
                                .contains(&bop) =>
                        {
                            format!("({lhs} {} {v})", bop.trim_end_matches('%'))
                        }
                        _ => {
                            self.unsupported(&format!("the assignment `{other}`"), s.span);
                            v
                        }
                    },
                };
                match when {
                    Some(c) => {
                        let c = self.ex(c);
                        out.line(indent, format!("if ({c}) {lhs} {assign} {rhs};"));
                    }
                    None => out.line(indent, format!("{lhs} {assign} {rhs};")),
                }
            }
            n::StmtKind::Coil { set, place, when } => {
                let (lhs, c) = (self.ex(place), self.ex(when));
                out.line(
                    indent,
                    format!("if ({c}) {lhs} {assign} 1'b{};", u8::from(*set)),
                );
            }
            n::StmtKind::If {
                branches,
                otherwise,
            } => {
                for (i, br) in branches.iter().enumerate() {
                    let c = self.ex(&br.cond);
                    let head = if i == 0 {
                        format!("if ({c}) begin")
                    } else {
                        format!("end else if ({c}) begin")
                    };
                    out.line(indent, head);
                    self.block(&br.body, indent + 1, clocked, out);
                }
                if let Some(o) = otherwise {
                    out.line(indent, "end else begin");
                    self.block(o, indent + 1, clocked, out);
                }
                out.line(indent, "end");
            }
            _ => self.unsupported("this statement", s.span),
        }
    }
}

/// Names written by the statements of a block.
fn written(b: &n::Block, out: &mut BTreeSet<String>) {
    for s in &b.stmts {
        match &s.s {
            n::StmtKind::Assign { place, .. } | n::StmtKind::Coil { place, .. } => {
                let mut p = place;
                while let n::ExprKind::Index { base, .. } = &p.e {
                    p = base;
                }
                if let Some(name) = flat(p) {
                    out.insert(name);
                }
            }
            n::StmtKind::If {
                branches,
                otherwise,
            } => {
                for br in branches {
                    written(&br.body, out);
                }
                if let Some(o) = otherwise {
                    written(o, out);
                }
            }
            _ => {}
        }
    }
}

/// Names read by an expression.
fn reads_expr(e: &n::Expr, out: &mut BTreeSet<String>) {
    match &e.e {
        n::ExprKind::Local { name, .. } | n::ExprKind::Ref { name, .. } => {
            out.insert(name.clone());
        }
        n::ExprKind::Binary { lhs, rhs, .. } => {
            reads_expr(lhs, out);
            reads_expr(rhs, out);
        }
        n::ExprKind::Compare { first, rest } => {
            reads_expr(first, out);
            for r in rest {
                reads_expr(&r.rhs, out);
            }
        }
        n::ExprKind::Unary { operand, .. } | n::ExprKind::Cast { operand, .. } => {
            reads_expr(operand, out)
        }
        n::ExprKind::Index { base, index, end } => {
            reads_expr(base, out);
            reads_expr(index, out);
            if let Some(x) = end {
                reads_expr(x, out);
            }
        }
        n::ExprKind::IfElse {
            cond,
            then,
            otherwise,
        } => {
            reads_expr(cond, out);
            reads_expr(then, out);
            reads_expr(otherwise, out);
        }
        _ => {}
    }
}

fn reads(b: &n::Block, out: &mut BTreeSet<String>) {
    for s in &b.stmts {
        match &s.s {
            n::StmtKind::Assign {
                value, when, place, ..
            } => {
                reads_expr(value, out);
                if let Some(w) = when {
                    reads_expr(w, out);
                }
                if let n::ExprKind::Index { index, end, .. } = &place.e {
                    reads_expr(index, out);
                    if let Some(x) = end {
                        reads_expr(x, out);
                    }
                }
            }
            n::StmtKind::Coil { when, .. } => reads_expr(when, out),
            n::StmtKind::If {
                branches,
                otherwise,
            } => {
                for br in branches {
                    reads_expr(&br.cond, out);
                    reads(&br.body, out);
                }
                if let Some(o) = otherwise {
                    reads(o, out);
                }
            }
            _ => {}
        }
    }
}

struct SyncItem {
    from: String,
    to: String,
    via: String,
    span: [usize; 2],
}

/// Translates the `hdl` modules of a package.
pub fn emit_verilog(nir: &n::Nir) -> Result<Verilog, Vec<HdlDiag>> {
    let mut out = Out::default();
    let mut v = Verilog::default();
    let mut diags = Vec::new();
    let mut circuits: BTreeMap<String, Vec<String>> = BTreeMap::new();
    out.line(
        0,
        "// Generated by neuron-codegen-hdl (task B7). Do not edit: change the .neuron source.",
    );
    out.line(0, "`default_nettype none");
    for m in nir.modules.iter().filter(|m| m.dialect == "hdl") {
        let mut ctx = Ctx {
            module: m,
            clocks: BTreeMap::new(),
            consts: BTreeMap::new(),
            diags: Vec::new(),
        };
        let mut syncs = Vec::new();
        for item in &m.items {
            match &item.def {
                n::ItemDef::Clock { name, freq, pin } => {
                    let hz = ctx.eval(freq).map_or(0, |f| f.round() as u64);
                    let pins = pin.as_ref().map(pin_names).unwrap_or_default();
                    ctx.clocks.insert(name.clone(), ClockInfo { hz, pins });
                }
                n::ItemDef::Sync { from, to, via } => syncs.push(SyncItem {
                    from: flat(from).unwrap_or_default(),
                    to: flat(to).unwrap_or_default(),
                    via: flat(via).unwrap_or_default(),
                    span: item.span,
                }),
                _ => {}
            }
        }
        for item in &m.items {
            match &item.def {
                n::ItemDef::Clock { .. } | n::ItemDef::Sync { .. } => {}
                n::ItemDef::Circuit { circuit } => {
                    let ports =
                        circuit_module(&mut ctx, circuit, &syncs, &circuits, &mut out, item.span);
                    circuits.insert(
                        circuit.name.clone(),
                        ports.iter().map(|p| p.name.clone()).collect(),
                    );
                    v.modules.push(vname(&circuit.name));
                    v.ports.push((vname(&circuit.name), ports));
                }
                _ => ctx.unsupported("this item in an `hdl` module", item.span),
            }
        }
        for s in &syncs {
            if s.via != "two_ff" {
                ctx.unsupported(
                    &format!("`sync ... via {}` (only `two_ff` is emitted)", s.via),
                    s.span,
                );
            }
        }
        diags.extend(ctx.diags);
    }
    out.line(0, "`default_nettype wire");
    if !diags.is_empty() {
        return Err(diags);
    }
    let mut text = out.lines.join("\n");
    text.push('\n');
    v.text = text;
    v.map = out.map;
    Ok(v)
}

/// Emits one circuit as a module and returns its ports.
fn circuit_module(
    ctx: &mut Ctx<'_>,
    c: &n::Circuit,
    syncs: &[SyncItem],
    known: &BTreeMap<String, Vec<String>>,
    out: &mut Out,
    span: [usize; 2],
) -> Vec<PortInfo> {
    // What the members declare and do.
    let mut consts: Vec<(String, &n::Expr)> = Vec::new();
    let mut regs: Vec<(String, u32, bool, Option<&n::Expr>)> = Vec::new();
    let mut wires: Vec<(String, u32, bool)> = Vec::new();
    let mut edges: Vec<(bool, String, &n::Block, [usize; 2])> = Vec::new();
    let mut combs: Vec<(&n::Block, [usize; 2])> = Vec::new();
    let mut instances: Vec<(String, &n::Expr, [usize; 2])> = Vec::new();
    for mem in &c.members {
        match &mem.def {
            n::CircuitDef::Const { name, value, .. } => {
                if let Some(v) = ctx.eval(value) {
                    ctx.consts.insert(name.clone(), v);
                }
                consts.push((name.clone(), value));
            }
            n::CircuitDef::Reg { name, ty, init, .. } => match type_width(ty) {
                Some((w, s)) => regs.push((name.clone(), w, s, init.as_ref())),
                None => ctx.unsupported(&format!("the type of register `{name}`"), mem.span),
            },
            n::CircuitDef::Wire { name, ty, .. } => match type_width(ty) {
                Some((w, s)) => wires.push((name.clone(), w, s)),
                None => ctx.unsupported(&format!("the type of wire `{name}`"), mem.span),
            },
            n::CircuitDef::OnEdge {
                rising,
                clock,
                body,
            } => edges.push((*rising, clock.clone(), body, mem.span)),
            n::CircuitDef::Comb { body } => combs.push((body, mem.span)),
            n::CircuitDef::Instance { name, of, .. } => {
                instances.push((name.clone(), of, mem.span))
            }
            n::CircuitDef::Fsm { .. } => {
                ctx.unsupported("a state machine inside a circuit", mem.span)
            }
        }
    }

    // Clock domains: the domain of a register is the clock of the block that writes it.
    let mut domain: BTreeMap<String, String> = BTreeMap::new();
    for (_, clk, body, _) in &edges {
        let mut w = BTreeSet::new();
        written(body, &mut w);
        for name in w {
            domain.insert(name, clk.clone());
        }
    }
    let synced: BTreeMap<&str, &SyncItem> = syncs.iter().map(|s| (s.to.as_str(), s)).collect();
    // A synchronized value belongs to the domain of the blocks that read it.
    let mut sync_domain: BTreeMap<String, String> = BTreeMap::new();
    for (_, clk, body, span) in &edges {
        let mut r = BTreeSet::new();
        reads(body, &mut r);
        for name in &r {
            if synced.contains_key(name.as_str()) {
                if let Some(prev) = sync_domain.insert(name.clone(), clk.clone()) {
                    if prev != *clk {
                        ctx.diags.push(HdlDiag {
                            code: "NE0702",
                            message: format!("`{name}` is synchronized once but read in the domains `{prev}` and `{clk}`"),
                            help: Some("write one `sync` per destination domain".into()),
                            file: ctx.module.file.clone(),
                            span: *span,
                        });
                    }
                }
                continue;
            }
            if let Some(src) = domain.get(name) {
                if src != clk {
                    ctx.diags.push(HdlDiag {
                        code: "NE0702",
                        message: format!("`{name}` belongs to the clock domain `{src}` and is read in the domain `{clk}` without a synchronizer"),
                        help: Some(format!("add `sync {name} -> {name}_{clk} via two_ff` and read `{name}_{clk}`")),
                        file: ctx.module.file.clone(),
                        span: *span,
                    });
                }
            }
        }
    }
    // Everything that is assigned procedurally is a `reg` in Verilog.
    let mut assigned = BTreeSet::new();
    for (_, _, body, _) in &edges {
        written(body, &mut assigned);
    }
    for (body, _) in &combs {
        written(body, &mut assigned);
    }

    // Ports: the clocks first, then the declared ports.
    let mut clocks: Vec<String> = Vec::new();
    if let Some(clk) = &c.clock {
        clocks.push(clk.clone());
    }
    for (_, clk, _, _) in &edges {
        if !clocks.contains(clk) {
            clocks.push(clk.clone());
        }
    }
    for clk in sync_domain.values() {
        if !clocks.contains(clk) {
            clocks.push(clk.clone());
        }
    }
    let mut ports: Vec<PortInfo> = Vec::new();
    let mut decls: Vec<String> = Vec::new();
    for clk in &clocks {
        let info = ctx.clocks.get(clk);
        ports.push(PortInfo {
            name: vname(clk),
            dir: "input".into(),
            width: 1,
            pins: info.map(|i| i.pins.clone()).unwrap_or_default(),
            clock_hz: info.map(|i| i.hz),
        });
        decls.push(format!("input  wire {}", vname(clk)));
        if info.is_none() {
            ctx.unsupported(
                &format!("the clock `{clk}`, which no `clock` item declares"),
                span,
            );
        }
    }
    let mut reset: Option<String> = None;
    for p in &c.ports {
        let Some((w, signed)) = type_width(&p.ty) else {
            ctx.unsupported(&format!("the type of port `{}`", p.name), p.span);
            continue;
        };
        let kind = match p.dir.as_str() {
            "in" => "input  wire",
            "out" if assigned.contains(&p.name) => "output reg ",
            "out" => "output wire",
            "inout" => "inout  wire",
            other => {
                ctx.unsupported(&format!("the port direction `{other}`"), p.span);
                continue;
            }
        };
        if p.dir == "in" && w == 1 && matches!(p.name.as_str(), "rst" | "reset") {
            reset = Some(vname(&p.name));
        }
        let sign = if signed { "signed " } else { "" };
        decls.push(format!("{kind} {sign}{}{}", range(w), vname(&p.name)));
        ports.push(PortInfo {
            name: vname(&p.name),
            dir: match p.dir.as_str() {
                "in" => "input",
                "out" => "output",
                _ => "inout",
            }
            .into(),
            width: w,
            pins: p.pin.as_ref().map(pin_names).unwrap_or_default(),
            clock_hz: None,
        });
    }

    let mname = vname(&c.name);
    out.line(0, "");
    out.line(0, format!("// circuit `{}` of {}", c.name, ctx.module.file));
    out.line(0, format!("module {mname} ("));
    let n_decl = decls.len();
    for (i, d) in decls.iter().enumerate() {
        out.line(1, format!("{d}{}", if i + 1 < n_decl { "," } else { "" }));
    }
    out.line(0, ");");
    for (name, value) in &consts {
        match ctx.consts.get(name).copied() {
            Some(v) => out.line(
                1,
                format!("localparam integer {} = {};", vname(name), v.round() as i64),
            ),
            None => {
                let e = ctx.ex(value);
                out.line(1, format!("localparam {} = {e};", vname(name)));
            }
        }
    }
    let mut inits: BTreeMap<String, String> = BTreeMap::new();
    for (name, w, signed, init) in &regs {
        let sign = if *signed { "signed " } else { "" };
        let init_text = init.map(|e| match ctx.eval(e) {
            Some(v) => format!("{}'d{}", w, v.round() as i64),
            None => ctx.ex(e),
        });
        match &init_text {
            Some(t) => {
                out.line(1, format!("reg {sign}{}{} = {t};", range(*w), vname(name)));
                inits.insert(name.clone(), t.clone());
            }
            None => out.line(1, format!("reg {sign}{}{};", range(*w), vname(name))),
        }
    }
    for (name, w, signed) in &wires {
        let sign = if *signed { "signed " } else { "" };
        let kind = if assigned.contains(name) || synced.contains_key(name.as_str()) {
            "reg "
        } else {
            "wire"
        };
        out.line(1, format!("{kind} {sign}{}{};", range(*w), vname(name)));
    }

    // Two-flop synchronizers.
    for (to, clk) in &sync_domain {
        let Some(s) = synced.get(to.as_str()) else {
            continue;
        };
        if s.via != "two_ff" {
            continue;
        }
        let w = wires
            .iter()
            .find(|(n, _, _)| n == to)
            .map(|(_, w, _)| *w)
            .or_else(|| {
                regs.iter()
                    .find(|(n, _, _, _)| n == &s.from)
                    .map(|(_, w, _, _)| *w)
            })
            .unwrap_or(1);
        let (a, b, stage) = (vname(&s.from), vname(to), format!("{}_meta", vname(to)));
        out.line(
            1,
            format!(
                "// two-flop synchronizer: `{}` into the domain `{clk}`",
                s.from
            ),
        );
        out.line(
            1,
            format!("(* ASYNC_REG = \"TRUE\" *) reg {}{stage} = 0;", range(w)),
        );
        if !wires.iter().any(|(n, _, _)| n == to) {
            out.line(
                1,
                format!("(* ASYNC_REG = \"TRUE\" *) reg {}{b} = 0;", range(w)),
            );
        }
        out.line(1, format!("always @(posedge {}) begin", vname(clk)));
        out.line(2, format!("{stage} <= {a};"));
        out.line(2, format!("{b} <= {stage};"));
        out.line(1, "end");
    }

    for (name, of, ispan) in &instances {
        let n::ExprKind::Call { callee, args, .. } = &of.e else {
            ctx.unsupported("this instance", *ispan);
            continue;
        };
        let Some(target) = flat(callee) else {
            ctx.unsupported("this instance", *ispan);
            continue;
        };
        let Some(target_ports) = known.get(&target).cloned() else {
            ctx.unsupported(
                &format!("an instance of `{target}`, which is not a circuit defined before it"),
                *ispan,
            );
            continue;
        };
        let mut conns: Vec<String> = Vec::new();
        let (clock_ports, data_ports): (Vec<String>, Vec<String>) = target_ports
            .into_iter()
            .partition(|p| ctx.clocks.contains_key(p) || clocks.contains(p));
        let mut positional = data_ports.into_iter();
        for clk in &clock_ports {
            conns.push(format!(".{clk}({clk})"));
        }
        for a in args {
            let port = match &a.name {
                Some(p) => vname(p),
                None => match positional.next() {
                    Some(p) => p,
                    None => {
                        ctx.unsupported("too many arguments for this instance", *ispan);
                        break;
                    }
                },
            };
            let val = ctx.ex(&a.value);
            conns.push(format!(".{port}({val})"));
        }
        out.line(
            1,
            format!("{} {} ({});", vname(&target), vname(name), conns.join(", ")),
        );
    }

    for (rising, clk, body, espan) in &edges {
        out.map.push(MapEntry {
            line: out.lines.len() + 1,
            file: ctx.module.file.clone(),
            span: *espan,
        });
        let edge = if *rising { "posedge" } else { "negedge" };
        out.line(1, format!("always @({edge} {}) begin", vname(clk)));
        match &reset {
            Some(rst) => {
                let mut w = BTreeSet::new();
                written(body, &mut w);
                out.line(2, format!("if ({rst}) begin"));
                for name in &w {
                    if let Some(init) = inits.get(name) {
                        out.line(3, format!("{} <= {init};", vname(name)));
                    }
                }
                out.line(2, "end else begin");
                ctx.block(body, 3, true, out);
                out.line(2, "end");
            }
            None => ctx.block(body, 2, true, out),
        }
        out.line(1, "end");
    }
    for (body, cspan) in &combs {
        out.map.push(MapEntry {
            line: out.lines.len() + 1,
            file: ctx.module.file.clone(),
            span: *cspan,
        });
        out.line(1, "always @* begin");
        ctx.block(body, 2, false, out);
        out.line(1, "end");
    }
    out.line(0, "endmodule");
    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_are_escaped() {
        assert_eq!(vname("edge"), "edge_n");
        assert_eq!(vname("count"), "count");
        assert_eq!(vname("3x"), "n_3x");
    }

    #[test]
    fn sized_literals_have_their_value() {
        assert_eq!(bits_value("10'b11_1111_1111"), Some(1023.0));
        assert_eq!(bits_value("8'hff"), Some(255.0));
        assert_eq!(bits_value("4'd9"), Some(9.0));
    }
}
