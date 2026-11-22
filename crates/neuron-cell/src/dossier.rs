//! The technical dossier of a cell (task D5.1): one Markdown document and one
//! self-contained HTML file (inline CSS and inline SVG, no scripts, no external
//! resources) generated only from data the cell compiler already computes. A PDF is
//! printed from the HTML by the user.
//!
//! Sections: summary (with the trust level of the numbers), build status per device,
//! wiring list, I/O list per device, pin and address tables, link table with budgets,
//! timing diagram, bill of materials, safety report (electrical, zones and mutexes,
//! watchdog bounds, cell verify, tags) and the test report of a co-simulation.
//!
//! The text is deterministic: no timestamps, no hash-map order, the same inputs give
//! byte-identical output. A part that cannot be computed (a link over its budget, a
//! device whose backend refuses its program) is shown with its diagnostic codes instead
//! of being left out.
//!
//! The dossier is a design aid. It is not a certified safety document.
use crate::electrical::{input_volts, output_volts};
use crate::{
    build, check_electrical, check_tags, cosimulate, fmt_ns, link_plans, link_table, register_map,
    verify_cell, watchdog_plans, zone_plans, Backend, CellBuild, CellDiag, CellEventKind, CellPlan,
    CellVerdict, Family, Handshake, Kind, Library, LinkModel, LinkPlan, LinkReport, Location,
    Pattern, SimError, SimOptions, SimRun, Trust, ZonePlan,
};
use neuron_nir::{ItemDef, Nir, Type};
use neuron_sema::SourceFile;
use serde::Serialize;
use std::collections::BTreeMap;

/// The safety wording of the product, repeated in the summary and the safety report.
pub const SAFETY_WORDING: &str = "This dossier is a design aid, not a certified safety tool. \
It makes no claim of compliance with IEC 61508 or ISO 13849. A hardwired E-STOP is the authority.";

/// What the dossier runs besides the static checks.
#[derive(Debug, Clone)]
pub struct DossierOptions {
    /// The co-simulation that becomes the test report; `None` leaves the test report
    /// empty ("not run").
    pub sim: Option<SimOptions>,
}

impl Default for DossierOptions {
    fn default() -> Self {
        DossierOptions {
            sim: Some(SimOptions::default()),
        }
    }
}

/// The build state of one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildStatus {
    /// Instance name.
    pub device: String,
    /// Manifest name.
    pub manifest: String,
    /// Backend of the manifest.
    pub backend: Backend,
    /// `built`, `failed`, or `not built`.
    pub state: String,
    /// Diagnostic codes of the failure (sorted, without repeats).
    pub codes: Vec<String>,
    /// Generated files (relative paths, sorted); empty unless the whole cell built.
    pub files: Vec<String>,
}

/// The two renderings of a dossier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dossier {
    /// Cell name.
    pub cell: String,
    /// Lowest trust level of the numbers used: `generic`, `measured` or `confirmed`.
    pub trust: String,
    /// Build state of every device, in plan order.
    pub build_status: Vec<BuildStatus>,
    /// The dossier as Markdown (inline SVG for the timing diagram).
    pub markdown: String,
    /// The dossier as one self-contained HTML file.
    pub html: String,
}

// ---------------------------------------------------------------------------
// Document model: one list of blocks rendered to Markdown and to HTML.

enum Block {
    Heading(u8, String),
    Para(String),
    Warn(String),
    List(Vec<String>),
    Table {
        head: Vec<&'static str>,
        rows: Vec<Vec<String>>,
    },
    Svg(String),
}

fn table(head: Vec<&'static str>, rows: Vec<Vec<String>>, none: &str) -> Block {
    if rows.is_empty() {
        Block::Para(none.to_string())
    } else {
        Block::Table { head, rows }
    }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn esc_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn esc_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in one_line(s).chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '|' => out.push_str("\\|"),
            '\\' => out.push_str("\\\\"),
            '`' => out.push_str("\\`"),
            '*' => out.push_str("\\*"),
            c => out.push(c),
        }
    }
    out
}

fn render_md(title: &str, blocks: &[Block]) -> String {
    let mut out = format!("# {}\n", esc_md(title));
    for b in blocks {
        out.push('\n');
        match b {
            Block::Heading(n, t) => {
                out.push_str(&"#".repeat(usize::from(*n)));
                out.push(' ');
                out.push_str(&esc_md(t));
                out.push('\n');
            }
            Block::Para(t) => {
                out.push_str(&esc_md(t));
                out.push('\n');
            }
            Block::Warn(t) => {
                out.push_str("> **Warning.** ");
                out.push_str(&esc_md(t));
                out.push('\n');
            }
            Block::List(items) => {
                for i in items {
                    out.push_str("- ");
                    out.push_str(&esc_md(i));
                    out.push('\n');
                }
            }
            Block::Table { head, rows } => {
                out.push_str(&format!("| {} |\n", head.join(" | ")));
                out.push_str(&format!("|{}\n", "---|".repeat(head.len())));
                for r in rows {
                    let cells: Vec<String> = r.iter().map(|c| esc_md(c)).collect();
                    out.push_str(&format!("| {} |\n", cells.join(" | ")));
                }
            }
            Block::Svg(svg) => {
                out.push_str(svg);
                out.push('\n');
            }
        }
    }
    out
}

const CSS: &str = "body{font-family:sans-serif;max-width:72rem;margin:2rem auto;padding:0 1rem;color:#111}\
table{border-collapse:collapse;margin:.5rem 0 1rem;font-size:.85rem}\
th,td{border:1px solid #999;padding:.2rem .5rem;text-align:left;vertical-align:top}\
th{background:#eee}\
.warn{border-left:.4rem solid #c60;background:#fff4e5;padding:.5rem 1rem}\
svg{max-width:100%;height:auto}\
@media print{body{max-width:none;margin:0}h2{page-break-after:avoid}table,svg{page-break-inside:avoid}}";

