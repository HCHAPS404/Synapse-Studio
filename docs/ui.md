# The `ui` dialect on small displays (task B9)

A `ui` module describes one `display` and its `screen`s. `crates/neuron-ui` lays the widgets
out, chooses the render strategy from the RAM of the chip, and writes the C tables that the
runtime renderer (`runtime/neuron-rt/ui/nrt_ui.c`) draws from. `neuron build` adds them to
the firmware when the package has a `ui` module.

```
module screen for ui on arduino_uno_r3
display oled: ssd1306(bus: i2c1, size: (128, 64), rotation: 0 deg)
signal temp: f32 degC

screen Main on oled:
  label title "Oven" at (0, 0) font sans_16
  label value bind temp at (0, 20)
```

## Signals

A widget binds a `signal` of the ui module. When an `mcu` module of the same package declares
a signal with the same name, they are the same signal: the logic writes it, the screen shows
it. A ui signal that no mcu module declares is a variable of the screen alone (it stays 0).
This name rule stands until signals can be imported across modules with `use`.

## Displays

| Driver | Bus | Pixels | Notes |
|---|---|---|---|
| `ssd1306` | I2C (address 0x3C, `addr:` to change) | monochrome, 128x64 or 128x32 | page layout, 1 KiB for a full frame |
| `ili9341` | SPI | RGB565 | rotation through MADCTL |
| `st7789` | SPI | RGB565 | same commands for what the renderer uses |
| `hd44780` | I2C backpack (PCF8574, address 0x27) | characters (`size: (16, 2)`) | `at (column, row)`; labels and leds |

Bus pins: on the UNO, I2C uses A4/A5 and SPI uses D11 (MOSI), D13 (SCK), D10 (chip select)
and D9 (data/command). On the ESP32-S3 port: I2C SDA GPIO8, SCL GPIO9; SPI MOSI GPIO11,
SCLK GPIO12, CS GPIO10, D/C GPIO13. The bare-metal Arm ports have no display bus yet
(`NE0610`).

## Widgets

| Widget | Drawing |
|---|---|
| `label` | the text, or the bound value with one decimal (`ON`/`OFF` for a `bool`); `font sans_16` doubles the 6x8 cell |
| `gauge` | a 100x14 frame, a filled part for the value in its `range`, the value under it |
| `bar` | a vertical frame (`size`, default 20x100) filled from the bottom |
| `slider` | a horizontal frame (`size`, default 100x14) with a knob |
| `led` | a 10x10 frame, filled when the value is true (or above 0.5) |
| `button` | its caption in a frame |

Values are shown in the unit of the ui signal: a `degC` signal is stored in kelvin and shown
in degrees Celsius. Other widgets of spec 09 §9.7 (`chart`, `list`, `image`, `keypad`,
`numeric_input`, `toggle`, layout containers) are `NE0610` for now. Touch input and the
`on press` handlers are not wired yet: buttons are drawn, and the first screen is the one
shown.

## Render strategy (MEM-1)

| Strategy | Chosen when | Buffer |
|---|---|---|
| framebuffer | the frame takes at most half of the RAM | `w * h / 8` (mono) or `w * h * 2` (color) |
| bands of 8 rows | a band takes at most a quarter of the RAM | `w` (mono) or `w * 16` (color) |
| character grid | HD44780 | `columns * rows` |
| none fits | | `NE0604`, with the bytes needed and the RAM of the chip |

The ILI9341 panel of `examples/panel_ui.neuron` gets a full framebuffer (150 KiB) on the
ESP32-S3; on the UNO neither the frame nor a band of 8 rows (5 KiB) fits in 2 KiB, so it is
`NE0604`. The SSD1306 OLED fits the UNO with a 1 KiB framebuffer.

The renderer redraws the screen when a bound value changes, every 100 ms at most (a task of
the lowest priority).

## How it is tested

`crates/neuron-ui/tests/b9_ui.rs` builds the firmware for the host with an mcu module that
writes the signals. The host runtime prints every I2C write and SPI transfer as a `BUS` line.
The display models of `neuron_ui::emu` (SSD1306, ILI9341/ST7789, HD44780 behind a PCF8574)
replay that traffic into an image, which must equal the reference renderer pixel for pixel
(property MOC-UI1): an OLED with a framebuffer, a color TFT drawn in bands, a character LCD.
The tests also check the strategy rules, `NE0604`, `NE0610`, the UNO build with `-Werror`,
the ESP-IDF project, and a property of bars.

Pending hardware: the dashboard on the ESP32-S3 with a TFT (it needs ESP-IDF and the panel),
and the OLED on the UNO.
