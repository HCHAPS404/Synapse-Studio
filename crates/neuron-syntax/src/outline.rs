//! Compact, stable text outline of a syntax tree (one line per top-level item).
//!
//! Used for golden tests: the outline changes only when the structure of a program
//! changes, not when spans or formatting do.

use crate::ast::*;
use crate::line_index::LineIndex;

/// Renders the structure of `file`: the header and one line per item.
pub fn outline(file: &File, src: &str) -> String {
    let index = LineIndex::new(src);
    let target = file
        .header
        .target
        .as_ref()
        .map_or_else(|| "-".to_string(), Path::dotted);
    let mut out = format!(
        "header {} name={} target={}\n",
        file.header.dialect.name(),
        file.header.name.dotted(),
        target
    );
    for item in &file.items {
        let (line, _) = index.line_col(src, item.span.start);
        let (kind, name, detail) = describe(&item.kind, src);
        let vis = if item.public { " pub" } else { "" };
        out.push_str(&format!("{line:>4} {kind}{vis} {name}{detail}\n"));
    }
    out
}

fn text(e: &Expr, src: &str) -> String {
    e.span
        .text(src)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn describe(kind: &ItemKind, src: &str) -> (&'static str, String, String) {
    let x = |e: &Expr| text(e, src);
    match kind {
        ItemKind::Use { path, names, alias } => (
            "use",
            path.dotted(),
            format!(
                " names={} alias={}",
                names.len(),
                alias.as_ref().map_or("-", |a| a.name.as_str())
            ),
        ),
        ItemKind::Const { name, ty, value } => (
            "const",
            name.name.clone(),
            format!(" typed={} value={}", ty.is_some(), x(value)),
        ),
        ItemKind::Var(v) => (
            "var",
            v.name.name.clone(),
            format!(
                " mutable={} retain={} at={}",
                v.mutable,
                v.retain,
                v.at.as_ref().map_or_else(|| "-".to_string(), x)
            ),
        ),
        ItemKind::Signal { name, init, .. } => (
            "signal",
            name.name.clone(),
            format!(" init={}", init.is_some()),
        ),
        ItemKind::Unit { name, def } => {
            ("unit", name.name.clone(), format!(" = {}", def.display()))
        }
        ItemKind::Event { name, fields, from } => (
            "event",
            name.name.clone(),
            format!(
                " fields={} from={}",
                fields.len(),
                from.as_ref().map_or("-", |f| f.name.as_str())
            ),
        ),
        ItemKind::Struct(d) => (
            "struct",
            d.name.name.clone(),
            format!(" fields={} generics={}", d.members.len(), d.generics.len()),
        ),
        ItemKind::Enum(d) => (
            "enum",
            d.name.name.clone(),
            format!(
                " variants={} generics={}",
                d.members.len(),
                d.generics.len()
            ),
        ),
        ItemKind::Class(c) => (
            "class",
            c.name.name.clone(),
            format!(
                " vars={} methods={} invariants={} interfaces={}",
                c.vars.len(),
                c.methods.len(),
                c.invariants.len(),
                c.interfaces.len()
            ),
        ),
        ItemKind::Interface { name, methods, .. } => (
            "interface",
            name.name.clone(),
            format!(" methods={}", methods.len()),
        ),
        ItemKind::Alias { name, .. } => ("type", name.name.clone(), String::new()),
        ItemKind::Fn(f) => (
            "fn",
            f.sig.name.name.clone(),
            format!(
                " params={} contracts={} ret={}",
                f.sig.params.len(),
                f.contracts.len(),
                f.sig.ret.is_some()
            ),
        ),
        ItemKind::Task(t) => {
            let trigger = match &t.trigger {
                Trigger::Every { period, .. } => format!("every {}", x(period)),
                Trigger::On(e) => format!("on {}", x(e)),
            };
            (
                "task",
                t.name.name.clone(),
                format!(
                    " trigger={trigger} attrs={} stmts={}",
                    t.attrs.len(),
                    t.body.stmts.len()
                ),
            )
        }
        ItemKind::Interrupt {
            source,
            attrs,
            body,
        } => (
            "interrupt",
            x(source),
            format!(" attrs={} stmts={}", attrs.len(), body.stmts.len()),
        ),
        ItemKind::Rule(r) => ("when", x(&r.cond), format!(" stmts={}", r.body.stmts.len())),
        ItemKind::Fsm(f) => (
            "fsm",
            f.name.name.clone(),
            format!(
                " states={} transitions={}",
                f.states.len(),
                f.states.iter().map(|s| s.transitions.len()).sum::<usize>()
            ),
        ),
        ItemKind::Sequence(s) => (
            "sequence",
            s.name.name.clone(),
            format!(" params={} items={}", s.params.len(), s.items.len()),
        ),
        ItemKind::Safety(m) => ("safety", "-".into(), format!(" members={}", m.len())),
        ItemKind::Verify(p) => ("verify", "-".into(), format!(" properties={}", p.len())),
        ItemKind::Test { name, body } => {
            ("test", name.clone(), format!(" stmts={}", body.stmts.len()))
        }
        ItemKind::Io {
            name,
            kind,
            location,
        } => (
            "io",
            name.name.clone(),
            format!(" kind={} at={}", x(kind), x(location)),
        ),
        ItemKind::Clocks(d) => ("clocks", "-".into(), format!(" defs={}", d.len())),
        ItemKind::Uses(p) => (
            "uses",
            p.iter().map(Path::dotted).collect::<Vec<_>>().join(","),
            String::new(),
        ),
        ItemKind::ExternBlock { abi, fns } => {
            ("extern-block", abi.clone(), format!(" fns={}", fns.len()))
        }
        ItemKind::ExternFn { abi, func } => (
            "extern-fn",
            func.sig.name.name.clone(),
            format!(" abi={abi}"),
        ),
        ItemKind::Asm(a) => (
            "asm",
            a.arch.name.clone(),
            format!(" lines={}", a.lines.len()),
        ),
        ItemKind::Scan(e) => ("scan", x(e), String::new()),
        ItemKind::PlcSection { section, vars } => (
            "plc-section",
            format!("{section:?}"),
            format!(" vars={}", vars.len()),
        ),
        ItemKind::Rung { name, body } => (
            "rung",
            name.name.clone(),
            format!(" stmts={}", body.stmts.len()),
        ),
        ItemKind::Clock { name, freq, .. } => {
            ("clock", name.name.clone(), format!(" freq={}", x(freq)))
        }
        ItemKind::Circuit(c) => (
            "circuit",
            c.name.name.clone(),
            format!(" ports={} members={}", c.ports.len(), c.members.len()),
        ),
        ItemKind::Sync { from, to, via } => (
            "sync",
            format!("{} -> {}", x(from), x(to)),
            format!(" via={}", x(via)),
        ),
        ItemKind::Geometry { kind, name, .. } => (
            match kind {
                GeometryKind::Frame => "frame",
                GeometryKind::Tool => "tool",
                GeometryKind::Point => "point",
            },
            name.name.clone(),
            String::new(),
        ),
        ItemKind::Display { name, driver } => (
            "display",
            name.name.clone(),
            format!(" driver={}", x(driver)),
        ),
        ItemKind::Screen {
            name,
            display,
            widgets,
        } => (
            "screen",
            name.name.clone(),
            format!(
                " on={} widgets={}",
                display.as_ref().map_or("-", |d| d.name.as_str()),
                widgets.len()
            ),
        ),
        ItemKind::Theme { name, body } => (
            "theme",
            name.name.clone(),
            format!(" stmts={}", body.stmts.len()),
        ),
        ItemKind::Device { name, ty, address } => (
            "device",
            name.name.clone(),
            format!(
                " type={} at={}",
                ty.dotted(),
                address.as_ref().map_or_else(|| "-".to_string(), x)
            ),
        ),
        ItemKind::Place { module, device } => {
            ("place", module.dotted(), format!(" on={}", device.name))
        }
        ItemKind::Link(l) => (
            "link",
            format!("{} -> {}", x(&l.from), x(&l.to)),
            format!(
                " via={} every={} within={}",
                x(&l.via),
                l.every.as_ref().map_or_else(|| "-".to_string(), x),
                l.within.as_ref().map_or_else(|| "-".to_string(), x)
            ),
        ),
        ItemKind::Tag {
            name, locations, ..
        } => (
            "tag",
            name.name.clone(),
            format!(" locations={}", locations.len()),
        ),
        ItemKind::Zone { name, shape } => {
            ("zone", name.name.clone(), format!(" shape={}", x(shape)))
        }
        ItemKind::Mutex { zone, parties } => (
            "mutex",
            zone.name.clone(),
            format!(" parties={}", parties.len()),
        ),
    }
}
