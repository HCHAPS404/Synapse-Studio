# Kiro usage log

Evidence of how Kiro was used to build SYNAPSE Studio. One entry per notable step.
This file feeds the "How we used Kiro" write-up required by Kiroween 2026.

Format: date, plan task, Kiro features used, what was produced, lessons.

---

## 2026-11-13 · Planning (before Task 1)
- **Kiro features:** Kiro IDE chat (vibe session) with the context-gatherer sub-agent
  (extracted the full text of the 9-page concept PDF, which the file reader could not
  decode), web fetch (official hackathon page, Neon pricing, Render free-tier docs).
- **Produced:** implementation plan v2.1 (78 tasks across 19 phases), architecture
  decisions (Rust core with WASM/PyO3/JNI outputs, own engine, three AI modes, three
  client apps, virtual lab for judges), compliance matrix built from the official rules.
- **Lessons:** asking clarifying questions before writing the plan changed most
  decisions: Neuron became a real programming language, not only an IR; the engine
  became from-scratch Rust; virtual devices running generated code were added so the
  twin is not just an animation.

## 2026-11-13 · Tasks 2-4 (foundation)
- **Kiro features:** steering (always-on `product`, `tech`, `structure`,
  `conventions`; file-match `rust-style`, `python-style`, `typescript-web-style`,
  `codegen-safety-rules`), agent hooks (secret guard before writes, cargo check on
  save, `.kiro`-not-ignored guard on `.gitignore` save, evidence logging after each
  spec task).
- **Produced:** Apache-2.0 monorepo skeleton, governance files, Rust workspace
  (`neuron-syntax`, `neuron-cli`) with property-based tests (proptest), Core API
  (FastAPI) with `/health` that never leaks the database URL, CI workflow with a
  hackathon-compliance job, Render blueprint, first spec (`neuron-language`).
- **Lessons:** the `.kiro`-not-ignored rule is a disqualification risk, so it is
  enforced three times (hook, local script, CI job).

## 2026-11-14 · Task 5 (Neuron language specification v0.1)
- **Kiro features:** spec-driven development (`.kiro/specs/neuron-language`
  requirements R1–R15 with correctness properties), steering (product non-goals and
  code-generation safety rules shaped the safety and dialect chapters).
- **Produced:** the language specification in `spec/language/` (10 chapters:
  overview, lexical structure, types and units, declarations and statements, memory,
  time and concurrency, hardware, safety, dialects, diagnostics) and the complete
  EBNF grammar in `spec/grammar/neuron.ebnf`. Architecture: one semantic core, models
  of computation, eight dialects (`core`, `mcu`, `plc`, `hdl`, `robot`, `ui`, `host`,
  `cell`), four-layer target stack, five safety layers.
- **Lessons:** the team asked to design the language before any testing; designing
  per model of computation (scan, RTL, tasks, motion) instead of per vendor kept the
  grammar small. An integrity check found that `requirements.md` had been committed
  as binary noise in the first commit; it was rewritten and a reusable integrity
  check (`tools/integrity-check.ps1`) was added.

## 2026-11-14 · Task 11 (lexer and parser)
- **Kiro features:** spec-driven implementation from `spec/language` and the EBNF;
  file-match steering (`rust-style`) enforced no `unwrap` and clippy-clean code;
  property-based testing with proptest.
- **Produced:** `crates/neuron-syntax`: lossless lexer (indentation, quantities with
  units, IEC addresses, bit vectors, nested comments), AST for all eight dialects,
  recursive-descent parser with error recovery, and `neuron lex | ast | check` in the
  CLI. 57 tests pass, including properties LEX-1 (lossless), LEX-2 (indentation
  balance) and parser totality, and a test that parses every complete program in the
  specification.
- **Lessons:** PBT found nothing the unit tests missed in this step, but the
  spec-driven test caught a spec example (`Ok(raw degC)`) that the grammar did not
  allow; the spec was corrected. A `PreToolUse` hook that blocked every file write
  doubled credit use and was removed; secret scanning stays in CI (gitleaks), which
  is where it actually protects the repository. Local `.git` and `.venv` files on
  drive E: were found damaged; git was rebuilt from GitHub (backup kept) and the
  virtual environment recreated.

## 2026-11-14 · Task A1-A3 review fixes and chapters 11-14 (spec v0.2)
- **Kiro features:** spec-driven development (spec first, then lexer, parser and
  tests), a structured review of the v0.1 spec before freezing it, golden-file tests
  generated from the parser, and a test that parses every `neuron` code block in the
  spec chapters.
