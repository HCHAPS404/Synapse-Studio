# Design: Neuron semantic analysis

Status: DRAFT v0.1 (2026-11-14). Implements `requirements.md`.

## Crate layout (`crates/neuron-sema`)

| Module | Content | Task |
|---|---|---|
| `manifest` | `neuron.toml` subset | A4 |
| `model` | files, modules, symbols, imports, cycles | A4 |
| `resolve` | scoped name resolution in bodies, suggestions | A4 |
| `consteval` | constant folding with units | A5 to A7 |
| `units` | exact rationals, dimensions, units, conversions (public) | A5 |
| `ty`, `check/` | semantic types, expressions, statements, patterns, definitions | A4, A5 |
| `visit`, `facts` | traversal helpers; bodies, accesses, calls, waits | A6, A7 |
| `rules` | dialect matrix, recursion, aliasing, slices, loops, waiting | A6 |
| `timing` | activities, SRP, WCET, response time, memory | A7 |
| `safety`, `mc` | safe state; automata and model checker | A8 |
| `report` | public report types | A7, A8 |

## Decisions

1. **One resolution table.** The resolver records for every identifier what it refers
   to (local id, module item, prelude, import). The checker and the later analyses read
   that table; no phase resolves names twice.
2. **Gradual boundary.** `Ty::Unknown` is compatible with everything. Anything that
   comes from the standard library or a target is `Unknown` until A12 and B1.
3. **Exact arithmetic.** Unit scales are rationals; the only inexact constant is π
   (15 digits, used by `deg` and `rev`). Affine units carry an offset.
4. **Facts before rules.** A single traversal computes, for every body, the module
   state it reads and writes, the functions it calls and the waits it contains. The
   dialect rules, the SRP analysis and the safety checks are queries over those facts.
5. **Abstract WCET.** Without a target, execution time is not guessed: the analysis
   reports `NL0402`. With a CPU clock, an operation-count model gives a deterministic,
   conservative estimate. B1 will replace the model with measured timing profiles.
6. **Environment is adversarial.** The model checker treats every condition that is not
   an automaton state as a free input per tick. A verified property therefore holds for
   every environment; a violation may need an environment that is unrealistic, which the
   counterexample makes visible.
7. **Shortest counterexamples.** Breadth-first search over (states, dwell times,
   monitor) with valuations enumerated in a fixed order.
8. **No `HashMap` in outputs.** Ordered maps and sorted diagnostics.

## Public API

```rust
let analysis = neuron_sema::analyze(sources, Some(manifest_text), &SemaOptions::default());
analysis.errors();            // iterator of diagnostics
analysis.render();            // text in the compiler's format
analysis.reports.timing;      // per activity
analysis.reports.counterexamples;
```

`neuron check <dir|file>` in `neuron-cli` calls it.

## Testing

* Unit tests of the unit algebra and the manifest parser.
* One integration test file per task (`a4_names.rs` to `a8_safety.rs`).
* Property tests: UNIT-1 (algebra and checker), SRP ceilings and blocking bounds,
  MEM-1 sums, shortest counterexamples against a reference breadth-first search.
* The `examples/` package is analyzed in CI.
