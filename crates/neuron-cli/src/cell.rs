//! `neuron cell check|build|sim|dossier` (Phase D, task D7): the cell compiler on the
//! command line. The work is done by `neuron-cell`; this module parses the arguments, runs
//! the analyzer, prints the diagnostics in the layout of the other commands and writes the
//! files. `neuron check` also calls [`check_package`]. Usage and details: `docs/cells.md`.
use crate::{load, USAGE};
use neuron_cell::{
    build, check_electrical, check_tags, cosimulate, dossier, duration_ns, fmt_ns, link_plans,
    link_table_with, partition, verify_cell, zone_plans, CellDiag, CellPlan, CellVerdict, Cut,
    DossierOptions, Drive, DriveValue, Library, LinkPlan, LinkReport, Replay, SimOptions,
    TimingOverlay,
};
use neuron_nir::{ModuleKind, Nir};
use neuron_sema::{analyze, SemaOptions, SourceFile};
use neuron_syntax::LineIndex;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// A diagnostic of the cell compiler with its file, line and source line, in the same
/// layout as the hardware checks.
pub fn render_diag(d: &CellDiag, paths: &[String], texts: &[String]) -> String {
    let mut out = format!("error[{}]: {}\n", d.code, d.message);
    match (paths.iter().position(|p| *p == d.file), d.span) {
        (Some(i), Some(span)) => {
            let text = &texts[i];
            let (line, col) = LineIndex::new(text).line_col(text, span[0]);
            out.push_str(&format!(" --> {}:{line}:{col}\n", d.file));
            if let Some(src) = text.lines().nth(line.saturating_sub(1)) {
                out.push_str(&format!("  |\n  | {src}\n"));
            }
        }
        _ => out.push_str(&format!(" --> {}\n", d.file)),
    }
    for h in &d.help {
        out.push_str(&format!("  = help: {h}\n"));
    }
    out
}

/// What the static checks found for one cell.
pub struct CellReport {
    /// The partition.
    pub plan: CellPlan,
    /// The link table (empty when a link has an error).
    pub rows: Vec<LinkReport>,
    /// The result of `verify:` and of the mutex proofs.
    pub verdict: CellVerdict,
}

/// `true` when the program has a `cell`.
pub fn has_cells(nir: &Nir) -> bool {
    nir.modules.iter().any(|m| m.kind == ModuleKind::Cell)
}

/// Runs partition, link table, tags, electrical, zones and cell verify on every cell of
/// `nir`. The diagnostics come in that order, without repeats.
pub fn check_cells(
    sources: &[SourceFile],
    nir: &Nir,
    lib: &Library,
) -> (Vec<CellReport>, Vec<CellDiag>) {
    check_cells_with(sources, nir, lib, &TimingOverlay::new())
}

/// [`check_cells`], but a calibration timing overlay (from `neuron cell check --profile`)
/// overrides the manifest timing used by the link budget (task E6.3).
pub fn check_cells_with(
    sources: &[SourceFile],
    nir: &Nir,
    lib: &Library,
    overlay: &TimingOverlay,
) -> (Vec<CellReport>, Vec<CellDiag>) {
    let plans = match partition(nir, lib) {
        Ok(p) => p,
        Err(d) => return (Vec::new(), d),
    };
    let mut diags: Vec<CellDiag> = Vec::new();
    let mut reports = Vec::new();
    for plan in plans {
        let rows = match link_table_with(nir, &plan, lib, overlay) {
            Ok(r) => r,
            Err(d) => {
                diags.extend(d);
                Vec::new()
            }
        };
        diags.extend(check_tags(nir, &plan, lib));
        let manifest_of = |i: &str| plan.device(i).map(|d| d.manifest.clone());
        diags.extend(check_electrical(&rows, &plan.file, &manifest_of, lib));
        let (zones, zone_diags) = zone_plans(nir, &plan, lib);
        diags.extend(zone_diags);
        let verdict = verify_cell(sources, nir, &plan, lib, &zones);
        diags.extend(verdict.diags.iter().cloned());
        reports.push(CellReport {
            plan,
            rows,
            verdict,
        });
    }
    let mut unique: Vec<CellDiag> = Vec::new();
    for d in diags {
        if !unique.contains(&d) {
            unique.push(d);
        }
    }
    (reports, unique)
}

