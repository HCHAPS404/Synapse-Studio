//! MIR to C99 (task B2).
//!
//! The generated file describes the program with the tables of `nrt.h` and contains no
//! scheduler of its own. It compiles for the target (`-DNRT_PORT_AVR`) and for the host
//! (`-DNRT_HOST`, used by the differential tests) from the same text.

use neuron_mir::*;
use std::collections::{BTreeMap, BTreeSet};

/// The generated C and its source map.
#[derive(Debug, Clone)]
pub struct Emitted {
    /// The C file.
    pub c: String,
    /// JSON: `{"files": [...], "lines": [[c_line, file, line], ...]}`.
    pub map_json: String,
    /// The program has a display: the ui runtime and the bus of the port are needed.
    pub ui: bool,
}

struct Em<'a> {
    p: &'a Program,
    out: Vec<String>,
    map: Vec<(usize, u32, u32)>,
    indent: usize,
    shared: BTreeSet<&'a str>,
    globals: BTreeMap<&'a str, &'a Global>,
    io_names: Vec<String>,
    tmp: usize,
    task: Option<String>,
    in_isr: bool,
    atomic: bool,
    ret: Option<Ty>,
    ui: Option<(&'a str, u32)>,
}

/// Generates the C file for a program.
pub fn emit_c(p: &Program) -> Emitted {
    emit(p, None)
}

/// Generates the C file for a program with a display: `ui_section` is the C of
/// `neuron_ui::emit_ui_c` (it defines `ui_task`), run every `ui_period_ms` at the lowest
/// priority.
pub fn emit_c_ui(p: &Program, ui_section: &str, ui_period_ms: u32) -> Emitted {
    emit(p, Some((ui_section, ui_period_ms)))
}

fn emit<'a>(p: &'a Program, ui: Option<(&'a str, u32)>) -> Emitted {
    let mut io_names: Vec<String> = Vec::new();
    for (i, io) in p.ios.iter().enumerate() {
        let base = format!("IO_{}", io.name);
        io_names.push(if io_names.contains(&base) {
            format!("{base}_{i}")
        } else {
            base
        });
    }
    let mut em = Em {
        p,
        out: Vec::new(),
        map: Vec::new(),
        indent: 0,
        shared: p
            .globals
            .iter()
            .filter(|g| g.shared)
            .map(|g| g.name.as_str())
            .collect(),
        globals: p.globals.iter().map(|g| (g.name.as_str(), g)).collect(),
        io_names,
        tmp: 0,
        task: None,
        in_isr: false,
        atomic: false,
        ret: None,
        ui,
    };
    em.all();
    let mut map = String::from("{\"files\":[");
    for (i, f) in p.files.iter().enumerate() {
        if i > 0 {
            map.push(',');
        }
        map.push_str(&format!("{:?}", f.replace('\\', "/")));
    }
    map.push_str("],\"lines\":[");
    for (i, (c, f, l)) in em.map.iter().enumerate() {
        if i > 0 {
            map.push(',');
        }
        map.push_str(&format!("[{c},{f},{l}]"));
    }
    map.push_str("]}");
    let mut c = em.out.join("\n");
    c.push('\n');
    Emitted {
        c,
        map_json: map,
        ui: ui.is_some(),
    }
}

fn c_string(s: &str) -> String {
    let mut o = String::from("\"");
    for b in s.bytes() {
        match b {
            b'\\' => o.push_str("\\\\"),
            b'"' => o.push_str("\\\""),
            b'\n' => o.push_str("\\n"),
            b'\r' => o.push_str("\\r"),
            b'\t' => o.push_str("\\t"),
            0x20..=0x7e => o.push(b as char),
            other => o.push_str(&format!("\\{other:03o}")),
        }
    }
    o.push('"');
    o
}

fn float_lit(f: f64) -> String {
    let t = format!("{f:?}");
    if t.contains("inf") || t.contains("NaN") {
        return "0.0f".to_string();
    }
    format!("{t}f")
}

fn int_lit(v: i64, t: Ty) -> String {
    let body = if v < 0 {
        format!("({v})")
    } else {
        v.to_string()
    };
    let suffix = if v.unsigned_abs() >= 32768 { "LL" } else { "" };
    format!("(({}){}{})", t.c_type(), body, suffix)
}

fn fit(t: Ty, inner: String) -> String {
    format!("nrt_fit_{}({})", t.tag(), inner)
}

