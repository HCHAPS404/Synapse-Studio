//! Type and unit checking (task A5, spec 15 §15.3).
//!
//! The checker is *strict where the program defines things* (its own types,
//! functions, units and literals) and *gradual where it touches external
//! vocabulary* (standard library, device manifests): those values have type
//! `Ty::Unknown`, which is compatible with everything. Task A12 and B1 replace
//! the vocabulary with real declarations and tighten the check.

mod expr;
mod lower;
mod pat;
mod stmt;

use crate::consteval::Consts;
use crate::model::{key, FileDiag, Program, Res, SpanKey, SymKind};
use crate::resolve::Resolved;
use crate::ty::{DefRef, FnTy, NumKind, Ty};
use neuron_syntax::ast::{
    CircuitMember, Expr, ExprKind, FnBody, FnDecl, FsmDecl, Ident, Item, ItemKind, Property,
    SafetyMember, SeqItem, SequenceDecl, TaskAttr, TaskDecl, Trigger, VarDecl, Widget,
};
use neuron_syntax::{Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};

/// Identity of a function or method.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FnKey {
    /// Module index.
    pub module: usize,
    /// Owning class, for methods.
    pub owner: Option<String>,
    /// Function name.
    pub name: String,
}

/// What a call expression calls.
#[derive(Debug, Clone, PartialEq)]
pub enum CallTarget {
    /// A user function.
    Fn(FnKey),
    /// A user method.
    Method {
        /// The method.
        key: FnKey,
        /// It takes `mut self`.
        mut_self: bool,
    },
    /// A struct, class or enum-variant constructor.
    Ctor(DefRef),
    /// An operation on an `io` binding.
    Io {
        /// Binding name.
        binding: String,
        /// Method name.
        method: String,
    },
    /// A prelude function or function block.
    Builtin(String),
    /// Anything else (external vocabulary).
    Foreign,
}

/// A field of a struct or class.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldInfo {
    /// Name.
    pub name: String,
    /// Type.
    pub ty: Ty,
    /// Can be assigned after construction.
    pub mutable: bool,
    /// Has a default value.
    pub has_default: bool,
}

/// A user type definition.
#[derive(Debug, Clone, PartialEq)]
pub enum DefInfo {
    /// `struct`.
    Struct {
        /// Generic parameter names.
        generics: Vec<String>,
        /// Fields.
        fields: Vec<FieldInfo>,
    },
    /// `enum`.
    Enum {
        /// Generic parameter names.
        generics: Vec<String>,
        /// Variants with payload types.
        variants: Vec<(String, Vec<Ty>)>,
    },
    /// `class`.
    Class {
        /// Generic parameter names.
        generics: Vec<String>,
        /// State fields.
        fields: Vec<FieldInfo>,
        /// Interfaces named after `is`.
        interfaces: Vec<DefRef>,
    },
    /// `interface`.
    Interface {
        /// Method signatures.
        methods: BTreeMap<String, FnTy>,
    },
}

/// Facts produced by the checker for the later analyses.
#[derive(Debug, Default)]
pub struct Checked {
    /// Type of every local binding (index = `Res::Local` id).
    pub local_ty: Vec<Ty>,
    /// Target of every call expression, by call span.
    pub calls: BTreeMap<SpanKey, CallTarget>,
    /// User type definitions.
    pub defs: BTreeMap<DefRef, DefInfo>,
    /// Signatures of functions and methods.
    pub sigs: BTreeMap<FnKey, FnTy>,
    /// Types of module-level items.
    pub item_ty: BTreeMap<(usize, String), Ty>,
    /// Static length of each iterated expression of a `for` loop, by span.
    pub iter_len: BTreeMap<SpanKey, Option<u64>>,
    /// Call targets found while typing callee expressions (consumed by `call`).
    pub targets: BTreeMap<SpanKey, CallTarget>,
}

