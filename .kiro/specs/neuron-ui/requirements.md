# Requirements: ui dialect on small displays (task B9)

Source: `docs/roadmap.md` task B9, `spec/language/09-dialects.md` §9.7.

## User stories

- As an integrator, I want a status screen bound to the signals of my program on an OLED or
  a TFT, without writing a display driver.
- As a student on an UNO, I want to be told before I flash that a color panel does not fit
  the RAM, and why.

## Acceptance criteria (EARS)

1. THE tool SHALL lay out the widgets `label`, `gauge`, `bar`, `slider`, `led` and `button`
   of the first screen of a `ui` module on SSD1306, ILI9341, ST7789 and HD44780 displays.
2. THE render strategy SHALL be chosen from the RAM of the chip (framebuffer, bands of 8
   rows, character grid); WHEN none fits THE tool SHALL report `NE0604` with the bytes needed
   and the RAM available.
3. A widget SHALL show a signal of the ui module; a signal with the same name in an `mcu`
   module of the package SHALL be the same signal.
4. THE firmware renderer SHALL produce, through the display bus, the image of the reference
   renderer pixel for pixel (property MOC-UI1, checked with display models).
5. Values SHALL be shown in the unit of the ui signal.
6. WHEN a widget or a driver is outside the set THE tool SHALL report `NE0610`.
7. The AVR and ESP-IDF ports SHALL carry the display bus (I2C, SPI).

## Out of scope

Touch input and `on press` handlers, screen switching, `chart`, `list`, `image`, `keypad`,
`numeric_input`, `toggle`, layout containers, e-paper and the other panels, the web HMI.