fn render_html(title: &str, blocks: &[Block]) -> String {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str(&format!("<title>{}</title>\n", esc_html(title)));
    out.push_str(&format!("<style>{CSS}</style>\n</head>\n<body>\n"));
    out.push_str(&format!("<h1>{}</h1>\n", esc_html(title)));
    for b in blocks {
        match b {
            Block::Heading(n, t) => {
                out.push_str(&format!("<h{n}>{}</h{n}>\n", esc_html(t)));
            }
            Block::Para(t) => out.push_str(&format!("<p>{}</p>\n", esc_html(t))),
            Block::Warn(t) => out.push_str(&format!(
                "<p class=\"warn\"><strong>Warning.</strong> {}</p>\n",
                esc_html(t)
            )),
            Block::List(items) => {
                out.push_str("<ul>\n");
                for i in items {
                    out.push_str(&format!("<li>{}</li>\n", esc_html(i)));
                }
                out.push_str("</ul>\n");
            }
            Block::Table { head, rows } => {
                out.push_str("<table>\n<thead><tr>");
                for h in head {
                    out.push_str(&format!("<th>{}</th>", esc_html(h)));
                }
                out.push_str("</tr></thead>\n<tbody>\n");
                for r in rows {
                    out.push_str("<tr>");
                    for c in r {
                        out.push_str(&format!("<td>{}</td>", esc_html(c)));
                    }
                    out.push_str("</tr>\n");
                }
                out.push_str("</tbody>\n</table>\n");
            }
            Block::Svg(svg) => {
                out.push_str(svg);
                out.push('\n');
            }
        }
    }
    out.push_str("</body>\n</html>\n");
    out
}

// ---------------------------------------------------------------------------
// Small text helpers.

fn trust_name(t: Trust) -> &'static str {
    match t {
        Trust::Generic => "generic",
        Trust::Measured => "measured",
        Trust::Confirmed => "confirmed",
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Plc => "plc",
        Kind::Mcu => "mcu",
        Kind::Fpga => "fpga",
        Kind::Robot => "robot",
        Kind::Host => "host",
    }
}

fn pattern_name(p: Pattern) -> &'static str {
    match p {
        Pattern::Register => "register",
        Pattern::Frame => "frame",
        Pattern::Pubsub => "pub/sub",
        Pattern::ZeroHop => "wire",
    }
}

fn opt_ns(v: Option<u64>) -> String {
    v.map_or_else(|| "-".to_string(), fmt_ns)
}

fn volts(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_string(), |v| format!("{v} V"))
}

fn diag_line(d: &CellDiag) -> String {
    let mut s = format!("{}: {}", d.code, one_line(&d.message));
    for h in &d.help {
        s.push_str(" Help: ");
        s.push_str(&one_line(h));
    }
    s
}

fn diag_list(diags: &[CellDiag]) -> Block {
    Block::List(diags.iter().map(diag_line).collect())
}

fn type_name(t: &Type) -> String {
    match t {
        Type::Bool => "bool".to_string(),
        Type::Num { kind, .. } => kind.clone(),
        Type::Bits { signed, width } => {
            format!("{}bit<{width}>", if *signed { "s" } else { "" })
        }
        _ => "other".to_string(),
    }
}

fn location_text(l: &Location) -> String {
    match l {
        Location::Register {
            owner,
            value,
            registers,
            seq,
            ack,
            busy,
            done,
        } => {
            let mut s = format!("registers on {owner}: value HR{value} (+{registers})");
            for (n, r) in [("seq", seq), ("ack", ack), ("busy", busy), ("done", done)] {
                if let Some(r) = r {
                    s.push_str(&format!(", {n} HR{r}"));
                }
            }
            s
        }
        Location::WireTag { id } => format!("Wire tag {id}"),
        Location::Topic { name } => format!("topic {name}"),
        Location::Pin { name, ack } => match ack {
            Some(a) => format!("pin {name}, ack wire {a}"),
            None => format!("pin {name}"),
        },
    }
}

fn handshake_name(h: Handshake) -> &'static str {
    match h {
        Handshake::FourPhase => "four-phase",
        Handshake::TwoPhase => "two-phase",
    }
}

fn model_name(m: LinkModel) -> &'static str {
    match m {
        LinkModel::Computed => "computed worst-case latency",
        LinkModel::Ideal => "ideal (zero latency)",
    }
}

fn plan_for<'a>(plans: &'a [LinkPlan], reports: &[LinkReport], i: usize) -> Option<&'a LinkPlan> {
    let r = reports.get(i)?;
    if plans.len() == reports.len() {
        plans.get(i)
    } else {
        plans
            .iter()
            .find(|p| p.from == r.from && p.to == r.to && p.medium == r.medium)
    }
}

// ---------------------------------------------------------------------------
// Timing diagram.

const LABEL_W: u64 = 250;
const PLOT_W: u64 = 560;
const RIGHT_W: u64 = 150;
const LANE_H: u64 = 34;
const TOP: u64 = 44;
const TERM_COLORS: [(&str, &str); 4] = [
    ("serialize", "#4e79a7"),
    ("transport", "#f28e2b"),
    ("receiver phase", "#59a14f"),
    ("polling", "#b07aa1"),
];

fn sx(ns: u64, max: u64) -> u64 {
    let v = u128::from(ns) * u128::from(PLOT_W) / u128::from(max.max(1));
    LABEL_W + u64::try_from(v).unwrap_or(PLOT_W)
}

