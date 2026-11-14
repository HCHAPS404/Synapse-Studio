# Requirements: Neuron semantic analysis (tasks A4 to A8)

Status: DRAFT v0.1 (2026-11-14). Parent spec: `neuron-language`. Normative text:
`spec/language/15-analysis.md`.

## Context

The parser (task A3) produces a syntax tree. Before NIR (task A9), the interpreter
(task A10) and the backends (phase B) can rely on a program, the compiler SHALL decide
whether the program is well formed. Users are students and engineers who need
diagnostics that teach, and evaluators who must be able to trust that a program that is
accepted behaves as written.

## Requirements

| Id | Requirement | Acceptance |
|---|---|---|
| S1 | Names, modules and packages (A4): scopes, imports, visibility, cycles, `neuron.toml` | NE0101 to NE0105 and NE0108 each have a positive and a negative test; the examples package has no errors |
| S2 | Match exhaustiveness (NE0103) for enums, `bool`, `Result` and integer ranges | tests for each case, including guards and alternatives |
| S3 | Types and units (A5): inference, quantities with dimensions, affine units, exact conversions, generics, classes and interfaces, `Option`, `Result` and `?` | NE0201 to NE0217 each tested; property UNIT-1 at the unit algebra and at the checker |
| S4 | Dialect rules and effects (A6): constructs per dialect, recursion, aliasing, escaping references, bounded loops, waiting, deadlines | one positive and one negative test per rule |
| S5 | Time and concurrency (A7): activities, priorities, ceilings, SRP blocking, WCET model, response-time analysis, signals, memory budget | NE0301, NE0304, NE0306, NE0401, NE0402, NE0406, NE0407, NE0604; properties SRP and MEM-1 on the computed model |
| S6 | Safety (A8): safe state completeness, `on fault`, model checking of state machines and sequences with counterexample traces | NE0802, NE0803, NE0901 to NE0904; counterexamples are shortest paths (property test) |
| S7 | Determinism: diagnostics and reports are identical across runs and platforms | diagnostics are sorted; no hash-order dependence |
| S8 | Honesty: results that cannot be decided are reported, never accepted silently | NL0402, NL0403, NL0901 |
| S9 | External vocabulary (standard library, targets) is typed gradually until tasks A12 and B1 | a value of unknown type produces no diagnostics; documented in chapter 15 |

## Out of scope

Peripheral legality and clock solving (B1), robot workspace checks (C3), zone locking and
electrical checks (D4), clock-domain crossings (B7), execution-level properties LET-1,
SAFE-1 and SAFE-2 (A10).