/// `true` when evaluating `e` can raise a fault: checked integer arithmetic, a division, a
/// conversion to an integer, a call (the callee may fault) or a `scale`.
fn may_trap(e: &Expr) -> bool {
    match e {
        Expr::Int(..)
        | Expr::Float(_)
        | Expr::Bool(_)
        | Expr::Local(..)
        | Expr::Global(..)
        | Expr::Signal(..)
        | Expr::IoGet(..) => false,
        Expr::Unary(op, x, t) => (*op == UnOp::Neg && t.is_int()) || may_trap(x),
        Expr::Binary(op, a, b, _) => {
            let int_arith = a.ty().is_int()
                && matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem
                );
            let float_div = a.ty().is_float() && *op == BinOp::Div;
            int_arith || float_div || may_trap(a) || may_trap(b)
        }
        Expr::Cast(x, t) => t.is_int() || may_trap(x),
        Expr::IfElse(c, a, b, _) => may_trap(c) || may_trap(a) || may_trap(b),
        Expr::Call(..) | Expr::Scale(_) => true,
        Expr::Builtin(_, a, t) => t.is_int() || a.iter().any(may_trap),
    }
}

impl<'a> Em<'a> {
    fn ln(&mut self, s: impl AsRef<str>) {
        let pad = "  ".repeat(self.indent);
        self.out.push(format!("{pad}{}", s.as_ref()));
    }

    fn blank(&mut self) {
        self.out.push(String::new());
    }

    fn mark(&mut self, loc: Loc) {
        if loc.line != 0 {
            self.map.push((self.out.len() + 1, loc.file, loc.line));
        }
    }

    fn io_name(&self, i: usize) -> &str {
        &self.io_names[i]
    }

    fn fault_code(&self, name: &str) -> String {
        match name {
            "Overflow" => "NRT_F_OVERFLOW".into(),
            "DivideByZero" => "NRT_F_DIVZERO".into(),
            "ContractViolation" => "NRT_F_CONTRACT".into(),
            "StackOverflow" => "NRT_F_STACK".into(),
            "LoopBound" => "NRT_F_LOOP".into(),
            "DeadlineMiss" => "NRT_F_DEADLINE".into(),
            "Watchdog" => "NRT_F_WATCHDOG".into(),
            other => {
                let k = self.p.faults.iter().position(|f| f == other).unwrap_or(0);
                format!("(NRT_F_USER + {k})")
            }
        }
    }

    // ---- file ----------------------------------------------------------------------------

    fn all(&mut self) {
        let p = self.p;
        self.ln("/* Generated by `neuron emit-c`. Do not edit: change the .neuron source. */");
        self.ln(format!(
            "/* program: {}   board: {} */",
            p.name,
            p.board.as_deref().unwrap_or("(none)")
        ));
        self.ln("#include \"nrt.h\"");
        self.ln("");
        self.ln("#define NRT_UNUSED __attribute__((unused))");
        self.blank();
        self.io_table();
        self.names();
        self.globals();
        self.protos();
        for f in &p.funcs {
            self.func(f);
        }
        for t in &p.tasks {
            self.task(t);
        }
        self.irqs();
        for (k, f) in p.fsms.iter().enumerate() {
            self.fsm(k, f);
        }
        for (k, r) in p.rules.iter().enumerate() {
            self.rule_fn(&format!("rule{k}"), r);
        }
        self.safety();
        if let Some((section, _)) = self.ui {
            for line in section.lines() {
                self.out.push(line.to_string());
            }
        }
        self.tables();
    }

    fn io_table(&mut self) {
        let p = self.p;
        self.ln("enum {");
        for n in self.io_names.clone() {
            self.ln(format!("  {n},"));
        }
        self.ln("  IO_COUNT");
        self.ln("};");
        self.blank();
        self.ln("static const nrt_io_def io_table[IO_COUNT + 1] = {");
        for io in &p.ios {
            let kind = match io.kind {
                IoK::GpioIn => "NRT_IO_GPIO_IN",
                IoK::GpioOut => "NRT_IO_GPIO_OUT",
                IoK::Pwm => "NRT_IO_PWM",
                IoK::Adc => "NRT_IO_ADC",
                IoK::Uart => "NRT_IO_UART",
            };
            let regs = match (io.port, io.bit) {
                (Some(port), Some(bit))
                    if matches!(io.kind, IoK::GpioIn | IoK::GpioOut | IoK::Pwm) =>
                {
                    format!("NRT_PIN(PORT{port}, DDR{port}, PIN{port}, {bit})")
                }
                // A flat GPIO bank: the pin number (RP2040, RP2350, ESP32-S3).
                (None, Some(bit)) if matches!(io.kind, IoK::GpioIn | IoK::GpioOut | IoK::Pwm) => {
                    format!("NRT_GPIO({bit})")
                }
                _ => "NULL, NULL, NULL, 0".to_string(),
            };
            self.ln(format!(
                "  {{ {kind}, {}, {regs}, {}, {} }},",
                c_string(&io.name),
                io.aux,
                io.cfg
            ));
        }
        self.ln("  { 0, \"\", NULL, NULL, NULL, 0, 0, 0 }");
        self.ln("};");
        self.blank();
    }