/// The SVG timing diagram: one lane per link, the four latency terms stacked against the
/// `within` budget. Coordinates are integers computed from the link table only.
fn timing_svg(rows: &[LinkReport]) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let max = rows
        .iter()
        .map(|r| r.latency.total_ns.max(r.within_ns.unwrap_or(0)))
        .max()
        .unwrap_or(1)
        .max(1);
    let n = rows.len() as u64;
    let width = LABEL_W + PLOT_W + RIGHT_W;
    let axis_y = TOP + n * LANE_H + 6;
    let height = axis_y + 64;
    let mut s = format!(
        "<svg viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" role=\"img\" aria-label=\"Link latency against budget\">\n"
    );
    s.push_str(
        "<title>Worst-case latency of every link, stacked by term, against its budget</title>\n",
    );
    s.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\" fill=\"#ffffff\"/>\n"
    ));
    s.push_str(&format!(
        "<text x=\"{LABEL_W}\" y=\"20\" font-size=\"13\" font-family=\"sans-serif\" font-weight=\"bold\">Latency per link (scale 0 to {})</text>\n",
        esc_html(&fmt_ns(max))
    ));
    for (i, r) in rows.iter().enumerate() {
        let y0 = TOP + i as u64 * LANE_H;
        let label = format!("l{i} {} -> {}", r.from, r.to);
        s.push_str(&format!(
            "<text x=\"4\" y=\"{}\" font-size=\"12\" font-family=\"sans-serif\">{}</text>\n",
            y0 + 20,
            esc_html(&label)
        ));
        s.push_str(&format!(
            "<line x1=\"{LABEL_W}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#dddddd\"/>\n",
            y0 + 30,
            LABEL_W + PLOT_W,
            y0 + 30
        ));
        let l = &r.latency;
        let terms = [
            l.serialize_ns,
            l.transport_ns,
            l.receiver_phase_ns,
            l.polling_ns,
        ];
        let mut cum = 0u64;
        for (t, (name, color)) in terms.iter().zip(TERM_COLORS) {
            let (a, b) = (sx(cum, max), sx(cum + t, max));
            cum += t;
            if b > a {
                s.push_str(&format!(
                    "<rect x=\"{a}\" y=\"{}\" width=\"{}\" height=\"16\" fill=\"{color}\"><title>{name}: {}</title></rect>\n",
                    y0 + 8,
                    b - a,
                    esc_html(&fmt_ns(*t))
                ));
            }
        }
        if let Some(w) = r.within_ns {
            let x = sx(w, max);
            s.push_str(&format!(
                "<line x1=\"{x}\" y1=\"{}\" x2=\"{x}\" y2=\"{}\" stroke=\"#d62728\" stroke-width=\"2\"><title>within {}</title></line>\n",
                y0 + 3,
                y0 + 30,
                esc_html(&fmt_ns(w))
            ));
        }
        let verdict = match r.within_ns {
            Some(w) => format!("{} / {}", fmt_ns(l.total_ns), fmt_ns(w)),
            None => format!("{} / no budget", fmt_ns(l.total_ns)),
        };
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"12\" font-family=\"sans-serif\">{}</text>\n",
            LABEL_W + PLOT_W + 8,
            y0 + 20,
            esc_html(&verdict)
        ));
    }
    // Axis.
    s.push_str(&format!(
        "<line x1=\"{LABEL_W}\" y1=\"{axis_y}\" x2=\"{}\" y2=\"{axis_y}\" stroke=\"#000000\"/>\n",
        LABEL_W + PLOT_W
    ));
    for ns in [0, max / 2, max] {
        let x = sx(ns, max);
        s.push_str(&format!(
            "<line x1=\"{x}\" y1=\"{axis_y}\" x2=\"{x}\" y2=\"{}\" stroke=\"#000000\"/>\n<text x=\"{x}\" y=\"{}\" font-size=\"11\" font-family=\"sans-serif\" text-anchor=\"middle\">{}</text>\n",
            axis_y + 5,
            axis_y + 18,
            esc_html(&fmt_ns(ns))
        ));
    }
    // Legend.
    let ly = axis_y + 40;
    let mut lx = LABEL_W;
    for (name, color) in TERM_COLORS {
        s.push_str(&format!(
            "<rect x=\"{lx}\" y=\"{}\" width=\"12\" height=\"12\" fill=\"{color}\"/><text x=\"{}\" y=\"{ly}\" font-size=\"11\" font-family=\"sans-serif\">{name}</text>\n",
            ly - 10,
            lx + 16
        ));
        lx += 112;
    }
    s.push_str(&format!(
        "<line x1=\"{lx}\" y1=\"{}\" x2=\"{lx}\" y2=\"{}\" stroke=\"#d62728\" stroke-width=\"2\"/><text x=\"{}\" y=\"{ly}\" font-size=\"11\" font-family=\"sans-serif\">within budget</text>\n",
        ly - 12,
        ly + 2,
        lx + 8
    ));
    s.push_str("</svg>");
    Some(s)
}

// ---------------------------------------------------------------------------
// Inputs gathered once.

struct Facts<'a> {
    plan: &'a CellPlan,
    lib: &'a Library,
    nir: &'a Nir,
    sources: &'a [SourceFile],
    reports: Result<Vec<LinkReport>, Vec<CellDiag>>,
    plans: Result<Vec<LinkPlan>, Vec<CellDiag>>,
    built: Result<CellBuild, Vec<CellDiag>>,
    watchdogs: Vec<crate::DeviceWatchdog>,
    zones: Vec<ZonePlan>,
    zone_diags: Vec<CellDiag>,
    electrical: Option<Vec<CellDiag>>,
    tags: Vec<CellDiag>,
    verdict: CellVerdict,
    run: Option<Result<SimRun, SimError>>,
    status: Vec<BuildStatus>,
}

fn build_status(plan: &CellPlan, built: &Result<CellBuild, Vec<CellDiag>>) -> Vec<BuildStatus> {
    let prefix = |inst: &str| format!("device `{inst}`:");
    plan.devices
        .iter()
        .map(|dev| {
            let mut st = BuildStatus {
                device: dev.instance.clone(),
                manifest: dev.manifest.clone(),
                backend: dev.backend,
                state: "built".into(),
                codes: Vec::new(),
                files: Vec::new(),
            };
            match built {
                Ok(b) => {
                    if let Some(d) = b.device(&dev.instance) {
                        st.files = d.files.keys().cloned().collect();
                    }
                }
                Err(diags) => {
                    let mine: Vec<&CellDiag> = diags
                        .iter()
                        .filter(|d| d.message.starts_with(&prefix(&dev.instance)))
                        .collect();
                    let any_device = diags.iter().any(|d| d.message.starts_with("device `"));
                    if !mine.is_empty() {
                        st.state = "failed".into();
                        st.codes = mine.iter().map(|d| d.code.to_string()).collect();
                        st.codes.sort();
                        st.codes.dedup();
                    } else if !any_device {
                        st.state = "not built".into();
                        st.codes = diags.iter().map(|d| d.code.to_string()).collect();
                        st.codes.sort();
                        st.codes.dedup();
                    }
                }
            }
            st
        })
        .collect()
}

