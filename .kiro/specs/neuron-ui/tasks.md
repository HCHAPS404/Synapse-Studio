# Tasks: ui dialect on small displays (B9)

- [x] 1. `neuron-ui`: lowering, bindings, units, strategy from the RAM, `NE0604`, `NE0610`.
- [x] 2. Reference renderer and font.
- [x] 3. C renderer and drivers (SSD1306, ILI9341, ST7789, HD44780) and the bus of the host, AVR and ESP-IDF ports.
- [x] 4. `emit_c_ui` and the CLI (`neuron build` adds the screen when the package has a ui module).
- [x] 5. Display models and the differential tests (OLED framebuffer, TFT bands, character LCD).
- [x] 6. Docs (`docs/ui.md`), evidence in `docs/kiro-log.md`.
- [ ] 7. Dashboard on the ESP32-S3 with a TFT and the OLED on the UNO (needs ESP-IDF, the panels and the boards).
- [ ] 8. Touch input, `on press`, screen switching, more widgets and panels, the web HMI (with Martin, G4/G6).
