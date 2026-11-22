//! SPEL+-style emitter for `robot` modules (task C3).
//!
//! Input: a `robot` module (frames, tools, points, `io` bindings, sequences, a `safe_state`).
//! Output per module: `<module>.prg` with `Function main` (frames, tools, declared points,
//! then one `Call` per sequence), one function per sequence and `NeuronSafe`; plus
//! `NeuronLib.prg` (the workspace check) and `NeuronSlave.prg` (a TCP command interpreter).
//!
//! Every target is checked against the robot manifest at emission (`NE0804`). A target whose
//! components are constants, or depend only on loops with constant bounds, is checked for
//! every iteration. Any other target gets `Call NeuronCheck(n)` before the motion.
//!
//! Dialect details that need the real controller are marked "to confirm in session I6".
//! The reference for the behaviour is the interpreter (`neuron-interp`), see `tests/c3_spel.rs`.

mod text;

use crate::fidelity::{Class, Fidelity};
use crate::nirx;
use crate::package::{Package, Writer};
use crate::robot::Robot;
use crate::{Diag, EmitOptions, Emitter, Manifest};
use neuron_nir::{
    Arg, Expr, ExprKind, Item, ItemDef, Lit, Nir, PatternKind, SafetyDef, SeqItem, Stmt, StmtKind,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// First error number of the faults of a program: `Error 8001` is the first fault.
pub const FAULT_BASE: i64 = 8000;
/// Error raised by `NeuronCheck` when a computed target is outside the workspace.
pub const WORKSPACE_ERROR: i64 = 8100;

const CONSTRUCTS: &[&str] = &[
    "point",
    "frame",
    "tool",
    "io",
    "move ptp/linear/arc",
    "blend",
    "above",
    "wait",
    "until",
    "for",
    "if",
    "let",
    "fault",
    "safety",
];

/// The SPEL+-style emitter (`neuron emit spel`).
pub struct Spel;
/// One target of a `move` of a robot module, as the emitter's workspace check sees it.
///
/// `world_mm` is the position of the flange target in the base frame of the robot (the
/// frame of the target, the lift of `above` and the origin and rotation of the frame
/// applied), or `None` when the target is not a constant Cartesian point (a joint target,
/// or a computed value such as a parameter): the emitter checks those at run time with
/// `NeuronCheck`.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionTarget {
    /// Module name.
    pub module: String,
    /// Byte range of the target expression.
    pub span: [usize; 2],
    /// `ptp`, `linear` or `arc`.
    pub kind: String,
    /// Constant position in the base frame, in mm.
    pub world_mm: Option<[f64; 3]>,
}
/// Every target of every `move` of robot module number `module` of `nir`, in source
/// order of the sequences, resolved like the emitter does for `NE0804`.
///
/// An index that is not a module gives an empty list. The cell compiler tests this
/// function through the zones of a cell (`neuron-cell`, `d4_zones`).
pub fn motion_targets(nir: &Nir, module: usize, robot: &Robot) -> Vec<MotionTarget> {
    if module >= nir.modules.len() {
        return Vec::new();
    }
    let mut l = Lower::new(nir, module, robot);
    l.declare();
    l.collect = Some(Vec::new());
    for item in &nir.modules[module].items {
        if let ItemDef::Sequence { items, .. } = &item.def {
            let mut env = Env::default();
            l.check_items(items, &mut env);
        }
    }
    l.collect.take().unwrap_or_default()
}

// ---- numbers and places ------------------------------------------------------------

/// A numeric expression: its value when it is constant, and its text in the program.
#[derive(Debug, Clone)]
struct Num {
    c: Option<f64>,
    t: String,
}

impl Num {
    fn k(v: f64) -> Num {
        let v = (v * 1e9).round() / 1e9;
        Num {
            c: Some(v),
            t: nirx::fmt_num(v),
        }
    }

    fn var(t: &str) -> Num {
        Num {
            c: None,
            t: t.to_string(),
        }
    }

    fn par(&self) -> String {
        if self.t.starts_with('-') {
            format!("({})", self.t)
        } else {
            self.t.clone()
        }
    }

    /// The text without a pair of parentheses that wraps all of it.
    fn bare(&self) -> String {
        let t = &self.t;
        if t.starts_with('(') && t.ends_with(')') {
            let mut depth = 0i32;
            for (i, ch) in t.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 && i + 1 < t.len() {
                            return t.clone();
                        }
                    }
                    _ => {}
                }
            }
            return t[1..t.len() - 1].to_string();
        }
        t.clone()
    }

    fn bin(op: &str, a: &Num, b: &Num) -> Option<Num> {
        let sop = match op {
            "+" | "-" | "*" | "/" => op,
            "%" => "Mod",
            _ => return None,
        };
        if let (Some(x), Some(y)) = (a.c, b.c) {
            let r = match op {
                "+" => x + y,
                "-" => x - y,
                "*" => x * y,
                "/" if y != 0.0 => x / y,
                "%" if y != 0.0 => x % y,
                _ => return None,
            };
            return Some(Num::k(r));
        }
        Some(Num {
            c: None,
            t: format!("({} {sop} {})", a.par(), b.par()),
        })
    }

    fn scale(&self, f: f64) -> Num {
        if f == 1.0 {
            return self.clone();
        }
        Num::bin("*", self, &Num::k(f)).unwrap_or_else(|| self.clone())
    }
}

/// Factor from the unit written after a geometry value to millimetres, degrees or kilograms.
fn unit_factor(unit: &str) -> Option<f64> {
    Some(match unit {
        "mm" | "deg" | "kg" | "mm/s" | "%" => 1.0,
        "cm" => 10.0,
        "m" => 1000.0,
        "um" => 0.001,
        "rad" => 180.0 / std::f64::consts::PI,
        "g" => 0.001,
        _ => return None,
    })
}

#[derive(Debug, Clone)]
enum Place {
    /// Joint angles in degrees.
    J(Vec<Num>),
    /// Coordinates in mm in frame `frame` (index in declaration order).
    C { frame: usize, v: [Num; 3] },
}

/// A resolved target: where it is, the point that holds it and a lift along the frame Z.
#[derive(Debug, Clone)]
struct Target {
    g: Place,
    pt: Option<u32>,
    dz: Option<Num>,
}

#[derive(Debug, Clone)]
struct Frame {
    id: String,
    origin: [f64; 3],
    rot: [f64; 3],
}

