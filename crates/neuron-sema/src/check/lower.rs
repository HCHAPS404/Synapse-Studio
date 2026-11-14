//! Lowering of syntactic types to semantic types and construction of definitions.

use super::{Checker, DefInfo, FieldInfo, FnKey};
use crate::model::Res;
use crate::prelude;
use crate::ty::{FnTy, NumKind, ParamTy, Ty};
use crate::units::lookup_unit;
use neuron_syntax::ast::{FnSig, GenericParam, ItemKind, Path, Type, TypeArg, TypeKind};

pub(super) fn generic_names(gs: &[GenericParam]) -> Vec<String> {
    gs.iter()
        .map(|g| match g {
            GenericParam::Type { name, .. } | GenericParam::Const { name, .. } => name.name.clone(),
        })
        .collect()
}

pub(super) fn generic_entry(g: &GenericParam) -> (String, Vec<String>) {
    match g {
        GenericParam::Type { name, bounds, .. } => (
            name.name.clone(),
            bounds
                .iter()
                .filter_map(|b| b.segments.last().map(|s| s.name.clone()))
                .collect(),
        ),
        GenericParam::Const { name, .. } => (name.name.clone(), Vec::new()),
    }
}

fn int(signed: bool, bits: u8) -> Ty {
    Ty::num(NumKind::Int { signed, bits })
}

