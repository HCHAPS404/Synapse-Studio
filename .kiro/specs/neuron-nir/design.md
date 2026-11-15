# Design: NIR (task A9)

Status: v1.0 (2026-11-15). Implements `requirements.md`. Normative format:
`spec/language/16-nir.md`.

## Components

| Component | Where | Role |
|---|---|---|
| Data model | `crates/neuron-nir/src/model.rs` | Serde types; every sum type is internally tagged (`item`, `e`, `s`, `p`, `t`, `lit`, `call`, ...) |
| JSON and schema | `crates/neuron-nir/src/lib.rs` | `to_json` (pretty, final newline), `from_json` (major version check), `schema_json` (schemars) |
| Printer | `crates/neuron-nir/src/print.rs` | NIR → `.neuron`, precedence of chapter 4 §4.6 |
| Lowering | `crates/neuron-sema/src/lower.rs` | `Program` + resolution + checker tables → NIR |
| Command line | `crates/neuron-cli` | `emit-nir`, `print-nir` |

## Decisions

1. **Separate crate without compiler dependencies** (N9). Editors, the twin and WASM
   builds read NIR without linking the analysis. Rejected: the model inside
   `neuron-sema`.
2. **Mirror the source structure** instead of a flattened program (`io` and `tasks`
   arrays sketched in the first draft of chapter 11). The printer needs the
   structure for P-RT, and graphical views map one-to-one onto items. Lowering of
   dialect constructs (scheduling, state encoding) belongs to MIR (B2).
3. **Ids by declaration order** (`m0.i3`), never positions, so layout edits do not
   change references.
4. **Types from the checker.** The checker records the type of every expression
   (`expr_ty`, keyed by file and span; the outer expression wins when spans are
   shared). Expressions it does not type are `unknown`.
5. **Exact numbers as strings** using the exact rationals of the unit algebra
   (`rat_text`): decimals when they terminate, `n/d` otherwise.
6. **Generated schema** (schemars, JSON Schema 2020-12) with a drift test, so the
   contract for Martin cannot diverge from the code. Regenerated with
   `tools/rust-check.ps1 -Golden`.
7. **P-RT modulo spans.** Spans change when text is reprinted, so P-RT compares NIR
   with spans removed and checks printer idempotence separately.
8. **IP addresses** stay a call in the grammar; lowering recognizes the dotted-quad
   shape and records an `ip` literal. Rejected: a new token, which would change the
   lexer for one form.
9. `clippy::large_enum_variant` is allowed in the model: it is a data tree built
   once per compilation, and boxing would only add noise.

## Properties and tests

`crates/neuron-sema/tests/a9_nir.rs`: N1, N2 (ids), NIR-1, JSON round trip, version
check, IP literal, golden file `conformance/golden/nir/examples.nir.json`, P-RT on the
examples package, on each example, on every valid spec program, and a proptest over
mutated examples. `crates/neuron-nir/tests/schema.rs`: schema drift.
