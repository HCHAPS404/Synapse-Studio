//! Name resolution inside bodies (task A4, spec 15 §15.2).
//!
//! Walks every item of every module, binds local names, and records for each
//! identifier what it refers to. Reports `NE0101` (unknown name), `NE0102`
//! (duplicate in scope), `NE0105` (private item) and `NE0206` (unknown unit).

use crate::model::{key, FileDiag, Program, Res, SpanKey, SymKind};
use neuron_syntax::ast::{
    Arg, Block, CircuitMember, ClassDecl, Dialect, Expr, ExprKind, FnBody, FnDecl, FsmDecl,
    GenericParam, Ident, Item, ItemKind, Path, Pattern, PatternKind, PortDir, Property, Rule,
    SafetyMember, SeqItem, SequenceDecl, Stmt, StmtKind, TaskAttr, TaskDecl, Trigger, Type,
    TypeArg, TypeKind, UnitExpr, VarDecl, Widget,
};
use neuron_syntax::Diagnostic;
use std::collections::{BTreeMap, BTreeSet};

/// How a local binding was introduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    /// Function or sequence parameter.
    Param {
        /// Declared `mut`.
        mutable: bool,
    },
    /// `let` (immutable).
    Let,
    /// `var` (mutable), registers, wires and output ports.
    Var,
    /// Introduced by a pattern in `match`.
    Pattern,
    /// Loop variable.
    For,
    /// Lambda parameter.
    Lambda,
    /// Input port of a circuit (read only).
    InPort,
}

/// A local binding.
#[derive(Debug, Clone)]
pub struct LocalInfo {
    /// Span of the declaring identifier.
    pub span: neuron_syntax::Span,
    /// Kind.
    pub kind: LocalKind,
}

/// Output of name resolution.
#[derive(Debug, Default)]
pub struct Resolved {
    /// What every resolved identifier refers to.
    pub res: BTreeMap<SpanKey, Res>,
    /// All local bindings.
    pub locals: Vec<LocalInfo>,
}

/// Resolves all names of all analyzable files.
pub fn resolve_program(prog: &Program, diags: &mut Vec<FileDiag>) -> Resolved {
    let mut out = Resolved::default();
    for (mi, f) in prog.files.iter().enumerate() {
        if f.syntax_errors {
            continue;
        }
        let mut w = Walker {
            prog,
            file: mi,
            module: mi,
            dialect: prog.modules[mi].dialect,
            res: &mut out.res,
            locals: &mut out.locals,
            diags,
            scopes: Vec::new(),
            generics: Vec::new(),
            steps: Vec::new(),
            verify_states: None,
            vocab: 0,
        };
        w.module();
    }
    out
}

struct Walker<'a> {
    prog: &'a Program,
    file: usize,
    module: usize,
    dialect: Dialect,
    res: &'a mut BTreeMap<SpanKey, Res>,
    locals: &'a mut Vec<LocalInfo>,
    diags: &'a mut Vec<FileDiag>,
    scopes: Vec<BTreeMap<String, usize>>,
    generics: Vec<Vec<String>>,
    steps: Vec<BTreeSet<String>>,
    verify_states: Option<BTreeMap<String, Vec<String>>>,
    vocab: u32,
}

/// Edit distance with adjacent transpositions (optimal string alignment).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[a.len()][b.len()]
}

