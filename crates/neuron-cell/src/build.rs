//! The build of every device of a cell (task D2.2) through the existing backends and
//! emitters: C for controller boards, Verilog and constraints for FPGA boards, Structured
//! Text, SCL or PLCopen XML for PLCs, SPEL+-style programs for robots, Python for hosts.
//!
//! Each device gets its own NIR package: the modules placed on it plus every `core`
//! module of the package (library code they may use). A controller module that names no
//! board gets the board of the device manifest. The link endpoints (task D3.1) are Neuron
//! source generated per device and link side, analyzed together with the cell's sources
//! and added to the package; when a backend cannot take them the device builds without
//! them and the link plan says so in its notes. A device with nothing placed and no link
//! builds to an empty file set. Each device with links also gets `links.json` and, for
//! register media, `register_map.csv`; the endpoint sources are kept under `links/`.
//! Cell sequences are added by a later task.
use crate::{
    endpoint_sources, link_plans, register_csv, Backend, CellDiag, CellPlan, DevicePlan,
    EndpointSource, Library, LinkPlan, Manifest,
};
use neuron_nir::{ModuleKind, Nir};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use serde::Serialize;
use std::collections::BTreeMap;

/// The generated files of one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceBuild {
    /// Instance name in the cell.
    pub instance: String,
    /// Manifest name.
    pub manifest: String,
    /// Backend used.
    pub backend: Backend,
    /// Modules built, in plan order.
    pub modules: Vec<String>,
    /// Files by relative path (`program.st`, `fidelity.json`, `top.v`, ...).
    pub files: BTreeMap<String, String>,
}

/// The generated files of a whole cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellBuild {
    /// Cell name.
    pub cell: String,
    /// One entry per device, in plan order.
    pub devices: Vec<DeviceBuild>,
    /// The plan of every link (endpoints, register map, crossing), in link order.
    pub links: Vec<LinkPlan>,
}

impl CellBuild {
    /// The build of a device instance.
    pub fn device(&self, instance: &str) -> Option<&DeviceBuild> {
        self.devices.iter().find(|d| d.instance == instance)
    }
}

/// The NIR package of one device, without link endpoints.
pub fn device_package(nir: &Nir, dev: &DevicePlan, man: &Manifest) -> Nir {
    device_package_with(nir, dev, man, &[])
}

/// The NIR package of one device: its placed modules, then the named endpoint modules
/// (which `nir` must contain), then the `core` libraries.
pub fn device_package_with(
    nir: &Nir,
    dev: &DevicePlan,
    man: &Manifest,
    endpoints: &[String],
) -> Nir {
    let mut placed: Vec<&str> = dev.modules.iter().map(|p| p.module.as_str()).collect();
    placed.extend(endpoints.iter().map(String::as_str));
    let mut out = nir.clone();
    out.modules.retain(|m| {
        m.kind == ModuleKind::Module && (placed.contains(&m.name.as_str()) || m.dialect == "core")
    });
    if !placed.is_empty() {
        // Placed modules first, in plan order, then the endpoints, then the libraries.
        out.modules.sort_by_key(|m| {
            placed
                .iter()
                .position(|p| *p == m.name)
                .unwrap_or(placed.len())
        });
    } else {
        out.modules.clear();
    }
    if let Some(board) = &man.target {
        for m in &mut out.modules {
            if m.dialect == "mcu" && m.target.is_none() {
                m.target = Some(board.clone());
            }
        }
    }
    out
}

fn diag(
    code: &'static str,
    file: &str,
    span: [usize; 2],
    message: &str,
    help: &Option<String>,
) -> CellDiag {
    let d = CellDiag::new(code, file, Some(span), message);
    match help {
        Some(h) => d.help(h.clone()),
        None => d,
    }
}