fn gather<'a>(
    plan: &'a CellPlan,
    nir: &'a Nir,
    sources: &'a [SourceFile],
    lib: &'a Library,
    opts: &DossierOptions,
) -> Facts<'a> {
    let reports = link_table(nir, plan, lib);
    let plans = link_plans(nir, plan, lib);
    let built = build(plan, nir, sources, lib);
    let (watchdogs, zones, zone_diags) = match &built {
        Ok(b) => (b.watchdogs.clone(), b.zones.clone(), Vec::new()),
        Err(_) => {
            let w = plans
                .as_ref()
                .map(|p| watchdog_plans(nir, plan, p))
                .unwrap_or_default();
            let (z, zd) = zone_plans(nir, plan, lib);
            (w, z, zd)
        }
    };
    let plans = match (&built, plans) {
        (Ok(b), _) => Ok(b.links.clone()),
        (_, p) => p,
    };
    let manifest_of = |inst: &str| plan.device(inst).map(|d| d.manifest.clone());
    let electrical = reports
        .as_ref()
        .ok()
        .map(|r| check_electrical(r, &plan.file, &manifest_of, lib));
    let tags = check_tags(nir, plan, lib);
    let verdict = verify_cell(sources, nir, plan, lib, &zones);
    let run = opts
        .sim
        .as_ref()
        .map(|o| cosimulate(plan, nir, sources, lib, o));
    let status = build_status(plan, &built);
    Facts {
        plan,
        lib,
        nir,
        sources,
        reports,
        plans,
        built,
        watchdogs,
        zones,
        zone_diags,
        electrical,
        tags,
        verdict,
        run,
        status,
    }
}

fn lowest_trust(f: &Facts<'_>) -> Trust {
    let mut t = Trust::Confirmed;
    for d in &f.plan.devices {
        if let Some(m) = f.lib.device(&d.manifest) {
            t = t.min(m.trust);
        }
    }
    if let Ok(rows) = &f.reports {
        for r in rows {
            t = t.min(r.trust);
        }
    }
    t
}

// ---------------------------------------------------------------------------
// Sections.

fn summary(f: &Facts<'_>, trust: Trust, out: &mut Vec<Block>) {
    out.push(Block::Heading(2, "1. Summary".into()));
    out.push(Block::Warn(SAFETY_WORDING.into()));
    let links = f.reports.as_ref().map_or(0, Vec::len);
    out.push(Block::List(vec![
        format!("Cell: {} (file {})", f.plan.cell, f.plan.file),
        format!("Devices: {}", f.plan.devices.len()),
        format!("Links: {links}"),
        format!("Trust level of the numbers: {}", trust_name(trust)),
    ]));
    match trust {
        Trust::Generic => out.push(Block::Warn(
            "The timing, scan and electrical numbers in this dossier are generic placeholders \
(trust level generic). They are not measured. They are to be measured in lab session I1 \
and the dossier regenerated before any figure is relied on."
                .into(),
        )),
        Trust::Measured => out.push(Block::Para(
            "The numbers are measured on the lab setup (trust level measured) but not yet \
confirmed on the final hardware."
                .into(),
        )),
        Trust::Confirmed => out.push(Block::Para(
            "The numbers are confirmed on the final hardware (trust level confirmed).".into(),
        )),
    }
    let rows = f
        .plan
        .devices
        .iter()
        .zip(&f.status)
        .map(|(d, st)| {
            let m = f.lib.device(&d.manifest);
            vec![
                d.instance.clone(),
                d.manifest.clone(),
                kind_name(d.kind).to_string(),
                m.map_or("-", |m| trust_name(m.trust)).to_string(),
                d.modules
                    .iter()
                    .map(|p| p.module.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
                st.state.clone(),
            ]
        })
        .collect();
    out.push(table(
        vec![
            "device",
            "manifest",
            "class",
            "trust",
            "placed modules",
            "build",
        ],
        rows,
        "The cell declares no device.",
    ));
}

fn build_section(f: &Facts<'_>, out: &mut Vec<Block>) {
    out.push(Block::Heading(2, "2. Build status".into()));
    let rows = f
        .status
        .iter()
        .map(|s| {
            vec![
                s.device.clone(),
                s.backend.name().to_string(),
                s.state.clone(),
                if s.codes.is_empty() {
                    "-".into()
                } else {
                    s.codes.join(", ")
                },
                s.files.len().to_string(),
                if s.files.is_empty() {
                    "-".into()
                } else {
                    s.files.join(", ")
                },
            ]
        })
        .collect();
    out.push(table(
        vec!["device", "backend", "build", "codes", "files", "file names"],
        rows,
        "No device.",
    ));
    if let Err(diags) = &f.built {
        out.push(Block::Para(
            "The cell does not build completely today. Each failed device is listed with the \
diagnostic of its backend; the other sections use the data that can be computed without the \
failed build."
                .into(),
        ));
        out.push(Block::Para(
            "Generated files are kept and listed only when the whole cell builds.".into(),
        ));
        out.push(diag_list(diags));
    }
}

fn wiring(f: &Facts<'_>, out: &mut Vec<Block>) {
    out.push(Block::Heading(2, "3. Wiring list".into()));
    let Ok(rows) = &f.reports else {
        out.push(Block::Para(
            "The link table could not be computed; see the link table section.".into(),
        ));
        return;
    };
    let mut t = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let Some(m) = f.lib.medium(&r.medium) else {
            continue;
        };
        if m.family != Family::ZeroHop {
            continue;
        }
        let out_v = f
            .plan
            .device(&r.sender)
            .and_then(|d| f.lib.device(&d.manifest))
            .and_then(output_volts);
        let in_v = f
            .plan
            .device(&r.receiver)
            .and_then(|d| f.lib.device(&d.manifest))
            .and_then(input_volts);
        let isolation = if m.isolated {
            "isolated medium".to_string()
        } else if let Some(o) = r.options.iter().find(|o| m.isolating.contains(o)) {
            format!("option {o}")
        } else {
            "none".to_string()
        };
        let pin = f
            .plans
            .as_ref()
            .ok()
            .and_then(|p| plan_for(p, rows, i))
            .map_or_else(
                || "-".to_string(),
                |p| {
                    format!(
                        "{} / {}",
                        location_text(&p.tx.location),
                        location_text(&p.rx.location)
                    )
                },
            );
        t.push(vec![
            format!("l{i}"),
            r.from.clone(),
            r.to.clone(),
            r.medium.clone(),
            if r.options.is_empty() {
                "-".into()
            } else {
                r.options.join(", ")
            },
            volts(out_v),
            volts(in_v),
            isolation,
            pin,
        ]);
    }
    out.push(table(
        vec![
            "link",
            "from",
            "to",
            "medium",
            "options",
            "sender output",
            "receiver input",
            "isolation",
            "logical pins (sender / receiver)",
        ],
        t,
        "No wired link: every link of this cell crosses a network or a bus.",
    ));
}

