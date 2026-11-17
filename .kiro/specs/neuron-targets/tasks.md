# Tasks: target model (B1)

- [x] 1. Crate `neuron-target`: structs, TOML loader, validation, `TGT-1` test.
- [x] 2. Files for the UNO R3 (arch, chip, board, runtime) with `source` fields.
- [x] 3. Pin solver and `NE0601`, `NE0603`, with `HW-1` as a property test.
- [x] 4. Clock solver and `NE0605`, `NE0606`, with `CLK-1` as a property test.
- [x] 5. Hook into the tool: `neuron check` loads the board after `on` and runs the
      hardware checks (`NE0607` for an unknown name or an invalid file). The checks live
      in `neuron-target::check` and run on the NIR, so `neuron-sema` stays independent of
      the target files.
- [x] 6. Error documentation in `docs/errors/targets.md` and
      `spec/language/10-diagnostics.md` (also `NE0608` and `NE0610`, added by this phase).
- [x] 7. Files for Mega, Pico, Pico 2 W, ESP32-S3, XIAO ESP32-S3, NUCLEO-F446RE and
      NUCLEO-N657X0-Q. The N657 board is a placeholder: its pin list is incomplete and its
      crystal is to be confirmed on the board.
- [x] 8. Evidence entry in `docs/kiro-log.md`.

## Notes

- Requirement 4 mentions `uses regs`: `uses` declarations claim the peripheral they
  name, so they collide with `io` bindings (`NE0601`). Register-level access rights
  (`NE0602`) come with typed registers, after B6.
- The UART tolerance default is 3 % (spec 07 §7.3 was updated): 115200 Bd from 16 MHz is
  2.1 % off, which works with every common USB-serial bridge.
