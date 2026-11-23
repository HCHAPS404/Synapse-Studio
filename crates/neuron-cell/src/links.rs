//! Link budgets (task D3.2): the worst-case latency of every `link` and `NE0701`.
//!
//! Spec 14.8: `latency = serialize + transport + receiver phase + polling`, where
//! - `serialize` is `bytes · 8 / bitrate` for media with a bit rate (the frame overhead
//!   of the medium plus the value; 4 bytes when the type is not known);
//! - `transport` adds the sender's output time, the medium's worst transport and the
//!   receiver's input time, each with its jitter (`[timing]` of both manifests and
//!   `transport_worst` of the medium): a deliberate worst case;
//! - the receiver phase is the receiver's shortest scan or task period (`scan_min`): a
//!   value waits at most one release;
//! - polling is the period after `every`, or the sender's `scan_min` when the link has no
//!   `every`; it is zero for an event-driven medium on a link without `every`.
//!
//! Every number carries the lowest trust level of the files it comes from, so a budget
//! made of `generic` numbers says so.
use crate::{
    duration_ns, CellDiag, CellPlan, Library, Manifest, Medium, MediumTiming, TimingOverlay, Trust,
};
use neuron_nir::{Expr, ExprKind, ItemDef, Lit, ModuleKind, Nir};
use serde::Serialize;

/// Bytes of a value on the wire when the caller does not know the link's type
/// (`latency`); `link_table` uses the real size of each link (task D3.1).
const DEFAULT_VALUE_BYTES: u64 = 4;

/// What the planner knows about the value of a link and its receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    /// Bytes of the value on the wire.
    pub value_bytes: u64,
    /// Period of the receiver's own program (scan or task), when it is known; the
    /// receiver phase is the longer of this and the manifest's `scan_min`.
    pub receiver_period_ns: Option<u64>,
}

impl Default for Shape {
    fn default() -> Shape {
        Shape {
            value_bytes: DEFAULT_VALUE_BYTES,
            receiver_period_ns: None,
        }
    }
}

/// The terms of one latency, in nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Latency {
    /// Bytes on the wire divided by the bit rate.
    pub serialize_ns: u64,
    /// Sender output, medium and receiver input, with jitter.
    pub transport_ns: u64,
    /// Wait for the receiver's next release.
    pub receiver_phase_ns: u64,
    /// Sender's sampling period (zero when event-driven).
    pub polling_ns: u64,
    /// The sum.
    pub total_ns: u64,
}

/// A medium or a period that would meet the budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Alternative {
    /// What to write, such as `via zero_hop` or `every 5 ms`.
    pub change: String,
    /// Latency with that change.
    pub latency_ns: u64,
}

/// One row of the link table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkReport {
    /// Source as written (`plc.cap_done`).
    pub from: String,
    /// Destination as written (`arm.pick`).
    pub to: String,
    /// Sender device instance.
    pub sender: String,
    /// Receiver device instance.
    pub receiver: String,
    /// Medium name.
    pub medium: String,
    /// Medium options (`gpio`, `opto_24v`).
    pub options: Vec<String>,
    /// Period after `every`.
    pub every_ns: Option<u64>,
    /// Budget after `within`.
    pub within_ns: Option<u64>,
    /// The computed worst case.
    pub latency: Latency,
    /// Lowest trust level of the numbers used.
    pub trust: Trust,
    /// Changes that meet the budget, fastest first (only when the budget is not met).
    pub alternatives: Vec<Alternative>,
    /// Byte range of the `link` item in the cell file.
    #[serde(skip)]
    pub span: [usize; 2],
}

impl LinkReport {
    /// The budget holds, or there is none.
    pub fn meets_budget(&self) -> bool {
        self.within_ns.is_none_or(|w| self.latency.total_ns <= w)
    }
}

/// Formats nanoseconds with the largest unit that keeps three significant digits.
pub fn fmt_ns(ns: u64) -> String {
    let (div, unit) = if ns >= 1_000_000_000 {
        (1e9, "s")
    } else if ns >= 1_000_000 {
        (1e6, "ms")
    } else if ns >= 1_000 {
        (1e3, "us")
    } else {
        return format!("{ns} ns");
    };
    let v = ns as f64 / div;
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{s} {unit}")
}

/// `a.b.c` of a field chain over a name.
pub(crate) fn dotted(e: &Expr) -> Option<Vec<String>> {
    match &e.e {
        ExprKind::Global { name } | ExprKind::Ref { name, .. } | ExprKind::Local { name, .. } => {
            Some(vec![name.clone()])
        }
        ExprKind::Field { base, name, .. } => {
            let mut v = dotted(base)?;
            v.push(name.clone());
            Some(v)
        }
        _ => None,
    }
}

