//! PLCopen XML (TC6 2.01) emitter: one program POU per `plc` module, with an LD body
//! when every rung is a network of contacts and coils, an SFC body when the module is one
//! sequence, and an ST body otherwise.

use super::gen::{fb_fields, flatten_seq, Block, Gen, PVar, Style, STD_BLOCKS};
use super::{no_plc, plc_modules};
use crate::fidelity::{Class, Fidelity};
use crate::nirx::{dur_ns, iec_time};
use crate::package::{Package, Writer};
use crate::{Diag, EmitOptions, Emitter, Manifest};
use neuron_nir::{Expr, ExprKind, ItemDef, Lit, Module, Nir, SafetyDef, Stmt, StmtKind};

/// The PLCopen XML emitter (`neuron emit plcopen`).
pub struct PlcOpen;

/// Escapes text for XML.
pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn st_text(code: &str) -> String {
    format!(
        "<ST><xhtml:p><![CDATA[{}]]></xhtml:p></ST>",
        code.replace("]]>", "]]]]><![CDATA[>")
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    Ld,
    Sfc(usize),
    St,
}

fn has_fault(b: &[Stmt]) -> bool {
    b.iter().any(|s| match &s.s {
        StmtKind::Fault { .. } => true,
        StmtKind::If {
            branches,
            otherwise,
        } => {
            branches.iter().any(|x| has_fault(&x.body.stmts))
                || otherwise.as_ref().is_some_and(|o| has_fault(&o.stmts))
        }
        StmtKind::For { body, .. } | StmtKind::While { body, .. } | StmtKind::Loop { body, .. } => {
            has_fault(&body.stmts)
        }
        _ => false,
    })
}

/// `true` for an expression that is a network of contacts.
fn contacts_ok(g: &Gen, e: &Expr) -> bool {
    match &e.e {
        ExprKind::Ref { id, .. } => {
            !g.blocks.iter().any(|b| &b.id == id) && matches!(e.ty, neuron_nir::Type::Bool)
        }
        ExprKind::Field { base, name, .. } => g.block_of(base).is_some_and(|b| {
            fb_fields(&b.kind)
                .iter()
                .any(|f| f.0 == name.as_str() && !f.2 && f.1.starts_with('Q'))
        }),
        ExprKind::Lit {
            lit: Lit::Bool { v: true },
        } => true,
        ExprKind::Unary { op, operand } if op == "not" => {
            matches!(operand.e, ExprKind::Ref { .. } | ExprKind::Field { .. })
                && contacts_ok(g, operand)
        }
        ExprKind::Binary { op, lhs, rhs } if op == "and" || op == "or" => {
            contacts_ok(g, lhs) && contacts_ok(g, rhs)
        }
        _ => false,
    }
}

fn coil_ok(g: &Gen, s: &Stmt) -> bool {
    match &s.s {
        StmtKind::Assign {
            place,
            op,
            value,
            when: None,
        } if op == "=" => {
            let target = match &place.e {
                ExprKind::Ref { id, .. } => !g.blocks.iter().any(|b| &b.id == id),
                ExprKind::Field { base, name, .. } => g.block_of(base).is_some_and(|b| {
                    fb_fields(&b.kind)
                        .iter()
                        .any(|f| f.0 == name.as_str() && f.2)
                }),
                _ => false,
            };
            target
                && matches!(place.ty, neuron_nir::Type::Bool | neuron_nir::Type::Unknown)
                && contacts_ok(g, value)
        }
        StmtKind::Coil { place, when, .. } => {
            matches!(place.e, ExprKind::Ref { .. }) && contacts_ok(g, when)
        }
        _ => false,
    }
}

fn classify(g: &Gen, m: &Module) -> Body {
    let mut rungs = 0;
    let mut seqs = Vec::new();
    let mut other = false;
    for (i, item) in m.items.iter().enumerate() {
        match &item.def {
            ItemDef::Rung { body, .. } => {
                rungs += 1;
                if !body.stmts.iter().all(|s| coil_ok(g, s)) {
                    other = true;
                }
            }
            ItemDef::Sequence { items, .. } => {
                seqs.push(i);
                let faults = flatten_seq(items).is_none_or(|ops| {
                    ops.iter()
                        .any(|(_, s)| s.is_some_and(|s| has_fault(std::slice::from_ref(s))))
                });
                if faults {
                    other = true;
                }
            }
            ItemDef::Rule { .. }
            | ItemDef::Fsm { .. }
            | ItemDef::Task { .. }
            | ItemDef::Interrupt { .. } => other = true,
            ItemDef::Safety { members } => {
                for mem in members {
                    match &mem.def {
                        SafetyDef::Rule { .. }
                        | SafetyDef::Reset { .. }
                        | SafetyDef::OnFault { .. } => other = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if g.blocks
        .iter()
        .any(|b| !STD_BLOCKS.contains(&b.kind.as_str()))
    {
        other = true;
    }
    if other {
        return Body::St;
    }
    match (rungs, seqs.as_slice()) {
        (r, []) if r > 0 => Body::Ld,
        (0, [s]) => Body::Sfc(*s),
        _ => Body::St,
    }
}

fn xml_type(t: &str) -> String {
    if let Some(rest) = t.strip_prefix("ARRAY") {
        return format!("<derived name=\"ARRAY{}\"/>", esc(rest));
    }
    match t {
        "BOOL" | "SINT" | "INT" | "DINT" | "LINT" | "USINT" | "UINT" | "UDINT" | "ULINT"
        | "REAL" | "LREAL" | "TIME" => {
            format!("<{t}/>")
        }
        "STRING" => "<string/>".into(),
        other => format!("<derived name=\"{}\"/>", esc(other)),
    }
}

struct Xml {
    out: String,
    depth: usize,
    next: usize,
    order: usize,
}

impl Xml {
    fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }
    fn open(&mut self, s: &str) {
        self.line(s);
        self.depth += 1;
    }
    fn close(&mut self, s: &str) {
        self.depth = self.depth.saturating_sub(1);
        self.line(s);
    }
    fn id(&mut self) -> usize {
        self.next += 1;
        self.next
    }
    fn order(&mut self) -> usize {
        self.order += 1;
        self.order
    }
    fn pos(&mut self, x: usize, y: usize) {
        self.line(&format!("<position x=\"{x}\" y=\"{y}\"/>"));
    }
    fn conn_in(&mut self, from: &[usize]) {
        if from.is_empty() {
            self.line("<connectionPointIn/>");
            return;
        }
        self.open("<connectionPointIn>");
        for f in from {
            self.line(&format!("<connection refLocalId=\"{f}\"/>"));
        }
        self.close("</connectionPointIn>");
    }
}

/// Contact network of a boolean expression; returns the elements that carry its result.
fn network(
    g: &mut Gen,
    x: &mut Xml,
    e: &Expr,
    inputs: Vec<usize>,
    col: &mut usize,
    row: usize,
) -> Vec<usize> {
    match &e.e {
        ExprKind::Lit {
            lit: Lit::Bool { v: true },
        } => inputs,
        ExprKind::Binary { op, lhs, rhs } if op == "and" => {
            let a = network(g, x, lhs, inputs, col, row);
            network(g, x, rhs, a, col, row)
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            let start = *col;
            let mut a = network(g, x, lhs, inputs.clone(), col, row);
            let end_a = *col;
            *col = start;
            let b = network(g, x, rhs, inputs, col, row + 1);
            *col = (*col).max(end_a);
            a.extend(b);
            a
        }
        ExprKind::Unary { operand, .. } => contact(g, x, operand, true, inputs, col, row),
        _ => contact(g, x, e, false, inputs, col, row),
    }
}

fn contact(
    g: &mut Gen,
    x: &mut Xml,
    e: &Expr,
    negated: bool,
    inputs: Vec<usize>,
    col: &mut usize,
    row: usize,
) -> Vec<usize> {
    let mut pre = Vec::new();
    let var = g.ex(e, &mut pre);
    let id = x.id();
    x.open(&format!(
        "<contact localId=\"{id}\" negated=\"{negated}\" edge=\"none\">"
    ));
    *col += 1;
    x.pos(*col * 80, row * 40);
    x.conn_in(&inputs);
    x.line("<connectionPointOut/>");
    x.line(&format!("<variable>{}</variable>", esc(&var)));
    x.close("</contact>");
    vec![id]
}

fn rail(x: &mut Xml, row: usize) -> usize {
    let id = x.id();
    x.open(&format!("<leftPowerRail localId=\"{id}\">"));
    x.pos(0, row * 40);
    x.line("<connectionPointOut formalParameter=\"\"/>");
    x.close("</leftPowerRail>");
    id
}

fn coil(x: &mut Xml, var: &str, storage: &str, from: &[usize], col: usize, row: usize) -> usize {
    let id = x.id();
    let order = x.order();
    x.open(&format!(
        "<coil localId=\"{id}\" negated=\"false\" storage=\"{storage}\" edge=\"none\" executionOrderId=\"{order}\">"
    ));
    x.pos((col + 1) * 80, row * 40);
    x.conn_in(from);
    x.line("<connectionPointOut/>");
    x.line(&format!("<variable>{}</variable>", esc(var)));
    x.close("</coil>");
    let r = x.id();
    x.open(&format!("<rightPowerRail localId=\"{r}\">"));
    x.pos((col + 2) * 80, row * 40);
    x.conn_in(&[id]);
    x.close("</rightPowerRail>");
    id
}

fn ld_body(g: &mut Gen, m: &Module, x: &mut Xml) {
    x.open("<LD>");
    let mut row = 0;
    for item in &m.items {
        let ItemDef::Rung { name, body } = &item.def else {
            continue;
        };
        x.line(&format!("<comment localId=\"{}\" height=\"20\" width=\"300\"><position x=\"0\" y=\"{}\"/><content><xhtml:p>rung {}</xhtml:p></content></comment>", x.next + 1, row * 40, esc(name)));
        x.next += 1;
        row += 1;
        for s in &body.stmts {
            let left = rail(x, row);
            let mut col = 0;
            match &s.s {
                StmtKind::Assign { place, value, .. } => {
                    let outs = network(g, x, value, vec![left], &mut col, row);
                    let target = g.place(place);
                    coil(x, &target, "none", &outs, col, row);
                }
                StmtKind::Coil { set, place, when } => {
                    let outs = network(g, x, when, vec![left], &mut col, row);
                    let target = g.place(place);
                    coil(
                        x,
                        &target,
                        if *set { "set" } else { "reset" },
                        &outs,
                        col,
                        row,
                    );
                }
                _ => {}
            }
            row += 2;
            g.fid.exact();
        }
    }
    // The standard blocks run once at the end of the scan, as in Neuron.
    let blocks: Vec<Block> = g.blocks.clone();
    for b in &blocks {
        let mut ins = Vec::new();
        for (_, iecf, input) in fb_fields(&b.kind) {
            if *input {
                let left = rail(x, row);
                let var = g.shadow(b, iecf);
                let id = x.id();
                x.open(&format!(
                    "<contact localId=\"{id}\" negated=\"false\" edge=\"none\">"
                ));
                x.pos(80, row * 40);
                x.conn_in(&[left]);
                x.line("<connectionPointOut/>");
                x.line(&format!("<variable>{}</variable>", esc(&var)));
                x.close("</contact>");
                ins.push((iecf.to_string(), id));
                row += 1;
            }
        }
        if let Some(p) = g.preset(b) {
            if !matches!(b.kind.as_str(), "r_trig" | "f_trig" | "sr" | "rs") {
                let id = x.id();
                x.open(&format!("<inVariable localId=\"{id}\">"));
                x.pos(80, row * 40);
                x.line("<connectionPointOut/>");
                x.line(&format!("<expression>{}</expression>", esc(&p)));
                x.close("</inVariable>");
                let f = if matches!(b.kind.as_str(), "ton" | "tof" | "tp") {
                    "PT"
                } else {
                    "PV"
                };
                ins.push((f.to_string(), id));
                row += 1;
            }
        }
        let id = x.id();
        let order = x.order();
        x.open(&format!(
            "<block localId=\"{id}\" typeName=\"{}\" instanceName=\"{}\" executionOrderId=\"{order}\">",
            b.kind.to_ascii_uppercase(),
            esc(&b.name)
        ));
        x.pos(240, row * 40);
        x.open("<inputVariables>");
        for (f, from) in &ins {
            x.open(&format!("<variable formalParameter=\"{f}\">"));
            x.conn_in(&[*from]);
            x.close("</variable>");
        }
        x.close("</inputVariables>");
        x.line("<inOutVariables/>");
        x.open("<outputVariables>");
        for (_, iecf, input) in fb_fields(&b.kind) {
            if !*input {
                x.open(&format!("<variable formalParameter=\"{iecf}\">"));
                x.line("<connectionPointOut/>");
                x.close("</variable>");
            }
        }
        x.close("</outputVariables>");
        x.close("</block>");
        row += 2;
    }
    x.close("</LD>");
}

/// One SFC step built from a run of statements between two wait points.
struct SStep {
    name: String,
    actions: String,
    exit: Exit,
}

enum Exit {
    None,
    Next(String),
    Jump(String, String),
    Timeout {
        cond: String,
        next: Option<String>,
        time: String,
        target: Option<String>,
    },
}

fn sfc_body(g: &mut Gen, items: &[neuron_nir::SeqItem], name: &str, x: &mut Xml) -> bool {
    let Some(ops) = flatten_seq(items) else {
        return false;
    };
    let mut steps: Vec<SStep> = Vec::new();
    let mut cur = SStep {
        name: String::new(),
        actions: String::new(),
        exit: Exit::None,
    };
    let mut base = String::from("Start");
    let mut part = 0;
    let mut open = true;
    let finish = |steps: &mut Vec<SStep>, cur: SStep| steps.push(cur);
    for (step, stmt) in &ops {
        if let Some(s) = step {
            if open
                && cur.actions.is_empty()
                && matches!(cur.exit, Exit::None)
                && cur.name.is_empty()
            {
                cur.name = (*s).to_string();
            } else if open {
                // A step that falls through into the next one: a transition that is always true.
                cur.exit = Exit::Next("TRUE".into());
                finish(
                    &mut steps,
                    std::mem::replace(
                        &mut cur,
                        SStep {
                            name: (*s).to_string(),
                            actions: String::new(),
                            exit: Exit::None,
                        },
                    ),
                );
            } else {
                cur = SStep {
                    name: (*s).to_string(),
                    actions: String::new(),
                    exit: Exit::None,
                };
                open = true;
            }
            base = (*s).to_string();
            part = 0;
            continue;
        }
        let Some(s) = stmt else { continue };
        if !open {
            part += 1;
            cur = SStep {
                name: format!("{base}_{part}"),
                actions: String::new(),
                exit: Exit::None,
            };
            open = true;
        }
        if cur.name.is_empty() {
            cur.name = base.clone();
        }
        match &s.s {
            StmtKind::Until {
                cond,
                timeout,
                on_timeout,
            } => {
                let mut pre = Vec::new();
                let c = g.ex(cond, &mut pre);
                if !pre.is_empty() {
                    return false;
                }
                cur.exit = match timeout {
                    None => Exit::Next(c),
                    Some(t) => Exit::Timeout {
                        cond: c,
                        next: None,
                        time: dur_ns(t).map_or_else(|| "T#0S".into(), iec_time),
                        target: on_timeout.clone(),
                    },
                };
                finish(
                    &mut steps,
                    std::mem::replace(
                        &mut cur,
                        SStep {
                            name: String::new(),
                            actions: String::new(),
                            exit: Exit::None,
                        },
                    ),
                );
                open = false;
            }
            StmtKind::Wait { duration } => {
                let d = dur_ns(duration).map_or_else(|| "T#0S".into(), iec_time);
                cur.exit = Exit::Next(format!("{}.T >= {d}", cur.name));
                finish(
                    &mut steps,
                    std::mem::replace(
                        &mut cur,
                        SStep {
                            name: String::new(),
                            actions: String::new(),
                            exit: Exit::None,
                        },
                    ),
                );
                open = false;
            }
            StmtKind::Jump { target } => {
                cur.exit = Exit::Jump("TRUE".into(), target.clone());
                finish(
                    &mut steps,
                    std::mem::replace(
                        &mut cur,
                        SStep {
                            name: String::new(),
                            actions: String::new(),
                            exit: Exit::None,
                        },
                    ),
                );
                open = false;
            }
            _ => {
                if super::gen::contains_wait(s) {
                    return false;
                }
                let mut w = Writer::new("  ");
                g.stmt(s, &mut w);
                cur.actions.push_str(&w.finish().0);
            }
        }
    }
    if open && !cur.name.is_empty() {
        steps.push(cur);
    }
    // Names of the steps that follow each step.
    let names: Vec<String> = steps.iter().map(|s| s.name.clone()).collect();
    for (i, s) in steps.iter_mut().enumerate() {
        if let Exit::Timeout { next, .. } = &mut s.exit {
            *next = names.get(i + 1).cloned();
        }
    }
    x.open("<SFC>");
    let mut prev: Option<usize> = None;
    let mut row = 0;
    for (i, s) in steps.iter().enumerate() {
        let sid = x.id();
        x.open(&format!(
            "<step localId=\"{sid}\" name=\"{}\" initialStep=\"{}\">",
            esc(&s.name),
            i == 0
        ));
        x.pos(100, row * 60);
        match prev {
            Some(p) => x.conn_in(&[p]),
            None => x.line("<connectionPointIn/>"),
        }
        x.line("<connectionPointOut formalParameter=\"sfc\"/>");
        x.close("</step>");
        row += 1;
        if !s.actions.trim().is_empty() {
            let aid = x.id();
            x.open(&format!("<actionBlock localId=\"{aid}\">"));
            x.pos(200, (row - 1) * 60);
            x.conn_in(&[sid]);
            x.open("<action localId=\"0\" qualifier=\"P1\">");
            x.line(&format!(
                "<inline>{}</inline>",
                st_text(s.actions.trim_end())
            ));
            x.close("</action>");
            x.close("</actionBlock>");
        }
        let transition = |x: &mut Xml, from: usize, cond: &str, row: usize| -> usize {
            let tid = x.id();
            x.open(&format!("<transition localId=\"{tid}\">"));
            x.pos(100, row * 60);
            x.conn_in(&[from]);
            x.line("<connectionPointOut/>");
            x.line(&format!(
                "<condition><inline name=\"\">{}</inline></condition>",
                st_text(cond)
            ));
            x.close("</transition>");
            tid
        };
        let jump = |x: &mut Xml, from: usize, target: &str, row: usize| {
            let jid = x.id();
            x.open(&format!(
                "<jumpStep localId=\"{jid}\" targetName=\"{}\">",
                esc(target)
            ));
            x.pos(100, row * 60);
            x.conn_in(&[from]);
            x.close("</jumpStep>");
        };
        prev = None;
        match &s.exit {
            Exit::None => {}
            Exit::Next(c) => {
                prev = Some(transition(x, sid, c, row));
                row += 1;
            }
            Exit::Jump(c, t) => {
                let tid = transition(x, sid, c, row);
                row += 1;
                jump(x, tid, t, row);
                row += 1;
            }
            Exit::Timeout {
                cond,
                next,
                time,
                target,
            } => {
                let did = x.id();
                x.open(&format!("<selectionDivergence localId=\"{did}\">"));
                x.pos(100, row * 60);
                x.conn_in(&[sid]);
                x.line("<connectionPointOut formalParameter=\"\"/>");
                x.line("<connectionPointOut formalParameter=\"\"/>");
                x.close("</selectionDivergence>");
                row += 1;
                let t1 = transition(x, did, cond, row);
                let t2 = transition(x, did, &format!("{}.T >= {time}", s.name), row);
                row += 1;
                if let Some(n) = next {
                    jump(x, t1, n, row);
                }
                if let Some(t) = target.as_ref().or(next.as_ref()) {
                    jump(x, t2, t, row);
                }
                row += 1;
            }
        }
    }
    x.close("</SFC>");
    let f = g.file.clone();
    g.fid.record(
        &f,
        [0, 0],
        Class::Equivalent,
        &format!("sequence `{name}` as SFC"),
        "steps advance once per scan; a step may end up to two scans earlier or later than in the reference interpreter",
    );
    true
}

impl Emitter for PlcOpen {
    fn manifest(&self) -> Manifest {
        Manifest {
            name: "plcopen",
            language: "PLCopen XML (TC6 2.01)",
            dialects: &["plc"],
            profiles: &["tc6_0201"],
            constructs: &[
                "rung (LD)",
                "set/reset coils",
                "standard function blocks",
                "sequence (SFC)",
                "anything else (ST body)",
            ],
        }
    }

    fn emit(&self, nir: &Nir, _opts: &EmitOptions) -> Result<Package, Vec<Diag>> {
        let mods = plc_modules(nir);
        if mods.is_empty() {
            return Err(no_plc(nir));
        }
        let mut fid = Fidelity::new("emit-plcopen", "plc.plcopen");
        let mut diags = Vec::new();
        let mut x = Xml {
            out: String::new(),
            depth: 0,
            next: 0,
            order: 0,
        };
        let project = nir
            .package
            .clone()
            .unwrap_or_else(|| super::gen::ident(&nir.modules[mods[0]].name));
        x.line("<?xml version=\"1.0\" encoding=\"utf-8\"?>");
        x.open("<project xmlns=\"http://www.plcopen.org/xml/tc6_0201\" xmlns:xhtml=\"http://www.w3.org/1999/xhtml\">");
        x.line(&format!(
            "<fileHeader companyName=\"SYNAPSE Studio\" productName=\"Neuron\" productVersion=\"{}\" creationDateTime=\"2026-01-01T00:00:00\"/>",
            env!("CARGO_PKG_VERSION")
        ));
        x.open(&format!("<contentHeader name=\"{}\">", esc(&project)));
        x.line("<coordinateInfo><fbd><scaling x=\"1\" y=\"1\"/></fbd><ld><scaling x=\"1\" y=\"1\"/></ld><sfc><scaling x=\"1\" y=\"1\"/></sfc></coordinateInfo>");
        x.close("</contentHeader>");
        x.open("<types>");
        x.line("<dataTypes/>");
        x.open("<pous>");
        let mut programs = Vec::new();
        let mut wiring_text = String::from("# Wiring and configuration (PLCopen XML)\n\n");
        for m in mods {
            let module = &nir.modules[m];
            let mut g = Gen::new(nir, m, Style::Iec, Fidelity::new("emit-plcopen", ""));
            let kind = classify(&g, module);
            let mut body = Xml {
                out: String::new(),
                depth: x.depth + 2,
                next: 0,
                order: 0,
            };
            let ok = match kind {
                Body::Ld => {
                    ld_body(&mut g, module, &mut body);
                    true
                }
                Body::Sfc(i) => match &module.items[i].def {
                    ItemDef::Sequence { name, items, .. } => {
                        sfc_body(&mut g, items, name, &mut body)
                    }
                    _ => false,
                },
                Body::St => false,
            };
            if !ok {
                body = Xml {
                    out: String::new(),
                    depth: x.depth + 2,
                    next: 0,
                    order: 0,
                };
                g = Gen::new(nir, m, Style::Iec, Fidelity::new("emit-plcopen", ""));
                let w = g.body("  ");
                let (text, _) = w.finish();
                body.line(&st_text(&text));
                let f = g.file.clone();
                g.fid.record(
                    &f,
                    [0, 0],
                    Class::Exact,
                    "program body",
                    "written as an ST body",
                );
            }
            if ok && kind != Body::St {
                let f = g.file.clone();
                g.fid.record(
                    &f,
                    [0, 0],
                    Class::Equivalent,
                    "safe state",
                    "no statement of this program raises a fault, so the LD or SFC body has no safe-state network; the PLC must still be wired so that a hardwired emergency stop removes power from the outputs",
                );
            }
            diags.append(&mut g.diags);
            wiring_text.push_str(&super::wiring(&g, |a| a.to_string()));
            programs.push((g.name.clone(), g.scan_ns));
            x.open(&format!(
                "<pou name=\"{}\" pouType=\"program\">",
                esc(&g.name)
            ));
            x.open("<interface>");
            let var = |x: &mut Xml, v: &PVar| {
                let addr =
                    v.at.as_ref()
                        .map_or(String::new(), |a| format!(" address=\"{}\"", esc(a)));
                x.open(&format!("<variable name=\"{}\"{addr}>", esc(&v.name)));
                x.line(&format!("<type>{}</type>", xml_type(&v.ty)));
                if let Some(i) = &v.init {
                    x.line(&format!(
                        "<initialValue><simpleValue value=\"{}\"/></initialValue>",
                        esc(i)
                    ));
                }
                x.close("</variable>");
            };
            let mut locals: Vec<PVar> = Vec::new();
            locals.extend(g.inputs.iter().cloned());
            locals.extend(g.outputs.iter().cloned());
            locals.extend(g.memory.iter().filter(|v| !v.retain).cloned());
            locals.extend(g.locals.iter().filter(|v| !v.constant).cloned());
            for b in &g.blocks {
                locals.push(PVar {
                    name: b.name.clone(),
                    ty: g.std_type(&b.kind),
                    at: None,
                    retain: false,
                    init: None,
                    constant: false,
                });
            }
            locals.extend(g.state.iter().cloned());
            if g.state_decls()
                .iter()
                .any(|d| d.starts_with("neuron_fault"))
            {
                for (n, t) in [("neuron_fault", "BOOL"), ("neuron_fault_code", "INT")] {
                    locals.push(PVar {
                        name: n.into(),
                        ty: t.into(),
                        at: None,
                        retain: false,
                        init: None,
                        constant: false,
                    });
                }
            }
            x.open("<localVars>");
            for v in &locals {
                var(&mut x, v);
            }
            x.close("</localVars>");
            let retained: Vec<&PVar> = g.memory.iter().filter(|v| v.retain).collect();
            if !retained.is_empty() {
                x.open("<localVars retain=\"true\">");
                for v in retained {
                    var(&mut x, v);
                }
                x.close("</localVars>");
            }
            let consts: Vec<&PVar> = g.locals.iter().filter(|v| v.constant).collect();
            if !consts.is_empty() {
                x.open("<localVars constant=\"true\">");
                for v in consts {
                    var(&mut x, v);
                }
                x.close("</localVars>");
            }
            x.close("</interface>");
            x.open("<body>");
            x.out.push_str(&body.out);
            x.close("</body>");
            x.close("</pou>");
            fid.merge(g.fid);
        }
        x.close("</pous>");
        x.close("</types>");
        x.open("<instances>");
        x.open("<configurations>");
        x.open("<configuration name=\"neuron_config\">");
        x.open("<resource name=\"neuron_resource\">");
        for (i, (name, ns)) in programs.iter().enumerate() {
            x.open(&format!(
                "<task name=\"task{i}\" interval=\"{}\" priority=\"1\">",
                iec_time(*ns)
            ));
            x.line(&format!(
                "<pouInstance name=\"{}_instance\" typeName=\"{}\"/>",
                esc(name),
                esc(name)
            ));
            x.close("</task>");
        }
        x.close("</resource>");
        x.close("</configuration>");
        x.close("</configurations>");
        x.close("</instances>");
        x.close("</project>");
        if !diags.is_empty() {
            return Err(diags);
        }
        let mut pkg = Package::new(fid);
        pkg.add_text(&format!("{}.xml", super::gen::ident(&project)), x.out);
        pkg.wiring = wiring_text;
        Ok(pkg)
    }
}