- **Produced:** 20 review fixes across spec, grammar, lexer, AST and parser (units on
  ranges and calls, contextual keywords, literal rules, cell file structure, contract
  placement, spans on every AST node); the `outline` module and 7 golden files in
  `conformance/golden/parser/`; 5 new examples (MCU, PLC, robot, UI, HDL); error
  catalogue `docs/errors/syntax.md`; chapters 11 (interchange), 12 (standard
  library), 13 (formatting) and 14 (models of computation); `docs/roadmap.md`.
- **Lessons:** the spec-block test caught an invalid example in chapter 12 on its
  first run (`f32 / s` is not a valid field type), so code in the spec cannot drift
  from the grammar. A script that lists every `NE` code used in the chapters found
  mismatched meanings (a latency error reusing a clock-crossing code), fixed before
  the freeze. Windows App Control intermittently blocks freshly built test
  executables (os error 4551); `tools/test-retry.ps1` rebuilds and retries.

## 2026-11-14 · Tasks A4 to A8 (semantic analysis)
- **Kiro features:** spec-driven development (new spec `.kiro/specs/neuron-sema` and
  chapter 15 of the language specification written before the code), steering
  (`rust-style`: typed `NE####` diagnostics, deterministic output, no `unwrap`),
  property-based testing (`proptest`) for UNIT-1, SRP ceilings and blocking, MEM-1 sums
  and shortest counterexamples.
- **Produced:** the `neuron-sema` crate: name resolution and packages (A4), exact unit
  algebra, type checker, generics, classes, match exhaustiveness (A5), dialect rules and
  effects (A6), activities, SRP ceilings, WCET model, response-time analysis and memory
  budget (A7), safe-state checks and an explicit-state model checker for state machines
  and sequences with counterexample traces (A8); `neuron check` now analyzes files and
  packages (`--cpu-hz`, `--ram`, `--report`); `examples/` became a package; error
  catalogue `docs/errors/analysis.md`.
- **Lessons:** writing the analysis against the existing examples exposed spec gaps that
  were fixed in the spec first: `use self.x` did not parse, fault states looked like
  deadlocks, and a robot example had a loop without a static bound. Silent corruption of
  four tracked files and of the `services/core` directory entry on drive E: was found by
  comparing file hashes with git (`tools/verify-worktree.ps1`) and repaired from git;
  the damaged directory was kept as `services/core-broken-20261114`.

## 2026-11-14 · Housekeeping before A9
- **Kiro features:** agent-run diagnosis (read-only health checks written as scripts),
  a clean clone on another drive as the reference for byte-by-byte comparison.
- **Produced:** drive E: repaired with `Repair-Volume` (spot fix, then offline scan and
  fix; the volume now reports `Healthy`); the damaged local `.git` replaced with a clean
  copy; every tracked file restored and verified against GitHub. `tools/` reduced from 26
  to 13 scripts, with `tools/README.md`; `test-retry.ps1` now retries any executable that
  Windows Smart App Control blocks. A test (`tests/catalogue.rs`) replaces the
  `check-codes` script: every diagnostic code the compiler emits must be in the catalogue
  and documented in `docs/errors/`. The checker now records the type of every expression,
  which NIR lowering (A9) needs.
- **Lessons:** the failures were a loose USB connection plus file-system errors, not the
  compiler. Comparing content with a clean clone, instead of trusting timestamps, was the
  only reliable way to find damaged files.

## 2026-11-15 · Task A9 (NIR)
- **Kiro features:** spec-driven development (new spec `.kiro/specs/neuron-nir`,
  chapter 16 of the language specification, chapter 11 §11.1 rewritten as a summary),
  steering (`rust-style`: deterministic output, no `unwrap`, documented public items),
  property-based testing (`proptest`) for P-RT over mutated examples.
- **Produced:** the `neuron-nir` crate (data model, canonical JSON, reader with version
  check, JSON Schema, printer back to `.neuron`), lowering in `neuron-sema`
  (`Analysis::nir`), the published schema `spec/schemas/nir.schema.json` with a drift
  test, the golden file `conformance/golden/nir/examples.nir.json`, and the commands
  `neuron emit-nir` and `neuron print-nir`. Properties NIR-1 and P-RT hold on the
  examples package, each example, every valid spec program and generated variations.
- **Lessons:** the round-trip property found two real problems before any user did:
  `-40..40 mm` was printed in a form that reads back differently, and the unit offset
  was lost when reading JSON. Reviewing the NIR of the examples also showed that
  `ip(192.168.0.10)` was parsed as a number with fields; NIR now records an `ip`
  literal. The drive failed a third time during this task; the work was rescued to a
  clean clone on C:, committed from there, and E: was rewritten from GitHub.
