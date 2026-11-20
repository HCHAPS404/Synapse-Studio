//! The virtual controller: runs a parsed program in simulated time.

use crate::{Ex, Func, Ins, Op, PRef, PointDef, Program};
use std::collections::BTreeMap;

/// Default time of one motion, like the reference interpreter's `move_ns`.
pub const DEFAULT_MOVE_NS: i64 = 1_000_000_000;
/// Error raised by `NeuronCheck` for a target outside the [`Limits`].
pub const WORKSPACE_ERROR: i64 = 8100;

/// Settings of a run.
#[derive(Debug, Clone)]
pub struct Options {
    /// Time of every motion, in nanoseconds.
    pub move_ns: i64,
    /// Polling period of a wait on a condition, in nanoseconds.
    pub tick_ns: i64,
    /// The run stops (as [`Outcome::TimedOut`]) when simulated time passes this instant.
    pub end_ns: i64,
    /// Maximum number of statements, against endless loops.
    pub max_steps: u64,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            move_ns: DEFAULT_MOVE_NS,
            tick_ns: 1_000_000,
            end_ns: 3_600_000_000_000,
            max_steps: 5_000_000,
        }
    }
}

/// Where a motion goes, as the program defined it.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Joint angles in degrees.
    Joints(Vec<f64>),
    /// A Cartesian point.
    Point {
        /// Number of the local frame (`/L1`), if any.
        local: Option<u32>,
        /// `x, y, z` in mm, in that frame.
        xyz: [f64; 3],
        /// `u, v, w` in degrees (rotation about z, y, x).
        uvw: [f64; 3],
    },
}

/// Kind of a motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionKind {
    /// `Go`: point to point.
    Ptp,
    /// `Move`: straight line.
    Linear,
    /// `Arc`: through a middle point.
    Arc,
}

/// One motion.
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
    /// Instant the motion started, in nanoseconds.
    pub t_ns: i64,
    /// Kind.
    pub kind: MotionKind,
    /// End point.
    pub target: Target,
    /// Middle point of an arc.
    pub via: Option<Target>,
    /// Last `Speed` (for `Go`) or `SpeedS` (for `Move` and `Arc`), 0 when never set.
    pub speed: f64,
    /// `CP On` was active.
    pub cp: bool,
}

/// One change of an output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutChange {
    /// Instant, in nanoseconds.
    pub t_ns: i64,
    /// Output bit.
    pub bit: i64,
    /// New value.
    pub on: bool,
}

/// One change of an output word register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordChange {
    /// Instant, in nanoseconds.
    pub t_ns: i64,
    /// Word register.
    pub word: i64,
    /// New value.
    pub value: i64,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// `main` returned.
    Finished,
    /// `Quit` or `Halt` (after an error handler, this is the usual end of a fault).
    Quit,
    /// An error without a handler (`Error n`).
    Faulted(i64),
    /// Simulated time reached [`Options::end_ns`].
    TimedOut,
    /// The program is wrong (unknown function, step limit, ...).
    Failed(String),
}

/// Workspace and joint limits for `NeuronCheck` (the robot manifest of the emitter).
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    /// Maximum distance from the shoulder point, in mm.
    pub reach_mm: f64,
    /// Height of the shoulder point, in mm.
    pub shoulder_mm: f64,
    /// Lowest allowed z in the base frame, in mm.
    pub z_min_mm: f64,
    /// Highest allowed z in the base frame, in mm.
    pub z_max_mm: f64,
    /// `[min, max]` of each joint, in degrees.
    pub joints: Vec<[f64; 2]>,
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    origin: [f64; 3],
    /// `u, v, w` = rotation about z, y, x, in degrees.
    uvw: [f64; 3],
}

impl Frame {
    fn world(&self, p: [f64; 3]) -> [f64; 3] {
        let (z, y, x) = (
            self.uvw[0].to_radians(),
            self.uvw[1].to_radians(),
            self.uvw[2].to_radians(),
        );
        let (y1, z1) = (
            p[1] * x.cos() - p[2] * x.sin(),
            p[1] * x.sin() + p[2] * x.cos(),
        );
        let (x2, z2) = (
            p[0] * y.cos() + z1 * y.sin(),
            -p[0] * y.sin() + z1 * y.cos(),
        );
        let (x3, y3) = (x2 * z.cos() - y1 * z.sin(), x2 * z.sin() + y1 * z.cos());
        [
            self.origin[0] + x3,
            self.origin[1] + y3,
            self.origin[2] + z2,
        ]
    }
}

