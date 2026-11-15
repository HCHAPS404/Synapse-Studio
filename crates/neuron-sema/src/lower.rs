//! Lowering of an analyzed package to NIR (task A9, spec 16).
//!
//! The NIR keeps the source structure (so it can be printed back) and adds stable ids,
//! resolved references and the type of every expression.

use crate::check::{CallTarget, Checked, FnKey};
use crate::consteval::Consts;
use crate::model::{key, Program, Res, SymKind};
use crate::resolve::Resolved;
use crate::ty::Ty;
use crate::units::{Rat, Unit};
use neuron_nir as nir;
use neuron_syntax::ast::{self, ExprKind, ItemKind};
use neuron_syntax::Span;
use std::collections::BTreeMap;

/// Exact decimal text of a rational, or `n/d` when the decimal does not terminate.
pub fn rat_text(r: Rat) -> String {
    let (n, d) = (r.numer(), r.denom());
    if d == 1 {
        return n.to_string();
    }
    // Terminating decimal: the denominator has only factors 2 and 5.
    let mut dd = d;
    let (mut twos, mut fives) = (0u32, 0u32);
    while dd % 2 == 0 {
        dd /= 2;
        twos += 1;
    }
    while dd % 5 == 0 {
        dd /= 5;
        fives += 1;
    }
    if dd != 1 {
        return format!("{n}/{d}");
    }
    let digits = twos.max(fives);
    let Some(scale) = 10i128.checked_pow(digits) else {
        return format!("{n}/{d}");
    };
    let Some(scaled) = n.checked_mul(scale / d) else {
        return format!("{n}/{d}");
    };
    let neg = scaled < 0;
    let a = scaled.unsigned_abs().to_string();
    let w = digits as usize;
    let padded = if a.len() <= w {
        format!("{}{a}", "0".repeat(w + 1 - a.len()))
    } else {
        a
    };
    let (int, frac) = padded.split_at(padded.len() - w);
    let frac = frac.trim_end_matches('0');
    let sign = if neg { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

fn span(s: Span) -> nir::Span {
    [s.start, s.end]
}

fn unit_ref(u: &ast::UnitExpr) -> nir::UnitRef {
    let factors: Vec<nir::UnitFactor> = u
        .factors
        .iter()
        .map(|(id, e)| nir::UnitFactor {
            name: id.name.clone(),
            exp: *e,
        })
        .collect();
    nir::UnitRef {
        text: nir::factors_text(&factors),
        factors,
    }
}

fn unit(u: &Unit) -> nir::Unit {
    nir::Unit {
        text: u.display(),
        dim: u.dim.0,
        scale: rat_text(u.scale),
        offset: rat_text(u.offset),
    }
}

fn binop(op: ast::BinOp) -> String {
    op.as_str().to_string()
}

struct Lower<'a> {
    prog: &'a Program,
    resolved: &'a Resolved,
    checked: &'a Checked,
    consts: Consts<'a>,
    mi: usize,
    locals: BTreeMap<usize, String>,
}

/// Lowers the whole program. Call only when the analysis reported no errors.
pub fn lower_program(prog: &Program, resolved: &Resolved, checked: &Checked) -> nir::Nir {
    let mut modules = Vec::new();
    for (mi, f) in prog.files.iter().enumerate() {
        let mut l = Lower {
            prog,
            resolved,
            checked,
            consts: Consts::new(prog, &resolved.res),
            mi,
            locals: BTreeMap::new(),
        };
        let h = &f.ast.header;
        let cell = h.dialect == ast::Dialect::Cell;
        let items = f
            .ast
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| l.item(it, format!("m{mi}.i{i}")))
            .collect();
        modules.push(nir::Module {
            id: format!("m{mi}"),
            name: h.name.dotted(),
            file: f.path.clone(),
            kind: if cell {
                nir::ModuleKind::Cell
            } else {
                nir::ModuleKind::Module
            },
            dialect: h.dialect.name().to_string(),
            target: h.target.as_ref().map(ast::Path::dotted),
            items,
        });
    }
    nir::Nir {
        nir_version: nir::NIR_VERSION.to_string(),
        edition: neuron_syntax::LANGUAGE_EDITION.to_string(),
        package: prog.manifest.as_ref().map(|m| m.name.clone()),
        modules,
    }
}

