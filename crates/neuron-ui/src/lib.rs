//! The `ui` dialect on small displays (task B9).
//!
//! [`lower_ui`] reads the `ui` module of a package: one `display` (SSD1306 over I2C, ILI9341 or
//! ST7789 over SPI, HD44780 behind an I2C backpack), its first `screen`, and the widgets
//! `label`, `gauge`, `bar`, `slider`, `led` and `button`. It chooses the render strategy from
//! the RAM of the chip (spec 09 §9.7): a full framebuffer when it takes at most half of the
//! RAM, bands of 8 rows when a band takes at most a quarter, otherwise `NE0604`.
//! [`emit_ui_c`] writes the C that the runtime renderer (`runtime/neuron-rt/ui`) draws from,
//! [`render`] is the reference renderer that the firmware must match pixel for pixel, and
//! [`emu`] decodes the bus traffic of the displays back into images.
//!
//! A widget binds a `signal` of the ui module. When an `mcu` module of the same package
//! declares a signal with the same name, the two are the same signal (the ui reads it).
//!
//! ```
//! assert_eq!(neuron_ui::font::index('a'), neuron_ui::font::index('A'));
//! ```

pub mod emu;
pub mod font;
mod render;

pub use render::{render, Image};

use neuron_mir::{Program, Ty};
use neuron_nir as n;
use neuron_target::Diag;

/// A display driver of `std.display`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    /// 128x64 (or 128x32) monochrome OLED over I2C.
    Ssd1306,
    /// RGB565 TFT over SPI.
    Ili9341,
    /// RGB565 TFT over SPI (same command set for what the renderer uses).
    St7789,
    /// Character LCD behind a PCF8574 I2C backpack.
    Hd44780,
}

/// Pixel format of the display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// One bit per pixel, in pages of 8 rows (SSD1306).
    Mono,
    /// 16-bit color.
    Rgb565,
    /// A grid of characters.
    Chars,
}

/// How the screen is drawn (chosen from the RAM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// The whole frame in RAM, sent at once.
    Framebuffer,
    /// Bands of 8 rows, drawn and sent one after another.
    Bands,
    /// A character grid.
    CharGrid,
}

/// Widget kinds the renderer draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetKind {
    /// Text, or the value of a signal.
    Label,
    /// A horizontal bar with the value under it.
    Gauge,
    /// A vertical bar.
    Bar,
    /// A horizontal track with a knob.
    Slider,
    /// A square lamp.
    Led,
    /// A framed caption (touch input is not wired yet).
    Button,
}

/// The source of a widget value.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    /// Name of the signal in the source.
    pub signal: String,
    /// C variable that holds it (a global of the MIR program, or one the ui section declares).
    pub c_name: String,
    /// Machine type of the variable.
    pub ty: Ty,
    /// Display value = (stored - offset) / scale (the unit of the ui signal).
    pub unit_scale: f32,
    /// See `unit_scale`.
    pub unit_offset: f32,
    /// The variable is declared by the ui section (no mcu module writes it).
    pub own: bool,
}

/// One widget, laid out.
#[derive(Debug, Clone, PartialEq)]
pub struct UiWidget {
    /// Kind.
    pub kind: WidgetKind,
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width (bars, sliders).
    pub w: i32,
    /// Height (bars, sliders).
    pub h: i32,
    /// Text scale (1 is the 6x8 cell).
    pub scale: i32,
    /// Fixed text.
    pub text: Option<String>,
    /// Bound value.
    pub bind: Option<Binding>,
    /// Low end of the range, in display units.
    pub lo: f32,
    /// High end of the range, in display units.
    pub hi: f32,
}

/// The display of the program.
#[derive(Debug, Clone, PartialEq)]
pub struct UiDisplay {
    /// Name in the source.
    pub name: String,
    /// Driver.
    pub driver: Driver,
    /// Bus name (`i2c1`, `spi1`).
    pub bus: String,
    /// I2C address (I2C drivers).
    pub addr: u8,
    /// Width in pixels (characters for HD44780).
    pub w: i32,
    /// Height in pixels (rows for HD44780).
    pub h: i32,
    /// Rotation in degrees.
    pub rotation: i32,
    /// Pixel format.
    pub format: Format,
}