struct Call {
    func: usize,
    pc: usize,
    onerr: Option<String>,
}

enum Flow {
    Next,
    Stop(Outcome),
}

/// The virtual controller.
pub struct Controller {
    prog: Program,
    opts: Options,
    inputs: Vec<(i64, i64, bool)>,
    word_inputs: Vec<(i64, i64, i64)>,
    word_out: BTreeMap<i64, i64>,
    word_changes: Vec<WordChange>,
    limits: Option<Limits>,
    now: i64,
    vars: BTreeMap<String, f64>,
    arrays: BTreeMap<String, BTreeMap<i64, f64>>,
    points: BTreeMap<u32, Target>,
    locals: BTreeMap<u32, Frame>,
    out: BTreeMap<i64, bool>,
    tw: bool,
    speed: f64,
    speed_s: f64,
    cp: bool,
    motions: Vec<Motion>,
    changes: Vec<OutChange>,
    fault: Option<i64>,
    tool: Option<u32>,
    tools: BTreeMap<u32, [f64; 3]>,
    weight: f64,
    labels: BTreeMap<u32, String>,
}

impl Controller {
    /// A controller for `prog`.
    pub fn new(prog: Program, opts: Options) -> Controller {
        Controller {
            prog,
            opts,
            inputs: Vec::new(),
            word_inputs: Vec::new(),
            word_out: BTreeMap::new(),
            word_changes: Vec::new(),
            limits: None,
            now: 0,
            vars: BTreeMap::new(),
            arrays: BTreeMap::new(),
            points: BTreeMap::new(),
            locals: BTreeMap::new(),
            out: BTreeMap::new(),
            tw: false,
            speed: 0.0,
            speed_s: 0.0,
            cp: false,
            motions: Vec::new(),
            changes: Vec::new(),
            fault: None,
            tool: None,
            tools: BTreeMap::new(),
            weight: 0.0,
            labels: BTreeMap::new(),
        }
    }

    /// Drives input `bit` to `on` from instant `t_ns` on.
    pub fn set_input(&mut self, t_ns: i64, bit: i64, on: bool) {
        self.inputs.push((t_ns, bit, on));
    }

    /// Drives input word register `word` to `value` from instant `t_ns` on (0 before).
    pub fn set_input_word(&mut self, t_ns: i64, word: i64, value: i64) {
        self.word_inputs.push((t_ns, word, value));
    }

    /// Output word changes in order. A word starts at 0; writing its current value is not a change.
    pub fn word_outputs(&self) -> &[WordChange] {
        &self.word_changes
    }

