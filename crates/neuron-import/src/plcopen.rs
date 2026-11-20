//! PLCopen XML (TC6 2.01) to a `plc` module.
//!
//! The interface of the program becomes the variable declarations of an ST program, an
//! ST body is read as it is, a Ladder body is walked from each coil back to the left
//! rail (series contacts are `AND`, parallel branches `OR`, negated contacts `NOT`) and
//! written as ST statements, and an SFC body becomes a `sequence`. The ST importer does
//! the rest, so the three kinds of body share one translation.

use crate::st::{self, SfcCond, SfcExit, SfcStep};
use crate::util::syntax;
use neuron_emit::{Class, Diag};
use roxmltree::{Document, Node};

fn child<'a, 'b>(n: Node<'a, 'b>, name: &str) -> Option<Node<'a, 'b>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

fn text_of(n: Node) -> String {
    n.descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect::<String>()
}

fn type_text(var: Node) -> String {
    let Some(t) = child(var, "type") else {
        return "BOOL".into();
    };
    let Some(e) = t.children().find(Node::is_element) else {
        return "BOOL".into();
    };
    match e.tag_name().name() {
        "derived" => e.attribute("name").unwrap_or("BOOL").to_string(),
        "string" => "STRING".into(),
        other => other.to_string(),
    }
}

/// An expression with enough structure to put parentheses only where they are needed.
#[derive(Clone)]
struct Ex {
    text: String,
    or: bool,
}

fn and(a: Option<Ex>, b: Option<Ex>) -> Option<Ex> {
    let wrap = |e: Ex| {
        if e.or {
            format!("({})", e.text)
        } else {
            e.text
        }
    };
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) => Some(Ex {
            text: format!("{} AND {}", wrap(a), wrap(b)),
            or: false,
        }),
    }
}

struct Ld<'a, 'b> {
    doc: &'a Document<'b>,
    diags: Vec<(String, usize)>,
}

impl<'a, 'b> Ld<'a, 'b> {
    fn node(&self, id: &str) -> Option<Node<'a, 'b>> {
        self.doc
            .descendants()
            .find(|n| n.is_element() && n.attribute("localId") == Some(id))
    }

    /// What reaches the input of `n`: `None` is the left rail (always true).
    fn inputs(&mut self, n: Node<'a, 'b>, depth: usize) -> Result<Option<Ex>, String> {
        let mut parts: Vec<Option<Ex>> = Vec::new();
        for cp in n.children().filter(|c| c.has_tag_name("connectionPointIn")) {
            for c in cp.children().filter(|c| c.has_tag_name("connection")) {
                let Some(id) = c.attribute("refLocalId") else {
                    continue;
                };
                let Some(src) = self.node(id) else {
                    return Err(format!("a connection refers to the missing element {id}"));
                };
                parts.push(self.out(src, c.attribute("formalParameter"), depth + 1)?);
            }
        }
        let mut it = parts.into_iter();
        let Some(first) = it.next() else {
            return Ok(None);
        };
        let mut acc = first;
        for p in it {
            acc = match (acc, p) {
                (Some(a), Some(b)) => Some(Ex {
                    text: format!("{} OR {}", a.text, b.text),
                    or: true,
                }),
                _ => None,
            };
        }
        Ok(acc)
    }

    /// The value an element puts on its output.
    fn out(
        &mut self,
        n: Node<'a, 'b>,
        param: Option<&str>,
        depth: usize,
    ) -> Result<Option<Ex>, String> {
        if depth > 500 {
            return Err("the ladder diagram has a loop".into());
        }
        match n.tag_name().name() {
            "leftPowerRail" => Ok(None),
            "contact" => {
                let before = self.inputs(n, depth)?;
                let var = child(n, "variable").map(text_of).unwrap_or_default();
                let var = var.trim().to_string();
                let edge = n.attribute("edge").unwrap_or("none");
                if edge != "none" {
                    self.diags.push((
                        format!("a contact with the {edge} edge of `{var}`"),
                        n.range().start,
                    ));
                }
                let neg = n.attribute("negated") == Some("true");
                let term = Ex {
                    text: if neg { format!("NOT {var}") } else { var },
                    or: false,
                };
                Ok(and(before, Some(term)))
            }
            "inVariable" => {
                let e = child(n, "expression").map(text_of).unwrap_or_default();
                Ok(Some(Ex {
                    text: format!("({})", e.trim()),
                    or: false,
                }))
            }
            "block" => {
                let inst = n.attribute("instanceName").unwrap_or("");
                Ok(Some(Ex {
                    text: format!("{inst}.{}", param.unwrap_or("Q")),
                    or: false,
                }))
            }
            other => Err(format!("the element `{other}` in a ladder diagram")),
        }
    }
}