fn source_text(sources: &[SourceFile], file: &str, span: [usize; 2]) -> String {
    sources
        .iter()
        .find(|s| s.path == file)
        .and_then(|s| s.text.get(span[0]..span[1]))
        .map(one_line)
        .unwrap_or_default()
}

fn io_list(f: &Facts<'_>, out: &mut Vec<Block>) {
    out.push(Block::Heading(2, "4. I/O list".into()));
    out.push(Block::Para(
        "Peripheral families of each device manifest, then the I/O the placed programs declare."
            .into(),
    ));
    let mut fam = Vec::new();
    let mut decl = Vec::new();
    for d in &f.plan.devices {
        if let Some(m) = f.lib.device(&d.manifest) {
            for (name, fa) in &m.io {
                fam.push(vec![
                    d.instance.clone(),
                    name.clone(),
                    fa.prefix.clone(),
                    volts(fa.volts),
                    fa.kind.clone().unwrap_or_else(|| "-".into()),
                    fa.max_ma.map_or("-".into(), |m| format!("{m} mA")),
                ]);
            }
        }
        for p in &d.modules {
            let Some(m) = f.nir.modules.iter().find(|m| m.name == p.module) else {
                continue;
            };
            for item in &m.items {
                match &item.def {
                    ItemDef::PlcSection { section, vars } => {
                        for v in vars {
                            decl.push(vec![
                                d.instance.clone(),
                                p.module.clone(),
                                section.clone(),
                                v.name.clone(),
                                type_name(&v.vty),
                            ]);
                        }
                    }
                    ItemDef::Io { name, .. } => {
                        let t = source_text(f.sources, &m.file, item.span);
                        decl.push(vec![
                            d.instance.clone(),
                            p.module.clone(),
                            "io".into(),
                            name.clone(),
                            if t.is_empty() {
                                "(source text not available)".into()
                            } else {
                                t
                            },
                        ]);
                    }
                    _ => {}
                }
            }
        }
    }
    out.push(Block::Heading(3, "Device I/O families".into()));
    out.push(table(
        vec!["device", "family", "prefix", "voltage", "kind", "limit"],
        fam,
        "No device declares I/O families.",
    ));
    out.push(Block::Heading(3, "Declared by the programs".into()));
    out.push(table(
        vec!["device", "module", "section", "name", "type or declaration"],
        decl,
        "The placed programs declare no `io` and no PLC variable sections.",
    ));
}

fn addresses(f: &Facts<'_>, out: &mut Vec<Block>) {
    out.push(Block::Heading(2, "5. Pin and address tables".into()));
    let Ok(plans) = &f.plans else {
        out.push(Block::Para(
            "The link plans could not be computed; no addresses are assigned.".into(),
        ));
        return;
    };
    out.push(Block::Heading(3, "Register map".into()));
    let regs = register_map(plans)
        .into_iter()
        .map(|r| {
            vec![
                r.owner,
                format!("l{}", r.link),
                r.field,
                format!("HR{}", r.address),
                r.registers.to_string(),
                r.ty,
                r.writer,
            ]
        })
        .collect();
    out.push(table(
        vec![
            "owner",
            "link",
            "field",
            "address",
            "registers",
            "type",
            "writer",
        ],
        regs,
        "No register medium is used.",
    ));
    out.push(Block::Heading(
        3,
        "Endpoints, Wire tags, topics and pins".into(),
    ));
    let eps = plans
        .iter()
        .map(|p| {
            vec![
                format!("l{}", p.index),
                format!("{} -> {}", p.from, p.to),
                p.medium.clone(),
                pattern_name(p.pattern).to_string(),
                format!("{} ({} bytes)", p.value_type, p.value_bytes),
                format!("{:?}", p.crossing),
                format!("{}: {}", p.tx.device, location_text(&p.tx.location)),
                format!("{}: {}", p.rx.device, location_text(&p.rx.location)),
            ]
        })
        .collect();
    out.push(table(
        vec![
            "link", "ends", "medium", "pattern", "value", "crossing", "sender", "receiver",
        ],
        eps,
        "The cell has no link.",
    ));
    let notes: Vec<String> = plans
        .iter()
        .flat_map(|p| p.notes.iter().map(move |n| format!("l{}: {}", p.index, n)))
        .collect();
    if !notes.is_empty() {
        out.push(Block::Heading(3, "Notes on the endpoints".into()));
        out.push(Block::List(notes));
    }
}