    /// Sets the limits that `Call NeuronCheck(n)` enforces.
    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = Some(limits);
    }

    /// Motions in the order they started.
    pub fn motions(&self) -> &[Motion] {
        &self.motions
    }

    /// Output changes in order. An output starts off; writing its current value is not a change.
    pub fn outputs(&self) -> &[OutChange] {
        &self.changes
    }

    /// The error number of the last `Error n`, if any.
    pub fn fault(&self) -> Option<i64> {
        self.fault
    }

    /// Simulated time now.
    pub fn now_ns(&self) -> i64 {
        self.now
    }

    /// The point `n` as the program defined it.
    pub fn point(&self, n: u32) -> Option<&Target> {
        self.points.get(&n)
    }

    /// The tool selected by `Tool k` and the last `Weight`.
    pub fn tool(&self) -> (Option<u32>, f64) {
        (self.tool, self.weight)
    }

    /// The tool centre point offset of tool `k` set by `TLSet`, in mm.
    pub fn tool_offset(&self, k: u32) -> Option<[f64; 3]> {
        self.tools.get(&k).copied()
    }

    /// The label of point `n` set by `PLabel`.
    pub fn label(&self, n: u32) -> Option<&str> {
        self.labels.get(&n).map(String::as_str)
    }

    fn input_at(&self, bit: i64) -> bool {
        let mut v = false;
        let mut best = i64::MIN;
        for (t, b, on) in &self.inputs {
            if *b == bit && *t <= self.now && *t >= best {
                best = *t;
                v = *on;
            }
        }
        v
    }

    fn word_input_at(&self, word: i64) -> i64 {
        let mut v = 0;
        let mut best = i64::MIN;
        for (t, w, x) in &self.word_inputs {
            if *w == word && *t <= self.now && *t >= best {
                best = *t;
                v = *x;
            }
        }
        v
    }

    fn eval(&self, e: &Ex) -> Result<f64, String> {
        let b = |x: bool| if x { 1.0 } else { 0.0 };
        Ok(match e {
            Ex::Num(n) => *n,
            Ex::Var(v) => match v.as_str() {
                "on" | "true" => 1.0,
                "off" | "false" => 0.0,
                "tw" => b(self.tw),
                other => self.vars.get(other).copied().unwrap_or(0.0),
            },
            Ex::Neg(a) => -self.eval(a)?,
            Ex::Not(a) => b(self.eval(a)? == 0.0),
            Ex::Bin(op, l, r) => {
                let (x, y) = (self.eval(l)?, self.eval(r)?);
                match op {
                    Op::Add => x + y,
                    Op::Sub => x - y,
                    Op::Mul => x * y,
                    Op::Div if y != 0.0 => x / y,
                    Op::Mod if y != 0.0 => x % y,
                    Op::Div | Op::Mod => return Err("division by zero".into()),
                    Op::Eq => b(x == y),
                    Op::Ne => b(x != y),
                    Op::Lt => b(x < y),
                    Op::Gt => b(x > y),
                    Op::Le => b(x <= y),
                    Op::Ge => b(x >= y),
                    Op::And => b(x != 0.0 && y != 0.0),
                    Op::Or => b(x != 0.0 || y != 0.0),
                }
            }
            Ex::Call(name, args) => {
                let a = args
                    .iter()
                    .map(|a| self.eval(a))
                    .collect::<Result<Vec<_>, _>>()?;
                let a0 = a.first().copied().unwrap_or(0.0);
                if self.prog.arrays.iter().any(|n| n == name) {
                    return Ok(self
                        .arrays
                        .get(name)
                        .and_then(|t| t.get(&(a0 as i64)))
                        .copied()
                        .unwrap_or(0.0));
                }
                match name.as_str() {
                    "sw" => b(self.input_at(a0 as i64)),
                    "inw" => self.word_input_at(a0 as i64) as f64,
                    "out" => b(self.out.get(&(a0 as i64)).copied().unwrap_or(false)),
                    "sqr" => a0.sqrt(),
                    "abs" => a0.abs(),
                    other => return Err(format!("unknown function `{other}`")),
                }
            }
        })
    }

    fn nums(&self, v: &[Ex]) -> Result<Vec<f64>, String> {
        v.iter().map(|e| self.eval(e)).collect()
    }

    fn six(&self, v: &[Ex]) -> Result<([f64; 3], [f64; 3]), String> {
        let a = self.nums(v)?;
        let g = |i: usize| a.get(i).copied().unwrap_or(0.0);
        Ok(([g(0), g(1), g(2)], [g(3), g(4), g(5)]))
    }

    fn resolve(&self, r: &PRef) -> Result<Target, String> {
        let t = self
            .points
            .get(&r.n)
            .cloned()
            .ok_or_else(|| format!("point P{} is not defined", r.n))?;
        let Some(dz) = &r.dz else { return Ok(t) };
        let d = self.eval(dz)?;
        match t {
            Target::Point {
                local,
                xyz: [x, y, z],
                uvw,
            } => Ok(Target::Point {
                local,
                xyz: [x, y, z + d],
                uvw,
            }),
            Target::Joints(_) => Err("`+Z` on a joint point".into()),
        }
    }

    fn check_point(&self, n: u32) -> bool {
        let (Some(l), Some(t)) = (&self.limits, self.points.get(&n)) else {
            return true;
        };
        match t {
            Target::Joints(v) => v
                .iter()
                .zip(&l.joints)
                .all(|(x, lim)| *x >= lim[0] && *x <= lim[1]),
            Target::Point { local, xyz, .. } => {
                let w = match local.and_then(|k| self.locals.get(&k)) {
                    Some(f) => f.world(*xyz),
                    None => *xyz,
                };
                let dz = w[2] - l.shoulder_mm;
                let d = (w[0] * w[0] + w[1] * w[1] + dz * dz).sqrt();
                d <= l.reach_mm && w[2] >= l.z_min_mm && w[2] <= l.z_max_mm
            }
        }
    }

    fn write_out(&mut self, bit: i64, on: bool) {
        let old = self.out.insert(bit, on).unwrap_or(false);
        if old != on {
            self.changes.push(OutChange {
                t_ns: self.now,
                bit,
                on,
            });
        }
    }

    fn secs(&self, e: &Ex) -> Result<i64, String> {
        Ok((self.eval(e)? * 1e9).round() as i64)
    }

    fn advance(&mut self, ns: i64) -> Option<Outcome> {
        self.now = self.now.saturating_add(ns);
        (self.now > self.opts.end_ns).then_some(Outcome::TimedOut)
    }

    /// Runs `main` to its end, a `Quit`, an error or the end of simulated time.
    pub fn run(&mut self) -> Outcome {
        for g in self.prog.globals.clone() {
            self.vars.entry(g).or_insert(0.0);
        }
        let Some(main) = self.prog.funcs.iter().position(|f| f.name == "main") else {
            return Outcome::Failed("no `Function main`".into());
        };
        let mut stack = vec![Call {
            func: main,
            pc: 0,
            onerr: None,
        }];
        let mut steps = 0u64;
        loop {
            steps += 1;
            if steps > self.opts.max_steps {
                return Outcome::Failed("too many statements".into());
            }
            let Some(top) = stack.last() else {
                return Outcome::Finished;
            };
            let (fi, pc) = (top.func, top.pc);
            let Some(ins) = self.prog.funcs[fi].code.get(pc).cloned() else {
                stack.pop();
                continue;
            };
            if let Some(t) = stack.last_mut() {
                t.pc += 1;
            }
            match self.exec(&ins, &mut stack) {
                Ok(Flow::Next) => {}
                Ok(Flow::Stop(o)) => return o,
                Err(m) => return Outcome::Failed(format!("{}: {m}", self.prog.funcs[fi].name)),
            }
        }
    }

    /// Raises error `code`: jumps to the innermost handler, or stops with a fault.
    fn raise(&mut self, code: i64, stack: &mut Vec<Call>) -> Result<Flow, String> {
        self.fault = Some(code);
        while let Some(top) = stack.last() {
            if let Some(label) = top.onerr.clone() {
                let f: &Func = &self.prog.funcs[top.func];
                let at = f
                    .labels
                    .iter()
                    .find(|(l, _)| *l == label)
                    .map(|(_, i)| *i)
                    .ok_or_else(|| format!("unknown label `{label}`"))?;
                if let Some(t) = stack.last_mut() {
                    t.pc = at;
                    t.onerr = None;
                }
                return Ok(Flow::Next);
            }
            stack.pop();
        }
        Ok(Flow::Stop(Outcome::Faulted(code)))
    }

    fn motion(&mut self, kind: MotionKind, target: Target, via: Option<Target>) -> Option<Outcome> {
        self.motions.push(Motion {
            t_ns: self.now,
            kind,
            target,
            via,
            speed: if kind == MotionKind::Ptp {
                self.speed
            } else {
                self.speed_s
            },
            cp: self.cp,
        });
        self.advance(self.opts.move_ns)
    }

    fn exec(&mut self, ins: &Ins, stack: &mut Vec<Call>) -> Result<Flow, String> {
        match ins {
            Ins::Set(v, e) => {
                let x = self.eval(e)?;
                self.vars.insert(v.clone(), x);
            }
            Ins::SetIdx(name, i, e) => {
                let (i, x) = (self.eval(i)? as i64, self.eval(e)?);
                self.arrays.entry(name.clone()).or_default().insert(i, x);
            }
            Ins::SetPoint(n, def) => {
                let t = match def {
                    PointDef::Ja(a) => Target::Joints(self.nums(a)?),
                    PointDef::Xy(a, local) => {
                        let (xyz, uvw) = self.six(a)?;
                        Target::Point {
                            local: *local,
                            xyz,
                            uvw,
                        }
                    }
                    PointDef::Copy(r) => self.resolve(r)?,
                };
                self.points.insert(*n, t);
            }
            Ins::Jmp(t) => {
                if let Some(top) = stack.last_mut() {
                    top.pc = *t;
                }
            }
            Ins::JmpIfNot(c, t) => {
                if self.eval(c)? == 0.0 {
                    if let Some(top) = stack.last_mut() {
                        top.pc = *t;
                    }
                }
            }
            Ins::ForTest {
                var,
                to,
                step,
                exit,
            } => {
                let v = self.vars.get(var).copied().unwrap_or(0.0);
                let (to, step) = (self.eval(to)?, self.eval(step)?);
                let go = if step >= 0.0 { v <= to } else { v >= to };
                if !go {
                    if let Some(top) = stack.last_mut() {
                        top.pc = *exit;
                    }
                }
            }
            Ins::GoTo(l) => {
                let Some(top) = stack.last_mut() else {
                    return Ok(Flow::Next);
                };
                let at = self.prog.funcs[top.func]
                    .labels
                    .iter()
                    .find(|(n, _)| n == l)
                    .map(|(_, i)| *i)
                    .ok_or_else(|| format!("unknown label `{l}`"))?;
                top.pc = at;
            }
            Ins::Call(name, args) => {
                if name == "neuroncheck" {
                    let n = args.first().map_or(Ok(0.0), |a| self.eval(a))? as u32;
                    if !self.check_point(n) {
                        return self.raise(WORKSPACE_ERROR, stack);
                    }
                } else if name == "neuronsafe" || self.prog.funcs.iter().any(|f| &f.name == name) {
                    let func = self
                        .prog
                        .funcs
                        .iter()
                        .position(|f| &f.name == name)
                        .ok_or_else(|| format!("unknown function `{name}`"))?;
                    if stack.len() > 64 {
                        return Err("call stack too deep".into());
                    }
                    stack.push(Call {
                        func,
                        pc: 0,
                        onerr: None,
                    });
                } else {
                    return Err(format!("unknown function `{name}`"));
                }
            }
            Ins::Exit => {
                stack.pop();
            }
            Ins::Speed(e) => self.speed = self.eval(e)?,
            Ins::SpeedS(e) => self.speed_s = self.eval(e)?,
            Ins::Cp(on) => self.cp = *on,
            Ins::Go(r) | Ins::Move(r) => {
                let t = self.resolve(r)?;
                let kind = if matches!(ins, Ins::Go(_)) {
                    MotionKind::Ptp
                } else {
                    MotionKind::Linear
                };
                if let Some(o) = self.motion(kind, t, None) {
                    return Ok(Flow::Stop(o));
                }
            }
            Ins::Arc(m, e) => {
                let (via, t) = (self.resolve(m)?, self.resolve(e)?);
                if let Some(o) = self.motion(MotionKind::Arc, t, Some(via)) {
                    return Ok(Flow::Stop(o));
                }
            }
            Ins::On(e) => {
                let b = self.eval(e)? as i64;
                self.write_out(b, true);
            }
            Ins::Off(e) => {
                let b = self.eval(e)? as i64;
                self.write_out(b, false);
            }
            Ins::OutW(w, v) => {
                let (word, value) = (self.eval(w)? as i64, self.eval(v)? as i64);
                let old = self.word_out.insert(word, value).unwrap_or(0);
                if old != value {
                    self.word_changes.push(WordChange {
                        t_ns: self.now,
                        word,
                        value,
                    });
                }
            }
            Ins::Wait(e) => {
                let ns = self.secs(e)?;
                if let Some(o) = self.advance(ns.max(0)) {
                    return Ok(Flow::Stop(o));
                }
            }
            Ins::WaitCond(c, timeout) => {
                let deadline = match timeout {
                    Some(t) => Some(self.now.saturating_add(self.secs(t)?)),
                    None => None,
                };
                self.tw = false;
                loop {
                    if self.eval(c)? != 0.0 {
                        break;
                    }
                    if deadline.is_some_and(|d| self.now >= d) {
                        self.tw = true;
                        break;
                    }
                    if let Some(o) = self.advance(self.opts.tick_ns) {
                        return Ok(Flow::Stop(o));
                    }
                }
            }
            Ins::Error(e) => {
                let code = self.eval(e)? as i64;
                return self.raise(code, stack);
            }
            Ins::OnErr(l) => {
                if let Some(top) = stack.last_mut() {
                    top.onerr = Some(l.clone());
                }
            }
            Ins::Quit => return Ok(Flow::Stop(Outcome::Quit)),
            Ins::Local(k, a) => {
                let (origin, uvw) = self.six(a)?;
                self.locals.insert(*k, Frame { origin, uvw });
            }
            Ins::TlSet(k, a) => {
                let (xyz, _) = self.six(a)?;
                self.tools.insert(*k, xyz);
            }
            Ins::Tool(k) => self.tool = Some(*k),
            Ins::Weight(e) => self.weight = self.eval(e)?,
            Ins::PLabel(n, s) => {
                self.labels.insert(*n, s.clone());
            }
        }
        Ok(Flow::Next)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::parse;
    use proptest::prelude::*;

    fn run(src: &str) -> Controller {
        let mut c = Controller::new(parse(src).expect("parse"), Options::default());
        c.run();
        c
    }

    #[test]
    fn a_wait_with_a_timeout_sets_tw() {
        let c = run("Function main\n  Wait Sw(3) = On, 0.5\n  If TW Then GoTo L\n  On 1\nL:\n  On 2\nFend\n");
        let bits: Vec<i64> = c.outputs().iter().map(|o| o.bit).collect();
        assert_eq!(bits, vec![2]);
        assert_eq!(c.outputs()[0].t_ns, 500_000_000);
    }

    #[test]
    fn an_input_that_rises_ends_the_wait_on_the_grid() {
        let p = parse("Function main\n  Wait Sw(3) = On, 0.5\n  If TW Then GoTo L\n  On 1\nL:\n  On 2\nFend\n").expect("parse");
        let mut c = Controller::new(p, Options::default());
        c.set_input(120_000_000, 3, true);
        c.run();
        assert_eq!(c.outputs()[0].bit, 1);
        assert_eq!(c.outputs()[0].t_ns, 120_000_000);
    }

    #[test]
    fn errors_run_the_handler_and_quit() {
        let c = run("Function main\n  Call f\nFend\nFunction f\n  OnErr GoTo H\n  Error 8001\n  On 1\n  Exit Function\nH:\n  On 7\n  Quit All\nFend\n");
        assert_eq!(c.fault(), Some(8001));
        assert_eq!(c.outputs().len(), 1);
        assert_eq!(c.outputs()[0].bit, 7);
    }

    #[test]
    fn if_chains_and_loops_run() {
        let c = run("Function main\n  Integer i\n  For i = 0 To 3\n    If i = 0 Then\n      On 10\n    ElseIf i = 1 Then\n      On 11\n    Else\n      Off 10\n    EndIf\n  Next i\nFend\n");
        let bits: Vec<(i64, bool)> = c.outputs().iter().map(|o| (o.bit, o.on)).collect();
        assert_eq!(bits, vec![(10, true), (11, true), (10, false)]);
    }

    #[test]
    fn points_frames_and_offsets_resolve() {
        let c = run("Function main\n  Local 1, XY(10, 0, 0, 90, 0, 0)\n  P1 = XY(1, 2, 3, 0, 0, 0) /L1\n  P2 = JA(0, 1, 2, 3, 4, 5)\n  Move P1 +Z(5)\n  Arc P1, P1\n  Go P2\nFend\n");
        assert_eq!(c.motions().len(), 3);
        assert_eq!(
            c.motions()[0].target,
            Target::Point {
                local: Some(1),
                xyz: [1.0, 2.0, 8.0],
                uvw: [0.0; 3]
            }
        );
        assert_eq!(c.motions()[2].t_ns, 2_000_000_000);
    }

    #[test]
    fn neuron_check_raises_the_workspace_error() {
        let p = parse("Function main\n  P1 = XY(900, 0, 0, 0, 0, 0)\n  OnErr GoTo H\n  Call NeuronCheck(1)\n  Go P1\n  Exit Function\nH:\n  On 5\nFend\n").expect("parse");
        let mut c = Controller::new(p, Options::default());
        c.set_limits(Limits {
            reach_mm: 700.0,
            shoulder_mm: 300.0,
            z_min_mm: -100.0,
            z_max_mm: 900.0,
            joints: vec![],
        });
        c.run();
        assert_eq!(c.fault(), Some(WORKSPACE_ERROR));
        assert!(c.motions().is_empty());
    }

    #[test]
    fn bad_lines_are_reported_with_their_number() {
        let e = parse("Function main\n  Go\nFend\n").expect_err("error");
        assert_eq!(e.line, 2);
    }

    proptest! {
        /// Integer arithmetic of the program equals Rust's.
        #[test]
        fn arithmetic_matches_rust(a in -1000i32..1000, b in -1000i32..1000, c in 1i32..100) {
            let src = format!(
                "Function main\n  Real x\n  x = ({a}) * ({b}) + ({a}) - ({b}) / ({c})\n  P1 = XY(x, 0, 0, 0, 0, 0)\n  Go P1\nFend\n"
            );
            let k = run(&src);
            let want = f64::from(a) * f64::from(b) + f64::from(a) - f64::from(b) / f64::from(c);
            match &k.motions()[0].target {
                Target::Point { xyz, .. } => prop_assert!((xyz[0] - want).abs() < 1e-9),
                other => prop_assert!(false, "{:?}", other),
            }
        }

        /// A wait takes exactly its duration.
        #[test]
        fn waits_take_their_duration(ms in 0i64..5000) {
            let k = run(&format!("Function main\n  Wait {}\n  On 1\nFend\n", ms as f64 / 1000.0));
            prop_assert_eq!(k.outputs()[0].t_ns, ms * 1_000_000);
        }
    }
}
