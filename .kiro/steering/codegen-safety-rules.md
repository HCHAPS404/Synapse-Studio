---
inclusion: fileMatch
fileMatchPattern: "crates/neuron-backend-*/**"
---

# Code generation rules for emitters and backends

These rules apply to everything the compiler emits for real devices.

## Embedded C / C++ (MCU)
- No blocking calls (`delay`, busy-wait on unbounded conditions). Use timers,
  interrupts and the static scheduler.
- No dynamic allocation. Static buffers with declared sizes.
- Every network or bus call checks and handles errors; failures go to the
  declared safe state.
- Volatile access only through generated typed register accessors.

## Verilog / HDL
- Purely synchronous design. No accidental latches (complete `case`/`if`
  assignments, default branches).
- Every clock-domain crossing uses a synchronizer; fast pulses crossing into a
  slower domain use latch + ACK.
- Constraints (XDC/PCF) generated from the pin plan with I/O standard and bank
  voltage. Verify with `verilator --lint-only` and `iverilog`.

## IEC 61131-3 (ST, SCL, PLCopen)
- Use deterministic scan-cycle semantics; no unbounded loops.
- Respect vendor reserved words and identifier limits; rename with a stable
  mangling and keep a mapping.

## Robot programs (SPEL+ and similar)
- Validate every target coordinate against the workspace bounding box declared in
  the device manifest **before** emitting a motion command.
- Honor mutex zones: emit lock/unlock around entry to shared volumes.
- Provide a safe-state routine for communication loss (heartbeat timeout 50 ms).

## Electrical
- Check logic-level compatibility (3.3 V TTL vs 24 V DC) and report the need for
  optocouplers or level shifters before deployment.

## General
- Emit a source map (generated line to `.neuron` line and graphic element).
- Emit a fidelity report (exact, approximate, unsupported) for every artifact.
- Respect per-target line endings and encodings; see `.gitattributes`.
