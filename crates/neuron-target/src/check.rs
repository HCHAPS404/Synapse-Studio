//! Hardware checks of a lowered package against its board (spec 07 §7.1 to §7.3).
//!
//! [`check`] reads the NIR, resolves the board named after `on`, binds every `io` to a pin
//! or a peripheral, and reports:
//!
//! | Code | Meaning |
//! |---|---|
//! | `NE0601` | a pin, peripheral or channel is owned twice (HW-1) |
//! | `NE0603` | a pin does not support the requested function, or does not exist |
//! | `NE0605` | the requested system clock cannot be made |
//! | `NE0606` | a derived rate (baud, PWM) misses its tolerance |
//! | `NE0607` | the board, chip, architecture or runtime is unknown |
//! | `NE0608` | a peripheral option is beyond what the chip can do |

use crate::model::Target;
use crate::registry::Registry;
use crate::solve::{self, ClockError, Owners};
use neuron_nir::{Expr, ExprKind, ItemDef, Lit, Module, Nir};
use std::collections::BTreeMap;

/// A diagnostic of the hardware checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    /// Code (`NE0601`).
    pub code: &'static str,
    /// Message.
    pub message: String,
    /// Suggested fix.
    pub help: Option<String>,
    /// Source file of the module.
    pub file: String,
    /// Byte range in that file.
    pub span: [usize; 2],
}

/// Kind of an `io` binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoKind {
    /// `gpio.input`.
    GpioInput,
    /// `gpio.output`.
    GpioOutput,
    /// `gpio.open_drain`.
    GpioOpenDrain,
    /// `pwm`.
    Pwm,
    /// `adc`.
    Adc,
    /// `uart`.
    Uart,
    /// `timer`.
    Timer,
    /// Any other peripheral kind (not checked beyond the pin).
    Other(String),
}

impl IoKind {
    fn from_name(name: &str) -> IoKind {
        match name {
            "gpio.input" => IoKind::GpioInput,
            "gpio.output" => IoKind::GpioOutput,
            "gpio.open_drain" => IoKind::GpioOpenDrain,
            "pwm" => IoKind::Pwm,
            "adc" => IoKind::Adc,
            "uart" | "usart" => IoKind::Uart,
            "timer" => IoKind::Timer,
            other => IoKind::Other(other.to_string()),
        }
    }
}

/// PWM resolved to a timer channel.
#[derive(Debug, Clone, PartialEq)]
pub struct PwmCfg {
    /// Timer (`timer1`).
    pub timer: String,
    /// Channel letter (`a`, `b`, `c`).
    pub channel: char,
    /// Prescaler.
    pub prescaler: u32,
    /// Requested carrier.
    pub freq_hz: f64,
    /// Carrier that results.
    pub achieved_hz: f64,
}

/// UART resolved to an instance and a divisor.
#[derive(Debug, Clone, PartialEq)]
pub struct UartCfg {
    /// Instance number (`uart0` is 0).
    pub index: u32,
    /// Requested rate.
    pub baud: u32,
    /// Divisor register value.
    pub ubrr: u32,
    /// Double-speed mode.
    pub double_speed: bool,
    /// Relative error of the rate.
    pub error: f64,
}

/// An `io` bound to hardware.
#[derive(Debug, Clone, PartialEq)]
pub struct IoBinding {
    /// Index of the module in the NIR.
    pub module: usize,
    /// Binding name.
    pub name: String,
    /// Kind.
    pub kind: IoKind,
    /// Location in the source file.
    pub span: [usize; 2],
    /// Chip pin.
    pub pin: Option<String>,
    /// Port letter of the pin.
    pub port: Option<char>,
    /// Bit of the pin in its port.
    pub bit: Option<u8>,
    /// PWM channel.
    pub pwm: Option<PwmCfg>,
    /// ADC channel.
    pub adc_channel: Option<u32>,
    /// UART instance.
    pub uart: Option<UartCfg>,
    /// External interrupt number of the pin.
    pub int_num: Option<u32>,
    /// `pull: up` was requested.
    pub pull_up: bool,
    /// Options as written, with quantities in base units.
    pub config: BTreeMap<String, String>,
}

