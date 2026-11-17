# Requirements: hdl dialect to Verilog (task B7)

Source: `docs/roadmap.md` task B7, `spec/language/09-dialects.md` §9.5.

## User stories

- As a control engineer, I want to write a PWM generator or a UART transmitter in Neuron
  and get Verilog that the open tools synthesize for the lab FPGA.
- As a student, I want a clock-domain crossing without a synchronizer to be reported
  before I load a bitstream that fails at random.

## Acceptance criteria (EARS)

1. WHEN a package has `hdl` modules THE tool SHALL emit one Verilog-2005 module per
   `circuit`, with registers, clocked blocks, combinational blocks, constants, instances and
   two-flop synchronizers.
2. WHEN a block reads a register of another clock domain without `sync ... via two_ff` THE
   tool SHALL report `NE0702`.
3. WHEN a construct is outside the subset THE tool SHALL report `NE0610`, naming it.
4. WHEN a board is given THE tool SHALL write its constraint file (PCF for iCE40, XDC for
   7-series) from the `@ pin(...)` locations and the clock frequencies, and report `NE0603`
   for a pin the board does not have.
5. THE Verilog of the corpus SHALL simulate in iverilog with the expected behavior and
   synthesize with yosys (CI).
6. WHEN yosys, nextpnr and icepack (or Vivado) are installed `neuron build --board <fpga>`
   SHALL produce the bitstream, and `neuron flash` SHALL load it with openFPGALoader.
7. The output SHALL be deterministic.

## Out of scope

State machines inside circuits, `latch_ack` and FIFO synchronizers, VHDL output.
