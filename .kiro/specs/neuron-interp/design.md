# Design: the reference interpreter (task A10)

Status: v1.0 (2026-11-16). Implements `requirements.md`. Normative text:
`spec/language/17-interpreter.md`.

## Components

| File | Role |
|---|---|
| `crates/neuron-interp/src/value.rs` | runtime values, units for display, equality |
| `.../eval.rs` | expressions, statements, patterns, places |
| `.../calls.rs` | user functions, methods, constructors, device actions |
| `.../builtin.rs` | math functions, function blocks, `io` bindings |
| `.../sched.rs` | setup, event loop, activities, faults |
| `.../testing.rs` | `test` blocks and the `std.test` functions |
| `.../scenario.rs`, `trace.rs` | scripted inputs and the observation |
| `crates/neuron-cli` | `neuron run`, `neuron test` |

## Decisions

1. **Interpret NIR, not the syntax tree.** NIR already has resolved references, ids and
   a type on every expression. The interpreter has no name resolution of its own, and
   every tool that emits NIR can be run.
2. **Discrete events in a map ordered by `(instant, rank, sequence)`.** The rank gives the
   order inside one instant (commits, inputs, events, releases, delayed bodies). There is
   one source of order, so runs are reproducible.
3. **Bodies run instantly; LET is the only place time shows.** A body executes at its
   release with a snapshot of the signals; its signal writes are queued for
   `release + deadline`. With jitter, the body starts later but keeps the release
   snapshot, which is how LET-1 is tested.
4. **Quantities in base SI with their display unit.** One representation for every unit
   including affine ones: `degC` is kelvin with an offset, so `degC - degC` and
   `degC + K` need no special case. Integer types with a unit lose their range check;
   documented.
5. **Function blocks are native.** The standard library (A12) also gets Neuron source
   for the same blocks; the tests compare the two implementations.
6. **Opaque devices.** A name outside the package becomes an opaque value. Inside `run`,
   a call on it starts an action with a configurable duration and the BUSY / DONE
   handshake. Rejected: failing on unknown names, which would make cells unrunnable.
7. **Bounded everything.** Statements per activation (5 million), array sizes, loop
   iterations, call depth and events per instant are limited, and the limit raises a
   named fault (`LoopBound`, `StackOverflow`). The interpreter must stay total because
   it runs generated and mutated programs.
8. **Tests as language items.** `test "name":` blocks use the same interpreter with five
   functions (`drive`, `advance`, `elapsed`, `current_fault`, `state_of`) and `assert`.
   Rejected: a separate test language.

## Tests

`tests/a10_run.rs`: one scenario per activity kind, the examples (thermostat fault,
PLC jam), LET-1 with five seeds, SAFE-1 and determinism, `test` blocks.
`tests/robust.rs`: every example, the examples package, every valid spec program and
generated variations. Unit tests in each module.