impl Checker<'_> {
    /// Lowers a type in the current context.
    pub(crate) fn lower(&mut self, t: &Type) -> Ty {
        let gens: Vec<String> = self.cur.generics.iter().map(|(n, _)| n.clone()).collect();
        self.lower_in(self.cur.module, t, &gens, 0)
    }

    /// Lowers a type with extra generic names in scope.
    pub(crate) fn lower_with(&mut self, t: &Type, extra: &[String]) -> Ty {
        let mut gens: Vec<String> = self.cur.generics.iter().map(|(n, _)| n.clone()).collect();
        gens.extend(extra.iter().cloned());
        self.lower_in(self.cur.module, t, &gens, 0)
    }

    pub(super) fn lower_in(&mut self, module: usize, t: &Type, gens: &[String], depth: u8) -> Ty {
        if depth > 16 {
            return Ty::Unknown;
        }
        let base = match &t.kind {
            TypeKind::Named { path, args } => self.lower_named(module, path, args, gens, depth),
            TypeKind::Array(inner, len) => {
                let elem = self.lower_in(module, inner, gens, depth + 1);
                let n = self
                    .consts
                    .int(module, len)
                    .and_then(|n| u64::try_from(n).ok());
                Ty::Array(Box::new(elem), n)
            }
            TypeKind::Slice(inner) => {
                Ty::Slice(Box::new(self.lower_in(module, inner, gens, depth + 1)))
            }
            TypeKind::Tuple(ts) => Ty::Tuple(
                ts.iter()
                    .map(|x| self.lower_in(module, x, gens, depth + 1))
                    .collect(),
            ),
            TypeKind::Unit => Ty::num(NumKind::Float(32)),
        };
        let base = match (&t.unit, base) {
            (Some(u), Ty::Num(k, old)) => match self.prog.units.resolve(u) {
                Ok(unit) if k != NumKind::Duration => Ty::Num(k, unit),
                _ => Ty::Num(k, old),
            },
            (_, b) => b,
        };
        if t.optional {
            Ty::Option(Box::new(base))
        } else {
            base
        }
    }

    fn arg_int(&self, module: usize, args: &[TypeArg], i: usize) -> Option<u64> {
        match args.get(i)? {
            TypeArg::Expr(e) => self
                .consts
                .int(module, e)
                .and_then(|n| u64::try_from(n).ok()),
            TypeArg::Type(_) => None,
        }
    }

    fn arg_ty(
        &mut self,
        module: usize,
        args: &[TypeArg],
        i: usize,
        gens: &[String],
        depth: u8,
    ) -> Ty {
        match args.get(i) {
            Some(TypeArg::Type(t)) => self.lower_in(module, t, gens, depth + 1),
            _ => Ty::Unknown,
        }
    }

    fn lower_named(
        &mut self,
        module: usize,
        path: &Path,
        args: &[TypeArg],
        gens: &[String],
        depth: u8,
    ) -> Ty {
        let Some(first) = path.segments.first() else {
            return Ty::Unknown;
        };
        if path.segments.len() == 1 && gens.contains(&first.name) {
            return Ty::Param(first.name.clone());
        }
        let found = self.prog.lookup(module, &first.name);
        let (m, name) = match (found, path.segments.get(1)) {
            (Some(Res::Module(m)), Some(second)) => (m, second.name.clone()),
            (Some(Res::Item { module: m, name }), _) => (m, name),
            (Some(Res::Foreign(_)), _) => return Ty::Opaque(first.name.clone()),
            _ => {
                return self
                    .prim(module, &first.name, args, gens, depth)
                    .unwrap_or(Ty::Unknown)
            }
        };
        let Some(sym) = self.prog.symbol(m, &name) else {
            return Ty::Unknown;
        };
        let kind = sym.kind;
        let item_idx = sym.item;
        use crate::model::SymKind;
        match kind {
            SymKind::Struct | SymKind::Enum | SymKind::Class => {
                let targs = (0..args.len())
                    .filter(|i| matches!(args[*i], TypeArg::Type(_)))
                    .map(|i| self.arg_ty(module, args, i, gens, depth))
                    .collect();
                Ty::Adt {
                    def: (m, name),
                    args: targs,
                }
            }
            SymKind::Interface => Ty::Interface((m, name)),
            SymKind::Alias => {
                let Some(item) = self
                    .prog
                    .files
                    .get(m)
                    .and_then(|f| f.ast.items.get(item_idx))
                else {
                    return Ty::Unknown;
                };
                let ItemKind::Alias { generics, ty, .. } = &item.kind else {
                    return Ty::Unknown;
                };
                let names = generic_names(generics);
                let inner = self.lower_in(m, ty, &names, depth + 1);
                let subst: Vec<(String, Ty)> = names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.clone(), self.arg_ty(module, args, i, gens, depth)))
                    .collect();
                inner.subst(&subst)
            }
            _ => Ty::Unknown,
        }
    }

    fn prim(
        &mut self,
        module: usize,
        name: &str,
        args: &[TypeArg],
        gens: &[String],
        depth: u8,
    ) -> Option<Ty> {
        Some(match name {
            "bool" => Ty::Bool,
            "u8" => int(false, 8),
            "u16" => int(false, 16),
            "u32" => int(false, 32),
            "u64" => int(false, 64),
            "i8" => int(true, 8),
            "i16" => int(true, 16),
            "i32" => int(true, 32),
            "i64" => int(true, 64),
            "index" => Ty::num(NumKind::Index),
            "f32" => Ty::num(NumKind::Float(32)),
            "f64" => Ty::num(NumKind::Float(64)),
            "duration" => Ty::Num(NumKind::Duration, lookup_unit("s")?),
            "instant" => Ty::Instant,
            "str" => Ty::Str(
                self.arg_int(module, args, 0)
                    .and_then(|n| u32::try_from(n).ok()),
            ),
            "bit" | "sbit" => Ty::Bits {
                signed: name == "sbit",
                width: self
                    .arg_int(module, args, 0)
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(1),
            },
            "fixed" | "ufixed" => {
                let i = self
                    .arg_int(module, args, 0)
                    .and_then(|n| u8::try_from(n).ok());
                let f = self
                    .arg_int(module, args, 1)
                    .and_then(|n| u8::try_from(n).ok());
                match (i, f) {
                    (Some(int), Some(frac)) => Ty::num(NumKind::Fixed {
                        signed: name == "fixed",
                        int,
                        frac,
                    }),
                    _ => Ty::Unknown,
                }
            }
            "range" => Ty::Range(Box::new(self.arg_ty(module, args, 0, gens, depth))),
            "Option" => Ty::Option(Box::new(self.arg_ty(module, args, 0, gens, depth))),
            "Result" => Ty::Result(
                Box::new(self.arg_ty(module, args, 0, gens, depth)),
                Box::new(self.arg_ty(module, args, 1, gens, depth)),
            ),
            "Handle" => Ty::Handle(Box::new(self.arg_ty(module, args, 0, gens, depth))),
            "channel" => Ty::Channel(
                Box::new(self.arg_ty(module, args, 0, gens, depth)),
                self.arg_int(module, args, 1),
            ),
            "pool" => Ty::Pool(
                Box::new(self.arg_ty(module, args, 0, gens, depth)),
                self.arg_int(module, args, 1),
            ),
            other if prelude::is_type_name(other) => Ty::Opaque(other.to_string()),
            _ => return None,
        })
    }

    /// Signature of a function or method.
    pub(super) fn sig_ty(&mut self, module: usize, sig: &FnSig, outer: &[String]) -> FnTy {
        let own: Vec<(String, Vec<String>)> = sig.generics.iter().map(generic_entry).collect();
        let mut gens: Vec<String> = outer.to_vec();
        gens.extend(own.iter().map(|(n, _)| n.clone()));
        let mut params = Vec::new();
        let mut self_mode = None;
        for p in &sig.params {
            if p.name.name == "self" {
                self_mode = Some(p.mutable);
                continue;
            }
            let ty = match &p.ty {
                Some(t) => self.lower_in(module, t, &gens, 0),
                None => Ty::Unknown,
            };
            params.push(ParamTy {
                name: p.name.name.clone(),
                ty,
                mutable: p.mutable,
                has_default: p.default.is_some(),
            });
        }
        let ret = match &sig.ret {
            Some(t) => self.lower_in(module, t, &gens, 0),
            None => Ty::Void,
        };
        FnTy {
            params,
            ret,
            generics: own,
            self_mode,
        }
    }

    /// Builds user type definitions and function signatures for every module.
    pub(super) fn build_defs(&mut self) {
        let prog = self.prog;
        for (mi, f) in prog.files.iter().enumerate() {
            if f.syntax_errors {
                continue;
            }
            for item in &f.ast.items {
                match &item.kind {
                    ItemKind::Struct(def) => {
                        let gens = generic_names(&def.generics);
                        let fields = def
                            .members
                            .iter()
                            .map(|fl| FieldInfo {
                                name: fl.name.name.clone(),
                                ty: self.lower_in(mi, &fl.ty, &gens, 0),
                                mutable: true,
                                has_default: fl.default.is_some(),
                            })
                            .collect();
                        self.out.defs.insert(
                            (mi, def.name.name.clone()),
                            DefInfo::Struct {
                                generics: gens,
                                fields,
                            },
                        );
                    }
                    ItemKind::Enum(def) => {
                        let gens = generic_names(&def.generics);
                        let variants = def
                            .members
                            .iter()
                            .map(|v| {
                                (
                                    v.name.name.clone(),
                                    v.payload
                                        .iter()
                                        .map(|t| self.lower_in(mi, t, &gens, 0))
                                        .collect(),
                                )
                            })
                            .collect();
                        self.out.defs.insert(
                            (mi, def.name.name.clone()),
                            DefInfo::Enum {
                                generics: gens,
                                variants,
                            },
                        );
                    }
                    ItemKind::Class(c) => {
                        let gens = generic_names(&c.generics);
                        let fields = c
                            .vars
                            .iter()
                            .map(|v| FieldInfo {
                                name: v.name.name.clone(),
                                ty: match &v.ty {
                                    Some(t) => self.lower_in(mi, t, &gens, 0),
                                    None => Ty::Unknown,
                                },
                                mutable: v.mutable,
                                has_default: v.init.is_some(),
                            })
                            .collect();
                        let interfaces = c
                            .interfaces
                            .iter()
                            .filter_map(|p| {
                                let first = p.segments.first()?;
                                match prog.lookup(mi, &first.name)? {
                                    Res::Item { module, name } => Some((module, name)),
                                    _ => None,
                                }
                            })
                            .collect();
                        for m in &c.methods {
                            if let ItemKind::Fn(func) = &m.kind {
                                let sig = self.sig_ty(mi, &func.sig, &gens);
                                self.out.sigs.insert(
                                    FnKey {
                                        module: mi,
                                        owner: Some(c.name.name.clone()),
                                        name: func.sig.name.name.clone(),
                                    },
                                    sig,
                                );
                            }
                        }
                        self.out.defs.insert(
                            (mi, c.name.name.clone()),
                            DefInfo::Class {
                                generics: gens,
                                fields,
                                interfaces,
                            },
                        );
                    }
                    ItemKind::Interface {
                        name,
                        generics,
                        methods,
                    } => {
                        let gens = generic_names(generics);
                        let map = methods
                            .iter()
                            .map(|m| (m.name.name.clone(), self.sig_ty(mi, m, &gens)))
                            .collect();
                        self.out
                            .defs
                            .insert((mi, name.name.clone()), DefInfo::Interface { methods: map });
                    }
                    ItemKind::Fn(func) | ItemKind::ExternFn { func, .. } => {
                        let sig = self.sig_ty(mi, &func.sig, &[]);
                        self.out.sigs.insert(
                            FnKey {
                                module: mi,
                                owner: None,
                                name: func.sig.name.name.clone(),
                            },
                            sig,
                        );
                    }
                    _ => {}
                }
            }
        }
    }
}
