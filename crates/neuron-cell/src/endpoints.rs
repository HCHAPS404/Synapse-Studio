//! Link endpoints (task D3.1): for every `link` of a cell, the plan of its two ends and
//! the Neuron source of the endpoint modules that are added to the device packages.
//!
//! # What a [`LinkPlan`] decides
//!
//! * **Pattern** from the medium family (design D3): register, frame, pub/sub or
//!   zero-hop.
//! * **Value type.** The type of the sender's signal when a module placed on the sender
//!   declares one (a `signal`, a PLC `inputs`/`outputs`/`memory` variable, an `io`
//!   binding), else the receiver's, else the default `bool` (the corpus links carry
//!   control flags: ready, start, part present). The plan says which one was used.
//! * **Crossing.** Each end has an effective period: the longer of the manifest's
//!   `scan_min` and the period (`scan`, or the shortest task `every`) of the module that
//!   declares the signal. A sender faster than the receiver gets **Latch+ACK**: the
//!   sender holds the value until the receiver echoes the sequence number after sampling
//!   it in its own cycle. A slower sender gets **sample + stale flag**. Equal periods are
//!   **direct**. A receiver that is a robot (motion takes time) over a medium that is not
//!   a bare wire gets **BUSY/DONE/TIMEOUT**; the timeout is [`ACTION_TIMEOUT_NS`].
//! * **Location of each end.** Register media: a block of holding registers on the
//!   receiver, one register per 16 bits, allocated from [`REGISTER_BASE`] in link order
//!   per receiver, laid out as `value`, then `seq` and `ack` (Latch+ACK) or `seq`, `busy`
//!   and `done` (action), one register each (a `bool` travels as `0` or `1`). Frame media:
//!   a Neuron Wire tag id, `link index + 1`, the same on both ends. Pub/sub media: the
//!   topic `neuron/<cell>/l<index>`. Zero-hop: a logical pin name (`l<index>_out`,
//!   `l<index>_in`; the physical pin comes from the device's I/O plan).
//!
//! # Endpoint modules
//!
//! Decision 1 of the design: the endpoints are Neuron source, one module per device and
//! link side (`link_<cell>_<index>_tx`, `..._rx`), in the dialect of the device. They are
//! analyzed with the device's modules and built by the device's own backend. They
//! declare the endpoint signals (`l<index>_value`, and `_seq`, `_ack`, `_hold`, `_busy`,
//! `_done`, `_timeout`, `_stale` as the pattern needs) and the small handshake logic
//! (hold until acknowledged, echo the sequence number, the action timer). Connecting an
//! endpoint to the signal of a user module and applying the register or Wire binding at
//! run time belong to the transport adapter and to D3.4 (tags); the co-simulation of D6.1
//! moves the values.
use crate::links::{duration_of, parse_links, ParsedLink, Shape};
use crate::{CellDiag, CellPlan, Family, Kind, Library, Manifest};
use neuron_nir::{ItemDef, Module, ModuleKind, Nir, Trigger, Type};
use serde::Serialize;
use std::collections::BTreeMap;

/// First holding register of the register map of a device.
pub const REGISTER_BASE: u16 = 100;
/// Timeout of the BUSY/DONE/TIMEOUT handshake, in nanoseconds (1 s).
pub const ACTION_TIMEOUT_NS: u64 = 1_000_000_000;
/// Period of an endpoint module when no module placed on the device gives one (10 ms).
pub const DEFAULT_ENDPOINT_PERIOD_NS: u64 = 10_000_000;

/// Endpoint pattern, from the medium family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pattern {
    /// Holding registers or data block addresses.
    Register,
    /// Neuron Wire frames.
    Frame,
    /// Publish and subscribe.
    Pubsub,
    /// A wire between two pins.
    ZeroHop,
}

/// How a value crosses between two time domains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Crossing {
    /// Same period: the receiver reads the value as it is.
    Direct,
    /// Fast sender, slow receiver: the sender holds the value until the receiver echoes
    /// its sequence number.
    LatchAck,
    /// Slow sender, fast receiver: the receiver samples and keeps a stale flag.
    SampleStale,
    /// Asynchronous action: BUSY, DONE and TIMEOUT.
    Action,
}