impl Lower<'_> {
    // ---- ids and types --------------------------------------------------------

    fn local_id(&mut self, idx: usize) -> String {
        let n = self.locals.len();
        self.locals
            .entry(idx)
            .or_insert_with(|| format!("m{}.l{n}", self.mi))
            .clone()
    }

    fn decl_local(&mut self, id: &ast::Ident) -> String {
        match self.resolved.res.get(&key(self.mi, id.span)) {
            Some(Res::Local(i)) => self.local_id(*i),
            _ => format!("m{}.l?{}", self.mi, id.name),
        }
    }

    fn local_ty(&self, id: &ast::Ident) -> nir::Type {
        match self.resolved.res.get(&key(self.mi, id.span)) {
            Some(Res::Local(i)) => self
                .checked
                .local_ty
                .get(*i)
                .map_or(nir::Type::Unknown, |t| self.ty(t)),
            _ => nir::Type::Unknown,
        }
    }

    fn item_id(&self, module: usize, name: &str) -> Option<String> {
        let sym = self.prog.symbol(module, name)?;
        let item = self.prog.files.get(module)?.ast.items.get(sym.item)?;
        Some(match (&sym.kind, &item.kind) {
            (SymKind::PlcVar(_), _) => format!("m{module}.i{}.v{}", sym.item, sym.sub),
            (SymKind::Clock, ItemKind::Clocks(_)) => {
                format!("m{module}.i{}.c{}", sym.item, sym.sub)
            }
            _ => format!("m{module}.i{}", sym.item),
        })
    }

    fn def_id(&self, def: &(usize, String)) -> String {
        self.item_id(def.0, &def.1)
            .unwrap_or_else(|| format!("m{}.?{}", def.0, def.1))
    }

    fn ty(&self, t: &Ty) -> nir::Type {
        let b = |t: &Ty| Box::new(self.ty(t));
        match t {
            Ty::Unknown => nir::Type::Unknown,
            Ty::Never => nir::Type::Never,
            Ty::Void => nir::Type::Void,
            Ty::Bool => nir::Type::Bool,
            Ty::Num(k, u) => nir::Type::Num {
                kind: k.name(),
                unit: if u.is_dimensionless() && u.label.is_empty() && u.scale == Rat::ONE {
                    None
                } else {
                    Some(unit(u))
                },
            },
            Ty::Bits { signed, width } => nir::Type::Bits {
                signed: *signed,
                width: *width,
            },
            Ty::Instant => nir::Type::Instant,
            Ty::Str(n) => nir::Type::Str { cap: *n },
            Ty::Array(e, n) => nir::Type::Array {
                elem: b(e),
                len: *n,
            },
            Ty::Slice(e) => nir::Type::Slice { elem: b(e) },
            Ty::Tuple(v) => nir::Type::Tuple {
                items: v.iter().map(|t| self.ty(t)).collect(),
            },
            Ty::Range(e) => nir::Type::Range { elem: b(e) },
            Ty::Option(e) => nir::Type::Option { inner: b(e) },
            Ty::NoneLit => nir::Type::NoneLit,
            Ty::Result(a, e) => nir::Type::Result {
                ok: b(a),
                err: b(e),
            },
            Ty::Adt { def, args } => nir::Type::Adt {
                def: self.def_id(def),
                name: def.1.clone(),
                args: args.iter().map(|t| self.ty(t)).collect(),
            },
            Ty::Interface(def) => nir::Type::Interface {
                def: self.def_id(def),
                name: def.1.clone(),
            },
            Ty::Param(n) => nir::Type::Param { name: n.clone() },
            Ty::Channel(e, n) => nir::Type::Channel {
                elem: b(e),
                cap: *n,
            },
            Ty::Pool(e, n) => nir::Type::Pool {
                elem: b(e),
                cap: *n,
            },
            Ty::Handle(e) => nir::Type::Handle { elem: b(e) },
            Ty::Fn(_) => nir::Type::Fn,
            Ty::Io(k) => nir::Type::Io { kind: k.clone() },
            Ty::Opaque(n) => nir::Type::Opaque { name: n.clone() },
            Ty::TypeRef(t) => nir::Type::TypeRef { of: b(t) },
            Ty::Module(m) => nir::Type::Module {
                id: format!("m{m}"),
            },
            Ty::Event => nir::Type::Event,
        }
    }

    fn expr_ty(&self, e: &ast::Expr) -> nir::Type {
        self.checked
            .expr_ty
            .get(&key(self.mi, e.span))
            .map_or(nir::Type::Unknown, |t| self.ty(t))
    }

    // ---- items ----------------------------------------------------------------

    fn item(&mut self, it: &ast::Item, id: String) -> nir::Item {
        let attrs = it
            .attrs
            .iter()
            .map(|a| nir::Attr {
                name: a.name.name.clone(),
                args: self.args(&a.args),
            })
            .collect();
        let def = self.def(&it.kind, &id);
        nir::Item {
            id,
            span: span(it.span),
            public: it.public,
            doc: it.doc.clone(),
            attrs,
            def,
        }
    }

    fn opt_ty(&mut self, t: &Option<ast::Type>) -> Option<nir::TypeExpr> {
        t.as_ref().map(|t| self.type_expr(t))
    }

    fn opt_expr(&mut self, e: &Option<ast::Expr>) -> Option<nir::Expr> {
        e.as_ref().map(|e| self.expr(e))
    }

    fn def(&mut self, k: &ItemKind, id: &str) -> nir::ItemDef {
        use nir::ItemDef as D;
        match k {
            ItemKind::Use { path, names, alias } => D::Use {
                path: path.segments.iter().map(|s| s.name.clone()).collect(),
                names: names.iter().map(|n| n.name.clone()).collect(),
                alias: alias.as_ref().map(|a| a.name.clone()),
            },
            ItemKind::Const { name, ty, value } => D::Const {
                name: name.name.clone(),
                ty: self.opt_ty(ty),
                value: self.expr(value),
            },
            ItemKind::Var(v) => {
                let vty = self
                    .checked
                    .item_ty
                    .get(&(self.mi, v.name.name.clone()))
                    .map_or(nir::Type::Unknown, |t| self.ty(t));
                D::Var {
                    var: self.var_decl(v, id.to_string(), vty),
                }
            }
            ItemKind::Signal { name, ty, init } => D::Signal {
                name: name.name.clone(),
                ty: self.type_expr(ty),
                init: self.opt_expr(init),
            },
            ItemKind::Unit { name, def } => D::Unit {
                name: name.name.clone(),
                def: unit_ref(def),
            },
            ItemKind::Event { name, fields, from } => D::Event {
                name: name.name.clone(),
                fields: fields
                    .iter()
                    .map(|(n, t)| nir::EventField {
                        name: n.name.clone(),
                        ty: self.opt_ty(t),
                    })
                    .collect(),
                from: from.as_ref().map(|d| d.name.clone()),
            },
            ItemKind::Struct(d) => D::Struct {
                name: d.name.name.clone(),
                generics: self.generics(&d.generics),
                fields: d
                    .members
                    .iter()
                    .map(|f| nir::Field {
                        name: f.name.name.clone(),
                        ty: self.type_expr(&f.ty),
                        default: self.opt_expr(&f.default),
                        span: span(f.span),
                    })
                    .collect(),
            },
            ItemKind::Enum(d) => D::Enum {
                name: d.name.name.clone(),
                generics: self.generics(&d.generics),
                repr: self.opt_ty(&d.repr),
                variants: d
                    .members
                    .iter()
                    .map(|v| nir::Variant {
                        name: v.name.name.clone(),
                        payload: v.payload.iter().map(|t| self.type_expr(t)).collect(),
                        value: self.opt_expr(&v.value),
                        span: span(v.span),
                    })
                    .collect(),
            },
            ItemKind::Class(c) => {
                let def = (self.mi, c.name.name.clone());
                let field_tys: Vec<nir::Type> = match self.checked.defs.get(&def) {
                    Some(crate::check::DefInfo::Class { fields, .. }) => {
                        fields.iter().map(|f| self.ty(&f.ty)).collect()
                    }
                    _ => Vec::new(),
                };
                let vars = c
                    .vars
                    .iter()
                    .enumerate()
                    .map(|(k, v)| {
                        let vty = field_tys.get(k).cloned().unwrap_or(nir::Type::Unknown);
                        self.var_decl(v, format!("{id}.f{k}"), vty)
                    })
                    .collect();
                let invariants = c.invariants.iter().map(|e| self.expr(e)).collect();
                let methods = c
                    .methods
                    .iter()
                    .enumerate()
                    .map(|(k, m)| self.item(m, format!("{id}.m{k}")))
                    .collect();
                D::Class {
                    name: c.name.name.clone(),
                    generics: self.generics(&c.generics),
                    interfaces: c.interfaces.iter().map(ast::Path::dotted).collect(),
                    vars,
                    invariants,
                    methods,
                }
            }
            ItemKind::Interface {
                name,
                generics,
                methods,
            } => D::Interface {
                name: name.name.clone(),
                generics: self.generics(generics),
                methods: methods.iter().map(|m| self.fn_sig(m)).collect(),
            },
            ItemKind::Alias { name, generics, ty } => D::Alias {
                name: name.name.clone(),
                generics: self.generics(generics),
                ty: self.type_expr(ty),
            },
            ItemKind::Fn(f) => D::Fn {
                func: self.fn_decl(f),
            },
            ItemKind::Task(t) => D::Task {
                name: t.name.name.clone(),
                trigger: self.trigger(&t.trigger),
                attrs: self.task_attrs(&t.attrs),
                body: self.block(&t.body),
            },
            ItemKind::Interrupt {
                source,
                attrs,
                body,
            } => D::Interrupt {
                source: self.expr(source),
                attrs: self.task_attrs(attrs),
                body: self.block(body),
            },
            ItemKind::Rule(r) => D::Rule { rule: self.rule(r) },
            ItemKind::Fsm(f) => D::Fsm { fsm: self.fsm(f) },
            ItemKind::Sequence(s) => D::Sequence {
                name: s.name.name.clone(),
                params: s.params.iter().map(|p| self.param(p)).collect(),
                on: s.on.as_ref().map(|d| d.name.clone()),
                items: self.seq_items(&s.items),
            },
            ItemKind::Safety(members) => D::Safety {
                members: members
                    .iter()
                    .map(|m| nir::SafetyMember {
                        span: span(m.span),
                        def: match &m.node {
                            ast::SafetyMember::Rate(e) => nir::SafetyDef::Rate {
                                value: self.expr(e),
                            },
                            ast::SafetyMember::Watchdog(e) => nir::SafetyDef::Watchdog {
                                value: self.expr(e),
                            },
                            ast::SafetyMember::SafeState(b) => nir::SafetyDef::SafeState {
                                body: self.block(b),
                            },
                            ast::SafetyMember::Rule(r) => {
                                nir::SafetyDef::Rule { rule: self.rule(r) }
                            }
                            ast::SafetyMember::OnFault { fault, body } => nir::SafetyDef::OnFault {
                                fault: fault.as_ref().map(ast::Path::dotted),
                                body: self.block(body),
                            },
                            ast::SafetyMember::Reset(e) => {
                                nir::SafetyDef::Reset { cond: self.expr(e) }
                            }
                        },
                    })
                    .collect(),
            },
            ItemKind::Verify(props) => D::Verify {
                props: props
                    .iter()
                    .map(|p| nir::Prop {
                        span: span(p.span),
                        def: match &p.node {
                            ast::Property::Never(e) => nir::PropDef::Never { cond: self.expr(e) },
                            ast::Property::Always { cond, implies } => nir::PropDef::Always {
                                cond: self.expr(cond),
                                implies: self.opt_expr(implies),
                            },
                            ast::Property::EventuallyWithin {
                                within,
                                after,
                                goal,
                            } => nir::PropDef::EventuallyWithin {
                                within: self.expr(within),
                                after: self.expr(after),
                                goal: self.expr(goal),
                            },
                        },
                    })
                    .collect(),
            },
            ItemKind::Test { name, body } => D::Test {
                name: name.clone(),
                body: self.block(body),
            },
            ItemKind::Io {
                name,
                kind,
                location,
            } => D::Io {
                name: name.name.clone(),
                kind: self.expr(kind),
                at: self.expr(location),
            },
            ItemKind::Clocks(defs) => D::Clocks {
                clocks: defs
                    .iter()
                    .enumerate()
                    .map(|(k, c)| nir::ClockDef {
                        id: format!("{id}.c{k}"),
                        name: c.name.name.clone(),
                        value: self.expr(&c.value),
                        from: self.opt_expr(&c.from),
                        span: span(c.span),
                    })
                    .collect(),
            },
            ItemKind::Uses(paths) => D::Uses {
                paths: paths.iter().map(ast::Path::dotted).collect(),
            },
            ItemKind::ExternBlock { abi, fns } => D::ExternBlock {
                abi: abi.clone(),
                fns: fns.iter().map(|f| self.fn_sig(f)).collect(),
            },
            ItemKind::ExternFn { abi, func } => D::ExternFn {
                abi: abi.clone(),
                func: self.fn_decl(func),
            },
            ItemKind::Asm(a) => D::Asm { asm: asm(a) },
            ItemKind::Scan(e) => D::Scan {
                period: self.expr(e),
            },
            ItemKind::PlcSection { section, vars } => {
                let section = match section {
                    ast::PlcSection::Inputs => "inputs",
                    ast::PlcSection::Outputs => "outputs",
                    ast::PlcSection::Memory => "memory",
                };
                D::PlcSection {
                    section: section.to_string(),
                    vars: vars
                        .iter()
                        .enumerate()
                        .map(|(k, v)| {
                            let vty = self
                                .checked
                                .item_ty
                                .get(&(self.mi, v.name.name.clone()))
                                .map_or(nir::Type::Unknown, |t| self.ty(t));
                            self.var_decl(v, format!("{id}.v{k}"), vty)
                        })
                        .collect(),
                }
            }
            ItemKind::Rung { name, body } => D::Rung {
                name: name.name.clone(),
                body: self.block(body),
            },
            ItemKind::Clock { name, freq, pin } => D::Clock {
                name: name.name.clone(),
                freq: self.expr(freq),
                pin: self.opt_expr(pin),
            },
            ItemKind::Circuit(c) => D::Circuit {
                circuit: self.circuit(c),
            },
            ItemKind::Sync { from, to, via } => D::Sync {
                from: self.expr(from),
                to: self.expr(to),
                via: self.expr(via),
            },
            ItemKind::Geometry {
                kind,
                name,
                ty,
                value,
            } => D::Geometry {
                kind: match kind {
                    ast::GeometryKind::Frame => "frame",
                    ast::GeometryKind::Tool => "tool",
                    ast::GeometryKind::Point => "point",
                }
                .to_string(),
                name: name.name.clone(),
                ty: self.opt_ty(ty),
                value: self.expr(value),
            },
            ItemKind::Display { name, driver } => D::Display {
                name: name.name.clone(),
                driver: self.expr(driver),
            },
            ItemKind::Screen {
                name,
                display,
                widgets,
            } => D::Screen {
                name: name.name.clone(),
                display: display.as_ref().map(|d| d.name.clone()),
                widgets: widgets.iter().map(|w| self.widget(w)).collect(),
            },
            ItemKind::Theme { name, body } => D::Theme {
                name: name.name.clone(),
                body: self.block(body),
            },
            ItemKind::Device { name, ty, address } => D::Device {
                name: name.name.clone(),
                ty: ty.dotted(),
                address: self.opt_expr(address),
            },
            ItemKind::Place { module, device } => D::Place {
                module: module.dotted(),
                device: device.name.clone(),
            },
            ItemKind::Link(l) => D::Link {
                from: self.expr(&l.from),
                to: self.expr(&l.to),
                via: self.expr(&l.via),
                every: self.opt_expr(&l.every),
                within: self.opt_expr(&l.within),
            },
            ItemKind::Tag {
                name,
                ty,
                locations,
            } => D::Tag {
                name: name.name.clone(),
                ty: self.type_expr(ty),
                locations: locations.iter().map(|e| self.expr(e)).collect(),
            },
            ItemKind::Zone { name, shape } => D::Zone {
                name: name.name.clone(),
                shape: self.expr(shape),
            },
            ItemKind::Mutex { zone, parties } => D::Mutex {
                zone: zone.name.clone(),
                parties: parties.iter().map(|e| self.expr(e)).collect(),
            },
        }
    }

    fn var_decl(&mut self, v: &ast::VarDecl, id: String, vty: nir::Type) -> nir::VarDecl {
        nir::VarDecl {
            id,
            mutable: v.mutable,
            retain: v.retain,
            name: v.name.name.clone(),
            ty: self.opt_ty(&v.ty),
            vty,
            init: self.opt_expr(&v.init),
            at: self.opt_expr(&v.at),
            span: span(v.span),
        }
    }

    fn local_var(&mut self, v: &ast::VarDecl) -> nir::VarDecl {
        let id = self.decl_local(&v.name);
        let vty = self.local_ty(&v.name);
        self.var_decl(v, id, vty)
    }

    fn generics(&mut self, g: &[ast::GenericParam]) -> Vec<nir::Generic> {
        g.iter()
            .map(|p| match p {
                ast::GenericParam::Type { name, bounds, .. } => nir::Generic::Type {
                    name: name.name.clone(),
                    bounds: bounds.iter().map(ast::Path::dotted).collect(),
                },
                ast::GenericParam::Const { name, ty, .. } => nir::Generic::Const {
                    name: name.name.clone(),
                    ty: self.type_expr(ty),
                },
            })
            .collect()
    }

    fn param(&mut self, p: &ast::Param) -> nir::Param {
        let is_self = p.name.name == "self" && p.ty.is_none();
        nir::Param {
            mutable: p.mutable,
            name: p.name.name.clone(),
            id: if is_self {
                None
            } else {
                Some(self.decl_local(&p.name))
            },
            ty: self.opt_ty(&p.ty),
            default: self.opt_expr(&p.default),
        }
    }

    fn fn_sig(&mut self, s: &ast::FnSig) -> nir::FnSig {
        nir::FnSig {
            name: s.name.name.clone(),
            generics: self.generics(&s.generics),
            params: s.params.iter().map(|p| self.param(p)).collect(),
            ret: self.opt_ty(&s.ret),
            span: span(s.span),
        }
    }

    fn fn_decl(&mut self, f: &ast::FnDecl) -> nir::FnDecl {
        nir::FnDecl {
            sig: self.fn_sig(&f.sig),
            contracts: f.contracts.iter().map(|c| self.contract(c)).collect(),
            body: match &f.body {
                ast::FnBody::Block(b) => nir::FnBody::Block {
                    block: self.block(b),
                },
                ast::FnBody::Expr(e) => nir::FnBody::Expr { expr: self.expr(e) },
            },
        }
    }

    fn contract(&mut self, c: &ast::Contract) -> nir::Contract {
        nir::Contract {
            kind: match c.kind {
                ast::ContractKind::Requires => "requires",
                ast::ContractKind::Ensures => "ensures",
                ast::ContractKind::Invariant => "invariant",
            }
            .to_string(),
            cond: self.expr(&c.cond),
            span: span(c.span),
        }
    }

    fn trigger(&mut self, t: &ast::Trigger) -> nir::Trigger {
        match t {
            ast::Trigger::Every { period, offset } => nir::Trigger::Every {
                period: self.expr(period),
                offset: self.opt_expr(offset),
            },
            ast::Trigger::On(e) => nir::Trigger::On {
                event: self.expr(e),
            },
        }
    }

    fn task_attrs(&mut self, a: &[ast::Spanned<ast::TaskAttr>]) -> Vec<nir::TaskAttr> {
        let names = |v: &[ast::Ident]| v.iter().map(|i| i.name.clone()).collect();
        a.iter()
            .map(|x| match &x.node {
                ast::TaskAttr::Prio(e) => nir::TaskAttr::Prio {
                    value: self.expr(e),
                },
                ast::TaskAttr::Deadline(e) => nir::TaskAttr::Deadline {
                    value: self.expr(e),
                },
                ast::TaskAttr::Core(e) => nir::TaskAttr::Core {
                    value: self.expr(e),
                },
                ast::TaskAttr::Stack(e) => nir::TaskAttr::Stack {
                    value: self.expr(e),
                },
                ast::TaskAttr::Reads(v) => nir::TaskAttr::Reads { names: names(v) },
                ast::TaskAttr::Writes(v) => nir::TaskAttr::Writes { names: names(v) },
            })
            .collect()
    }

    fn rule(&mut self, r: &ast::Rule) -> nir::Rule {
        nir::Rule {
            cond: self.expr(&r.cond),
            hold: self.opt_expr(&r.hold),
            body: self.block(&r.body),
            span: span(r.span),
        }
    }

    fn fsm(&mut self, f: &ast::FsmDecl) -> nir::Fsm {
        let trigger = f.trigger.as_ref().map(|t| self.trigger(t));
        let vars = f.vars.iter().map(|v| self.local_var(v)).collect();
        let states = f
            .states
            .iter()
            .map(|s| nir::State {
                initial: s.initial,
                is_final: s.is_final,
                name: s.name.name.clone(),
                entry: s.entry.as_ref().map(|b| self.block(b)),
                exit: s.exit.as_ref().map(|b| self.block(b)),
                during: s.during.as_ref().map(|b| self.block(b)),
                transitions: s
                    .transitions
                    .iter()
                    .map(|t| nir::Transition {
                        target: t.target.name.clone(),
                        when: self.opt_expr(&t.when),
                        after: self.opt_expr(&t.after),
                        action: t.action.as_ref().map(|b| self.block(b)),
                        span: span(t.span),
                    })
                    .collect(),
                span: span(s.span),
            })
            .collect();
        nir::Fsm {
            name: f.name.name.clone(),
            trigger,
            vars,
            states,
        }
    }

    fn seq_items(&mut self, items: &[ast::SeqItem]) -> Vec<nir::SeqItem> {
        items
            .iter()
            .map(|it| match it {
                ast::SeqItem::Step {
                    initial,
                    name,
                    body,
                    span: s,
                } => nir::SeqItem::Step {
                    initial: *initial,
                    name: name.name.clone(),
                    body: self.seq_items(body),
                    span: span(*s),
                },
                ast::SeqItem::Parallel { branches, span: s } => nir::SeqItem::Parallel {
                    branches: branches
                        .iter()
                        .map(|(n, b)| nir::SeqBranch {
                            name: n.name.clone(),
                            items: self.seq_items(b),
                        })
                        .collect(),
                    span: span(*s),
                },
                ast::SeqItem::Stmt(s) => nir::SeqItem::Stmt { stmt: self.stmt(s) },
            })
            .collect()
    }

    fn circuit(&mut self, c: &ast::CircuitDecl) -> nir::Circuit {
        let ports = c
            .ports
            .iter()
            .map(|p| nir::Port {
                id: self.decl_local(&p.name),
                name: p.name.name.clone(),
                dir: match p.dir {
                    ast::PortDir::In => "in",
                    ast::PortDir::Out => "out",
                    ast::PortDir::InOut => "inout",
                }
                .to_string(),
                ty: self.type_expr(&p.ty),
                pin: self.opt_expr(&p.pin),
                span: span(p.span),
            })
            .collect();
        let members = c
            .members
            .iter()
            .map(|m| nir::CircuitMember {
                span: span(m.span),
                def: match &m.node {
                    ast::CircuitMember::Reg { name, ty, init } => nir::CircuitDef::Reg {
                        id: self.decl_local(name),
                        name: name.name.clone(),
                        ty: self.type_expr(ty),
                        init: self.opt_expr(init),
                    },
                    ast::CircuitMember::Wire { name, ty } => nir::CircuitDef::Wire {
                        id: self.decl_local(name),
                        name: name.name.clone(),
                        ty: self.type_expr(ty),
                    },
                    ast::CircuitMember::Const { name, ty, value } => nir::CircuitDef::Const {
                        id: self.decl_local(name),
                        name: name.name.clone(),
                        ty: self.opt_ty(ty),
                        value: self.expr(value),
                    },
                    ast::CircuitMember::Instance { name, of } => nir::CircuitDef::Instance {
                        id: self.decl_local(name),
                        name: name.name.clone(),
                        of: self.expr(of),
                    },
                    ast::CircuitMember::Comb(b) => nir::CircuitDef::Comb {
                        body: self.block(b),
                    },
                    ast::CircuitMember::OnEdge {
                        rising,
                        clock,
                        body,
                    } => nir::CircuitDef::OnEdge {
                        rising: *rising,
                        clock: clock.name.clone(),
                        body: self.block(body),
                    },
                    ast::CircuitMember::Fsm(f) => nir::CircuitDef::Fsm { fsm: self.fsm(f) },
                },
            })
            .collect();
        nir::Circuit {
            name: c.name.name.clone(),
            ports,
            clock: c.clock.as_ref().map(|k| k.name.clone()),
            members,
        }
    }

    fn widget(&mut self, w: &ast::Widget) -> nir::Widget {
        nir::Widget {
            kind: w.kind.name.clone(),
            name: w.name.as_ref().map(|n| n.name.clone()),
            text: w.text.clone(),
            options: w
                .options
                .iter()
                .map(|(n, e)| nir::NamedExpr {
                    name: n.name.clone(),
                    value: self.expr(e),
                })
                .collect(),
            children: w.children.iter().map(|c| self.widget(c)).collect(),
            events: w
                .events
                .iter()
                .map(|(n, b)| nir::WidgetEvent {
                    name: n.name.clone(),
                    body: self.block(b),
                })
                .collect(),
            span: span(w.span),
        }
    }

    // ---- types ----------------------------------------------------------------

    fn type_expr(&mut self, t: &ast::Type) -> nir::TypeExpr {
        let kind = match &t.kind {
            ast::TypeKind::Named { path, args } => nir::TypeExprKind::Named {
                path: path.dotted(),
                args: args
                    .iter()
                    .map(|a| match a {
                        ast::TypeArg::Type(t) => nir::TypeArg::Type {
                            ty: self.type_expr(t),
                        },
                        ast::TypeArg::Expr(e) => nir::TypeArg::Expr { expr: self.expr(e) },
                    })
                    .collect(),
            },
            ast::TypeKind::Array(elem, len) => nir::TypeExprKind::Array {
                elem: Box::new(self.type_expr(elem)),
                len: Box::new(self.expr(len)),
            },
            ast::TypeKind::Slice(elem) => nir::TypeExprKind::Slice {
                elem: Box::new(self.type_expr(elem)),
            },
            ast::TypeKind::Tuple(ts) => nir::TypeExprKind::Tuple {
                items: ts.iter().map(|t| self.type_expr(t)).collect(),
            },
            ast::TypeKind::Unit => nir::TypeExprKind::Unit,
        };
        nir::TypeExpr {
            kind,
            unit: t.unit.as_ref().map(unit_ref),
            optional: t.optional,
            span: span(t.span),
        }
    }

    // ---- statements -----------------------------------------------------------

    fn block(&mut self, b: &ast::Block) -> nir::Block {
        nir::Block {
            contracts: b.contracts.iter().map(|c| self.contract(c)).collect(),
            stmts: b.stmts.iter().map(|s| self.stmt(s)).collect(),
        }
    }

    fn args(&mut self, a: &[ast::Arg]) -> Vec<nir::Arg> {
        a.iter()
            .map(|x| nir::Arg {
                mutable: x.mutable,
                name: x.name.as_ref().map(|n| n.name.clone()),
                value: self.expr(&x.value),
            })
            .collect()
    }

    fn stmt(&mut self, s: &ast::Stmt) -> nir::Stmt {
        use ast::StmtKind as K;
        use nir::StmtKind as N;
        let kind = match &s.kind {
            K::Let {
                mutable,
                pattern,
                ty,
                value,
            } => {
                // The value is lowered first: its names resolve before the new binding.
                let value = self.expr(value);
                N::Let {
                    mutable: *mutable,
                    pattern: self.pattern(pattern),
                    ty: self.opt_ty(ty),
                    value,
                }
            }
            K::Assign {
                place,
                op,
                value,
                when,
            } => N::Assign {
                place: self.expr(place),
                op: match op {
                    ast::AssignOp::Set => "=".to_string(),
                    ast::AssignOp::Compound(b) => format!("{}=", b.as_str()),
                },
                value: self.expr(value),
                when: self.opt_expr(when),
            },
            K::Expr(e) => N::Expr { expr: self.expr(e) },
            K::Pass => N::Pass,
            K::Return(e) => N::Return {
                value: self.opt_expr(e),
            },
            K::Break => N::Break,
            K::Continue => N::Continue,
            K::Fault { name, args } => N::Fault {
                name: name.dotted(),
                args: self.args(args),
            },
            K::Assert { cond, msg } => N::Assert {
                cond: self.expr(cond),
                msg: msg.clone(),
            },
            K::Emit { event, args } => N::Emit {
                event: event.dotted(),
                args: self.args(args),
            },
            K::Coil { set, place, when } => N::Coil {
                set: *set,
                place: self.expr(place),
                when: self.expr(when),
            },
            K::Wait(e) => N::Wait {
                duration: self.expr(e),
            },
            K::Until {
                cond,
                timeout,
                on_timeout,
            } => N::Until {
                cond: self.expr(cond),
                timeout: self.opt_expr(timeout),
                on_timeout: on_timeout.as_ref().map(|i| i.name.clone()),
            },
            K::Jump(t) => N::Jump {
                target: t.name.clone(),
            },
            K::Move(m) => N::Move {
                kind: match m.kind {
                    ast::MoveKind::Ptp => "ptp",
                    ast::MoveKind::Linear => "linear",
                    ast::MoveKind::Arc => "arc",
                }
                .to_string(),
                to: self.expr(&m.to),
                via: self.opt_expr(&m.via),
                options: m
                    .options
                    .iter()
                    .map(|(n, e)| nir::NamedExpr {
                        name: n.name.clone(),
                        value: self.expr(e),
                    })
                    .collect(),
            },
            K::If {
                branches,
                otherwise,
            } => N::If {
                branches: branches
                    .iter()
                    .map(|(c, b)| nir::IfBranch {
                        cond: self.expr(c),
                        body: self.block(b),
                    })
                    .collect(),
                otherwise: otherwise.as_ref().map(|b| self.block(b)),
            },
            K::Match { value, arms } => N::Match {
                value: self.expr(value),
                arms: arms
                    .iter()
                    .map(|a| nir::Arm {
                        pattern: self.pattern(&a.pattern),
                        guard: self.opt_expr(&a.guard),
                        body: self.block(&a.body),
                        span: span(a.span),
                    })
                    .collect(),
            },
            K::For {
                pattern,
                iter,
                body,
            } => {
                let iter_e = self.expr(iter);
                N::For {
                    pattern: self.pattern(pattern),
                    count: self
                        .checked
                        .iter_len
                        .get(&key(self.mi, iter.span))
                        .copied()
                        .flatten(),
                    iter: iter_e,
                    body: self.block(body),
                }
            }
            K::While {
                pattern,
                cond,
                max,
                body,
            } => {
                let cond = self.expr(cond);
                N::While {
                    pattern: pattern.as_ref().map(|p| self.pattern(p)),
                    cond,
                    max: self.opt_expr(max),
                    body: self.block(body),
                }
            }
            K::Loop { max, body } => N::Loop {
                max: self.opt_expr(max),
                body: self.block(body),
            },
            K::Atomic(b) => N::Atomic {
                body: self.block(b),
            },
            K::Rule(r) => N::Rule { rule: self.rule(r) },
            K::Asm(a) => N::Asm { asm: asm(a) },
        };
        nir::Stmt {
            span: span(s.span),
            s: kind,
        }
    }

    fn pattern(&mut self, p: &ast::Pattern) -> nir::Pattern {
        use ast::PatternKind as K;
        use nir::PatternKind as N;
        let kind = match &p.kind {
            K::Wildcard => N::Wildcard,
            K::Bind(id) => N::Bind {
                name: id.name.clone(),
                id: self.decl_local(id),
                ty: self.local_ty(id),
            },
            K::Lit(e) => N::Lit {
                value: self.expr(e),
            },
            K::Range { lo, hi, inclusive } => N::Range {
                lo: self.expr(lo),
                hi: self.expr(hi),
                inclusive: *inclusive,
            },
            K::Constructor { path, args, fields } => N::Ctor {
                path: path.dotted(),
                args: args.iter().map(|a| self.pattern(a)).collect(),
                fields: fields
                    .iter()
                    .map(|(n, x)| nir::FieldPattern {
                        name: n.name.clone(),
                        pattern: self.pattern(x),
                    })
                    .collect(),
            },
            K::Path(path) => N::Path {
                path: path.dotted(),
            },
            K::Tuple(ps) => N::Tuple {
                items: ps.iter().map(|x| self.pattern(x)).collect(),
            },
            K::Or(ps) => N::Or {
                alts: ps.iter().map(|x| self.pattern(x)).collect(),
            },
        };
        nir::Pattern {
            span: span(p.span),
            p: kind,
        }
    }

    // ---- expressions ----------------------------------------------------------

    fn call_target(&self, t: &CallTarget) -> nir::CallTarget {
        let fn_id = |k: &FnKey| {
            self.item_id(k.module, &k.name)
                .unwrap_or_else(|| k.name.clone())
        };
        match t {
            CallTarget::Fn(k) => nir::CallTarget::Fn { id: fn_id(k) },
            CallTarget::Method { key: k, mut_self } => nir::CallTarget::Method {
                class: k
                    .owner
                    .as_ref()
                    .and_then(|o| self.item_id(k.module, o))
                    .unwrap_or_default(),
                name: k.name.clone(),
                mut_self: *mut_self,
            },
            CallTarget::Ctor(d) => nir::CallTarget::Ctor {
                def: self.def_id(d),
            },
            CallTarget::Io { binding, method } => nir::CallTarget::Io {
                binding: binding.clone(),
                method: method.clone(),
            },
            CallTarget::Builtin(n) => nir::CallTarget::Builtin { name: n.clone() },
            CallTarget::Foreign => nir::CallTarget::Foreign,
        }
    }

    fn name_ref(&mut self, id: &ast::Ident) -> nir::ExprKind {
        match self.resolved.res.get(&key(self.mi, id.span)).cloned() {
            Some(Res::Local(i)) => nir::ExprKind::Local {
                id: self.local_id(i),
                name: id.name.clone(),
            },
            Some(Res::Item { module, name }) => match self.item_id(module, &name) {
                Some(iid) => nir::ExprKind::Ref {
                    id: iid,
                    name: id.name.clone(),
                },
                None => nir::ExprKind::Global {
                    name: id.name.clone(),
                },
            },
            Some(Res::Module(m)) => nir::ExprKind::Module {
                id: format!("m{m}"),
                name: id.name.clone(),
            },
            Some(Res::State { fsm, state }) => nir::ExprKind::State { fsm, state },
            Some(Res::Foreign(_)) | Some(Res::Prelude(_)) | None => nir::ExprKind::Global {
                name: id.name.clone(),
            },
        }
    }

    fn bx(&mut self, e: &ast::Expr) -> Box<nir::Expr> {
        Box::new(self.expr(e))
    }

    fn expr(&mut self, e: &ast::Expr) -> nir::Expr {
        let ty = self.expr_ty(e);
        self.expr_with(e, ty)
    }

    fn expr_with(&mut self, e: &ast::Expr, ty: nir::Type) -> nir::Expr {
        let kind = match &e.kind {
            ExprKind::Lit(l) => nir::ExprKind::Lit { lit: lit(l) },
            ExprKind::Quantity(inner, u) => {
                // A unit shared from the upper bound of a range (`0..120 degC`) gives the
                // quantity and its number the same span; the number is then typed from
                // the quantity with the unit removed.
                let value = if inner.span == e.span {
                    let plain = match &ty {
                        nir::Type::Num { kind, .. } => nir::Type::Num {
                            kind: kind.clone(),
                            unit: None,
                        },
                        other => other.clone(),
                    };
                    Box::new(self.expr_with(inner, plain))
                } else {
                    self.bx(inner)
                };
                let si = self.consts.eval(self.mi, e).map(|q| rat_text(q.value));
                nir::ExprKind::Quantity {
                    value,
                    unit: unit_ref(u),
                    si,
                }
            }
            ExprKind::Name(id) => self.name_ref(id),
            ExprKind::SelfValue => nir::ExprKind::SelfValue,
            ExprKind::Result => nir::ExprKind::Result,
            ExprKind::Binary(op, a, b) => nir::ExprKind::Binary {
                op: binop(*op),
                lhs: self.bx(a),
                rhs: self.bx(b),
            },
            ExprKind::Compare(first, rest) => nir::ExprKind::Compare {
                first: self.bx(first),
                rest: rest
                    .iter()
                    .map(|(op, x)| nir::CmpRest {
                        op: binop(*op),
                        rhs: self.expr(x),
                    })
                    .collect(),
            },
            ExprKind::Unary(op, x) => nir::ExprKind::Unary {
                op: match op {
                    ast::UnOp::Neg => "-",
                    ast::UnOp::BitNot => "~",
                    ast::UnOp::Not => "not",
                }
                .to_string(),
                operand: self.bx(x),
            },
            ExprKind::Range { lo, hi, inclusive } => nir::ExprKind::Range {
                lo: self.bx(lo),
                hi: self.bx(hi),
                inclusive: *inclusive,
            },
            ExprKind::Map(a, b) => nir::ExprKind::Map {
                from: self.bx(a),
                to: self.bx(b),
            },
            ExprKind::IfElse(c, a, b) => nir::ExprKind::IfElse {
                cond: self.bx(c),
                then: self.bx(a),
                otherwise: self.bx(b),
            },
            ExprKind::Pipe(a, b) => {
                let lhs = self.bx(a);
                // The function side of a pipe is typed as the whole pipe.
                let rhs = Box::new(self.expr_with(b, ty.clone()));
                nir::ExprKind::Pipe { lhs, rhs }
            }
            ExprKind::Call(callee, args) => {
                let target = self
                    .checked
                    .calls
                    .get(&key(self.mi, e.span))
                    .map(|t| self.call_target(t));
                nir::ExprKind::Call {
                    callee: self.bx(callee),
                    args: self.args(args),
                    target,
                }
            }
            ExprKind::Index(b, i, end) => nir::ExprKind::Index {
                base: self.bx(b),
                index: self.bx(i),
                end: end.as_ref().map(|x| self.bx(x)),
            },
            ExprKind::Field(..) if ipv4_text(e).is_some() => nir::ExprKind::Lit {
                lit: nir::Lit::Ip {
                    text: ipv4_text(e).unwrap_or_default(),
                },
            },
            ExprKind::Field(b, id) => {
                let target = match self.resolved.res.get(&key(self.mi, id.span)) {
                    Some(Res::Item { module, name }) => self.item_id(*module, name),
                    Some(Res::State { fsm, state }) => Some(format!("state:{fsm}.{state}")),
                    _ => None,
                };
                nir::ExprKind::Field {
                    base: self.bx(b),
                    name: id.name.clone(),
                    target,
                }
            }
            ExprKind::Try(x) => nir::ExprKind::Try {
                operand: self.bx(x),
            },
            ExprKind::Cast(x, t) => nir::ExprKind::Cast {
                operand: self.bx(x),
                to: self.type_expr(t),
            },
            ExprKind::Tuple(xs) => nir::ExprKind::Tuple {
                items: xs.iter().map(|x| self.expr(x)).collect(),
            },
            ExprKind::Array(xs) => nir::ExprKind::Array {
                items: xs.iter().map(|x| self.expr(x)).collect(),
            },
            ExprKind::ArrayRepeat(v, n) => nir::ExprKind::ArrayRepeat {
                value: self.bx(v),
                count: self.bx(n),
            },
            ExprKind::Lambda(params, body) => nir::ExprKind::Lambda {
                params: params
                    .iter()
                    .map(|p| nir::LambdaParam {
                        name: p.name.clone(),
                        id: self.decl_local(p),
                    })
                    .collect(),
                body: self.bx(body),
            },
            ExprKind::Old(x) => nir::ExprKind::Old {
                operand: self.bx(x),
            },
            ExprKind::Run(x) => nir::ExprKind::Run { action: self.bx(x) },
            // Programs with errors are never lowered; keep the slot well formed anyway.
            ExprKind::Error => nir::ExprKind::Global {
                name: "<error>".to_string(),
            },
        };
        nir::Expr {
            span: span(e.span),
            ty,
            e: kind,
        }
    }
}