/// A duration literal (`10 ms`) in nanoseconds.
pub(crate) fn duration_of(e: &Expr) -> Option<u64> {
    let ExprKind::Quantity { value, unit, .. } = &e.e else {
        return None;
    };
    let v = match &value.e {
        ExprKind::Lit {
            lit: Lit::Int { v, .. } | Lit::Real { v, .. },
        } => v.clone(),
        _ => return None,
    };
    duration_ns(&format!("{v} {}", unit.text)).ok()
}

fn medium_parts(via: &Expr) -> Option<(String, Vec<String>)> {
    match &via.e {
        ExprKind::Call { callee, args, .. } => {
            let name = dotted(callee)?.join(".");
            let opts = args
                .iter()
                .filter_map(|a| dotted(&a.value).map(|d| d.join(".")))
                .collect();
            Some((name, opts))
        }
        _ => Some((dotted(via)?.join("."), Vec::new())),
    }
}

/// The latency of a link from `s` to `r` over `m`, for a 4-byte value and a receiver
/// that runs at its shortest period. See [`latency_with`].
pub fn latency(s: &Manifest, r: &Manifest, m: &Medium, every_ns: Option<u64>) -> Option<Latency> {
    latency_with(s, r, m, every_ns, Shape::default())
}

/// The latency of a link with the real value size and receiver period of `shape`.
pub fn latency_with(
    s: &Manifest,
    r: &Manifest,
    m: &Medium,
    every_ns: Option<u64>,
    shape: Shape,
) -> Option<Latency> {
    latency_with_overlay(s, r, m, every_ns, shape, &TimingOverlay::new())
}

/// The effective timing of a device on a medium: a calibration profile from `overlay` when it
/// has one for this device manifest and medium, otherwise the manifest's own `[timing]`.
fn effective_timing(man: &Manifest, m: &Medium, overlay: &TimingOverlay) -> Option<MediumTiming> {
    if let Some(p) = overlay.get(&man.name, &m.name) {
        return Some(p.timing());
    }
    man.timing.get(&m.name).copied()
}

/// [`latency_with`], but a calibration profile in `overlay` overrides the manifest timing of a
/// device on this medium (task E6.3). An empty overlay reproduces [`latency_with`] exactly.
pub fn latency_with_overlay(
    s: &Manifest,
    r: &Manifest,
    m: &Medium,
    every_ns: Option<u64>,
    shape: Shape,
    overlay: &TimingOverlay,
) -> Option<Latency> {
    let st = effective_timing(s, m, overlay)?;
    let rt = effective_timing(r, m, overlay)?;
    let bytes = u64::from(m.frame_overhead_bytes) + shape.value_bytes;
    let serialize_ns = m
        .bitrate_bd
        .filter(|b| *b > 0)
        .map_or(0, |b| (bytes * 8 * 1_000_000_000).div_ceil(b));
    let transport_ns =
        st.transport_ns + st.jitter_ns + m.transport_worst_ns + rt.transport_ns + rt.jitter_ns;
    let receiver_phase_ns = r
        .scan_min_ns
        .unwrap_or(0)
        .max(shape.receiver_period_ns.unwrap_or(0));
    let polling_ns = match every_ns {
        Some(p) => p,
        None if m.event_driven => 0,
        None => s.scan_min_ns.unwrap_or(0),
    };
    Some(Latency {
        serialize_ns,
        transport_ns,
        receiver_phase_ns,
        polling_ns,
        total_ns: serialize_ns + transport_ns + receiver_phase_ns + polling_ns,
    })
}

pub(crate) fn speaks(m: &Medium, s: &Manifest, r: &Manifest) -> bool {
    m.speaks.contains(&s.kind)
        && m.speaks.contains(&r.kind)
        && s.timing.contains_key(&m.name)
        && r.timing.contains_key(&m.name)
}

