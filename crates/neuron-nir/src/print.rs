//! Printing NIR back to Neuron source (property P-RT, spec 16 §16.4).
//!
//! The printer produces one canonical layout: two-space indentation, every block on
//! its own lines, one blank line between top-level items, and parentheses only where
//! the precedence of chapter 4 §4.6 requires them.

use crate::model::*;

/// Prints one module as Neuron source.
pub fn print_module(m: &Module) -> String {
    let mut p = Printer::default();
    match m.kind {
        ModuleKind::Cell => {
            p.line(0, &format!("cell {}", m.name));
            for (i, item) in m.items.iter().enumerate() {
                if i > 0 {
                    p.blank();
                }
                p.item(item, 1);
            }
        }
        ModuleKind::Module => {
            let mut h = format!("module {}", m.name);
            if m.dialect != "core" {
                h.push_str(&format!(" for {}", m.dialect));
            }
            if let Some(t) = &m.target {
                h.push_str(&format!(" on {t}"));
            }
            p.line(0, &h);
            for item in &m.items {
                p.blank();
                p.item(item, 0);
            }
        }
    }
    p.out
}

/// Text of a unit expression with its factors in source order.
pub fn unit_text(u: &UnitRef) -> String {
    factors_text(&u.factors)
}

/// Text of unit factors: `m/s^2`, `N*m`, `%`.
pub fn factors_text(f: &[UnitFactor]) -> String {
    let mut s = String::new();
    for (i, fac) in f.iter().enumerate() {
        let pow = |e: i32| {
            if e == 1 {
                String::new()
            } else {
                format!("^{e}")
            }
        };
        if i == 0 {
            s.push_str(&fac.name);
            s.push_str(&pow(fac.exp));
        } else if fac.exp < 0 {
            s.push('/');
            s.push_str(&fac.name);
            s.push_str(&pow(-fac.exp));
        } else {
            s.push('*');
            s.push_str(&fac.name);
            s.push_str(&pow(fac.exp));
        }
    }
    s
}

#[derive(Default)]
struct Printer {
    out: String,
}

fn ind(n: usize) -> String {
    "  ".repeat(n)
}

// Precedence levels (spec 04 §4.6). Higher binds tighter.
const P_TOP: u8 = 0;
const P_PIPE: u8 = 1;
const P_COALESCE: u8 = 3;
const P_OR: u8 = 4;
const P_AND: u8 = 5;
const P_NOT: u8 = 6;
const P_CMP: u8 = 7;
const P_RANGE: u8 = 8;
const P_BOR: u8 = 9;
const P_BXOR: u8 = 10;
const P_BAND: u8 = 11;
const P_SHIFT: u8 = 12;
const P_ADD: u8 = 13;
const P_MUL: u8 = 14;
const P_UNARY: u8 = 15;
const P_POW: u8 = 16;
const P_CAST: u8 = 17;
const P_POSTFIX: u8 = 18;

fn binop_prec(op: &str) -> u8 {
    match op {
        "??" => P_COALESCE,
        "or" => P_OR,
        "and" => P_AND,
        "==" | "!=" | "<" | "<=" | ">" | ">=" | "in" => P_CMP,
        "|" => P_BOR,
        "^" => P_BXOR,
        "&" => P_BAND,
        "<<" | ">>" => P_SHIFT,
        "+" | "-" | "+%" | "-%" | "+|" | "-|" | "++" => P_ADD,
        "**" => P_POW,
        _ => P_MUL,
    }
}

/// Precedence of an expression as an operand. Expressions that extend as far to the
/// right as possible (`if … then … else`, lambdas) report 0 so that they are wrapped
/// in parentheses anywhere except at the top of an expression.
fn prec(e: &Expr) -> u8 {
    match &e.e {
        ExprKind::Pipe { .. } => P_PIPE,
        ExprKind::IfElse { .. } | ExprKind::Lambda { .. } => P_TOP,
        ExprKind::Binary { op, .. } => binop_prec(op),
        ExprKind::Compare { .. } => P_CMP,
        ExprKind::Unary { op, .. } if op == "not" => P_NOT,
        ExprKind::Unary { .. } | ExprKind::Run { .. } => P_UNARY,
        ExprKind::Range { .. } | ExprKind::Map { .. } => P_RANGE,
        ExprKind::Cast { .. } | ExprKind::Quantity { .. } => P_CAST,
        _ => P_POSTFIX,
    }
}