/// The device library: the built-in manifests, then the `devices/` folder of every package
/// directory given, then the `--devices` folder.
fn library_for(paths: &[String], explicit: Option<&str>) -> Result<Library, ExitCode> {
    let fail = |e: &dyn std::fmt::Display| {
        eprintln!("error[NE0607]: {e}");
        ExitCode::from(1)
    };
    let mut lib = Library::builtin().map_err(|e| fail(&e))?;
    let mut folders: Vec<PathBuf> = paths
        .iter()
        .map(|p| Path::new(p).join("devices"))
        .filter(|d| d.is_dir())
        .collect();
    if let Some(e) = explicit {
        folders.push(PathBuf::from(e));
    }
    for f in folders {
        lib.overlay_dir(&f).map_err(|e| fail(&e))?;
    }
    lib.check().map_err(|e| fail(&e))?;
    Ok(lib)
}

/// The cell checks that `neuron check` runs after the analyzer: everything of
/// `neuron cell check` except the summary. `paths` are the command-line paths (a package
/// directory may carry its own `devices/`).
pub fn check_package(
    sources: &[SourceFile],
    nir: &Nir,
    paths: &[String],
) -> Result<Vec<CellDiag>, ExitCode> {
    if !has_cells(nir) {
        return Ok(Vec::new());
    }
    let lib = library_for(paths, None)?;
    // A package may hold modules that no device of a cell can run (a library of examples,
    // a host tool). `neuron cell check` reports them as `NE0705`; `neuron check` leaves them
    // out and checks the rest.
    let mut nir = nir.clone();
    let mut left_out = 0usize;
    while let Err(diags) = partition(&nir, &lib) {
        let loose: Vec<String> = diags
            .iter()
            .filter(|d| d.code == "NE0705")
            .filter_map(|d| unplaced_module(&d.message))
            .collect();
        if loose.is_empty() {
            break;
        }
        let before = nir.modules.len();
        nir.modules.retain(|m| !loose.contains(&m.name));
        left_out += before - nir.modules.len();
        if nir.modules.len() == before {
            break;
        }
    }
    if left_out > 0 {
        eprintln!(
            "note: {left_out} module(s) fit no device of a cell and were left out of the cell checks; \
             `neuron cell check` reports them as NE0705"
        );
    }
    Ok(check_cells(sources, &nir, &lib).1)
}

/// The module named by an `NE0705` message such as ``module `m` (`host`) fits no device``.
fn unplaced_module(message: &str) -> Option<String> {
    let rest = message.strip_prefix("module `")?;
    rest.split_once('`').map(|(name, _)| name.to_string())
}

/// The analyzed program and the device library.
struct Ctx {
    sources: Vec<SourceFile>,
    paths: Vec<String>,
    texts: Vec<String>,
    nir: Nir,
    lib: Library,
}

fn load_ctx(a: &Args) -> Result<Ctx, ExitCode> {
    let (sources, manifest) = load(&a.paths, false)?;
    let lib = library_for(&a.paths, a.devices.as_deref())?;
    let analysis = analyze(
        sources.clone(),
        manifest.as_deref(),
        &SemaOptions::default(),
    );
    eprint!("{}", analysis.render());
    let Some(nir) = analysis.nir.clone() else {
        eprintln!("error: the program has errors, so the cell was not checked");
        return Err(ExitCode::from(1));
    };
    if !has_cells(&nir) {
        eprintln!("error: no `cell` in the given files");
        return Err(ExitCode::from(1));
    }
    Ok(Ctx {
        sources,
        paths: analysis.paths,
        texts: analysis.texts,
        nir,
        lib,
    })
}

fn print_diags(ctx: &Ctx, diags: &[CellDiag]) {
    for d in diags {
        eprintln!("{}", render_diag(d, &ctx.paths, &ctx.texts));
    }
}

/// Parsed command line of the four subcommands.
#[derive(Default)]
struct Args {
    paths: Vec<String>,
    devices: Option<String>,
    cell: Option<String>,
    out: Option<String>,
    replay: Option<String>,
    end_ns: Option<u64>,
    cuts: Vec<String>,
    drives: Vec<String>,
    profiles: Vec<String>,
}