/// A ui program ready to emit.
#[derive(Debug, Clone, PartialEq)]
pub struct UiApp {
    /// Source file of the ui module.
    pub file: String,
    /// The display.
    pub display: UiDisplay,
    /// The screen shown (the first one).
    pub screen: String,
    /// Its widgets.
    pub widgets: Vec<UiWidget>,
    /// Render strategy.
    pub strategy: Strategy,
    /// Rows drawn per band (the height for a framebuffer).
    pub band_rows: i32,
    /// Bytes of the render buffer.
    pub buffer_bytes: u32,
    /// Refresh period of the renderer in milliseconds.
    pub period_ms: u32,
}

fn diag(
    file: &str,
    span: [usize; 2],
    code: &'static str,
    message: String,
    help: Option<String>,
) -> Diag {
    Diag {
        code,
        message,
        help,
        file: file.to_string(),
        span,
    }
}

fn flat(e: &n::Expr) -> Option<String> {
    match &e.e {
        n::ExprKind::Global { name }
        | n::ExprKind::Ref { name, .. }
        | n::ExprKind::Local { name, .. } => Some(name.clone()),
        n::ExprKind::Field { base, name, .. } => Some(format!("{}.{name}", flat(base)?)),
        _ => None,
    }
}

fn num(e: &n::Expr) -> Option<f64> {
    match &e.e {
        n::ExprKind::Lit {
            lit: n::Lit::Int { v, .. } | n::Lit::Real { v, .. },
        } => v.parse().ok(),
        n::ExprKind::Quantity { value, .. } => num(value),
        n::ExprKind::Unary { op, operand } if op == "-" => num(operand).map(|v| -v),
        _ => None,
    }
}

/// The value of a quantity in base SI units (plain numbers as they are).
fn si(e: &n::Expr) -> Option<f64> {
    match &e.e {
        n::ExprKind::Quantity { si: Some(s), .. } => s.parse().ok(),
        _ => num(e),
    }
}

fn pair(e: &n::Expr) -> Option<(i32, i32)> {
    match &e.e {
        n::ExprKind::Tuple { items } if items.len() == 2 => {
            Some((num(&items[0])? as i32, num(&items[1])? as i32))
        }
        _ => None,
    }
}

fn exact(s: &str) -> f64 {
    match s.split_once('/') {
        Some((a, b)) => {
            a.trim().parse::<f64>().unwrap_or(1.0) / b.trim().parse::<f64>().unwrap_or(1.0)
        }
        None => s.trim().parse().unwrap_or(1.0),
    }
}

fn unit_of(t: Option<&n::Type>) -> (f64, f64) {
    match t {
        Some(n::Type::Num { unit: Some(u), .. }) => (exact(&u.scale), exact(&u.offset)),
        _ => (1.0, 0.0),
    }
}

/// The ui program of a package, or `None` when it has no `ui` module. `ram_bytes` is the RAM
/// of the chip; `program` is the MIR of the `mcu` modules (its signals feed the widgets).
pub fn lower_ui(
    nir: &n::Nir,
    ram_bytes: u64,
    program: Option<&Program>,
) -> Result<Option<UiApp>, Vec<Diag>> {
    let Some(m) = nir.modules.iter().find(|m| m.dialect == "ui") else {
        return Ok(None);
    };
    let file = m.file.clone();
    let mut diags = Vec::new();
    let mut display: Option<UiDisplay> = None;
    let mut signals: Vec<(String, Option<n::Type>)> = Vec::new();
    let mut screen: Option<(String, &Vec<n::Widget>)> = None;
    for item in &m.items {
        match &item.def {
            n::ItemDef::Display { name, driver } => {
                if display.is_some() {
                    diags.push(diag(
                        &file,
                        item.span,
                        "NE0610",
                        "a second display".into(),
                        Some("one display per program for now".into()),
                    ));
                    continue;
                }
                display = lower_display(&file, item.span, name, driver, &mut diags);
            }
            n::ItemDef::Signal { name, vty, .. } => signals.push((name.clone(), vty.clone())),
            n::ItemDef::Screen { name, widgets, .. } if screen.is_none() => {
                screen = Some((name.clone(), widgets));
            }
            _ => {}
        }
    }
    let Some(display) = display else {
        diags.push(diag(
            &file,
            [0, 0],
            "NE0610",
            "a ui module without a `display`".into(),
            None,
        ));
        return Err(diags);
    };
    let Some((screen_name, widgets)) = screen else {
        diags.push(diag(
            &file,
            [0, 0],
            "NE0610",
            "a ui module without a `screen`".into(),
            None,
        ));
        return Err(diags);
    };

    let mut out = Vec::new();
    for w in widgets {
        if let Some(u) = lower_widget(&file, w, &display, &signals, program, &mut diags) {
            out.push(u);
        }
    }

    // Render strategy from the RAM (MEM-1).
    let (w, h) = (display.w as u64, display.h as u64);
    let (strategy, rows, bytes) = match display.format {
        Format::Chars => (Strategy::CharGrid, display.h, (w * h) as u32),
        Format::Mono | Format::Rgb565 => {
            let bpp = if display.format == Format::Mono {
                1
            } else {
                16
            };
            let frame = w * h * bpp / 8;
            let band = w * 8 * bpp / 8;
            if frame <= ram_bytes / 2 {
                (Strategy::Framebuffer, display.h, frame as u32)
            } else if band <= ram_bytes / 4 {
                (Strategy::Bands, 8, band as u32)
            } else {
                diags.push(diag(
                    &file,
                    [0, 0],
                    "NE0604",
                    format!(
                        "the display `{}` ({}x{}) needs {frame} bytes for a framebuffer or {band} bytes for a band of 8 rows; the chip has {ram_bytes} bytes of RAM",
                        display.name, display.w, display.h
                    ),
                    Some("use a smaller or monochrome display, or a board with more RAM".into()),
                ));
                (Strategy::Bands, 8, band as u32)
            }
        }
    };
    if !diags.is_empty() {
        return Err(diags);
    }
    Ok(Some(UiApp {
        file,
        display,
        screen: screen_name,
        widgets: out,
        strategy,
        band_rows: rows,
        buffer_bytes: bytes,
        period_ms: 100,
    }))
}

