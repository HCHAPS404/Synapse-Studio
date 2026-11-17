# The `hdl` dialect to Verilog (task B7)

`crates/neuron-codegen-hdl` turns the `circuit`s of the `hdl` modules of a package into
synthesizable Verilog-2005, writes the pin constraints of an FPGA board, and drives the open
tools when they are installed.

## Commands

| Command | What it does |
|---|---|
| `neuron emit-verilog prog.neuron` | Prints the Verilog of every circuit. |
| `neuron emit-verilog prog.neuron --board ice40up5k_breakout -o out` | Writes `top.v` and `top.pcf` (iCE40) or `top.xdc` (7-series). |
| `neuron build prog.neuron --board <fpga>` | Also builds the bitstream: yosys, nextpnr-ice40 and icepack for iCE40; Vivado in batch mode for 7-series. |
| `neuron flash prog.neuron --board <fpga>` | Builds, then loads the bitstream with openFPGALoader. |

`neuron targets` lists the FPGA boards. The top module is the last circuit of the package.

## Mapping

| Neuron | Verilog |
|---|---|
| `circuit c(a: in bit<8>, y: out bit) on clk` | `module c (input wire clk, input wire [7:0] a, output reg y)` |
| `reg r: bit<4> = 3` | `reg [3:0] r = 4'd3;` (initial value; see reset) |
| `wire w: bit` | `wire w;`, or `reg w;` when a block assigns it |
| `const k = clk.freq / 115200 Bd` | `localparam integer k = 868;` (evaluated, rounded to the nearest integer) |
| `on rising(clk):` / `on falling(clk):` | `always @(posedge clk)` / `always @(negedge clk)` with `<=` |
| `comb:` | `always @*` with `=` |
| `a ++ b`, `x[9:1]`, `x[0]` | `{a, b}`, `x[9:1]`, `x[0]` |
| `+%`, `-%` | `+`, `-` (the width of the target truncates) |
| `and`, `or`, `not` | `&&`, `\|\|`, `!` on one bit; `&`, `\|`, `~` on vectors |
| `instance u = c(a: x, y: z)` | `c u (.clk(clk), .a(x), .y(z));` |
| `sync a -> b via two_ff` | two flip-flops in the clock domain that reads `b`, marked `ASYNC_REG` |

Names that are Verilog keywords get the suffix `_n` (`edge` becomes `edge_n`).

**Reset.** Every register starts at its declared value (`initial` value, supported by iCE40
and 7-series). When a circuit has a one-bit input called `rst` or `reset`, the clocked blocks
also reset their registers synchronously while it is high.

**Clock domains (`NE0702`).** The domain of a register is the clock of the block that writes
it. A block that reads a register of another domain is `NE0702` unless the value goes through
a `sync ... via two_ff`.

**Outside the subset (`NE0610`).** State machines inside circuits, loops, calls and the other
`sync` methods (`latch_ack`, `fifo`) are reported, never dropped.

## Boards

`targets/fpga/*.fpga.toml`: name, family (`ice40` or `xc7`), part, package, user pins, the
I/O standard and the `openFPGALoader` arguments. Two generic boards ship today:
`artix7_35t_board` (Artix-7 35T, CPG236, 100 MHz clock on W5) and `ice40up5k_breakout`
(iCE40 UP5K, SG48, 12 MHz clock on pin 35). Their pins come from public package pinouts.
Confirm them against the lab board (session I3) before flashing.

## Tests

`crates/neuron-codegen-hdl/tests/b7_hdl.rs`:

- golden Verilog for every program of `examples/corpus/hdl` and `examples/pwm_hdl.neuron`
  (`conformance/golden/verilog/`);
- constraints for both families, `NE0603` for a pin the board lacks, `NE0702`, `NE0610`;
- simulation with iverilog of the counter, the edge detector, the shift register and the UART
  transmitter (a full 8N1 frame), compared with values computed in the test;
- synthesis with yosys of every program;
- a property test on register widths.

iverilog and yosys are installed in CI (`NEURON_REQUIRE_HDL=1` turns a missing tool into a
failure). On a machine without them those tests print `skipped`.

Pending hardware (session I3): counter, PWM and UART transmitter on the lab FPGA, and the
MCU↔FPGA UART link.