- **Follow-up:** the README now shows the status after A9, the NIR commands, and the
  two code samples copied from `examples/` (checked in CI) instead of an older draft
  syntax.

## 2026-11-16 · Task A10 (reference interpreter)
- **Kiro features:** spec-driven development (new spec `.kiro/specs/neuron-interp`,
  chapter 17 of the language specification), steering (`rust-style`: no `unwrap`, bounded
  loops, deterministic order), property-based testing (`proptest`) for SAFE-1 and for
  totality on mutated programs, agent-run builds through logged scripts
  (`tools/build-check.ps1`, `tools/ship.ps1`).
- **Produced:** the `neuron-interp` crate: a discrete-event interpreter of NIR in simulated
  time with logical execution time for signals, interrupts, state machines, sequences,
  PLC scans, the safety logic with safe state and reset, function blocks, a model of
  devices outside the package, scenarios and `test` blocks; `neuron run` and
  `neuron test`; the prelude functions `drive`, `advance`, `elapsed`, `current_fault` and
  `state_of`.
- **Lessons:** the properties found their own mistakes. The first draft of the PLC test
  expected no output at the first scan, and the interpreter correctly published the
  initial image, so the spec now says so. Writing the SAFE-1 property forced the
  definition of "no output changes after a fault" to be precise. A publishing script that
  checks for deleted files and NUL bytes before committing replaces the earlier
  commit-everything script, after a damaged drive made a commit that deleted half of the
  repository (reverted without rewriting history).

## 2026-11-16 · Task A11 (formatter, explain, LSP)
- **Kiro features:** spec-driven development (new spec `.kiro/specs/neuron-tooling`),
  steering (`rust-style`), property-based testing (`proptest`) for formatter
  idempotence, token preservation and totality, agent-run builds through logged scripts.
- **Produced:** `neuron fmt` (token based, with a self-check that falls back to the
  input), `neuron explain NE####` (embeds `docs/errors`), `neuron lsp` (diagnostics,
  hover, go to definition, formatting over the whole workspace), a VS Code and Kiro
  extension in `editors/vscode`, and `docs/editor-support.md`. CI now checks that the
  examples are formatted and that their tests pass.
- **Lessons:** a tree-based printer was rejected because it drops comments; a token
  based one cannot lose them. The test that widens spacing found that a unit must stay
  one space from its number, also after a call such as `xyz(1, 2, 3) mm` (`NE0014`);
  the generator, not the formatter, was wrong. Line breaking at 88 columns is not
  implemented and is documented as such.

## 2026-11-16 · Task A12 (standard library, base layer)
- **Kiro features:** spec-driven development (new spec `.kiro/specs/neuron-stdlib`),
  steering (`rust-style`), property-based testing (`proptest`) for the differential
  tests, agent-run builds through logged scripts.
- **Produced:** `lib/`, a Neuron package of 13 modules in the layers core, units, time,
  logic, control and test (timers, edge detectors, latches, counters, PID with
  anti-windup, filters, hysteresis, debounce, ramp), 30 `test` blocks run by
  `neuron test lib`, the layering rule `NE0106` in the analyzer, and
  `crates/neuron-interp/tests/a12_stdlib.rs`, which feeds each library class and the
  matching native block the same random inputs and requires the same outputs.
- **Lessons:** the library type-checked and passed its tests almost at once, which says
  the language is usable for its own library. The differential test found that
  `use std.test.asserts` did not parse because `test` is a reserved word; the parser
  now accepts reserved words as later path segments. Native debounce reads the clock, so
  the test advances simulated time between steps.

## 2026-11-16 · Task A13 (validation corpus and freeze)
- **Kiro features:** spec-driven development (`.kiro/specs/neuron-corpus`), agent-run
  checks through `tools/corpus-check.ps1`, steering (`conventions`: English, spec first).
- **Produced:** 50 programs in `examples/corpus/` across the eight dialects, each with
  tests where behavior is observable, a Rust test that analyzes, runs and tests every
  file, CI steps for every folder, `docs/usability-review.md` and the specification at
  v1.0-rc1.
- **Lessons:** writing fifty programs from the specification found eight friction points
  (a keyword used as a task name, `|>` precedence, `u8 <` read as generics, PLC
  inputs in `drive`). They are listed with their decisions. The final freeze needs a
  reviewer who did not write the specification.
