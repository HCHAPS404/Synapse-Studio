# Design: Neuron language (edition 2026)

Status: DRAFT v0.2 (2026-11-14). Implements `requirements.md`.

The normative design is the language specification in
[`spec/language/`](../../../spec/language/README.md) and the grammar in
[`spec/grammar/neuron.ebnf`](../../../spec/grammar/neuron.ebnf). This file records the
architecture decisions and how requirements map to chapters.

## Architecture

One semantic **Core** + seven **models of computation** + eight **dialects**
(`core`, `mcu`, `plc`, `hdl`, `robot`, `ui`, `host`, `cell`). Each file declares
its dialect in its header. Graphical views are bidirectional projections of the text.
Hardware facts come from a four-layer target stack (architecture, chip, board,
platform runtime), never from the grammar.

## Requirement to chapter map

| Requirement | Chapter |
|---|---|
| R1 two levels (module/cell) | 01, 09 §9.9 |
| R2 units | 03 §3.3 |
| R3 memory | 05 |
| R4 concurrency | 06 §6.2–6.4 |
| R5 real time | 06 §6.5, 04 loops |
| R6 hardware access | 07 |
| R7 reactive and sequential | 06 §6.7–6.10 |
| R8 types and abstraction | 03 |
| R9 contracts | 04 §4.2, 08 §8.3 |
| R10 dialects (was "profiles") | 09 |
| R11 modules and packages | 04 §4.7 |
| R12 syntax quality and diagnostics | 02, 10 |
| R13 IR round trip | 01 §1.6 |
| R14 safety layers | 08 |
| R15 architecture-aware compilation | 07 §7.7 |

## Key decisions (ADR summary)

1. Indentation blocks, 2 spaces, `:` opens a block; braces never delimit blocks.
2. `let`/`var`; `#` comments; units as postfix on numbers and types.
3. Read-only parameters by default, `mut` marked at definition and call site; no
   stored references (no lifetimes in edition 2026).
4. Shared state protected automatically with SRP; `signal` gives LET semantics.
5. One `io` keyword for every peripheral binding; exclusive ownership of resources.
6. Every output needs a safe state; `fault` is the only abrupt control transfer.
7. No undefined behavior in safe code; `trusted` marks unverified code.
8. Vendor differences live in manifests and emitters, not in syntax.

## Correctness properties (become property-based tests)

LEX-1, LEX-2 (02) · UNIT-1 (03) · MEM-1, MEM-2 (05) · SRP-1..3, LET-1 (06) ·
HW-1 (07) · SAFE-1, SAFE-2 (08) · VIEW-1 (09) · P-FMT and P-RT (requirements R12, R13).