impl Walker<'_> {
    fn err(&mut self, code: &'static str, msg: impl Into<String>, span: neuron_syntax::Span) {
        self.diags.push(FileDiag {
            file: self.file,
            diag: Diagnostic::error(code, msg, span),
        });
    }

    fn err_help(
        &mut self,
        code: &'static str,
        msg: impl Into<String>,
        span: neuron_syntax::Span,
        help: Option<String>,
    ) {
        let mut d = Diagnostic::error(code, msg, span);
        if let Some(h) = help {
            d = d.with_help(h);
        }
        self.diags.push(FileDiag {
            file: self.file,
            diag: d,
        });
    }

    fn record(&mut self, id: &Ident, r: Res) {
        self.res.insert(key(self.file, id.span), r);
    }

    fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, id: &Ident, kind: LocalKind) -> usize {
        let idx = self.locals.len();
        self.locals.push(LocalInfo {
            span: id.span,
            kind,
        });
        if id.name != "_" {
            let dup = self
                .scopes
                .last()
                .and_then(|s| s.get(&id.name))
                .map(|&prev| self.locals[prev].span);
            if let Some(prev) = dup {
                self.diags.push(FileDiag {
                    file: self.file,
                    diag: Diagnostic::error(
                        "NE0102",
                        format!("`{}` is already defined in this scope", id.name),
                        id.span,
                    )
                    .with_label(prev, "first definition here"),
                });
            }
            if let Some(s) = self.scopes.last_mut() {
                s.insert(id.name.clone(), idx);
            }
        }
        self.record(id, Res::Local(idx));
        idx
    }

    fn lookup_local(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn suggest(&self, name: &str) -> Option<String> {
        let limit = (name.chars().count() / 3).max(1);
        let mut best: Option<(usize, String)> = None;
        let module = &self.prog.modules[self.module];
        let locals = self.scopes.iter().flat_map(|s| s.keys());
        let syms = module.symbols.keys();
        let imps = module.imports.keys();
        for c in locals.chain(syms).chain(imps) {
            let d = levenshtein(name, c);
            let better = match &best {
                Some((bd, _)) => d < *bd,
                None => true,
            };
            if d > 0 && d <= limit && better {
                best = Some((d, c.clone()));
            }
        }
        best.map(|(_, c)| format!("did you mean `{c}`?"))
    }

    /// Resolves a name in value or type position and records the result.
    fn resolve_name(&mut self, id: &Ident) -> Option<Res> {
        if let Some(states) = &self.verify_states {
            if self.lookup_local(&id.name).is_none() {
                if let Some(fsms) = states.get(&id.name) {
                    if fsms.len() == 1 {
                        let r = Res::State {
                            fsm: fsms[0].clone(),
                            state: id.name.clone(),
                        };
                        self.record(id, r.clone());
                        return Some(r);
                    }
                    self.err(
                        "NE0101",
                        format!(
                            "state `{}` exists in several state machines; write `{}.{}`",
                            id.name, fsms[0], id.name
                        ),
                        id.span,
                    );
                    return None;
                }
            }
        }
        let found = self
            .lookup_local(&id.name)
            .map(Res::Local)
            .or_else(|| self.prog.lookup(self.module, &id.name));
        match found {
            Some(r) => {
                self.record(id, r.clone());
                Some(r)
            }
            None => {
                let is_generic = self.generics.iter().any(|g| g.contains(&id.name));
                if self.vocab == 0 && !is_generic {
                    let help = self.suggest(&id.name);
                    self.err_help(
                        "NE0101",
                        format!("unknown name `{}`", id.name),
                        id.span,
                        help,
                    );
                }
                None
            }
        }
    }

    fn module_member(&mut self, m: usize, id: &Ident) {
        match self
            .prog
            .modules
            .get(m)
            .and_then(|mm| mm.symbols.get(&id.name))
        {
            None => {
                if self.vocab == 0 {
                    let mname = self.prog.modules[m].name.clone();
                    self.err(
                        "NE0101",
                        format!("module `{mname}` has no item `{}`", id.name),
                        id.span,
                    );
                }
            }
            Some(sym) => {
                if !sym.public && m != self.module {
                    let mname = self.prog.modules[m].name.clone();
                    self.err(
                        "NE0105",
                        format!("`{}` is private to module `{mname}`", id.name),
                        id.span,
                    );
                } else {
                    self.record(
                        id,
                        Res::Item {
                            module: m,
                            name: id.name.clone(),
                        },
                    );
                }
            }
        }
    }

    // ---- module ---------------------------------------------------------

    fn module(&mut self) {
        let items = &self.prog.files[self.file].ast.items;
        self.push_scope();
        // `scopes[0]` is only used for names introduced by sections that live at
        // module level in no other way; module symbols are looked up in the model.
        for item in items {
            self.item(item);
        }
        self.pop_scope();
    }

    fn item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Use { .. } => {}
            ItemKind::Const { ty, value, .. } => {
                if let Some(t) = ty {
                    self.type_(t);
                }
                self.expr(value);
            }
            ItemKind::Var(v) => self.module_var(v),
            ItemKind::Signal { ty, init, .. } => {
                self.type_(ty);
                if let Some(e) = init {
                    self.expr(e);
                }
            }
            ItemKind::Unit { def, .. } => self.unit_check(def),
            ItemKind::Event { fields, from, .. } => {
                for (_, t) in fields {
                    if let Some(t) = t {
                        self.type_(t);
                    }
                }
                if let Some(d) = from {
                    self.resolve_name(d);
                }
            }
            ItemKind::Struct(def) => {
                self.generics_scope(&def.generics);
                for f in &def.members {
                    self.type_(&f.ty);
                    if let Some(e) = &f.default {
                        self.expr(e);
                    }
                }
                self.generics.pop();
            }
            ItemKind::Enum(def) => {
                self.generics_scope(&def.generics);
                if let Some(t) = &def.repr {
                    self.type_(t);
                }
                for v in &def.members {
                    for t in &v.payload {
                        self.type_(t);
                    }
                    if let Some(e) = &v.value {
                        self.expr(e);
                    }
                }
                self.generics.pop();
            }
            ItemKind::Class(c) => self.class(c),
            ItemKind::Interface {
                generics, methods, ..
            } => {
                self.generics_scope(generics);
                for m in methods {
                    self.generics_scope(&m.generics);
                    for p in &m.params {
                        if let Some(t) = &p.ty {
                            self.type_(t);
                        }
                    }
                    if let Some(t) = &m.ret {
                        self.type_(t);
                    }
                    self.generics.pop();
                }
                self.generics.pop();
            }
            ItemKind::Alias { generics, ty, .. } => {
                self.generics_scope(generics);
                self.type_(ty);
                self.generics.pop();
            }
            ItemKind::Fn(f) => self.fn_decl(f),
            ItemKind::Task(t) => self.task(t),
            ItemKind::Interrupt {
                source,
                attrs,
                body,
            } => {
                self.expr(source);
                for a in attrs {
                    self.task_attr(&a.node);
                }
                self.block(body);
            }
            ItemKind::Rule(r) => self.rule(r),
            ItemKind::Fsm(f) => self.fsm(f),
            ItemKind::Sequence(s) => self.sequence(s),
            ItemKind::Safety(members) => {
                for m in members {
                    match &m.node {
                        SafetyMember::Rate(e)
                        | SafetyMember::Watchdog(e)
                        | SafetyMember::Reset(e) => {
                            self.expr(e);
                        }
                        SafetyMember::SafeState(b) => self.block(b),
                        SafetyMember::Rule(r) => self.rule(r),
                        SafetyMember::OnFault { body, .. } => self.block(body),
                    }
                }
            }
            ItemKind::Verify(props) => self.verify(props),
            ItemKind::Test { body, .. } => self.block(body),
            ItemKind::Io { kind, location, .. } => {
                self.vocab += 1;
                self.expr(kind);
                self.expr(location);
                self.vocab -= 1;
            }
            ItemKind::Clocks(defs) => {
                for c in defs {
                    self.expr(&c.value);
                    if let Some(f) = &c.from {
                        self.vocab += 1;
                        self.expr(f);
                        self.vocab -= 1;
                    }
                }
            }
            ItemKind::Uses(_) | ItemKind::Asm(_) => {}
            ItemKind::ExternBlock { fns, .. } => {
                for f in fns {
                    for p in &f.params {
                        if let Some(t) = &p.ty {
                            self.type_(t);
                        }
                    }
                    if let Some(t) = &f.ret {
                        self.type_(t);
                    }
                }
            }
            ItemKind::ExternFn { func, .. } => self.fn_decl(func),
            ItemKind::Scan(e) => self.expr(e),
            ItemKind::PlcSection { vars, .. } => {
                for v in vars {
                    self.module_var(v);
                }
            }
            ItemKind::Rung { body, .. } => self.block(body),
            ItemKind::Clock { freq, pin, .. } => {
                self.expr(freq);
                if let Some(p) = pin {
                    self.vocab += 1;
                    self.expr(p);
                    self.vocab -= 1;
                }
            }
            ItemKind::Circuit(c) => self.circuit(c),
            ItemKind::Sync { from, to, via } => {
                self.vocab += 1;
                self.expr(from);
                self.expr(to);
                self.expr(via);
                self.vocab -= 1;
            }
            ItemKind::Geometry { ty, value, .. } => {
                if let Some(t) = ty {
                    self.type_(t);
                }
                self.expr(value);
            }
            ItemKind::Display { driver, .. } => {
                self.vocab += 1;
                self.expr(driver);
                self.vocab -= 1;
            }
            ItemKind::Screen {
                display, widgets, ..
            } => {
                if let Some(d) = display {
                    self.expect_kind(d, SymKind::Display, "display");
                }
                for w in widgets {
                    self.widget(w);
                }
            }
            ItemKind::Theme { .. } => {}
            ItemKind::Device { address, .. } => {
                if let Some(a) = address {
                    self.vocab += 1;
                    self.expr(a);
                    self.vocab -= 1;
                }
            }
            ItemKind::Place { module, device } => {
                let segs: Vec<String> = module.segments.iter().map(|s| s.name.clone()).collect();
                if self.prog.find_module(&segs).is_none() {
                    self.err(
                        "NE0101",
                        format!("module `{}` not found in this package", module.dotted()),
                        module.span,
                    );
                }
                self.expect_kind(device, SymKind::Device, "device");
            }
            ItemKind::Link(l) => {
                self.expr(&l.from);
                self.expr(&l.to);
                self.vocab += 1;
                self.expr(&l.via);
                self.vocab -= 1;
                if let Some(e) = &l.every {
                    self.expr(e);
                }
                if let Some(e) = &l.within {
                    self.expr(e);
                }
            }
            ItemKind::Tag { ty, locations, .. } => {
                self.type_(ty);
                for l in locations {
                    self.expr(l);
                }
            }
            ItemKind::Zone { shape, .. } => {
                self.vocab += 1;
                self.expr(shape);
                self.vocab -= 1;
            }
            ItemKind::Mutex { zone, parties } => {
                self.expect_kind(zone, SymKind::Zone, "zone");
                for p in parties {
                    self.expr(p);
                }
            }
        }
    }

    fn expect_kind(&mut self, id: &Ident, kind: SymKind, what: &str) {
        match self.prog.symbol(self.module, &id.name) {
            Some(s) if s.kind == kind => self.record(
                id,
                Res::Item {
                    module: self.module,
                    name: id.name.clone(),
                },
            ),
            _ => self.err(
                "NE0101",
                format!("`{}` is not a {what} of this file", id.name),
                id.span,
            ),
        }
    }

    fn module_var(&mut self, v: &VarDecl) {
        if let Some(t) = &v.ty {
            self.type_(t);
        }
        if let Some(e) = &v.init {
            self.expr(e);
        }
        if let Some(e) = &v.at {
            self.vocab += 1;
            self.expr(e);
            self.vocab -= 1;
        }
    }

    // ---- generics, types, units ----------------------------------------

    fn generics_scope(&mut self, gs: &[GenericParam]) {
        let mut names = Vec::new();
        for g in gs {
            match g {
                GenericParam::Type { name, .. } | GenericParam::Const { name, .. } => {
                    names.push(name.name.clone());
                }
            }
        }
        self.generics.push(names);
        for g in gs {
            match g {
                GenericParam::Type { bounds, .. } => {
                    for b in bounds {
                        self.bound(b);
                    }
                }
                GenericParam::Const { ty, .. } => self.type_(ty),
            }
        }
    }

    fn bound(&mut self, p: &Path) {
        let Some(first) = p.segments.first() else {
            return;
        };
        if p.segments.len() == 1 && crate::prelude::BOUND_NAMES.contains(&first.name.as_str()) {
            return;
        }
        self.type_path(p);
    }

    fn type_path(&mut self, p: &Path) {
        let Some(first) = p.segments.first() else {
            return;
        };
        if p.segments.len() == 1 && self.generics.iter().any(|g| g.contains(&first.name)) {
            return;
        }
        match self.resolve_name(first) {
            Some(Res::Item { module, name }) if p.segments.len() == 1 => {
                let ok = self
                    .prog
                    .symbol(module, &name)
                    .is_some_and(|s| s.kind.is_type() || s.kind == SymKind::Const);
                if !ok {
                    self.err(
                        "NE0101",
                        format!("`{}` is not a type", first.name),
                        first.span,
                    );
                }
            }
            Some(Res::Module(m)) => {
                if let Some(second) = p.segments.get(1) {
                    self.module_member(m, second);
                }
            }
            _ => {}
        }
    }

    fn type_(&mut self, t: &Type) {
        match &t.kind {
            TypeKind::Named { path, args } => {
                self.type_path(path);
                for a in args {
                    match a {
                        TypeArg::Type(ty) => self.type_(ty),
                        TypeArg::Expr(e) => self.expr(e),
                    }
                }
            }
            TypeKind::Array(inner, len) => {
                self.type_(inner);
                self.expr(len);
            }
            TypeKind::Slice(inner) => self.type_(inner),
            TypeKind::Tuple(ts) => {
                for x in ts {
                    self.type_(x);
                }
            }
            TypeKind::Unit => {}
        }
        if let Some(u) = &t.unit {
            self.unit_check(u);
        }
    }

    fn unit_check(&mut self, u: &UnitExpr) {
        if let Err(name) = self.prog.units.resolve(u) {
            self.err("NE0206", format!("unknown unit `{name}`"), u.span);
        }
    }

    // ---- functions and classes -----------------------------------------

    fn fn_decl(&mut self, f: &FnDecl) {
        self.generics_scope(&f.sig.generics);
        self.push_scope();
        for p in &f.sig.params {
            if let Some(t) = &p.ty {
                self.type_(t);
            }
            if let Some(e) = &p.default {
                self.expr(e);
            }
        }
        for p in &f.sig.params {
            if p.name.name == "self" {
                continue;
            }
            self.declare(&p.name, LocalKind::Param { mutable: p.mutable });
        }
        if let Some(t) = &f.sig.ret {
            self.type_(t);
        }
        for c in &f.contracts {
            self.expr(&c.cond);
        }
        match &f.body {
            FnBody::Block(b) => {
                for c in &b.contracts {
                    self.expr(&c.cond);
                }
                for s in &b.stmts {
                    self.stmt(s);
                }
            }
            FnBody::Expr(e) => self.expr(e),
        }
        self.pop_scope();
        self.generics.pop();
    }

    fn class(&mut self, c: &ClassDecl) {
        self.generics_scope(&c.generics);
        for i in &c.interfaces {
            if let Some(first) = i.segments.first() {
                if let Some(Res::Item { module, name }) = self.resolve_name(first) {
                    let ok = self
                        .prog
                        .symbol(module, &name)
                        .is_some_and(|s| s.kind == SymKind::Interface);
                    if !ok {
                        self.err(
                            "NE0101",
                            format!("`{}` is not an interface", first.name),
                            first.span,
                        );
                    }
                }
            }
        }
        for v in &c.vars {
            if let Some(t) = &v.ty {
                self.type_(t);
            }
            if let Some(e) = &v.init {
                self.expr(e);
            }
        }
        for m in &c.methods {
            if let ItemKind::Fn(f) = &m.kind {
                self.fn_decl(f);
            }
        }
        for e in &c.invariants {
            self.expr(e);
        }
        self.generics.pop();
    }

    // ---- activities -----------------------------------------------------

    fn task_attr(&mut self, a: &TaskAttr) {
        match a {
            TaskAttr::Prio(e) | TaskAttr::Deadline(e) | TaskAttr::Core(e) | TaskAttr::Stack(e) => {
                self.expr(e);
            }
            TaskAttr::Reads(ids) | TaskAttr::Writes(ids) => {
                for id in ids {
                    self.resolve_name(id);
                }
            }
        }
    }

    fn trigger(&mut self, t: &Trigger) {
        match t {
            Trigger::Every { period, offset } => {
                self.expr(period);
                if let Some(o) = offset {
                    self.expr(o);
                }
            }
            Trigger::On(e) => {
                // An event name need not be declared (spec 06 §6.10).
                if let ExprKind::Name(n) = &e.kind {
                    self.vocab += 1;
                    self.resolve_name(n);
                    self.vocab -= 1;
                } else {
                    self.expr(e);
                }
            }
        }
    }

    fn task(&mut self, t: &TaskDecl) {
        self.trigger(&t.trigger);
        for a in &t.attrs {
            self.task_attr(&a.node);
        }
        self.block(&t.body);
    }

    fn rule(&mut self, r: &Rule) {
        self.expr(&r.cond);
        if let Some(h) = &r.hold {
            self.expr(h);
        }
        self.block(&r.body);
    }

    fn fsm(&mut self, f: &FsmDecl) {
        if let Some(t) = &f.trigger {
            self.trigger(t);
        }
        self.push_scope();
        for v in &f.vars {
            self.local_var(v);
        }
        let mut names: BTreeMap<String, neuron_syntax::Span> = BTreeMap::new();
        for s in &f.states {
            if let Some(prev) = names.get(&s.name.name) {
                let prev = *prev;
                self.diags.push(FileDiag {
                    file: self.file,
                    diag: Diagnostic::error(
                        "NE0102",
                        format!(
                            "state `{}` is already defined in `{}`",
                            s.name.name, f.name.name
                        ),
                        s.name.span,
                    )
                    .with_label(prev, "first definition here"),
                });
            } else {
                names.insert(s.name.name.clone(), s.name.span);
            }
        }
        for s in &f.states {
            for b in [&s.entry, &s.exit, &s.during].into_iter().flatten() {
                self.block(b);
            }
            for tr in &s.transitions {
                if names.contains_key(&tr.target.name) {
                    self.record(
                        &tr.target,
                        Res::State {
                            fsm: f.name.name.clone(),
                            state: tr.target.name.clone(),
                        },
                    );
                } else {
                    let help = names
                        .keys()
                        .find(|k| levenshtein(k, &tr.target.name) <= 2)
                        .map(|k| format!("did you mean `{k}`?"));
                    self.err_help(
                        "NE0101",
                        format!("`{}` has no state `{}`", f.name.name, tr.target.name),
                        tr.target.span,
                        help,
                    );
                }
                if let Some(e) = &tr.when {
                    self.expr(e);
                }
                if let Some(e) = &tr.after {
                    self.expr(e);
                }
                if let Some(b) = &tr.action {
                    self.block(b);
                }
            }
        }
        self.pop_scope();
    }

    fn collect_steps(items: &[SeqItem], out: &mut Vec<Ident>) {
        for it in items {
            match it {
                SeqItem::Step { name, body, .. } => {
                    out.push(name.clone());
                    Self::collect_steps(body, out);
                }
                SeqItem::Parallel { branches, .. } => {
                    for (_, b) in branches {
                        Self::collect_steps(b, out);
                    }
                }
                SeqItem::Stmt(_) => {}
            }
        }
    }

    fn sequence(&mut self, s: &SequenceDecl) {
        self.push_scope();
        for p in &s.params {
            if let Some(t) = &p.ty {
                self.type_(t);
            }
        }
        for p in &s.params {
            self.declare(&p.name, LocalKind::Param { mutable: p.mutable });
        }
        if let Some(d) = &s.on {
            self.resolve_name(d);
        }
        let mut ids = Vec::new();
        Self::collect_steps(&s.items, &mut ids);
        let mut names = BTreeSet::new();
        let mut first: BTreeMap<String, neuron_syntax::Span> = BTreeMap::new();
        for id in &ids {
            if let Some(prev) = first.get(&id.name) {
                let prev = *prev;
                self.diags.push(FileDiag {
                    file: self.file,
                    diag: Diagnostic::error(
                        "NE0102",
                        format!("step `{}` is already defined in `{}`", id.name, s.name.name),
                        id.span,
                    )
                    .with_label(prev, "first definition here"),
                });
            } else {
                first.insert(id.name.clone(), id.span);
                names.insert(id.name.clone());
            }
        }
        self.steps.push(names);
        self.seq_items(&s.items);
        self.steps.pop();
        self.pop_scope();
    }

    fn seq_items(&mut self, items: &[SeqItem]) {
        for it in items {
            match it {
                SeqItem::Step { body, .. } => {
                    self.push_scope();
                    self.seq_items(body);
                    self.pop_scope();
                }
                SeqItem::Parallel { branches, .. } => {
                    for (_, b) in branches {
                        self.push_scope();
                        self.seq_items(b);
                        self.pop_scope();
                    }
                }
                SeqItem::Stmt(s) => self.stmt(s),
            }
        }
    }

    fn step_ref(&mut self, id: &Ident) {
        let Some(steps) = self.steps.last() else {
            return;
        };
        if !steps.contains(&id.name) {
            let help = steps
                .iter()
                .find(|k| levenshtein(k, &id.name) <= 2)
                .map(|k| format!("did you mean `{k}`?"));
            self.err_help(
                "NE0101",
                format!("unknown step `{}`", id.name),
                id.span,
                help,
            );
        }
    }

    fn circuit(&mut self, c: &neuron_syntax::ast::CircuitDecl) {
        self.push_scope();
        for p in &c.ports {
            self.type_(&p.ty);
            if let Some(pin) = &p.pin {
                self.vocab += 1;
                self.expr(pin);
                self.vocab -= 1;
            }
        }
        for p in &c.ports {
            let kind = match p.dir {
                PortDir::In => LocalKind::InPort,
                PortDir::Out | PortDir::InOut => LocalKind::Var,
            };
            self.declare(&p.name, kind);
        }
        if let Some(clk) = &c.clock {
            self.expect_kind(clk, SymKind::Clock, "clock");
        }
        for m in &c.members {
            match &m.node {
                CircuitMember::Reg { name, ty, .. } | CircuitMember::Wire { name, ty } => {
                    self.type_(ty);
                    self.declare(name, LocalKind::Var);
                }
                CircuitMember::Const { name, ty, .. } => {
                    if let Some(t) = ty {
                        self.type_(t);
                    }
                    self.declare(name, LocalKind::Let);
                }
                CircuitMember::Instance { name, .. } => {
                    self.declare(name, LocalKind::Let);
                }
                CircuitMember::Comb(_) | CircuitMember::OnEdge { .. } | CircuitMember::Fsm(_) => {}
            }
        }
        for m in &c.members {
            match &m.node {
                CircuitMember::Reg { init, .. } => {
                    if let Some(e) = init {
                        self.expr(e);
                    }
                }
                CircuitMember::Const { value, .. } => self.expr(value),
                CircuitMember::Instance { of, .. } => self.expr(of),
                CircuitMember::Comb(b) => self.block(b),
                CircuitMember::OnEdge { clock, body, .. } => {
                    self.expect_kind(clock, SymKind::Clock, "clock");
                    self.block(body);
                }
                CircuitMember::Fsm(f) => self.fsm(f),
                CircuitMember::Wire { .. } => {}
            }
        }
        self.pop_scope();
    }

    fn verify(&mut self, props: &[neuron_syntax::ast::Spanned<Property>]) {
        let mut states: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let items = &self.prog.files[self.file].ast.items;
        let mut add = |f: &FsmDecl| {
            for s in &f.states {
                states
                    .entry(s.name.name.clone())
                    .or_default()
                    .push(f.name.name.clone());
            }
        };
        for it in items {
            match &it.kind {
                ItemKind::Fsm(f) => add(f),
                ItemKind::Circuit(c) => {
                    for m in &c.members {
                        if let CircuitMember::Fsm(f) = &m.node {
                            add(f);
                        }
                    }
                }
                _ => {}
            }
        }
        self.verify_states = Some(states);
        for p in props {
            match &p.node {
                Property::Never(e) => self.expr(e),
                Property::Always { cond, implies } => {
                    self.expr(cond);
                    if let Some(i) = implies {
                        self.expr(i);
                    }
                }
                Property::EventuallyWithin {
                    within,
                    after,
                    goal,
                } => {
                    self.expr(within);
                    self.expr(after);
                    self.expr(goal);
                }
            }
        }
        self.verify_states = None;
    }

    fn widget(&mut self, w: &Widget) {
        for (name, e) in &w.options {
            match name.name.as_str() {
                "font" | "color" | "theme" => {
                    self.vocab += 1;
                    self.expr(e);
                    self.vocab -= 1;
                }
                _ => self.expr(e),
            }
        }
        for (_, b) in &w.events {
            self.block(b);
        }
        for c in &w.children {
            self.widget(c);
        }
    }

    // ---- statements -----------------------------------------------------

    fn local_var(&mut self, v: &VarDecl) {
        if let Some(t) = &v.ty {
            self.type_(t);
        }
        if let Some(e) = &v.init {
            self.expr(e);
        }
        let kind = if v.mutable {
            LocalKind::Var
        } else {
            LocalKind::Let
        };
        self.declare(&v.name, kind);
    }

    fn block(&mut self, b: &Block) {
        self.push_scope();
        for c in &b.contracts {
            self.expr(&c.cond);
        }
        for s in &b.stmts {
            self.stmt(s);
        }
        self.pop_scope();
    }

    fn args(&mut self, args: &[Arg]) {
        for a in args {
            self.expr(&a.value);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Let {
                mutable,
                pattern,
                ty,
                value,
            } => {
                if let Some(t) = ty {
                    self.type_(t);
                }
                self.expr(value);
                let kind = if *mutable {
                    LocalKind::Var
                } else {
                    LocalKind::Let
                };
                self.pattern(pattern, kind, true);
            }
            StmtKind::Assign {
                place, value, when, ..
            } => {
                self.expr(place);
                self.expr(value);
                if let Some(w) = when {
                    self.expr(w);
                }
            }
            StmtKind::Expr(e) => self.expr(e),
            StmtKind::Pass | StmtKind::Break | StmtKind::Continue | StmtKind::Asm(_) => {}
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            StmtKind::Fault { args, .. } | StmtKind::Emit { args, .. } => self.args(args),
            StmtKind::Assert { cond, .. } => self.expr(cond),
            StmtKind::Coil { place, when, .. } => {
                self.expr(place);
                self.expr(when);
            }
            StmtKind::Wait(e) => self.expr(e),
            StmtKind::Until {
                cond,
                timeout,
                on_timeout,
            } => {
                self.expr(cond);
                if let Some(t) = timeout {
                    self.expr(t);
                }
                if let Some(id) = on_timeout {
                    self.step_ref(id);
                }
            }
            StmtKind::Jump(id) => self.step_ref(id),
            StmtKind::Move(m) => {
                self.expr(&m.to);
                if let Some(v) = &m.via {
                    self.expr(v);
                }
                for (_, e) in &m.options {
                    self.expr(e);
                }
            }
            StmtKind::If {
                branches,
                otherwise,
            } => {
                for (c, b) in branches {
                    self.expr(c);
                    self.block(b);
                }
                if let Some(b) = otherwise {
                    self.block(b);
                }
            }
            StmtKind::Match { value, arms } => {
                self.expr(value);
                for a in arms {
                    self.push_scope();
                    self.pattern(&a.pattern, LocalKind::Pattern, true);
                    if let Some(g) = &a.guard {
                        self.expr(g);
                    }
                    self.block(&a.body);
                    self.pop_scope();
                }
            }
            StmtKind::For {
                pattern,
                iter,
                body,
            } => {
                self.expr(iter);
                self.push_scope();
                self.pattern(pattern, LocalKind::For, true);
                self.block(body);
                self.pop_scope();
            }
            StmtKind::While {
                pattern,
                cond,
                max,
                body,
            } => {
                self.expr(cond);
                if let Some(m) = max {
                    self.expr(m);
                }
                self.push_scope();
                if let Some(p) = pattern {
                    self.pattern(p, LocalKind::Pattern, true);
                }
                self.block(body);
                self.pop_scope();
            }
            StmtKind::Loop { max, body } => {
                if let Some(m) = max {
                    self.expr(m);
                }
                self.block(body);
            }
            StmtKind::Atomic(b) => self.block(b),
            StmtKind::Rule(r) => self.rule(r),
        }
    }

    fn pattern(&mut self, p: &Pattern, kind: LocalKind, declare: bool) {
        match &p.kind {
            PatternKind::Wildcard => {}
            PatternKind::Bind(id) => {
                if declare {
                    self.declare(id, kind);
                } else if let Some(l) = self.lookup_local(&id.name) {
                    self.record(id, Res::Local(l));
                }
            }
            PatternKind::Lit(e) => self.expr(e),
            PatternKind::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            PatternKind::Constructor { path, args, fields } => {
                self.pattern_path(path);
                for a in args {
                    self.pattern(a, kind, declare);
                }
                for (_, f) in fields {
                    self.pattern(f, kind, declare);
                }
            }
            PatternKind::Path(path) => self.pattern_path(path),
            PatternKind::Tuple(ps) => {
                for x in ps {
                    self.pattern(x, kind, declare);
                }
            }
            PatternKind::Or(ps) => {
                for (i, x) in ps.iter().enumerate() {
                    self.pattern(x, kind, declare && i == 0);
                }
            }
        }
    }

    fn pattern_path(&mut self, p: &Path) {
        let Some(first) = p.segments.first() else {
            return;
        };
        if let Some(Res::Module(m)) = self.resolve_name(first) {
            if let Some(second) = p.segments.get(1) {
                self.module_member(m, second);
            }
        }
    }

    // ---- expressions ----------------------------------------------------

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Lit(_) | ExprKind::SelfValue | ExprKind::Result | ExprKind::Error => {}
            ExprKind::Quantity(inner, unit) => {
                self.expr(inner);
                self.unit_check(unit);
            }
            ExprKind::Name(id) => {
                self.resolve_name(id);
            }
            ExprKind::Binary(_, a, b) | ExprKind::Map(a, b) | ExprKind::Pipe(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Compare(first, rest) => {
                self.expr(first);
                for (_, x) in rest {
                    self.expr(x);
                }
            }
            ExprKind::Unary(_, x) | ExprKind::Try(x) | ExprKind::Old(x) | ExprKind::Run(x) => {
                self.expr(x)
            }
            ExprKind::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            ExprKind::IfElse(c, a, b) => {
                self.expr(c);
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Call(callee, args) => {
                let lenient = self.dialect == Dialect::Cell;
                if lenient {
                    self.vocab += 1;
                }
                self.expr(callee);
                if lenient {
                    self.vocab -= 1;
                }
                self.args(args);
            }
            ExprKind::Index(base, idx, end) => {
                self.expr(base);
                self.expr(idx);
                if let Some(x) = end {
                    self.expr(x);
                }
            }
            ExprKind::Field(base, id) => self.field(base, id),
            ExprKind::Cast(x, ty) => {
                self.expr(x);
                self.type_(ty);
            }
            ExprKind::Tuple(xs) | ExprKind::Array(xs) => {
                for x in xs {
                    self.expr(x);
                }
            }
            ExprKind::ArrayRepeat(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Lambda(params, body) => {
                self.push_scope();
                for p in params {
                    self.declare(p, LocalKind::Lambda);
                }
                self.expr(body);
                self.pop_scope();
            }
        }
    }

    fn field(&mut self, base: &Expr, id: &Ident) {
        if let ExprKind::Name(n) = &base.kind {
            match self.resolve_name(n) {
                Some(Res::Module(m)) => self.module_member(m, id),
                Some(Res::Item { module, name }) if self.verify_states.is_some() => {
                    let is_fsm = self
                        .prog
                        .symbol(module, &name)
                        .is_some_and(|s| s.kind == SymKind::Fsm);
                    if is_fsm {
                        self.record(
                            id,
                            Res::State {
                                fsm: name,
                                state: id.name.clone(),
                            },
                        );
                    }
                }
                _ => {}
            }
        } else {
            self.expr(base);
        }
    }
}
