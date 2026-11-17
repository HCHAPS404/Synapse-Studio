/* neuron-rt ui renderer and display drivers (task B9).
 *
 * The drawing rules equal the reference renderer of the compiler (crates/neuron-ui,
 * render.rs): same integer coordinates, same single-precision arithmetic, so the image that
 * reaches the panel is the one the tests compute. Display command sets: SSD1306, ILI9341,
 * ST7789 and HD44780 public datasheets.
 */
#include "nrt_ui.h"

static const nrt_ui_display *g_ui;
static int16_t g_y0;
static int16_t g_rows;
static bool g_ui_ready;

static bool is_tft(const nrt_ui_display *d) { return d->driver == NRT_UI_ILI9341 || d->driver == NRT_UI_ST7789; }

/* ---- drawing ------------------------------------------------------------------------ */

static void ui_set(int16_t x, int16_t y, uint16_t c) {
  const nrt_ui_display *d = g_ui;
  if (x < 0 || x >= d->w || y < g_y0 || y >= g_y0 + g_rows || y >= d->h) {
    return;
  }
  int16_t ry = (int16_t)(y - g_y0);
  if (d->driver == NRT_UI_SSD1306) {
    uint8_t *b = &d->buf[(uint16_t)(ry / 8) * (uint16_t)d->w + (uint16_t)x];
    uint8_t m = (uint8_t)(1u << (ry % 8));
    *b = c ? (uint8_t)(*b | m) : (uint8_t)(*b & ~m);
  } else if (is_tft(d)) {
    uint32_t i = ((uint32_t)ry * (uint32_t)d->w + (uint32_t)x) * 2u;
    d->buf[i] = (uint8_t)(c >> 8);
    d->buf[i + 1u] = (uint8_t)(c & 0xffu);
  } else {
    d->buf[(uint16_t)y * (uint16_t)d->w + (uint16_t)x] = (uint8_t)c;
  }
}

static void ui_fill(int16_t x, int16_t y, int16_t w, int16_t h, uint16_t c) {
  for (int16_t yy = y; yy < y + h; yy++) {
    if (yy < g_y0 || yy >= g_y0 + g_rows) {
      continue;
    }
    for (int16_t xx = x; xx < x + w; xx++) {
      ui_set(xx, yy, c);
    }
  }
}

static void ui_frame(int16_t x, int16_t y, int16_t w, int16_t h, uint16_t c) {
  ui_fill(x, y, w, 1, c);
  ui_fill(x, (int16_t)(y + h - 1), w, 1, c);
  ui_fill(x, y, 1, h, c);
  ui_fill((int16_t)(x + w - 1), y, 1, h, c);
}

static uint8_t ui_glyph(char ch) {
  if (ch >= 'a' && ch <= 'z') {
    ch = (char)(ch - 32);
  }
  for (uint8_t i = 0; nrt_ui_chars[i] != '\0'; i++) {
    if (nrt_ui_chars[i] == ch) {
      return i;
    }
  }
  return 44; /* '?' */
}

static void ui_text(int16_t x, int16_t y, int16_t scale, const char *s, uint16_t c) {
  const nrt_ui_display *d = g_ui;
  for (int16_t i = 0; s[i] != '\0'; i++) {
    uint8_t g = ui_glyph(s[i]);
    if (d->driver == NRT_UI_HD44780) {
      ui_set((int16_t)(x + i), y, (uint8_t)nrt_ui_chars[g]);
      continue;
    }
    int16_t gx = (int16_t)(x + i * 6 * scale);
    for (int16_t r = 0; r < 7; r++) {
      uint8_t row = nrt_ui_font[g][r];
      for (int16_t col = 0; col < 5; col++) {
        if (row & (0x10u >> col)) {
          ui_fill((int16_t)(gx + col * scale), (int16_t)(y + r * scale), scale, scale, c);
        }
      }
    }
  }
}

