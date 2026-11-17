//! The reference renderer. `runtime/neuron-rt/ui/nrt_ui.c` draws with the same integer and
//! single-precision arithmetic, so the firmware image equals this one pixel for pixel.

use crate::{font, Format, UiApp, UiWidget, WidgetKind};
use std::collections::BTreeMap;

/// A rendered screen: pixels row by row (0 or 1 for monochrome, RGB565 for color, the
/// character code for character displays).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
    /// Pixel format.
    pub format: Format,
    /// `w * h` values.
    pub px: Vec<u16>,
}

impl Image {
    fn new(w: i32, h: i32, format: Format) -> Image {
        let fill = if format == Format::Chars {
            u16::from(b' ')
        } else {
            0
        };
        Image {
            w,
            h,
            format,
            px: vec![fill; (w.max(0) * h.max(0)) as usize],
        }
    }

    /// The pixel at `(x, y)`.
    pub fn at(&self, x: i32, y: i32) -> u16 {
        self.px[(y * self.w + x) as usize]
    }

    fn set(&mut self, x: i32, y: i32, c: u16) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.px[(y * self.w + x) as usize] = c;
        }
    }

    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: u16) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, c);
            }
        }
    }

    fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, c: u16) {
        self.fill(x, y, w, 1, c);
        self.fill(x, y + h - 1, w, 1, c);
        self.fill(x, y, 1, h, c);
        self.fill(x + w - 1, y, 1, h, c);
    }

    fn text(&mut self, x: i32, y: i32, scale: i32, s: &str, c: u16) {
        if self.format == Format::Chars {
            for (i, ch) in s.chars().enumerate() {
                self.set(
                    x + i as i32,
                    y,
                    u16::from(font::CHARS.as_bytes()[font::index(ch)]),
                );
            }
            return;
        }
        for (i, ch) in s.chars().enumerate() {
            let g = font::GLYPHS[font::index(ch)];
            let gx = x + i as i32 * 6 * scale;
            for (r, row) in g.iter().enumerate() {
                for col in 0..5 {
                    if row & (0x10 >> col) != 0 {
                        self.fill(gx + col * scale, y + r as i32 * scale, scale, scale, c);
                    }
                }
            }
        }
    }

    /// The image as text: `#` for a lit pixel, `.` otherwise (characters as they are).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for y in 0..self.h {
            for x in 0..self.w {
                let v = self.at(x, y);
                s.push(match self.format {
                    Format::Chars => char::from(v as u8),
                    _ if v == 0 => '.',
                    _ => '#',
                });
            }
            s.push('\n');
        }
        s
    }
}

/// One decimal, as the firmware prints it.
pub(crate) fn fmt1(v: f32) -> String {
    let neg = v < 0.0;
    let a = if neg { -v } else { v };
    let t = (a * 10.0_f32 + 0.5_f32) as i32;
    format!("{}{}.{}", if neg { "-" } else { "" }, t / 10, t % 10)
}

fn frac(v: f32, lo: f32, hi: f32) -> f32 {
    if hi == lo {
        return 0.0;
    }
    let f = (v - lo) / (hi - lo);
    f.clamp(0.0, 1.0)
}

fn part(f: f32, n: i32) -> i32 {
    (f * n as f32 + 0.5_f32) as i32
}

/// The screen with the given stored values of the signals (in base SI units, keyed by the
/// signal name; a missing signal is 0).
pub fn render(app: &UiApp, values: &BTreeMap<String, f32>) -> Image {
    let d = &app.display;
    let mut img = Image::new(d.w, d.h, d.format);
    let (fg, accent) = match d.format {
        Format::Mono => (1, 1),
        Format::Rgb565 => (0xffff, 0x07e0),
        Format::Chars => (0, 0),
    };
    for w in &app.widgets {
        draw(&mut img, w, values, fg, accent);
    }
    img
}

fn draw(img: &mut Image, w: &UiWidget, values: &BTreeMap<String, f32>, fg: u16, accent: u16) {
    let (v, is_bool) = match &w.bind {
        Some(b) => {
            let raw = values.get(&b.signal).copied().unwrap_or(0.0);
            if b.ty == neuron_mir::Ty::Bool {
                (if raw != 0.0 { 1.0 } else { 0.0 }, true)
            } else {
                ((raw - b.unit_offset) / b.unit_scale, false)
            }
        }
        None => (0.0, false),
    };
    let value_text = || {
        if is_bool {
            (if v != 0.0 { "ON" } else { "OFF" }).to_string()
        } else {
            fmt1(v)
        }
    };
    if img.format == Format::Chars {
        match w.kind {
            WidgetKind::Led => img.text(w.x, w.y, 1, if v > 0.5 { "+" } else { "O" }, 0),
            _ => {
                let t = w.text.clone().unwrap_or_else(value_text);
                img.text(w.x, w.y, 1, &t, 0);
            }
        }
        return;
    }
    let f = frac(v, w.lo, w.hi);
    match w.kind {
        WidgetKind::Label => {
            let t = w.text.clone().unwrap_or_else(value_text);
            img.text(w.x, w.y, w.scale, &t, fg);
        }
        WidgetKind::Gauge => {
            img.frame(w.x, w.y, 100, 14, fg);
            img.fill(w.x + 2, w.y + 2, part(f, 96), 10, accent);
            img.text(w.x, w.y + 18, 1, &value_text(), fg);
        }
        WidgetKind::Bar => {
            img.frame(w.x, w.y, w.w, w.h, fg);
            let fh = part(f, w.h - 4);
            img.fill(w.x + 2, w.y + 2 + (w.h - 4 - fh), w.w - 4, fh, accent);
        }
        WidgetKind::Slider => {
            img.frame(w.x, w.y, w.w, w.h, fg);
            let kx = w.x + 2 + part(f, w.w - 8);
            img.fill(kx, w.y + 2, 4, w.h - 4, fg);
        }
        WidgetKind::Led => {
            img.frame(w.x, w.y, 10, 10, fg);
            if v > 0.5 {
                img.fill(w.x + 2, w.y + 2, 6, 6, accent);
            }
        }
        WidgetKind::Button => {
            let t = w.text.clone().unwrap_or_default();
            img.frame(w.x, w.y, 6 * t.chars().count() as i32 + 6, 14, fg);
            img.text(w.x + 4, w.y + 4, 1, &t, fg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fmt1;

    #[test]
    fn values_have_one_decimal() {
        assert_eq!(fmt1(25.0), "25.0");
        assert_eq!(fmt1(-3.26), "-3.3");
        assert_eq!(fmt1(0.04), "0.0");
    }
}