    fn names(&mut self) {
        let p = self.p;
        let list = |v: &[String]| -> String {
            if v.is_empty() {
                "\"\"".to_string()
            } else {
                v.iter().map(|s| c_string(s)).collect::<Vec<_>>().join(", ")
            }
        };
        self.ln(format!(
            "static const char *const event_names[] = {{ {} }};",
            list(&p.events)
        ));
        self.ln(format!(
            "static const char *const fault_names[] = {{ {} }};",
            list(&p.faults)
        ));
        self.blank();
    }

    fn globals(&mut self) {
        let p = self.p;
        for g in &p.globals {
            let init = match &g.init {
                Some(e) => self.expr(e),
                None => {
                    if g.ty.is_float() {
                        "0.0f".into()
                    } else if g.ty == Ty::Bool {
                        "false".into()
                    } else {
                        "0".into()
                    }
                }
            };
            let vol = if g.shared { "volatile " } else { "" };
            self.ln(format!(
                "static NRT_UNUSED {vol}{} {} = {init};",
                g.ty.c_type(),
                g.name
            ));
            if g.signal {
                self.ln(format!(
                    "static NRT_UNUSED {} nx_{};",
                    g.ty.c_type(),
                    g.name
                ));
                self.ln(format!("static NRT_UNUSED bool dirty_{};", g.name));
                self.ln(format!("static NRT_UNUSED bool seen_{};", g.name));
            }
        }
        // Signals: one setter each (the commit point, traced on the host).
        for g in p.globals.iter().filter(|g| g.signal) {
            self.ln(format!(
                "static NRT_UNUSED void set_{}({} v) {{",
                g.name,
                g.ty.c_type()
            ));
            self.ln(format!(
                "  bool changed = !seen_{n} || {n} != v;",
                n = g.name
            ));
            self.ln(format!("  seen_{} = true;", g.name));
            if g.shared {
                self.ln("  NRT_ATOMIC_BEGIN()");
                self.ln(format!("  {} = v;", g.name));
                self.ln("  NRT_ATOMIC_END()");
            } else {
                self.ln(format!("  {} = v;", g.name));
            }
            match g.ty {
                Ty::Bool => self.ln(format!(
                    "  if (changed) {{ NRT_TRACE_SIG_B({}, v); }}",
                    c_string(&g.source_name)
                )),
                Ty::F32 => self.ln("  (void)changed;"),
                _ => self.ln(format!(
                    "  if (changed) {{ NRT_TRACE_SIG_I({}, v); }}",
                    c_string(&g.source_name)
                )),
            }
            self.ln("}");
        }
        // Snapshots of the signals each task reads.
        for t in &p.tasks {
            for s in &t.reads {
                let ty = self.globals.get(s.as_str()).map(|g| g.ty);
                if let Some(ty) = ty {
                    self.ln(format!(
                        "static NRT_UNUSED {} sn_{}_{};",
                        ty.c_type(),
                        t.name,
                        s
                    ));
                }
            }
        }
        self.blank();
    }

    fn protos(&mut self) {
        let p = self.p;
        for f in &p.funcs {
            let sig = self.sig(f);
            self.ln(format!("static NRT_UNUSED {sig};"));
        }
        self.blank();
    }