impl Printer {
    fn line(&mut self, n: usize, s: &str) {
        self.out.push_str(&ind(n));
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn blank(&mut self) {
        if !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
    }

    // ---- items --------------------------------------------------------------

    fn item(&mut self, item: &Item, n: usize) {
        if let Some(doc) = &item.doc {
            for l in doc.split('\n') {
                if l.is_empty() {
                    self.line(n, "##");
                } else {
                    self.line(n, &format!("## {l}"));
                }
            }
        }
        for a in &item.attrs {
            if a.args.is_empty() {
                self.line(n, &format!("@{}", a.name));
            } else {
                self.line(n, &format!("@{}({})", a.name, args(&a.args)));
            }
        }
        let pubs = if item.public { "pub " } else { "" };
        self.def(&item.def, pubs, n);
    }

    fn def(&mut self, d: &ItemDef, pubs: &str, n: usize) {
        match d {
            ItemDef::Use { path, names, alias } => {
                let mut s = format!("{pubs}use {}", path.join("."));
                if !names.is_empty() {
                    s.push_str(&format!(".{{{}}}", names.join(", ")));
                }
                if let Some(a) = alias {
                    s.push_str(&format!(" as {a}"));
                }
                self.line(n, &s);
            }
            ItemDef::Const { name, ty, value } => {
                let t = ty
                    .as_ref()
                    .map_or(String::new(), |t| format!(": {}", type_expr(t)));
                self.line(n, &format!("{pubs}const {name}{t} = {}", expr(value)));
            }
            ItemDef::Var { var } => self.line(n, &format!("{pubs}{}", var_decl(var))),
            ItemDef::Signal { name, ty, init } => {
                let i = init
                    .as_ref()
                    .map_or(String::new(), |e| format!(" = {}", expr(e)));
                self.line(n, &format!("{pubs}signal {name}: {}{i}", type_expr(ty)));
            }
            ItemDef::Unit { name, def } => {
                self.line(n, &format!("{pubs}unit {name} = {}", unit_text(def)));
            }
            ItemDef::Event { name, fields, from } => {
                let mut s = format!("{pubs}event {name}");
                if !fields.is_empty() {
                    let f: Vec<String> = fields
                        .iter()
                        .map(|f| match &f.ty {
                            Some(t) => format!("{}: {}", f.name, type_expr(t)),
                            None => f.name.clone(),
                        })
                        .collect();
                    s.push_str(&format!(": {}", f.join(", ")));
                }
                if let Some(d) = from {
                    s.push_str(&format!(" from {d}"));
                }
                self.line(n, &s);
            }
            ItemDef::Struct {
                name,
                generics: g,
                fields,
            } => {
                self.line(n, &format!("{pubs}struct {name}{}:", generics(g)));
                for f in fields {
                    let d = f
                        .default
                        .as_ref()
                        .map_or(String::new(), |e| format!(" = {}", expr(e)));
                    self.line(n + 1, &format!("{}: {}{d}", f.name, type_expr(&f.ty)));
                }
            }
            ItemDef::Enum {
                name,
                generics: g,
                repr,
                variants,
            } => {
                let r = repr
                    .as_ref()
                    .map_or(String::new(), |t| format!(": {}", type_expr(t)));
                self.line(n, &format!("{pubs}enum {name}{}{r}:", generics(g)));
                for v in variants {
                    let mut s = v.name.clone();
                    if !v.payload.is_empty() {
                        let p: Vec<String> = v.payload.iter().map(type_expr).collect();
                        s.push_str(&format!("({})", p.join(", ")));
                    }
                    if let Some(e) = &v.value {
                        s.push_str(&format!(" = {}", expr(e)));
                    }
                    self.line(n + 1, &s);
                }
            }
            ItemDef::Class {
                name,
                generics: g,
                interfaces,
                vars,
                invariants,
                methods,
            } => {
                let is = if interfaces.is_empty() {
                    String::new()
                } else {
                    format!(" is {}", interfaces.join(", "))
                };
                self.line(n, &format!("{pubs}class {name}{}{is}:", generics(g)));
                for v in vars {
                    self.line(n + 1, &var_decl(v));
                }
                for e in invariants {
                    self.line(n + 1, &format!("invariant {}", expr(e)));
                }
                for m in methods {
                    self.blank();
                    self.item(m, n + 1);
                }
                if vars.is_empty() && invariants.is_empty() && methods.is_empty() {
                    self.line(n + 1, "pass");
                }
            }
            ItemDef::Interface {
                name,
                generics: g,
                methods,
            } => {
                self.line(n, &format!("{pubs}interface {name}{}:", generics(g)));
                for m in methods {
                    self.line(n + 1, &fn_sig(m));
                }
            }
            ItemDef::Alias {
                name,
                generics: g,
                ty,
            } => {
                self.line(
                    n,
                    &format!("{pubs}type {name}{} = {}", generics(g), type_expr(ty)),
                );
            }
            ItemDef::Fn { func } => self.fn_decl(func, pubs, "", n),
            ItemDef::ExternFn { abi, func } => {
                self.fn_decl(func, pubs, &format!("extern {abi} "), n)
            }
            ItemDef::Task {
                name,
                trigger,
                attrs,
                body,
            } => {
                let a = task_attrs(attrs);
                self.line(
                    n,
                    &format!("{pubs}task {name} {}{a}:", trigger_text(trigger)),
                );
                self.block(body, n + 1);
            }
            ItemDef::Interrupt {
                source,
                attrs,
                body,
            } => {
                let a = task_attrs(attrs);
                self.line(n, &format!("{pubs}on interrupt {}{a}:", expr(source)));
                self.block(body, n + 1);
            }
            ItemDef::Rule { rule } => self.rule(rule, n),
            ItemDef::Fsm { fsm } => self.fsm(fsm, pubs, n),
            ItemDef::Sequence {
                name,
                params,
                on,
                items,
            } => {
                let p = if params.is_empty() {
                    String::new()
                } else {
                    format!(
                        "({})",
                        params.iter().map(param).collect::<Vec<_>>().join(", ")
                    )
                };
                let o = on.as_ref().map_or(String::new(), |d| format!(" on {d}"));
                self.line(n, &format!("{pubs}sequence {name}{p}{o}:"));
                self.seq_items(items, n + 1);
            }
            ItemDef::Safety { members } => {
                self.line(n, &format!("{pubs}safety:"));
                for m in members {
                    match &m.def {
                        SafetyDef::Rate { value } => {
                            self.line(n + 1, &format!("rate {}", expr(value)))
                        }
                        SafetyDef::Watchdog { value } => {
                            self.line(n + 1, &format!("watchdog {}", expr(value)));
                        }
                        SafetyDef::SafeState { body } => {
                            self.line(n + 1, "safe_state:");
                            self.block(body, n + 2);
                        }
                        SafetyDef::Rule { rule } => self.rule(rule, n + 1),
                        SafetyDef::OnFault { fault, body } => {
                            match fault {
                                Some(f) => self.line(n + 1, &format!("on fault {f}:")),
                                None => self.line(n + 1, "on fault:"),
                            }
                            self.block(body, n + 2);
                        }
                        SafetyDef::Reset { cond } => {
                            self.line(n + 1, &format!("reset when {}", expr(cond)))
                        }
                    }
                }
            }
            ItemDef::Verify { props } => {
                self.line(n, &format!("{pubs}verify:"));
                for p in props {
                    let s = match &p.def {
                        PropDef::Never { cond } => format!("never {}", expr(cond)),
                        PropDef::Always { cond, implies } => match implies {
                            Some(i) => format!("always {} implies {}", expr(cond), expr(i)),
                            None => format!("always {}", expr(cond)),
                        },
                        PropDef::EventuallyWithin {
                            within,
                            after,
                            goal,
                        } => format!(
                            "eventually_within {} after {}: {}",
                            expr(within),
                            expr(after),
                            expr(goal)
                        ),
                    };
                    self.line(n + 1, &s);
                }
            }
            ItemDef::Test { name, body } => {
                self.line(n, &format!("{pubs}test {name}:"));
                self.block(body, n + 1);
            }
            ItemDef::Io { name, kind, at } => {
                self.line(
                    n,
                    &format!("{pubs}io {name}: {} @ {}", expr(kind), expr(at)),
                );
            }
            ItemDef::Clocks { clocks } => {
                self.line(n, &format!("{pubs}clocks:"));
                for c in clocks {
                    let f = c
                        .from
                        .as_ref()
                        .map_or(String::new(), |e| format!(" from {}", expr(e)));
                    self.line(n + 1, &format!("{} = {}{f}", c.name, expr(&c.value)));
                }
            }
            ItemDef::Uses { paths } => self.line(n, &format!("{pubs}uses {}", paths.join(", "))),
            ItemDef::ExternBlock { abi, fns } => {
                self.line(n, &format!("{pubs}trusted extern {abi}:"));
                for f in fns {
                    self.line(n + 1, &fn_sig(f));
                }
            }
            ItemDef::Asm { asm } => self.asm(asm, n),
            ItemDef::Scan { period } => self.line(n, &format!("{pubs}scan {}", expr(period))),
            ItemDef::PlcSection { section, vars } => {
                self.line(n, &format!("{section}:"));
                for v in vars {
                    let r = if v.retain { "retain " } else { "" };
                    let t =
                        v.ty.as_ref()
                            .map_or(String::new(), |t| format!(": {}", type_expr(t)));
                    let i = v
                        .init
                        .as_ref()
                        .map_or(String::new(), |e| format!(" = {}", expr(e)));
                    let a =
                        v.at.as_ref()
                            .map_or(String::new(), |e| format!(" @ {}", expr(e)));
                    self.line(n + 1, &format!("{r}{}{t}{i}{a}", v.name));
                }
            }
            ItemDef::Rung { name, body } => {
                self.line(n, &format!("{pubs}rung {name}:"));
                self.block(body, n + 1);
            }
            ItemDef::Clock { name, freq, pin } => {
                let p = pin
                    .as_ref()
                    .map_or(String::new(), |e| format!(" @ {}", expr(e)));
                self.line(n, &format!("{pubs}clock {name}: {}{p}", expr(freq)));
            }
            ItemDef::Circuit { circuit } => self.circuit(circuit, pubs, n),
            ItemDef::Sync { from, to, via } => {
                self.line(
                    n,
                    &format!(
                        "{pubs}sync {} -> {} via {}",
                        expr(from),
                        expr(to),
                        expr(via)
                    ),
                );
            }
            ItemDef::Geometry {
                kind,
                name,
                ty,
                value,
            } => {
                let t = ty
                    .as_ref()
                    .map_or(String::new(), |t| format!(": {}", type_expr(t)));
                self.line(n, &format!("{pubs}{kind} {name}{t} = {}", expr(value)));
            }
            ItemDef::Display { name, driver } => {
                self.line(n, &format!("{pubs}display {name}: {}", expr(driver)));
            }
            ItemDef::Screen {
                name,
                display,
                widgets,
            } => {
                let o = display
                    .as_ref()
                    .map_or(String::new(), |d| format!(" on {d}"));
                self.line(n, &format!("{pubs}screen {name}{o}:"));
                for w in widgets {
                    self.widget(w, n + 1);
                }
            }
            ItemDef::Theme { name, body } => {
                self.line(n, &format!("{pubs}theme {name}:"));
                self.block(body, n + 1);
            }
            ItemDef::Device { name, ty, address } => {
                let a = address
                    .as_ref()
                    .map_or(String::new(), |e| format!(" @ {}", expr(e)));
                self.line(n, &format!("{pubs}device {name}: {ty}{a}"));
            }
            ItemDef::Place { module, device } => {
                self.line(n, &format!("{pubs}place {module} on {device}"))
            }
            ItemDef::Link {
                from,
                to,
                via,
                every,
                within,
            } => {
                let mut s = format!(
                    "{pubs}link {} -> {} via {}",
                    expr(from),
                    expr(to),
                    expr(via)
                );
                if let Some(e) = every {
                    s.push_str(&format!(" every {}", expr(e)));
                }
                if let Some(e) = within {
                    s.push_str(&format!(" within {}", expr(e)));
                }
                self.line(n, &s);
            }
            ItemDef::Tag {
                name,
                ty,
                locations,
            } => {
                let l: Vec<String> = locations.iter().map(expr).collect();
                self.line(
                    n,
                    &format!("{pubs}tag {name}: {} @ {}", type_expr(ty), l.join(", ")),
                );
            }
            ItemDef::Zone { name, shape } => {
                self.line(n, &format!("{pubs}zone {name}: {}", expr(shape)))
            }
            ItemDef::Mutex { zone, parties } => {
                let p: Vec<String> = parties.iter().map(expr).collect();
                self.line(n, &format!("{pubs}mutex {zone} between {}", p.join(", ")));
            }
        }
    }

    fn fn_decl(&mut self, f: &FnDecl, pubs: &str, prefix: &str, n: usize) {
        match &f.body {
            FnBody::Expr { expr: e } if f.contracts.is_empty() => {
                self.line(
                    n,
                    &format!("{pubs}{prefix}{} = {}", fn_sig(&f.sig), expr(e)),
                );
            }
            FnBody::Expr { expr: e } => {
                // Contracts need a block body.
                self.line(n, &format!("{pubs}{prefix}{}:", fn_sig(&f.sig)));
                self.contracts(&f.contracts, n + 1);
                self.line(n + 1, &format!("return {}", expr(e)));
            }
            FnBody::Block { block } => {
                self.line(n, &format!("{pubs}{prefix}{}:", fn_sig(&f.sig)));
                self.contracts(&f.contracts, n + 1);
                self.block_inner(block, n + 1, !f.contracts.is_empty());
            }
        }
    }

    fn contracts(&mut self, cs: &[Contract], n: usize) {
        for c in cs {
            self.line(n, &format!("{} {}", c.kind, expr(&c.cond)));
        }
    }

    fn rule(&mut self, r: &Rule, n: usize) {
        let h = r
            .hold
            .as_ref()
            .map_or(String::new(), |e| format!(" for {}", expr(e)));
        self.line(n, &format!("when {}{h}:", expr(&r.cond)));
        self.block(&r.body, n + 1);
    }

    fn fsm(&mut self, f: &Fsm, pubs: &str, n: usize) {
        let t = f
            .trigger
            .as_ref()
            .map_or(String::new(), |t| format!(" {}", trigger_text(t)));
        self.line(n, &format!("{pubs}fsm {}{t}:", f.name));
        for v in &f.vars {
            self.line(n + 1, &var_decl(v));
        }
        for s in &f.states {
            let mut h = String::new();
            if s.initial {
                h.push_str("initial ");
            }
            if s.is_final {
                h.push_str("final ");
            }
            self.line(n + 1, &format!("{h}state {}:", s.name));
            let mut any = false;
            for (kw, b) in [("entry", &s.entry), ("exit", &s.exit), ("do", &s.during)] {
                if let Some(b) = b {
                    any = true;
                    self.line(n + 2, &format!("{kw}:"));
                    self.block(b, n + 3);
                }
            }
            for t in &s.transitions {
                any = true;
                let mut l = format!("-> {}", t.target);
                match (&t.when, &t.after) {
                    (Some(w), Some(a)) => {
                        l.push_str(&format!(" when {} or after {}", expr(w), expr(a)))
                    }
                    (Some(w), None) => l.push_str(&format!(" when {}", expr(w))),
                    (None, Some(a)) => l.push_str(&format!(" after {}", expr(a))),
                    (None, None) => {}
                }
                match &t.action {
                    Some(b) => {
                        l.push(':');
                        self.line(n + 2, &l);
                        self.block(b, n + 3);
                    }
                    None => self.line(n + 2, &l),
                }
            }
            if !any {
                self.line(n + 2, "do:");
                self.line(n + 3, "pass");
            }
        }
    }

    fn seq_items(&mut self, items: &[SeqItem], n: usize) {
        for it in items {
            match it {
                SeqItem::Step {
                    initial,
                    name,
                    body,
                    ..
                } => {
                    let i = if *initial { "initial " } else { "" };
                    self.line(n, &format!("{i}step {name}:"));
                    self.seq_items(body, n + 1);
                }
                SeqItem::Parallel { branches, .. } => {
                    self.line(n, "parallel:");
                    for b in branches {
                        self.line(n + 1, &format!("branch {}:", b.name));
                        self.seq_items(&b.items, n + 2);
                    }
                }
                SeqItem::Stmt { stmt } => self.stmt(stmt, n),
            }
        }
    }

    fn circuit(&mut self, c: &Circuit, pubs: &str, n: usize) {
        let ports: Vec<String> = c
            .ports
            .iter()
            .map(|p| {
                let pin = p
                    .pin
                    .as_ref()
                    .map_or(String::new(), |e| format!(" @ {}", expr(e)));
                format!("{}: {} {}{pin}", p.name, p.dir, type_expr(&p.ty))
            })
            .collect();
        let on = c
            .clock
            .as_ref()
            .map_or(String::new(), |k| format!(" on {k}"));
        self.line(
            n,
            &format!("{pubs}circuit {}({}){on}:", c.name, ports.join(", ")),
        );
        for m in &c.members {
            match &m.def {
                CircuitDef::Reg { name, ty, init, .. } => {
                    let i = init
                        .as_ref()
                        .map_or(String::new(), |e| format!(" = {}", expr(e)));
                    self.line(n + 1, &format!("reg {name}: {}{i}", type_expr(ty)));
                }
                CircuitDef::Wire { name, ty, .. } => {
                    self.line(n + 1, &format!("wire {name}: {}", type_expr(ty)));
                }
                CircuitDef::Const {
                    name, ty, value, ..
                } => {
                    let t = ty
                        .as_ref()
                        .map_or(String::new(), |t| format!(": {}", type_expr(t)));
                    self.line(n + 1, &format!("const {name}{t} = {}", expr(value)));
                }
                CircuitDef::Instance { name, of, .. } => {
                    self.line(n + 1, &format!("instance {name} = {}", expr(of)));
                }
                CircuitDef::Comb { body } => {
                    self.line(n + 1, "comb:");
                    self.block(body, n + 2);
                }
                CircuitDef::OnEdge {
                    rising,
                    clock,
                    body,
                } => {
                    let e = if *rising { "rising" } else { "falling" };
                    self.line(n + 1, &format!("on {e}({clock}):"));
                    self.block(body, n + 2);
                }
                CircuitDef::Fsm { fsm } => self.fsm(fsm, "", n + 1),
            }
        }
        if c.members.is_empty() {
            self.line(n + 1, "comb:");
            self.line(n + 2, "pass");
        }
    }

    fn widget(&mut self, w: &Widget, n: usize) {
        let mut s = w.kind.clone();
        if let Some(name) = &w.name {
            s.push_str(&format!(" {name}"));
        }
        if let Some(t) = &w.text {
            s.push_str(&format!(" {t}"));
        }
        for o in &w.options {
            s.push_str(&format!(" {} {}", o.name, expr(&o.value)));
        }
        if w.children.is_empty() && w.events.is_empty() {
            self.line(n, &s);
            return;
        }
        s.push(':');
        self.line(n, &s);
        for c in &w.children {
            self.widget(c, n + 1);
        }
        for ev in &w.events {
            self.line(n + 1, &format!("on {}:", ev.name));
            self.block(&ev.body, n + 2);
        }
    }

    fn asm(&mut self, a: &Asm, n: usize) {
        self.line(n, &format!("trusted asm {}({}):", a.arch, a.operands));
        for l in &a.lines {
            self.line(n + 1, l.trim());
        }
    }

    // ---- statements ---------------------------------------------------------

    fn block(&mut self, b: &Block, n: usize) {
        self.block_inner(b, n, false);
    }

    fn block_inner(&mut self, b: &Block, n: usize, has_lines: bool) {
        self.contracts(&b.contracts, n);
        for s in &b.stmts {
            self.stmt(s, n);
        }
        if b.contracts.is_empty() && b.stmts.is_empty() && !has_lines {
            self.line(n, "pass");
        }
    }

    fn stmt(&mut self, s: &Stmt, n: usize) {
        match &s.s {
            StmtKind::Let {
                mutable,
                pattern: p,
                ty,
                value,
            } => {
                let kw = if *mutable { "var" } else { "let" };
                let t = ty
                    .as_ref()
                    .map_or(String::new(), |t| format!(": {}", type_expr(t)));
                self.line(n, &format!("{kw} {}{t} = {}", pattern(p), expr(value)));
            }
            StmtKind::Assign {
                place,
                op,
                value,
                when,
            } => {
                let w = when
                    .as_ref()
                    .map_or(String::new(), |e| format!(" when {}", expr(e)));
                self.line(n, &format!("{} {op} {}{w}", expr(place), expr(value)));
            }
            StmtKind::Expr { expr: e } => self.line(n, &expr(e)),
            StmtKind::Pass => self.line(n, "pass"),
            StmtKind::Return { value } => match value {
                Some(e) => self.line(n, &format!("return {}", expr(e))),
                None => self.line(n, "return"),
            },
            StmtKind::Break => self.line(n, "break"),
            StmtKind::Continue => self.line(n, "continue"),
            StmtKind::Fault { name, args: a } => {
                if a.is_empty() {
                    self.line(n, &format!("fault {name}"));
                } else {
                    self.line(n, &format!("fault {name}({})", args(a)));
                }
            }
            StmtKind::Assert { cond, msg } => match msg {
                Some(m) => self.line(n, &format!("assert {}, {m}", expr(cond))),
                None => self.line(n, &format!("assert {}", expr(cond))),
            },
            StmtKind::Emit { event, args: a } => {
                if a.is_empty() {
                    self.line(n, &format!("emit {event}"));
                } else {
                    self.line(n, &format!("emit {event}({})", args(a)));
                }
            }
            StmtKind::Coil { set, place, when } => {
                let kw = if *set { "set" } else { "reset" };
                self.line(n, &format!("{kw} {} when {}", expr(place), expr(when)));
            }
            StmtKind::Wait { duration } => self.line(n, &format!("wait {}", expr(duration))),
            StmtKind::Until {
                cond,
                timeout,
                on_timeout,
            } => {
                let mut l = format!("until {}", expr(cond));
                if let Some(t) = timeout {
                    l.push_str(&format!(" timeout {}", expr(t)));
                    if let Some(j) = on_timeout {
                        l.push_str(&format!(" -> {j}"));
                    }
                }
                self.line(n, &l);
            }
            StmtKind::Jump { target } => self.line(n, &format!("-> {target}")),
            StmtKind::Move {
                kind,
                to,
                via,
                options,
            } => {
                let mut l = format!("move {kind} to {}", expr(to));
                if let Some(v) = via {
                    l.push_str(&format!(" via {}", expr(v)));
                }
                for o in options {
                    l.push_str(&format!(" {} {}", o.name, expr(&o.value)));
                }
                self.line(n, &l);
            }
            StmtKind::If {
                branches,
                otherwise,
            } => {
                for (i, b) in branches.iter().enumerate() {
                    let kw = if i == 0 { "if" } else { "elif" };
                    self.line(n, &format!("{kw} {}:", expr(&b.cond)));
                    self.block(&b.body, n + 1);
                }
                if let Some(o) = otherwise {
                    self.line(n, "else:");
                    self.block(o, n + 1);
                }
            }
            StmtKind::Match { value, arms } => {
                self.line(n, &format!("match {}:", expr(value)));
                for a in arms {
                    let g = a
                        .guard
                        .as_ref()
                        .map_or(String::new(), |e| format!(" if {}", expr(e)));
                    self.line(n + 1, &format!("{}{g} =>", pattern(&a.pattern)));
                    self.block(&a.body, n + 2);
                }
            }
            StmtKind::For {
                pattern: p,
                iter,
                body,
                ..
            } => {
                self.line(n, &format!("for {} in {}:", pattern(p), expr(iter)));
                self.block(body, n + 1);
            }
            StmtKind::While {
                pattern: p,
                cond,
                max,
                body,
            } => {
                let m = max
                    .as_ref()
                    .map_or(String::new(), |e| format!(" max {}", expr(e)));
                match p {
                    Some(p) => {
                        self.line(n, &format!("while let {} = {}{m}:", pattern(p), expr(cond)))
                    }
                    None => self.line(n, &format!("while {}{m}:", expr(cond))),
                }
                self.block(body, n + 1);
            }
            StmtKind::Loop { max, body } => {
                match max {
                    Some(e) => self.line(n, &format!("loop max {}:", expr(e))),
                    None => self.line(n, "loop:"),
                }
                self.block(body, n + 1);
            }
            StmtKind::Atomic { body } => {
                self.line(n, "atomic:");
                self.block(body, n + 1);
            }
            StmtKind::Rule { rule } => self.rule(rule, n),
            StmtKind::Asm { asm } => self.asm(asm, n),
        }
    }
}

// ---- pieces -----------------------------------------------------------------

fn var_decl(v: &VarDecl) -> String {
    let r = if v.retain { "retain " } else { "" };
    let kw = if v.mutable { "var" } else { "let" };
    let t =
        v.ty.as_ref()
            .map_or(String::new(), |t| format!(": {}", type_expr(t)));
    let i = v
        .init
        .as_ref()
        .map_or(String::new(), |e| format!(" = {}", expr(e)));
    let a =
        v.at.as_ref()
            .map_or(String::new(), |e| format!(" @ {}", expr(e)));
    format!("{r}{kw} {}{t}{i}{a}", v.name)
}

fn generics(g: &[Generic]) -> String {
    if g.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = g
        .iter()
        .map(|p| match p {
            Generic::Type { name, bounds } if bounds.is_empty() => name.clone(),
            Generic::Type { name, bounds } => format!("{name}: {}", bounds.join(" + ")),
            Generic::Const { name, ty } => format!("const {name}: {}", type_expr(ty)),
        })
        .collect();
    format!("<{}>", parts.join(", "))
}

fn param(p: &Param) -> String {
    let m = if p.mutable { "mut " } else { "" };
    match &p.ty {
        None => format!("{m}{}", p.name),
        Some(t) => {
            let d = p
                .default
                .as_ref()
                .map_or(String::new(), |e| format!(" = {}", expr(e)));
            format!("{m}{}: {}{d}", p.name, type_expr(t))
        }
    }
}

fn fn_sig(s: &FnSig) -> String {
    let ps: Vec<String> = s.params.iter().map(param).collect();
    let r = s
        .ret
        .as_ref()
        .map_or(String::new(), |t| format!(" -> {}", type_expr(t)));
    format!(
        "fn {}{}({}){r}",
        s.name,
        generics(&s.generics),
        ps.join(", ")
    )
}

fn trigger_text(t: &Trigger) -> String {
    match t {
        Trigger::Every { period, offset } => match offset {
            Some(o) => format!("every {} offset {}", expr(period), expr(o)),
            None => format!("every {}", expr(period)),
        },
        Trigger::On { event } => format!("on {}", expr(event)),
    }
}

fn task_attrs(a: &[TaskAttr]) -> String {
    if a.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = a
        .iter()
        .map(|x| match x {
            TaskAttr::Prio { value } => format!("prio {}", expr(value)),
            TaskAttr::Deadline { value } => format!("deadline {}", expr(value)),
            TaskAttr::Core { value } => format!("core {}", expr(value)),
            TaskAttr::Stack { value } => format!("stack {}", expr(value)),
            TaskAttr::Reads { names } => format!("reads {}", names.join(" ")),
            TaskAttr::Writes { names } => format!("writes {}", names.join(" ")),
        })
        .collect();
    format!(" ({})", parts.join(", "))
}

/// Prints a written type.
pub fn type_expr(t: &TypeExpr) -> String {
    let mut s = match &t.kind {
        TypeExprKind::Named { path, args } => {
            if args.is_empty() {
                path.clone()
            } else {
                let a: Vec<String> = args
                    .iter()
                    .map(|a| match a {
                        TypeArg::Type { ty } => type_expr(ty),
                        TypeArg::Expr { expr: e } => expr(e),
                    })
                    .collect();
                format!("{path}<{}>", a.join(", "))
            }
        }
        TypeExprKind::Array { elem, len } => format!("[{}; {}]", type_expr(elem), expr(len)),
        TypeExprKind::Slice { elem } => format!("[{}]", type_expr(elem)),
        TypeExprKind::Tuple { items } => {
            let v: Vec<String> = items.iter().map(type_expr).collect();
            format!("({})", v.join(", "))
        }
        TypeExprKind::Unit => String::new(),
    };
    if let Some(u) = &t.unit {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&unit_text(u));
    }
    if t.optional {
        s.push('?');
    }
    s
}