/// Result of the hardware checks of one package.
#[derive(Debug, Clone, Default)]
pub struct HwReport {
    /// The board of the first module that names one.
    pub target: Option<Target>,
    /// Problems found.
    pub diags: Vec<Diag>,
    /// Bindings that were resolved (also those with problems when they could be located).
    pub bindings: Vec<IoBinding>,
    /// System clock in hertz.
    pub sysclk_hz: u64,
}

impl HwReport {
    /// `true` when no error was found.
    pub fn is_ok(&self) -> bool {
        self.diags.is_empty()
    }
}

/// Flattens a name expression: `gpio.output`, `ADC1.in0`, `D13`.
pub fn flat(e: &Expr) -> Option<String> {
    match &e.e {
        ExprKind::Global { name } | ExprKind::Ref { name, .. } | ExprKind::Local { name, .. } => {
            Some(name.clone())
        }
        ExprKind::Field { base, name, .. } => Some(format!("{}.{}", flat(base)?, name)),
        ExprKind::Call { callee, .. } => flat(callee),
        _ => None,
    }
}

/// The value of a configuration expression as text, with quantities in base units.
pub fn expr_text(e: &Expr) -> Option<String> {
    match &e.e {
        ExprKind::Lit { lit } => match lit {
            Lit::Int { v, .. } | Lit::Real { v, .. } => Some(v.clone()),
            Lit::Bool { v } => Some(v.to_string()),
            Lit::Str { text } => Some(text.trim_matches('"').to_string()),
            _ => None,
        },
        ExprKind::Quantity { value, si, .. } => si.clone().or_else(|| expr_text(value)),
        ExprKind::Unary { op, operand } if op == "-" => expr_text(operand).map(|t| format!("-{t}")),
        _ => flat(e),
    }
}

fn call_parts(e: &Expr) -> (String, Vec<(Option<String>, String)>) {
    match &e.e {
        ExprKind::Call { callee, args, .. } => {
            let head = flat(callee).unwrap_or_default();
            let args = args
                .iter()
                .map(|a| (a.name.clone(), expr_text(&a.value).unwrap_or_default()))
                .collect();
            (head, args)
        }
        _ => (flat(e).unwrap_or_default(), Vec::new()),
    }
}

struct Ctx<'a> {
    target: &'a Target,
    module: &'a Module,
    index: usize,
    owners: Owners,
    diags: Vec<Diag>,
    sysclk_hz: u64,
}

impl Ctx<'_> {
    fn diag(
        &mut self,
        code: &'static str,
        message: String,
        help: Option<String>,
        span: [usize; 2],
    ) {
        self.diags.push(Diag {
            code,
            message,
            help,
            file: self.module.file.clone(),
            span,
        });
    }

    fn claim(&mut self, resource: &str, who: &str, span: [usize; 2]) {
        if let Err(owner) = self.owners.claim(resource, who) {
            self.diag(
                "NE0601",
                format!("`{resource}` is already used by `{owner}`, so `{who}` cannot use it"),
                Some(
                    "each pin, peripheral instance and channel can belong to one binding only"
                        .into(),
                ),
                span,
            );
        }
    }
}

fn pin_function_class(kind: &IoKind) -> Option<&'static str> {
    match kind {
        IoKind::GpioInput | IoKind::GpioOutput | IoKind::GpioOpenDrain => Some("gpio"),
        IoKind::Pwm => Some("pwm:"),
        IoKind::Adc => Some("adc:"),
        _ => None,
    }
}

fn parse_f64(text: Option<&String>) -> Option<f64> {
    text.and_then(|t| t.parse::<f64>().ok())
}