    fn sig(&self, f: &Func) -> String {
        let params = if f.params.is_empty() {
            "void".to_string()
        } else {
            f.params
                .iter()
                .map(|(n, t)| format!("{} {n}", t.c_type()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!("{} {}({params})", f.ret.map_or("void", Ty::c_type), f.name)
    }

    // ---- activities -------------------------------------------------------------------------

    fn func(&mut self, f: &Func) {
        let sig = self.sig(f);
        self.ret = f.ret;
        self.ln(format!("static NRT_UNUSED {sig} {{"));
        self.indent += 1;
        self.block(&f.body);
        if f.ret.is_some() {
            self.ln("return 0;");
        }
        self.indent -= 1;
        self.ln("}");
        self.blank();
        self.ret = None;
    }

    fn task(&mut self, t: &Task) {
        self.task = Some(t.name.clone());
        if !t.reads.is_empty() {
            self.ln(format!("static void {}_snap(void) {{", t.name));
            for s in &t.reads {
                self.ln(format!("  sn_{}_{s} = {s};", t.name));
            }
            self.ln("}");
        }
        if !t.writes.is_empty() {
            self.ln(format!("static void {}_commit(void) {{", t.name));
            for s in &t.writes {
                self.ln(format!(
                    "  if (dirty_{s}) {{ dirty_{s} = false; set_{s}(nx_{s}); }}"
                ));
            }
            self.ln("}");
        }
        self.ln(format!("static void {}_run(void) {{", t.name));
        self.indent += 1;
        self.block(&t.body);
        self.indent -= 1;
        self.ln("}");
        self.blank();
        self.task = None;
    }

    fn irqs(&mut self) {
        let p = self.p;
        self.in_isr = true;
        for i in &p.irqs {
            self.ln(format!("static void {}(void) {{", i.name));
            self.indent += 1;
            self.block(&i.body);
            self.indent -= 1;
            self.ln("}");
        }
        self.in_isr = false;
        // One handler per external interrupt number; several edges on one pin share it.
        let mut by_num: BTreeMap<u8, Vec<&Irq>> = BTreeMap::new();
        for i in &p.irqs {
            let n = p.ios[i.io].aux;
            if n != 255 {
                by_num.entry(n).or_default().push(i);
            }
        }
        for (n, list) in &by_num {
            self.ln(format!("static void irq_{n}(void) {{"));
            for i in list {
                let io = self.io_name(i.io).to_string();
                match i.edge {
                    Edge::Rising => {
                        self.ln(format!("  if (nrt_gpio_get({io})) {{ {}(); }}", i.name))
                    }
                    Edge::Falling => {
                        self.ln(format!("  if (!nrt_gpio_get({io})) {{ {}(); }}", i.name))
                    }
                    Edge::Change => self.ln(format!("  {}();", i.name)),
                }
            }
            self.ln("}");
        }
        let max = by_num.keys().max().map_or(0, |m| usize::from(*m) + 1);
        let entries: Vec<String> = (0..max.max(1))
            .map(|k| {
                if by_num.contains_key(&(k as u8)) {
                    format!("irq_{k}")
                } else {
                    "NULL".to_string()
                }
            })
            .collect();
        self.ln(format!(
            "static const nrt_fn irq_table[{}] = {{ {} }};",
            max.max(1),
            entries.join(", ")
        ));
        self.ln(format!("#define N_IRQ {max}"));
        self.blank();
    }

    fn enter_state(&mut self, k: usize, f: &Fsm, s: usize) {
        let st = &f.states[s];
        self.ln(format!("fsm{k}_t0 = nrt_now();"));
        self.ln(format!("fsm{k}_st = {s};"));
        self.ln(format!(
            "NRT_TRACE(\"STATE\", {}, {});",
            c_string(&f.name),
            c_string(&st.name)
        ));
        self.block(&st.entry);
    }

    fn fsm(&mut self, k: usize, f: &Fsm) {
        self.ln(format!("static NRT_UNUSED uint8_t fsm{k}_st;"));
        self.ln(format!("static NRT_UNUSED bool fsm{k}_on;"));
        self.ln(format!("static NRT_UNUSED uint32_t fsm{k}_t0;"));
        for (s, _) in f.states.iter().enumerate() {
            self.ln(format!("static NRT_UNUSED void fsm{k}_enter{s}(void) {{"));
            self.indent += 1;
            self.enter_state(k, f, s);
            self.indent -= 1;
            self.ln("}");
        }
        self.ln(format!("static void fsm{k}_step(void) {{"));
        self.indent += 1;
        self.ln(format!(
            "if (!fsm{k}_on) {{ fsm{k}_on = true; fsm{k}_enter{}(); }}",
            f.initial
        ));
        self.ln(format!("switch (fsm{k}_st) {{"));
        for (s, st) in f.states.iter().enumerate() {
            self.ln(format!("case {s}: {{"));
            self.indent += 1;
            self.block(&st.during);
            for t in &st.trans {
                self.ln("{");
                self.indent += 1;
                let init = if t.when.is_none() && t.after_ms.is_none() {
                    "true"
                } else {
                    "false"
                };
                self.ln(format!("bool fire = {init};"));
                if let Some(w) = &t.when {
                    let c = self.expr(w);
                    self.ln(format!("fire = {c};"));
                }
                if let Some(a) = t.after_ms {
                    self.ln(format!(
                        "if (!fire) {{ fire = (uint32_t)(nrt_now() - fsm{k}_t0) >= {a}u; }}"
                    ));
                }
                self.ln("if (fire) {");
                self.indent += 1;
                self.block(&st.exit);
                self.block(&t.action);
                self.ln(format!("fsm{k}_enter{}();", t.target));
                self.ln("return;");
                self.indent -= 1;
                self.ln("}");
                self.indent -= 1;
                self.ln("}");
            }
            self.ln("break;");
            self.indent -= 1;
            self.ln("}");
        }
        self.ln("default: break;");
        self.ln("}");
        self.indent -= 1;
        self.ln("}");
        self.blank();
    }

    fn rule_fn(&mut self, name: &str, r: &Rule) {
        self.ln(format!("static NRT_UNUSED bool {name}_prev;"));
        self.ln(format!("static NRT_UNUSED bool {name}_armed;"));
        self.ln(format!("static NRT_UNUSED bool {name}_fired;"));
        self.ln(format!("static NRT_UNUSED uint32_t {name}_since;"));
        self.ln(format!("static void {name}_step(void) {{"));
        self.indent += 1;
        let c = self.expr(&r.cond);
        self.ln(format!("bool c = {c};"));
        match r.hold_ms {
            None => {
                self.ln(format!("bool fire = c && !{name}_prev;"));
                self.ln(format!("{name}_prev = c;"));
                self.ln("if (fire) {");
                self.indent += 1;
                self.block(&r.body);
                self.indent -= 1;
                self.ln("}");
            }
            Some(h) => {
                self.ln("if (c) {");
                self.ln(format!(
                    "  if (!{name}_armed) {{ {name}_armed = true; {name}_since = nrt_now(); }}"
                ));
                self.ln(format!(
                    "  if (!{name}_fired && (uint32_t)(nrt_now() - {name}_since) >= {h}u) {{"
                ));
                self.indent += 2;
                self.ln(format!("{name}_fired = true;"));
                self.block(&r.body);
                self.indent -= 2;
                self.ln("  }");
                self.ln(format!(
                    "}} else {{ {name}_armed = false; {name}_fired = false; }}"
                ));
            }
        }
        self.indent -= 1;
        self.ln("}");
        self.blank();
    }

    fn safety(&mut self) {
        let p = self.p;
        self.ln("static void safe_state(void) {");
        self.indent += 1;
        self.block(&p.safety.safe_state);
        self.indent -= 1;
        self.ln("}");
        self.ln("static void on_fault(uint8_t code) {");
        self.indent += 1;
        self.ln("(void)code;");
        for (f, body) in &p.safety.on_fault {
            match f {
                Some(name) => {
                    let code = self.fault_code(name);
                    self.ln(format!("if (code == {code}) {{"));
                }
                None => self.ln("{"),
            }
            self.indent += 1;
            self.block(body);
            self.indent -= 1;
            self.ln("}");
        }
        self.indent -= 1;
        self.ln("}");
        for (k, r) in p.safety.rules.iter().enumerate() {
            self.rule_fn(&format!("srule{k}"), r);
        }
        self.ln("static NRT_UNUSED void safety_poll(void) {");
        self.indent += 1;
        for k in 0..p.safety.rules.len() {
            self.ln(format!("srule{k}_step();"));
        }
        if let Some(r) = &p.safety.reset {
            let c = self.expr(r);
            self.ln(format!("if (nrt_is_faulted() && {c}) {{ nrt_reset(); }}"));
        }
        self.indent -= 1;
        self.ln("}");
        self.blank();
    }

    fn tables(&mut self) {
        let p = self.p;
        let mut rows: Vec<String> = Vec::new();
        for t in &p.tasks {
            let snap = if t.reads.is_empty() {
                "NULL".to_string()
            } else {
                format!("{}_snap", t.name)
            };
            let commit = if t.writes.is_empty() {
                "NULL".to_string()
            } else {
                format!("{}_commit", t.name)
            };
            rows.push(format!(
                "{{ {}, {}u, {}u, {}, {}, {}u, {snap}, {}_run, {commit} }}",
                c_string(&t.name),
                t.period_ms,
                t.offset_ms,
                t.prio,
                t.event.map_or(0, |e| e + 1),
                t.deadline_ms,
                t.name
            ));
        }
        for (k, f) in p.fsms.iter().enumerate() {
            rows.push(format!(
                "{{ {}, {}u, 0u, 1, 0, 0u, NULL, fsm{k}_step, NULL }}",
                c_string(&f.name),
                f.period_ms
            ));
        }
        for k in 0..p.rules.len() {
            rows.push(format!(
                "{{ \"when\", 10u, 0u, 1, 0, 0u, NULL, rule{k}_step, NULL }}"
            ));
        }
        if let Some((_, period)) = self.ui {
            rows.push(format!(
                "{{ \"ui\", {period}u, 0u, 0, 0, 0u, NULL, ui_task, NULL }}"
            ));
        }
        let n = rows.len();
        if rows.is_empty() {
            rows.push("{ \"\", 0u, 0u, 0, 0, 0u, NULL, NULL, NULL }".to_string());
        }
        self.ln("static const nrt_task_def task_table[] = {");
        for r in &rows {
            self.ln(format!("  {r},"));
        }
        self.ln("};");
        self.blank();
        let has_safety = !p.safety.rules.is_empty() || p.safety.reset.is_some();
        self.ln("const nrt_app neuron_app = {");
        self.ln(format!("  {},", c_string(&p.name)));
        self.ln("  IO_COUNT, io_table,");
        self.ln(format!("  {n}, task_table,"));
        self.ln("  N_IRQ, irq_table,");
        self.ln(format!("  {}, event_names,", p.events.len()));
        self.ln(format!("  {}, fault_names,", p.faults.len()));
        self.ln("  NULL,");
        self.ln("  safe_state,");
        self.ln(format!(
            "  {},",
            if has_safety { "safety_poll" } else { "NULL" }
        ));
        self.ln(format!(
            "  {},",
            if has_safety { p.safety.rate_ms } else { 0 }
        ));
        self.ln("  on_fault,");
        self.ln(format!("  {}", p.safety.watchdog_ms.unwrap_or(0)));
        self.ln("};");
        self.blank();
        self.ln("#if defined(NRT_HOST)");
        self.ln("int main(int argc, char **argv) { return nrt_main(&neuron_app, argc, argv); }");
        self.ln("#else");
        self.ln("int main(void) {");
        self.ln("  nrt_run(&neuron_app);");
        self.ln("  return 0;");
        self.ln("}");
        self.ln("#endif");
    }

    // ---- statements -----------------------------------------------------------------------------

    fn block(&mut self, b: &[Stmt]) {
        for s in b {
            self.stmt(s);
        }
    }

    fn place_is_shared(&self, p: &Place) -> bool {
        match p {
            Place::Global(n) => self.shared.contains(n.as_str()),
            Place::Signal { name, deferred } => !*deferred && self.shared.contains(name.as_str()),
            Place::Local(_) => false,
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        self.mark(s.loc);
        match &s.kind {
            StmtKind::Let { name, ty, value } => {
                let mut pre = Vec::new();
                let v = self.val(value, &mut pre);
                self.flush(pre);
                self.ln(format!("{} {name} = {v};", ty.c_type()));
            }
            StmtKind::Assign { place, value } => {
                let wrap = !self.in_isr && self.place_is_shared(place);
                let saved = self.atomic;
                if wrap {
                    self.atomic = true;
                }
                let trap = may_trap(value);
                let mut pre = Vec::new();
                let v = if wrap {
                    self.expr(value)
                } else {
                    self.val(value, &mut pre)
                };
                let stmt = match place {
                    Place::Local(n) | Place::Global(n) => format!("{n} = {v};"),
                    Place::Signal {
                        name,
                        deferred: true,
                    } => format!("nx_{name} = {v}; dirty_{name} = true;"),
                    Place::Signal {
                        name,
                        deferred: false,
                    } => format!("set_{name}({v});"),
                };
                if wrap {
                    // The value and the store happen in one critical section, and a fault
                    // while computing the value leaves the target untouched.
                    self.ln("NRT_ATOMIC_BEGIN()");
                    if trap {
                        let ty = value.ty().c_type();
                        self.tmp += 1;
                        let t = format!("t{}_", self.tmp);
                        let stmt = stmt.replacen(&v, &t, 1);
                        self.ln(format!("  {ty} {t} = {v};"));
                        self.ln(format!("  if (!nrt_is_faulted()) {{ {stmt} }}"));
                    } else {
                        self.ln(format!("  {stmt}"));
                    }
                    self.ln("NRT_ATOMIC_END()");
                    if trap {
                        self.trap_check();
                    }
                } else {
                    self.flush(pre);
                    self.ln(stmt);
                }
                self.atomic = saved;
            }
            StmtKind::IoPut { io, value } => {
                let mut pre = Vec::new();
                let v = self.val(value, &mut pre);
                self.flush(pre);
                let n = self.io_name(*io).to_string();
                self.ln(format!("nrt_gpio_put({n}, {v});"));
            }
            StmtKind::IoFlip { io } => {
                let n = self.io_name(*io).to_string();
                self.ln(format!("nrt_gpio_flip({n});"));
            }
            StmtKind::IoPwm { io, value } => {
                let mut pre = Vec::new();
                let v = self.val(value, &mut pre);
                self.flush(pre);
                let n = self.io_name(*io).to_string();
                self.ln(format!("nrt_pwm_put({n}, {v});"));
            }
            StmtKind::Print { io, parts, newline } => {
                let n = self.io_name(*io).to_string();
                for part in parts {
                    match part {
                        PrintPart::Text(t) => {
                            self.ln(format!("nrt_uart_text({n}, {});", c_string(t)))
                        }
                        PrintPart::Value(e) => {
                            let mut pre = Vec::new();
                            let v = self.val(e, &mut pre);
                            self.flush(pre);
                            match e.ty() {
                                Ty::Bool => self.ln(format!(
                                    "nrt_uart_text({n}, ({v}) ? \"true\" : \"false\");"
                                )),
                                Ty::F32 => self.ln(format!("nrt_uart_f32({n}, {v});")),
                                _ => self.ln(format!("nrt_uart_i32({n}, (int32_t)({v}));")),
                            }
                        }
                    }
                }
                if *newline {
                    self.ln(format!("nrt_uart_text({n}, \"\\n\");"));
                }
            }
            StmtKind::CallVoid { name, args } => {
                let mut pre = Vec::new();
                let a: Vec<String> = args.iter().map(|x| self.val(x, &mut pre)).collect();
                self.flush(pre);
                self.ln(format!("{name}({});", a.join(", ")));
                // The callee may have faulted: stop this activity too.
                self.trap_check();
            }
            StmtKind::If {
                cond,
                then,
                otherwise,
            } => {
                let mut pre = Vec::new();
                let c = self.val(cond, &mut pre);
                self.flush(pre);
                self.ln(format!("if ({c}) {{"));
                self.indent += 1;
                self.block(then);
                self.indent -= 1;
                if otherwise.is_empty() {
                    self.ln("}");
                } else {
                    self.ln("} else {");
                    self.indent += 1;
                    self.block(otherwise);
                    self.indent -= 1;
                    self.ln("}");
                }
            }
            StmtKind::For {
                var,
                ty,
                lo,
                hi,
                body,
            } => {
                self.tmp += 1;
                let k = self.tmp;
                self.ln(format!(
                    "for (int32_t i{k}_ = {lo}; i{k}_ < {hi}; i{k}_++) {{"
                ));
                self.indent += 1;
                self.ln(format!("{} {var} = ({}) i{k}_;", ty.c_type(), ty.c_type()));
                self.ln(format!("(void){var};"));
                self.block(body);
                self.indent -= 1;
                self.ln("}");
            }
            StmtKind::While { cond, max, body } => {
                self.tmp += 1;
                let k = self.tmp;
                let mut pre = Vec::new();
                let c = self.val(cond, &mut pre);
                if pre.is_empty() {
                    self.ln(format!(
                        "for (int32_t n{k}_ = 0; n{k}_ < {max} && ({c}); n{k}_++) {{"
                    ));
                    self.indent += 1;
                } else {
                    // The condition can fault: evaluate it inside the loop, then test it.
                    self.ln(format!(
                        "for (int32_t n{k}_ = 0; n{k}_ < {max}; n{k}_++) {{"
                    ));
                    self.indent += 1;
                    self.flush(pre);
                    self.ln(format!("if (!({c})) {{ break; }}"));
                }
                self.block(body);
                self.indent -= 1;
                self.ln("}");
            }
            StmtKind::Return(v) => match v {
                Some(e) => {
                    let mut pre = Vec::new();
                    let x = self.val(e, &mut pre);
                    self.flush(pre);
                    self.ln(format!("return {x};"));
                }
                None => self.ln("return;"),
            },
            StmtKind::Break => self.ln("break;"),
            StmtKind::Continue => self.ln("continue;"),
            StmtKind::Fault(f) => {
                let code = self.fault_code(f);
                let ret = if self.ret.is_some() {
                    " return 0;"
                } else {
                    " return;"
                };
                self.ln(format!("{{ nrt_fault({code});{ret} }}"));
            }
            StmtKind::Emit(e) => self.ln(format!("nrt_emit({});", e + 1)),
        }
    }

    // ---- faults inside expressions ----------------------------------------------------------------

    /// The text of `e`; when it can fault, it is computed into a temporary first (`pre`).
    fn val(&mut self, e: &Expr, pre: &mut Vec<String>) -> String {
        let s = self.expr(e);
        if may_trap(e) {
            self.tmp += 1;
            let t = format!("t{}_", self.tmp);
            pre.push(format!("{} {t} = {s};", e.ty().c_type()));
            t
        } else {
            s
        }
    }

    /// Emits the temporaries and the test that leaves the activity after a fault.
    fn flush(&mut self, pre: Vec<String>) {
        if pre.is_empty() {
            return;
        }
        for l in pre {
            self.ln(l);
        }
        self.trap_check();
    }

    fn trap_check(&mut self) {
        if self.ret.is_some() {
            self.ln("if (nrt_is_faulted()) { return 0; }");
        } else {
            self.ln("if (nrt_is_faulted()) { return; }");
        }
    }

    // ---- expressions ----------------------------------------------------------------------------

    fn load(&self, name: &str, t: Ty) -> String {
        if self.shared.contains(name) && !self.atomic && !self.in_isr {
            format!("nrt_ld_{}(&{name})", t.tag())
        } else {
            name.to_string()
        }
    }

    fn expr(&mut self, e: &Expr) -> String {
        match e {
            Expr::Int(v, t) => int_lit(*v, *t),
            Expr::Float(f) => float_lit(*f),
            Expr::Bool(b) => b.to_string(),
            Expr::Local(n, _) => n.clone(),
            Expr::Global(n, t) => self.load(n, *t),
            Expr::Signal(n, t, SigAccess::Current) => self.load(n, *t),
            Expr::Signal(n, t, SigAccess::Snapshot) => match &self.task {
                Some(task) => format!("sn_{task}_{n}"),
                None => self.load(n, *t),
            },
            Expr::Unary(op, x, t) => {
                let v = self.expr(x);
                match (op, t) {
                    (UnOp::Not, _) => format!("(!({v}))"),
                    (UnOp::Neg, Ty::F32) => format!("(-({v}))"),
                    (UnOp::Neg, t) => fit(*t, format!("-(int64_t)({v})")),
                    (UnOp::BitNot, t) => format!("(({})(~({v})))", t.c_type()),
                }
            }
            Expr::Binary(op, a, b, t) => self.binary(*op, a, b, *t),
            Expr::Cast(x, t) => {
                let v = self.expr(x);
                match (x.ty(), *t) {
                    (_, Ty::F32) => format!("((float)({v}))"),
                    (Ty::Bool, t) => format!("(({})({v}))", t.c_type()),
                    (_, t) => fit(t, format!("(int64_t)({v})")),
                }
            }
            Expr::IfElse(c, a, b, _) => {
                let (c, a, b) = (self.expr(c), self.expr(a), self.expr(b));
                format!("(({c}) ? ({a}) : ({b}))")
            }
            Expr::Call(n, args, _) => {
                let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
                format!("{n}({})", a.join(", "))
            }
            Expr::Builtin(n, args, t) => {
                let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
                if t.is_float() {
                    let f = match n.as_str() {
                        "abs" => "nrt_fabs",
                        "min" => "nrt_fmin",
                        "max" => "nrt_fmax",
                        _ => "nrt_fclamp",
                    };
                    format!("{f}({})", a.join(", "))
                } else {
                    let f = match n.as_str() {
                        "abs" => "nrt_iabs",
                        "min" => "nrt_imin",
                        "max" => "nrt_imax",
                        _ => "nrt_iclamp",
                    };
                    let a: Vec<String> = a.iter().map(|x| format!("(int64_t)({x})")).collect();
                    fit(*t, format!("{f}({})", a.join(", ")))
                }
            }
            Expr::IoGet(i, t) => {
                let n = self.io_name(*i).to_string();
                if *t == Ty::Bool {
                    format!("nrt_gpio_get({n})")
                } else {
                    format!("nrt_adc_get({n})")
                }
            }
            Expr::Scale(v) => {
                let a: Vec<String> = v.iter().map(|x| self.expr(x)).collect();
                format!("nrt_scale({})", a.join(", "))
            }
        }
    }

    fn binary(&mut self, op: BinOp, a: &Expr, b: &Expr, t: Ty) -> String {
        let (x, y) = (self.expr(a), self.expr(b));
        let ot = a.ty();
        let wide = |s: &str| format!("(int64_t)({s})");
        match op {
            BinOp::And => format!("(({x}) && ({y}))"),
            BinOp::Or => format!("(({x}) || ({y}))"),
            BinOp::Eq => format!("(({x}) == ({y}))"),
            BinOp::Ne => format!("(({x}) != ({y}))"),
            BinOp::Lt => format!("(({x}) < ({y}))"),
            BinOp::Le => format!("(({x}) <= ({y}))"),
            BinOp::Gt => format!("(({x}) > ({y}))"),
            BinOp::Ge => format!("(({x}) >= ({y}))"),
            _ if ot.is_float() => match op {
                BinOp::Add => format!("(({x}) + ({y}))"),
                BinOp::Sub => format!("(({x}) - ({y}))"),
                BinOp::Mul => format!("(({x}) * ({y}))"),
                BinOp::Div => format!("nrt_fdiv({x}, {y})"),
                _ => "0.0f".to_string(),
            },
            BinOp::Add => fit(t, format!("{} + {}", wide(&x), wide(&y))),
            BinOp::Sub => fit(t, format!("{} - {}", wide(&x), wide(&y))),
            BinOp::Mul => fit(t, format!("{} * {}", wide(&x), wide(&y))),
            BinOp::Div => fit(t, format!("nrt_div({}, {})", wide(&x), wide(&y))),
            BinOp::Rem => fit(t, format!("nrt_rem({}, {})", wide(&x), wide(&y))),
            BinOp::AddW => format!("(({})({} + {}))", t.c_type(), wide(&x), wide(&y)),
            BinOp::SubW => format!("(({})({} - {}))", t.c_type(), wide(&x), wide(&y)),
            BinOp::MulW => format!("(({})({} * {}))", t.c_type(), wide(&x), wide(&y)),
            BinOp::BitAnd => format!("(({})(({x}) & ({y})))", t.c_type()),
            BinOp::BitOr => format!("(({})(({x}) | ({y})))", t.c_type()),
            BinOp::BitXor => format!("(({})(({x}) ^ ({y})))", t.c_type()),
            BinOp::Shl => format!("(({})({} << ({y})))", t.c_type(), wide(&x)),
            BinOp::Shr => format!("(({})({} >> ({y})))", t.c_type(), wide(&x)),
        }
    }
}
