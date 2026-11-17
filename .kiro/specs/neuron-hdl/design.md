# Design: hdl dialect to Verilog (B7)

`crates/neuron-codegen-hdl`:

- `emit.rs`: NIR circuits to Verilog. Constants are evaluated (`clk.freq / 115200 Bd` is 868),
  register initial values are sized literals, assignments are non-blocking in clocked blocks
  and blocking in `always @*`. Clock domains come from the blocks that write each register;
  reads across domains are `NE0702` unless a `sync` covers them. Verilog keywords get `_n`.
  A source map records the Verilog line of every statement.
- `board.rs`: `targets/fpga/*.fpga.toml` (embedded), PCF and XDC writers.
- `tools.rs`: iverilog/vvp, yosys (`synth_ice40`, `synth_xilinx`), nextpnr-ice40 + icepack,
  Vivado batch, openFPGALoader. Missing tools are a typed error; nothing is installed.

The mapping table and the reset convention are in `docs/hdl.md`.

Tests (`tests/b7_hdl.rs`): goldens, structure, constraints, `NE0702`, `NE0610`, iverilog
simulation against expected sequences, yosys synthesis, a width property.