/// Where one end of a link lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Location {
    /// A block of holding registers on `owner` (the receiver).
    Register {
        /// Device that owns the registers.
        owner: String,
        /// First register (the value).
        value: u16,
        /// Registers of the value.
        registers: u16,
        /// Sequence number register (Latch+ACK and actions).
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u16>,
        /// Acknowledge register (Latch+ACK).
        #[serde(skip_serializing_if = "Option::is_none")]
        ack: Option<u16>,
        /// BUSY register (actions).
        #[serde(skip_serializing_if = "Option::is_none")]
        busy: Option<u16>,
        /// DONE register (actions).
        #[serde(skip_serializing_if = "Option::is_none")]
        done: Option<u16>,
    },
    /// A Neuron Wire tag id.
    WireTag {
        /// The id carried in DATA frames.
        id: u16,
    },
    /// A topic.
    Topic {
        /// Topic name.
        name: String,
    },
    /// A logical pin.
    Pin {
        /// Pin name.
        name: String,
        /// Name of the acknowledge wire when the crossing is Latch+ACK.
        #[serde(skip_serializing_if = "Option::is_none")]
        ack: Option<String>,
    },
}

/// One end of a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Endpoint {
    /// Device instance.
    pub device: String,
    /// Generated endpoint module.
    pub module: String,
    /// Where the value lives.
    pub location: Location,
    /// Names the endpoint module declares, in order.
    pub names: Vec<String>,
}

/// The plan of one link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkPlan {
    /// Position among the `link` items of the cell.
    pub index: usize,
    /// Source as written.
    pub from: String,
    /// Destination as written.
    pub to: String,
    /// Medium name.
    pub medium: String,
    /// Endpoint pattern.
    pub pattern: Pattern,
    /// Neuron type of the value.
    pub value_type: String,
    /// Bytes of the value on the wire (spec 11.3).
    pub value_bytes: u64,
    /// Where the type came from.
    pub type_source: String,
    /// Crossing mechanism.
    pub crossing: Crossing,
    /// Why that mechanism.
    pub crossing_reason: String,
    /// Effective period of the sender, in nanoseconds.
    pub sender_period_ns: u64,
    /// Effective period of the receiver, in nanoseconds.
    pub receiver_period_ns: u64,
    /// Timeout of an action handshake.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_timeout_ns: Option<u64>,
    /// Sender end.
    pub tx: Endpoint,
    /// Receiver end.
    pub rx: Endpoint,
    /// Fidelity notes: what the endpoints do not do.
    pub notes: Vec<String>,
}

/// The generated source of one endpoint module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointSource {
    /// Device instance that runs it.
    pub device: String,
    /// Module name.
    pub module: String,
    /// Source file name given to the analyzer.
    pub file: String,
    /// Neuron source.
    pub text: String,
}

/// A row of the register map of one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegisterRow {
    /// Device that owns the register.
    pub owner: String,
    /// Link index.
    pub link: usize,
    /// `value`, `seq`, `ack`, `busy` or `done`.
    pub field: String,
    /// Holding register address.
    pub address: u16,
    /// Registers used.
    pub registers: u16,
    /// Neuron type of the field.
    pub ty: String,
    /// Device that writes it.
    pub writer: String,
}

// ---------------------------------------------------------------------------------
// Types and periods
// ---------------------------------------------------------------------------------

/// Name and size of a wire type; `None` for a type endpoints do not carry.
fn wire_type(t: &Type) -> Option<(String, u64)> {
    match t {
        Type::Bool => Some(("bool".into(), 1)),
        Type::Num { kind, .. } => {
            let bytes = match kind.as_str() {
                "u8" | "i8" => 1,
                "u16" | "i16" => 2,
                "u32" | "i32" | "f32" => 4,
                "u64" | "i64" | "f64" => 8,
                _ => return None,
            };
            Some((kind.clone(), bytes))
        }
        _ => None,
    }
}

fn module_period(m: &Module) -> Option<u64> {
    let mut best: Option<u64> = None;
    let mut take = |ns: u64| best = Some(best.map_or(ns, |b| b.min(ns)));
    for item in &m.items {
        match &item.def {
            ItemDef::Scan { period } => {
                if let Some(ns) = duration_of(period) {
                    take(ns);
                }
            }
            ItemDef::Task {
                trigger: Trigger::Every { period, .. },
                ..
            } => {
                if let Some(ns) = duration_of(period) {
                    take(ns);
                }
            }
            _ => {}
        }
    }
    best
}

/// What a module placed on `device` says about a signal.
struct Found<'a> {
    module: &'a Module,
    ty: Option<Type>,
}