fn bind_io(
    ctx: &mut Ctx<'_>,
    name: &str,
    kind_e: &Expr,
    at_e: &Expr,
    span: [usize; 2],
) -> Option<IoBinding> {
    let (kind_name, kind_args) = call_parts(kind_e);
    let kind = IoKind::from_name(&kind_name);
    let config: BTreeMap<String, String> = kind_args
        .into_iter()
        .filter_map(|(n, v)| n.map(|n| (n, v)))
        .collect();
    let (head, at_args) = call_parts(at_e);
    let target = ctx.target;
    let mut b = IoBinding {
        module: ctx.index,
        name: name.to_string(),
        kind: kind.clone(),
        span,
        pin: None,
        port: None,
        bit: None,
        pwm: None,
        adc_channel: None,
        uart: None,
        int_num: None,
        pull_up: config.get("pull").is_some_and(|p| p == "up"),
        config: config.clone(),
    };

    // Peripheral locations: `UART0`, `USART0(tx: PD1, rx: PD0)`, `TIMER1`.
    if matches!(kind, IoKind::Uart | IoKind::Timer) {
        let Some(per) = target.peripheral(&head) else {
            ctx.diag(
                "NE0603",
                format!("`{head}` is not a peripheral of `{}`", target.chip.name),
                Some(format!(
                    "peripherals of this chip: {}",
                    target
                        .chip
                        .peripherals
                        .iter()
                        .filter(|p| p.kind
                            == if kind == IoKind::Uart {
                                "uart"
                            } else {
                                "timer"
                            })
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                span,
            );
            return Some(b);
        };
        let per = per.clone();
        ctx.claim(&per.name, name, span);
        if ctx.target.runtime.reserves.contains(&per.name) {
            ctx.diag(
                "NE0601",
                format!(
                    "`{}` is used by the runtime tick, so `{name}` cannot use it",
                    per.name
                ),
                Some("choose another timer".into()),
                span,
            );
        }
        for (role, pin) in &per.pins {
            ctx.claim(pin, &format!("{name}.{role}"), span);
        }
        if kind == IoKind::Uart {
            let baud = parse_f64(config.get("baud")).unwrap_or(9600.0) as u32;
            let tol = parse_f64(config.get("tolerance")).unwrap_or(0.03);
            if ctx.target.arch.isa == "avr" {
                match solve::uart_avr(ctx.sysclk_hz, baud) {
                    Some(d) => {
                        if d.error > tol {
                            ctx.diag(
                                "NE0606",
                                format!(
                                    "`{name}` asks for {baud} Bd but the nearest rate at {} Hz is {:.0} Bd ({:+.1} %)",
                                    ctx.sysclk_hz,
                                    d.achieved_baud,
                                    100.0 * (d.achieved_baud - f64::from(baud)) / f64::from(baud)
                                ),
                                Some("pick a standard rate that divides the clock better, or raise `tolerance`".into()),
                                span,
                            );
                        }
                        b.uart = Some(UartCfg {
                            index: per.index.unwrap_or(0),
                            baud,
                            ubrr: d.ubrr,
                            double_speed: d.double_speed,
                            error: d.error,
                        });
                    }
                    None => ctx.diag(
                        "NE0606",
                        format!(
                            "`{name}`: {baud} Bd cannot be made from {} Hz",
                            ctx.sysclk_hz
                        ),
                        None,
                        span,
                    ),
                }
            } else {
                b.uart = Some(UartCfg {
                    index: per.index.unwrap_or(0),
                    baud,
                    ubrr: 0,
                    double_speed: false,
                    error: 0.0,
                });
            }
        }
        return Some(b);
    }

    // Pin locations: `D13`, `PA5`, or `ADC1.in0(PA0)` (peripheral path with its pin).
    let (pin_name, required): (String, Option<String>) = if head.contains('.') {
        let pin = at_args.first().map(|(_, v)| v.clone()).unwrap_or_default();
        (pin, Some(head.to_ascii_lowercase()))
    } else {
        (head.clone(), None)
    };
    let Some(pin) = target.pin(&pin_name) else {
        let mut help = String::new();
        if let Some(cls) = pin_function_class(&kind) {
            let list = target.pins_with(cls);
            if !list.is_empty() {
                help = format!("pins with this function: {}", list.join(", "));
            }
        }
        ctx.diag(
            "NE0603",
            format!(
                "`{pin_name}` is not a pin of the board `{}`",
                target.board.name
            ),
            if help.is_empty() { None } else { Some(help) },
            span,
        );
        return Some(b);
    };
    let pin = pin.clone();
    b.pin = Some(pin.name.clone());
    b.port = pin.port.as_ref().and_then(|p| p.chars().next());
    b.bit = pin.bit;

    let ok = match (&required, pin_function_class(&kind)) {
        (Some(path), _) => pin.functions.iter().any(|f| f == path),
        (None, Some(cls)) => pin.functions.iter().any(|f| f.starts_with(cls)),
        (None, None) => true,
    };
    if !ok {
        let want = required
            .clone()
            .or_else(|| pin_function_class(&kind).map(|c| c.trim_end_matches(':').to_string()))
            .unwrap_or_default();
        let list = target.pins_with(&want);
        ctx.diag(
            "NE0603",
            format!(
                "pin `{pin_name}` ({}) does not support `{want}` as `{name}` needs",
                pin.name
            ),
            if list.is_empty() {
                None
            } else {
                Some(format!("pins that do: {}", list.join(", ")))
            },
            span,
        );
        return Some(b);
    }
    ctx.claim(&pin.name, name, span);

    match kind {
        IoKind::GpioInput => {
            b.int_num = pin
                .functions
                .iter()
                .find_map(|f| f.strip_prefix("int:").and_then(|n| n.parse().ok()));
        }
        IoKind::Adc => {
            b.adc_channel = pin
                .functions
                .iter()
                .find_map(|f| f.strip_prefix("adc:").and_then(|n| n.parse().ok()));
            let want = parse_f64(config.get("resolution")).unwrap_or(10.0) as u32;
            let max = ctx
                .target
                .chip
                .peripherals
                .iter()
                .find(|p| p.kind == "adc")
                .and_then(|p| p.bits)
                .unwrap_or(12);
            if want > max {
                ctx.diag(
                    "NE0608",
                    format!(
                        "`{name}` asks for {want}-bit conversions; the ADC of `{}` has {max} bits",
                        ctx.target.chip.name
                    ),
                    Some(format!("write `resolution: {max}` or less")),
                    span,
                );
            }
        }
        IoKind::Pwm => {
            let chan = pin.functions.iter().find_map(|f| f.strip_prefix("pwm:"));
            if let Some(chan) = chan {
                let (timer, ch) = chan.split_once('.').unwrap_or((chan, "a"));
                let channel = ch.chars().next().unwrap_or('a');
                ctx.claim(&format!("{timer}.{ch}"), name, span);
                if ctx.target.runtime.reserves.iter().any(|r| r == timer) {
                    ctx.diag(
                        "NE0601",
                        format!("`{timer}` is used by the runtime tick, so `{name}` cannot use it"),
                        Some("use a pin of another timer".into()),
                        span,
                    );
                }
                let want = parse_f64(config.get("freq")).unwrap_or(1000.0);
                let tol = parse_f64(config.get("tolerance")).unwrap_or(0.05);
                let prescalers = ctx
                    .target
                    .chip
                    .peripherals
                    .iter()
                    .find(|p| p.name == timer)
                    .map(|p| p.prescalers.clone())
                    .unwrap_or_default();
                if ctx.target.arch.isa == "avr" {
                    if let Some(sol) = solve::pwm_fast8(ctx.sysclk_hz, &prescalers, want) {
                        if sol.error > tol {
                            ctx.diag(
                                "NE0606",
                                format!(
                                    "`{name}` asks for a {want} Hz carrier; the nearest on `{timer}` is {:.1} Hz ({:.1} % off)",
                                    sol.achieved_hz,
                                    100.0 * sol.error
                                ),
                                Some("change `freq`, use another timer, or raise `tolerance`".into()),
                                span,
                            );
                        }
                        b.pwm = Some(PwmCfg {
                            timer: timer.to_string(),
                            channel,
                            prescaler: sol.prescaler,
                            freq_hz: want,
                            achieved_hz: sol.achieved_hz,
                        });
                    }
                } else {
                    b.pwm = Some(PwmCfg {
                        timer: timer.to_string(),
                        channel,
                        prescaler: 1,
                        freq_hz: want,
                        achieved_hz: want,
                    });
                }
            }
        }
        _ => {}
    }
    Some(b)
}

fn check_clocks(ctx: &mut Ctx<'_>, def: &neuron_nir::ClockDef) {
    if def.name != "sysclk" {
        return;
    }
    let want = parse_f64(expr_text(&def.value).as_ref()).unwrap_or(0.0) as u64;
    let (source, source_hz) = match &def.from {
        Some(f) => {
            let (name, args) = call_parts(f);
            let hz = args
                .first()
                .and_then(|(_, v)| v.parse::<f64>().ok())
                .map_or(ctx.target.board.crystal_hz, |v| v as u64);
            (name, hz)
        }
        None => (
            ctx.target
                .chip
                .clock
                .sources
                .first()
                .map(|s| s.name.clone())
                .unwrap_or_default(),
            ctx.target.board.crystal_hz,
        ),
    };
    match solve::solve_sysclk(&ctx.target.chip.clock, &source, source_hz, want) {
        Ok(sol) => ctx.sysclk_hz = sol.hz,
        Err(ClockError::BadSource { name, hz }) => ctx.diag(
            "NE0605",
            format!(
                "clock source `{name}` at {hz} Hz is not available on `{}`",
                ctx.target.chip.name
            ),
            Some(format!(
                "sources: {}",
                ctx.target
                    .chip
                    .clock
                    .sources
                    .iter()
                    .map(|s| format!("{} ({}..{} Hz)", s.name, s.min_hz, s.max_hz))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            def.span,
        ),
        Err(ClockError::Unreachable {
            requested_hz,
            below,
            above,
        }) => {
            let near = [below, above]
                .iter()
                .flatten()
                .map(|h| format!("{h} Hz"))
                .collect::<Vec<_>>()
                .join(" or ");
            ctx.diag(
                "NE0605",
                format!(
                    "`sysclk = {requested_hz} Hz` cannot be made from {source_hz} Hz on `{}`",
                    ctx.target.chip.name
                ),
                if near.is_empty() {
                    None
                } else {
                    Some(format!("nearest achievable: {near}"))
                },
                def.span,
            );
        }
    }
}

/// Checks every `mcu` module of the package that names a board.
pub fn check(reg: &Registry, nir: &Nir) -> HwReport {
    let mut report = HwReport::default();
    for (mi, module) in nir.modules.iter().enumerate() {
        if module.dialect != "mcu" {
            continue;
        }
        let Some(board) = &module.target else {
            continue;
        };
        let target = match reg.resolve(board) {
            Ok(t) => t,
            Err(e) => {
                let span = module.items.first().map_or([0, 0], |i| i.span);
                report.diags.push(Diag {
                    code: "NE0607",
                    message: e.to_string(),
                    help: Some("`neuron targets` lists the boards".into()),
                    file: module.file.clone(),
                    span,
                });
                continue;
            }
        };
        let mut ctx = Ctx {
            target: &target,
            module,
            index: mi,
            owners: Owners::default(),
            diags: Vec::new(),
            sysclk_hz: target.board.sysclk_hz.unwrap_or(target.board.crystal_hz),
        };
        for r in &target.runtime.reserves {
            let _ = ctx.owners.claim(r, "the runtime");
        }
        // Clocks first: the derived rates below depend on the system clock.
        for item in &module.items {
            if let ItemDef::Clocks { clocks } = &item.def {
                for c in clocks {
                    check_clocks(&mut ctx, c);
                }
            }
        }
        for item in &module.items {
            match &item.def {
                ItemDef::Io { name, kind, at } => {
                    if let Some(b) = bind_io(&mut ctx, name, kind, at, item.span) {
                        report.bindings.push(b);
                    }
                }
                ItemDef::Uses { paths } => {
                    for p in paths {
                        let res = p.rsplit('.').next().unwrap_or(p).to_ascii_lowercase();
                        ctx.claim(&res, &format!("uses {p}"), item.span);
                    }
                }
                _ => {}
            }
        }
        // Interrupt sources need a pin that can raise them.
        for item in &module.items {
            if let ItemDef::Interrupt { source, .. } = &item.def {
                if let ExprKind::Field { base, name, .. } = &source.e {
                    if matches!(name.as_str(), "rising" | "falling" | "change") {
                        if let ExprKind::Ref { name: io_name, .. } = &base.e {
                            let found = report
                                .bindings
                                .iter()
                                .find(|b| b.module == mi && &b.name == io_name);
                            if let Some(b) = found {
                                if b.pin.is_some() && b.int_num.is_none() {
                                    let list = target.pins_with("int:");
                                    ctx.diag(
                                        "NE0603",
                                        format!("pin `{}` of `{io_name}` cannot raise `{name}` interrupts", b.pin.clone().unwrap_or_default()),
                                        Some(format!("pins with an external interrupt: {}", list.join(", "))),
                                        item.span,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        if report.target.is_none() {
            report.sysclk_hz = ctx.sysclk_hz;
            report.target = Some(target.clone());
        }
        report.diags.extend(ctx.diags);
    }
    // Keep the binding order stable and the diagnostics sorted by place.
    report
        .diags
        .sort_by(|a, b| (a.file.as_str(), a.span, a.code).cmp(&(b.file.as_str(), b.span, b.code)));
    report
}
