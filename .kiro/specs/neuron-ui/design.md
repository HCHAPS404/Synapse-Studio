# Design: ui dialect on small displays (B9)

See `docs/ui.md` for the widgets, the drivers, the bus pins and the strategy table.

- `crates/neuron-ui`: `lower_ui` (NIR to `UiApp`: display, widgets, bindings to MIR globals,
  strategy), `render` (the reference renderer, single-precision arithmetic like the C),
  `emit_ui_c` (font, widget table, display description, `ui_task`), `emu` (SSD1306,
  ILI9341/ST7789 and HD44780 models that replay `BUS` traffic).
- `runtime/neuron-rt/ui/nrt_ui.c`: renderer and drivers on top of four port functions
  (`nrt_hal_bus_init`, `nrt_hal_i2c_write`, `nrt_hal_spi_write`, `nrt_hal_delay_ms`),
  implemented by the host (trace), AVR (TWI and SPI registers) and ESP-IDF ports.
- `neuron-codegen-c::emit_c_ui` appends the ui section and a lowest-priority task that runs
  every 100 ms and redraws when a bound value changed.
- The font is a 5x7 design of this project, written into the generated C from the same
  table the reference renderer uses.