fn lower_display(
    file: &str,
    span: [usize; 2],
    name: &str,
    driver: &n::Expr,
    diags: &mut Vec<Diag>,
) -> Option<UiDisplay> {
    let n::ExprKind::Call { callee, args, .. } = &driver.e else {
        diags.push(diag(
            file,
            span,
            "NE0610",
            "this display driver".into(),
            None,
        ));
        return None;
    };
    let kind = flat(callee).unwrap_or_default();
    let (drv, format, addr) = match kind.as_str() {
        "ssd1306" => (Driver::Ssd1306, Format::Mono, 0x3c),
        "ili9341" => (Driver::Ili9341, Format::Rgb565, 0),
        "st7789" => (Driver::St7789, Format::Rgb565, 0),
        "hd44780" => (Driver::Hd44780, Format::Chars, 0x27),
        other => {
            diags.push(diag(
                file,
                span,
                "NE0610",
                format!("the display driver `{other}`"),
                Some("drivers: ssd1306, ili9341, st7789, hd44780".into()),
            ));
            return None;
        }
    };
    let mut d = UiDisplay {
        name: name.to_string(),
        driver: drv,
        bus: String::new(),
        addr,
        w: if format == Format::Chars { 16 } else { 128 },
        h: if format == Format::Chars { 2 } else { 64 },
        rotation: 0,
        format,
    };
    for a in args {
        match a.name.as_deref() {
            Some("bus") => d.bus = flat(&a.value).unwrap_or_default(),
            Some("size") => match pair(&a.value) {
                Some((w, h)) => {
                    d.w = w;
                    d.h = h;
                }
                None => diags.push(diag(
                    file,
                    span,
                    "NE0610",
                    "a `size` that is not a constant pair".into(),
                    None,
                )),
            },
            Some("rotation") => d.rotation = num(&a.value).unwrap_or(0.0) as i32,
            Some("addr" | "address") => d.addr = num(&a.value).unwrap_or(f64::from(addr)) as u8,
            _ => {}
        }
    }
    if format == Format::Mono && d.h % 8 != 0 {
        diags.push(diag(
            file,
            span,
            "NE0610",
            "an SSD1306 height that is not a multiple of 8".into(),
            None,
        ));
    }
    if d.w <= 0 || d.h <= 0 || d.w > 480 || d.h > 480 {
        diags.push(diag(
            file,
            span,
            "NE0610",
            format!("a display of {}x{}", d.w, d.h),
            None,
        ));
    }
    Some(d)
}