fn ld_text<'a, 'b>(
    body: Node<'a, 'b>,
    doc: &'a Document<'b>,
    notes: &mut Vec<(String, usize)>,
) -> Result<String, String> {
    let mut ld = Ld {
        doc,
        diags: Vec::new(),
    };
    let mut out = String::new();
    for e in body.children().filter(Node::is_element) {
        match e.tag_name().name() {
            "comment" => {
                let t = text_of(e);
                out.push_str(&format!("(* {} *)\n", t.trim().replace("*)", "* )")));
            }
            "coil" => {
                let var = child(e, "variable").map(text_of).unwrap_or_default();
                let var = var.trim().to_string();
                let before = ld.inputs(e, 0)?;
                let neg = e.attribute("negated") == Some("true");
                let value = match before {
                    None => "TRUE".to_string(),
                    Some(x) => x.text,
                };
                match e.attribute("storage").unwrap_or("none") {
                    "set" | "reset" if neg => {
                        notes.push((
                            format!("a negated set or reset coil `{var}`"),
                            e.range().start,
                        ));
                    }
                    s @ ("set" | "reset") => {
                        out.push_str(&format!(
                            "IF {value} THEN\n  {var} := {};\nEND_IF;\n",
                            if s == "set" { "TRUE" } else { "FALSE" }
                        ));
                    }
                    "none" => {
                        let v = if neg { format!("NOT ({value})") } else { value };
                        out.push_str(&format!("{var} := {v};\n"));
                    }
                    other => notes.push((format!("a `{other}` coil `{var}`"), e.range().start)),
                }
            }
            "block" => {
                let inst = e.attribute("instanceName").unwrap_or("");
                let mut args = Vec::new();
                if let Some(iv) = child(e, "inputVariables") {
                    for v in iv.children().filter(|c| c.has_tag_name("variable")) {
                        let Some(p) = v.attribute("formalParameter") else {
                            continue;
                        };
                        let x = ld.inputs(v, 0)?;
                        args.push(format!(
                            "{p} := {}",
                            x.map_or_else(|| "TRUE".to_string(), |x| x.text)
                        ));
                    }
                }
                out.push_str(&format!("{inst}({});\n", args.join(", ")));
            }
            _ => {}
        }
    }
    notes.append(&mut ld.diags);
    Ok(out)
}

/// The time in a condition such as `Stir.T >= T#5S`.
fn elapsed(code: &str) -> Option<i64> {
    let (l, r) = code.split_once(">=")?;
    if !l.trim().to_ascii_uppercase().ends_with(".T") {
        return None;
    }
    let src = format!("PROGRAM a\nx := {};\nEND_PROGRAM\n", r.trim());
    match neuron_st::parse(&src).ok()?.pous.first()?.body.first()? {
        neuron_st::ast::Stmt::Assign(_, neuron_st::ast::Expr::Time(ns), _) => Some(*ns),
        _ => None,
    }
}

fn st_of(n: Node) -> String {
    // `<ST><xhtml:p>code</xhtml:p></ST>` inside `inline`, `condition` or `action`.
    n.descendants()
        .find(|d| d.is_element() && d.tag_name().name() == "ST")
        .map(|s| text_of(s).trim().to_string())
        .unwrap_or_default()
}

fn refs(n: Node) -> Vec<String> {
    n.descendants()
        .filter(|c| c.has_tag_name("connection"))
        .filter_map(|c| c.attribute("refLocalId").map(str::to_string))
        .collect()
}

fn sfc_steps(sfc: Node) -> Vec<SfcStep> {
    let els: Vec<Node> = sfc.children().filter(Node::is_element).collect();
    let id = |n: &Node| n.attribute("localId").unwrap_or("").to_string();
    let mut steps = Vec::new();
    for s in els.iter().filter(|e| e.has_tag_name("step")) {
        let sid = id(s);
        let actions: Vec<String> = els
            .iter()
            .filter(|e| e.has_tag_name("actionBlock") && refs(**e).contains(&sid))
            .map(|e| st_of(*e))
            .collect();
        let cond_of = |t: &Node| {
            let c = child(*t, "condition").map(st_of).unwrap_or_default();
            if c.trim().eq_ignore_ascii_case("TRUE") {
                SfcCond::True
            } else if let Some(d) = elapsed(&c) {
                SfcCond::Elapsed(d)
            } else {
                SfcCond::Text(c)
            }
        };
        let jump_after = |tid: &str| {
            els.iter()
                .find(|e| e.has_tag_name("jumpStep") && refs(**e).iter().any(|r| r == tid))
                .and_then(|j| j.attribute("targetName"))
                .map(str::to_string)
        };
        let div = els
            .iter()
            .find(|e| e.has_tag_name("selectionDivergence") && refs(**e).contains(&sid));
        let exit = if let Some(d) = div {
            let did = id(d);
            let ts: Vec<&Node> = els
                .iter()
                .filter(|e| e.has_tag_name("transition") && refs(**e).contains(&did))
                .collect();
            match (ts.first(), ts.get(1)) {
                (Some(t1), Some(t2)) => match (cond_of(t1), cond_of(t2)) {
                    (SfcCond::Text(c), SfcCond::Elapsed(ns)) => SfcExit::Timeout {
                        cond: c,
                        ns,
                        target: jump_after(&id(t2)),
                    },
                    _ => SfcExit::None,
                },
                _ => SfcExit::None,
            }
        } else if let Some(t) = els
            .iter()
            .find(|e| e.has_tag_name("transition") && refs(**e).contains(&sid))
        {
            SfcExit::Transition {
                cond: cond_of(t),
                jump: jump_after(&id(t)),
            }
        } else {
            SfcExit::None
        };
        steps.push(SfcStep {
            name: s.attribute("name").unwrap_or("step").to_string(),
            actions: actions.join("\n"),
            exit,
        });
    }
    steps
}

