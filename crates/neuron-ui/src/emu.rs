//! Display models: they replay the bus traffic of a firmware (the `BUS` lines that the host
//! runtime prints) and rebuild the image the panel would show. The digital twin uses the same
//! models for its virtual displays.
//!
//! Trace lines: `BUS i2c 3c: 00 ae d5 ...` (one I2C write to address 0x3c) and
//! `BUS spi c: 2a` / `BUS spi d: 00 00 01 3f` (an SPI command and its data).

use crate::{Driver, Format, Image, UiApp};

/// The SSD1306 command bytes that take arguments, with their count.
fn ssd_args(cmd: u8) -> usize {
    match cmd {
        0x21 | 0x22 => 2,
        0x20 | 0x81 | 0x8d | 0xa8 | 0xd3 | 0xd5 | 0xd9 | 0xda | 0xdb => 1,
        _ => 0,
    }
}

/// The SSD1306 memory and its address window.
#[derive(Debug, Clone)]
pub struct Ssd1306 {
    w: usize,
    pages: usize,
    mem: Vec<u8>,
    col: (usize, usize),
    page: (usize, usize),
    at: (usize, usize),
}

impl Ssd1306 {
    /// A blank panel of `w` x `h` pixels.
    pub fn new(w: usize, h: usize) -> Ssd1306 {
        let pages = h / 8;
        Ssd1306 {
            w,
            pages,
            mem: vec![0; w * pages],
            col: (0, w.saturating_sub(1)),
            page: (0, pages.saturating_sub(1)),
            at: (0, 0),
        }
    }

    /// One I2C write: a control byte, then commands (0x00) or display data (0x40).
    pub fn write(&mut self, bytes: &[u8]) {
        let Some((&ctrl, rest)) = bytes.split_first() else {
            return;
        };
        if ctrl == 0x40 {
            for &b in rest {
                let (c, p) = self.at;
                if c < self.w && p < self.pages {
                    self.mem[p * self.w + c] = b;
                }
                let mut c = c + 1;
                let mut p = p;
                if c > self.col.1 {
                    c = self.col.0;
                    p = if p >= self.page.1 { self.page.0 } else { p + 1 };
                }
                self.at = (c, p);
            }
            return;
        }
        let mut i = 0;
        while i < rest.len() {
            let cmd = rest[i];
            let n = ssd_args(cmd);
            let args = &rest[(i + 1).min(rest.len())..(i + 1 + n).min(rest.len())];
            match (cmd, args) {
                (0x21, [a, b]) => {
                    self.col = (usize::from(*a), usize::from(*b));
                    self.at.0 = usize::from(*a);
                }
                (0x22, [a, b]) => {
                    self.page = (usize::from(*a), usize::from(*b));
                    self.at.1 = usize::from(*a);
                }
                _ => {}
            }
            i += 1 + n;
        }
    }

    /// The panel as an image.
    pub fn image(&self) -> Image {
        let (w, h) = (self.w as i32, (self.pages * 8) as i32);
        let mut px = vec![0u16; (w * h) as usize];
        for y in 0..h as usize {
            for x in 0..self.w {
                let bit = (self.mem[(y / 8) * self.w + x] >> (y % 8)) & 1;
                px[y * self.w + x] = u16::from(bit);
            }
        }
        Image {
            w,
            h,
            format: Format::Mono,
            px,
        }
    }
}

/// An ILI9341 or ST7789 panel: column and row windows, memory write in RGB565.
#[derive(Debug, Clone)]
pub struct Tft {
    w: usize,
    h: usize,
    px: Vec<u16>,
    cmd: u8,
    args: Vec<u8>,
    win: (usize, usize, usize, usize),
    at: (usize, usize),
    half: Option<u8>,
}

impl Tft {
    /// A blank panel.
    pub fn new(w: usize, h: usize) -> Tft {
        Tft {
            w,
            h,
            px: vec![0; w * h],
            cmd: 0,
            args: Vec::new(),
            win: (0, w.saturating_sub(1), 0, h.saturating_sub(1)),
            at: (0, 0),
            half: None,
        }
    }

    /// A command byte (D/C low).
    pub fn command(&mut self, c: u8) {
        self.cmd = c;
        self.args.clear();
        self.half = None;
        if c == 0x2c {
            self.at = (self.win.0, self.win.2);
        }
    }

