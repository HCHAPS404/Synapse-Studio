//! Dialect rules and effects (task A6, spec 15 §15.4).
//!
//! Reports `NE0501` (construct not allowed in the dialect), `NE0302` (recursion),
//! `NE0303` (aliasing of `mut` arguments), `NE0305` (references that escape),
//! `NE0403` (loops without a static bound), `NE0404` (waiting in a task or
//! interrupt) and `NE0405` (hard deadline on a soft real-time dialect).

use crate::check::{CallTarget, Checked, FnKey};
use crate::consteval::Consts;
use crate::facts::{BodyKind, FactBase};
use crate::model::{key, FileDiag, Program, Res};
use crate::resolve::Resolved;
use crate::ty::Ty;
use crate::visit::{each_expr, each_stmt, stmt_exprs};
use neuron_syntax::ast::{
    CircuitMember, Dialect, Expr, ExprKind, Item, ItemKind, Lit, Stmt, StmtKind, TaskAttr, Type,
    TypeArg, TypeKind,
};
use neuron_syntax::{Diagnostic, Span};

use Dialect::{Cell, Core, Hdl, Host, Mcu, Plc, Robot, Ui};

struct Ctx<'a> {
    prog: &'a Program,
    resolved: &'a Resolved,
    checked: &'a Checked,
    facts: &'a FactBase<'a>,
    consts: Consts<'a>,
    diags: &'a mut Vec<FileDiag>,
}

impl Ctx<'_> {
    fn err(
        &mut self,
        file: usize,
        code: &'static str,
        msg: impl Into<String>,
        span: Span,
        help: Option<&str>,
    ) {
        let mut d = Diagnostic::error(code, msg, span);
        if let Some(h) = help {
            d = d.with_help(h);
        }
        self.diags.push(FileDiag { file, diag: d });
    }
}

/// Dialects in which an item kind is legal (spec 09 §9.1).
fn legal(item: &ItemKind) -> Option<(&'static str, &'static [Dialect])> {
    const ANY_DEVICE: &[Dialect] = &[Mcu, Plc, Hdl, Robot, Ui, Host, Cell];
    Some(match item {
        ItemKind::Var(_) => ("`var`/`let` state", &[Mcu, Plc, Robot, Ui, Host]),
        ItemKind::Signal { .. } => ("`signal`", ANY_DEVICE),
        ItemKind::Task(_) => ("`task`", &[Mcu, Plc, Robot, Host]),
        ItemKind::Interrupt { .. } => ("`on interrupt`", &[Mcu, Plc]),
        ItemKind::Rule(_) => ("`when` rule", ANY_DEVICE),
        ItemKind::Fsm(_) => ("`fsm`", ANY_DEVICE),
        ItemKind::Sequence(_) => ("`sequence`", &[Mcu, Plc, Robot, Host, Cell]),
        ItemKind::Safety(_) => ("`safety`", &[Mcu, Plc, Robot, Ui, Host, Cell]),
        ItemKind::Verify(_) => ("`verify`", &[Mcu, Plc, Robot, Ui, Host, Cell, Hdl]),
        ItemKind::Event { .. } => ("`event`", &[Mcu, Plc, Robot, Ui, Host, Cell]),
        ItemKind::Io { .. } => ("`io`", &[Mcu, Robot, Host]),
        ItemKind::Clocks(_) | ItemKind::Uses(_) => ("`clocks`/`uses`", &[Mcu]),
        ItemKind::ExternBlock { .. } | ItemKind::ExternFn { .. } | ItemKind::Asm(_) => {
            ("`extern`/`asm`", &[Mcu, Ui, Host])
        }
        ItemKind::Scan(_) => ("`scan`", &[Plc]),
        ItemKind::PlcSection { .. } => ("`inputs`/`outputs`/`memory`", &[Plc]),
        ItemKind::Rung { .. } => ("`rung`", &[Plc]),
        ItemKind::Clock { .. } => ("`clock`", &[Hdl]),
        ItemKind::Circuit(_) => ("`circuit`", &[Hdl]),
        ItemKind::Sync { .. } => ("`sync`", &[Hdl]),
        ItemKind::Geometry { .. } => ("`frame`/`tool`/`point`", &[Robot]),
        ItemKind::Display { .. } => ("`display`", &[Ui]),
        ItemKind::Screen { .. } => ("`screen`", &[Ui]),
        ItemKind::Theme { .. } => ("`theme`", &[Ui]),
        ItemKind::Device { .. } => ("`device`", &[Cell]),
        ItemKind::Place { .. } => ("`place`", &[Cell]),
        ItemKind::Link(_) => ("`link`", &[Cell]),
        ItemKind::Tag { .. } => ("`tag`", &[Cell]),
        ItemKind::Zone { .. } => ("`zone`", &[Cell]),
        ItemKind::Mutex { .. } => ("`mutex`", &[Cell]),
        _ => return None,
    })
}