impl Frame {
    /// Position in the base frame of a point given in this frame (roll, pitch, yaw: `Rz Ry Rx`).
    fn world(&self, p: [f64; 3]) -> [f64; 3] {
        let (r, q, y) = (
            self.rot[0].to_radians(),
            self.rot[1].to_radians(),
            self.rot[2].to_radians(),
        );
        let (x0, y0, z0) = (p[0], p[1], p[2]);
        let (y1, z1) = (y0 * r.cos() - z0 * r.sin(), y0 * r.sin() + z0 * r.cos());
        let (x2, z2) = (x0 * q.cos() + z1 * q.sin(), -x0 * q.sin() + z1 * q.cos());
        let (x3, y3) = (x2 * y.cos() - y1 * y.sin(), x2 * y.sin() + y1 * y.cos());
        [
            self.origin[0] + x3,
            self.origin[1] + y3,
            self.origin[2] + z2,
        ]
    }
}

#[derive(Debug, Clone)]
struct Tool {
    offset: [f64; 3],
    payload: f64,
}

#[derive(Debug, Clone, Default)]
struct Env {
    num: HashMap<String, Num>,
    geo: HashMap<String, Target>,
}

enum Check {
    Ok,
    Computed,
    Bad(String),
}

/// Characters of a name that are valid in a program identifier.
fn ident(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !s.starts_with(|c: char| c.is_ascii_alphabetic()) {
        s.insert(0, 'v');
    }
    s
}

fn secs(ns: i64) -> String {
    nirx::fmt_num(ns as f64 / 1e9)
}

// ---- the lowering of one module -----------------------------------------------------

struct Lower<'a> {
    nir: &'a Nir,
    m: usize,
    file: String,
    name: String,
    robot: &'a Robot,
    defs: HashMap<String, &'a ItemDef>,
    fid: Fidelity,
    diags: Vec<Diag>,
    frames: Vec<Frame>,
    frame_names: Vec<String>,
    tools: Vec<Tool>,
    tool_names: Vec<String>,
    points: HashMap<String, Target>,
    decl_points: Vec<(u32, String, Place, [usize; 2])>,
    io: HashMap<String, (bool, i64)>,
    io_names: Vec<(String, bool, i64)>,
    faults: Vec<String>,
    next_pt: u32,
    vars: BTreeMap<String, &'static str>,
    status: BTreeMap<usize, bool>,
    seen: BTreeSet<(usize, usize)>,
    budget: u32,
    sequences: Vec<String>,
    globals: Vec<(String, Option<String>)>,
    /// Targets seen by the check pass, when a caller asked for them.
    collect: Option<Vec<MotionTarget>>,
}

