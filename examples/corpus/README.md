# Validation corpus

Fifty small programs, one folder per dialect. Each folder is a package with its own
`neuron.toml`. Every file must analyze with no error, be in the canonical layout, run
for two simulated seconds without a runtime trap, and pass its `test` blocks. The
check is automatic:

```text
neuron check examples/corpus/<dialect>
neuron fmt --check examples/corpus/<dialect>
neuron test examples/corpus/<dialect>
```

`crates/neuron-interp/tests/a13_corpus.rs` runs the same checks over all files, and CI
runs the commands above for each folder.

| Folder | Programs | What they show |
|---|---|---|
| `mcu/` | `m01` to `m11` | blink, interrupts and events, debounce, PWM and ramp, hysteresis, a state machine, signals with deadlines, safety with a fault and reset, PID with a filter |
| `plc/` | `p01` to `p10` | start/stop latch, set and reset coils, timers and counters, flasher, pump with levels, edge counter, two-hand control, a sequence (SFC), an analog alarm |
| `core/` | `c01` to `c05` | units in signatures, enums and `match`, `Option` and `Result`, a class with contracts, a struct with units |
| `host/` | `h01` to `h05` | a gateway task, a moving average, a bounded retry loop, loops and pipelines, a rule that raises a fault |
| `hdl/` | `d01` to `d05` | counter, clock divider, shift register, edge detector, UART transmitter |
| `robot/` | `r01` to `r06` | joint moves, palletizing, gripper check with timeout, a welding seam, an inspection grid, an arc |
| `ui/` | `u01` to `u04` | a label on an OLED, a start/stop panel, a tank gauge, two screens |
| `cell/` | `x01` to `x04` | two linked devices, a shared zone with a mutex, a coordination sequence, an event with a timestamp |

The programs are original and use generic device profiles. They are design examples,
not certified safety logic: a hardwired emergency stop remains the authority.