fn dialect_list(ds: &[Dialect]) -> String {
    ds.iter()
        .map(|d| format!("`{}`", d.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn item_span(item: &Item) -> Span {
    item.span
}

fn type_names<'a>(t: &'a Type, out: &mut Vec<(&'a str, Span)>) {
    match &t.kind {
        TypeKind::Named { path, args } => {
            if path.segments.len() == 1 {
                out.push((path.segments[0].name.as_str(), t.span));
            }
            for a in args {
                if let TypeArg::Type(x) = a {
                    type_names(x, out);
                }
            }
        }
        TypeKind::Array(x, _) | TypeKind::Slice(x) => type_names(x, out),
        TypeKind::Tuple(ts) => {
            for x in ts {
                type_names(x, out);
            }
        }
        TypeKind::Unit => {}
    }
}

fn declared_types(item: &Item) -> Vec<&Type> {
    let mut v: Vec<&Type> = Vec::new();
    match &item.kind {
        ItemKind::Const { ty: Some(t), .. }
        | ItemKind::Signal { ty: t, .. }
        | ItemKind::Alias { ty: t, .. } => v.push(t),
        ItemKind::Tag { ty, .. } => v.push(ty),
        ItemKind::Var(d) => v.extend(d.ty.iter()),
        ItemKind::Struct(def) => v.extend(def.members.iter().map(|f| &f.ty)),
        ItemKind::Enum(def) => v.extend(def.members.iter().flat_map(|m| m.payload.iter())),
        ItemKind::Class(c) => {
            v.extend(c.vars.iter().filter_map(|x| x.ty.as_ref()));
            for m in &c.methods {
                if let ItemKind::Fn(f) = &m.kind {
                    v.extend(f.sig.params.iter().filter_map(|p| p.ty.as_ref()));
                    v.extend(f.sig.ret.iter());
                }
            }
        }
        ItemKind::Fn(f) => {
            v.extend(f.sig.params.iter().filter_map(|p| p.ty.as_ref()));
            v.extend(f.sig.ret.iter());
        }
        ItemKind::PlcSection { vars, .. } => v.extend(vars.iter().filter_map(|x| x.ty.as_ref())),
        ItemKind::Circuit(c) => {
            v.extend(c.ports.iter().map(|p| &p.ty));
            for m in &c.members {
                match &m.node {
                    CircuitMember::Reg { ty, .. } | CircuitMember::Wire { ty, .. } => v.push(ty),
                    CircuitMember::Const { ty: Some(t), .. } => v.push(t),
                    _ => {}
                }
            }
        }
        _ => {}
    }
    v
}

fn check_items(cx: &mut Ctx<'_>) {
    let prog = cx.prog;
    for (mi, f) in prog.files.iter().enumerate() {
        if f.syntax_errors {
            continue;
        }
        let dialect = prog.modules[mi].dialect;
        for item in &f.ast.items {
            if let Some((what, allowed)) = legal(&item.kind) {
                if !allowed.contains(&dialect) {
                    cx.err(
                        mi,
                        "NE0501",
                        format!("{what} is not allowed in `{}` modules", dialect.name()),
                        item_span(item),
                        Some(&format!("it is legal in {}", dialect_list(allowed))),
                    );
                }
            }
            for t in declared_types(item) {
                let mut names = Vec::new();
                type_names(t, &mut names);
                for (n, span) in names {
                    if dialect == Hdl && matches!(n, "f32" | "f64") {
                        cx.err(
                            mi,
                            "NE0501",
                            "floating-point types are not synthesizable in `hdl`",
                            span,
                            Some("use `fixed<I, F>` or an integer type"),
                        );
                    }
                    if dialect != Host && matches!(n, "list" | "map" | "text") {
                        cx.err(
                            mi,
                            "NE0501",
                            format!("`{n}` needs the heap, which only `host` modules have"),
                            span,
                            Some("use an array, `pool<T, N>` or `str<N>`"),
                        );
                    }
                }
            }
            // Slices must not escape (NE0305).
            let slice_in_decl = |t: &Type, what: &str, cx: &mut Ctx<'_>| {
                if matches!(t.kind, TypeKind::Slice(_)) {
                    cx.err(
                        mi,
                        "NE0305",
                        format!("a slice cannot be used as {what}"),
                        t.span,
                        Some("slices only exist as parameters; use an array type"),
                    );
                }
            };
            match &item.kind {
                ItemKind::Fn(func) => {
                    if let Some(r) = &func.sig.ret {
                        slice_in_decl(r, "a return type", cx);
                    }
                }
                ItemKind::Struct(def) => {
                    for fl in &def.members {
                        slice_in_decl(&fl.ty, "a field type", cx);
                    }
                }
                ItemKind::Class(c) => {
                    for v in &c.vars {
                        if let Some(t) = &v.ty {
                            slice_in_decl(t, "a field type", cx);
                        }
                    }
                }
                ItemKind::Var(v) => {
                    if let Some(t) = &v.ty {
                        slice_in_decl(t, "the type of a variable", cx);
                    }
                }
                _ => {}
            }
        }
    }
}

fn check_body_rules(cx: &mut Ctx<'_>) {
    let prog = cx.prog;
    let facts = cx.facts;
    for (bi, b) in facts.bodies.iter().enumerate() {
        let dialect = prog.modules[b.module].dialect;
        let mi = b.module;
        // Statements.
        let mut stmts: Vec<&Stmt> = Vec::new();
        for &s in &b.stmts {
            each_stmt(std::slice::from_ref(s), &mut |x| stmts.push(x));
        }
        for s in &stmts {
            match &s.kind {
                StmtKind::Coil { .. } if dialect != Plc => {
                    cx.err(
                        mi,
                        "NE0501",
                        "`set`/`reset` coils are only allowed in `plc` modules",
                        s.span,
                        Some("assign the variable instead"),
                    );
                }
                StmtKind::Move(_) if !matches!(dialect, Robot | Cell) => {
                    cx.err(
                        mi,
                        "NE0501",
                        "`move` is only allowed in `robot` modules",
                        s.span,
                        None,
                    );
                }
                StmtKind::Asm(_) if !matches!(dialect, Mcu | Ui | Host) => {
                    cx.err(mi, "NE0501", "`asm` is not allowed here", s.span, None);
                }
                _ => {}
            }
            if b.kind == BodyKind::Rung {
                let ok = matches!(
                    &s.kind,
                    StmtKind::Assign { .. }
                        | StmtKind::Coil { .. }
                        | StmtKind::Expr(_)
                        | StmtKind::Pass
                );
                if !ok {
                    cx.err(
                        mi,
                        "NE0501",
                        "a `rung` holds only boolean networks: assignments, `set`/`reset` and function-block calls",
                        s.span,
                        Some("move other logic into a `fn`, a `fsm` or a `sequence`"),
                    );
                }
            }
        }
        // Real literals in hdl.
        if dialect == Hdl {
            let mut exprs: Vec<&Expr> = b.exprs.clone();
            for s in &stmts {
                stmt_exprs(s, &mut |e| exprs.push(e));
            }
            for e in exprs {
                each_expr(e, &mut |x| {
                    if let ExprKind::Lit(Lit::Real(_)) = &x.kind {
                        cx.err(
                            mi,
                            "NE0501",
                            "floating-point literals are not synthesizable in `hdl`",
                            x.span,
                            Some("use an integer or a fixed-point value"),
                        );
                    }
                });
            }
        }
        // Loops.
        if matches!(dialect, Core | Mcu | Plc | Hdl | Robot) {
            for s in &stmts {
                loop_bound(cx, mi, s);
            }
        }
        // Blocking.
        let blocking_ctx = b.kind.is_run_to_completion() && dialect != Host;
        if blocking_ctx {
            let blocks: Vec<(Span, String)> = facts.facts[bi].blocking.clone();
            for (span, what) in blocks {
                cx.err(
                    mi,
                    "NE0404",
                    format!("`{what}` waits, which a {} must not do", kind_name(b.kind)),
                    span,
                    Some("express the wait with an `fsm` state, a `sequence` step or an event"),
                );
            }
            let calls: Vec<(FnKey, Span)> = facts.facts[bi].calls.clone();
            for (callee, span) in calls {
                if let Some(&ci) = facts.fn_index.get(&callee) {
                    if !facts.facts[ci].blocking.is_empty() {
                        cx.err(
                            mi,
                            "NE0404",
                            format!(
                                "`{}` waits and is called from a {}",
                                callee.name,
                                kind_name(b.kind)
                            ),
                            span,
                            None,
                        );
                    }
                }
            }
        }
    }
}

fn kind_name(k: BodyKind) -> &'static str {
    match k {
        BodyKind::Task => "task",
        BodyKind::Interrupt => "interrupt handler",
        BodyKind::Rule | BodyKind::SafetyRule => "rule",
        BodyKind::Fsm => "state machine",
        BodyKind::Rung => "rung",
        BodyKind::SafeState | BodyKind::OnFault => "safety block",
        BodyKind::Circuit => "circuit",
        BodyKind::UiEvent => "UI handler",
        BodyKind::Fn => "function",
        BodyKind::Sequence => "sequence",
        BodyKind::Test => "test",
    }
}

fn loop_bound(cx: &mut Ctx<'_>, mi: usize, s: &Stmt) {
    let consts = &cx.consts;
    let bad: Option<(String, Span)> = match &s.kind {
        StmtKind::For { iter, .. } => {
            let len = cx
                .checked
                .iter_len
                .get(&key(mi, iter.span))
                .copied()
                .flatten();
            if len.is_none() {
                Some((
                    "this `for` loop has no static bound: use a range with constant limits or a fixed-size array".to_string(),
                    iter.span,
                ))
            } else {
                None
            }
        }
        StmtKind::While { max, .. } | StmtKind::Loop { max, .. } => match max {
            None => Some((
                "this loop has no bound: add `max N` with a constant `N`".to_string(),
                s.span,
            )),
            Some(m) => {
                if consts.int(mi, m).is_none() {
                    Some((
                        "the `max` bound of a loop must be a constant".to_string(),
                        m.span,
                    ))
                } else {
                    None
                }
            }
        },
        _ => None,
    };
    if let Some((msg, span)) = bad {
        cx.err(
            mi,
            "NE0403",
            msg,
            span,
            Some("unbounded loops are legal only in `host`"),
        );
    }
}

// ---- recursion ----------------------------------------------------------

fn check_recursion(cx: &mut Ctx<'_>) {
    let facts = cx.facts;
    let n = facts.bodies.len();
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, f) in facts.facts.iter().enumerate() {
        for (callee, _) in &f.calls {
            if let Some(&j) = facts.fn_index.get(callee) {
                if !edges[i].contains(&j) {
                    edges[i].push(j);
                }
            }
        }
    }
    // Tarjan strongly connected components.
    struct T<'e> {
        edges: &'e [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        sccs: Vec<Vec<usize>>,
    }
    fn go(t: &mut T<'_>, v: usize) {
        t.index[v] = Some(t.next);
        t.low[v] = t.next;
        t.next += 1;
        t.stack.push(v);
        t.on[v] = true;
        for k in 0..t.edges[v].len() {
            let w = t.edges[v][k];
            match t.index[w] {
                None => {
                    go(t, w);
                    t.low[v] = t.low[v].min(t.low[w]);
                }
                Some(iw) if t.on[w] => t.low[v] = t.low[v].min(iw),
                _ => {}
            }
        }
        if Some(t.low[v]) == t.index[v] {
            let mut comp = Vec::new();
            while let Some(w) = t.stack.pop() {
                t.on[w] = false;
                comp.push(w);
                if w == v {
                    break;
                }
            }
            t.sccs.push(comp);
        }
    }
    let mut t = T {
        edges: &edges,
        index: vec![None; n],
        low: vec![0; n],
        on: vec![false; n],
        stack: Vec::new(),
        next: 0,
        sccs: Vec::new(),
    };
    for v in 0..n {
        if t.index[v].is_none() && facts.bodies[v].kind == BodyKind::Fn {
            go(&mut t, v);
        }
    }
    let sccs = std::mem::take(&mut t.sccs);
    for comp in sccs {
        let recursive = comp.len() > 1 || edges[comp[0]].contains(&comp[0]);
        if !recursive {
            continue;
        }
        let mut names: Vec<String> = comp.iter().map(|&i| facts.bodies[i].name.clone()).collect();
        names.sort();
        for &i in &comp {
            let b = &facts.bodies[i];
            let dialect = cx.prog.modules[b.module].dialect;
            if dialect == Host {
                continue;
            }
            cx.err(
                b.module,
                "NE0302",
                format!("`{}` is recursive ({})", b.name, names.join(" -> ")),
                b.span,
                Some("recursion is only allowed in `host`; use a loop with a static bound"),
            );
        }
    }
}

