//! Tags across devices (task D3.4, requirement D-R6, spec 11.2, `NE0703`).
//!
//! `tag t: T @ d1.addr, d2.name` maps one value onto several devices. [`check_tags`]
//! checks every location of every `tag` of a cell:
//!
//! * the device is an instance of the cell (else `NE0607`, as for links);
//! * an **IEC direct address** (`plc.%MD20`) is well formed, starts with an address
//!   family of the device manifest's `[io]` tables, and has the size of the tag's type
//!   ([`address_fits`]);
//! * a **symbol** (`ctl.speed`) agrees in type and unit with the `signal`, `var`, PLC
//!   variable or `io` binding of that name in a module placed on the device, when one
//!   exists;
//! * the tag has **one writer**.
//!
//! Locations that are not an address or a plain symbol (`plc.db(1).x0`, `opcua(..)`,
//! `mqtt(..)`) are not checked yet: their syntax belongs to the transport adapters.
//! Every problem of a tag mapping is `NE0703`.
//!
//! # Address sizes
//!
//! A direct address is `%`, an area (`I` input, `Q` output, `M` memory), an optional size
//! letter and a number: `X` or none is one bit, `B` 8 bits, `W` 16, `D` 32, `L` 64. A
//! `bool` tag needs a bit address; any other scalar type needs the address whose width
//! is the width of the type (`f32`, `i32`, `u32` at `%MD`; `u16` at `%MW`; `u8` at
//! `%MB`; 64-bit types at `%ML`). The address must start with the prefix of one `[io]`
//! family of the device, or share its area (`%MD20` fits a family `%M` or `%MW`).
//!
//! # Single writer
//!
//! A location is a **write location** when the program of its device produces the value:
//! a symbol that a module placed on the device assigns (`name = ...`, `name += ...` in a
//! rung, task, rule, function or state machine); an output address (`%Q...`), which only
//! the program writes; or a memory address that a placed PLC variable declares with
//! `@ address` and the module assigns. Input addresses (`%I...`) are written by the
//! hardware and a symbol nobody assigns is a reader. A tag whose write locations are on
//! more than one device has two writers, which is `NE0703`; the message lists the writers
//! in name order, so the result does not depend on the order of the locations.
use crate::{CellDiag, CellPlan, Library, Manifest};
use neuron_nir::{
    Expr, ExprKind, ItemDef, Lit, Module, ModuleKind, Nir, Type, TypeExpr, TypeExprKind,
};
use serde_json::Value;
use std::collections::BTreeSet;

/// A parsed IEC direct address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectAddress {
    /// Area letter: `I`, `Q` or `M`.
    pub area: char,
    /// Width in bits: 1, 8, 16, 32 or 64.
    pub bits: u32,
}

/// Parses `%MD20`, `%IX0.1`, `%I0.0`, `%QB4`. `None` when `text` is not a direct address.
///
/// ```
/// let a = neuron_cell::parse_direct_address("%MD20").unwrap();
/// assert_eq!((a.area, a.bits), ('M', 32));
/// assert_eq!(neuron_cell::parse_direct_address("%I0.0").unwrap().bits, 1);
/// assert!(neuron_cell::parse_direct_address("%MD").is_none());
/// ```
pub fn parse_direct_address(text: &str) -> Option<DirectAddress> {
    let rest = text.strip_prefix('%')?;
    let mut chars = rest.chars();
    let area = chars.next().filter(|c| matches!(c, 'I' | 'Q' | 'M'))?;
    let after_area = chars.as_str();
    let (bits, number) = match after_area.chars().next() {
        Some('X') => (1, &after_area[1..]),
        Some('B') => (8, &after_area[1..]),
        Some('W') => (16, &after_area[1..]),
        Some('D') => (32, &after_area[1..]),
        Some('L') => (64, &after_area[1..]),
        _ => (1, after_area),
    };
    let valid = number.chars().next().is_some_and(|c| c.is_ascii_digit())
        && number.chars().all(|c| c.is_ascii_digit() || c == '.');
    valid.then_some(DirectAddress { area, bits })
}