fn alternatives(
    lib: &Library,
    p: &ParsedLink<'_>,
    within: u64,
    now: &Latency,
    shape: Shape,
    overlay: &TimingOverlay,
) -> Vec<Alternative> {
    let (s, r, current, every_ns) = (p.s, p.r, p.m, p.every_ns);
    let mut out = Vec::new();
    // A shorter period on the same medium.
    if now.polling_ns > 0 {
        let rest = now.total_ns - now.polling_ns;
        if rest < within {
            let p = within - rest;
            out.push(Alternative {
                change: format!("every {} (or shorter) on `{}`", fmt_ns(p), current.name),
                latency_ns: within,
            });
        }
    }
    for name in lib.medium_names() {
        let Some(m) = lib.medium(&name) else { continue };
        if !speaks(m, s, r) {
            continue;
        }
        let mut variants = Vec::new();
        if m.name != current.name {
            variants.push((format!("via {}", m.name), every_ns));
        }
        if m.event_driven && every_ns.is_some() {
            variants.push((
                format!("via {} without `every` (event-driven)", m.name),
                None,
            ));
        }
        for (change, every) in variants {
            if let Some(l) = latency_with_overlay(s, r, m, every, shape, overlay) {
                if l.total_ns <= within {
                    out.push(Alternative {
                        change,
                        latency_ns: l.total_ns,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.latency_ns
            .cmp(&b.latency_ns)
            .then(a.change.cmp(&b.change))
    });
    out
}

/// One `link` of a cell, resolved against the manifests.
pub(crate) struct ParsedLink<'a> {
    /// Position among the `link` items of the cell (0-based).
    pub index: usize,
    /// Source as written (`plc.cap_done`).
    pub from: String,
    /// Destination as written.
    pub to: String,
    /// Sender device instance.
    pub sender: String,
    /// Receiver device instance.
    pub receiver: String,
    /// Sender manifest.
    pub s: &'a Manifest,
    /// Receiver manifest.
    pub r: &'a Manifest,
    /// Medium.
    pub m: &'a Medium,
    /// Medium options.
    pub options: Vec<String>,
    /// Period after `every`.
    pub every_ns: Option<u64>,
    /// Budget after `within`.
    pub within_ns: Option<u64>,
    /// Byte range of the `link` item.
    pub span: [usize; 2],
}

/// Resolves every `link` of the cell of `plan`: both ends are devices of the cell, the
/// medium exists and both ends speak it. `NE0607` for each one that is not.
pub(crate) fn parse_links<'a>(
    nir: &Nir,
    plan: &CellPlan,
    lib: &'a Library,
) -> (Vec<ParsedLink<'a>>, Vec<CellDiag>) {
    let Some(cell) = nir
        .modules
        .iter()
        .find(|m| m.kind == ModuleKind::Cell && m.name == plan.cell && m.file == plan.file)
    else {
        return (Vec::new(), Vec::new());
    };
    let file = cell.file.as_str();
    let mut rows = Vec::new();
    let mut diags = Vec::new();
    let mut index = 0usize;
    for item in &cell.items {
        let ItemDef::Link {
            from,
            to,
            via,
            every,
            within,
        } = &item.def
        else {
            continue;
        };
        let this = index;
        index += 1;
        let span = Some(item.span);
        let name = |e: &Expr| dotted(e).map(|d| d.join(".")).unwrap_or_else(|| "?".into());
        let (from_s, to_s) = (name(from), name(to));
        let end = |text: &str| -> Option<(String, &Manifest)> {
            let dev = text.split('.').next()?;
            let p = plan.device(dev)?;
            Some((dev.to_string(), lib.device(&p.manifest)?))
        };
        let (Some((sender, s)), Some((receiver, r))) = (end(&from_s), end(&to_s)) else {
            diags.push(
                CellDiag::new(
                    "NE0607",
                    file,
                    span,
                    format!(
                        "link `{from_s} -> {to_s}`: both ends must be `device.signal` of this cell"
                    ),
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
        let Some((medium_name, options)) = medium_parts(via) else {
            diags.push(CellDiag::new(
                "NE0607",
                file,
                span,
                "`via` must name a medium",
            ));
            continue;
        };
        let Some(m) = lib.medium(&medium_name) else {
            diags.push(
                CellDiag::new(
                    "NE0607",
                    file,
                    span,
                    format!("unknown medium `{medium_name}`"),
                )
                .help(format!("media: {}", lib.medium_names().join(", "))),
            );
            continue;
        };
        if !speaks(m, s, r) {
            let lacks = [(&sender, s), (&receiver, r)]
                .iter()
                .filter(|(_, man)| {
                    !man.timing.contains_key(&m.name) || !m.speaks.contains(&man.kind)
                })
                .map(|(inst, man)| {
                    format!(
                        "`{inst}` ({}) speaks {}",
                        man.name,
                        man.timing.keys().cloned().collect::<Vec<_>>().join(", ")
                    )
                })
                .collect::<Vec<_>>();
            let mut d = CellDiag::new(
                "NE0607",
                file,
                span,
                format!(
                    "link `{from_s} -> {to_s}`: medium `{medium_name}` is not spoken by both ends"
                ),
            );
            for l in lacks {
                d = d.help(l);
            }
            diags.push(d);
            continue;
        }
        rows.push(ParsedLink {
            index: this,
            from: from_s,
            to: to_s,
            sender,
            receiver,
            s,
            r,
            m,
            options,
            every_ns: every.as_ref().and_then(duration_of),
            within_ns: within.as_ref().and_then(duration_of),
            span: item.span,
        });
    }
    (rows, diags)
}

/// The link table of the cell of `plan`. Each link is costed with the real size of its
/// value and the period of the receiver's program (see `endpoints::link_plans`).
///
/// # Errors
/// `NE0607` for an endpoint that is not a device of the cell, an unknown medium or a
/// medium that one end does not speak; `NE0701` for every link over its budget, with
/// the alternatives that would meet it.
pub fn link_table(
    nir: &Nir,
    plan: &CellPlan,
    lib: &Library,
) -> Result<Vec<LinkReport>, Vec<CellDiag>> {
    link_table_with(nir, plan, lib, &TimingOverlay::new())
}

/// The trust of one end's timing on a medium: the profile's trust when `overlay` supplies it,
/// otherwise the manifest's own trust.
fn end_trust(man: &Manifest, m: &Medium, overlay: &TimingOverlay) -> Trust {
    overlay
        .get(&man.name, &m.name)
        .map_or(man.trust, |p| p.trust)
}

/// [`link_table`], but a calibration profile in `overlay` overrides the manifest timing of a
/// device on a medium (task E6.3); each link's trust is the lowest of the numbers actually used,
/// so a link costed from a `measured` profile reports `measured`. An empty overlay reproduces
/// [`link_table`] exactly.
///
/// # Errors
/// The same diagnostics as [`link_table`].
pub fn link_table_with(
    nir: &Nir,
    plan: &CellPlan,
    lib: &Library,
    overlay: &TimingOverlay,
) -> Result<Vec<LinkReport>, Vec<CellDiag>> {
    let (parsed, mut diags) = parse_links(nir, plan, lib);
    let file = plan.file.as_str();
    let mut rows = Vec::new();
    for p in parsed {
        let (s, r, m) = (p.s, p.r, p.m);
        let (from_s, to_s, medium_name) = (&p.from, &p.to, &p.m.name);
        let span = Some(p.span);
        let (every_ns, within_ns) = (p.every_ns, p.within_ns);
        let shape = crate::endpoints::shape_of(nir, plan, &p);
        let Some(lat) = latency_with_overlay(s, r, m, every_ns, shape, overlay) else {
            continue;
        };
        let trust = end_trust(s, m, overlay)
            .min(end_trust(r, m, overlay))
            .min(m.trust);
        let mut row = LinkReport {
            from: from_s.clone(),
            to: to_s.clone(),
            sender: p.sender.clone(),
            receiver: p.receiver.clone(),
            medium: medium_name.clone(),
            options: p.options.clone(),
            every_ns,
            within_ns,
            latency: lat,
            trust,
            alternatives: Vec::new(),
            span: p.span,
        };
        if let Some(w) = within_ns {
            if lat.total_ns > w {
                row.alternatives = alternatives(lib, &p, w, &lat, shape, overlay);
                let mut d = CellDiag::new(
                    "NE0701",
                    file,
                    span,
                    format!(
                        "link `{from_s} -> {to_s}` via {medium_name}: worst case {} exceeds `within {}`",
                        fmt_ns(lat.total_ns),
                        fmt_ns(w)
                    ),
                )
                .help(format!(
                    "serialize {} + transport {} + receiver phase {} + polling {} ({} numbers)",
                    fmt_ns(lat.serialize_ns),
                    fmt_ns(lat.transport_ns),
                    fmt_ns(lat.receiver_phase_ns),
                    fmt_ns(lat.polling_ns),
                    format!("{trust:?}").to_lowercase()
                ));
                if row.alternatives.is_empty() {
                    d = d.help("no medium both devices speak meets it: raise `within`, or move the logic to one device");
                }
                for a in &row.alternatives {
                    d = d.help(format!("{} gives {}", a.change, fmt_ns(a.latency_ns)));
                }
                diags.push(d);
            }
        }
        rows.push(row);
    }
    if diags.is_empty() {
        Ok(rows)
    } else {
        Err(diags)
    }
}
