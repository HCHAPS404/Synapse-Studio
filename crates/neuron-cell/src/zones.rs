//! Zones and mutexes across the cell (task D4.1, requirement D-R7, `NE0805`).
//!
//! `zone press: box(x: 100..220 mm, y: -40..40 mm, z: 0..120 mm)` names a volume and
//! `mutex press between arm, plc.press_cylinder` says which devices may enter it, one
//! at a time. [`zone_plans`] turns every `mutex` into a [`ZonePlan`]:
//!
//! * **Parties.** `arm` is a device (every motion of its robot program into the zone is
//!   an entry); `plc.press_cylinder` is an output of a device (driving it moves
//!   something into the zone, so it is an entry too). Both name a device that must
//!   belong to the cell and, when a module is placed on it, an output that exists.
//! * **Owner.** The lock lives on one device: a `host` that is a party, else the first
//!   party in the mutex list that can run the arbiter (`plc`, `mcu` or `host`; a robot
//!   program has no background task and an `hdl` circuit is not a program that can
//!   arbitrate). A party that cannot reach the owner over any medium is `NE0805`.
//! * **Protocol.** A four-phase handshake with two signals per party,
//!   `z_<zone>_req_<party>` (written by the party) and `z_<zone>_grant_<party>` (written
//!   by the owner). The party raises `req`, enters the zone when it sees `grant`, leaves,
//!   lowers `req` and may ask again only after it sees `grant` low. The owner grants a
//!   party when no other grant is up, keeps the grant while `req` is up and withdraws it
//!   when `req` drops; ties go to the party that is first in the mutex list. The
//!   arbiter is generated as Neuron source in the dialect of the owner
//!   ([`zone_sources`]), like the link endpoints of D3.1.
//! * **Entries.** Every robot `move` whose constant target lies in the zone (box, in the
//!   base frame of the robot, the same frame as the cell), and every party output.
//!
//! # What is wrapped, and what is not
//!
//! The planner finds the entries and generates the lock endpoints on every party. The
//! user's own sequences are **not rewritten**: the acquire before and the release after
//! an entry are a contract that the program (or, later, the transport adapter) keeps,
//! and the plan lists every entry that the contract covers ([`ZoneEntry::wrapped`]).
//! `NE0805` is the entry the contract cannot cover: a motion into a zone by a robot that
//! is not a party of a mutex of that zone, a party that does not exist or cannot take the
//! lock. Targets that are not constant Cartesian points (joint targets, computed
//! values) cannot be placed in the zone statically; they are counted in
//! [`ZonePlan::unchecked_targets`] and the robot checks them only against its own
//! workspace (`NE0804`).
//!
//! The robot base frame is taken to be the cell frame, and the flange target is used
//! (tool offsets are not applied).
use crate::endpoints::{dialect_of, endpoint_period, neuron_duration, slug};
use crate::links::{dotted, latency, parse_links, speaks};
use crate::watchdog::actuators_of;
use crate::{CellDiag, CellPlan, EndpointSource, Kind, Library};
use neuron_emit::robot::Robot;
use neuron_emit::spel::motion_targets;
use neuron_nir::{Expr, ExprKind, ItemDef, Lit, Module, ModuleKind, Nir};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// An axis-aligned box in millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ZoneBox {
    /// `[min, max]` along x.
    pub x: [f64; 2],
    /// `[min, max]` along y.
    pub y: [f64; 2],
    /// `[min, max]` along z.
    pub z: [f64; 2],
}

impl ZoneBox {
    /// Whether `p` (mm) lies in the box, borders included.
    ///
    /// ```
    /// let b = neuron_cell::ZoneBox { x: [0.0, 10.0], y: [0.0, 10.0], z: [0.0, 10.0] };
    /// assert!(b.contains([10.0, 0.0, 5.0]));
    /// assert!(!b.contains([10.1, 0.0, 5.0]));
    /// ```
    pub fn contains(&self, p: [f64; 3]) -> bool {
        let inside = |v: f64, r: [f64; 2]| v >= r[0] && v <= r[1];
        inside(p[0], self.x) && inside(p[1], self.y) && inside(p[2], self.z)
    }
}