/// Per-item checking context.
#[derive(Debug, Clone)]
pub(crate) struct Ctx {
    pub module: usize,
    pub ret: Option<Ty>,
    pub result_ty: Option<Ty>,
    pub self_ty: Option<Ty>,
    pub self_mut: bool,
    pub loop_depth: u32,
    pub generics: Vec<(String, Vec<String>)>,
}

impl Ctx {
    pub(crate) fn new(module: usize) -> Ctx {
        Ctx {
            module,
            ret: None,
            result_ty: None,
            self_ty: None,
            self_mut: false,
            loop_depth: 0,
            generics: Vec::new(),
        }
    }
}

pub(crate) struct Checker<'a> {
    pub prog: &'a Program,
    pub resolved: &'a Resolved,
    pub consts: Consts<'a>,
    pub diags: &'a mut Vec<FileDiag>,
    pub out: Checked,
    pub cur: Ctx,
    pub silent: u32,
    pub in_progress: BTreeSet<(usize, String)>,
}

/// Checks every analyzable module.
pub fn check_program(prog: &Program, resolved: &Resolved, diags: &mut Vec<FileDiag>) -> Checked {
    let mut c = Checker {
        prog,
        resolved,
        consts: Consts::new(prog, &resolved.res),
        diags,
        out: Checked {
            local_ty: vec![Ty::Unknown; resolved.locals.len()],
            ..Checked::default()
        },
        cur: Ctx::new(0),
        silent: 0,
        in_progress: BTreeSet::new(),
    };
    c.build_defs();
    for (mi, f) in prog.files.iter().enumerate() {
        if f.syntax_errors {
            continue;
        }
        c.cur = Ctx::new(mi);
        for item in &f.ast.items {
            c.item(item);
        }
    }
    // Types of all stateful module-level items, for the memory analysis.
    for (mi, m) in prog.modules.iter().enumerate() {
        if prog.files[mi].syntax_errors {
            continue;
        }
        for (name, sym) in &m.symbols {
            if matches!(
                sym.kind,
                SymKind::Var | SymKind::PlcVar(_) | SymKind::Signal | SymKind::Io
            ) {
                c.cur = Ctx::new(mi);
                let _ = c.item_ty(mi, name);
            }
        }
    }
    c.out
}

