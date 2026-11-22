//! The partitioner (task D2.1): which modules and cell sequences run on which device.
//!
//! Rules (design D2):
//! 1. `place m on d` is taken as given. The device must belong to the cell, run the
//!    dialect of `m`, and, when `m` names a board (`module m for mcu on b`), be that board.
//!    Otherwise `NE0704`.
//! 2. A module that no cell of the package places, whose dialect is not `core` (library
//!    code), and whose board (if any) is the board of a device of the cell, is assigned
//!    automatically: to the device that runs its dialect and already holds the most
//!    modules sharing its signal and `io` names; ties go to the device declared first.
//!    When no device runs it, `NE0705` says what each device lacks. Candidates are taken
//!    in name order, so the plan does not depend on the order of the source files.
//! 3. A cell `sequence ... on d` runs on `d`; without `on` it runs on the first `plc`,
//!    else the first `host`.
//! 4. Every device gets a plan, even with no module: it still runs the link endpoints
//!    that task D3 adds.
use crate::{Backend, CellDiag, Kind, Library, Manifest};
use neuron_nir::{ItemDef, Module, ModuleKind, Nir};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// How a module came to a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum How {
    /// `place m on d`.
    Explicit,
    /// Chosen by the partitioner.
    Automatic,
}

/// A module placed on a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Placed {
    /// Module path.
    pub module: String,
    /// Its dialect.
    pub dialect: String,
    /// How it was placed.
    pub how: How,
}

/// What one device of a cell runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DevicePlan {
    /// Instance name in the cell (`plc`).
    pub instance: String,
    /// Manifest name (`generic_s7_1200`).
    pub manifest: String,
    /// Device class.
    pub kind: Kind,
    /// Backend that builds its program.
    pub backend: Backend,
    /// Emitter profile.
    pub profile: String,
    /// Modules, explicit placements first, then automatic ones in name order.
    pub modules: Vec<Placed>,
    /// Cell sequences that run on it.
    pub sequences: Vec<String>,
}

/// The partition of one cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellPlan {
    /// Cell name.
    pub cell: String,
    /// Source file of the cell.
    pub file: String,
    /// One entry per `device`, in source order.
    pub devices: Vec<DevicePlan>,
}

impl CellPlan {
    /// The plan of a device instance.
    pub fn device(&self, instance: &str) -> Option<&DevicePlan> {
        self.devices.iter().find(|d| d.instance == instance)
    }
}

/// Why `module` cannot run on a device described by `man`, if it cannot.
fn misfit(module: &Module, man: &Manifest) -> Option<String> {
    if !man.dialects.contains(&module.dialect) {
        return Some(format!(
            "runs {}, not `{}`",
            man.dialects.join(", "),
            module.dialect
        ));
    }
    if let Some(board) = &module.target {
        if man.target.as_deref() != Some(board.as_str()) {
            let is = man.target.as_deref().unwrap_or("not a controller board");
            return Some(format!(
                "the module is written for board `{board}`; this is {is}"
            ));
        }
    }
    None
}