/// One party of a mutex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Party {
    /// Device instance.
    pub device: String,
    /// The output that moves into the zone, for `device.output`; `None` for a whole
    /// device (its motions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// What enters the zone.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryKind {
    /// A robot `move` whose constant target lies in the zone.
    Motion {
        /// `ptp`, `linear` or `arc`.
        motion: String,
        /// Target in the base frame, in mm.
        target_mm: [f64; 3],
    },
    /// A party output that drives something into the zone.
    Output {
        /// Output name.
        name: String,
    },
}

/// A place where a device enters a zone.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ZoneEntry {
    /// Device instance.
    pub device: String,
    /// Module of the device that holds the entry (`None` for a party output with no
    /// module placed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// What enters.
    #[serde(flatten)]
    pub entry: EntryKind,
    /// Byte range of the `move`, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<[usize; 2]>,
    /// `true` when the device is a party, so the acquire/release contract covers it. The
    /// program itself is not rewritten (see the module documentation).
    pub wrapped: bool,
}

/// How a party reaches the owner of the lock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LockLink {
    /// Party device.
    pub party: String,
    /// Medium of the request and grant signals (`None` for the owner itself).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium: Option<String>,
    /// Worst one-way delay of the medium, in nanoseconds (0 for the owner itself).
    pub one_way_ns: u64,
}

/// The lock of one `mutex`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ZonePlan {
    /// Zone name.
    pub zone: String,
    /// Its volume.
    pub shape: ZoneBox,
    /// Device that owns the lock.
    pub owner: String,
    /// Parties in mutex order.
    pub parties: Vec<Party>,
    /// Lock transport per party, in mutex order.
    pub locks: Vec<LockLink>,
    /// Entries found in the placed programs and party outputs.
    pub entries: Vec<ZoneEntry>,
    /// Robot targets that could not be placed in the zone (not constant Cartesian).
    pub unchecked_targets: usize,
    /// Facts the planner could not verify, one per line.
    pub notes: Vec<String>,
    /// Byte range of the `mutex` item.
    pub span: [usize; 2],
}

impl ZonePlan {
    /// The distinct party devices in mutex order.
    pub fn devices(&self) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        self.parties
            .iter()
            .filter(|p| seen.insert(p.device.as_str()))
            .map(|p| p.device.as_str())
            .collect()
    }

    /// Name of the request signal of `device`.
    ///
    /// ```
    /// assert_eq!(neuron_cell::lock_signal("press", "arm", true), "z_press_req_arm");
    /// ```
    pub fn req(&self, device: &str) -> String {
        lock_signal(&self.zone, device, true)
    }

    /// Name of the grant signal of `device`.
    pub fn grant(&self, device: &str) -> String {
        lock_signal(&self.zone, device, false)
    }
}

/// Name of the request (`req`) or grant signal of `device` for `zone`.
pub fn lock_signal(zone: &str, device: &str, req: bool) -> String {
    format!(
        "z_{}_{}_{}",
        slug(zone),
        if req { "req" } else { "grant" },
        slug(device)
    )
}

// ---------------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------------

fn number(e: &Expr) -> Option<f64> {
    match &e.e {
        ExprKind::Lit {
            lit: Lit::Int { v, .. } | Lit::Real { v, .. },
        } => Some(neuron_emit::nirx::parse_num(v)),
        ExprKind::Unary { op, operand } if op == "-" => number(operand).map(|v| -v),
        ExprKind::Quantity { value, unit, .. } => Some(number(value)? * mm_per(&unit.text)?),
        _ => None,
    }
}

fn mm_per(unit: &str) -> Option<f64> {
    match unit {
        "mm" => Some(1.0),
        "cm" => Some(10.0),
        "m" => Some(1000.0),
        _ => None,
    }
}

fn unit_of(e: &Expr) -> Option<f64> {
    match &e.e {
        ExprKind::Quantity { unit, .. } => mm_per(&unit.text),
        ExprKind::Unary { operand, .. } => unit_of(operand),
        _ => None,
    }
}

/// `lo..hi` in mm. A bound without a unit takes the unit of the other bound.
fn range_mm(e: &Expr) -> Option<[f64; 2]> {
    let ExprKind::Range { lo, hi, .. } = &e.e else {
        return None;
    };
    let f = unit_of(lo).or_else(|| unit_of(hi))?;
    let bound = |b: &Expr| {
        if unit_of(b).is_some() {
            number(b)
        } else {
            number(b).map(|v| v * f)
        }
    };
    let (a, b) = (bound(lo)?, bound(hi)?);
    (a <= b).then_some([a, b])
}

