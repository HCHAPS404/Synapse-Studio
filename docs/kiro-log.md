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
