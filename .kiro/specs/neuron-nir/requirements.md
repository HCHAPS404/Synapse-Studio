# Requirements: NIR, the Neuron intermediate representation (task A9)

Status: v1.0 (2026-11-15). Parent spec: `neuron-language`. Normative text:
`spec/language/16-nir.md` and `spec/schemas/nir.schema.json`.

## Context

The analysis (tasks A4 to A8) decides whether a package is well formed. Every later
tool needs that result in one stable form: Martin's editors and graphical views and
the digital twin (phase F), the interpreter (A10), the emitters and importers (phase
C), the compiler bindings (E1) and the backends (phase B). The roadmap commits the
schema to Martin by 18 November.

## Requirements

| Id | Requirement | Acceptance |
|---|---|---|
| N1 | Lower every analyzed package with no errors to NIR; never for a package with errors | `Analysis::nir` is `Some` for the examples package and `None` when there is an error |
| N2 | Stable ids for modules, items, class members, PLC variables, clocks and locals; references resolved to ids | ids follow chapter 16 §16.3; the examples have resolved `ref`, `local` and call targets |
| N3 | The type of every expression and the exact value of constant quantities | `ty` on every expression; `si` on constant quantities; numbers as strings |
| N4 | Canonical JSON: the same package always gives the same bytes (property NIR-1) | two runs give byte-identical output with LF and a final newline |
| N5 | JSON round trip: reading the JSON gives the same NIR | `from_json(to_json(n)) == n` on the examples and every valid spec program |
| N6 | Printer back to `.neuron` with round trip (property P-RT) | P-RT on the examples package, each example alone, every valid spec program and generated variations (proptest) |
| N7 | Published JSON Schema that cannot drift from the code | `spec/schemas/nir.schema.json` is generated; a test fails when it differs |
| N8 | Command line | `neuron emit-nir <file|dir>... [-o out]` and `neuron print-nir <file.nir.json>` |
| N9 | Usable without the compiler | `crates/neuron-nir` depends only on serde, serde_json and schemars |
| N10 | Versioning | `nir_version` `1.0`; readers accept `1.x` and reject other majors |

## Out of scope

Formatter-based NIR-1 (A11). Types of device and driver vocabulary (D1, A12): such
expressions are `unknown` in NIR 1.0. Monomorphization and MIR (B2).