    /// Data bytes (D/C high).
    pub fn data(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.cmd {
                0x2a | 0x2b => {
                    self.args.push(b);
                    if self.args.len() == 4 {
                        let a = (usize::from(self.args[0]) << 8) | usize::from(self.args[1]);
                        let z = (usize::from(self.args[2]) << 8) | usize::from(self.args[3]);
                        if self.cmd == 0x2a {
                            self.win.0 = a;
                            self.win.1 = z;
                        } else {
                            self.win.2 = a;
                            self.win.3 = z;
                        }
                    }
                }
                0x2c => match self.half.take() {
                    None => self.half = Some(b),
                    Some(hi) => {
                        let (x, y) = self.at;
                        if x < self.w && y < self.h {
                            self.px[y * self.w + x] = (u16::from(hi) << 8) | u16::from(b);
                        }
                        let mut x = x + 1;
                        let mut y = y;
                        if x > self.win.1 {
                            x = self.win.0;
                            y += 1;
                        }
                        self.at = (x, y);
                    }
                },
                _ => {}
            }
        }
    }

    /// The panel as an image.
    pub fn image(&self) -> Image {
        Image {
            w: self.w as i32,
            h: self.h as i32,
            format: Format::Rgb565,
            px: self.px.clone(),
        }
    }
}

/// An HD44780 character LCD behind a PCF8574 backpack (P0 RS, P2 EN, P4..P7 D4..D7).
#[derive(Debug, Clone)]
pub struct Hd44780 {
    cols: usize,
    rows: usize,
    ddram: [u8; 128],
    addr: usize,
    eight_bit: bool,
    high: Option<u8>,
    last: u8,
}

impl Hd44780 {
    /// A blank display.
    pub fn new(cols: usize, rows: usize) -> Hd44780 {
        Hd44780 {
            cols,
            rows,
            ddram: [b' '; 128],
            addr: 0,
            eight_bit: true,
            high: None,
            last: 0,
        }
    }

    fn byte(&mut self, rs: bool, b: u8) {
        if rs {
            self.ddram[self.addr & 0x7f] = b;
            self.addr = (self.addr + 1) & 0x7f;
        } else if b & 0x80 != 0 {
            self.addr = usize::from(b & 0x7f);
        } else if b == 0x01 {
            self.ddram = [b' '; 128];
            self.addr = 0;
        }
    }

    /// One byte written to the backpack.
    pub fn write(&mut self, v: u8) {
        let falling = self.last & 0x04 != 0 && v & 0x04 == 0;
        if falling {
            let nibble = self.last >> 4;
            let rs = self.last & 0x01 != 0;
            if self.eight_bit {
                if nibble == 0x2 {
                    self.eight_bit = false;
                }
            } else {
                match self.high.take() {
                    None => self.high = Some(nibble),
                    Some(hi) => self.byte(rs, (hi << 4) | nibble),
                }
            }
        }
        self.last = v;
    }

    /// The display as a character image.
    pub fn image(&self) -> Image {
        let base = [0x00usize, 0x40, 0x14, 0x54];
        let mut px = Vec::with_capacity(self.cols * self.rows);
        for r in 0..self.rows {
            for c in 0..self.cols {
                px.push(u16::from(self.ddram[(base[r.min(3)] + c) & 0x7f]));
            }
        }
        Image {
            w: self.cols as i32,
            h: self.rows as i32,
            format: Format::Chars,
            px,
        }
    }
}

fn hex_bytes(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .filter_map(|t| u8::from_str_radix(t, 16).ok())
        .collect()
}

/// Replays the `BUS` lines of a host trace into the display of `app`.
pub fn replay(trace: &str, app: &UiApp) -> Image {
    let (w, h) = (app.display.w.max(1) as usize, app.display.h.max(1) as usize);
    let mut ssd = Ssd1306::new(w, h);
    let mut tft = Tft::new(w, h);
    let mut lcd = Hd44780::new(w, h);
    for line in trace.lines() {
        let Some((_, rest)) = line.split_once(" BUS ") else {
            continue;
        };
        let rest = rest.trim();
        let Some((head, bytes)) = rest.split_once(':') else {
            continue;
        };
        let bytes = hex_bytes(bytes);
        let mut it = head.split_whitespace();
        match (it.next(), it.next()) {
            (Some("i2c"), Some(_addr)) => match app.display.driver {
                Driver::Hd44780 => bytes.iter().for_each(|b| lcd.write(*b)),
                _ => ssd.write(&bytes),
            },
            (Some("spi"), Some("c")) => bytes.iter().for_each(|b| tft.command(*b)),
            (Some("spi"), Some("d")) => tft.data(&bytes),
            _ => {}
        }
    }
    match app.display.driver {
        Driver::Ssd1306 => ssd.image(),
        Driver::Ili9341 | Driver::St7789 => tft.image(),
        Driver::Hd44780 => lcd.image(),
    }
}