fn parse(cmd: &str, args: &[String], allowed: &[&str]) -> Result<Args, ExitCode> {
    let mut a = Args::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let flag = match arg.as_str() {
            "--output" | "--out" => "-o",
            other => other,
        };
        if flag.starts_with('-') && flag.len() > 1 {
            if !allowed.contains(&flag) {
                eprintln!("error: `neuron cell {cmd}` has no option `{arg}`\n\n{USAGE}");
                return Err(ExitCode::from(2));
            }
            let Some(v) = it.next() else {
                eprintln!("error: `{arg}` needs a value\n\n{USAGE}");
                return Err(ExitCode::from(2));
            };
            match flag {
                "-o" => a.out = Some(v.clone()),
                "--devices" => a.devices = Some(v.clone()),
                "--cell" => a.cell = Some(v.clone()),
                "--replay" => a.replay = Some(v.clone()),
                "--cut" => a.cuts.push(v.clone()),
                "--drive" => a.drives.push(v.clone()),
                "--profile" => a.profiles.push(v.clone()),
                _ => match duration_ns(v) {
                    Ok(ns) => a.end_ns = Some(ns),
                    Err(e) => {
                        eprintln!("error: --for: {e}");
                        return Err(ExitCode::from(2));
                    }
                },
            }
        } else {
            a.paths.push(arg.clone());
        }
    }
    if a.paths.is_empty() {
        eprintln!("error: `neuron cell {cmd}` needs at least one file or directory\n\n{USAGE}");
        return Err(ExitCode::from(2));
    }
    Ok(a)
}