fn zone_box(shape: &Expr) -> Option<ZoneBox> {
    let ExprKind::Call { callee, args, .. } = &shape.e else {
        return None;
    };
    if !matches!(&callee.e, ExprKind::Global { name } if name == "box") {
        return None;
    }
    let axis = |n: &str, pos: usize| {
        args.iter()
            .find(|a| a.name.as_deref() == Some(n))
            .or_else(|| args.get(pos).filter(|a| a.name.is_none()))
            .and_then(|a| range_mm(&a.value))
    };
    Some(ZoneBox {
        x: axis("x", 0)?,
        y: axis("y", 1)?,
        z: axis("z", 2)?,
    })
}

// ---------------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------------

/// Dialects that can run the arbiter of a lock (a background activity) or ask for it.
fn can_arbitrate(dialect: &str) -> bool {
    matches!(dialect, "plc" | "mcu" | "host")
}

fn can_ask(dialect: &str) -> bool {
    matches!(dialect, "plc" | "mcu" | "host" | "robot")
}

fn cell_of<'a>(nir: &'a Nir, plan: &CellPlan) -> Option<&'a Module> {
    nir.modules
        .iter()
        .find(|m| m.kind == ModuleKind::Cell && m.name == plan.cell && m.file == plan.file)
}

/// The planned locks of the cell of `plan`, and the `NE0805` diagnostics.
///
/// One [`ZonePlan`] per `mutex`, in source order. A diagnostic does not remove the
/// plan it comes from, so tools can show both.
pub fn zone_plans(nir: &Nir, plan: &CellPlan, lib: &Library) -> (Vec<ZonePlan>, Vec<CellDiag>) {
    let Some(cell) = cell_of(nir, plan) else {
        return (Vec::new(), Vec::new());
    };
    let file = cell.file.as_str();
    let mut diags = Vec::new();
    let mut shapes: BTreeMap<String, ZoneBox> = BTreeMap::new();
    for item in &cell.items {
        if let ItemDef::Zone { name, shape } = &item.def {
            match zone_box(shape) {
                Some(b) => {
                    shapes.insert(name.clone(), b);
                }
                None => diags.push(
                    CellDiag::new(
                        "NE0805",
                        file,
                        Some(item.span),
                        format!("zone `{name}` is not a constant `box(x: .., y: .., z: ..)`"),
                    )
                    .help("write each axis as a range with a length unit, e.g. `x: 100..220 mm`"),
                ),
            }
        }
    }
    let links = parse_links(nir, plan, lib).0;
    let mut plans: Vec<ZonePlan> = Vec::new();
    for item in &cell.items {
        let ItemDef::Mutex { zone, parties } = &item.def else {
            continue;
        };
        let span = item.span;
        let Some(shape) = shapes.get(zone).copied() else {
            diags.push(
                CellDiag::new(
                    "NE0805",
                    file,
                    Some(span),
                    format!("`mutex {zone}` names a zone that this cell does not declare"),
                )
                .help("declare it with `zone <name>: box(x: .., y: .., z: ..)`"),
            );
            continue;
        };
        let mut ps: Vec<Party> = Vec::new();
        let mut notes = Vec::new();
        for e in parties {
            let parts = dotted(e).unwrap_or_default();
            let (dev, out) = match parts.as_slice() {
                [d] => (d.clone(), None),
                [d, o] => (d.clone(), Some(o.clone())),
                _ => {
                    diags.push(CellDiag::new(
                        "NE0805",
                        file,
                        Some(span),
                        format!("`mutex {zone}`: a party is `device` or `device.output`"),
                    ));
                    continue;
                }
            };
            let Some(dp) = plan.device(&dev) else {
                diags.push(
                    CellDiag::new(
                        "NE0805",
                        file,
                        Some(span),
                        format!(
                            "`mutex {zone}`: `{dev}` is not a device of cell `{}`",
                            plan.cell
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
            if let Some(o) = &out {
                if dp.modules.is_empty() {
                    notes.push(format!(
                        "output `{dev}.{o}` was not checked: no module is placed on `{dev}`"
                    ));
                } else if !actuators_of(nir, plan, &dev).contains(o) {
                    diags.push(
                        CellDiag::new(
                            "NE0805",
                            file,
                            Some(span),
                            format!("`mutex {zone}`: `{dev}` has no output `{o}`"),
                        )
                        .help(format!(
                            "outputs of the modules placed on `{dev}`: {}",
                            actuators_of(nir, plan, &dev).join(", ")
                        )),
                    );
                    // The device is real; only the output is dropped from the plan.
                    ps.push(Party {
                        device: dev,
                        output: None,
                    });
                    continue;
                }
            }
            ps.push(Party {
                device: dev,
                output: out,
            });
        }
        let dialect_of_dev = |d: &str| {
            plan.device(d)
                .and_then(|p| lib.device(&p.manifest))
                .map(|m| dialect_of(m).to_string())
                .unwrap_or_default()
        };
        let mut devices: Vec<String> = Vec::new();
        for p in &ps {
            if !devices.contains(&p.device) {
                devices.push(p.device.clone());
            }
        }
        for d in &devices {
            let dia = dialect_of_dev(d);
            if !can_ask(&dia) {
                diags.push(
                    CellDiag::new(
                        "NE0805",
                        file,
                        Some(span),
                        format!(
                            "`mutex {zone}`: `{d}` runs `{dia}`, which cannot take part in a lock"
                        ),
                    )
                    .help("a party runs plc, mcu, host or robot programs"),
                );
            }
        }
        // Owner: a host that is a party, else the first party that can arbitrate.
        let kind_of = |d: &str| plan.device(d).map(|p| p.kind);
        let owner = devices
            .iter()
            .find(|d| kind_of(d) == Some(Kind::Host))
            .or_else(|| devices.iter().find(|d| can_arbitrate(&dialect_of_dev(d))))
            .cloned();
        let Some(owner) = owner else {
            if !devices.is_empty() {
                diags.push(
                    CellDiag::new(
                        "NE0805",
                        file,
                        Some(span),
                        format!("`mutex {zone}`: no party can own the lock"),
                    )
                    .help("the owner runs the arbiter: add a plc, mcu or host as a party"),
                );
            }
            continue;
        };
        // Transport of the lock signals.
        let mut locks = Vec::new();
        for d in &devices {
            if *d == owner {
                locks.push(LockLink {
                    party: d.clone(),
                    medium: None,
                    one_way_ns: 0,
                });
                continue;
            }
            match lock_medium(plan, lib, &links, d, &owner) {
                Some((m, ns)) => locks.push(LockLink {
                    party: d.clone(),
                    medium: Some(m),
                    one_way_ns: ns,
                }),
                None => diags.push(
                    CellDiag::new(
                        "NE0805",
                        file,
                        Some(span),
                        format!(
                            "`mutex {zone}`: `{d}` and the owner `{owner}` share no medium for the lock"
                        ),
                    )
                    .help("add a `link` between them over a medium both speak"),
                ),
            }
        }
        // Party outputs are entries.
        let mut entries: Vec<ZoneEntry> = ps
            .iter()
            .filter_map(|p| {
                p.output.as_ref().map(|o| ZoneEntry {
                    device: p.device.clone(),
                    module: None,
                    entry: EntryKind::Output { name: o.clone() },
                    span: Some(span),
                    wrapped: true,
                })
            })
            .collect();
        plans.push(ZonePlan {
            zone: zone.clone(),
            shape,
            owner,
            parties: ps,
            locks,
            entries: std::mem::take(&mut entries),
            unchecked_targets: 0,
            notes,
            span,
        });
    }
    robot_entries(nir, plan, lib, &shapes, file, &mut plans, &mut diags);
    for p in &mut plans {
        p.entries
            .sort_by(|a, b| (&a.device, &a.module, a.span).cmp(&(&b.device, &b.module, b.span)));
    }
    (plans, diags)
}

/// Finds every robot motion that ends in a zone; uncovered ones are `NE0805`.
fn robot_entries(
    nir: &Nir,
    plan: &CellPlan,
    lib: &Library,
    shapes: &BTreeMap<String, ZoneBox>,
    file: &str,
    plans: &mut [ZonePlan],
    diags: &mut Vec<CellDiag>,
) {
    for dev in &plan.devices {
        if dev.kind != Kind::Robot {
            continue;
        }
        let Some(robot) = lib
            .device(&dev.manifest)
            .and_then(|m| Robot::builtin(&m.name))
        else {
            continue;
        };
        for placed in &dev.modules {
            let Some(mi) = nir
                .modules
                .iter()
                .position(|m| m.kind == ModuleKind::Module && m.name == placed.module)
            else {
                continue;
            };
            if nir.modules[mi].dialect != "robot" {
                continue;
            }
            let targets = motion_targets(nir, mi, &robot);
            let unchecked = targets.iter().filter(|t| t.world_mm.is_none()).count();
            for (zone, shape) in shapes {
                let covering: Vec<usize> = plans
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| {
                        &p.zone == zone && p.parties.iter().any(|q| q.device == dev.instance)
                    })
                    .map(|(i, _)| i)
                    .collect();
                let mut flagged = false;
                for t in &targets {
                    let Some(w) = t.world_mm else { continue };
                    if !shape.contains(w) {
                        continue;
                    }
                    if covering.is_empty() {
                        let mut d = CellDiag::new(
                            "NE0805",
                            &nir.modules[mi].file,
                            Some(t.span),
                            format!(
                                "`move {}` of `{}` on `{}` enters zone `{zone}` at ({}, {}, {}) mm without a lock",
                                t.kind,
                                t.module,
                                dev.instance,
                                neuron_emit::nirx::fmt_num(w[0]),
                                neuron_emit::nirx::fmt_num(w[1]),
                                neuron_emit::nirx::fmt_num(w[2])
                            ),
                        );
                        if !flagged {
                            d = d.help(format!(
                                "add `{}` to a `mutex {zone} between ...` in cell `{}` ({file})",
                                dev.instance, plan.cell
                            ));
                            flagged = true;
                        }
                        diags.push(d);
                    }
                    for i in &covering {
                        plans[*i].entries.push(ZoneEntry {
                            device: dev.instance.clone(),
                            module: Some(t.module.clone()),
                            entry: EntryKind::Motion {
                                motion: t.kind.clone(),
                                target_mm: w,
                            },
                            span: Some(t.span),
                            wrapped: true,
                        });
                    }
                }
                for i in &covering {
                    plans[*i].unchecked_targets += unchecked;
                }
            }
        }
    }
}

/// The medium for the lock signals between a party and the owner: the medium of an
/// existing `link` between the two devices when there is one, else the one that both
/// speak with the lowest worst-case latency. Returns the name and the worst one-way delay.
fn lock_medium(
    plan: &CellPlan,
    lib: &Library,
    links: &[crate::links::ParsedLink<'_>],
    a: &str,
    b: &str,
) -> Option<(String, u64)> {
    let ma = lib.device(&plan.device(a)?.manifest)?;
    let mb = lib.device(&plan.device(b)?.manifest)?;
    let cost = |name: &str| -> Option<u64> {
        let m = lib.medium(name)?;
        if !speaks(m, ma, mb) {
            return None;
        }
        let ab = latency(ma, mb, m, None)?.total_ns;
        let ba = latency(mb, ma, m, None)?.total_ns;
        Some(ab.max(ba))
    };
    let used = links
        .iter()
        .filter(|l| (l.sender == a && l.receiver == b) || (l.sender == b && l.receiver == a))
        .filter_map(|l| cost(&l.m.name).map(|c| (l.m.name.clone(), c)))
        .min_by_key(|(n, c)| (*c, n.clone()));
    used.or_else(|| {
        lib.medium_names()
            .into_iter()
            .filter_map(|n| cost(&n).map(|c| (n, c)))
            .min_by_key(|(n, c)| (*c, n.clone()))
    })
}

// ---------------------------------------------------------------------------------
// Generated source
// ---------------------------------------------------------------------------------

fn module_header(dialect: &str, target: Option<&str>, name: &str) -> String {
    match (dialect, target) {
        ("mcu", Some(b)) => format!("module {name} for mcu on {b}\n"),
        (d, _) => format!("module {name} for {d}\n"),
    }
}

/// The arbiter equations: `grant_i = req_i and (grant_i or not (other grants))`, in
/// mutex order, each rung seeing the grants already updated by the earlier ones.
fn arbiter_lines(z: &ZonePlan) -> Vec<String> {
    let devs = z.devices();
    devs.iter()
        .map(|d| {
            let others: Vec<String> = devs
                .iter()
                .filter(|o| *o != d)
                .map(|o| z.grant(o))
                .collect();
            let free = match others.len() {
                0 => String::new(),
                1 => format!(" and ({} or not {})", z.grant(d), others[0]),
                _ => format!(" and ({} or not ({}))", z.grant(d), others.join(" or ")),
            };
            format!("{} = {}{}", z.grant(d), z.req(d), free)
        })
        .collect()
}

/// The value of every grant after one arbiter scan: the model that the generated rungs
/// implement, used by the tests to check exclusion.
///
/// `grants` and `reqs` are indexed like the party devices of the plan; the grants are
/// updated in order, as the generated rungs are.
///
/// ```
/// let g = neuron_cell::arbitrate(&[true, true], &[false, false]);
/// assert_eq!(g, vec![true, false]);
/// ```
pub fn arbitrate(reqs: &[bool], grants: &[bool]) -> Vec<bool> {
    let mut g = grants.to_vec();
    for i in 0..reqs.len().min(g.len()) {
        let others = g.iter().enumerate().any(|(j, v)| j != i && *v);
        g[i] = reqs[i] && (g[i] || !others);
    }
    g
}

/// Generates the lock modules of every device that takes part in a lock, keyed by
/// device instance: one arbiter module on the owner and one small module with the
/// request and grant signals on every other party. A mutex whose owner or party
/// cannot be generated (an `hdl` device) is skipped; [`zone_plans`] reports it.
pub fn zone_sources(
    nir: &Nir,
    plan: &CellPlan,
    lib: &Library,
    zones: &[ZonePlan],
) -> BTreeMap<String, Vec<EndpointSource>> {
    let mut out: BTreeMap<String, Vec<EndpointSource>> = BTreeMap::new();
    for z in zones {
        for dev in z.devices() {
            let Some(man) = plan.device(dev).and_then(|d| lib.device(&d.manifest)) else {
                continue;
            };
            let dialect = dialect_of(man);
            if !can_ask(dialect) {
                continue;
            }
            let owner = dev == z.owner;
            let module = format!(
                "lock_{}_{}_{}",
                slug(&plan.cell),
                slug(&z.zone),
                if owner {
                    "owner".to_string()
                } else {
                    slug(dev)
                }
            );
            let period = endpoint_period(nir, plan, dev, man);
            let mut s = module_header(dialect, man.target.as_deref(), &module);
            s.push_str(&format!(
                "## Lock of zone `{}` ({}); generated by `neuron cell build`.\n",
                z.zone,
                if owner { "owner and arbiter" } else { "party" }
            ));
            let names: Vec<String> = if owner {
                z.devices()
                    .iter()
                    .flat_map(|d| [z.req(d), z.grant(d)])
                    .collect()
            } else {
                vec![z.req(dev), z.grant(dev)]
            };
            match dialect {
                "plc" => {
                    s.push_str(&format!("scan {}\nmemory:\n", neuron_duration(period)));
                    for n in &names {
                        s.push_str(&format!("  {n}: bool\n"));
                    }
                    if owner {
                        for (i, line) in arbiter_lines(z).iter().enumerate() {
                            s.push_str(&format!(
                                "rung z_{}_arbiter_{i}:\n  {line}\n",
                                slug(&z.zone)
                            ));
                        }
                    }
                }
                "robot" => {
                    for n in &names {
                        s.push_str(&format!("var {n}: bool = false\n"));
                    }
                }
                _ => {
                    for n in &names {
                        s.push_str(&format!("signal {n}: bool\n"));
                    }
                    if owner {
                        let attr = if dialect == "mcu" { " (prio 1)" } else { "" };
                        s.push_str(&format!(
                            "task z_{}_arbiter every {}{attr}:\n",
                            slug(&z.zone),
                            neuron_duration(period)
                        ));
                        for line in arbiter_lines(z) {
                            s.push_str(&format!("  {line}\n"));
                        }
                    }
                }
            }
            out.entry(dev.to_string())
                .or_default()
                .push(EndpointSource {
                    device: dev.to_string(),
                    module: module.clone(),
                    file: format!("{module}.neuron"),
                    text: s,
                });
        }
    }
    out
}