fn find_signal<'a>(nir: &'a Nir, plan: &CellPlan, device: &str, end: &str) -> Option<Found<'a>> {
    let name = end.split_once('.')?.1;
    let dev = plan.device(device)?;
    for placed in &dev.modules {
        let Some(m) = nir
            .modules
            .iter()
            .find(|m| m.kind == ModuleKind::Module && m.name == placed.module)
        else {
            continue;
        };
        for item in &m.items {
            let ty = match &item.def {
                ItemDef::Signal {
                    name: n,
                    vty: Some(t),
                    ..
                } if n == name => Some(t.clone()),
                ItemDef::Signal { name: n, .. } if n == name => None,
                ItemDef::PlcSection { vars, .. } => match vars.iter().find(|v| v.name == name) {
                    Some(v) => Some(v.vty.clone()),
                    None => continue,
                },
                ItemDef::Io { name: n, .. } if n == name => Some(Type::Bool),
                _ => continue,
            };
            return Some(Found { module: m, ty });
        }
    }
    None
}

struct Facts {
    ty: String,
    bytes: u64,
    source: String,
    sender_period: u64,
    receiver_period: u64,
    receiver_program_period: Option<u64>,
}

fn facts_of(nir: &Nir, plan: &CellPlan, p: &ParsedLink<'_>) -> Facts {
    let tx = find_signal(nir, plan, &p.sender, &p.from);
    let rx = find_signal(nir, plan, &p.receiver, &p.to);
    let from_tx = tx
        .as_ref()
        .and_then(|f| f.ty.as_ref().and_then(wire_type).map(|w| (w, f.module)));
    let from_rx = rx
        .as_ref()
        .and_then(|f| f.ty.as_ref().and_then(wire_type).map(|w| (w, f.module)));
    let ((ty, bytes), source) = match (from_tx, from_rx) {
        (Some((w, m)), _) => (w, format!("signal of module `{}` on the sender", m.name)),
        (None, Some((w, m))) => (w, format!("signal of module `{}` on the receiver", m.name)),
        (None, None) => (
            ("bool".to_string(), 1),
            "default (no placed module declares the signal)".to_string(),
        ),
    };
    let tx_prog = tx.as_ref().and_then(|f| module_period(f.module));
    let rx_prog = rx.as_ref().and_then(|f| module_period(f.module));
    Facts {
        ty,
        bytes,
        source,
        sender_period: p.s.scan_min_ns.unwrap_or(0).max(tx_prog.unwrap_or(0)),
        receiver_period: p.r.scan_min_ns.unwrap_or(0).max(rx_prog.unwrap_or(0)),
        receiver_program_period: rx_prog,
    }
}

/// The value size and receiver period of a link, for the latency formula.
pub(crate) fn shape_of(nir: &Nir, plan: &CellPlan, p: &ParsedLink<'_>) -> Shape {
    let f = facts_of(nir, plan, p);
    Shape {
        value_bytes: f.bytes,
        receiver_period_ns: f.receiver_program_period,
    }
}

// ---------------------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------------------

fn registers_of(bytes: u64) -> u16 {
    u16::try_from(bytes.div_ceil(2).max(1)).unwrap_or(u16::MAX)
}

fn slug(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push('c');
    }
    out
}

fn module_name(cell: &str, index: usize, side: &str) -> String {
    format!("link_{}_{index}_{side}", slug(cell))
}

