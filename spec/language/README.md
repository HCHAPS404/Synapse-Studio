# The Neuron Language Specification — Edition 2026

Status: **v0.1 draft** (2026-11-14). Normative once frozen at gate G1.
Owner: Hell. Reviewers: Martin, `spec-reviewer` agent.

Neuron is a programming language for automation and hardware. One language, one
semantic core, six dialects (one per model of computation), and one interchange
standard so that programs, devices and third-party languages can meet.

| Chapter | Content |
|---|---|
| [01 Overview](01-overview.md) | Goals, non-goals, architecture (core, models of computation, dialects), program structure, examples |
| [02 Lexical structure](02-lexical.md) | Source text, indentation, comments, tokens, literals, quantities, keywords |
| [03 Types and units](03-types.md) | Scalar and compound types, physical units, numeric semantics, conversions |
| [04 Declarations and statements](04-declarations-statements.md) | Items, functions, contracts, control flow, patterns, errors, modules |
| [05 Memory and ownership](05-memory.md) | Static memory, parameter modes, aliasing rules, pools, channels |
| [06 Time and concurrency](06-time-concurrency.md) | Tasks, interrupts, priorities, SRP, LET, signals, rules, state machines, sequences |
| [07 Hardware](07-hardware.md) | I/O bindings, ownership of resources, typed registers, clock tree, capabilities, `asm`, `extern` |
| [08 Safety](08-safety.md) | Safe state, faults, watchdogs, overflow policy, trusted code, deployment rules |
| [09 Dialects](09-dialects.md) | `core`, `mcu`, `plc`, `hdl`, `robot`, `ui`, `host` and `cell`, with their graphical views |
| [10 Diagnostics](10-diagnostics.md) | Error code families and the initial catalogue |
| [Grammar](../grammar/neuron.ebnf) | Complete EBNF grammar |

## Conventions used in this specification

- **SHALL / SHALL NOT / MAY** have their usual normative meaning.
- Code examples are Neuron unless marked otherwise.
- `NE####` refers to a diagnostic in chapter 10.
- *Property* blocks state correctness properties that the implementation must
  satisfy; each becomes a property-based test.

## Decisions taken in this draft

| Question | Decision |
|---|---|
| Comments | `#` line, `##` documentation, `#[ ... ]#` block (nestable) |
| Mutability | `let` immutable, `var` mutable |
| Blocks | Indentation (2 spaces) after a `:`; braces are never block delimiters |
| Compound units | `mm/s`, `m/s^2`, `N*m` |
| Generics | `<...>` in type positions only; inferred in expressions |
| Dialect selection | Declared in the file header: `module name for mcu on nucleo_f446re` |
| Hardware bindings | One keyword for every peripheral: `io name: kind(config) @ location` |
| Parameter passing | Read-only by default; `mut` for exclusive in-out, marked at the call site |
| Errors | `Result<T, E>` and `T?` (Option) with `?` propagation; `fault` for safe-state entry; no exceptions |