fn lower_widget(
    file: &str,
    w: &n::Widget,
    display: &UiDisplay,
    signals: &[(String, Option<n::Type>)],
    program: Option<&Program>,
    diags: &mut Vec<Diag>,
) -> Option<UiWidget> {
    let kind = match w.kind.as_str() {
        "label" => WidgetKind::Label,
        "gauge" => WidgetKind::Gauge,
        "bar" => WidgetKind::Bar,
        "slider" => WidgetKind::Slider,
        "led" => WidgetKind::Led,
        "button" => WidgetKind::Button,
        other => {
            diags.push(diag(
                file,
                w.span,
                "NE0610",
                format!("the widget `{other}`"),
                Some("widgets: label, gauge, bar, slider, led, button".into()),
            ));
            return None;
        }
    };
    if display.format == Format::Chars && !matches!(kind, WidgetKind::Label | WidgetKind::Led) {
        diags.push(diag(
            file,
            w.span,
            "NE0610",
            format!("a `{}` on a character display", w.kind),
            Some("character displays show labels and leds".into()),
        ));
        return None;
    }
    let mut u = UiWidget {
        kind,
        x: 0,
        y: 0,
        w: 0,
        h: 0,
        scale: 1,
        text: w.text.as_ref().map(|t| t.trim_matches('"').to_string()),
        bind: None,
        lo: 0.0,
        hi: 1.0,
    };
    let mut range_si: Option<(f64, f64)> = None;
    for o in &w.options {
        match o.name.as_str() {
            "at" => {
                if let Some((x, y)) = pair(&o.value) {
                    u.x = x;
                    u.y = y;
                }
            }
            "size" => {
                if let Some((sw, sh)) = pair(&o.value) {
                    u.w = sw;
                    u.h = sh;
                }
            }
            "font" => {
                let name = flat(&o.value).unwrap_or_default();
                let px: i32 = name
                    .trim_start_matches(|c: char| !c.is_ascii_digit())
                    .parse()
                    .unwrap_or(8);
                u.scale = (px / 8).max(1);
            }
            "range" => {
                if let n::ExprKind::Range { lo, hi, .. } = &o.value.e {
                    if let (Some(a), Some(b)) = (si(lo), si(hi)) {
                        range_si = Some((a, b));
                    }
                }
            }
            "bind" => {
                let Some(sig) = flat(&o.value) else {
                    diags.push(diag(
                        file,
                        w.span,
                        "NE0610",
                        "a `bind` that is not a signal".into(),
                        None,
                    ));
                    continue;
                };
                let vty = signals
                    .iter()
                    .find(|(n, _)| *n == sig)
                    .and_then(|(_, t)| t.clone());
                let (scale, offset) = unit_of(vty.as_ref());
                let is_bool = matches!(vty, Some(n::Type::Bool));
                let shared = program.and_then(|p| p.globals.iter().find(|g| g.source_name == sig));
                let (c_name, ty, own) = match shared {
                    Some(g) => (g.name.clone(), g.ty, false),
                    None => (
                        format!("ui_sig_{sig}"),
                        if is_bool { Ty::Bool } else { Ty::F32 },
                        true,
                    ),
                };
                u.bind = Some(Binding {
                    signal: sig,
                    c_name,
                    ty,
                    unit_scale: scale as f32,
                    unit_offset: offset as f32,
                    own,
                });
            }
            _ => {}
        }
    }
    if let Some((a, b)) = range_si {
        let (scale, offset) = u.bind.as_ref().map_or((1.0, 0.0), |b| {
            (f64::from(b.unit_scale), f64::from(b.unit_offset))
        });
        u.lo = ((a - offset) / scale) as f32;
        u.hi = ((b - offset) / scale) as f32;
    }
    match kind {
        WidgetKind::Bar if u.w == 0 => (u.w, u.h) = (20, 100),
        WidgetKind::Slider if u.w == 0 => (u.w, u.h) = (100, 14),
        _ => {}
    }
    if matches!(
        kind,
        WidgetKind::Gauge | WidgetKind::Bar | WidgetKind::Slider
    ) && u.bind.is_none()
    {
        diags.push(diag(
            file,
            w.span,
            "NE0610",
            format!("a `{}` without `bind`", w.kind),
            None,
        ));
    }
    Some(u)
}

fn c_float(v: f32) -> String {
    let s = format!("{v:?}");
    if s.contains('e') || s.contains('.') {
        format!("{s}f")
    } else {
        format!("{s}.0f")
    }
}

fn c_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if c.is_ascii() && !c.is_ascii_control() => o.push(c),
            _ => o.push('?'),
        }
    }
    o.push('"');
    o
}

