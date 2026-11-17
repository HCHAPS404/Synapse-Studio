# Design: native AVR backend (B8)

See `docs/native-backend.md` for the register conventions, the subset and the tests.

- `asm.rs`: typed instructions (`Op`), labels, two passes with fixed sizes, encodings of
  the AVR instruction set summary, range checks, Intel HEX with checksums.
- `codegen.rs`: memory layout (runtime state, globals, signal buffers, snapshots, task
  state, locals), vector table (reset, INT0, INT1, TIMER0_COMPA), start-up generated last so
  that it clears all the SRAM in use, `rt_step` (commits, releases with the deadline-miss
  fault, the run loop in priority order), `rt_fault`, `rt_udivmod16`, the activities with
  their trap paths (each saves its stack pointer on entry), the interrupt handlers (level
  filter for the edge, as the C runtime does).
- Long conditional jumps are an inverted short branch over a `JMP`, so code size never
  breaks a branch.