/* One decimal, as the reference renderer prints it. */
static void ui_fmt1(float v, char *out) {
  bool neg = v < 0.0f;
  float a = neg ? -v : v;
  int32_t t = (int32_t)(a * 10.0f + 0.5f);
  int32_t whole = t / 10;
  char tmp[12];
  uint8_t n = 0;
  do {
    tmp[n++] = (char)('0' + whole % 10);
    whole /= 10;
  } while (whole != 0 && n < 10);
  uint8_t k = 0;
  if (neg) {
    out[k++] = '-';
  }
  while (n > 0) {
    out[k++] = tmp[--n];
  }
  out[k++] = '.';
  out[k++] = (char)('0' + t % 10);
  out[k] = '\0';
}

static float ui_raw(const nrt_ui_widget *w) {
  if (w->src == NULL) {
    return 0.0f;
  }
  switch (w->ty) {
    case NRT_UI_BOOL: return *(const volatile bool *)w->src ? 1.0f : 0.0f;
    case NRT_UI_U8: return (float)*(const volatile uint8_t *)w->src;
    case NRT_UI_I8: return (float)*(const volatile int8_t *)w->src;
    case NRT_UI_U16: return (float)*(const volatile uint16_t *)w->src;
    case NRT_UI_I16: return (float)*(const volatile int16_t *)w->src;
    case NRT_UI_U32: return (float)*(const volatile uint32_t *)w->src;
    case NRT_UI_I32: return (float)*(const volatile int32_t *)w->src;
    case NRT_UI_F32: return *(const volatile float *)w->src;
    default: return 0.0f;
  }
}

static float ui_frac(float v, float lo, float hi) {
  if (hi == lo) {
    return 0.0f;
  }
  float f = (v - lo) / (hi - lo);
  return f < 0.0f ? 0.0f : (f > 1.0f ? 1.0f : f);
}

static int16_t ui_part(float f, int16_t n) { return (int16_t)(f * (float)n + 0.5f); }

static uint8_t ui_len(const char *s) {
  uint8_t n = 0;
  while (s[n] != '\0') {
    n++;
  }
  return n;
}

static void ui_widget(const nrt_ui_widget *w, uint16_t fg, uint16_t accent) {
  bool is_bool = w->ty == NRT_UI_BOOL;
  float raw = ui_raw(w);
  float v = is_bool ? raw : (w->src != NULL ? (raw - w->uoff) / w->uscale : 0.0f);
  char num[16];
  const char *value = num;
  if (is_bool) {
    value = v != 0.0f ? "ON" : "OFF";
  } else {
    ui_fmt1(v, num);
  }
  const char *text = w->text != NULL ? w->text : value;
  if (g_ui->driver == NRT_UI_HD44780) {
    ui_text(w->x, w->y, 1, w->kind == NRT_UI_LED ? (v > 0.5f ? "+" : "O") : text, 0);
    return;
  }
  float f = ui_frac(v, w->lo, w->hi);
  switch (w->kind) {
    case NRT_UI_LABEL:
      ui_text(w->x, w->y, w->scale, text, fg);
      break;
    case NRT_UI_GAUGE:
      ui_frame(w->x, w->y, 100, 14, fg);
      ui_fill((int16_t)(w->x + 2), (int16_t)(w->y + 2), ui_part(f, 96), 10, accent);
      ui_text(w->x, (int16_t)(w->y + 18), 1, value, fg);
      break;
    case NRT_UI_BAR: {
      ui_frame(w->x, w->y, w->w, w->h, fg);
      int16_t fh = ui_part(f, (int16_t)(w->h - 4));
      ui_fill((int16_t)(w->x + 2), (int16_t)(w->y + 2 + (w->h - 4 - fh)), (int16_t)(w->w - 4), fh, accent);
      break;
    }
    case NRT_UI_SLIDER: {
      ui_frame(w->x, w->y, w->w, w->h, fg);
      int16_t kx = (int16_t)(w->x + 2 + ui_part(f, (int16_t)(w->w - 8)));
      ui_fill(kx, (int16_t)(w->y + 2), 4, (int16_t)(w->h - 4), fg);
      break;
    }
    case NRT_UI_LED:
      ui_frame(w->x, w->y, 10, 10, fg);
      if (v > 0.5f) {
        ui_fill((int16_t)(w->x + 2), (int16_t)(w->y + 2), 6, 6, accent);
      }
      break;
    case NRT_UI_BUTTON: {
      const char *t = w->text != NULL ? w->text : "";
      ui_frame(w->x, w->y, (int16_t)(6 * ui_len(t) + 6), 14, fg);
      ui_text((int16_t)(w->x + 4), (int16_t)(w->y + 4), 1, t, fg);
      break;
    }
    default:
      break;
  }
}