fn ty_code(t: Ty) -> &'static str {
    match t {
        Ty::Bool => "NRT_UI_BOOL",
        Ty::U8 => "NRT_UI_U8",
        Ty::I8 => "NRT_UI_I8",
        Ty::U16 => "NRT_UI_U16",
        Ty::I16 => "NRT_UI_I16",
        Ty::U32 => "NRT_UI_U32",
        Ty::I32 => "NRT_UI_I32",
        Ty::F32 => "NRT_UI_F32",
    }
}

/// The C section of the ui program: the font, the widget table, the display description and
/// the refresh task `ui_task`. It is placed in the generated `app.c` after the globals.
pub fn emit_ui_c(app: &UiApp) -> String {
    let mut s = String::new();
    s.push_str("/* ---- ui (task B9): the display, its widgets and the refresh task ---- */\n");
    s.push_str("#include \"nrt_ui.h\"\n");
    s.push_str(&font::c_table());
    let mut own: Vec<&Binding> = Vec::new();
    for b in app.widgets.iter().filter_map(|w| w.bind.as_ref()) {
        if b.own && !own.iter().any(|o| o.c_name == b.c_name) {
            own.push(b);
        }
    }
    for b in &own {
        let t = if b.ty == Ty::Bool { "bool" } else { "float" };
        s.push_str(&format!("static volatile {t} {} = 0;\n", b.c_name));
    }
    s.push_str(&format!(
        "static uint8_t ui_buf[{}];\n",
        app.buffer_bytes.max(1)
    ));
    s.push_str(&format!(
        "static float ui_last[{}];\n",
        app.widgets.len().max(1)
    ));
    s.push_str(&format!(
        "static const nrt_ui_widget ui_widgets[{}] = {{\n",
        app.widgets.len().max(1)
    ));
    for w in &app.widgets {
        let kind = match w.kind {
            WidgetKind::Label => "NRT_UI_LABEL",
            WidgetKind::Gauge => "NRT_UI_GAUGE",
            WidgetKind::Bar => "NRT_UI_BAR",
            WidgetKind::Slider => "NRT_UI_SLIDER",
            WidgetKind::Led => "NRT_UI_LED",
            WidgetKind::Button => "NRT_UI_BUTTON",
        };
        let text = w.text.as_deref().map_or("NULL".to_string(), c_str);
        let (src, ty, scale, off) = match &w.bind {
            Some(b) => (
                format!("(const volatile void *)&{}", b.c_name),
                ty_code(b.ty),
                c_float(b.unit_scale),
                c_float(b.unit_offset),
            ),
            None => (
                "NULL".to_string(),
                "NRT_UI_NONE",
                "1.0f".to_string(),
                "0.0f".to_string(),
            ),
        };
        s.push_str(&format!(
            "  {{ {kind}, {}, {}, {}, {}, {}, {}, {}, {scale}, {off}, {text}, {src}, {ty} }},\n",
            w.x,
            w.y,
            w.w,
            w.h,
            w.scale,
            c_float(w.lo),
            c_float(w.hi)
        ));
    }
    if app.widgets.is_empty() {
        s.push_str(
            "  { NRT_UI_LABEL, 0, 0, 0, 0, 1, 0.0f, 1.0f, 1.0f, 0.0f, NULL, NULL, NRT_UI_NONE },\n",
        );
    }
    s.push_str("};\n");
    let driver = match app.display.driver {
        Driver::Ssd1306 => "NRT_UI_SSD1306",
        Driver::Ili9341 => "NRT_UI_ILI9341",
        Driver::St7789 => "NRT_UI_ST7789",
        Driver::Hd44780 => "NRT_UI_HD44780",
    };
    s.push_str(&format!(
        "static const nrt_ui_display ui_display = {{ {driver}, 0x{:02x}, {}, {}, {}, {}, ui_buf, {}, ui_widgets, ui_last }};\n",
        app.display.addr,
        app.display.w,
        app.display.h,
        app.display.rotation,
        app.band_rows,
        app.widgets.len()
    ));
    s.push_str("static void ui_task(void) { nrt_ui_refresh(&ui_display); }\n\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_scales_are_read_exactly() {
        assert!((exact("1/1000") - 0.001).abs() < 1e-12);
        assert!((exact("273.15") - 273.15).abs() < 1e-12);
    }
}