/// Imports a PLCopen XML project.
pub(crate) fn import(
    file: &str,
    name: &str,
    xml: &str,
    scan_ns: Option<i64>,
) -> Result<st::Out, Vec<Diag>> {
    let doc = Document::parse(xml).map_err(|e| {
        let pos = e.pos();
        vec![syntax(
            file,
            0,
            1,
            &format!("PLCopen XML: {e} (line {}, column {})", pos.row, pos.col),
        )]
    })?;
    let root = doc.root_element();
    if root.tag_name().name() != "project" {
        return Err(vec![syntax(
            file,
            0,
            1,
            "PLCopen XML: the root element is not `project`",
        )]);
    }
    let Some(pou) = doc.descendants().find(|n| {
        n.has_tag_name("pou") && n.attribute("pouType").unwrap_or("program") == "program"
    }) else {
        return Err(vec![syntax(
            file,
            0,
            1,
            "PLCopen XML: the project has no program POU",
        )]);
    };
    let pname = pou.attribute("name").unwrap_or("program");
    let mut text = format!("PROGRAM {pname}\n");
    if let Some(iface) = child(pou, "interface") {
        for sec in iface.children().filter(Node::is_element) {
            let kw = match sec.tag_name().name() {
                "inputVars" => "VAR_INPUT",
                "outputVars" => "VAR_OUTPUT",
                "inOutVars" => "VAR_IN_OUT",
                "localVars" | "tempVars" | "externalVars" => "VAR",
                _ => continue,
            };
            let retain = sec.attribute("retain") == Some("true");
            text.push_str(&format!("{kw}{}\n", if retain { " RETAIN" } else { "" }));
            for v in sec.children().filter(|c| c.has_tag_name("variable")) {
                let at = v
                    .attribute("address")
                    .map(|a| format!(" AT {a}"))
                    .unwrap_or_default();
                let init = child(v, "initialValue")
                    .and_then(|i| child(i, "simpleValue"))
                    .and_then(|s| s.attribute("value"))
                    .map(|x| format!(" := {x}"))
                    .unwrap_or_default();
                text.push_str(&format!(
                    "  {}{at} : {}{init};\n",
                    v.attribute("name").unwrap_or("x"),
                    type_text(v)
                ));
            }
            text.push_str("END_VAR\n");
        }
    }
    let mut notes: Vec<(String, usize)> = Vec::new();
    let mut sfc = None;
    if let Some(body) = child(pou, "body") {
        if let Some(b) = body.children().find(Node::is_element) {
            match b.tag_name().name() {
                "ST" => text.push_str(&text_of(b)),
                "LD" => match ld_text(b, &doc, &mut notes) {
                    Ok(t) => text.push_str(&t),
                    Err(m) => {
                        return Err(vec![syntax(
                            file,
                            b.range().start,
                            1,
                            &format!("PLCopen XML: {m}"),
                        )])
                    }
                },
                "SFC" => sfc = Some(sfc_steps(b)),
                other => notes.push((format!("a `{other}` body"), b.range().start)),
            }
        }
    }
    text.push_str("\nEND_PROGRAM\n");
    // The task interval of the configuration.
    let interval = doc
        .descendants()
        .find(|n| n.has_tag_name("task") && n.attribute("interval").is_some())
        .and_then(|t| {
            let inst = t
                .descendants()
                .find(|c| c.has_tag_name("pouInstance"))
                .and_then(|c| c.attribute("typeName"))
                .unwrap_or(pname);
            t.attribute("interval")
                .map(|i| (inst.to_string(), i.to_string()))
        });
    if let Some((inst, iv)) = interval {
        text.push_str(&format!(
            "CONFIGURATION c\n  RESOURCE r ON PLC\n    TASK t (INTERVAL := {iv}, PRIORITY := 1);\n    PROGRAM i WITH t : {inst};\n  END_RESOURCE\nEND_CONFIGURATION\n"
        ));
    }
    let mut out = st::import(st::Input {
        file,
        name,
        text: &text,
        base: 0,
        scan_ns,
        tool: "import-plcopen",
        sfc,
    })?;
    for (what, pos) in notes {
        let span = [pos, pos + 1];
        out.diags.push(Diag {
            code: "NE1002",
            message: format!("{what} has no Neuron equivalent yet"),
            help: Some(
                "write that part in Neuron by hand; the rest of the program was imported".into(),
            ),
            file: file.to_string(),
            span,
        });
        out.fid.record(
            file,
            span,
            Class::Unsupported,
            &what,
            "not imported: it has no Neuron equivalent yet",
        );
    }
    Ok(out)
}