fn link_section(f: &Facts<'_>, out: &mut Vec<Block>) {
    out.push(Block::Heading(2, "6. Link table and budgets".into()));
    let rows = match &f.reports {
        Ok(r) => r,
        Err(diags) => {
            out.push(Block::Para(
                "The link table could not be computed. Diagnostics:".into(),
            ));
            out.push(diag_list(diags));
            return;
        }
    };
    out.push(Block::Para(
        "Worst-case latency of each link in the four terms of the cell specification. \
Numbers use the trust level in the last column."
            .into(),
    ));
    let t = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            vec![
                format!("l{i}"),
                r.from.clone(),
                r.to.clone(),
                r.medium.clone(),
                if r.options.is_empty() {
                    "-".into()
                } else {
                    r.options.join(", ")
                },
                opt_ns(r.every_ns),
                fmt_ns(r.latency.serialize_ns),
                fmt_ns(r.latency.transport_ns),
                fmt_ns(r.latency.receiver_phase_ns),
                fmt_ns(r.latency.polling_ns),
                fmt_ns(r.latency.total_ns),
                r.latency.total_ns.to_string(),
                opt_ns(r.within_ns),
                if r.meets_budget() { "yes" } else { "NO" }.to_string(),
                trust_name(r.trust).to_string(),
            ]
        })
        .collect();
    out.push(table(
        vec![
            "link",
            "from",
            "to",
            "medium",
            "options",
            "every",
            "serialize",
            "transport",
            "receiver phase",
            "polling",
            "total",
            "total (ns)",
            "within",
            "meets budget",
            "trust",
        ],
        t,
        "The cell has no link.",
    ));
    let alts: Vec<String> = rows
        .iter()
        .enumerate()
        .flat_map(|(i, r)| {
            r.alternatives
                .iter()
                .map(move |a| format!("l{i}: {} gives {}", a.change, fmt_ns(a.latency_ns)))
        })
        .collect();
    if !alts.is_empty() {
        out.push(Block::Heading(3, "Alternatives".into()));
        out.push(Block::List(alts));
    }
    out.push(Block::Heading(3, "Timing diagram".into()));
    match timing_svg(rows) {
        Some(svg) => {
            out.push(Block::Para(
                "One lane per link. The stacked bars are serialize, transport, receiver phase \
and polling; the red line is the `within` budget."
                    .into(),
            ));
            out.push(Block::Svg(svg));
        }
        None => out.push(Block::Para("No link, no diagram.".into())),
    }
}

fn bom(f: &Facts<'_>, out: &mut Vec<Block>) {
    out.push(Block::Heading(2, "7. Bill of materials".into()));
    out.push(Block::Para(
        "Generic device classes from the manifests. Vendor names are not used; the vendor \
class says which family of vendor tooling the device speaks."
            .into(),
    ));
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for d in &f.plan.devices {
        groups
            .entry(d.manifest.as_str())
            .or_default()
            .push(d.instance.as_str());
    }
    let rows = groups
        .iter()
        .map(|(name, inst)| {
            let m = f.lib.device(name);
            vec![
                (*name).to_string(),
                m.map_or("-", |m| kind_name(m.kind)).to_string(),
                m.map_or("-", |m| m.vendor_class.as_str()).to_string(),
                inst.len().to_string(),
                inst.join(", "),
                m.map_or("-", |m| trust_name(m.trust)).to_string(),
            ]
        })
        .collect();
    out.push(table(
        vec![
            "device manifest",
            "class",
            "vendor class",
            "count",
            "instances",
            "trust",
        ],
        rows,
        "No device.",
    ));
    if let Ok(reports) = &f.reports {
        let mut media: BTreeMap<&str, usize> = BTreeMap::new();
        for r in reports {
            *media.entry(r.medium.as_str()).or_default() += 1;
        }
        let rows = media
            .iter()
            .map(|(m, n)| {
                vec![
                    (*m).to_string(),
                    f.lib
                        .medium(m)
                        .map_or("-", |m| trust_name(m.trust))
                        .to_string(),
                    n.to_string(),
                ]
            })
            .collect();
        out.push(Block::Heading(3, "Media".into()));
        out.push(table(vec!["medium", "trust", "links"], rows, "No link."));
    }
}

