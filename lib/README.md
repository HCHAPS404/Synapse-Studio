# Neuron standard library (`std`), base layer

Written in Neuron. Check and test it with:

```text
neuron check lib
neuron test lib
```

Every item owns its state and receives the elapsed time `dt` from the caller. Nothing
here allocates, touches hardware or reads a clock, so it behaves the same in the
interpreter, on a microcontroller and in a PLC. The layering rule of spec 12.1 is
checked by `neuron check` (`NE0106`).

The classes below are the Neuron source of the function blocks that the interpreter
also provides natively (`ton`, `pid`, ...). A differential test
(`crates/neuron-interp/tests/a12_stdlib.rs`) feeds both the same random inputs and
requires the same outputs.

| Layer | Module | Items |
|---|---|---|
| core | `std.core.math` | `lerp`, `map_range`, `abs_diff`, `between` |
| core | `std.core.bits` | `bit_get`, `bit_set`, `bit_clear`, `popcount` |
| units | `std.units.constants` | `pi`, `g0`, `absolute_zero` |
| time | `std.time.timers` | `Ton`, `Tof`, `Tp` |
| time | `std.time.clock` | `Stopwatch`, `Every` |
| logic | `std.logic.edges` | `RTrig`, `FTrig` |
| logic | `std.logic.latches` | `Sr` (set dominant), `Rs` (reset dominant) |
| logic | `std.logic.counters` | `Ctu`, `Ctd`, `Ctud` |
| control | `std.control.pid` | `Pid` with anti-windup |
| control | `std.control.filters` | `Lowpass`, `Highpass`, `MovingAverage` (window 1 to 16) |
| control | `std.control.hysteresis` | `Hysteresis`, `Debounce` |
| control | `std.control.ramp` | `Ramp` |
| test | `std.test.asserts` | `close` |

## Limits of this version

- Controllers and filters take plain `f32` values and keep the unit of the caller. A
  unit-aware version needs generic classes over `Num`, planned after the freeze.
- `hal`, `proto`, `motion`, `dsp`, `drivers`, `safety`, `diag` and `bsp` belong to
  phase B and later (spec 12.8).
- Programs still use the native blocks through the prelude. Importing a `std.*` name
  from another package is external vocabulary until the package manager exists.