/// `neuron cell <check|build|sim|dossier> ...`
pub fn cmd_cell(args: &[String]) -> ExitCode {
    let result = match args.first().map(String::as_str) {
        Some("check") => {
            parse("check", &args[1..], &["--devices", "--profile"]).and_then(|a| check(&a))
        }
        Some("build") => {
            parse("build", &args[1..], &["-o", "--cell", "--devices"]).and_then(|a| build_cmd(&a))
        }
        Some("sim") => parse(
            "sim",
            &args[1..],
            &[
                "--cell",
                "--devices",
                "--for",
                "--cut",
                "--drive",
                "--replay",
            ],
        )
        .and_then(|a| sim(&a)),
        Some("dossier") => parse(
            "dossier",
            &args[1..],
            &["-o", "--cell", "--devices", "--for", "--cut", "--drive"],
        )
        .and_then(|a| dossier_cmd(&a)),
        _ => {
            eprintln!("error: `cell` needs `check`, `build`, `sim` or `dossier`\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn lower(v: impl std::fmt::Debug) -> String {
    format!("{v:?}").to_lowercase()
}

fn print_summary(r: &CellReport) {
    println!("cell {} ({})", r.plan.cell, r.plan.file);
    println!("  devices");
    for d in &r.plan.devices {
        println!(
            "    {:<10} {:<22} {:<6} {:<8} {} module(s), {} sequence(s)",
            d.instance,
            d.manifest,
            lower(d.kind),
            d.backend.name(),
            d.modules.len(),
            d.sequences.len()
        );
    }
    if r.rows.is_empty() {
        println!("  links: none costed");
    } else {
        println!("  links");
        for (i, row) in r.rows.iter().enumerate() {
            let budget = row.within_ns.map_or("none".to_string(), fmt_ns);
            let met = if row.meets_budget() { "ok" } else { "OVER" };
            println!(
                "    l{i}  {} -> {}  {}  worst {}  budget {}  {met}  trust {}",
                row.from,
                row.to,
                row.medium,
                fmt_ns(row.latency.total_ns),
                budget,
                lower(row.trust)
            );
        }
    }
    for z in &r.verdict.zones {
        let state = if z.holds() {
            "mutex proved"
        } else {
            "mutex NOT proved"
        };
        println!("  zone {}: {state}", z.zone);
    }
    println!(
        "  verify: {} proved, {} violated",
        r.verdict.proved.len(),
        r.verdict.violations.len()
    );
    for n in &r.verdict.notes {
        println!("  note: {n}");
    }
    for v in &r.verdict.violations {
        println!("  counterexample for `{}`", v.property);
        for s in &v.trace {
            let inputs: Vec<String> = s
                .inputs
                .iter()
                .map(|(n, v)| format!("{n}={}", u8::from(*v)))
                .collect();
            println!(
                "    tick {:>3} t={} ns  {}  inputs: {}",
                s.tick,
                s.time_ns,
                s.states.join(", "),
                inputs.join(" ")
            );
        }
    }
}

/// Reads the `--profile` files into a timing overlay; a bad profile is `NE1112`.
fn load_overlay(a: &Args) -> Result<TimingOverlay, Vec<CellDiag>> {
    let mut overlay = TimingOverlay::new();
    let mut diags = Vec::new();
    for path in &a.profiles {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                if let Err(d) = overlay.load(path, &text) {
                    diags.push(d);
                }
            }
            Err(e) => diags.push(CellDiag::new(
                neuron_cell::PROFILE_INVALID,
                path,
                None,
                format!("cannot read profile `{path}`: {e}"),
            )),
        }
    }
    if diags.is_empty() {
        Ok(overlay)
    } else {
        Err(diags)
    }
}

/// `neuron cell check <file|dir>... [--devices dir] [--profile file.toml]...`
fn check(a: &Args) -> Result<(), ExitCode> {
    let ctx = load_ctx(a)?;
    let overlay = match load_overlay(a) {
        Ok(o) => o,
        Err(diags) => {
            print_diags(&ctx, &diags);
            eprintln!("error: {} profile error(s)", diags.len());
            return Err(ExitCode::from(1));
        }
    };
    if !overlay.is_empty() {
        eprintln!(
            "note: applying {} calibration profile(s); affected links report `measured`",
            a.profiles.len()
        );
    }
    let (reports, diags) = check_cells_with(&ctx.sources, &ctx.nir, &ctx.lib, &overlay);
    print_diags(&ctx, &diags);
    for r in &reports {
        print_summary(r);
    }
    if diags.is_empty() {
        println!("ok: no cell error (all numbers are as trusted as their manifests say)");
        Ok(())
    } else {
        eprintln!("error: {} cell error(s)", diags.len());
        Err(ExitCode::from(1))
    }
}

/// The plan of the cell to build, simulate or document.
fn select(ctx: &Ctx, name: Option<&str>) -> Result<CellPlan, ExitCode> {
    let plans = partition(&ctx.nir, &ctx.lib).map_err(|d| {
        print_diags(ctx, &d);
        ExitCode::from(1)
    })?;
    let names: Vec<String> = plans.iter().map(|p| p.cell.clone()).collect();
    match (name, plans.len()) {
        (Some(n), _) => plans.into_iter().find(|p| p.cell == n).ok_or_else(|| {
            eprintln!("error: no cell `{n}` (the cells are: {})", names.join(", "));
            ExitCode::from(1)
        }),
        (None, 1) => Ok(plans.into_iter().next().ok_or(ExitCode::from(1))?),
        (None, _) => {
            eprintln!(
                "error: several cells ({}); pick one with `--cell <name>`",
                names.join(", ")
            );
            Err(ExitCode::from(2))
        }
    }
}

fn out_dir(a: &Args, plan: &CellPlan) -> PathBuf {
    a.out
        .as_ref()
        .map_or_else(|| Path::new("build").join(&plan.cell), PathBuf::from)
}

/// Writes `text` to `dir/rel`; `rel` must stay inside `dir`.
fn write_rel(dir: &Path, rel: &str, text: &str) -> Result<(), ExitCode> {
    let unsafe_path = rel.is_empty()
        || rel.starts_with('/')
        || rel.starts_with('\\')
        || rel.contains(':')
        || rel.split(['/', '\\']).any(|s| s == "..");
    if unsafe_path {
        eprintln!("error: refusing to write `{rel}` outside {}", dir.display());
        return Err(ExitCode::from(1));
    }
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            eprintln!("error: cannot create {}: {e}", parent.display());
            ExitCode::from(2)
        })?;
    }
    std::fs::write(&path, text).map_err(|e| {
        eprintln!("error: cannot write {}: {e}", path.display());
        ExitCode::from(2)
    })
}