/// Width in bits of a scalar Neuron type (`bool` 1, `u8` 8, ... `f64` 64).
pub fn type_bits(kind: &str) -> Option<u32> {
    match kind {
        "bool" => Some(1),
        "u8" | "i8" => Some(8),
        "u16" | "i16" => Some(16),
        "u32" | "i32" | "f32" => Some(32),
        "u64" | "i64" | "f64" => Some(64),
        _ => None,
    }
}

/// Whether a value of scalar type `kind` fits the direct address `address`: a `bool`
/// at a bit address, any other type at the address of its own width.
///
/// ```
/// assert!(neuron_cell::address_fits("f32", "%MD20"));
/// assert!(!neuron_cell::address_fits("f32", "%MW20"));
/// assert!(neuron_cell::address_fits("bool", "%QX0.0"));
/// ```
pub fn address_fits(kind: &str, address: &str) -> bool {
    match (type_bits(kind), parse_direct_address(address)) {
        (Some(t), Some(a)) => t == a.bits,
        _ => false,
    }
}

/// Whether the address starts with an `[io]` prefix of the manifest, or shares its area
/// with one (`%MD20` against `%M` or `%MW`).
fn prefix_ok(man: &Manifest, address: &str) -> bool {
    man.io.values().any(|f| {
        address.starts_with(&f.prefix)
            || (f.prefix.starts_with('%')
                && f.prefix.len() >= 2
                && address.len() >= 2
                && f.prefix.get(..2) == address.get(..2))
    })
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `(kind, unit)` of a tag type that is a scalar: `f32 mm/s`, `u32`, `bool`, `degC`.
fn tag_scalar(ty: &TypeExpr) -> Option<(String, String)> {
    let unit = ty
        .unit
        .as_ref()
        .map(|u| squash(&u.text))
        .unwrap_or_default();
    match &ty.kind {
        TypeExprKind::Named { path, args } if args.is_empty() && type_bits(path).is_some() => {
            Some((path.clone(), unit))
        }
        TypeExprKind::Unit if !unit.is_empty() => Some(("f32".to_string(), unit)),
        _ => None,
    }
}

fn show(kind: &str, unit: &str) -> String {
    if unit.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} {unit}")
    }
}

/// `(kind, unit)` of a resolved type, for the types that can be compared.
fn decl_scalar(t: &Type) -> Option<(String, String)> {
    match t {
        Type::Bool => Some(("bool".into(), String::new())),
        Type::Num { kind, unit } => Some((
            kind.clone(),
            unit.as_ref().map(|u| squash(&u.text)).unwrap_or_default(),
        )),
        _ => None,
    }
}

enum Loc {
    Address(String, String),
    Symbol(String, String),
    /// A device-rooted location of another shape (`plc.db(1).x0`).
    Other(String),
    /// Not rooted at a device (`opcua("...")`).
    None,
}

fn simple_name(e: &Expr) -> Option<&str> {
    match &e.e {
        ExprKind::Global { name } | ExprKind::Ref { name, .. } | ExprKind::Local { name, .. } => {
            Some(name)
        }
        _ => None,
    }
}

fn root(e: &Expr) -> Option<&str> {
    match &e.e {
        ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => root(base),
        ExprKind::Call { callee, .. } => root(callee),
        _ => simple_name(e),
    }
}

fn classify(e: &Expr) -> Loc {
    match &e.e {
        ExprKind::Field { base, name, .. } => match simple_name(base) {
            Some(dev) if name.starts_with('%') => Loc::Address(dev.to_string(), name.clone()),
            Some(dev) => Loc::Symbol(dev.to_string(), name.clone()),
            None => root(base).map_or(Loc::None, |d| Loc::Other(d.to_string())),
        },
        _ => Loc::None,
    }
}

fn placed_modules<'a>(nir: &'a Nir, plan: &CellPlan, device: &str) -> Vec<&'a Module> {
    plan.device(device)
        .into_iter()
        .flat_map(|d| d.modules.iter())
        .filter_map(|p| {
            nir.modules
                .iter()
                .find(|m| m.kind == ModuleKind::Module && m.name == p.module)
        })
        .collect()
}