fn safety(f: &Facts<'_>, out: &mut Vec<Block>) {
    out.push(Block::Heading(2, "8. Safety report".into()));
    out.push(Block::Warn(SAFETY_WORDING.into()));

    out.push(Block::Heading(3, "Electrical compatibility".into()));
    match &f.electrical {
        None => out.push(Block::Para(
            "Not checked: the link table could not be computed.".into(),
        )),
        Some(d) if d.is_empty() => out.push(Block::Para(
            "No finding: every wired link joins matching voltage domains or uses isolation or level shifting.".into(),
        )),
        Some(d) => out.push(diag_list(d)),
    }

    out.push(Block::Heading(3, "Tags".into()));
    if f.tags.is_empty() {
        out.push(Block::Para("No finding in the tag checks.".into()));
    } else {
        out.push(diag_list(&f.tags));
    }

    out.push(Block::Heading(3, "Zones and mutexes".into()));
    if !f.zone_diags.is_empty() {
        out.push(diag_list(&f.zone_diags));
    }
    let zrows = f
        .zones
        .iter()
        .map(|z| {
            vec![
                z.zone.clone(),
                format!(
                    "x {}..{}, y {}..{}, z {}..{} mm",
                    z.shape.x[0],
                    z.shape.x[1],
                    z.shape.y[0],
                    z.shape.y[1],
                    z.shape.z[0],
                    z.shape.z[1]
                ),
                z.owner.clone(),
                z.parties
                    .iter()
                    .map(|p| match &p.output {
                        Some(o) => format!("{}.{o}", p.device),
                        None => p.device.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                z.locks
                    .iter()
                    .map(|l| match &l.medium {
                        Some(m) => {
                            format!("{} via {m} ({})", l.party, fmt_ns(l.one_way_ns))
                        }
                        None => format!("{} (local to the owner)", l.party),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                z.entries.len().to_string(),
                z.entries.iter().filter(|e| e.wrapped).count().to_string(),
                z.unchecked_targets.to_string(),
            ]
        })
        .collect();
    out.push(table(
        vec![
            "zone",
            "box",
            "lock owner",
            "parties",
            "lock transport (one way)",
            "entries",
            "covered by the lock contract",
            "unchecked targets",
        ],
        zrows,
        "The cell declares no mutex.",
    ));
    let zn: Vec<String> = f
        .zones
        .iter()
        .flat_map(|z| z.notes.iter().map(move |n| format!("{}: {n}", z.zone)))
        .collect();
    if !zn.is_empty() {
        out.push(Block::List(zn));
    }
    let proofs = f
        .verdict
        .zones
        .iter()
        .map(|p| {
            vec![
                p.zone.clone(),
                handshake_name(p.handshake).to_string(),
                fmt_ns(p.tick_ns),
                p.proved.len().to_string(),
                p.violations.len().to_string(),
                p.undecided.len().to_string(),
                if p.holds() { "proved" } else { "NOT proved" }.to_string(),
            ]
        })
        .collect();
    out.push(table(
        vec![
            "zone",
            "handshake",
            "tick",
            "properties proved",
            "violations",
            "undecided",
            "mutual exclusion",
        ],
        proofs,
        "No mutex to prove.",
    ));

    out.push(Block::Heading(3, "Watchdog bounds".into()));
    out.push(Block::Para(
        "Static bound on the time from the loss of a link to the safe state of the receiving \
device: watchdog period plus the safe-state time of its manifest. Heartbeats are 10 ms and five \
may be missed."
            .into(),
    ));
    let wrows = f
        .watchdogs
        .iter()
        .map(|w| {
            vec![
                w.device.clone(),
                w.links
                    .iter()
                    .map(|l| format!("l{l}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                fmt_ns(w.watchdog_ns),
                fmt_ns(w.safe_state_ns),
                fmt_ns(w.bound_ns),
                if w.actuators.is_empty() {
                    "-".into()
                } else {
                    w.actuators.join(", ")
                },
            ]
        })
        .collect();
    out.push(table(
        vec![
            "device",
            "links",
            "watchdog",
            "safe-state time",
            "bound",
            "actuators driven to safe state",
        ],
        wrows,
        "No device consumes a link.",
    ));

    out.push(Block::Heading(3, "Cell verify".into()));
    let v = &f.verdict;
    let crow = v
        .channels
        .iter()
        .map(|c| vec![c.from.clone(), c.to.clone(), fmt_ns(c.delay_ns)])
        .collect();
    out.push(table(
        vec!["channel from", "channel to", "modeled delay bound"],
        crow,
        "No channel was modeled.",
    ));
    out.push(Block::List(vec![
        format!("Properties proved: {}", v.proved.len()),
        format!("Counterexamples: {}", v.violations.len()),
    ]));
    for p in &v.proved {
        out.push(Block::Para(format!(
            "Proved: {} ({} states)",
            p.property, p.states
        )));
    }
    for c in &v.violations {
        out.push(Block::Para(format!(
            "Violated: {} (counterexample of {} steps)",
            c.property,
            c.trace.len()
        )));
    }
    if !v.diags.is_empty() {
        out.push(diag_list(&v.diags));
    }
    if !v.notes.is_empty() {
        out.push(Block::List(v.notes.clone()));
    }
}

fn test_report(f: &Facts<'_>, out: &mut Vec<Block>) {
    out.push(Block::Heading(2, "9. Test report (co-simulation)".into()));
    let run = match &f.run {
        None => {
            out.push(Block::Para("The co-simulation was not run.".into()));
            return;
        }
        Some(Err(e)) => {
            out.push(Block::Warn(format!("The co-simulation could not run: {e}")));
            return;
        }
        Some(Ok(r)) => r,
    };
    out.push(Block::List(vec![
        format!("Simulated time: {}", fmt_ns(run.end_ns)),
        format!("Link model: {}", model_name(run.link_model)),
        format!("Events in the trace: {}", run.events.len()),
        format!("Cuts applied: {}", run.cuts.len()),
    ]));
    let drows = run
        .devices
        .iter()
        .map(|d| {
            vec![
                d.instance.clone(),
                d.manifest.clone(),
                d.modules.join(", "),
                if d.runs { "yes" } else { "no (nothing to run)" }.to_string(),
            ]
        })
        .collect();
    out.push(table(
        vec!["device", "manifest", "modules", "ran"],
        drows,
        "No device.",
    ));
    let lrows = run
        .links
        .iter()
        .map(|l| {
            let d: Vec<_> = run
                .deliveries
                .iter()
                .filter(|d| d.link == l.index)
                .collect();
            let lat = |d: &&crate::Delivery| d.delivered_ns - d.sent_ns;
            let min = d.iter().map(lat).min();
            let max = d.iter().map(lat).max();
            vec![
                format!("l{}", l.index),
                format!("{} -> {}", l.from, l.to),
                l.medium.clone(),
                d.len().to_string(),
                fmt_ns(l.latency_ns),
                opt_ns(l.within_ns),
                opt_ns(min),
                opt_ns(max),
                match (max, l.within_ns) {
                    (Some(m), Some(w)) => if m <= w { "yes" } else { "NO" }.to_string(),
                    _ => "-".into(),
                },
            ]
        })
        .collect();
    out.push(table(
        vec![
            "link",
            "ends",
            "medium",
            "deliveries",
            "modeled latency",
            "within",
            "observed min",
            "observed max",
            "inside budget",
        ],
        lrows,
        "No link.",
    ));
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &run.events {
        *kinds.entry(e.kind.tag()).or_default() += 1;
    }
    let krows = kinds
        .iter()
        .map(|(k, n)| vec![(*k).to_string(), n.to_string()])
        .collect();
    out.push(table(
        vec!["event kind", "count"],
        krows,
        "The trace is empty.",
    ));
    let faults: Vec<Vec<String>> = run
        .events
        .iter()
        .filter(|e| e.kind == CellEventKind::Fault)
        .map(|e| {
            vec![
                fmt_ns(e.t_ns),
                e.device.clone(),
                e.name.clone(),
                e.value.clone(),
            ]
        })
        .collect();
    out.push(Block::Heading(3, "Faults".into()));
    out.push(table(
        vec!["time", "device", "fault", "value"],
        faults,
        "No fault was raised.",
    ));
    if !run.losses.is_empty() {
        out.push(Block::Heading(3, "Link loss".into()));
        let rows = run
            .losses
            .iter()
            .map(|l| {
                vec![
                    format!("l{}", l.link),
                    l.device.clone(),
                    fmt_ns(l.cut_ns),
                    opt_ns(l.fault_ns),
                    fmt_ns(l.bound_ns),
                    if l.within_bound { "yes" } else { "NO" }.to_string(),
                ]
            })
            .collect();
        out.push(table(
            vec![
                "link",
                "device",
                "cut at",
                "fault at",
                "bound",
                "within bound",
            ],
            rows,
            "",
        ));
    }
    if !run.notes.is_empty() {
        out.push(Block::List(run.notes.clone()));
    }
}

/// Generates the dossier of `plan` from the data the cell compiler computes: partition,
/// per-device build, link table and plans, watchdogs, zones, tags, electrical checks,
/// cell verify and, unless `opts.sim` is `None`, a co-simulation as the test report.
///
/// Nothing is typed by hand and nothing depends on the clock or on hash order: the same
/// inputs give byte-identical Markdown and HTML.
///
/// ```no_run
/// # use neuron_cell::{dossier, partition, DossierOptions, Library};
/// # fn demo(nir: &neuron_nir::Nir, sources: &[neuron_sema::SourceFile]) {
/// let lib = Library::builtin().unwrap();
/// let plan = partition(nir, &lib).unwrap().remove(0);
/// let d = dossier(&plan, nir, sources, &lib, &DossierOptions::default());
/// assert!(d.html.starts_with("<!DOCTYPE html>"));
/// # }
/// ```
pub fn dossier(
    plan: &CellPlan,
    nir: &Nir,
    sources: &[SourceFile],
    lib: &Library,
    opts: &DossierOptions,
) -> Dossier {
    let f = gather(plan, nir, sources, lib, opts);
    let trust = lowest_trust(&f);
    let mut blocks = Vec::new();
    summary(&f, trust, &mut blocks);
    build_section(&f, &mut blocks);
    wiring(&f, &mut blocks);
    io_list(&f, &mut blocks);
    addresses(&f, &mut blocks);
    link_section(&f, &mut blocks);
    bom(&f, &mut blocks);
    safety(&f, &mut blocks);
    test_report(&f, &mut blocks);
    let title = format!("Technical dossier: cell {}", plan.cell);
    Dossier {
        cell: plan.cell.clone(),
        trust: trust_name(trust).to_string(),
        build_status: f.status.clone(),
        markdown: render_md(&title, &blocks),
        html: render_html(&title, &blocks),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::Latency;
    use proptest::prelude::*;

    fn report(terms: [u64; 4], within: Option<u64>) -> LinkReport {
        LinkReport {
            from: "a.x<b>".into(),
            to: "b.y".into(),
            sender: "a".into(),
            receiver: "b".into(),
            medium: "modbus_tcp".into(),
            options: Vec::new(),
            every_ns: None,
            within_ns: within,
            latency: Latency {
                serialize_ns: terms[0],
                transport_ns: terms[1],
                receiver_phase_ns: terms[2],
                polling_ns: terms[3],
                total_ns: terms.iter().sum(),
            },
            trust: Trust::Generic,
            alternatives: Vec::new(),
            span: [0, 0],
        }
    }

    #[test]
    fn text_is_escaped_in_both_formats() {
        assert_eq!(
            esc_html("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        assert_eq!(esc_md("a|b\n<c>"), "a\\|b &lt;c&gt;");
        let blocks = [Block::Para("<script>alert(1)</script>".into())];
        let html = render_html("t", &blocks);
        assert!(!html.contains("<script"));
        assert!(!render_md("t", &blocks).contains("<script"));
    }

    #[test]
    fn a_link_without_a_budget_has_no_marker() {
        let svg = timing_svg(&[report([1_000, 2_000, 3_000, 0], None)]).unwrap();
        assert!(svg.contains("no budget"));
        assert!(!svg.contains("#d62728\" stroke-width=\"2\"><title>"));
        assert!(timing_svg(&[]).is_none());
    }

    proptest! {
        #[test]
        fn escaped_html_has_no_markup(s in ".*") {
            let e = esc_html(&s);
            prop_assert!(!e.contains(['<', '>', '"', '\'']));
        }

        #[test]
        fn escaped_markdown_cells_keep_the_table(s in ".*") {
            let e = esc_md(&s);
            prop_assert!(!e.contains(['<', '>', '\n', '\r']));
            let b: Vec<char> = e.chars().collect();
            for (i, c) in b.iter().enumerate() {
                if *c == '|' {
                    prop_assert!(i > 0 && b[i - 1] == '\\');
                }
            }
        }

        /// The stacked bars of a lane cover exactly the scaled total, in any scale.
        #[test]
        fn the_bars_of_a_lane_sum_to_the_scaled_total(
            a in 0u64..5_000_000, b in 0u64..5_000_000, c in 0u64..5_000_000, d in 0u64..5_000_000,
            w in proptest::option::of(1u64..20_000_000),
        ) {
            let r = report([a, b, c, d], w);
            let svg = timing_svg(std::slice::from_ref(&r)).unwrap();
            let max = r.latency.total_ns.max(w.unwrap_or(0)).max(1);
            let widths: u64 = svg
                .lines()
                .filter(|l| l.starts_with("<rect") && l.contains("height=\"16\""))
                .map(|l| {
                    let rest = l.split("width=\"").nth(1).unwrap();
                    rest.split('"').next().unwrap().parse::<u64>().unwrap()
                })
                .sum();
            prop_assert_eq!(widths, sx(r.latency.total_ns, max) - LABEL_W);
            prop_assert_eq!(svg.clone(), timing_svg(std::slice::from_ref(&r)).unwrap());
        }
    }
}