/// The plan of every `link` of the cell of `plan`.
///
/// # Errors
/// `NE0607` for the links that [`crate::link_table`] also refuses (a device that is not
/// of the cell, an unknown medium, a medium one end does not speak) and for a register
/// map that does not fit in 16-bit addresses. Latency budgets are not checked here.
pub fn link_plans(
    nir: &Nir,
    plan: &CellPlan,
    lib: &Library,
) -> Result<Vec<LinkPlan>, Vec<CellDiag>> {
    let (parsed, mut diags) = parse_links(nir, plan, lib);
    let mut next_register: BTreeMap<String, u32> = BTreeMap::new();
    let mut plans = Vec::new();
    for p in &parsed {
        let f = facts_of(nir, plan, p);
        let pattern = match p.m.family {
            Family::Register => Pattern::Register,
            Family::Frame => Pattern::Frame,
            Family::Pubsub => Pattern::Pubsub,
            Family::ZeroHop => Pattern::ZeroHop,
        };
        let mut notes = Vec::new();
        let (crossing, reason) = if p.r.kind == Kind::Robot && pattern != Pattern::ZeroHop {
            (
                Crossing::Action,
                "the receiver is a robot: its motion takes time, so the sender gets BUSY, DONE and TIMEOUT"
                    .to_string(),
            )
        } else if f.sender_period < f.receiver_period {
            (
                Crossing::LatchAck,
                format!(
                    "sender period {} is shorter than receiver period {}: Latch+ACK",
                    crate::fmt_ns(f.sender_period),
                    crate::fmt_ns(f.receiver_period)
                ),
            )
        } else if f.sender_period > f.receiver_period {
            (
                Crossing::SampleStale,
                format!(
                    "sender period {} is longer than receiver period {}: sample with a stale flag",
                    crate::fmt_ns(f.sender_period),
                    crate::fmt_ns(f.receiver_period)
                ),
            )
        } else {
            (
                Crossing::Direct,
                format!("both ends run at {}", crate::fmt_ns(f.sender_period)),
            )
        };
        if f.source.starts_with("default") {
            notes.push(format!("value type `bool` is the default: {}", f.source));
        }
        let n = p.index;
        let (tx_loc, rx_loc) = match pattern {
            Pattern::Register => {
                let regs = registers_of(f.bytes);
                let extra: u32 = match crossing {
                    Crossing::LatchAck => 2,
                    Crossing::Action => 3,
                    _ => 0,
                };
                let slot = next_register
                    .entry(p.receiver.clone())
                    .or_insert(u32::from(REGISTER_BASE));
                let first = *slot;
                *slot += u32::from(regs) + extra;
                let (Ok(value), Ok(_)) = (u16::try_from(first), u16::try_from(*slot)) else {
                    diags.push(CellDiag::new(
                        "NE0607",
                        &plan.file,
                        Some(p.span),
                        format!(
                            "link `{} -> {}`: the register map of `{}` does not fit in 16-bit addresses",
                            p.from, p.to, p.receiver
                        ),
                    ));
                    continue;
                };
                let after = value + regs;
                let loc = Location::Register {
                    owner: p.receiver.clone(),
                    value,
                    registers: regs,
                    seq: (extra > 0).then_some(after),
                    ack: (crossing == Crossing::LatchAck).then_some(after + 1),
                    busy: (crossing == Crossing::Action).then_some(after + 1),
                    done: (crossing == Crossing::Action).then_some(after + 2),
                };
                (loc.clone(), loc)
            }
            Pattern::Frame => {
                let id = u16::try_from(n + 1).unwrap_or(u16::MAX);
                (Location::WireTag { id }, Location::WireTag { id })
            }
            Pattern::Pubsub => {
                let name = format!("neuron/{}/l{n}", slug(&plan.cell));
                (
                    Location::Topic { name: name.clone() },
                    Location::Topic { name },
                )
            }
            Pattern::ZeroHop => {
                let ack = (crossing == Crossing::LatchAck).then(|| format!("l{n}_ack"));
                (
                    Location::Pin {
                        name: format!("l{n}_out"),
                        ack: ack.clone(),
                    },
                    Location::Pin {
                        name: format!("l{n}_in"),
                        ack,
                    },
                )
            }
        };
        plans.push(LinkPlan {
            index: n,
            from: p.from.clone(),
            to: p.to.clone(),
            medium: p.m.name.clone(),
            pattern,
            value_type: f.ty,
            value_bytes: f.bytes,
            type_source: f.source,
            crossing,
            crossing_reason: reason,
            sender_period_ns: f.sender_period,
            receiver_period_ns: f.receiver_period,
            action_timeout_ns: (crossing == Crossing::Action).then_some(ACTION_TIMEOUT_NS),
            tx: Endpoint {
                device: p.sender.clone(),
                module: module_name(&plan.cell, n, "tx"),
                location: tx_loc,
                names: names_of(n, pattern, crossing, true),
            },
            rx: Endpoint {
                device: p.receiver.clone(),
                module: module_name(&plan.cell, n, "rx"),
                location: rx_loc,
                names: names_of(n, pattern, crossing, false),
            },
            notes,
        });
    }
    if diags.is_empty() {
        Ok(plans)
    } else {
        Err(diags)
    }
}