/* ---- drivers --------------------------------------------------------------------------- */

static void ssd_cmds(const uint8_t *c, uint16_t n) {
  uint8_t buf[17];
  buf[0] = 0x00;
  for (uint16_t i = 0; i < n && i < 16u; i++) {
    buf[i + 1u] = c[i];
  }
  nrt_hal_i2c_write(g_ui->addr, buf, (uint16_t)(n + 1u));
}

static void tft_cmd(uint8_t c, const uint8_t *data, uint16_t n) {
  nrt_hal_spi_write(false, &c, 1);
  if (n != 0u) {
    nrt_hal_spi_write(true, data, n);
  }
}

static void lcd_nibble(uint8_t nib, bool rs) {
  uint8_t b = (uint8_t)((nib << 4) | 0x08u | (rs ? 0x01u : 0x00u));
  uint8_t pulse[2] = {(uint8_t)(b | 0x04u), b};
  nrt_hal_i2c_write(g_ui->addr, pulse, 2);
}

static void lcd_byte(uint8_t v, bool rs) {
  lcd_nibble((uint8_t)(v >> 4), rs);
  lcd_nibble((uint8_t)(v & 0x0fu), rs);
}

static void ui_init(const nrt_ui_display *d) {
  nrt_hal_bus_init(d->driver);
  if (d->driver == NRT_UI_SSD1306) {
    const uint8_t init[] = {0xae, 0xd5, 0x80, 0xa8, (uint8_t)(d->h - 1), 0xd3, 0x00, 0x40, 0x8d, 0x14, 0x20, 0x00, 0xa1, 0xc8};
    const uint8_t init2[] = {0xda, (uint8_t)(d->h == 32 ? 0x02 : 0x12), 0x81, 0xcf, 0xd9, 0xf1, 0xdb, 0x40, 0xa4, 0xa6, 0xaf};
    ssd_cmds(init, sizeof init);
    ssd_cmds(init2, sizeof init2);
  } else if (is_tft(d)) {
    uint8_t madctl;
    int16_t r = d->rotation;
    if (d->driver == NRT_UI_ILI9341) {
      madctl = r == 90 ? 0x28 : (r == 180 ? 0x88 : (r == 270 ? 0xe8 : 0x48));
    } else {
      madctl = r == 90 ? 0x60 : (r == 180 ? 0xc0 : (r == 270 ? 0xa0 : 0x00));
    }
    const uint8_t colmod = 0x55; /* 16 bits per pixel */
    tft_cmd(0x01, NULL, 0); /* software reset */
    nrt_hal_delay_ms(120);
    tft_cmd(0x11, NULL, 0); /* sleep out */
    nrt_hal_delay_ms(120);
    tft_cmd(0x3a, &colmod, 1);
    tft_cmd(0x36, &madctl, 1);
    tft_cmd(0x29, NULL, 0); /* display on */
  } else {
    nrt_hal_delay_ms(50);
    lcd_nibble(0x3, false);
    nrt_hal_delay_ms(5);
    lcd_nibble(0x3, false);
    lcd_nibble(0x3, false);
    lcd_nibble(0x2, false); /* 4-bit mode */
    lcd_byte(0x28, false);  /* 2 lines, 5x8 */
    lcd_byte(0x0c, false);  /* display on, no cursor */
    lcd_byte(0x06, false);  /* entry mode: increment */
    lcd_byte(0x01, false);  /* clear */
    nrt_hal_delay_ms(2);
  }
}

