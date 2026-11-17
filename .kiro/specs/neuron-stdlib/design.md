# Design: Neuron standard library, base layer (task A12)

## Layout

`lib/` is a Neuron package named `std` (`lib/neuron.toml`). Each layer is a folder;
each module is a file whose `module` line carries the dotted path:

```text
lib/
  neuron.toml
  core/       math.neuron  bits.neuron
  units/      constants.neuron  convert.neuron
  time/       timers.neuron  edges.neuron  rate.neuron
  logic/      latches.neuron  counters.neuron  select.neuron
  control/    pid.neuron  filters.neuron  hysteresis.neuron  ramp.neuron
  test/       asserts.neuron
```

## Naming

The interpreter provides native function blocks with lower-case names (`ton`, `pid`,
...). The library provides the same behavior as classes with UpperCamel names (`Ton`,
`Pid`), as spec 12.2 asks ("state is explicit: a `class` the caller owns"). A class
receives the elapsed time as an argument (`step(mut self, input: bool, dt: duration)`),
so it needs no clock and is deterministic.

## Tests

Each module carries `test` blocks (unit tests). The differential tests live in
`lib/test/` and run a class and the native block side by side on generated inputs;
the Rust test `crates/neuron-interp/tests/a12_stdlib.rs` repeats the comparison over
random input sequences with `proptest` (STD-DIFF) and checks STD-PID-1 and STD-PID-2.

## Limits

- Library modules are checked and tested as their own package. User programs keep
  using the native blocks through the prelude; importing `std.*` names from a user
  package stays external vocabulary until the package manager exists (phase B).
- The WCET figures of spec 12.7 need a timing model and are not produced here.