/// The declared type of `name` in the modules placed on `device`.
fn declared(modules: &[&Module], name: &str) -> Option<Type> {
    for m in modules {
        for item in &m.items {
            match &item.def {
                ItemDef::Signal {
                    name: n,
                    vty: Some(t),
                    ..
                } if n == name => return Some(t.clone()),
                ItemDef::Var { var } if var.name == name => return Some(var.vty.clone()),
                ItemDef::PlcSection { vars, .. } => {
                    if let Some(v) = vars.iter().find(|v| v.name == name) {
                        return Some(v.vty.clone());
                    }
                }
                ItemDef::Io { name: n, .. } if n == name => return Some(Type::Bool),
                _ => {}
            }
        }
    }
    None
}

/// Whether a statement tree assigns the module-level name `name`.
fn assigns(v: &Value, name: &str) -> bool {
    match v {
        Value::Object(o) => {
            if o.get("s").and_then(Value::as_str) == Some("assign") {
                let target = o
                    .get("place")
                    .and_then(|p| p.get("e"))
                    .filter(|k| {
                        matches!(
                            k.get("e").and_then(Value::as_str),
                            Some("ref" | "global" | "local")
                        )
                    })
                    .and_then(|k| k.get("name"))
                    .and_then(Value::as_str);
                if target == Some(name) {
                    return true;
                }
            }
            o.values().any(|x| assigns(x, name))
        }
        Value::Array(a) => a.iter().any(|x| assigns(x, name)),
        _ => false,
    }
}

fn module_assigns(m: &Module, name: &str) -> bool {
    m.items.iter().any(|i| {
        serde_json::to_value(&i.def)
            .map(|v| assigns(&v, name))
            .unwrap_or(false)
    })
}

/// The variable a placed PLC module binds to the address `address` (`x: f32 @ %MD20`).
fn var_at<'a>(modules: &[&'a Module], address: &str) -> Option<(&'a Module, String)> {
    for m in modules {
        for item in &m.items {
            let ItemDef::PlcSection { vars, .. } = &item.def else {
                continue;
            };
            for v in vars {
                let Some(at) = &v.at else { continue };
                if let ExprKind::Lit {
                    lit: Lit::Iec { text },
                } = &at.e
                {
                    if text.eq_ignore_ascii_case(address) {
                        return Some((m, v.name.clone()));
                    }
                }
            }
        }
    }
    None
}

/// Whether the program of the device produces the value at this location (see the
/// module documentation for the rule).
fn is_writer(modules: &[&Module], loc: &Loc) -> bool {
    match loc {
        Loc::Symbol(_, name) => modules.iter().any(|m| module_assigns(m, name)),
        Loc::Address(_, text) => match parse_direct_address(text) {
            Some(a) if a.area == 'Q' => true,
            Some(a) if a.area == 'M' => {
                var_at(modules, text).is_some_and(|(m, name)| module_assigns(m, &name))
            }
            _ => false,
        },
        _ => false,
    }
}