impl Checker<'_> {
    pub(crate) fn err(&mut self, code: &'static str, msg: impl Into<String>, span: Span) {
        if self.silent > 0 {
            return;
        }
        self.diags.push(FileDiag {
            file: self.cur.module,
            diag: Diagnostic::error(code, msg, span),
        });
    }

    pub(crate) fn err_help(
        &mut self,
        code: &'static str,
        msg: impl Into<String>,
        span: Span,
        help: impl Into<String>,
    ) {
        if self.silent > 0 {
            return;
        }
        self.diags.push(FileDiag {
            file: self.cur.module,
            diag: Diagnostic::error(code, msg, span).with_help(help),
        });
    }

    /// The resolution recorded for an identifier of the current file.
    pub(crate) fn res_of(&self, id: &Ident) -> Option<&Res> {
        self.resolved.res.get(&key(self.cur.module, id.span))
    }

    pub(crate) fn set_local(&mut self, id: &Ident, ty: Ty) {
        if let Some(Res::Local(i)) = self.res_of(id).cloned() {
            if let Some(slot) = self.out.local_ty.get_mut(i) {
                *slot = ty;
            }
        }
    }

    // ---- items ----------------------------------------------------------

    fn item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Const { name, ty, value } => {
                let declared = ty.as_ref().map(|t| self.lower(t));
                let actual = self.expr(value, declared.as_ref());
                if let Some(d) = &declared {
                    self.coerce(d, &actual, value);
                } else {
                    let _ = name;
                }
            }
            ItemKind::Var(v) => self.var_item(v),
            ItemKind::Signal { ty, init, .. } => {
                let t = self.lower(ty);
                if let Some(e) = init {
                    let a = self.expr(e, Some(&t));
                    self.coerce(&t, &a, e);
                }
            }
            ItemKind::Struct(def) => {
                let gens = lower::generic_names(&def.generics);
                self.cur.generics = def.generics.iter().map(lower::generic_entry).collect();
                for f in &def.members {
                    if let Some(d) = &f.default {
                        let t = self.lower_with(&f.ty, &gens);
                        let a = self.expr(d, Some(&t));
                        self.coerce(&t, &a, d);
                    }
                }
                self.cur.generics.clear();
            }
            ItemKind::Enum(def) => {
                for v in &def.members {
                    if let Some(e) = &v.value {
                        let a = self.expr(e, None);
                        if !matches!(&a, Ty::Num(k, _) if k.is_integer()) && !a.is_unknown() {
                            self.err(
                                "NE0208",
                                format!(
                                    "enum discriminant must be an integer, found `{}`",
                                    a.display()
                                ),
                                e.span,
                            );
                        }
                    }
                }
            }
            ItemKind::Class(c) => self.class(c),
            ItemKind::Fn(f) => self.fn_decl(f, None),
            ItemKind::ExternFn { func, .. } => self.fn_decl(func, None),
            ItemKind::Task(t) => self.task(t),
            ItemKind::Interrupt { attrs, body, .. } => {
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
                        SafetyMember::Rate(e) | SafetyMember::Watchdog(e) => self.duration(e),
                        SafetyMember::Reset(e) => self.cond(e),
                        SafetyMember::SafeState(b) | SafetyMember::OnFault { body: b, .. } => {
                            self.block(b);
                        }
                        SafetyMember::Rule(r) => self.rule(r),
                    }
                }
            }
            ItemKind::Verify(props) => {
                for p in props {
                    match &p.node {
                        Property::Never(e) => self.cond(e),
                        Property::Always { cond, implies } => {
                            self.cond(cond);
                            if let Some(i) = implies {
                                self.cond(i);
                            }
                        }
                        Property::EventuallyWithin {
                            within,
                            after,
                            goal,
                        } => {
                            self.duration(within);
                            self.cond(after);
                            self.cond(goal);
                        }
                    }
                }
            }
            ItemKind::Test { body, .. } => self.block(body),
            ItemKind::Scan(e) => self.duration(e),
            ItemKind::PlcSection { vars, .. } => {
                for v in vars {
                    self.var_item(v);
                }
            }
            ItemKind::Rung { body, .. } => self.block(body),
            ItemKind::Clock { freq, .. } => {
                let t = self.expr(freq, None);
                self.expect_dim(&t, "frequency", freq.span, |d| {
                    d.0 == [-1, 0, 0, 0, 0, 0, 0, 0, 0]
                });
            }
            ItemKind::Circuit(c) => self.circuit(c),
            ItemKind::Geometry { ty, value, .. } => {
                let declared = ty.as_ref().map(|t| self.lower(t));
                let a = self.expr(value, declared.as_ref());
                if let Some(d) = &declared {
                    self.coerce(d, &a, value);
                }
            }
            ItemKind::Screen { widgets, .. } => {
                for w in widgets {
                    self.widget(w);
                }
            }
            ItemKind::Link(l) => {
                self.expr(&l.from, None);
                self.expr(&l.to, None);
                if let Some(e) = &l.every {
                    self.duration(e);
                }
                if let Some(e) = &l.within {
                    self.duration(e);
                }
            }
            ItemKind::Tag { locations, .. } => {
                for l in locations {
                    self.expr(l, None);
                }
            }
            ItemKind::Mutex { parties, .. } => {
                for p in parties {
                    self.expr(p, None);
                }
            }
            _ => {}
        }
    }

    fn var_item(&mut self, v: &VarDecl) {
        let declared = v.ty.as_ref().map(|t| self.lower(t));
        if let Some(e) = &v.init {
            let a = self.expr(e, declared.as_ref());
            if let Some(d) = &declared {
                self.coerce(d, &a, e);
            }
        }
    }

    fn widget(&mut self, w: &Widget) {
        for (_, e) in &w.options {
            self.expr(e, None);
        }
        for (_, b) in &w.events {
            self.block(b);
        }
        for c in &w.children {
            self.widget(c);
        }
    }

    /// Checks that `e` is a constant-or-not duration expression (dimension time).
    pub(crate) fn duration(&mut self, e: &Expr) {
        let t = self.expr(e, None);
        self.expect_dim(&t, "time", e.span, |d| d.0 == [1, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    pub(crate) fn expect_dim(
        &mut self,
        t: &Ty,
        what: &str,
        span: Span,
        ok: impl Fn(&crate::units::Dim) -> bool,
    ) {
        match t {
            Ty::Num(_, u) if !ok(&u.dim) => {
                self.err_help(
                    "NE0201",
                    format!("expected a {what}, found {}", u.dim.describe()),
                    span,
                    format!("write a quantity with a {what} unit"),
                );
            }
            Ty::Num(..) | Ty::Unknown | Ty::Never => {}
            other => self.err(
                "NE0208",
                format!("expected a {what}, found `{}`", other.display()),
                span,
            ),
        }
    }

    fn task_attr(&mut self, a: &TaskAttr) {
        match a {
            TaskAttr::Prio(e) | TaskAttr::Core(e) | TaskAttr::Stack(e) => {
                let t = self.expr(e, None);
                self.expect_int(&t, e.span);
            }
            TaskAttr::Deadline(e) => self.duration(e),
            TaskAttr::Reads(_) | TaskAttr::Writes(_) => {}
        }
    }

    pub(crate) fn expect_int(&mut self, t: &Ty, span: Span) {
        match t {
            Ty::Num(k, u) if k.is_integer() && u.is_dimensionless() => {}
            Ty::Unknown | Ty::Never => {}
            other => self.err(
                "NE0208",
                format!("expected an integer, found `{}`", other.display()),
                span,
            ),
        }
    }

    fn trigger(&mut self, t: &Trigger) {
        match t {
            Trigger::Every { period, offset } => {
                self.duration(period);
                if let Some(o) = offset {
                    self.duration(o);
                }
            }
            Trigger::On(e) => {
                if !matches!(e.kind, ExprKind::Name(_)) {
                    self.expr(e, None);
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

    fn fsm(&mut self, f: &FsmDecl) {
        if let Some(t) = &f.trigger {
            self.trigger(t);
        }
        for v in &f.vars {
            self.local_var(v);
        }
        for s in &f.states {
            for b in [&s.entry, &s.exit, &s.during].into_iter().flatten() {
                self.block(b);
            }
            for tr in &s.transitions {
                if let Some(e) = &tr.when {
                    self.cond(e);
                }
                if let Some(e) = &tr.after {
                    self.duration(e);
                }
                if let Some(b) = &tr.action {
                    self.block(b);
                }
            }
        }
    }

    fn sequence(&mut self, s: &SequenceDecl) {
        for p in &s.params {
            if let Some(t) = &p.ty {
                let ty = self.lower(t);
                self.set_local(&p.name, ty);
            }
        }
        self.seq_items(&s.items);
    }

    fn seq_items(&mut self, items: &[SeqItem]) {
        for it in items {
            match it {
                SeqItem::Step { body, .. } => self.seq_items(body),
                SeqItem::Parallel { branches, .. } => {
                    for (_, b) in branches {
                        self.seq_items(b);
                    }
                }
                SeqItem::Stmt(s) => self.stmt(s),
            }
        }
    }

    fn circuit(&mut self, c: &neuron_syntax::ast::CircuitDecl) {
        for p in &c.ports {
            let t = self.lower(&p.ty);
            self.set_local(&p.name, t);
        }
        for m in &c.members {
            match &m.node {
                CircuitMember::Reg { name, ty, .. } | CircuitMember::Wire { name, ty } => {
                    let t = self.lower(ty);
                    self.set_local(name, t);
                }
                CircuitMember::Const { name, ty, value } => {
                    let declared = ty.as_ref().map(|t| self.lower(t));
                    let a = self.expr(value, declared.as_ref());
                    let t = match declared {
                        Some(d) => {
                            self.coerce(&d, &a, value);
                            d
                        }
                        None => a,
                    };
                    self.set_local(name, t);
                }
                CircuitMember::Instance { name, of } => {
                    let t = self.expr(of, None);
                    self.set_local(name, t);
                }
                _ => {}
            }
        }
        for m in &c.members {
            match &m.node {
                CircuitMember::Reg {
                    ty, init: Some(e), ..
                } => {
                    let t = self.lower(ty);
                    let a = self.expr(e, Some(&t));
                    self.coerce(&t, &a, e);
                }
                CircuitMember::Comb(b) => self.block(b),
                CircuitMember::OnEdge { body, .. } => self.block(body),
                CircuitMember::Fsm(f) => self.fsm(f),
                _ => {}
            }
        }
    }

    // ---- functions and classes -----------------------------------------

    pub(crate) fn fn_decl(&mut self, f: &FnDecl, owner: Option<(&DefRef, bool)>) {
        let fkey = FnKey {
            module: self.cur.module,
            owner: owner.map(|(d, _)| d.1.clone()),
            name: f.sig.name.name.clone(),
        };
        let Some(sig) = self.out.sigs.get(&fkey).cloned() else {
            return;
        };
        let saved = self.cur.clone();
        let mut gens = saved.generics.clone();
        gens.extend(sig.generics.iter().cloned());
        self.cur.generics = gens;
        self.cur.ret = Some(sig.ret.clone());
        self.cur.result_ty = Some(sig.ret.clone());
        self.cur.loop_depth = 0;
        if let Some((def, _)) = owner {
            let gen_names = match self.out.defs.get(def) {
                Some(DefInfo::Class { generics, .. }) => generics.clone(),
                _ => Vec::new(),
            };
            self.cur.self_ty = Some(Ty::Adt {
                def: def.clone(),
                args: gen_names.iter().map(|g| Ty::Param(g.clone())).collect(),
            });
            self.cur.self_mut = sig.self_mode == Some(true);
        } else {
            self.cur.self_ty = None;
            self.cur.self_mut = false;
        }
        for (p, pt) in f
            .sig
            .params
            .iter()
            .filter(|p| p.name.name != "self")
            .zip(&sig.params)
        {
            self.set_local(&p.name, pt.ty.clone());
            if let Some(d) = &p.default {
                let a = self.expr(d, Some(&pt.ty));
                self.coerce(&pt.ty, &a, d);
            }
        }
        let contracts: Vec<&neuron_syntax::ast::Contract> = f
            .contracts
            .iter()
            .chain(match &f.body {
                FnBody::Block(b) => b.contracts.iter(),
                FnBody::Expr(_) => [].iter(),
            })
            .collect();
        for c in contracts {
            self.cond(&c.cond);
        }
        match &f.body {
            FnBody::Block(b) => {
                for s in &b.stmts {
                    self.stmt(s);
                }
                if sig.ret != Ty::Void && !sig.ret.is_unknown() && !stmt::always_returns(&b.stmts) {
                    self.err_help(
                        "NE0217",
                        format!(
                            "function `{}` does not return a `{}` on every path",
                            f.sig.name.name,
                            sig.ret.display()
                        ),
                        f.sig.name.span,
                        "end every branch with `return` or `fault`",
                    );
                }
            }
            FnBody::Expr(e) => {
                let a = self.expr(e, Some(&sig.ret));
                if sig.ret != Ty::Void {
                    self.coerce(&sig.ret, &a, e);
                }
            }
        }
        self.cur = saved;
    }

    fn class(&mut self, c: &neuron_syntax::ast::ClassDecl) {
        let def: DefRef = (self.cur.module, c.name.name.clone());
        let gen_names = lower::generic_names(&c.generics);
        let saved = self.cur.clone();
        self.cur.generics = c.generics.iter().map(lower::generic_entry).collect();
        for v in &c.vars {
            if let (Some(t), Some(e)) = (&v.ty, &v.init) {
                let ty = self.lower_with(t, &gen_names);
                let a = self.expr(e, Some(&ty));
                self.coerce(&ty, &a, e);
            }
        }
        for m in &c.methods {
            if let ItemKind::Fn(f) = &m.kind {
                let mutating = f
                    .sig
                    .params
                    .iter()
                    .any(|p| p.name.name == "self" && p.mutable);
                self.fn_decl(f, Some((&def, mutating)));
            }
        }
        // Invariants are boolean expressions over `self`.
        self.cur.self_ty = Some(Ty::Adt {
            def: def.clone(),
            args: gen_names.iter().map(|g| Ty::Param(g.clone())).collect(),
        });
        for e in &c.invariants {
            self.cond(e);
        }
        self.cur = saved;
        // Interface conformance.
        let ifaces = match self.out.defs.get(&def) {
            Some(DefInfo::Class { interfaces, .. }) => interfaces.clone(),
            _ => Vec::new(),
        };
        for (iface, path) in ifaces.iter().zip(&c.interfaces) {
            let methods = match self.out.defs.get(iface) {
                Some(DefInfo::Interface { methods }) => methods.clone(),
                _ => continue,
            };
            for (name, want) in methods {
                let fk = FnKey {
                    module: def.0,
                    owner: Some(def.1.clone()),
                    name: name.clone(),
                };
                match self.out.sigs.get(&fk) {
                    None => self.err_help(
                        "NE0211",
                        format!("class `{}` does not implement `{}.{name}`", def.1, iface.1),
                        path.span,
                        format!("add `fn {name}(...)` to the class"),
                    ),
                    Some(have) => {
                        if have.params.len() != want.params.len()
                            || have.self_mode != want.self_mode
                        {
                            self.err(
                                "NE0211",
                                format!(
                                    "method `{name}` of `{}` does not match interface `{}`",
                                    def.1, iface.1
                                ),
                                path.span,
                            );
                        }
                    }
                }
            }
        }
    }

    // ---- shared helpers --------------------------------------------------

    /// Does the class `def` implement interface `iface`?
    pub(crate) fn implements(&self, def: &DefRef, iface: &DefRef) -> bool {
        matches!(
            self.out.defs.get(def),
            Some(DefInfo::Class { interfaces, .. }) if interfaces.contains(iface)
        )
    }

    pub(crate) fn symbol_kind(&self, module: usize, name: &str) -> Option<SymKind> {
        self.prog.symbol(module, name).map(|s| s.kind)
    }
}

/// Default type of a variable initialized from a value of type `t`.
pub(crate) fn default_ty(t: &Ty) -> Ty {
    match t {
        Ty::Num(NumKind::LitInt, u) => Ty::Num(
            NumKind::Int {
                signed: true,
                bits: 32,
            },
            u.clone(),
        ),
        Ty::Num(NumKind::LitReal, u) => Ty::Num(NumKind::Float(32), u.clone()),
        Ty::NoneLit => Ty::Option(Box::new(Ty::Unknown)),
        Ty::Array(e, n) => Ty::Array(Box::new(default_ty(e)), *n),
        Ty::Tuple(v) => Ty::Tuple(v.iter().map(default_ty).collect()),
        Ty::Range(e) => Ty::Range(Box::new(default_ty(e))),
        other => other.clone(),
    }
}
