# Tasks: hdl dialect to Verilog (B7)

- [x] 1. Crate `neuron-codegen-hdl`: Verilog emitter for ports, `reg`, `wire`, `const`, clocked and combinational blocks, instances.
- [x] 2. Clock domains, `NE0702`, two-flop synchronizers for `sync ... via two_ff`.
- [x] 3. FPGA board files (iCE40 UP5K breakout, Artix-7 35T board), PCF and XDC, `NE0603`.
- [x] 4. Tool drivers: iverilog, yosys, nextpnr/icepack, Vivado batch, openFPGALoader.
- [x] 5. Goldens for the corpus, iverilog simulation and yosys synthesis tests (CI installs the tools).
- [x] 6. CLI: `neuron emit-verilog`, `neuron build --board <fpga>`, `neuron flash` for FPGA boards.
- [x] 7. Docs (`docs/hdl.md`, `docs/errors/hdl.md`), evidence in `docs/kiro-log.md`.
- [ ] 8. Counter, PWM and UART TX on the lab FPGA; MCU↔FPGA UART link (session I3, needs the board).
- [ ] 9. State machines inside circuits, `latch_ack` and FIFO synchronizers.
