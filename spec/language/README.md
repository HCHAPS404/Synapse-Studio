# The Neuron Language Specification — Edition 2026

Status: **v0.5 draft** (2026-11-16). Normative once frozen at gate G1.
Owner: Hell. Reviewers: Martin, `spec-reviewer` agent.

Neuron is a programming language for automation and hardware. One language, one
semantic core, eight dialects (one per model of computation or level), and one
interchange standard so that programs, devices and third-party languages can meet.

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
| [11 Interchange standard](11-interchange.md) | NIR, tag dictionary, Neuron Wire, fidelity report, source map, deployment package |
| [12 Standard library](12-stdlib.md) | Library layers, design rules, HAL interfaces, display stack, quality bar, scope for the submission |
| [13 Canonical formatting](13-formatting.md) | Formatter rules, line breaking, formatting properties, command line |
| [14 Models of computation](14-computation-models.md) | Formal semantics per model, cell time model, equivalence and refinement for translation |
| [15 Semantic analysis](15-analysis.md) | Names and modules, types and units, dialect rules, time and concurrency, safe state and model checking |
| [16 NIR](16-nir.md) | Intermediate representation: structure, ids, types and units, exact numbers, canonical JSON, printer and round trip, versioning |
| [NIR schema](../schemas/nir.schema.json) | JSON Schema of NIR (generated, checked in CI) |
| [17 The reference interpreter](17-interpreter.md) | Running programs in simulated time: scheduling and LET, activities, faults and the safe state, devices outside the package, `test` blocks |
| [Grammar](../grammar/neuron.ebnf) | Complete EBNF grammar |

## Conventions used in this specification

- **SHALL / SHALL NOT / MAY** have their usual normative meaning.
- Code examples are Neuron unless marked otherwise. Every `neuron` code block that
  starts with `module` or `cell` is parsed by the test suite, so examples cannot drift
  from the grammar. Pseudo-code and before/after layouts use `text` fences.
- `NE####` refers to a diagnostic in chapter 10; each has an entry in
  [`docs/errors/`](../../docs/errors/).
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

## Changes from v0.4 to v0.5

Found while implementing task A10:

- **New chapter 17** defines the reference interpreter: the order of events inside one
  instant, logical execution time in practice, how each activity runs, faults and
  reset, the model of devices outside the package, and `test` blocks.
- **`std.test`.** `drive`, `advance`, `elapsed`, `current_fault` and `state_of` are
  prelude functions meant for `test` blocks (chapter 17 section 17.8).
- **Runtime faults.** `LoopBound` (a body executed more than 5 million statements, or a
  collection or loop larger than 2^20) joins the built-in faults of chapter 8.
- **`signal` in NIR** carries its resolved type (`vty`), so tools can show its unit.

## Changes from v0.3 to v0.4

Found while implementing task A9:

- **New chapter 16** defines NIR. Chapter 11 §11.1 now summarizes it and points
  there; its earlier sketch (separate `io` and `tasks` arrays, durations in
  nanoseconds) is replaced by a model that mirrors the source, so NIR can be printed
  back (property P-RT).
- **NIR-1** is stated without the formatter, which does not exist until A11: the same
  package always gives the same bytes. The formatter extends it in A11.
- **IP addresses.** `ip(192.168.0.10)` is still not a token; NIR records it as an `ip`
  literal (chapter 2 §2.8, chapter 16 §16.5).
- **JSON Schema** published as `spec/schemas/nir.schema.json`.

## Changes from v0.2 to v0.3

Found while implementing tasks A4 to A8:

- **New chapter 15** defines the semantic analysis. Chapter 10 gains the codes
  `NE0108`, `NE0209` to `NE0212`, `NE0214` to `NE0217`, `NE0306`, `NE0406`, `NE0407`,
  `NE0502`, `NE0904`, `NL0301`, `NL0403` and `NL0901`.
- **`use self.module`** parses: `self` is the package root.
- **Loops in `core`.** `core` code is checked like device code for loop bounds and
  recursion, because it must be usable from every device dialect.
- **Fault states.** A state whose entry or `do` raises `fault` is terminal and does not
  need `final` (chapter 6 §6.8).
- **Robot example.** The palletizing example in chapter 9 uses constants for its loop
  bounds, as `NE0403` requires.
- **`examples/` is a package** (`examples/neuron.toml`) and is analyzed in CI.

## Changes from v0.1 to v0.2

Applied after the review of chapters 01 to 10 (20 findings):

- **Units.** A unit after a range applies to both bounds (`0..120 degC`). A unit may
  follow a number, a tuple, a vector constructor or any call; the semantic phase
  reports `NE0202` for calls that cannot take one. `%` is a unit when no operand
  follows it. Number and unit are separated by exactly one space (`NE0014`).
- **Keywords.** `max`, `result` and `old` are contextual. `impl` and `Self` are not
  reserved.
- **Literals.** `.5` and `5.` are errors (`NE0006`); misplaced `_` is an error.
  Names are ASCII; only the unit characters µ, Ω and ° are accepted outside strings
  (`NE0015`). String interpolation is validated (`NE0016`).
- **Structure.** A cell file indents its items under the header (`NE0005`) and has no
  content after the block (`NE0013`). Contracts are valid only at the start of a
  function body (`NE0012`).
- **Library surface.** Robot inputs and outputs are `din(5)` and `dout(3)`. Bit
  replication is `x.repeat(4)`. The `reg ... reset` clause is gone; the initializer
  of a `reg` is its reset value.
- **Tooling.** Every AST node carries a span. An `outline` view backs the golden
  tests in `conformance/golden/parser/`.
- **New chapters.** 11 (interchange), 12 (standard library), 13 (formatting) and 14
  (models of computation).