/// Names a module defines that other modules can share (signals and `io` bindings).
fn shared_names(module: &Module) -> BTreeSet<String> {
    module
        .items
        .iter()
        .filter_map(|i| match &i.def {
            ItemDef::Signal { name, .. } | ItemDef::Io { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Dialects able to run a cell `sequence`.
const SEQUENCE_DIALECTS: &[&str] = &["plc", "robot", "mcu", "host"];

/// Partitions every cell of `nir`.
///
/// ```
/// use neuron_sema::{analyze, SemaOptions, SourceFile};
/// let src = |p: &str, t: &str| SourceFile { path: p.into(), text: t.into() };
/// let a = analyze(
///     vec![src("c.neuron", "cell C\n  device plc: generic_s7_1200 @ usb\n")],
///     None,
///     &SemaOptions::default(),
/// );
/// let lib = neuron_cell::Library::builtin().unwrap();
/// let plans = neuron_cell::partition(a.nir.as_ref().unwrap(), &lib).unwrap();
/// assert_eq!(plans[0].devices[0].instance, "plc");
/// ```
///
/// # Errors
/// Every `NE0607` (unknown device type), `NE0704` (impossible placement) and `NE0705`
/// (a module or sequence that fits no device) of every cell.
pub fn partition(nir: &Nir, lib: &Library) -> Result<Vec<CellPlan>, Vec<CellDiag>> {
    let modules: BTreeMap<&str, &Module> = nir
        .modules
        .iter()
        .filter(|m| m.kind == ModuleKind::Module)
        .map(|m| (m.name.as_str(), m))
        .collect();
    let placed_anywhere: BTreeSet<&str> = nir
        .modules
        .iter()
        .filter(|m| m.kind == ModuleKind::Cell)
        .flat_map(|c| c.items.iter())
        .filter_map(|i| match &i.def {
            ItemDef::Place { module, .. } => Some(module.as_str()),
            _ => None,
        })
        .collect();
    let mut plans = Vec::new();
    let mut diags = Vec::new();
    for cell in nir.modules.iter().filter(|m| m.kind == ModuleKind::Cell) {
        let (plan, mut d) = partition_cell(cell, &modules, &placed_anywhere, lib);
        diags.append(&mut d);
        plans.push(plan);
    }
    if diags.is_empty() {
        Ok(plans)
    } else {
        Err(diags)
    }
}

fn partition_cell(
    cell: &Module,
    modules: &BTreeMap<&str, &Module>,
    placed_anywhere: &BTreeSet<&str>,
    lib: &Library,
) -> (CellPlan, Vec<CellDiag>) {
    let file = cell.file.as_str();
    let mut diags = Vec::new();
    let mut devices: Vec<(DevicePlan, &Manifest)> = Vec::new();
    for item in &cell.items {
        if let ItemDef::Device { name, ty, .. } = &item.def {
            match lib.device(ty) {
                Some(man) => devices.push((
                    DevicePlan {
                        instance: name.clone(),
                        manifest: man.name.clone(),
                        kind: man.kind,
                        backend: man.backend,
                        profile: man.profile.clone(),
                        modules: Vec::new(),
                        sequences: Vec::new(),
                    },
                    man,
                )),
                None => diags.push(
                    CellDiag::new(
                        "NE0607",
                        file,
                        Some(item.span),
                        format!("unknown device `{ty}` for `{name}`"),
                    )
                    .help(format!("known devices: {}", lib.device_names().join(", "))),
                ),
            }
        }
    }
    let all_instances = devices
        .iter()
        .map(|(d, _)| d.instance.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let instances = || all_instances.clone();

    // 1. Explicit placements.
    let mut here: BTreeSet<String> = BTreeSet::new();
    for item in &cell.items {
        let ItemDef::Place { module, device } = &item.def else {
            continue;
        };
        let span = Some(item.span);
        let Some(idx) = devices.iter().position(|(d, _)| d.instance == *device) else {
            diags.push(
                CellDiag::new(
                    "NE0704",
                    file,
                    span,
                    format!("`{device}` is not a device of cell `{}`", cell.name),
                )
                .help(format!("devices: {}", instances())),
            );
            continue;
        };
        let Some(m) = modules.get(module.as_str()) else {
            diags.push(CellDiag::new(
                "NE0704",
                file,
                span,
                format!("no module `{module}` in the package"),
            ));
            continue;
        };
        if !here.insert(module.clone()) {
            diags.push(CellDiag::new(
                "NE0704",
                file,
                span,
                format!("module `{module}` is placed twice"),
            ));
            continue;
        }
        let (plan, man) = &mut devices[idx];
        if let Some(why) = misfit(m, man) {
            diags.push(
                CellDiag::new(
                    "NE0704",
                    file,
                    span,
                    format!(
                        "module `{module}` (`{}`) cannot run on `{device}` ({})",
                        m.dialect, man.name
                    ),
                )
                .help(format!("`{}` {why}", man.name)),
            );
            continue;
        }
        plan.modules.push(Placed {
            module: module.clone(),
            dialect: m.dialect.clone(),
            how: How::Explicit,
        });
    }

    // 2. Automatic assignment, in name order.
    let boards: BTreeSet<&str> = devices
        .iter()
        .filter_map(|(_, man)| man.target.as_deref())
        .collect();
    for (name, m) in modules {
        let candidate = !placed_anywhere.contains(name)
            && m.dialect != "core"
            && m.target.as_deref().is_none_or(|b| boards.contains(b));
        if !candidate {
            continue;
        }
        let mine = shared_names(m);
        let mut best: Option<(usize, usize)> = None; // (score, device index)
        let mut reasons = Vec::new();
        for (i, (plan, man)) in devices.iter().enumerate() {
            if let Some(why) = misfit(m, man) {
                reasons.push(format!("`{}` ({}) {why}", plan.instance, man.name));
                continue;
            }
            let score = plan
                .modules
                .iter()
                .filter_map(|p| modules.get(p.module.as_str()))
                .map(|other| shared_names(other).intersection(&mine).count())
                .sum::<usize>();
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, i));
            }
        }
        match best {
            Some((_, i)) => devices[i].0.modules.push(Placed {
                module: (*name).to_string(),
                dialect: m.dialect.clone(),
                how: How::Automatic,
            }),
            None => {
                let mut d = CellDiag::new(
                    "NE0705",
                    file,
                    None,
                    format!(
                        "module `{name}` (`{}`) fits no device of cell `{}`",
                        m.dialect, cell.name
                    ),
                );
                for r in reasons {
                    d = d.help(r);
                }
                if devices.is_empty() {
                    d = d.help("the cell declares no device");
                }
                diags.push(d.help(format!(
                    "place it explicitly, or add a device that runs `{}`",
                    m.dialect
                )));
            }
        }
    }

    // 3. Cell sequences.
    for item in &cell.items {
        let ItemDef::Sequence { name, on, .. } = &item.def else {
            continue;
        };
        let span = Some(item.span);
        let runs = |man: &Manifest| {
            man.dialects
                .iter()
                .any(|d| SEQUENCE_DIALECTS.contains(&d.as_str()))
        };
        let idx = match on {
            Some(dev) => match devices.iter().position(|(d, _)| d.instance == *dev) {
                Some(i) if runs(devices[i].1) => Some(i),
                Some(i) => {
                    diags.push(CellDiag::new(
                        "NE0704",
                        file,
                        span,
                        format!(
                            "sequence `{name}` cannot run on `{dev}` ({} runs {})",
                            devices[i].1.name,
                            devices[i].1.dialects.join(", ")
                        ),
                    ));
                    None
                }
                None => {
                    diags.push(
                        CellDiag::new(
                            "NE0704",
                            file,
                            span,
                            format!("`{dev}` is not a device of cell `{}`", cell.name),
                        )
                        .help(format!("devices: {}", instances())),
                    );
                    None
                }
            },
            None => {
                let found = [Kind::Plc, Kind::Host]
                    .iter()
                    .find_map(|k| devices.iter().position(|(d, _)| d.kind == *k));
                if found.is_none() {
                    diags.push(
                        CellDiag::new(
                            "NE0705",
                            file,
                            span,
                            format!(
                                "sequence `{name}` has no `on` and the cell has no plc or host"
                            ),
                        )
                        .help("write `sequence Name on <device>:`"),
                    );
                }
                found
            }
        };
        if let Some(i) = idx {
            devices[i].0.sequences.push(name.clone());
        }
    }

    let plan = CellPlan {
        cell: cell.name.clone(),
        file: file.to_string(),
        devices: devices.into_iter().map(|(d, _)| d).collect(),
    };
    (plan, diags)
}
