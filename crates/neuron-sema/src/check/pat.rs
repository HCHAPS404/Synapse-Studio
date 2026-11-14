//! Patterns and match exhaustiveness (NE0103).

use super::{Checker, DefInfo};
use crate::model::SymKind;
use crate::resolve::LocalKind;
use crate::ty::{DefRef, NumKind, Ty};
use neuron_syntax::ast::{Arm, ExprKind, Lit, Path, Pattern, PatternKind};
use neuron_syntax::Span;

fn irrefutable(p: &Pattern) -> bool {
    match &p.kind {
        PatternKind::Wildcard | PatternKind::Bind(_) => true,
        PatternKind::Or(ps) => ps.iter().any(irrefutable),
        PatternKind::Tuple(ps) => ps.iter().all(irrefutable),
        _ => false,
    }
}

fn flatten<'p>(p: &'p Pattern, out: &mut Vec<&'p Pattern>) {
    match &p.kind {
        PatternKind::Or(ps) => {
            for x in ps {
                flatten(x, out);
            }
        }
        _ => out.push(p),
    }
}

impl Checker<'_> {
    fn pattern_variant(&self, path: &Path, scrut: &Ty) -> Option<(DefRef, String)> {
        match path.segments.as_slice() {
            [head, second] => match self.res_of(head)? {
                crate::model::Res::Item { module, name }
                    if self.symbol_kind(*module, name) == Some(SymKind::Enum) =>
                {
                    Some(((*module, name.clone()), second.name.clone()))
                }
                _ => None,
            },
            [only] => match scrut {
                Ty::Adt { def, .. } => match self.out.defs.get(def) {
                    Some(DefInfo::Enum { variants, .. })
                        if variants.iter().any(|(v, _)| *v == only.name) =>
                    {
                        Some((def.clone(), only.name.clone()))
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// Checks a pattern against the type of the value it matches and binds its names.
    pub(crate) fn pattern(&mut self, p: &Pattern, scrut: &Ty) {
        match &p.kind {
            PatternKind::Wildcard => {}
            PatternKind::Bind(id) => self.set_local(id, scrut.clone()),
            PatternKind::Lit(e) => {
                let t = self.expr(e, Some(scrut));
                self.coerce(scrut, &t, e);
            }
            PatternKind::Range { lo, hi, .. } => {
                for e in [lo, hi] {
                    let t = self.expr(e, Some(scrut));
                    self.coerce(scrut, &t, e);
                }
            }
            PatternKind::Path(path) => {
                if let Some((def, v)) = self.pattern_variant(path, scrut) {
                    self.variant_pattern(&def, &v, &[], scrut, p.span);
                }
            }
            PatternKind::Tuple(ps) => match scrut {
                Ty::Tuple(ts) if ts.len() == ps.len() => {
                    for (x, t) in ps.iter().zip(ts) {
                        self.pattern(x, t);
                    }
                }
                Ty::Unknown | Ty::Opaque(_) => {
                    for x in ps {
                        self.pattern(x, &Ty::Unknown);
                    }
                }
                other => {
                    self.err(
                        "NE0208",
                        format!("a tuple pattern cannot match `{}`", other.display()),
                        p.span,
                    );
                    for x in ps {
                        self.pattern(x, &Ty::Unknown);
                    }
                }
            },
            PatternKind::Or(ps) => {
                for x in ps {
                    self.pattern(x, scrut);
                }
            }
            PatternKind::Constructor { path, args, fields } => {
                let single = if path.segments.len() == 1 {
                    path.segments.first().map(|s| s.name.as_str())
                } else {
                    None
                };
                if let (Some(name @ ("Ok" | "Err")), Ty::Result(a, b)) = (single, scrut) {
                    let inner = if name == "Ok" {
                        (**a).clone()
                    } else {
                        (**b).clone()
                    };
                    for x in args {
                        self.pattern(x, &inner);
                    }
                    return;
                }
                if let Some((def, v)) = self.pattern_variant(path, scrut) {
                    self.variant_pattern(&def, &v, args, scrut, p.span);
                    return;
                }
                if let Some(first) = path.segments.first() {
                    if let Some(crate::model::Res::Item { module, name }) =
                        self.res_of(first).cloned()
                    {
                        if let Some(DefInfo::Struct {
                            fields: defs,
                            generics,
                        }) = self.out.defs.get(&(module, name.clone())).cloned()
                        {
                            let targs: Vec<Ty> = match scrut {
                                Ty::Adt { args, .. } => args.clone(),
                                _ => Vec::new(),
                            };
                            let subst: Vec<(String, Ty)> = generics
                                .iter()
                                .cloned()
                                .zip(targs.into_iter().chain(std::iter::repeat(Ty::Unknown)))
                                .collect();
                            if let Ty::Adt { def, .. } = scrut {
                                if *def != (module, name.clone()) {
                                    self.err(
                                        "NE0208",
                                        format!(
                                            "pattern `{name}` cannot match `{}`",
                                            scrut.display()
                                        ),
                                        p.span,
                                    );
                                }
                            }
                            for (i, x) in args.iter().enumerate() {
                                let t = defs.get(i).map_or(Ty::Unknown, |f| f.ty.subst(&subst));
                                self.pattern(x, &t);
                            }
                            for (fid, x) in fields {
                                match defs.iter().find(|f| f.name == fid.name) {
                                    Some(f) => {
                                        let t = f.ty.subst(&subst);
                                        self.pattern(x, &t);
                                    }
                                    None => {
                                        self.err(
                                            "NE0212",
                                            format!("`{name}` has no field `{}`", fid.name),
                                            fid.span,
                                        );
                                        self.pattern(x, &Ty::Unknown);
                                    }
                                }
                            }
                            return;
                        }
                    }
                }
                for x in args {
                    self.pattern(x, &Ty::Unknown);
                }
                for (_, x) in fields {
                    self.pattern(x, &Ty::Unknown);
                }
            }
        }
    }

    fn variant_pattern(&mut self, def: &DefRef, v: &str, args: &[Pattern], scrut: &Ty, span: Span) {
        if let Ty::Adt { def: d2, .. } = scrut {
            if d2 != def {
                self.err(
                    "NE0208",
                    format!("pattern `{}.{v}` cannot match `{}`", def.1, scrut.display()),
                    span,
                );
                for x in args {
                    self.pattern(x, &Ty::Unknown);
                }
                return;
            }
        }
        let Some(DefInfo::Enum { generics, variants }) = self.out.defs.get(def).cloned() else {
            return;
        };
        let Some((_, payload)) = variants.iter().find(|(n, _)| n == v) else {
            self.err(
                "NE0212",
                format!("enum `{}` has no variant `{v}`", def.1),
                span,
            );
            for x in args {
                self.pattern(x, &Ty::Unknown);
            }
            return;
        };
        if payload.len() != args.len() {
            self.err(
                "NE0209",
                format!(
                    "variant `{}.{v}` carries {} value(s), the pattern has {}",
                    def.1,
                    payload.len(),
                    args.len()
                ),
                span,
            );
        }
        let targs: Vec<Ty> = match scrut {
            Ty::Adt { args, .. } => args.clone(),
            _ => Vec::new(),
        };
        let subst: Vec<(String, Ty)> = generics
            .iter()
            .cloned()
            .zip(targs.into_iter().chain(std::iter::repeat(Ty::Unknown)))
            .collect();
        for (i, x) in args.iter().enumerate() {
            let t = payload.get(i).map_or(Ty::Unknown, |t| t.subst(&subst));
            self.pattern(x, &t);
        }
    }

    /// Reports `NE0103` when the arms of a `match` do not cover every value.
    pub(crate) fn check_exhaustive(&mut self, scrut: &Ty, arms: &[Arm], span: Span) {
        let unguarded: Vec<&Pattern> = arms
            .iter()
            .filter(|a| a.guard.is_none())
            .map(|a| &a.pattern)
            .collect();
        if unguarded.iter().any(|p| irrefutable(p)) {
            return;
        }
        let mut flat: Vec<&Pattern> = Vec::new();
        for p in &unguarded {
            flatten(p, &mut flat);
        }
        let missing: Option<String> = match scrut {
            Ty::Bool => {
                let has = |b: bool| {
                    flat.iter().any(|p| {
                        matches!(&p.kind, PatternKind::Lit(e) if matches!(&e.kind, ExprKind::Lit(Lit::Bool(x)) if *x == b))
                    })
                };
                match (has(true), has(false)) {
                    (true, true) => None,
                    (false, _) => Some("`true`".into()),
                    (_, false) => Some("`false`".into()),
                }
            }
            Ty::Adt { def, .. } => match self.out.defs.get(def).cloned() {
                Some(DefInfo::Enum { variants, .. }) => {
                    let covered: Vec<String> = flat
                        .iter()
                        .filter_map(|p| match &p.kind {
                            PatternKind::Path(path) => {
                                self.pattern_variant(path, scrut).map(|(_, v)| v)
                            }
                            PatternKind::Constructor { path, args, fields } => {
                                if args.iter().all(irrefutable)
                                    && fields.iter().all(|(_, x)| irrefutable(x))
                                {
                                    self.pattern_variant(path, scrut).map(|(_, v)| v)
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        })
                        .collect();
                    let miss: Vec<String> = variants
                        .iter()
                        .filter(|(v, _)| !covered.contains(v))
                        .map(|(v, _)| format!("`{}.{v}`", def.1))
                        .collect();
                    if miss.is_empty() {
                        None
                    } else {
                        Some(miss.join(", "))
                    }
                }
                _ => None,
            },
            Ty::Option(_) => Some("a value (add a `_` arm)".into()),
            Ty::Result(..) => {
                let has = |n: &str| {
                    flat.iter().any(|p| match &p.kind {
                        PatternKind::Constructor { path, args, .. } => {
                            path.segments.len() == 1
                                && path.segments[0].name == n
                                && args.iter().all(irrefutable)
                        }
                        _ => false,
                    })
                };
                match (has("Ok"), has("Err")) {
                    (true, true) => None,
                    (false, _) => Some("`Ok(_)`".into()),
                    (_, false) => Some("`Err(_)`".into()),
                }
            }
            Ty::Num(k, _) => self.missing_ints(k, &flat),
            Ty::Bits { width, .. } if *width < 127 => {
                self.missing_ints_range(0, (1i128 << *width) - 1, &flat)
            }
            Ty::Str(_) | Ty::Array(..) => Some("other values (add a `_` arm)".into()),
            _ => None,
        };
        if let Some(m) = missing {
            self.err_help(
                "NE0103",
                format!("non-exhaustive `match`: missing {m}"),
                span,
                "add the missing arms or a `_` arm",
            );
        }
    }

    fn missing_ints(&self, k: &NumKind, flat: &[&Pattern]) -> Option<String> {
        match k.int_range() {
            Some((lo, hi)) => self.missing_ints_range(lo, hi, flat),
            None => match k {
                NumKind::LitInt => None,
                _ => Some("other values (add a `_` arm)".into()),
            },
        }
    }

    fn missing_ints_range(&self, lo: i128, hi: i128, flat: &[&Pattern]) -> Option<String> {
        let m = self.cur.module;
        let mut spans: Vec<(i128, i128)> = Vec::new();
        for p in flat {
            match &p.kind {
                PatternKind::Lit(e) => {
                    let v = self.consts.int(m, e)?;
                    spans.push((v, v));
                }
                PatternKind::Range {
                    lo: a,
                    hi: b,
                    inclusive,
                } => match (self.consts.int(m, a), self.consts.int(m, b)) {
                    (Some(x), Some(y)) => {
                        let y = if *inclusive { y } else { y - 1 };
                        if y >= x {
                            spans.push((x, y));
                        }
                    }
                    _ => return None,
                },
                _ => return None,
            }
        }
        spans.sort_unstable();
        let mut next = lo;
        for (a, b) in spans {
            if a > next {
                break;
            }
            if b >= next {
                next = b + 1;
            }
        }
        if next > hi {
            None
        } else {
            Some(format!("values from {next} (add a `_` arm)"))
        }
    }

    /// Kind of a local, used by tests and later analyses.
    #[allow(dead_code)]
    pub(crate) fn local_kind_of(&self, i: usize) -> Option<LocalKind> {
        self.resolved.locals.get(i).map(|l| l.kind)
    }
}