/// `neuron cell build <file|dir>... [--cell name] [-o dir]`
fn build_cmd(a: &Args) -> Result<(), ExitCode> {
    let ctx = load_ctx(a)?;
    let plan = select(&ctx, a.cell.as_deref())?;
    let (_, diags) = check_cells(&ctx.sources, &ctx.nir, &ctx.lib);
    let own: Vec<CellDiag> = diags
        .into_iter()
        .filter(|d| plan_owns(&ctx, &plan, d))
        .collect();
    if !own.is_empty() {
        print_diags(&ctx, &own);
        eprintln!("error: the cell has errors, so nothing was built");
        return Err(ExitCode::from(1));
    }
    let built = match build(&plan, &ctx.nir, &ctx.sources, &ctx.lib) {
        Ok(b) => b,
        Err(d) => {
            print_diags(&ctx, &d);
            eprintln!("error: the cell does not build, so nothing was written");
            return Err(ExitCode::from(1));
        }
    };
    let dir = out_dir(a, &plan);
    let mut count = 0usize;
    for dev in &built.devices {
        let sub = dir.join(&dev.instance);
        for (rel, text) in &dev.files {
            write_rel(&sub, rel, text)?;
            count += 1;
        }
        println!(
            "{}: {} ({}), {} file(s) in {}",
            dev.instance,
            dev.manifest,
            dev.backend.name(),
            dev.files.len(),
            sub.display()
        );
    }
    write_rel(&dir, "links.json", &neuron_emit::to_json(&built.links))?;
    write_rel(
        &dir,
        "watchdogs.json",
        &neuron_emit::to_json(&built.watchdogs),
    )?;
    eprintln!(
        "wrote {count} file(s) for {} device(s), links.json and watchdogs.json to {}",
        built.devices.len(),
        dir.display()
    );
    Ok(())
}

/// `true` when the diagnostic belongs to the cell of `plan` (its file, or a file whose
/// modules the plan places).
fn plan_owns(ctx: &Ctx, plan: &CellPlan, d: &CellDiag) -> bool {
    if d.file == plan.file {
        return true;
    }
    plan.devices
        .iter()
        .flat_map(|dev| dev.modules.iter())
        .any(|m| {
            ctx.nir
                .modules
                .iter()
                .any(|x| x.name == m.module && x.file == d.file)
        })
}

fn time_of(spec: &str) -> Result<u64, String> {
    duration_ns(spec.trim())
}

/// `l0`, `0` or `from->to` (spaces ignored): the position of a link.
fn link_of(spec: &str, plans: &[LinkPlan]) -> Result<usize, String> {
    let squashed: String = spec.chars().filter(|c| !c.is_whitespace()).collect();
    let digits = squashed.strip_prefix('l').unwrap_or(&squashed);
    if let Ok(i) = digits.parse::<usize>() {
        return if plans.iter().any(|p| p.index == i) {
            Ok(i)
        } else {
            Err(format!("the cell has no link number {i}"))
        };
    }
    plans
        .iter()
        .find(|p| format!("{}->{}", p.from, p.to).replace(' ', "") == squashed)
        .map(|p| p.index)
        .ok_or_else(|| {
            let known: Vec<String> = plans
                .iter()
                .map(|p| format!("l{} = {} -> {}", p.index, p.from, p.to))
                .collect();
            format!("no link `{spec}` (links: {})", known.join(", "))
        })
}

/// `link@time`, such as `l0@50ms` or `plc.motor->arm.go@120 ms`.
fn parse_cut(spec: &str, plans: &[LinkPlan]) -> Result<Cut, String> {
    let (link, at) = spec
        .rsplit_once('@')
        .ok_or_else(|| format!("`--cut {spec}`: write `<link>@<time>`, such as `l0@50ms`"))?;
    Ok(Cut {
        link: link_of(link, plans)?,
        at_ns: time_of(at).map_err(|e| format!("`--cut {spec}`: {e}"))?,
    })
}

/// `device.name=value[@time]`, such as `plc.start=true@10ms`.
fn parse_drive(spec: &str) -> Result<Drive, String> {
    let bad = || format!("`--drive {spec}`: write `<device>.<name>=<value>@<time>`");
    let (lhs, rhs) = spec.split_once('=').ok_or_else(bad)?;
    let (device, name) = lhs.trim().split_once('.').ok_or_else(bad)?;
    let (value, at) = match rhs.rsplit_once('@') {
        Some((v, t)) => (
            v.trim(),
            time_of(t).map_err(|e| format!("`--drive {spec}`: {e}"))?,
        ),
        None => (rhs.trim(), 0),
    };
    let value = match value {
        "true" => DriveValue::Bool(true),
        "false" => DriveValue::Bool(false),
        v => match v.parse::<i64>() {
            Ok(i) => DriveValue::Int(i),
            Err(_) => DriveValue::Real(v.parse::<f64>().map_err(|_| bad())?),
        },
    };
    Ok(Drive {
        t_ns: at,
        device: device.to_string(),
        name: name.to_string(),
        value,
    })
}