/// Names an endpoint module declares, in declaration order.
fn names_of(n: usize, pattern: Pattern, crossing: Crossing, tx: bool) -> Vec<String> {
    let mut v = vec![format!("l{n}_value")];
    let extra: &[&str] = match (crossing, tx) {
        (Crossing::LatchAck, true) => &["seq", "ack", "hold"],
        (Crossing::LatchAck, false) => &["seq", "ack"],
        (Crossing::Action, true) => &["seq", "busy", "done", "timeout"],
        (Crossing::Action, false) => &["seq", "busy", "done"],
        (Crossing::SampleStale, false) => &["stale"],
        _ => &[],
    };
    for s in extra {
        v.push(format!("l{n}_{s}"));
    }
    let _ = pattern;
    v
}

/// The register map rows of a set of plans, ordered by owner, then address.
pub fn register_map(plans: &[LinkPlan]) -> Vec<RegisterRow> {
    let mut rows = Vec::new();
    for l in plans {
        let Location::Register {
            owner,
            value,
            registers,
            seq,
            ack,
            busy,
            done,
        } = &l.tx.location
        else {
            continue;
        };
        let mut row = |field: &str, address: u16, regs: u16, ty: &str, writer: &str| {
            rows.push(RegisterRow {
                owner: owner.clone(),
                link: l.index,
                field: field.into(),
                address,
                registers: regs,
                ty: ty.into(),
                writer: writer.into(),
            });
        };
        row("value", *value, *registers, &l.value_type, &l.tx.device);
        if let Some(a) = seq {
            row("seq", *a, 1, "u16", &l.tx.device);
        }
        if let Some(a) = ack {
            row("ack", *a, 1, "u16", &l.rx.device);
        }
        if let Some(a) = busy {
            row("busy", *a, 1, "u16", &l.rx.device);
        }
        if let Some(a) = done {
            row("done", *a, 1, "u16", &l.rx.device);
        }
    }
    rows.sort_by(|a, b| (&a.owner, a.address, a.link).cmp(&(&b.owner, b.address, b.link)));
    rows
}

/// The register map as CSV, for the rows that involve `device` (as owner or as a client).
pub fn register_csv(plans: &[LinkPlan], device: &str) -> Option<String> {
    let rows: Vec<RegisterRow> = register_map(plans)
        .into_iter()
        .filter(|r| {
            plans
                .iter()
                .find(|l| l.index == r.link)
                .is_some_and(|l| l.tx.device == device || l.rx.device == device)
        })
        .collect();
    if rows.is_empty() {
        return None;
    }
    let mut out = String::from("owner,link,field,address,registers,type,writer\n");
    for r in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            r.owner, r.link, r.field, r.address, r.registers, r.ty, r.writer
        ));
    }
    Some(out)
}

// ---------------------------------------------------------------------------------
// Neuron source of the endpoint modules
// ---------------------------------------------------------------------------------

fn neuron_duration(ns: u64) -> String {
    if ns >= 1_000_000_000 && ns % 1_000_000_000 == 0 {
        format!("{} s", ns / 1_000_000_000)
    } else if ns >= 1_000_000 && ns % 1_000_000 == 0 {
        format!("{} ms", ns / 1_000_000)
    } else if ns >= 1_000 && ns % 1_000 == 0 {
        format!("{} us", ns / 1_000)
    } else {
        format!("{ns} ns")
    }
}

/// The dialect an endpoint module is written in: the first dialect of the manifest that
/// is not `core` or `ui`.
fn dialect_of(man: &Manifest) -> &str {
    man.dialects
        .iter()
        .map(String::as_str)
        .find(|d| *d != "core" && *d != "ui")
        .unwrap_or("core")
}

fn hdl_type(t: &str) -> String {
    match t {
        "bool" => "bit".into(),
        "u8" => "bit<8>".into(),
        "u16" => "bit<16>".into(),
        "u32" | "f32" => "bit<32>".into(),
        "u64" | "f64" => "bit<64>".into(),
        "i8" => "sbit<8>".into(),
        "i16" => "sbit<16>".into(),
        "i32" => "sbit<32>".into(),
        "i64" => "sbit<64>".into(),
        other => other.into(),
    }
}