/// Builds every device of `plan`. `sources` are the analyzed source files (the C
/// backend uses them for its source maps).
///
/// # Errors
/// The diagnostics of every backend that refused its device program, prefixed with the
/// device in the message.
pub fn build(
    plan: &CellPlan,
    nir: &Nir,
    sources: &[SourceFile],
    lib: &Library,
) -> Result<CellBuild, Vec<CellDiag>> {
    let mut links = link_plans(nir, plan, lib)?;
    let mut by_device = endpoint_sources(nir, plan, lib, &links);
    let mut devices = Vec::new();
    let mut diags = Vec::new();
    for dev in &plan.devices {
        let Some(man) = lib.device(&dev.manifest) else {
            diags.push(CellDiag::new(
                "NE0607",
                &plan.file,
                None,
                format!("unknown device `{}`", dev.manifest),
            ));
            continue;
        };
        let eps = by_device.remove(&dev.instance).unwrap_or_default();
        let mut built_endpoints = false;
        let mut files = None;
        if !eps.is_empty() {
            match build_with_endpoints(dev, man, sources, &eps) {
                Ok(f) => {
                    built_endpoints = true;
                    files = Some(Ok(f));
                }
                Err(why) => {
                    // Fidelity note instead of a failed build: the program of the
                    // device still builds without its endpoints.
                    for l in &mut links {
                        if l.tx.device == dev.instance || l.rx.device == dev.instance {
                            l.notes.push(format!(
                                "the endpoint module of `{}` is not part of its {} program: {why}",
                                dev.instance,
                                man.backend.name()
                            ));
                        }
                    }
                }
            }
        }
        let files = files.unwrap_or_else(|| {
            let pkg = device_package(nir, dev, man);
            if pkg.modules.is_empty() {
                Ok(BTreeMap::new())
            } else {
                build_device(&pkg, man, sources)
            }
        });
        match files {
            Ok(mut files) => {
                if built_endpoints {
                    for e in &eps {
                        files.insert(format!("links/{}", e.file), e.text.clone());
                    }
                }
                devices.push(DeviceBuild {
                    instance: dev.instance.clone(),
                    manifest: dev.manifest.clone(),
                    backend: dev.backend,
                    modules: dev.modules.iter().map(|p| p.module.clone()).collect(),
                    files,
                });
            }
            Err(ds) => diags.extend(ds.into_iter().map(|mut d| {
                d.message = format!("device `{}`: {}", dev.instance, d.message);
                d
            })),
        }
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    // Per-device link data, after every note is known.
    for d in &mut devices {
        let mine: Vec<&LinkPlan> = links
            .iter()
            .filter(|l| l.tx.device == d.instance || l.rx.device == d.instance)
            .collect();
        if mine.is_empty() {
            continue;
        }
        let doc = serde_json::json!({ "device": d.instance, "links": mine });
        if let Ok(mut text) = serde_json::to_string_pretty(&doc) {
            text.push('\n');
            d.files.insert("links.json".into(), text);
        }
        if let Some(csv) = register_csv(&links, &d.instance) {
            d.files.insert("register_map.csv".into(), csv);
        }
    }
    Ok(CellBuild {
        cell: plan.cell.clone(),
        devices,
        links,
    })
}

/// Analyzes the cell's sources together with the endpoint modules of one device and
/// builds the device package with them. The error is a one-line reason.
fn build_with_endpoints(
    dev: &DevicePlan,
    man: &Manifest,
    sources: &[SourceFile],
    eps: &[EndpointSource],
) -> Result<BTreeMap<String, String>, String> {
    let mut all: Vec<SourceFile> = sources.to_vec();
    all.extend(eps.iter().map(|e| SourceFile {
        path: e.file.clone(),
        text: e.text.clone(),
    }));
    let a = analyze(all.clone(), None, &SemaOptions::default());
    if let Some(e) = a.errors().next() {
        return Err(format!(
            "the generated endpoint source was rejected: {} {}",
            e.diag.code, e.diag.message
        ));
    }
    let Some(nir2) = a.nir.as_ref() else {
        return Err("the generated endpoint source gave no NIR".to_string());
    };
    let names: Vec<String> = eps.iter().map(|e| e.module.clone()).collect();
    let pkg = device_package_with(nir2, dev, man, &names);
    build_device(&pkg, man, &all).map_err(|ds| {
        ds.first().map_or_else(
            || "the backend refused the program".to_string(),
            |d| format!("{} {}", d.code, d.message),
        )
    })
}

fn build_device(
    pkg: &Nir,
    man: &Manifest,
    sources: &[SourceFile],
) -> Result<BTreeMap<String, String>, Vec<CellDiag>> {
    match man.backend {
        Backend::St | Backend::Scl | Backend::Plcopen | Backend::Spel | Backend::Python => {
            let Some(emitter) = neuron_emit::emitter(man.backend.name()) else {
                return Err(vec![CellDiag::new(
                    "NE0607",
                    "",
                    None,
                    format!("no emitter `{}`", man.backend.name()),
                )]);
            };
            let opts = neuron_emit::EmitOptions {
                profile: man.profile.clone(),
                robot: if man.backend == Backend::Spel {
                    man.name.clone()
                } else {
                    String::new()
                },
            };
            emitter
                .emit(pkg, &opts)
                .map(|p| p.all_files())
                .map_err(|ds| {
                    ds.iter()
                        .map(|d| diag(d.code, &d.file, d.span, &d.message, &d.help))
                        .collect()
                })
        }
        Backend::C => build_c(pkg, sources),
        Backend::Verilog => build_verilog(pkg, man),
        Backend::Native => Err(vec![CellDiag::new(
            "NE0607",
            "",
            None,
            format!(
                "`{}` uses the native backend, which cells do not build yet; use `backend = \"c\"` or `neuron build --native`",
                man.name
            ),
        )]),
    }
}

fn build_c(pkg: &Nir, sources: &[SourceFile]) -> Result<BTreeMap<String, String>, Vec<CellDiag>> {
    let reg = neuron_target::Registry::builtin()
        .map_err(|e| vec![CellDiag::new("NE0607", "targets", None, e.to_string())])?;
    let to_cell = |ds: &[neuron_target::Diag]| -> Vec<CellDiag> {
        ds.iter()
            .map(|d| diag(d.code, &d.file, d.span, &d.message, &d.help))
            .collect()
    };
    let hw = neuron_target::check(&reg, pkg);
    if !hw.diags.is_empty() {
        return Err(to_cell(&hw.diags));
    }
    let Some(target) = hw.target.clone() else {
        return Err(vec![CellDiag::new(
            "NE0607",
            "",
            None,
            "no board for the controller program",
        )]);
    };
    let srcs: Vec<neuron_mir::Source> = sources
        .iter()
        .map(|s| neuron_mir::Source {
            file: s.path.clone(),
            text: s.text.clone(),
        })
        .collect();
    let mut lowered = neuron_mir::lower(pkg, &hw, &srcs);
    if !lowered.diags.is_empty() {
        return Err(to_cell(&lowered.diags));
    }
    if lowered.program.board.is_none() {
        lowered.program.board = Some(target.board.name.clone());
        lowered.program.sysclk_hz = hw.sysclk_hz;
    }
    let problems = neuron_mir::verify(&lowered.program);
    if !problems.is_empty() {
        return Err(vec![CellDiag::new(
            "NE0610",
            "",
            None,
            format!(
                "internal: the MIR verifier rejected the program: {}",
                problems.join("; ")
            ),
        )]);
    }
    let ui = neuron_ui::lower_ui(pkg, target.chip.ram_bytes, Some(&lowered.program))
        .map_err(|ds| to_cell(&ds))?;
    let emitted = match &ui {
        Some(app) => {
            neuron_codegen_c::emit_c_ui(&lowered.program, &neuron_ui::emit_ui_c(app), app.period_ms)
        }
        None => neuron_codegen_c::emit_c(&lowered.program),
    };
    let name = if lowered.program.name.is_empty() {
        "program".to_string()
    } else {
        lowered.program.name.clone()
    };
    let mut files = BTreeMap::new();
    files.insert(format!("{name}.c"), emitted.c);
    files.insert(format!("{name}.c.map.json"), emitted.map_json);
    Ok(files)
}

fn build_verilog(pkg: &Nir, man: &Manifest) -> Result<BTreeMap<String, String>, Vec<CellDiag>> {
    let hdl = |ds: Vec<neuron_codegen_hdl::HdlDiag>| -> Vec<CellDiag> {
        ds.iter()
            .map(|d| diag(d.code, &d.file, d.span, &d.message, &d.help))
            .collect()
    };
    let v = neuron_codegen_hdl::emit_verilog(pkg).map_err(hdl)?;
    let mut files = BTreeMap::new();
    files.insert("top.v".to_string(), v.text.clone());
    let board = man
        .board
        .as_deref()
        .and_then(neuron_codegen_hdl::FpgaBoard::builtin);
    if let Some(board) = board {
        let top = v.modules.last().cloned().unwrap_or_default();
        let constraints = neuron_codegen_hdl::emit_constraints(&v, &top, &board).map_err(hdl)?;
        let ext = match board.family {
            neuron_codegen_hdl::Family::Ice40 => "pcf",
            neuron_codegen_hdl::Family::Xc7 => "xdc",
        };
        files.insert(format!("top.{ext}"), constraints);
    }
    Ok(files)
}
