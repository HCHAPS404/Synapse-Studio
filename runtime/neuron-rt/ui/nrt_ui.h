/* neuron-rt ui (task B9): retained-mode widgets on small displays.
 *
 * The generated program describes its display and widgets with these tables and calls
 * nrt_ui_refresh() from a periodic task. The renderer draws into a framebuffer or into bands
 * of 8 rows (chosen by the compiler from the RAM), then sends them with the display driver.
 * The bus functions at the end are implemented by each port (port/<name>/nrt_bus_<name>.c).
 */
#ifndef NRT_UI_H
#define NRT_UI_H

#include "nrt.h"

enum { NRT_UI_LABEL = 1, NRT_UI_GAUGE, NRT_UI_BAR, NRT_UI_SLIDER, NRT_UI_LED, NRT_UI_BUTTON };
enum { NRT_UI_NONE = 0, NRT_UI_BOOL, NRT_UI_U8, NRT_UI_I8, NRT_UI_U16, NRT_UI_I16, NRT_UI_U32, NRT_UI_I32, NRT_UI_F32 };
enum { NRT_UI_SSD1306 = 1, NRT_UI_ILI9341, NRT_UI_ST7789, NRT_UI_HD44780 };

typedef struct nrt_ui_widget {
  uint8_t kind;
  int16_t x, y, w, h;
  uint8_t scale;
  float lo, hi;          /* range, in display units */
  float uscale, uoff;    /* display value = (stored - uoff) / uscale */
  const char *text;      /* fixed text, or NULL */
  const volatile void *src; /* the bound variable, or NULL */
  uint8_t ty;            /* NRT_UI_BOOL ... NRT_UI_F32 */
} nrt_ui_widget;

typedef struct nrt_ui_display {
  uint8_t driver;
  uint8_t addr;          /* I2C address */
  int16_t w, h;          /* pixels (characters for HD44780) */
  int16_t rotation;      /* degrees */
  int16_t band_rows;     /* rows drawn at a time (the height for a framebuffer) */
  uint8_t *buf;
  uint8_t n;
  const nrt_ui_widget *widgets;
  float *last;           /* last drawn value of each widget */
} nrt_ui_display;

/* The font (5x7 glyphs, written into the generated program). */
extern const uint8_t nrt_ui_font[45][7];
extern const char nrt_ui_chars[];

/* Draws the screen again when a bound value changed (always the first time). */
void nrt_ui_refresh(const nrt_ui_display *d);

/* ---- bus, per port ---- */
void nrt_hal_bus_init(uint8_t driver);
void nrt_hal_i2c_write(uint8_t addr, const uint8_t *data, uint16_t n);
void nrt_hal_spi_write(bool is_data, const uint8_t *data, uint16_t n);
void nrt_hal_delay_ms(uint16_t ms);

#endif /* NRT_UI_H */