/// `NE0703` for every tag mapping of the cell of `plan` that is invalid, and `NE0607`
/// for a location on a device the cell does not declare. Diagnostics come in tag order,
/// then location order.
pub fn check_tags(nir: &Nir, plan: &CellPlan, lib: &Library) -> Vec<CellDiag> {
    let Some(cell) = nir
        .modules
        .iter()
        .find(|m| m.kind == ModuleKind::Cell && m.name == plan.cell && m.file == plan.file)
    else {
        return Vec::new();
    };
    let mut diags = Vec::new();
    for item in &cell.items {
        let ItemDef::Tag {
            name,
            ty,
            locations,
        } = &item.def
        else {
            continue;
        };
        let scalar = tag_scalar(ty);
        let tag_text = scalar
            .as_ref()
            .map_or_else(|| "?".to_string(), |(k, u)| show(k, u));
        let mut writers: BTreeSet<String> = BTreeSet::new();
        for e in locations {
            let loc = classify(e);
            let dev = match &loc {
                Loc::Address(d, _) | Loc::Symbol(d, _) | Loc::Other(d) => d.clone(),
                Loc::None => continue,
            };
            let span = Some(e.span);
            let Some(dp) = plan.device(&dev) else {
                diags.push(
                    CellDiag::new(
                        "NE0607",
                        &plan.file,
                        span,
                        format!("tag `{name}`: `{dev}` is not a device of this cell"),
                    )
                    .help(format!(
                        "devices: {}",
                        plan.devices
                            .iter()
                            .map(|d| d.instance.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                );
                continue;
            };
            let Some(man) = lib.device(&dp.manifest) else {
                continue;
            };
            let modules = placed_modules(nir, plan, &dev);
            let mut bad = |msg: String, help: Option<String>| {
                let d = CellDiag::new("NE0703", &plan.file, span, msg);
                diags.push(match help {
                    Some(h) => d.help(h),
                    None => d,
                });
            };
            match &loc {
                Loc::Address(_, text) => {
                    let Some(addr) = parse_direct_address(text) else {
                        bad(
                            format!("tag `{name}`: `{dev}.{text}` is not an IEC direct address"),
                            Some("write `%` an area (I, Q, M), a size (X, B, W, D, L) and a number, as in `%MD20`".into()),
                        );
                        continue;
                    };
                    if !prefix_ok(man, text) {
                        let prefixes: Vec<&str> =
                            man.io.values().map(|f| f.prefix.as_str()).collect();
                        bad(
                            format!(
                                "tag `{name}`: `{text}` is not an address family of `{}` (`{dev}`)",
                                man.name
                            ),
                            Some(format!(
                                "address families of {}: {}",
                                man.name,
                                prefixes.join(", ")
                            )),
                        );
                        continue;
                    }
                    match &scalar {
                        Some((kind, _)) if !address_fits(kind, text) => bad(
                            format!(
                                "tag `{name}`: `{kind}` is {} bits and `{dev}.{text}` is {}",
                                type_bits(kind).unwrap_or(0),
                                size_text(addr.bits)
                            ),
                            Some(format!("{} at {}", kind, fitting(kind))),
                        ),
                        Some(_) => {}
                        None => bad(
                            format!(
                                "tag `{name}`: its type cannot be placed at the address `{dev}.{text}`"
                            ),
                            Some("only `bool` and the fixed-width numbers (`u8` ... `f64`) have a direct address".into()),
                        ),
                    }
                }
                Loc::Symbol(_, sym) => {
                    if let (Some((tk, tu)), Some(t)) = (&scalar, declared(&modules, sym)) {
                        match decl_scalar(&t) {
                            Some((dk, du)) if (&dk, &du) == (tk, tu) => {}
                            Some((dk, du)) => bad(
                                format!(
                                    "tag `{name}` is `{tag_text}` but `{dev}.{sym}` is `{}`",
                                    show(&dk, &du)
                                ),
                                Some(format!(
                                    "a tag carries one type and unit on every device: change the tag or `{sym}` on `{dev}`"
                                )),
                            ),
                            None => bad(
                                format!(
                                    "tag `{name}` is `{tag_text}` but `{dev}.{sym}` has a type that a tag cannot carry"
                                ),
                                None,
                            ),
                        }
                    }
                }
                _ => {}
            }
            if is_writer(&modules, &loc) {
                writers.insert(dev);
            }
        }
        if writers.len() > 1 {
            let list: Vec<String> = writers.into_iter().collect();
            diags.push(
                CellDiag::new(
                    "NE0703",
                    &plan.file,
                    Some(item.span),
                    format!(
                        "tag `{name}` has {} writers: {}",
                        list.len(),
                        list.join(", ")
                    ),
                )
                .help("a tag has one owner: keep the write location on one device and make the others read it (an input address, or a symbol the module does not assign)"),
            );
        }
    }
    diags
}

fn size_text(bits: u32) -> String {
    if bits == 1 {
        "a bit address".to_string()
    } else {
        format!("{bits} bits")
    }
}

/// The address letters that fit `kind`, for the help line.
fn fitting(kind: &str) -> &'static str {
    match type_bits(kind) {
        Some(1) => "`%MX`, `%IX` or `%QX`",
        Some(8) => "`%MB`, `%IB` or `%QB`",
        Some(16) => "`%MW`, `%IW` or `%QW`",
        Some(32) => "`%MD`, `%ID` or `%QD`",
        Some(64) => "`%ML`, `%IL` or `%QL`",
        _ => "no direct address",
    }
}