// ---- aliasing -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Root {
    Local(usize),
    Item(usize, String),
    SelfValue,
}

fn place_root(cx: &Ctx<'_>, file: usize, e: &Expr) -> Option<(Root, Vec<String>)> {
    let mut path: Vec<String> = Vec::new();
    let mut cur = e;
    loop {
        match &cur.kind {
            ExprKind::Field(b, id) => {
                path.push(id.name.clone());
                cur = b;
            }
            ExprKind::Index(b, _, _) => {
                path.push("[]".to_string());
                cur = b;
            }
            ExprKind::SelfValue => {
                path.reverse();
                return Some((Root::SelfValue, path));
            }
            ExprKind::Name(id) => {
                path.reverse();
                return match cx.resolved.res.get(&key(file, id.span))? {
                    Res::Local(i) => Some((Root::Local(*i), path)),
                    Res::Item { module, name } => Some((Root::Item(*module, name.clone()), path)),
                    _ => None,
                };
            }
            _ => return None,
        }
    }
}

fn overlaps(a: &[String], b: &[String]) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| x == y || x == "[]" || y == "[]")
}

fn check_aliasing(cx: &mut Ctx<'_>) {
    let facts = cx.facts;
    let closure = facts.fn_access_closure();
    for b in &facts.bodies {
        let mi = b.module;
        let mut exprs: Vec<&Expr> = b.exprs.clone();
        let mut stmts: Vec<&Stmt> = Vec::new();
        for &s in &b.stmts {
            each_stmt(std::slice::from_ref(s), &mut |x| stmts.push(x));
        }
        for s in &stmts {
            stmt_exprs(s, &mut |e| exprs.push(e));
        }
        let mut calls: Vec<&Expr> = Vec::new();
        for e in exprs {
            each_expr(e, &mut |x| {
                if matches!(x.kind, ExprKind::Call(..)) {
                    calls.push(x);
                }
            });
        }
        for call in calls {
            let ExprKind::Call(callee, args) = &call.kind else {
                continue;
            };
            let target = cx.checked.calls.get(&key(mi, call.span)).cloned();
            let callee_key = match &target {
                Some(CallTarget::Fn(k)) => k.clone(),
                Some(CallTarget::Method { key: k, .. }) => k.clone(),
                _ => continue,
            };
            // (root, path, mutable, span)
            let mut places: Vec<(Root, Vec<String>, bool, Span)> = Vec::new();
            if let (Some(CallTarget::Method { mut_self, .. }), ExprKind::Field(recv, _)) =
                (&target, &callee.kind)
            {
                if let Some((r, p)) = place_root(cx, mi, recv) {
                    places.push((r, p, *mut_self, recv.span));
                }
            }
            for a in args {
                if let Some((r, p)) = place_root(cx, mi, &a.value) {
                    places.push((r, p, a.mutable, a.span));
                }
            }
            for i in 0..places.len() {
                for j in (i + 1)..places.len() {
                    let (a, bb) = (&places[i], &places[j]);
                    if (a.2 || bb.2) && a.0 == bb.0 && overlaps(&a.1, &bb.1) {
                        let span = bb.3;
                        cx.err(
                            mi,
                            "NE0303",
                            "a `mut` argument aliases another argument of the same call",
                            span,
                            Some("pass distinct places, or copy one of them first"),
                        );
                    }
                }
            }
            if let Some(acc) = closure.get(&callee_key) {
                for (root, _, is_mut, span) in &places {
                    if !*is_mut {
                        continue;
                    }
                    if let Root::Item(m, n) = root {
                        if acc.contains(&(*m, n.clone())) {
                            cx.err(
                                mi,
                                "NE0303",
                                format!("`{n}` is passed as `mut` but `{}` also accesses it directly", callee_key.name),
                                *span,
                                Some("pass the value in, or stop accessing the global inside the function"),
                            );
                        }
                    }
                }
            }
        }
    }
}

