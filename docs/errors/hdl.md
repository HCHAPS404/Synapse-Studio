# FPGA (hdl) errors (task B7)

Source of truth for the catalogue: `spec/language/10-diagnostics.md`. The translation rules
are in `docs/hdl.md`. `neuron explain NE0702` prints the matching section. `NE0610` (a
construct outside the subset of the selected code generator, `docs/errors/targets.md`) also
covers hdl constructs that the Verilog backend does not translate yet, and `NE0603` covers a
`pin(...)` that the FPGA board does not have.

## Clock domains

### NE0702 clock-domain crossing without a synchronizer
A block clocked by one clock reads a register written by a block of another clock. Without a
synchronizer the reading flip-flop can go metastable. Write `sync a -> a_dst via two_ff` and
read `a_dst` in the destination domain: the backend emits two flip-flops clocked by the
destination clock (marked `ASYNC_REG`). One `sync` serves one destination domain; a
synchronized value read in two domains is also `NE0702`. Multi-bit values need a handshake
(`latch_ack`) or a FIFO, which are not emitted yet (`NE0610`).