/// The period an endpoint module of `device` runs at: that of the first placed module
/// with one, else the larger of the manifest's `scan_min` and 10 ms.
fn endpoint_period(nir: &Nir, plan: &CellPlan, device: &str, man: &Manifest) -> u64 {
    plan.device(device)
        .into_iter()
        .flat_map(|d| d.modules.iter())
        .filter_map(|p| {
            nir.modules
                .iter()
                .find(|m| m.kind == ModuleKind::Module && m.name == p.module)
        })
        .find_map(module_period)
        .unwrap_or_else(|| man.scan_min_ns.unwrap_or(0).max(DEFAULT_ENDPOINT_PERIOD_NS))
}

struct Side<'a> {
    l: &'a LinkPlan,
    tx: bool,
    dialect: &'a str,
    target: Option<&'a str>,
    period_ns: u64,
}

impl Side<'_> {
    fn header(&self, module: &str) -> String {
        match (self.dialect, self.target) {
            ("mcu", Some(b)) => format!("module {module} for mcu on {b}\n"),
            (d, _) => format!("module {module} for {d}\n"),
        }
    }

    fn ty(&self) -> String {
        if self.dialect == "hdl" {
            hdl_type(&self.l.value_type)
        } else {
            self.l.value_type.clone()
        }
    }

    fn reg(&self, which: &str) -> Option<u16> {
        match &self.l.tx.location {
            Location::Register {
                value,
                seq,
                ack,
                busy,
                done,
                ..
            } => match which {
                "value" => Some(*value),
                "seq" => *seq,
                "ack" => *ack,
                "busy" => *busy,
                "done" => *done,
                _ => None,
            },
            _ => None,
        }
    }

    fn render(&self) -> String {
        let n = self.l.index;
        let ep = if self.tx { &self.l.tx } else { &self.l.rx };
        let mut s = self.header(&ep.module);
        s.push_str(&format!(
            "## Link {n}: {} -> {} via {} ({:?}, {:?}); generated by `neuron cell build`.\n",
            self.l.from, self.l.to, self.l.medium, self.l.pattern, self.l.crossing
        ));
        match self.dialect {
            "plc" => self.plc(&mut s, ep),
            "hdl" => self.hdl(&mut s),
            "mcu" => self.tasks(&mut s, ep, true),
            _ => self.tasks(&mut s, ep, false),
        }
        s
    }

    fn plc(&self, s: &mut String, ep: &Endpoint) {
        let n = self.l.index;
        s.push_str(&format!(
            "scan {}\nmemory:\n",
            neuron_duration(self.period_ns)
        ));
        let value_ty = self.ty();
        for name in &ep.names {
            let field = name.rsplit('_').next().unwrap_or("");
            let at = |which: &str, words: u64| -> String {
                match self.reg(which) {
                    Some(a) if words >= 2 => format!(" @ %MD{a}"),
                    Some(a) => format!(" @ %MW{a}"),
                    None => String::new(),
                }
            };
            let line = match field {
                "value" => {
                    let words = self.l.value_bytes.div_ceil(2);
                    match value_ty.as_str() {
                        "bool" | "u8" | "i8" => format!("  {name}: u16{}", at("value", 1)),
                        t if words <= 2 => format!("  {name}: {t}{}", at("value", words)),
                        t => format!("  {name}: {t}"),
                    }
                }
                "seq" | "ack" | "busy" | "done" => format!("  {name}: u16{}", at(field, 1)),
                _ => format!("  {name}: bool"),
            };
            s.push_str(&line);
            s.push('\n');
        }
        match (self.l.crossing, self.tx) {
            (Crossing::LatchAck, true) => {
                s.push_str(&format!(
                    "rung l{n}_latch:\n  l{n}_hold = l{n}_seq != l{n}_ack\n"
                ));
            }
            (Crossing::LatchAck, false) => {
                s.push_str(&format!("rung l{n}_ack_echo:\n  l{n}_ack = l{n}_seq\n"));
            }
            (Crossing::Action, true) => {
                s.push_str(&format!(
                    "let l{n}_timer = ton(preset: {})\nrung l{n}_watch:\n  l{n}_timer.input = l{n}_busy != 0 and l{n}_done == 0\nrung l{n}_expired:\n  l{n}_timeout = l{n}_timer.q\n",
                    neuron_duration(self.l.action_timeout_ns.unwrap_or(ACTION_TIMEOUT_NS))
                ));
            }
            _ => {}
        }
    }

    fn tasks(&self, s: &mut String, ep: &Endpoint, prio: bool) {
        let n = self.l.index;
        for name in &ep.names {
            let field = name.rsplit('_').next().unwrap_or("");
            let ty = match field {
                "value" => self.ty(),
                "seq" | "ack" => "u16".to_string(),
                _ => "bool".to_string(),
            };
            if self.dialect == "robot" {
                // Robot programs have globals, not signals.
                let init = if ty == "bool" { "false" } else { "0" };
                s.push_str(&format!("var {name}: {ty} = {init}\n"));
            } else {
                s.push_str(&format!("signal {name}: {ty}\n"));
            }
        }
        let attr = if prio { " (prio 1)" } else { "" };
        let every = neuron_duration(self.period_ns);
        let body = match (self.l.crossing, self.tx) {
            (Crossing::LatchAck, true) => Some(format!("  l{n}_hold = l{n}_seq != l{n}_ack\n")),
            (Crossing::LatchAck, false) => Some(format!("  l{n}_ack = l{n}_seq\n")),
            _ => None,
        };
        if let Some(b) = body {
            s.push_str(&format!("task l{n}_handshake every {every}{attr}:\n{b}"));
        }
    }

    fn hdl(&self, s: &mut String) {
        let n = self.l.index;
        let ty = self.ty();
        s.push_str("clock sys: 100 MHz @ pin(W5)\n");
        let latch = self.l.crossing == Crossing::LatchAck;
        if self.tx {
            let mut ports = vec![format!("l{n}_in: in {ty}")];
            if latch {
                ports.push(format!("l{n}_strobe: in bit"));
                ports.push(format!("l{n}_ack: in bit"));
            }
            ports.push(format!("l{n}_value: out {ty}"));
            if latch {
                ports.push(format!("l{n}_hold: out bit"));
            }
            s.push_str(&format!(
                "circuit l{n}_tx({}) on sys:\n  reg held: {ty} = 0\n",
                ports.join(", ")
            ));
            if latch {
                s.push_str("  reg pending: bit = 0\n");
            }
            s.push_str(&format!("  on rising(sys):\n    held = l{n}_in\n"));
            if latch {
                s.push_str(&format!(
                    "    pending = (pending or l{n}_strobe) and not l{n}_ack\n"
                ));
            }
            s.push_str(&format!("  comb:\n    l{n}_value = held\n"));
            if latch {
                s.push_str(&format!("    l{n}_hold = pending\n"));
            }
        } else {
            let mut ports = vec![format!("l{n}_in: in {ty}")];
            if latch {
                ports.push(format!("l{n}_req: in bit"));
                ports.push(format!("l{n}_ack: out bit"));
            }
            ports.push(format!("l{n}_value: out {ty}"));
            s.push_str(&format!(
                "circuit l{n}_rx({}) on sys:\n  reg seen: {ty} = 0\n",
                ports.join(", ")
            ));
            if latch {
                s.push_str("  reg echo: bit = 0\n");
            }
            s.push_str(&format!("  on rising(sys):\n    seen = l{n}_in\n"));
            if latch {
                s.push_str(&format!("    echo = l{n}_req\n"));
            }
            s.push_str(&format!("  comb:\n    l{n}_value = seen\n"));
            if latch {
                s.push_str(&format!("    l{n}_ack = echo\n"));
            }
        }
    }
}

/// The endpoint modules of every device of the cell, in link order, tx before rx.
///
/// `nir` is the analyzed package: the period of each device's endpoint modules comes
/// from the modules placed on it.
pub fn endpoint_sources(
    nir: &Nir,
    plan: &CellPlan,
    lib: &Library,
    plans: &[LinkPlan],
) -> BTreeMap<String, Vec<EndpointSource>> {
    let mut out: BTreeMap<String, Vec<EndpointSource>> = BTreeMap::new();
    for l in plans {
        for tx in [true, false] {
            let ep = if tx { &l.tx } else { &l.rx };
            let Some(man) = plan
                .device(&ep.device)
                .and_then(|d| lib.device(&d.manifest))
            else {
                continue;
            };
            let side = Side {
                l,
                tx,
                dialect: dialect_of(man),
                target: man.target.as_deref(),
                period_ns: endpoint_period(nir, plan, &ep.device, man),
            };
            out.entry(ep.device.clone())
                .or_default()
                .push(EndpointSource {
                    device: ep.device.clone(),
                    module: ep.module.clone(),
                    file: format!("{}.neuron", ep.module),
                    text: side.render(),
                });
        }
    }
    out
}