fn asm(a: &ast::AsmBlock) -> nir::Asm {
    nir::Asm {
        arch: a.arch.name.clone(),
        operands: a.operands.clone(),
        lines: a.lines.clone(),
    }
}

/// `192.168.0.10` is read by the parser as the real `192.168` followed by the fields
/// `0` and `10` (spec 02 §2.8: an IP address is not a token). This recognizes that shape
/// when every part is a decimal octet and returns the address as written.
fn ipv4_text(e: &ast::Expr) -> Option<String> {
    let octet = |s: &str| {
        !s.is_empty()
            && s.len() <= 3
            && s.bytes().all(|b| b.is_ascii_digit())
            && s.parse::<u16>().is_ok_and(|v| v <= 255)
    };
    let ExprKind::Field(mid, d) = &e.kind else {
        return None;
    };
    let ExprKind::Field(base, c) = &mid.kind else {
        return None;
    };
    let ExprKind::Lit(ast::Lit::Real(ab)) = &base.kind else {
        return None;
    };
    let (a, b) = ab.split_once('.')?;
    [a, b, c.name.as_str(), d.name.as_str()]
        .iter()
        .all(|p| octet(p))
        .then(|| format!("{ab}.{}.{}", c.name, d.name))
}

fn lit(l: &ast::Lit) -> nir::Lit {
    let num = |t: &str| Rat::parse(t).map_or_else(|| t.to_string(), rat_text);
    match l {
        ast::Lit::Int(t) => nir::Lit::Int {
            text: t.clone(),
            v: num(t),
        },
        ast::Lit::Real(t) => nir::Lit::Real {
            text: t.clone(),
            v: num(t),
        },
        ast::Lit::Str(t) => nir::Lit::Str { text: t.clone() },
        ast::Lit::RawStr(t) => nir::Lit::RawStr { text: t.clone() },
        ast::Lit::ByteStr(t) => nir::Lit::ByteStr { text: t.clone() },
        ast::Lit::Char(t) => nir::Lit::Char { text: t.clone() },
        ast::Lit::Bool(b) => nir::Lit::Bool { v: *b },
        ast::Lit::None => nir::Lit::None,
        ast::Lit::Iec(t) => nir::Lit::Iec { text: t.clone() },
        ast::Lit::Bits(t) => nir::Lit::Bits { text: t.clone() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rationals_print_as_exact_decimals() {
        let r = |n, d| Rat::new(n, d).unwrap_or(Rat::ZERO);
        assert_eq!(rat_text(r(1, 2)), "0.5");
        assert_eq!(rat_text(r(1, 1000)), "0.001");
        assert_eq!(rat_text(r(-5, 4)), "-1.25");
        assert_eq!(rat_text(r(5463, 20)), "273.15");
        assert_eq!(rat_text(r(42, 1)), "42");
        assert_eq!(rat_text(r(1, 3)), "1/3");
        assert_eq!(rat_text(r(1, 60)), "1/60");
    }
}