impl<'a> Lower<'a> {
    /// Position of a constant Cartesian target in the base frame.
    fn world_of(&self, t: &Target) -> Option<[f64; 3]> {
        let Place::C { frame, v } = &t.g else {
            return None;
        };
        let fr = self.frames.get(*frame)?;
        let dz = match &t.dz {
            None => 0.0,
            Some(d) => d.c?,
        };
        Some(fr.world([v[0].c?, v[1].c?, v[2].c? + dz]))
    }
    fn new(nir: &'a Nir, m: usize, robot: &'a Robot) -> Lower<'a> {
        let module = &nir.modules[m];
        let mut defs = HashMap::new();
        for it in &module.items {
            defs.insert(it.id.clone(), &it.def);
        }
        Lower {
            nir,
            m,
            file: module.file.clone(),
            name: ident(&module.name),
            robot,
            defs,
            fid: Fidelity::new("emit-spel", &format!("robot.{}", robot.name)),
            diags: Vec::new(),
            frames: Vec::new(),
            frame_names: Vec::new(),
            tools: Vec::new(),
            tool_names: Vec::new(),
            points: HashMap::new(),
            decl_points: Vec::new(),
            io: HashMap::new(),
            io_names: Vec::new(),
            faults: Vec::new(),
            next_pt: 1,
            vars: BTreeMap::new(),
            status: BTreeMap::new(),
            seen: BTreeSet::new(),
            budget: 200_000,
            sequences: Vec::new(),
            globals: Vec::new(),
            collect: None,
        }
    }

    fn unsupported(&mut self, span: [usize; 2], what: &str) {
        self.diags.push(Diag {
            code: "NE0610",
            message: format!("{what} is not translated by the spel emitter"),
            help: Some(
                "write it with the constructs listed in `neuron emit spel --help`, or translate that part by hand"
                    .into(),
            ),
            file: self.file.clone(),
            span,
        });
    }

    fn alloc(&mut self) -> u32 {
        let n = self.next_pt;
        self.next_pt += 1;
        n
    }

    // ---- numbers and targets ---------------------------------------------------------

    fn num(&self, e: &Expr, env: &Env) -> Result<Num, String> {
        match &e.e {
            ExprKind::Lit {
                lit: Lit::Int { v, .. } | Lit::Real { v, .. },
            } => Ok(Num::k(nirx::parse_num(v))),
            ExprKind::Quantity { value, unit, .. } => {
                let n = self.num(value, env)?;
                let f =
                    unit_factor(&unit.text).ok_or_else(|| format!("the unit `{}`", unit.text))?;
                Ok(n.scale(f))
            }
            ExprKind::Ref { id, name } => match self.defs.get(id.as_str()).copied() {
                Some(ItemDef::Const { value, .. }) => self.num(value, &Env::default()),
                Some(ItemDef::Var { var }) => Ok(Num::var(&ident(&var.name))),
                _ => Err(format!("`{name}`")),
            },
            ExprKind::Local { id, name } => env
                .num
                .get(id)
                .cloned()
                .ok_or_else(|| format!("the local `{name}`")),
            ExprKind::Binary { op, lhs, rhs } => {
                let (a, b) = (self.num(lhs, env)?, self.num(rhs, env)?);
                Num::bin(op, &a, &b).ok_or_else(|| format!("the operator `{op}`"))
            }
            ExprKind::Unary { op, operand } if op == "-" => {
                let a = self.num(operand, env)?;
                Ok(match a.c {
                    Some(v) => Num::k(-v),
                    None => Num {
                        c: None,
                        t: format!("-{}", a.par()),
                    },
                })
            }
            _ => Err("this expression".to_string()),
        }
    }

    fn frame_index(&self, e: &Expr) -> Result<usize, String> {
        if let ExprKind::Ref { id, name } = &e.e {
            if let Some(i) = self.frames.iter().position(|f| &f.id == id) {
                return Ok(i);
            }
            return Err(format!("`{name}` is not a frame"));
        }
        Err("a frame expression".to_string())
    }

    fn geo(&self, e: &Expr, env: &Env) -> Result<Target, String> {
        match &e.e {
            ExprKind::Quantity { value, unit, .. } => {
                let f =
                    unit_factor(&unit.text).ok_or_else(|| format!("the unit `{}`", unit.text))?;
                self.geo_call(value, env, f)
            }
            ExprKind::Call { .. } => self.geo_call(e, env, 1.0),
            ExprKind::Ref { id, name } => self
                .points
                .get(id)
                .cloned()
                .ok_or_else(|| format!("`{name}` is not a point")),
            ExprKind::Local { id, name } => env
                .geo
                .get(id)
                .cloned()
                .ok_or_else(|| format!("`{name}` is not a point")),
            _ => Err("a target expression".to_string()),
        }
    }

    fn geo_call(&self, e: &Expr, env: &Env, f: f64) -> Result<Target, String> {
        let ExprKind::Call { callee, args, .. } = &e.e else {
            return Err("a target expression".to_string());
        };
        if let ExprKind::Field { base, name, .. } = &callee.e {
            match name.as_str() {
                "xyz" => {
                    let frame = self.frame_index(base)?;
                    let mut v = [Num::k(0.0), Num::k(0.0), Num::k(0.0)];
                    for (slot, a) in v.iter_mut().zip(args) {
                        *slot = self.num(&a.value, env)?.scale(f);
                    }
                    return Ok(Target {
                        g: Place::C { frame, v },
                        pt: None,
                        dz: None,
                    });
                }
                "above" => {
                    let b = self.geo(base, env)?;
                    let d = args
                        .first()
                        .ok_or_else(|| "`above` without a distance".to_string())
                        .and_then(|a| self.num(&a.value, env))?;
                    if matches!(b.g, Place::J(_)) {
                        return Err("`above` on a joint target".to_string());
                    }
                    let dz = match &b.dz {
                        Some(x) => Num::bin("+", x, &d).ok_or("`above`")?,
                        None => d,
                    };
                    return Ok(Target {
                        g: b.g,
                        pt: b.pt,
                        dz: Some(dz),
                    });
                }
                _ => {}
            }
        }
        if nirx::builtin(e) == Some("joints") {
            let v = args
                .iter()
                .map(|a| self.num(&a.value, env).map(|n| n.scale(f)))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Target {
                g: Place::J(v),
                pt: None,
                dz: None,
            });
        }
        Err("this call as a target".to_string())
    }

    fn vec3(&self, e: &Expr) -> Result<[f64; 3], String> {
        let (call, f) = match &e.e {
            ExprKind::Quantity { value, unit, .. } => (
                &**value,
                unit_factor(&unit.text).ok_or_else(|| format!("the unit `{}`", unit.text))?,
            ),
            _ => (e, 1.0),
        };
        let ExprKind::Call { args, .. } = &call.e else {
            return Err("a vector".to_string());
        };
        let mut v = [0.0; 3];
        for (slot, a) in v.iter_mut().zip(args) {
            let n = self.num(&a.value, &Env::default())?;
            *slot = n.c.ok_or_else(|| "a constant".to_string())? * f;
        }
        Ok(v)
    }

    // ---- the limits of the robot ------------------------------------------------------

    fn check(&self, t: &Target) -> Check {
        let r = self.robot;
        match &t.g {
            Place::J(v) => {
                if v.len() != r.joint_limits_deg.len() {
                    return Check::Bad(format!(
                        "it has {} joint values but `{}` has {} joints",
                        v.len(),
                        r.name,
                        r.joint_limits_deg.len()
                    ));
                }
                let mut computed = false;
                for (i, (n, l)) in v.iter().zip(&r.joint_limits_deg).enumerate() {
                    match n.c {
                        None => computed = true,
                        Some(x) if x < l[0] || x > l[1] => {
                            return Check::Bad(format!(
                                "J{} = {} deg is outside {} to {} deg",
                                i + 1,
                                nirx::fmt_num(x),
                                nirx::fmt_num(l[0]),
                                nirx::fmt_num(l[1])
                            ))
                        }
                        Some(_) => {}
                    }
                }
                if computed {
                    Check::Computed
                } else {
                    Check::Ok
                }
            }
            Place::C { frame, v } => {
                let Some(fr) = self.frames.get(*frame) else {
                    return Check::Computed;
                };
                let dz = match &t.dz {
                    None => Some(0.0),
                    Some(d) => d.c,
                };
                let (Some(x), Some(y), Some(z0), Some(dz)) = (v[0].c, v[1].c, v[2].c, dz) else {
                    return Check::Computed;
                };
                let w = fr.world([x, y, z0 + dz]);
                let dzs = w[2] - r.shoulder_height_mm;
                let d = (w[0] * w[0] + w[1] * w[1] + dzs * dzs).sqrt();
                if d > r.reach_mm {
                    return Check::Bad(format!(
                        "it is {} mm from the shoulder of `{}` (reach {} mm)",
                        nirx::fmt_num((d * 10.0).round() / 10.0),
                        r.name,
                        nirx::fmt_num(r.reach_mm)
                    ));
                }
                if w[2] < r.z_min_mm || w[2] > r.z_max_mm {
                    return Check::Bad(format!(
                        "z = {} mm is outside {} to {} mm",
                        nirx::fmt_num((w[2] * 10.0).round() / 10.0),
                        nirx::fmt_num(r.z_min_mm),
                        nirx::fmt_num(r.z_max_mm)
                    ));
                }
                Check::Ok
            }
        }
    }

    fn bad_target(&mut self, span: [usize; 2], why: &str) {
        if !self.seen.insert((span[0], span[1])) {
            return;
        }
        self.diags.push(Diag {
            code: "NE0804",
            message: format!(
                "target outside the workspace of `{}`: {why}",
                self.robot.name
            ),
            help: Some(
                "move the point, change the frame, or use another robot manifest (`--robot`)"
                    .into(),
            ),
            file: self.file.clone(),
            span,
        });
    }

    // ---- declarations -----------------------------------------------------------------

    fn declare(&mut self) {
        let nir = self.nir;
        for item in &nir.modules[self.m].items {
            match &item.def {
                ItemDef::Geometry {
                    kind, name, value, ..
                } => self.declare_geometry(item, kind, name, value),
                ItemDef::Io { name, at, .. } => {
                    let bit = match &at.e {
                        ExprKind::Call { args, .. } => {
                            args.first().and_then(|a| nirx::int(&a.value))
                        }
                        _ => None,
                    };
                    match (nirx::builtin(at), bit) {
                        (Some(k @ ("dout" | "din")), Some(b)) => {
                            let out = k == "dout";
                            self.io.insert(item.id.clone(), (out, b));
                            self.io_names.push((name.clone(), out, b));
                        }
                        _ => self.unsupported(item.span, &format!("the location of `io {name}`")),
                    }
                }
                ItemDef::Var { var } => {
                    let init = var
                        .init
                        .as_ref()
                        .and_then(|e| self.num(e, &Env::default()).ok())
                        .map(|n| n.bare());
                    self.globals.push((ident(&var.name), init));
                }
                ItemDef::Sequence { name, .. } => self.sequences.push(name.clone()),
                ItemDef::Fn { .. }
                | ItemDef::Task { .. }
                | ItemDef::Rule { .. }
                | ItemDef::Fsm { .. }
                | ItemDef::Interrupt { .. }
                | ItemDef::Signal { .. } => {
                    self.unsupported(item.span, "this item of a `robot` module")
                }
                _ => {}
            }
        }
    }

    fn declare_geometry(&mut self, item: &Item, kind: &str, name: &str, value: &Expr) {
        match kind {
            "frame" => {
                let ExprKind::Call { args, .. } = &value.e else {
                    return self.unsupported(item.span, &format!("the frame `{name}`"));
                };
                let origin = nirx::arg(args, "origin", 0).map(|e| self.vec3(e));
                let rot = nirx::arg(args, "rot", 1).map(|e| self.vec3(e));
                match (origin, rot) {
                    (Some(Ok(o)), r) => {
                        let rot = match r {
                            Some(Ok(r)) => r,
                            _ => [0.0; 3],
                        };
                        self.frames.push(Frame {
                            id: item.id.clone(),
                            origin: o,
                            rot,
                        });
                        self.frame_names.push(name.to_string());
                    }
                    _ => self.unsupported(item.span, &format!("the frame `{name}` (not constant)")),
                }
            }
            "tool" => {
                let ExprKind::Call { args, .. } = &value.e else {
                    return self.unsupported(item.span, &format!("the tool `{name}`"));
                };
                let offset = nirx::arg(args, "offset", 0).map(|e| self.vec3(e));
                let payload = nirx::arg(args, "payload", 1)
                    .and_then(|e| self.num(e, &Env::default()).ok())
                    .and_then(|n| n.c)
                    .unwrap_or(0.0);
                match offset {
                    Some(Ok(o)) => {
                        self.tools.push(Tool { offset: o, payload });
                        self.tool_names.push(name.to_string());
                    }
                    _ => self.unsupported(item.span, &format!("the tool `{name}` (not constant)")),
                }
            }
            "point" => match self.geo(value, &Env::default()) {
                Ok(t) => {
                    match self.check(&t) {
                        Check::Bad(why) => self.bad_target(value.span, &format!("`{name}` {why}")),
                        Check::Ok | Check::Computed => {}
                    }
                    let n = self.alloc();
                    self.decl_points
                        .push((n, name.to_string(), t.g.clone(), item.span));
                    self.points.insert(
                        item.id.clone(),
                        Target {
                            g: t.g,
                            pt: Some(n),
                            dz: None,
                        },
                    );
                }
                Err(why) => self.unsupported(item.span, &format!("the point `{name}` ({why})")),
            },
            _ => {}
        }
    }

    // ---- statements -------------------------------------------------------------------

    fn fault_code(&mut self, name: &str) -> i64 {
        let i = match self.faults.iter().position(|f| f == name) {
            Some(i) => i,
            None => {
                self.faults.push(name.to_string());
                self.faults.len() - 1
            }
        };
        FAULT_BASE + 1 + i as i64
    }

    fn io_of(&self, e: &Expr) -> Option<(bool, i64)> {
        match &e.e {
            ExprKind::Ref { id, .. } => self.io.get(id).copied(),
            _ => None,
        }
    }

    fn cond(&self, e: &Expr, env: &Env) -> Result<String, String> {
        match &e.e {
            ExprKind::Lit {
                lit: Lit::Bool { v },
            } => Ok(if *v { "True" } else { "False" }.to_string()),
            ExprKind::Ref { name, .. } => match self.io_of(e) {
                Some((true, b)) => Ok(format!("Out({b}) = On")),
                Some((false, b)) => Ok(format!("Sw({b}) = On")),
                None => Err(format!("the condition `{name}`")),
            },
            ExprKind::Unary { op, operand } if op == "not" || op == "!" => {
                Ok(format!("Not ({})", self.cond(operand, env)?))
            }
            ExprKind::Binary { op, lhs, rhs }
                if matches!(op.as_str(), "and" | "or" | "&&" | "||") =>
            {
                let word = if matches!(op.as_str(), "and" | "&&") {
                    "And"
                } else {
                    "Or"
                };
                Ok(format!(
                    "({}) {word} ({})",
                    self.cond(lhs, env)?,
                    self.cond(rhs, env)?
                ))
            }
            ExprKind::Compare { first, rest } if rest.len() == 1 => {
                let op = match rest[0].op.as_str() {
                    "==" => "=",
                    "!=" => "<>",
                    o @ ("<" | ">" | "<=" | ">=") => o,
                    o => return Err(format!("the comparison `{o}`")),
                };
                let (a, b) = (self.num(first, env)?, self.num(&rest[0].rhs, env)?);
                Ok(format!("{} {op} {}", a.bare(), b.bare()))
            }
            _ => Err("this condition".to_string()),
        }
    }

    fn assign_text(n: u32, g: &Place) -> String {
        match g {
            Place::J(v) => format!(
                "P{n} = JA({})",
                v.iter().map(Num::bare).collect::<Vec<_>>().join(", ")
            ),
            Place::C { frame, v } => format!(
                "P{n} = XY({}, {}, {}, 0, 0, 0) /L{}",
                v[0].bare(),
                v[1].bare(),
                v[2].bare(),
                frame + 1
            ),
        }
    }

    /// The point that holds `t`, assigning a scratch point when needed.
    fn hold(&mut self, t: &Target, w: &mut Writer) -> u32 {
        match t.pt {
            Some(n) => n,
            None => {
                let n = self.alloc();
                w.line(&Self::assign_text(n, &t.g));
                n
            }
        }
    }

    /// The operand of a motion: `P3`, or `P3 +Z(50)` (a scratch point when `plain`).
    fn operand(&mut self, t: &Target, w: &mut Writer, plain: bool) -> (String, Option<u32>) {
        let n = self.hold(t, w);
        match &t.dz {
            None => (format!("P{n}"), Some(n)),
            Some(d) if plain => {
                let m = self.alloc();
                w.line(&format!("P{m} = P{n} +Z({})", d.bare()));
                (format!("P{m}"), Some(m))
            }
            Some(d) => (format!("P{n} +Z({})", d.bare()), None),
        }
    }

    fn option<'b>(opts: &'b [neuron_nir::NamedExpr], name: &str) -> Option<&'b Expr> {
        opts.iter().find(|o| o.name == name).map(|o| &o.value)
    }

    fn stmts(&mut self, b: &'a [Stmt], env: &mut Env, w: &mut Writer, view: &str) {
        for s in b {
            self.stmt(s, env, w, view);
        }
    }

    fn stmt(&mut self, s: &'a Stmt, env: &mut Env, w: &mut Writer, view: &str) {
        let mark = w.mark();
        self.stmt_inner(s, env, w, view);
        w.map(mark, &self.file.clone(), s.span, Some(view.to_string()));
    }

    fn stmt_inner(&mut self, s: &'a Stmt, env: &mut Env, w: &mut Writer, view: &str) {
        match &s.s {
            StmtKind::Pass => {}
            StmtKind::Let { pattern, value, .. } => {
                let PatternKind::Bind { name, id, .. } = &pattern.p else {
                    return self.unsupported(s.span, "a `let` pattern");
                };
                if let Ok(t) = self.geo(value, env) {
                    let n = self.hold(&t, w);
                    env.geo.insert(
                        id.clone(),
                        Target {
                            g: t.g,
                            pt: Some(n),
                            dz: t.dz,
                        },
                    );
                    self.fid.exact();
                    return;
                }
                match self.num(value, env) {
                    Ok(n) => {
                        let v = ident(name);
                        self.vars.insert(v.clone(), "Real");
                        w.line(&format!("{v} = {}", n.bare()));
                        env.num.insert(id.clone(), Num::var(&v));
                        self.fid.exact();
                    }
                    Err(why) => self.unsupported(value.span, &format!("`let {name}` ({why})")),
                }
            }
            StmtKind::Assign {
                place,
                op,
                value,
                when: None,
            } => {
                let ExprKind::Local { id, .. } = &place.e else {
                    return self
                        .unsupported(s.span, "an assignment to something other than a local");
                };
                let Some(target) = env.num.get(id).cloned() else {
                    return self.unsupported(s.span, "an assignment to an unknown local");
                };
                let rhs = match self.num(value, env) {
                    Ok(n) => n,
                    Err(why) => {
                        return self.unsupported(value.span, &format!("this value ({why})"))
                    }
                };
                let new = match op.as_str() {
                    "=" => rhs,
                    "+=" | "-=" => {
                        let o = &op[..1];
                        match Num::bin(o, &target, &rhs) {
                            Some(n) => n,
                            None => return self.unsupported(s.span, "this assignment"),
                        }
                    }
                    _ => return self.unsupported(s.span, &format!("the assignment `{op}`")),
                };
                w.line(&format!("{} = {}", target.t, new.bare()));
                self.fid.exact();
            }
            StmtKind::Expr { expr } => self.io_write(expr, s.span, env, w),
            StmtKind::Move {
                kind,
                to,
                via,
                options,
            } => self.motion(s, kind, to, via.as_ref(), options, env, w),
            StmtKind::Wait { duration } => match nirx::dur_ns(duration) {
                Some(ns) => {
                    w.line(&format!("Wait {}", secs(ns)));
                    self.fid.exact();
                }
                None => self.unsupported(s.span, "a `wait` with a computed duration"),
            },
            StmtKind::Until {
                cond,
                timeout,
                on_timeout,
            } => {
                let c = match self.cond(cond, env) {
                    Ok(c) => c,
                    Err(why) => return self.unsupported(cond.span, &format!("`until` on {why}")),
                };
                match timeout {
                    Some(t) => {
                        let Some(ns) = nirx::dur_ns(t) else {
                            return self.unsupported(t.span, "a computed timeout");
                        };
                        w.line(&format!("Wait {c}, {}", secs(ns)));
                        if let Some(label) = on_timeout {
                            w.line(&format!("If TW Then GoTo {}", ident(label)));
                        }
                    }
                    None => w.line(&format!("Wait {c}")),
                }
                self.fid.exact();
            }
            StmtKind::Jump { target } => {
                w.line(&format!("GoTo {}", ident(target)));
                self.fid.exact();
            }
            StmtKind::Fault { name, .. } => {
                let code = self.fault_code(name);
                w.line(&format!("Error {code}  ' fault {name}"));
                self.fid.exact();
            }
            StmtKind::Return { value: None } => {
                w.line("Exit Function");
                self.fid.exact();
            }
            StmtKind::If {
                branches,
                otherwise,
            } => {
                for (i, br) in branches.iter().enumerate() {
                    let c = match self.cond(&br.cond, env) {
                        Ok(c) => c,
                        Err(why) => {
                            return self.unsupported(br.cond.span, &format!("`if` on {why}"))
                        }
                    };
                    w.line(&format!(
                        "{} {c} Then",
                        if i == 0 { "If" } else { "ElseIf" }
                    ));
                    w.indent();
                    let mut inner = env.clone();
                    self.stmts(&br.body.stmts, &mut inner, w, view);
                    w.dedent();
                }
                if let Some(b) = otherwise {
                    w.line("Else");
                    w.indent();
                    let mut inner = env.clone();
                    self.stmts(&b.stmts, &mut inner, w, view);
                    w.dedent();
                }
                w.line("EndIf");
                self.fid.exact();
            }
            StmtKind::For {
                pattern,
                iter,
                body,
                ..
            } => {
                let PatternKind::Bind { name, id, .. } = &pattern.p else {
                    return self.unsupported(s.span, "a `for` pattern");
                };
                let ExprKind::Range { lo, hi, inclusive } = &iter.e else {
                    return self
                        .unsupported(iter.span, "a `for` over something other than a range");
                };
                let (a, b) = match (self.num(lo, env), self.num(hi, env)) {
                    (Ok(a), Ok(b)) => (a, b),
                    _ => {
                        return self
                            .unsupported(iter.span, "a `for` with a bound that is not a number")
                    }
                };
                let last = if *inclusive {
                    b
                } else {
                    match Num::bin("-", &b, &Num::k(1.0)) {
                        Some(n) => n,
                        None => return self.unsupported(iter.span, "this range"),
                    }
                };
                let v = ident(name);
                self.vars.insert(v.clone(), "Integer");
                w.line(&format!("For {v} = {} To {}", a.bare(), last.bare()));
                w.indent();
                let mut inner = env.clone();
                inner.num.insert(id.clone(), Num::var(&v));
                self.stmts(&body.stmts, &mut inner, w, view);
                w.dedent();
                w.line(&format!("Next {v}"));
                self.fid.exact();
            }
            _ => self.unsupported(s.span, "this statement of a robot program"),
        }
    }

    fn io_write(&mut self, e: &Expr, span: [usize; 2], env: &Env, w: &mut Writer) {
        let ExprKind::Call { callee, args, .. } = &e.e else {
            return self.unsupported(span, "this expression statement");
        };
        let ExprKind::Field { base, name, .. } = &callee.e else {
            return self.unsupported(span, "this call");
        };
        let Some((true, bit)) = self.io_of(base) else {
            return self.unsupported(span, "a call on something other than an output");
        };
        if name != "write" {
            return self.unsupported(span, &format!("`{name}` on an output"));
        }
        let Some(Arg { value, .. }) = args.first() else {
            return self.unsupported(span, "`write` without a value");
        };
        match nirx::boolean(value) {
            Some(true) => w.line(&format!("On {bit}")),
            Some(false) => w.line(&format!("Off {bit}")),
            None => match self.cond(value, env) {
                Ok(c) => {
                    w.line(&format!("If {c} Then"));
                    w.line(&format!("  On {bit}"));
                    w.line("Else");
                    w.line(&format!("  Off {bit}"));
                    w.line("EndIf");
                }
                Err(why) => return self.unsupported(value.span, &format!("`write` of {why}")),
            },
        }
        self.fid.exact();
    }

    #[allow(clippy::too_many_arguments)]
    fn motion(
        &mut self,
        s: &Stmt,
        kind: &str,
        to: &Expr,
        via: Option<&Expr>,
        options: &[neuron_nir::NamedExpr],
        env: &Env,
        w: &mut Writer,
    ) {
        let t = match self.geo(to, env) {
            Ok(t) => t,
            Err(why) => return self.unsupported(to.span, &format!("this target ({why})")),
        };
        let v = match via.map(|e| self.geo(e, env)) {
            None => None,
            Some(Ok(v)) => Some(v),
            Some(Err(why)) => return self.unsupported(s.span, &format!("this `via` ({why})")),
        };
        let computed = self.status.get(&s.span[0]).copied().unwrap_or(false);
        let speed = Self::option(options, "speed").and_then(|e| {
            let inner = match &e.e {
                ExprKind::Quantity { value, .. } => value,
                _ => e,
            };
            self.num(inner, env).ok()
        });
        let blend = Self::option(options, "blend").is_some();
        let (op, check1) = match kind {
            "arc" => {
                let Some(v) = &v else {
                    return self.unsupported(s.span, "an `arc` without `via`");
                };
                let (mid, m) = self.operand(v, w, true);
                let (end, e) = self.operand(&t, w, true);
                (format!("Arc {mid}, {end}"), vec![m, e])
            }
            "ptp" | "linear" => {
                let (o, n) = self.operand(&t, w, computed);
                (
                    format!("{} {o}", if kind == "ptp" { "Go" } else { "Move" }),
                    vec![n],
                )
            }
            other => return self.unsupported(s.span, &format!("`move {other}`")),
        };
        if computed {
            for n in check1.into_iter().flatten() {
                w.line(&format!("Call NeuronCheck({n})"));
            }
        }
        if let Some(sp) = speed {
            w.line(&format!(
                "{} {}",
                if kind == "ptp" { "Speed" } else { "SpeedS" },
                sp.bare()
            ));
        }
        if blend {
            w.line("CP On");
        }
        w.line(&op);
        if blend {
            w.line("CP Off");
            let f = self.file.clone();
            self.fid.record(
                &f,
                s.span,
                Class::Approximate,
                "blend",
                "`blend r mm` becomes `CP On` for this motion; the controller rounds the corner by its own rule and the radius is not transferred, so the path near the target differs from the reference, which passes through the point (to confirm in session I6)",
            );
        }
        self.fid.exact();
    }

    // ---- the check pass ---------------------------------------------------------------

    fn check_items(&mut self, items: &'a [SeqItem], env: &mut Env) {
        for it in items {
            match it {
                SeqItem::Step { body, .. } => self.check_items(body, env),
                SeqItem::Stmt { stmt } => self.check_stmt(stmt, env),
                SeqItem::Parallel { .. } => {}
            }
        }
    }

    fn check_stmt(&mut self, s: &'a Stmt, env: &mut Env) {
        match &s.s {
            StmtKind::Let { pattern, value, .. } => {
                let PatternKind::Bind { id, name, .. } = &pattern.p else {
                    return;
                };
                if let Ok(t) = self.geo(value, env) {
                    env.geo.insert(id.clone(), t);
                } else if let Ok(n) = self.num(value, env) {
                    env.num.insert(id.clone(), n);
                } else {
                    env.num.insert(id.clone(), Num::var(&ident(name)));
                }
            }
            StmtKind::Assign { place, .. } => {
                if let ExprKind::Local { id, name } = &place.e {
                    env.num.insert(id.clone(), Num::var(&ident(name)));
                }
            }
            StmtKind::If {
                branches,
                otherwise,
            } => {
                for b in branches {
                    let mut inner = env.clone();
                    self.check_block(&b.body.stmts, &mut inner);
                }
                if let Some(b) = otherwise {
                    let mut inner = env.clone();
                    self.check_block(&b.stmts, &mut inner);
                }
            }
            StmtKind::For {
                pattern,
                iter,
                body,
                ..
            } => {
                let PatternKind::Bind { id, name, .. } = &pattern.p else {
                    return;
                };
                let range = match &iter.e {
                    ExprKind::Range { lo, hi, inclusive } => {
                        match (
                            self.num(lo, env).ok().and_then(|n| n.c),
                            self.num(hi, env).ok().and_then(|n| n.c),
                        ) {
                            (Some(a), Some(b)) => Some((a, if *inclusive { b + 1.0 } else { b })),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match range {
                    Some((a, b)) if b - a <= 10_000.0 && self.budget > 0 => {
                        let mut i = a;
                        while i < b {
                            self.budget = self.budget.saturating_sub(1);
                            let mut inner = env.clone();
                            inner.num.insert(id.clone(), Num::k(i));
                            self.check_block(&body.stmts, &mut inner);
                            i += 1.0;
                        }
                    }
                    _ => {
                        let mut inner = env.clone();
                        inner.num.insert(id.clone(), Num::var(&ident(name)));
                        self.check_block(&body.stmts, &mut inner);
                    }
                }
            }
            StmtKind::Move { kind, to, via, .. } => {
                let mut computed = false;
                if self.collect.is_some() {
                    for e in std::iter::once(to).chain(via.iter()) {
                        let world = self.geo(e, env).ok().and_then(|t| self.world_of(&t));
                        let rec = MotionTarget {
                            module: self.nir.modules[self.m].name.clone(),
                            span: e.span,
                            kind: kind.clone(),
                            world_mm: world,
                        };
                        if let Some(c) = self.collect.as_mut() {
                            if !c.contains(&rec) {
                                c.push(rec);
                            }
                        }
                    }
                }
                for e in std::iter::once(to).chain(via.iter()) {
                    // A declared point was checked where it is declared.
                    if matches!(e.e, ExprKind::Ref { .. }) {
                        continue;
                    }
                    match self.geo(e, env) {
                        Ok(t) => match self.check(&t) {
                            Check::Ok => {}
                            Check::Computed => computed = true,
                            Check::Bad(why) => {
                                let text = Self::describe(&t);
                                self.bad_target(e.span, &format!("{text} {why}"));
                            }
                        },
                        Err(_) => computed = true,
                    }
                }
                let slot = self.status.entry(s.span[0]).or_insert(false);
                *slot |= computed;
            }
            _ => {}
        }
    }

    fn check_block(&mut self, stmts: &'a [Stmt], env: &mut Env) {
        for s in stmts {
            self.check_stmt(s, env);
        }
    }

    fn describe(t: &Target) -> String {
        let z = |n: &Num| n.c.map_or_else(|| n.t.clone(), nirx::fmt_num);
        let mut s = match &t.g {
            Place::J(v) => format!(
                "joints({}) deg",
                v.iter().map(z).collect::<Vec<_>>().join(", ")
            ),
            Place::C { frame, v } => format!(
                "xyz({}, {}, {}) mm in frame {}",
                z(&v[0]),
                z(&v[1]),
                z(&v[2]),
                frame + 1
            ),
        };
        if let Some(d) = &t.dz {
            s.push_str(&format!(" lifted by {} mm", z(d)));
        }
        format!("`{s}`")
    }

    // ---- the program ------------------------------------------------------------------

    fn safe_state(&self) -> Option<&'a [Stmt]> {
        let nir = self.nir;
        for item in &nir.modules[self.m].items {
            if let ItemDef::Safety { members } = &item.def {
                for mem in members {
                    if let SafetyDef::SafeState { body } = &mem.def {
                        return Some(&body.stmts);
                    }
                }
            }
        }
        None
    }

    fn function(&mut self, name: &str, items: &'a [SeqItem], span: [usize; 2], out: &mut Writer) {
        self.vars.clear();
        let mut env = Env::default();
        self.check_items(items, &mut env);
        let view = format!("sequence:{name}");
        let mut body = Writer::new("  ");
        body.indent();
        body.line("OnErr GoTo NeuronFault");
        let mut env = Env::default();
        let main: Vec<&SeqItem> = items
            .iter()
            .take_while(|i| !matches!(i, SeqItem::Step { .. }))
            .collect();
        for it in main {
            match it {
                SeqItem::Stmt { stmt } => self.stmt(stmt, &mut env, &mut body, &view),
                SeqItem::Parallel { span, .. } => {
                    self.unsupported(*span, "a `parallel` block (one robot program is one task)")
                }
                SeqItem::Step { .. } => {}
            }
        }
        body.line("Exit Function");
        for it in items {
            match it {
                SeqItem::Step {
                    name: step,
                    body: sb,
                    ..
                } => {
                    let sview = format!("step:{step}");
                    body.raw(&format!("{}:\n", ident(step)));
                    for inner in sb {
                        match inner {
                            SeqItem::Stmt { stmt } => {
                                self.stmt(stmt, &mut env, &mut body, &sview);
                            }
                            SeqItem::Step { span, .. } | SeqItem::Parallel { span, .. } => {
                                self.unsupported(*span, "a nested `step` or `parallel`")
                            }
                        }
                    }
                    body.line("Exit Function");
                }
                SeqItem::Stmt { .. } | SeqItem::Parallel { .. } => {}
            }
        }
        body.raw("NeuronFault:\n");
        body.line("Call NeuronSafe");
        body.line("Quit All");
        let file = self.file.clone();
        let mark = out.mark();
        out.line(&format!("Function {}", ident(name)));
        out.indent();
        let mut ints = Vec::new();
        let mut reals = Vec::new();
        for (v, t) in &self.vars {
            if *t == "Integer" {
                ints.push(v.clone());
            } else {
                reals.push(v.clone());
            }
        }
        if !ints.is_empty() {
            out.line(&format!("Integer {}", ints.join(", ")));
        }
        if !reals.is_empty() {
            out.line(&format!("Real {}", reals.join(", ")));
        }
        out.dedent();
        out.append(body);
        out.line("Fend");
        out.map(mark, &file, span, Some(view));
        out.line("");
    }

    fn program(&mut self) -> Writer {
        self.declare();
        let nir = self.nir;
        let module = &nir.modules[self.m];
        let mut w = Writer::new("  ");
        w.line(&format!(
            "' {}: generated by Neuron {} from {}. Change the .neuron source, not this file.",
            self.name,
            env!("CARGO_PKG_VERSION"),
            self.file
        ));
        w.line(&format!(
            "' Robot manifest: {}. Dialect details marked \"to confirm in session I6\" need the real controller.",
            self.robot.name
        ));
        w.line("");
        // Sequences are lowered first so that the numbers of scratch points and faults are known.
        let mut seq = Writer::new("  ");
        let mut first_seq: Option<[usize; 2]> = None;
        for item in &module.items {
            if let ItemDef::Sequence { name, items, .. } = &item.def {
                first_seq.get_or_insert(item.span);
                self.function(name, items, item.span, &mut seq);
            }
        }
        let mut safe = Writer::new("  ");
        safe.line("Function NeuronSafe");
        safe.indent();
        if let Some(stmts) = self.safe_state() {
            let mut env = Env::default();
            self.stmts(stmts, &mut env, &mut safe, "safety:safe_state");
        }
        safe.dedent();
        safe.line("Fend");
        for item in &module.items {
            if let ItemDef::Safety { members } = &item.def {
                for mem in members {
                    if !matches!(mem.def, SafetyDef::SafeState { .. }) {
                        self.unsupported(
                            mem.span,
                            "this member of `safety` (only `safe_state` is translated)",
                        );
                    }
                }
            }
        }
        // main
        let file = self.file.clone();
        for (g, _) in &self.globals {
            w.line(&format!("Global Real {g}"));
        }
        if !self.globals.is_empty() {
            w.line("");
        }
        w.line("Function main");
        w.indent();
        for (g, init) in &self.globals {
            if let Some(i) = init {
                w.line(&format!("{g} = {i}"));
            }
        }
        w.line("Motor On  ' to confirm in session I6: power-up and homing sequence");
        for (i, f) in self.frames.iter().enumerate() {
            w.line(&format!(
                "Local {}, XY({}, {}, {}, {}, {}, {})  ' frame {}; rpy(roll, pitch, yaw) is written as rz, ry, rx",
                i + 1,
                nirx::fmt_num(f.origin[0]),
                nirx::fmt_num(f.origin[1]),
                nirx::fmt_num(f.origin[2]),
                nirx::fmt_num(f.rot[2]),
                nirx::fmt_num(f.rot[1]),
                nirx::fmt_num(f.rot[0]),
                self.frame_names[i]
            ));
        }
        for (i, t) in self.tools.iter().enumerate() {
            w.line(&format!(
                "TLSet {}, XY({}, {}, {}, 0, 0, 0)  ' tool {}",
                i + 1,
                nirx::fmt_num(t.offset[0]),
                nirx::fmt_num(t.offset[1]),
                nirx::fmt_num(t.offset[2]),
                self.tool_names[i]
            ));
        }
        if let Some(t) = self.tools.first() {
            w.line("Tool 1  ' the first declared tool is the active one");
            w.line(&format!("Weight {}", nirx::fmt_num(t.payload)));
        }
        for (n, name, g, span) in self.decl_points.clone() {
            let mark = w.mark();
            w.line(&Self::assign_text(n, &g));
            w.line(&format!("PLabel {n}, \"{name}\""));
            w.map(mark, &file, span, Some(format!("point:{name}")));
        }
        for s in &self.sequences {
            w.line(&format!("Call {}", ident(s)));
        }
        w.dedent();
        w.line("Fend");
        w.line("");
        w.append(seq);
        w.append(safe);
        if self.sequences.len() > 1 {
            let f = self.file.clone();
            self.fid.record(
                &f,
                first_seq.unwrap_or([0, 0]),
                Class::Approximate,
                "sequences",
                "the sequences of the module run one after the other from `main`; the reference starts them together",
            );
        }
        if self.fid.summary.exact > 0 {
            let f = self.file.clone();
            self.fid.record(
                &f,
                first_seq.unwrap_or([0, 0]),
                Class::Equivalent,
                "motion time",
                "the controller times a motion from its speed and acceleration; the reference interpreter uses a fixed time per motion (1 s by default). The order of motions and output changes is the same; the instants differ",
            );
        }
        w
    }

    fn wiring(&self) -> String {
        let mut s = format!("## Program `{}` (from `{}`)\n\n", self.name, self.file);
        s.push_str(&format!(
            "Robot manifest `{}`: reach {} mm from the shoulder (height {} mm), z from {} to {} mm. Every constant target was checked against it (`NE0804`); a computed target calls `NeuronCheck` (error {WORKSPACE_ERROR}).\n\n",
            self.robot.name,
            nirx::fmt_num(self.robot.reach_mm),
            nirx::fmt_num(self.robot.shoulder_height_mm),
            nirx::fmt_num(self.robot.z_min_mm),
            nirx::fmt_num(self.robot.z_max_mm)
        ));
        if !self.io_names.is_empty() {
            s.push_str("| I/O | Direction | Bit |\n|---|---|---|\n");
            for (n, out, b) in &self.io_names {
                s.push_str(&format!(
                    "| `{n}` | {} | {b} |\n",
                    if *out {
                        "output (`On`/`Off`)"
                    } else {
                        "input (`Sw`)"
                    }
                ));
            }
            s.push('\n');
        }
        if !self.frames.is_empty() {
            s.push_str("| Local | Frame |\n|---|---|\n");
            for (i, n) in self.frame_names.iter().enumerate() {
                s.push_str(&format!("| {} | `{n}` |\n", i + 1));
            }
            s.push('\n');
        }
        if !self.tools.is_empty() {
            s.push_str("| Tool | Name | Payload (kg) |\n|---|---|---|\n");
            for (i, n) in self.tool_names.iter().enumerate() {
                s.push_str(&format!(
                    "| {} | `{n}` | {} |\n",
                    i + 1,
                    nirx::fmt_num(self.tools[i].payload)
                ));
            }
            s.push_str("\nTool 1 is selected at the start. The reference interpreter does not model tools.\n\n");
        }
        if !self.decl_points.is_empty() {
            s.push_str("| Point | Label |\n|---|---|\n");
            for (n, name, _, _) in &self.decl_points {
                s.push_str(&format!("| P{n} | `{name}` |\n"));
            }
            s.push_str(&format!(
                "\nPoints from P{} up are scratch points assigned by the program before each motion.\n\n",
                self.decl_points.len() + 1
            ));
        }
        if !self.faults.is_empty() {
            s.push_str("Fault codes (`Error n`; the error handler runs `NeuronSafe` and stops all tasks):\n\n| Code | Fault |\n|---|---|\n");
            for (i, f) in self.faults.iter().enumerate() {
                s.push_str(&format!("| {} | `{f}` |\n", FAULT_BASE + 1 + i as i64));
            }
            s.push('\n');
        }
        s
    }
}

pub(crate) fn robot_modules(nir: &Nir) -> Vec<usize> {
    nir.modules
        .iter()
        .enumerate()
        .filter(|(_, m)| m.dialect == "robot")
        .map(|(i, _)| i)
        .collect()
}

impl Emitter for Spel {
    fn manifest(&self) -> Manifest {
        Manifest {
            name: "spel",
            language: "SPEL+-style robot program",
            dialects: &["robot"],
            profiles: &["generic_6axis", "generic_scara"],
            constructs: CONSTRUCTS,
        }
    }

    fn emit(&self, nir: &Nir, opts: &EmitOptions) -> Result<Package, Vec<Diag>> {
        let robot = Robot::load(&opts.robot).map_err(|message| {
            vec![Diag {
                code: "NE0610",
                message,
                help: Some("use `--robot generic_6axis`, `generic_scara` or the path of a `.toml` manifest".into()),
                file: String::new(),
                span: [0, 0],
            }]
        })?;
        let mods = robot_modules(nir);
        if mods.is_empty() {
            return Err(vec![Diag {
                code: "NE0610",
                message: "the program has no `robot` module to translate".into(),
                help: Some(
                    "the spel emitter reads modules declared `module name for robot`".into(),
                ),
                file: nir
                    .modules
                    .first()
                    .map(|m| m.file.clone())
                    .unwrap_or_default(),
                span: [0, 0],
            }]);
        }
        let mut fid = Fidelity::new("emit-spel", &format!("robot.{}", robot.name));
        let mut pkg = Package::new(fid.clone());
        let mut diags = Vec::new();
        let mut wiring = format!(
            "# Wiring and configuration (SPEL+-style, robot `{}`)\n\n",
            robot.name
        );
        let mut max_points = 0;
        for m in mods {
            let mut l = Lower::new(nir, m, &robot);
            let w = l.program();
            diags.append(&mut l.diags);
            wiring.push_str(&l.wiring());
            max_points = max_points.max(l.next_pt.saturating_sub(1));
            pkg.add(&format!("{}.prg", l.name), w);
            fid.merge(l.fid);
        }
        if !diags.is_empty() {
            return Err(diags);
        }
        pkg.add_text("NeuronLib.prg", text::lib(&robot));
        pkg.add_text("NeuronSlave.prg", text::slave(&robot, max_points));
        wiring.push_str(
            "`NeuronLib.prg` holds `NeuronCheck`, called by the programs. `NeuronSlave.prg` is a TCP command interpreter (`GO n`, `MOVE n`, `OUT b v`, `IN b`, `SPEED v`, `HOME`; it answers `OK`, `ERR <code>` or the value). Both use dialect details to confirm in session I6 (network statements, reading a point). The program is a design aid, not a certified safety function: the hardwired emergency stop remains the authority.\n",
        );
        pkg.wiring = wiring;
        pkg.fidelity = fid;
        Ok(pkg)
    }
}