// ---- let bindings of slices and deadlines -------------------------------

fn check_slices_and_deadlines(cx: &mut Ctx<'_>) {
    let prog = cx.prog;
    for b in &cx.facts.bodies {
        let mi = b.module;
        let mut stmts: Vec<&Stmt> = Vec::new();
        for &s in &b.stmts {
            each_stmt(std::slice::from_ref(s), &mut |x| stmts.push(x));
        }
        for s in stmts {
            if let StmtKind::Let { pattern, .. } = &s.kind {
                if let neuron_syntax::ast::PatternKind::Bind(id) = &pattern.kind {
                    if let Some(Res::Local(i)) = cx.resolved.res.get(&key(mi, id.span)) {
                        if matches!(cx.checked.local_ty.get(*i), Some(Ty::Slice(_))) {
                            cx.err(
                                mi,
                                "NE0305",
                                "a slice cannot be stored in a variable",
                                s.span,
                                Some("slices only live for the duration of the call; copy into an array"),
                            );
                        }
                    }
                }
            }
        }
    }
    for (mi, f) in prog.files.iter().enumerate() {
        if f.syntax_errors || prog.modules[mi].dialect != Host {
            continue;
        }
        for item in &f.ast.items {
            if let ItemKind::Task(t) = &item.kind {
                for a in &t.attrs {
                    if let TaskAttr::Deadline(_) = &a.node {
                        cx.err(
                            mi,
                            "NE0405",
                            "a hard deadline cannot be guaranteed on a soft real-time `host` target",
                            a.span,
                            Some("place the time-critical logic on an `mcu` or `hdl` device"),
                        );
                    }
                }
            }
        }
    }
}

/// Runs all A6 checks.
pub fn check_rules(
    prog: &Program,
    resolved: &Resolved,
    checked: &Checked,
    facts: &FactBase<'_>,
    diags: &mut Vec<FileDiag>,
) {
    let mut cx = Ctx {
        prog,
        resolved,
        checked,
        facts,
        consts: Consts::new(prog, &resolved.res),
        diags,
    };
    check_items(&mut cx);
    check_body_rules(&mut cx);
    check_recursion(&mut cx);
    check_aliasing(&mut cx);
    check_slices_and_deadlines(&mut cx);
}