fn pattern(p: &Pattern) -> String {
    match &p.p {
        PatternKind::Wildcard => "_".into(),
        PatternKind::Bind { name, .. } => name.clone(),
        PatternKind::Lit { value } => expr(value),
        PatternKind::Range { lo, hi, inclusive } => {
            let op = if *inclusive { "..=" } else { ".." };
            format!("{}{op}{}", expr_at(lo, P_BOR), expr_at(hi, P_BOR))
        }
        PatternKind::Ctor {
            path,
            args: a,
            fields,
        } => {
            let mut parts: Vec<String> = a.iter().map(pattern).collect();
            parts.extend(
                fields
                    .iter()
                    .map(|f| format!("{}: {}", f.name, pattern(&f.pattern))),
            );
            format!("{path}({})", parts.join(", "))
        }
        PatternKind::Path { path } => path.clone(),
        PatternKind::Tuple { items } => {
            let v: Vec<String> = items.iter().map(pattern).collect();
            if v.len() == 1 {
                format!("({},)", v[0])
            } else {
                format!("({})", v.join(", "))
            }
        }
        PatternKind::Or { alts } => alts.iter().map(pattern).collect::<Vec<_>>().join(" | "),
    }
}

fn args(a: &[Arg]) -> String {
    a.iter()
        .map(|x| {
            let m = if x.mutable { "mut " } else { "" };
            let n = x.name.as_ref().map_or(String::new(), |n| format!("{n}: "));
            format!("{m}{n}{}", expr(&x.value))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn lit(l: &Lit) -> String {
    match l {
        Lit::Int { text, .. }
        | Lit::Real { text, .. }
        | Lit::Str { text }
        | Lit::RawStr { text }
        | Lit::ByteStr { text }
        | Lit::Char { text }
        | Lit::Iec { text }
        | Lit::Bits { text }
        | Lit::Ip { text } => text.clone(),
        Lit::Bool { v } => if *v { "true" } else { "false" }.into(),
        Lit::None => "none".into(),
    }
}

/// Prints an expression at the top level.
pub fn expr(e: &Expr) -> String {
    expr_at(e, P_TOP)
}

fn expr_at(e: &Expr, min: u8) -> String {
    let s = expr_raw(e);
    let p = prec(e);
    let wrap = if p == P_TOP { min > P_TOP } else { p < min };
    if wrap {
        format!("({s})")
    } else {
        s
    }
}

/// `true` for a numeric literal, possibly negated (`40`, `-40`, `0.5`).
fn is_plain_number(e: &Expr) -> bool {
    match &e.e {
        ExprKind::Lit {
            lit: Lit::Int { .. } | Lit::Real { .. },
        } => true,
        ExprKind::Unary { op, operand } if op == "-" => is_plain_number(operand),
        _ => false,
    }
}

fn expr_raw(e: &Expr) -> String {
    match &e.e {
        ExprKind::Lit { lit: l } => lit(l),
        ExprKind::Quantity { value, unit, .. } => {
            let v = match &value.e {
                ExprKind::Tuple { .. } | ExprKind::Call { .. } | ExprKind::Lit { .. } => {
                    expr_raw(value)
                }
                _ => expr_at(value, P_POSTFIX),
            };
            format!("{v} {}", unit_text(unit))
        }
        ExprKind::Ref { name, .. }
        | ExprKind::Local { name, .. }
        | ExprKind::Global { name }
        | ExprKind::Module { name, .. } => name.clone(),
        ExprKind::State { state, .. } => state.clone(),
        ExprKind::SelfValue => "self".into(),
        ExprKind::Result => "result".into(),
        ExprKind::Binary { op, lhs, rhs } => {
            let p = binop_prec(op);
            let (lp, rp) = match op.as_str() {
                // Right associative.
                "??" => (p + 1, p),
                // `cast_expr ** unary_expr`.
                "**" => (P_CAST, P_UNARY),
                // Non-associative comparisons take range operands.
                _ if p == P_CMP => (P_RANGE, P_RANGE),
                _ => (p, p + 1),
            };
            format!("{} {op} {}", expr_at(lhs, lp), expr_at(rhs, rp))
        }
        ExprKind::Compare { first, rest } => {
            let mut s = expr_at(first, P_RANGE);
            for r in rest {
                s.push_str(&format!(" {} {}", r.op, expr_at(&r.rhs, P_RANGE)));
            }
            s
        }
        ExprKind::Unary { op, operand } => {
            if op == "not" {
                format!("not {}", expr_at(operand, P_NOT))
            } else {
                format!("{op}{}", expr_at(operand, P_UNARY))
            }
        }
        ExprKind::Range { lo, hi, inclusive } => {
            let op = if *inclusive { "..=" } else { ".." };
            // `-40..40 mm`: the parser gives a plain-number lower bound the unit of the
            // upper bound, so print that short form back (it is the only way to write a
            // quantity of a negated literal).
            if let (
                ExprKind::Quantity {
                    value, unit: lu, ..
                },
                ExprKind::Quantity { unit: hu, .. },
            ) = (&lo.e, &hi.e)
            {
                if lu.text == hu.text && is_plain_number(value) {
                    return format!("{}{op}{}", expr_raw(value), expr_at(hi, P_BOR));
                }
            }
            format!("{}{op}{}", expr_at(lo, P_BOR), expr_at(hi, P_BOR))
        }
        ExprKind::Map { from, to } => {
            format!("{} -> {}", expr_at(from, P_RANGE), expr_at(to, P_RANGE))
        }
        ExprKind::IfElse {
            cond,
            then,
            otherwise,
        } => {
            format!(
                "if {} then {} else {}",
                expr(cond),
                expr(then),
                expr(otherwise)
            )
        }
        ExprKind::Pipe { lhs, rhs } => {
            format!("{} |> {}", expr_at(lhs, P_PIPE), expr_at(rhs, P_POSTFIX))
        }
        ExprKind::Call {
            callee, args: a, ..
        } => format!("{}({})", expr_at(callee, P_POSTFIX), args(a)),
        ExprKind::Index { base, index, end } => match end {
            Some(x) => format!("{}[{}:{}]", expr_at(base, P_POSTFIX), expr(index), expr(x)),
            None => format!("{}[{}]", expr_at(base, P_POSTFIX), expr(index)),
        },
        ExprKind::Field { base, name, .. } => format!("{}.{name}", expr_at(base, P_POSTFIX)),
        ExprKind::Try { operand } => format!("{}?", expr_at(operand, P_POSTFIX)),
        ExprKind::Cast { operand, to } => {
            format!("{} as {}", expr_at(operand, P_CAST), type_expr(to))
        }
        ExprKind::Tuple { items } => {
            let v: Vec<String> = items.iter().map(expr).collect();
            if v.len() == 1 {
                format!("({},)", v[0])
            } else {
                format!("({})", v.join(", "))
            }
        }
        ExprKind::Array { items } => {
            let v: Vec<String> = items.iter().map(expr).collect();
            format!("[{}]", v.join(", "))
        }
        ExprKind::ArrayRepeat { value, count } => format!("[{}; {}]", expr(value), expr(count)),
        ExprKind::Lambda { params, body } => {
            let p: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
            format!("fn({}) => {}", p.join(", "), expr(body))
        }
        ExprKind::Old { operand } => format!("old({})", expr(operand)),
        ExprKind::Run { action } => format!("run {}", expr_at(action, P_POSTFIX)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_text_keeps_source_order() {
        let f = |n: &str, e: i32| UnitFactor {
            name: n.into(),
            exp: e,
        };
        assert_eq!(factors_text(&[f("m", 1), f("s", -2)]), "m/s^2");
        assert_eq!(factors_text(&[f("N", 1), f("m", 1)]), "N*m");
        assert_eq!(factors_text(&[f("m", 1), f("s", -1), f("kg", 1)]), "m/s*kg");
        assert_eq!(factors_text(&[f("s", -1)]), "s^-1");
        assert_eq!(factors_text(&[f("%", 1)]), "%");
    }
}