fn sim_options(ctx: &Ctx, plan: &CellPlan, a: &Args) -> Result<SimOptions, ExitCode> {
    let mut opts = SimOptions::default();
    if let Some(e) = a.end_ns {
        opts.end_ns = e;
    }
    let plans = link_plans(&ctx.nir, plan, &ctx.lib).map_err(|d| {
        print_diags(ctx, &d);
        ExitCode::from(1)
    })?;
    for c in &a.cuts {
        opts.cuts.push(parse_cut(c, &plans).map_err(|e| {
            eprintln!("error: {e}");
            ExitCode::from(2)
        })?);
    }
    for d in &a.drives {
        opts.drives.push(parse_drive(d).map_err(|e| {
            eprintln!("error: {e}");
            ExitCode::from(2)
        })?);
    }
    Ok(opts)
}

/// `neuron cell sim <file|dir>... [--cell name] [--for T] [--cut link@t]... [--drive d.n=v@t]...
/// [--replay out.json]`
fn sim(a: &Args) -> Result<(), ExitCode> {
    let ctx = load_ctx(a)?;
    let plan = select(&ctx, a.cell.as_deref())?;
    let opts = sim_options(&ctx, &plan, a)?;
    let run = cosimulate(&plan, &ctx.nir, &ctx.sources, &ctx.lib, &opts).map_err(|e| {
        match &e {
            neuron_cell::SimError::Cell(d) => print_diags(&ctx, d),
            other => eprintln!("error: {other}"),
        }
        ExitCode::from(1)
    })?;
    print!("{}", run.render());
    for l in &run.losses {
        let fault = l.fault_ns.map_or("no fault raised".to_string(), |f| {
            format!("fault at {}", fmt_ns(f))
        });
        println!(
            "loss of l{} at {}: `{}` {fault}, bound {} ({})",
            l.link,
            fmt_ns(l.cut_ns),
            l.device,
            fmt_ns(l.bound_ns),
            if l.within_bound {
                "within"
            } else {
                "NOT within"
            }
        );
    }
    for n in &run.notes {
        eprintln!("note: {n}");
    }
    eprintln!(
        "ran {} of cell time on {} device(s), {} event(s), {} delivery(ies), {} cut(s)",
        fmt_ns(run.end_ns),
        run.devices.len(),
        run.events.len(),
        run.deliveries.len(),
        run.cuts.len()
    );
    if let Some(path) = &a.replay {
        let json = Replay::from_run(&run).to_json();
        std::fs::write(path, json).map_err(|e| {
            eprintln!("error: cannot write {path}: {e}");
            ExitCode::from(2)
        })?;
        eprintln!("wrote the replay to {path}");
    }
    Ok(())
}

/// `neuron cell dossier <file|dir>... [--cell name] [-o dir] [--for T] [--cut ..] [--drive ..]`
fn dossier_cmd(a: &Args) -> Result<(), ExitCode> {
    let ctx = load_ctx(a)?;
    let plan = select(&ctx, a.cell.as_deref())?;
    let sim = sim_options(&ctx, &plan, a)?;
    let d = dossier(
        &plan,
        &ctx.nir,
        &ctx.sources,
        &ctx.lib,
        &DossierOptions { sim: Some(sim) },
    );
    let dir = out_dir(a, &plan);
    write_rel(&dir, "dossier.md", &d.markdown)?;
    write_rel(&dir, "dossier.html", &d.html)?;
    for s in &d.build_status {
        let codes = if s.codes.is_empty() {
            String::new()
        } else {
            format!(" ({})", s.codes.join(", "))
        };
        println!("{}: {}{codes}", s.device, s.state);
    }
    eprintln!(
        "wrote dossier.md and dossier.html to {} (numbers: {})",
        dir.display(),
        d.trust
    );
    Ok(())
}