static void ui_flush(const nrt_ui_display *d) {
  if (d->driver == NRT_UI_SSD1306) {
    uint8_t win[] = {0x21, 0x00, (uint8_t)(d->w - 1), 0x22, (uint8_t)(g_y0 / 8), (uint8_t)((g_y0 + g_rows) / 8 - 1)};
    ssd_cmds(win, sizeof win);
    uint32_t total = (uint32_t)d->w * (uint32_t)(g_rows / 8);
    uint8_t chunk[17];
    chunk[0] = 0x40;
    for (uint32_t i = 0; i < total; i += 16u) {
      uint32_t n = total - i < 16u ? total - i : 16u;
      for (uint32_t k = 0; k < n; k++) {
        chunk[k + 1u] = d->buf[i + k];
      }
      nrt_hal_i2c_write(d->addr, chunk, (uint16_t)(n + 1u));
    }
  } else if (is_tft(d)) {
    uint16_t x1 = (uint16_t)(d->w - 1);
    uint16_t y0 = (uint16_t)g_y0;
    uint16_t y1 = (uint16_t)(g_y0 + g_rows - 1);
    uint8_t ca[4] = {0, 0, (uint8_t)(x1 >> 8), (uint8_t)(x1 & 0xffu)};
    uint8_t ra[4] = {(uint8_t)(y0 >> 8), (uint8_t)(y0 & 0xffu), (uint8_t)(y1 >> 8), (uint8_t)(y1 & 0xffu)};
    tft_cmd(0x2a, ca, 4);
    tft_cmd(0x2b, ra, 4);
    tft_cmd(0x2c, NULL, 0);
    uint32_t total = (uint32_t)d->w * (uint32_t)g_rows * 2u;
    for (uint32_t i = 0; i < total; i += 64u) {
      uint32_t n = total - i < 64u ? total - i : 64u;
      nrt_hal_spi_write(true, &d->buf[i], (uint16_t)n);
    }
  } else {
    static const uint8_t base[4] = {0x00, 0x40, 0x14, 0x54};
    for (int16_t r = 0; r < d->h && r < 4; r++) {
      lcd_byte((uint8_t)(0x80u | base[r]), false);
      for (int16_t c = 0; c < d->w; c++) {
        lcd_byte(d->buf[(uint16_t)r * (uint16_t)d->w + (uint16_t)c], true);
      }
    }
  }
}

void nrt_ui_refresh(const nrt_ui_display *d) {
  g_ui = d;
  bool changed = !g_ui_ready;
  for (uint8_t i = 0; i < d->n; i++) {
    float r = ui_raw(&d->widgets[i]);
    if (r != d->last[i]) {
      changed = true;
    }
    d->last[i] = r;
  }
  if (!changed) {
    return;
  }
  if (!g_ui_ready) {
    ui_init(d);
    g_ui_ready = true;
  }
  uint16_t fg = is_tft(d) ? 0xffffu : 1u;
  uint16_t accent = is_tft(d) ? 0x07e0u : 1u;
  int16_t rows = d->driver == NRT_UI_HD44780 ? d->h : d->band_rows;
  for (int16_t y0 = 0; y0 < d->h; y0 = (int16_t)(y0 + rows)) {
    g_y0 = y0;
    g_rows = rows;
    uint32_t bytes;
    uint8_t clear = 0;
    if (d->driver == NRT_UI_SSD1306) {
      bytes = (uint32_t)d->w * (uint32_t)(rows / 8);
    } else if (is_tft(d)) {
      bytes = (uint32_t)d->w * (uint32_t)rows * 2u;
    } else {
      bytes = (uint32_t)d->w * (uint32_t)d->h;
      clear = (uint8_t)' ';
    }
    for (uint32_t i = 0; i < bytes; i++) {
      d->buf[i] = clear;
    }
    for (uint8_t i = 0; i < d->n; i++) {
      ui_widget(&d->widgets[i], fg, accent);
    }
    ui_flush(d);
  }
}
