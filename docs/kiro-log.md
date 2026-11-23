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

## 2026-11-16 · Task B1 (target model)
- **Kiro features:** spec-driven development (`.kiro/specs/neuron-targets`), steering
  (`conventions`: derive register facts with attribution, no vendor manuals copied),
  agent-run tests through logged scripts.
- **Produced:** crate `neuron-target` (TOML model, registry with the files embedded,
  validation, pin and peripheral binding, clock-tree, UART, timer and PWM solvers),
  `targets/` with 5 architectures, 7 chips, 8 boards and 4 runtimes
  (`tools/gen-targets.ps1` generates the regular ones), `NE0601`–`NE0608` with
  `docs/errors/targets.md`, and `neuron check` now runs the hardware checks.
  Tests: TGT-1, HW-1 and CLK-1 (property tests) and one test per code.
- **Lessons:** the UART default tolerance of 2 % in spec 07 would reject 115200 Bd on a
  16 MHz UNO (2.1 % off, which works in practice); it is now 3 % and the spec says so.
  Owning `timer0` for the runtime tick makes PWM on D5/D6 an `NE0601`, which is the
  honest answer.

## 2026-11-16 · Tasks B2 and B3 (MIR, C backend, runtime)
- **Kiro features:** specs (`.kiro/specs/neuron-codegen-c`), differential testing against
  the reference interpreter, golden files.
- **Produced:** crate `neuron-mir` (typed structured IR, lowering from NIR with
  `NE0610` for constructs outside the subset, verifier, printer), crate
  `neuron-codegen-c` (C99 emitter with checked integer arithmetic, source map, embedded
  runtime, gcc and avr-gcc drivers), `runtime/neuron-rt` (core scheduler, host port in
  simulated time, AVR port for the ATmega328P and ATmega2560), goldens in
  `conformance/golden/c/`, and `tools/install-toolchains.ps1` (gcc, avr-gcc and
  avrdude, every download checked with SHA-256).
- **Lessons:** the differential test against the interpreter found semantic differences
  that reading the code had missed: a task release at the same instant as a fault reset
  must be dropped, and a fault must abort the running activity. The oldest avr-gcc in the Arduino index (4.8.1) fails to
  link on Windows, so the installer pins 7.3.0.

## 2026-11-16 · Tasks B4 and B5 (build, flash, more boards)
- **Kiro features:** spec task lists updated with what is done and what is not.
- **Produced:** `neuron targets [board]`, `neuron emit-c`, `neuron build` (UNO, Mega,
  `--board`, `--host`) and `neuron flash` (avrdude, programmer from the board file),
  `crates/neuron-cli/tests/b4_cli.rs`, `docs/targets.md` with the tier table and the
  supported subset, and CI installing `gcc-avr` so these tests never skip there.
- **Lessons:** Pico, Pico 2 W, ESP32-S3, XIAO and NUCLEO boards are `described`: their
  pins, clocks and rates are checked and their C is generated, but they have no runtime
  port yet, and the tool says so instead of failing late. The UNO flash (gate G2) needs
  the board on USB.

## 2026-11-16 · Task B6 (instruction-set simulators)
- **Kiro features:** spec written for the task after the AVR work
  (`.kiro/specs/neuron-isa-simulators`), property tests.
- **Produced:** crate `neuron-sim-avr` (ATmega328P, cycle counted, the peripherals the
  runtime uses) whose tests run the avr-gcc firmware of six corpus programs and compare
  the pins with the interpreter millisecond by millisecond (MOC-A1, also on random
  inputs); crate `neuron-sim-thumb` (ARMv6-M core with SysTick, NVIC and an RP2040-style
  GPIO block) tested with hand-assembled programs and a flags property test.
- **Lessons:** the AVR simulator agreed with the interpreter on the first full run, which
  checks the AVR port, the compiler flags and the tick timer together without a board.
  The Thumb simulator cannot run Neuron firmware until the RP2040 runtime port exists.

## 2026-11-17 · Tasks B5 (ports), B6 (completion), B7, B8 and B9 (Phase B closed)
- **Kiro features:** spec-driven development (new specs `.kiro/specs/neuron-ports`,
  `neuron-hdl`, `neuron-native`, `neuron-ui`, written before each crate), steering
  (`rust-style`, `codegen-safety-rules`, `conventions`), property-based testing,
  differential testing against the reference interpreter, detached logged jobs
  (`tools/job.ps1`) after the editor terminal proved unreliable.
- **Produced:**
  - B5: runtime ports for RP2040 and RP2350 (RAM UF2, 12 MHz crystal), STM32F4 (HSI
    16 MHz, flash image) and an ESP-IDF project generator; `Tier::Sdk`; UF2 writer;
    `neuron build` and `neuron flash` dispatch by tier and programmer (`avrdude`, `uf2`,
    `swd`, `esptool`). The RP2040 firmware runs in `neuron-sim-thumb` and matches the
    interpreter, which closes B6.
  - B7: crate `neuron-codegen-hdl` (hdl dialect to Verilog-2005, source map,
    `NE0702` for clock-domain crossings without a synchronizer, `two_ff` synchronizer,
    XDC and PCF constraints for two generic FPGA boards, Icarus Verilog and Yosys
    drivers) and `neuron emit-verilog`.
  - B8: crate `neuron-native-avr`, a native ATmega328P backend (own assembler and Intel
    HEX writer) for an integer subset, run in `neuron-sim-avr` against the interpreter.
  - B9: crate `neuron-ui` (widget tree from the ui dialect, framebuffer or band render
    strategy chosen from RAM, `NE0604` when neither fits, 5x7 font, SSD1306, ST7735-class
    TFT and HD44780 drivers, C renderer in `runtime/neuron-rt/ui`, host emulators).
- **Lessons:** the tier line is drawn honestly: what runs without a board is verified in
  simulators, and the device steps (gate G2, I2 boards, I3 FPGA, ESP-IDF on a real
  ESP32-S3) are marked pending instead of done. A label collision between a signal and
  a task in the native backend (`task_` prefix) was found by the differential test and
  fixed with an `act_` prefix. A job runner that redirected only the last command of an
  `a && b` chain lost clippy output; the command is now grouped before redirection.

## 2026-11-19 · Workspace check, drive repair and the test scope fix (A10.12)
- **Kiro features:** agent-run readiness check (git, `fsck`, content comparison of every
  tracked file, toolchains, every test), spec-first fix (`spec/language/17-interpreter.md`
  §17.8, `.kiro/specs/neuron-interp` requirement I11 and task A10.12), a regression test.
- **Produced:** a loose USB cable had damaged one git object and one spec file on drive E:;
  both were restored from GitHub through a reference clone on C:, with a backup of the
  damaged copies, and `git fsck` and the 410-file comparison came back clean. `neuron test
  examples` now passes (18 of 18): each `test` block runs its own module and the modules
  it refers to, and CI runs the whole examples package too.
- **Lessons:** three long tests failed only when the whole package was tested because
  three corpus programs fault on purpose (`r02` at 1 s, `r03` at 1.8 s, `h05` at 2 s) and a
  fault stops the whole program. Testing each folder alone had hidden the cause instead of
  fixing it; the specification did not say which modules a test runs.

## 2026-11-19 · Tasks C1 and C2 (emitter framework, IEC 61131-3 emitters)
- **Kiro features:** spec-driven development (new spec `.kiro/specs/neuron-translate`
  written before any code, covering C1 to C6), steering (a new always-on
  `current-work.md` that hands the session state to the next chat), differential testing
  against the reference interpreter, agent-run builds through logged jobs.
- **Produced:** crate `neuron-st` (IEC 61131-3 ST subset: parser, and a virtual PLC that
  scans programs in simulated time with scan-counted or wall-clock timers); crate
  `neuron-emit` (emitter plugins with manifests, `fidelity.json`, source maps, wiring
  report, a project policy that refuses worse translations with `NE1003`) and its IEC
  emitters: Structured Text (generic and a Yaskawa MotionWorks-style profile), Siemens-style
  SCL with its tag table, and PLCopen XML with LD and SFC bodies. Diagnostics family NE10xx.
- **Lessons:** running the output instead of reading it settled the semantics: the
  interpreter advances function blocks at the end of the scan, so the emitter writes the
  block inputs to shadow variables and calls every block once after the rungs; with that,
  the ST of every `plc` program of the corpus gives exactly the interpreter's trace. With
  real IEC timers a flasher cycle is one scan longer per timer, which the test first
  expected to be 1000 ms and is in fact 1020 ms in the reference (two rungs pass the
  timer outputs on); the fidelity report now says so. The project moved from the USB
  drive E: to C: after `chkdsk` removed 65 files; all were restored from git first.

## 2026-11-19 · Task C3, stage 1 (robot semantics in the reference interpreter)
- **Kiro features:** sub-agents in a clean context (one read the robot path and found the
  gaps, a second fixed them), steering (`current-work.md` records the gaps and how to run
  jobs from the agent shell), a read-only `tools/diagnose.ps1` health check.
- **Produced:** geometry values (`joints`, `xyz`, `rpy`, `frame`, `tool`, `f.xyz(..)`,
  `p.above(d)`) in mm and deg; loops that wait inside a sequence are flattened, so `move`,
  `wait` and `until` in a `for` really wait; sequence locals live across ticks; `until`
  on an `io` input reads it; a `MOVE` trace entry for every motion (spec 17). Robot steps
  are handlers, entered only by a jump or a timeout. 11 tests over the six robot programs
  of the corpus, both branches of `r02` and `r03`.
- **Lessons:** 423 green tests had not caught that robot programs did not run as written:
  points were opaque, and a motion inside a loop did not wait. The SPEL+ differential test
  needs a correct reference, so the interpreter was fixed first. Launching jobs with
  `Start-Process` from the agent shell silently never ran them; long foreground waits
  froze the shell twice.

## 2026-11-19 · Task C3, stage 2 (robot manifests, SPEL+-style emitter, virtual controller)
- **Kiro features:** spec-driven development (the C3 mapping table of
  `.kiro/specs/neuron-translate/design.md` drove the emitter), a sub-agent in a clean
  context, differential testing against the reference interpreter.
- **Produced:** generic robot manifests `devices/robots/generic_6axis.toml` and
  `generic_scara.toml`; `NE0804` for constant targets outside the workspace (checked per
  iteration in loops with constant bounds) and a run-time `NeuronCheck` for computed
  targets; the `spel` emitter (program, `NeuronLib.prg`, the TCP slave `NeuronSlave.prg`,
  fidelity report, source maps, fault table in `wiring.md`); crate `neuron-spel` (parser
  and a virtual controller with a fixed time per motion).
- **Results:** every robot program of the corpus gives the same motions and output
  changes in the virtual controller as in the interpreter, both branches of `r02` and
  `r03` included: 68 motions and 23 output changes, instants exact, targets within 1e-6.
  `blend` is reported as `approximate`; motion time as `equivalent`. Workspace tests: 453.
- **Lessons:** dialect details that need the real controller (network statements, point
  coordinates, `Motor On`) are marked "to confirm in session I6"; the slave program is
  only checked as text until then.

## 2026-11-19 · Task C4 (Python emitter for host modules)
- **Kiro features:** spec-driven development (T5 of `neuron-translate`), a sub-agent in a
  clean context, differential testing against the reference interpreter.
- **Produced:** the `python` emitter: one standard-library-only file per `host` module
  plus `neuron_rt.py` (an event queue in integer nanoseconds, signals with logical
  execution time, tasks, events, faults and the safe state). `python m.py --test` runs the
  test blocks in simulated time, `--trace 3s` prints the interpreter's trace format,
  `--realtime` follows the wall clock. CI installs Python 3.12 and the test must not skip.
- **Results:** nine host programs (the five of the corpus and four written in the test):
  168 trace lines byte-equal to the interpreter, 8 test blocks with the same verdicts and
  messages. Workspace tests: 458.
- **Lessons:** a plain heap of events reproduced the interpreter's order exactly where an
  asyncio loop would have added its own ordering. `--ros2` is refused for now; timers,
  counters, `pid`, `fsm`, `sequence`, `struct` and `match` in host modules are `NE0610`.

## 2026-11-19 · Tasks C5.1 and C5.2 (ST, SCL, PLCopen XML and SPEL+ importers)
- **Kiro features:** spec-driven development (T6), sub-agents in a clean context (one for
  the importers, one for the two defects they exposed), round-trip testing by traces.
- **Produced:** crate `neuron-import` (`st`, `scl`, `plcopen`, `spel`): each writes
  formatted Neuron source, a fidelity report and `NE1001`/`NE1002` diagnostics. LD
  networks are walked from each coil back to the rail; SFC and CASE machines come back as
  `sequence`.
- **Results:** every `plc` program of the corpus round trips (Neuron, emitted, imported,
  analyzed and run gives the same trace) through ST generic and yaskawa, SCL and PLCopen:
  1036 output changes compared; every robot program through SPEL+: 68 motions and 23
  output changes. Hand-written ST and SPEL+ import with an entry for every construct.
  Workspace tests: 478.
- **Lessons:** the round trip found a defect of the reference interpreter: an `io` input
  inside an expression (`until not holding`, `holding == false`) was compared as a
  handle, not read; only a bare `until x` worked. Inputs are now read wherever a value is
  needed. A module called like a keyword (`cell`) is renamed and reported.

## 2026-11-20 · Task C5.3 (C subset and Verilog subset importers)
- **Kiro features:** spec-driven development (T6 and its out-of-scope list), a sub-agent
  in a clean context with strict editing rules, differential testing against the host
  compiler.
- **Produced:** `c` importer (integer functions with bounded loops become `core`
  functions; `+ - *` become wrapping operators, as in C) and `verilog` importer (one
  module with clocked non-blocking registers, `assign` and `case` becomes an `hdl`
  circuit). Everything outside the subsets is a located `NE1002`.
- **Results:** 16 imported C functions give the same results as the same C built with
  gcc on 572 input rows; the six Verilog designs of the corpus re-emit identically after
  import and give the same 4200 output values over 3600 simulated cycles. Workspace
  tests: 492.
- **Lessons:** Neuron's `for` gives its variable the type `index`, so counted C loops
  become bounded `while` loops; ports named `rst` that are not a plain synchronous reset
  are renamed, because the backend adds its own reset.

## 2026-11-20 · Task C6 (micro-kernel: state tables and the ST, C99 and SPEL+ kernels)
- **Kiro features:** spec-driven development (T7), a sub-agent in a clean context,
  differential testing across four executors.
- **Produced:** `kernel` emitter: `fsm` and `sequence` compile to a state table
  (`.nst.json` and a checksummed `u16` word encoding, `.nst.hex`), run by a Rust
  reference executor and by table-independent kernels in ST (`NEURON_KERNEL`), C99
  (`nrt_kernel.c`, built with `-Wall -Wextra -Werror`) and SPEL+. Tables beyond the
  limits are `NE1004`. `neuron-st` reads array elements in place; `neuron-spel` gained
  global arrays.
- **Results:** for the corpus traffic light and mixer and two synthetic machines, the
  reference executor matches the interpreter (state machines exactly, sequences within
  one scan) and the ST, C and SPEL+ kernels match the executor; 72 random tables agree in
  ST and C. Every flipped bit of a table is refused. Workspace tests: 502.
- **Lessons:** only two corpus programs fit the kernel; robot motions, device actions,
  reals and edge reads stay outside it and are `NE0610`. The SPEL+ kernel is checked on
  boolean ports only, because the controller's numbers are floating point.

## 2026-11-22 · Fix: ST lexer panic found by CI, and the promotion of `main`
- **Kiro features:** property-based testing (the E1 proptest found it on GitHub CI with a
  seed the local run never drew), steering (`current-work.md` now requires reading the CI
  result after every push), a release script.
- **Produced:** the ST lexer keeps a whole multi-byte character after `$` instead of
  stepping into it; a regression test with the exact input from CI.
  `tools/release-main.ps1` verifies, commits, pushes, waits for a green CI on `develop`,
  fast-forwards `main` and tags it.
- **Lessons:** two commits were reported green from local runs while GitHub CI was red;
  `main` is promoted only after the CI of the `develop` tip passes. Two test runs at the
  same time collide on `out/c6/driver.exe`; run one at a time.

## 2026-11-21 · Task E1 (compiler bindings: WebAssembly and Python)
- **Kiro features:** a sub-agent in a clean context, property-based testing of every entry
  point with random and mutated input.
- **Produced:** crate `neuron-wasm`: a plain-Rust JSON API (`check`, `format`, `hover`,
  `definition`, `run`, `test`, `emit` for all seven emitters, `import`, `version`) and its
  wasm-bindgen exports; crate `neuron-py` (PyO3, built with maturin, outside the
  workspace build); `packages/wasm-glue/README.md` for Martin. The LSP hover and
  definition logic became public functions so both the LSP and the bindings use it. CI
  builds `neuron-wasm` for `wasm32-unknown-unknown`.
- **Results:** 22 tests compare every API result with the crate it wraps; malformed input
  never panics. The wasm32 release build passes locally. Workspace tests: 578.
- **Lessons:** the workspace `forbid(unsafe_code)` cannot hold in crates where
  wasm-bindgen and PyO3 generate unsafe glue; those two use `deny` and allow it only in
  the generated module. The browser glue and the Python wheel are not run yet.

## 2026-11-21 · Task E4 contract (relay and twin gateway protocol)
- **Kiro features:** spec-driven development (the contract is written before the relay
  server), a sub-agent in a clean context, property-based testing, single source of
  truth (Rust types generate the JSON Schemas; a test fails on drift).
- **Produced:** crate `synapse-protocol` (versioned messages tagged by `type`: session,
  subscriptions, cell snapshot, telemetry, trace, simulation control, replay, deployment
  status, advisory E-STOP), `spec/protocols/relay.md`, `spec/schemas/relay/` and 28
  example messages. The hardwired E-STOP is the authority: the only E-STOP message a
  client sends is an advisory request, and no message can disable a safety function.
  The replay format is the one the Phase D co-simulation will write.
- **Results:** every example parses and re-serializes to the same JSON; invalid messages
  are rejected; workspace tests: 554.
- **Lessons:** the property test found that a telemetry real lost its last bit through
  JSON; `serde_json`'s `float_roundtrip` fixes it. Turning that feature on rebuilt the
  whole workspace with one compiler per core and ran the PC out of memory (os error
  1455); `tools/job.ps1` now limits cargo to two jobs. `tools/setup-kiro.ps1` sets the
  Kiro permissions and Autopilot, which the agent cannot edit itself.

## 2026-11-20 · SPEL+ kernel on integer ports, and the Phase D spec
- **Kiro features:** spec-driven development (new spec `.kiro/specs/neuron-cell` written
  before any Phase D code), a sub-agent in a clean context, steering (the decisions
  waiting for review are in `current-work.md`).
- **Produced:** `neuron-spel` gained word I/O (`OutW w, v`, `InW(w)`, to confirm on the
  real controller in session I6); the SPEL+ micro-kernel is compared with the reference
  executor on integer inputs and outputs, including 16-bit wrapping. Spec for the cell
  compiler: manifests, partitioner, link synthesis and timing, cell safety,
  co-simulation with a replay format for the twin, and the technical dossier.
- **Results:** 125 integer output changes over 10 schedules equal the reference
  executor. The spec lists three design decisions for review before link synthesis.
- **Lessons:** the language and NIR already carry every cell construct; what was missing
  is the compiler that checks them. `NE0701`, `NE0703`, `NE0801` and `NE0805` were
  catalogued in chapter 10 but no code emits them yet.

## 2026-11-20 · C99 emitter for `core` modules
- **Kiro features:** a sub-agent resumed the half-written code of an aborted one in a
  clean context, differential testing against the host compiler and the interpreter.
- **Produced:** `neuron emit c`: `core` functions become portable C99 (`<module>.c`,
  `.h`, `nrt_core.h`). Overflow, division by zero and a false `requires` set `nrt_fault`
  with the interpreter's fault name; units are erased to `double` in base SI units.
- **Results:** 754 calls on 13 programs (8 written in the test, 3 of the corpus, 2 of
  `lib/core`) built with `gcc -std=c99 -pedantic -Wall -Wextra -Werror` give the
  interpreter's results; all 33 fault cases fault with the same code. Workspace tests: 540.
- **Lessons:** arithmetic must keep the type the analyzer gave it: `x & ((1 << n) - 1)`
  with `n = 32` faulted in C and not in the interpreter until the context type stopped
  leaking into the operation.

## 2026-11-20 · Python emitter, second pass (blocks, fsm, sequence, types, ROS 2)
- **Kiro features:** a sub-agent in a clean context with strict file-editing rules,
  differential testing against the reference interpreter, steering (the hardware plan
  and the rule "work that needs no device is done in the repo" are in `current-work.md`).
- **Produced:** the `python` emitter now translates `ton`/`tof`/`tp`, counters, edge
  detectors, `sr`/`rs`, `pid`, `fsm` (entry, exit, `do`, `after`, `when`, `every`, `on`),
  `sequence` (waits, timeouts, loops that wait), `struct`, `enum` with payload, `match`
  with guards, `Ok`/`Err`/`none`/`?` and `requires`/`ensures`/`old`. `--ros2` writes a
  `<module>_ros2.py` node (a publisher per signal, a subscriber per input or event).
- **Results:** six new host programs: 655 trace lines byte-equal to the interpreter and
  20 test blocks with the same verdicts. The ROS 2 node publishes the interpreter's signal
  changes against a stub of `rclpy`. Workspace tests: 524.
- **Lessons:** the stub proves the node's logic, not ROS 2. It is recorded as an
  `equivalent` fidelity entry and as an open item for session I4 on the Raspberry Pi 5.
  Still `NE0610`: `mut` parameters, string interpolation, bit vectors, lambdas, classes.

## 2026-11-20 · Task C7 (command line, user guide, CI) and close of Phase C
- **Kiro features:** spec-driven development (T8 and T9), steering (`current-work.md`
  hands over the closed phase and the next steps), a sub-agent in a clean context.
- **Produced:** `neuron emit <st|scl|plcopen|spel|python|kernel>` and `neuron import
  <st|scl|plcopen|spel|c|verilog>` (module `translate.rs`; emit applies the fidelity
  policy and writes nothing on `NE1003`), `docs/translation.md`, smoke steps in CI and
  `tools/phase-check.ps1`, the roadmap and README status.
- **Results:** 15 CLI tests (every emitter, every importer, policy, `NE0610`, `NE0804`,
  `NE1001`, and a round trip that ends in `neuron check`). Phase C in total: 517
  workspace tests, up from 423 at its start.
- **Lessons:** the phase found three defects of earlier phases that 423 green tests had
  not caught: robot programs did not run as written, motions in loops did not wait, and
  an `io` input inside an expression was not read. Each was found by comparing a
  translation with the reference interpreter, not by reading code. What is not proven:
  nothing was run on a real PLC, robot or board (sessions I5 and I6).

## 2026-11-22 - Cloud setup: Neon and Render
- **Kiro features:** steering (`current-work.md` records the state), the Neon agent skills and
  MCP server installed by `neon init`, the file tools to read the Core API before advising.
- **Produced:** Neon project `synapse-studio` (Ohio) linked in `services/core` (`neon.ts`,
  `.neon`), and the Render blueprint service `synapse-core` (Starter) from `render.yaml`.
- **Results:** `GET /health` on the Render URL returned `status: ok` with `database: ok`
  (about 436 ms on the first call, which includes waking Neon). Vercel is not set up yet:
  `apps/web` does not exist on `main`.
- **Lessons:** the Blueprint was created from a commit without `region`, so Render runs in
  Oregon while Neon is in Ohio. Render cannot move a service, so it stays as a TODO in
  `render.yaml`. `neon init` minted an account-wide API key for the MCP server; revoke it
  when setup ends. A restricted PowerShell execution policy blocked `npm.ps1`, which also
  explains the agent shell failures.

## 2026-11-22 - Web skeleton (`apps/web`) that reads the Core API health
- **Kiro features:** spec-driven development (`.kiro/specs/web-skeleton/`, W1 to W4),
  steering (TypeScript style, `current-work.md`), agent permissions (they denied writing
  `.env.example`, so the variable is documented in the README instead).
- **Produced:** a self-contained Next.js 16 app: `src/lib/config.ts` (validated
  `NEXT_PUBLIC_API_URL`, required in production), `src/lib/health.ts` (server-side fetch,
  no cache, 8 s timeout, never throws, runtime shape check), `src/app/page.tsx`.
- **Results:** `pnpm lint`, `pnpm typecheck` and `pnpm build` pass. A production server
  built against the Render URL rendered "Connected - status ok", database ok (474 ms).
- **Lessons:** the newest majors were not usable together: typescript-eslint rejects
  TypeScript 7 and the React ESLint plugins stop at ESLint 9, so both are pinned one major
  lower. Next.js inlines `NEXT_PUBLIC_*` at build time, so Vercel needs a redeploy after
  the variable changes.

## 2026-11-22 - README brought up to date
- **Kiro features:** steering (`current-work.md` and the roadmap as the sources of status).
- **Produced:** status changed from "early development" to "alpha" (badge and notice, with
  what works and what does not); a "Milestones and changes" section per phase and tag;
  progress table as of 22 nov (Phase C 7/7, total 37 of 90 tasks, 41 %); deployed Core
  API, web app commands, Render region TODO and Vercel as the open account step; Phase D
  and E headers, E7 row (also in `docs/roadmap.md`), risks and Gantt updated.
- **Lessons:** the progress table had stayed at 17 nov and Phase C was listed with six
  tasks instead of seven; status now has to be updated with every finished task.

## 2026-11-22 - Vercel live and CORS fixed
- **Kiro features:** steering (`current-work.md`), spec `web-skeleton` (W4 closed).
- **Produced:** the web app on Vercel at https://synapse-studio-eight.vercel.app, checked
  from outside: "Connected - status ok", database ok (431 ms). `SYNAPSE_CORS_ORIGINS` in
  `render.yaml` now names that origin.
- **Lessons:** the origin first written in `render.yaml` (`synapse-studio.vercel.app`) is
  a different, unrelated project, so the API allowed browser calls from a third party.
  Only `/health` existed, so nothing was exposed. Check who owns a URL before trusting it.

## 2026-11-22 - Web domain moved to synapsest.vercel.app
- **Produced:** the Vercel production domain is now https://synapsest.vercel.app, checked
  from outside ("Connected", database ok, 433 ms); the old -eight URL answers 307 to it.
  `SYNAPSE_CORS_ORIGINS` lists both origins; README, roadmap and steering use the new one.
- **Lessons:** `*.vercel.app` names are shared by every Vercel user: `synapsestudio` was
  taken by a stranger's site. Check a name with a request before choosing it.

## 2026-11-22 · Task D1.1 (device and medium manifests)
- **Kiro features:** spec-driven development (`.kiro/specs/neuron-cell/`, D-R1 and D-R11),
  a sub-agent in a clean context for the first draft, steering (Rust style, codegen
  safety rules: generic manifests, no vendor data).
- **Produced:** crate `neuron-cell` (manifest schema, loader, unit checks, cell
  resolution), `devices/media/{modbus_tcp,s7,zero_hop}.toml`, robot manifests extended
  with device, I/O, timing and safety tables; `NE0607` widened to device manifests.
- **Results:** 11 tests (3 property tests) and 6 doctests pass; the C3 robot tests of
  `neuron-emit` still pass on the extended manifests; fmt and clippy clean.
- **Lessons:** the sub-agent lost its shell halfway; its code compiled, and the only
  failure was a test that added an unknown table on purpose to a case that checked a
  different error. A manifest that refuses unknown fields has to be tested one error
  at a time.

## 2026-11-22 · Task D1.2 (manifests for every cell device)
- **Kiro features:** spec-driven development (D-R1), steering (no vendor data, generic
  numbers with the session that measures them).
- **Produced:** manifests `generic_s7_1200` (SCL), `generic_iec_plc` (ST, `yaskawa`
  profile), `nucleo_f446re` (C) and `generic_artix7_board` (Verilog), embedded in the
  built-in library; test `d1_cells.rs`.
- **Results:** the four corpus cells and the bottling example resolve every device and
  medium; `conveyor_plc` builds for two PLC vendor classes and gives two programs.
  neuron-cell: 13 tests and 6 doctests pass; fmt and clippy clean.
- **Lessons:** the bottling example only analyzes as part of the `examples` package
  (it places `thermostat` and `conveyor` from other files), so cell tests load packages,
  not single files.

## 2026-11-22 · Task D2.1 (partitioner)
- **Kiro features:** spec-driven development (D-R2; the rules the design left open are
  recorded in "Results of D2.1"), steering (deterministic output, typed `NE####` codes).
- **Produced:** `neuron-cell::partition` with `CellPlan`, `DevicePlan`, `Placed`;
  `CellDiag`; codes `NE0704` and `NE0705` in chapter 10, `docs/errors/cell.md` and
  `neuron explain`.
- **Results:** the bottling cell partitions into four programs (two explicit, two
  automatic placements, the `Cycle` sequence on the PLC); 8 tests including a property
  test that the plan does not depend on the order of the source files; the diagnostics
  catalogue test still passes.
- **Lessons:** the spec said "automatic assignment" but not which modules are
  candidates. Without a rule, every module of a package would be pushed into the cell
  (the examples package also holds an UNO blink and an ESP32 panel); the board named
  after `on` is what keeps them out.

## 2026-11-22 · Task D2.2 (build per device)
- **Kiro features:** spec-driven development (D-R2), steering (deterministic output,
  generic manifests).
- **Produced:** `neuron-cell::build` (`CellBuild`, `DeviceBuild`, `device_package`)
  over the C, Verilog and Phase C backends; manifests `arduino_uno_r3` and
  `raspberry_pi_5`.
- **Results:** a cell with one device of each class builds to SCL, C, SPEL+, Verilog
  with XDC and Python, deterministically; neuron-cell: 25 tests and 7 doctests pass.
- **Lessons:** building the bottling example found two limits of earlier backends that
  no test had hit: the C backend has no function blocks, so the PID thermostat of the
  README does not compile to C, and the example's pick program is six-axis while the
  cell's arm is a SCARA. They are reported per device and kept as open items.

## 2026-11-22 · Task D3.2 (link budgets and NE0701)
- **Kiro features:** spec-driven development (D-R4, spec 14.8), steering (every number
  traceable to a manifest, trust level shown).
- **Produced:** `neuron-cell::link_table` (`LinkReport`, `Latency`, `Alternative`),
  `NE0701` implemented (`docs/errors/cell.md`), robot placeholder timing adjusted.
- **Results:** the five corpus and bottling links meet their budgets; a 200 us Modbus
  link between two controllers is `NE0701` and proposes zero_hop at 150 us; 6 tests
  with a property test of the formula and the check.
- **Lessons:** with the first placeholder numbers the README's own bottling cell broke
  its 25 ms budget by 1 ms. Placeholder numbers decide whether an example compiles, so
  they are labeled `generic` in every message until session I1 measures them.

## 2026-11-22 · Task D4.2 (electrical compatibility, NE0801)
- **Kiro features:** spec-driven development (D-R7), steering (codegen safety rules:
  never wire 3.3 V logic to 24 V I/O; the tool is a design aid).
- **Produced:** `neuron-cell::check_electrical`; `NE0801` implemented
  (`docs/errors/cell.md`); spans kept in the link table.
- **Results:** a 3.3 V FPGA pin on a 24 V PLC input and a 3.3 V board on a 5 V board
  are `NE0801` with the fixed `via`; `opto_24v` makes them legal; the corpus and
  bottling links pass; 5 tests.
- **Lessons:** a test that asked for a 1 ms link into an UNO got an honest `NE0701`
  first: the budgets of D3.2 now catch mistakes in the tests of later tasks.

## 2026-11-23 · Task D3.1 (link endpoints, register maps, Neuron Wire)
- **Kiro features:** spec-driven development (D-R3, design decision 1), steering
  (codegen safety rules: latch + ACK across clock domains, no unbounded waits), property
  tests for the Wire properties of spec 11.3.
- **Produced:** `neuron-cell::{link_plans, endpoint_sources, register_map}` and the
  `wire` module (frame codec, CRC-16/CCITT-FALSE, COBS, TCP prefix, `DedupFilter`);
  `build` adds one generated Neuron endpoint module per device and link side to each
  device package and writes `links.json` and `register_map.csv`; the link table uses the
  real value size and the receiver's program period.
- **Results:** the endpoints of x01, x02, x04 and of a five-class cell with four links
  (S7-class PLC, UNO, six-axis arm, FPGA, Pi 5) reach the SCL, C, SPEL+-style, Verilog
  and Python programs of both ends, with no fallback note; WIRE-1, WIRE-2 and WIRE-3 hold
  under proptest; 19 new tests, 55 in the crate, 9 doctests. No new diagnostic code.
- **Lessons:** the SPEL+ emitter refuses `signal` but takes `var`, so the endpoint
  generator picks the construct per dialect and a refusal becomes a note, not a failed
  build. A PowerShell session keeps its variables between calls: a stale `$t` was
  written over a new file once, so file edits stay with the file tools.

## 2026-11-23 · Task D3.3 (cell watchdog and safe state on loss)
- **Kiro features:** spec-driven development (D-R5, MOC-C2), steering (codegen safety
  rules: safe state on fault, no unbounded waits), property test of the bound.
- **Produced:** `neuron-cell::watchdog` (`WatchdogPlan`, `DeviceWatchdog`,
  `watchdog_plans`); receiver endpoint modules declare `l<i>_alive` and `l<i>_lost` and
  raise `LinkLost` after 50 ms without a value; `LinkPlan.watchdog`, `CellBuild.watchdogs`
  and a `watchdog` entry in each `links.json`.
- **Results:** the PLC (SCL/ST), C, host and Verilog endpoints take the watchdog with no
  fallback note, and the ST program has `neuron_fault := TRUE` for it; the bound is
  watchdog + manifest safe-state time (60 ms for the S7-class PLC, 100 ms for the arm);
  7 new tests, 62 in the crate. The robot has no watchdog logic (the SPEL+ emitter has no
  rules): the plan says so, open for session I6.
- **Lessons:** the robot emitter translates only `sequence`, `var`, `io` and `safe_state`,
  so the generator declares the flags and states what is missing instead of inventing a
  pin for `until`. The bottling cell still does not build, so its watchdog plan is tested
  from the analysis, not from a build. Reading and rewriting a UTF-8 file from PowerShell
  without an encoding mangled it; docs go through the file tools too.

## 2026-11-23 · Task D3.4 (tags across devices, NE0703, single writer)
- **Kiro features:** spec-driven development (D-R6, spec 11.2), the error catalogue
  tests, property tests for the address rule and the writer rule.
- **Produced:** `neuron-cell::{check_tags, parse_direct_address, address_fits,
  type_bits}`; `NE0703` documented in `docs/errors/cell.md` (moved from `reserved.md`).
- **Results:** the tags of x01, x04 and the bottling package check; a unit mismatch, a
  type mismatch, a wrong address width, a missing address family and two writers (by
  address and by symbol) each give `NE0703`; the writer result is the same for any
  order of the locations; 14 new tests, 76 in the crate, catalogue and explain green.
- **Lessons:** an unknown device in a tag is stopped by the analyzer (`NE0101`) before
  the cell check, so `NE0607` stays as a guard. Statement walking through the
  serialized NIR (`"s": "assign"`) is enough for "is this name assigned" and avoids a
  dependency on the analyzer's private fact base.
## 2026-11-23 · Task D4.1 (zones, mutexes, robot targets in zones, NE0805)
- **Kiro features:** spec-driven development (D-R7, design D4), the error catalogue tests,
  property tests for the arbiter, a sub-task split that put the robot target machinery
  in `neuron-emit` and the cell rules in `neuron-cell`.
- **Produced:** `neuron_emit::spel::{motion_targets, MotionTarget}` (the constant targets
  of every `move`, in the robot base frame, resolved as for `NE0804`);
  `neuron-cell::{zone_plans, zone_sources, arbitrate, lock_signal, ZonePlan, ZoneBox,
  ZoneEntry, EntryKind, Party, LockLink}`; `CellBuild.zones` and `zones.json` per party
  device; lock modules under `links/lock_*.neuron`; `NE0805` in `docs/errors/cell.md`
  (moved from `reserved.md`); the example `examples/press_plc.neuron`.
- **Results:** x02 plans one lock owned by the PLC with a Modbus TCP lock transport; the
  lock modules (arbiter rungs on the owner, request/grant `var`s on the arm) build with the
  device programs; a motion into the zone by a robot that is not a party, a missing party
  output, a missing zone and an `hdl` party are `NE0805`; 11 new tests in `d4_zones.rs`
  including the property LOCK-1 (at most one grant, no grant without a request, ties to
  the first party). The user's sequences are not rewritten: acquire/release around an
  entry is a contract listed in `zones.json`, not inserted code.
- **Lessons:** the bottling example named `plc.press_cylinder` but no PLC module declared
  it, so the new check found a real gap; a module with that output was added. Replacing
  text in source files through PowerShell string methods silently did nothing twice
  (no match), so every edit was re-read. The new example also changed the NIR and parser
  golden files, which the first commit missed and the amend fixed.
## 2026-11-23 · Task D4.3 (cell verify over the composed machines, shortest counterexample)
- **Kiro features:** spec-driven development (D-R7, spec 08 §8.3 and 15 §15.6), property
  tests (CEX-1 minimality against a closed-form chain, MUTEX-1 over lock delays), the
  existing A8 model checker reused instead of a second one.
- **Produced:** `neuron_sema::{verify_cell, CellModel, CellMember, CellChannel, ChannelEnd,
  CellVerification, Proved}` (the checker takes members and channels; module verification
  is unchanged); `neuron-cell::{verify_cell, prove_zone, prove_zone_with, CellVerdict,
  ZoneProof, CellCounterexample, CellStep, Handshake}`; spec text in 15-analysis.md.
- **Results:** x02's mutex is proved at its real lock delay (two ticks), at four ticks and
  with three parties; the two-phase handshake is refuted at a 30 ms delay with a shortest
  trace, so the four-phase protocol is justified by the checker; a cell with two sequences
  joined by a link proves one property and gives a two-step counterexample for another;
  the delay bound decides an `eventually_within`; a one-tick pulse is not lost.
- **Lessons:** the first channel model let the destination skip samples, which loses
  pulses and made the proofs too weak; the lossless FIFO fixed it. Putting every link
  delay into the greatest common divisor of the durations made the tick 400 ns and the
  search explode, so delays are now rounded up to a tick chosen from the scan periods.
  Properties of a cell cannot be checked by `analyze` alone, so it now defers them.

## 2026-11-23 · Task D6.1 (co-simulation on the cell clock, link models, cuts, partition equivalence)

- **Kiro features:** spec-driven development (D-R8, MOC-C1, MOC-C2, partition
  equivalence, design decision 2), property-based tests with proptest, steering
  (rust-style, conventions), agent hooks for the interpreter, long-term memory notes.
- **Produced:** `neuron-cell::{cosimulate, cosimulate_single, SimOptions, SimRun, Cut,
  Drive, LinkModel, CellEvent, Delivery, LossReport}` in `cosim.rs`; in `neuron-interp`
  the hooks `next_event_ns`, `read_named`, `write_named`, `advance_clock`
  (`hooks.rs`); `tests/d6_cosim.rs`.
- **Results:** four corpus cells and the five-class cell run; deliveries take exactly the
  link latency, inside `within`; a cut faults a PLC or host receiver within watchdog plus
  safe-state time; traces are byte-identical across runs; two host modules joined by an
  ideal link behave like one interpreter over random periods.
- **Lessons:** a write between steps needs the clock moved first or its trace entry gets
  the previous instant, hence `advance_clock`. The corpus cells place no modules, so
  the value that crosses is the endpoint's own signal; connecting user symbols is done
  by the co-simulation and not by rewriting generated modules. A robot receiver has no
  watchdog logic, and the run reports that instead of passing.

## 2026-11-23 · Task D6.2 (replay format and JSON Schema for the twin)

- **Kiro features:** spec-driven development (D-R8), schema generation from Rust types
  (schemars, as for NIR), property-based tests with proptest, steering (rust-style).
- **Produced:** `neuron-cell::{Replay, ReplayEvent, ReplayEventKind, ReplayDevice,
  ReplayLink, replay_schema_json}` in `replay.rs`; `spec/schemas/replay.schema.json`;
  `tests/d6_replay.rs` with a checker for the schema subset schemars writes.
- **Results:** a co-simulation run and a D4.3 counterexample both become replays that
  validate, round trip byte for byte and are deterministic; folding the replay's state
  events at the violation instant gives the states of the last counterexample step; the
  committed schema equals the generated one.
- **Lessons:** with no validator crate in the workspace, a 60-line checker over the
  schema subset was enough to reject an unknown field, kind, missing field and wrong type,
  which makes "validates" a real test. A counterexample is a model trace, so its replay
  proves the violation in the model and not in the device programs.

## 2026-11-23 · Task D5.1 (technical dossier)
- **Kiro features:** spec-driven development (D-R9, design decision 3), property-based
  tests with proptest, steering (rust-style, conventions), a sub-agent with strict
  shell rules (file tools for edits, foreground cargo).
- **Produced:** `neuron-cell::{dossier, Dossier, DossierOptions, BuildStatus,
  SAFETY_WORDING}` in `dossier.rs`; `tests/d5_dossier.rs`; the generated dossiers of the
  `Lab` and bottling cells can be written with `NEURON_DOSSIER_OUT=dir`.
- **Results:** Markdown and one self-contained HTML (inline CSS and SVG, no scripts, no
  external resources) from one block model, byte-identical across runs; the link-table
  cells equal `link_table`; the bottling dossier marks `ctl` (`NE0610`) and `arm`
  (`NE0610`, `NE0804`) as failed and still shows links, zones, watchdogs and the mutex
  proof; the `Lab` dossier is complete, with a delivery at 23.005 ms and a `LinkLost`
  after a cut.
- **Lessons:** rendering one block list twice keeps both formats equal without a second
  source of text. The build returns one result for the whole cell, so the files of the
  devices that did build are not kept when another fails; the dossier says so instead of
  hiding it. The default co-simulation has no stimuli, so the test report needs drives
  to show a delivery.

## 2026-11-23 · Task D7 (cell command line, guide, CI) and close of Phase D
- **Kiro features:** spec-driven development (D-R10, `.kiro/specs/neuron-cell/`), steering
  (rust-style, conventions, current-work), strict shell rules for this machine (file tools
  for every edit, one foreground cargo at a time).
- **Produced:** `crates/neuron-cli/src/cell.rs` (`neuron cell check|build|sim|dossier`,
  `neuron check` also runs the cell checks), `crates/neuron-cli/tests/d7_cell.rs` (20 tests),
  `docs/cells.md`, CI and `tools/phase-check.ps1` smoke steps, roadmap, README and
  `tasks.md` ("Results of D7" and "Phase D closed").
- **Results:** every command runs on the corpus cells; `--cut` takes `l0@50ms` or
  `a.x->b.y@50ms`; one CLI test per diagnostic family (`NE0607`, `NE0701`, `NE0703`,
  `NE0704`, `NE0705`, `NE0801`, `NE0805`, `NE0901`); a build or dossier twice gives the same
  bytes; `neuron check examples` stays green. Phase D is closed in software; D-HW (timing
  profiles in I1, the cell on real devices in I4 to I7) stays open.
- **Lessons:** moving the `verify:` of a cell out of the analyzer (D4.3) meant `neuron check`
  had to call the cell checks, and the partitioner's `NE0705` is noise in a package that
  holds the whole corpus, so `neuron check` leaves unplaced modules out with a note and
  `neuron cell check` reports them. The stale NIR golden (press_plc reformatted) showed up
  only when the wider suites were run, so run the golden tests after any change to `examples/`.

## 2026-11-23 · Task E2.1 (service layout, errors, logging, database baseline)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules,
  steering for Python style.
- **Produced:** `services/core` modules `errors.py`, `logging_config.py`, `middleware.py`,
  `schemas/errors.py`, `db/`, Alembic baseline `0001`; `docs/errors/services.md`, chapter 10
  rows NE1101..NE1112, `explain.rs` embedding; CI Postgres service container.
- **Results:** 23 Python tests pass (2 Postgres tests wait for CI); ruff clean;
  `cargo test -p neuron-sema --test catalogue` and the `explain` tests pass; `/health` unchanged.
- **Lessons:** a property test of redaction first failed because the generated password
  was a substring of "postgresql"; generated secrets need a prefix that cannot occur in
  the surrounding text. Writing source from the shell added a byte-order mark: use the file tools.

## 2026-11-23 · Task E2.2 (`--format json` for check, build and emit)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules.
- **Produced:** `crates/neuron-cli/src/report.rs`, `tests/e2_json.rs`, `docs/cli-json.md`,
  `--format` in the usage text of `check`, `build` and `emit`.
- **Results:** 13 new CLI tests and 5 parser unit tests pass with every older `neuron-cli`
  suite; clippy and fmt clean. Human output is unchanged and still the default.
- **Lessons:** shadowing the print macros in the binary gave structured output without
  touching about sixty print sites. The price is that the JSON diagnostics come from parsing
  the rendered text, which a test pins against the human mode.

## 2026-11-23 · Task E2.3 (compile runner)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules.
- **Produced:** `services/core/src/synapse_core/compile/` (models, paths, sandbox, runner),
  `/health` field `sandbox.network`, settings for the compiler, fake `neuron` and `unshare`
  test tools, 55 tests, updated `NE1101` and `NE1102` documentation.
- **Results:** 78 Python tests pass and 6 skip on Windows; ruff clean. The tests that need
  POSIX (group kill of a grandchild, CPU limit, links) and the real binary run only on
  Linux CI or with `NEURON_BIN`; Docker was not running here, so they are not yet run.
- **Lessons:** a newer ruff flags blocking file operations inside `async` functions
  (`ASYNC240`), so directory preparation, cleanup and artifact reading moved to
  `asyncio.to_thread`. A fake `unshare` that marks the environment lets the self-test
  logic run without privileges.
## 2026-11-23 · Task E2.4 (projects, immutable versions and files)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules.
- **Produced:** migration `0002_projects`, ORM models, `services/projects.py`, `api/projects.py`,
  `schemas/projects.py`, `tests/conftest.py`, `tests/test_projects.py`, `NE1107` text.
- **Results:** 12 new tests (hash properties and routes). The route tests ran against SQLite
  locally and run against Postgres through Alembic in CI; the migration matches the models.
- **Lessons:** with no Postgres at hand, `JSON`-free portable column types let the same models
  run on SQLite for a quick check, but that proves the logic only, not the migration on
  Postgres.
## 2026-11-23 · Task E2.5 (builds API on the runner)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules.
- **Produced:** migration `0003_builds`, `services/builds.py` (`BuildManager`),
  `services/capabilities.py`, `api/builds.py`, `api/deps.py`, `schemas/builds.py`,
  `GET /v1/capabilities`, `runner.validate`, `tests/test_builds.py`.
- **Results:** 13 new tests (21 in the two suites) pass; model matches the migration;
  offline SQL confirms JSONB and BYTEA on Postgres. SQLite locally, Postgres in CI.
- **Lessons:** a gated fake runner (an injected subclass holding a `peak` counter) proves
  the semaphore bound without timing. Storing artifact bytes in a table keeps one Render
  instance simple; the input and output caps already bound the size.
## 2026-11-23 · Task E2.6 (device catalog from the manifests)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules.
- **Produced:** `synapse_core/devices/` (`models.py`, `catalog.py`), `api/devices.py`,
  `devices_dir` setting, `tests/test_devices.py`.
- **Results:** 20 new tests (one per manifest file, catalog/route tests, override,
  rejections); 101 pass, 21 skip; ruff clean. Read-only, no database.
- **Lessons:** the Python models mirror `manifest.rs` but keep durations as strings, since
  the Rust side owns the authoritative numeric parsing; duplicating it would risk drift.
  `devices/` is outside the Render `rootDir`, so a package-relative default locates it and
  the E2.10 image must copy it in.
## 2026-11-23 · Task E2.7 (token bucket rate limiter)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules.
- **Produced:** `synapse_core/limits.py` (token bucket, scopes, `scope_for`, `client_key`),
  `ratelimit.py` (`RateLimitMiddleware`), six `ratelimit_*` settings and a `limiter`
  property, wiring in `main.py`, `tests/test_ratelimit.py`.
- **Results:** 9 new tests pass, including a Hypothesis property that the admitted count
  over any window never exceeds `capacity + rate * elapsed`; full suite 110 pass, 21 skip;
  ruff clean. `/health` and schema paths exempt; `NE1103` with `Retry-After` on refusal.
- **Lessons:** the limiter reads `app.state.rate_limiter` on each request instead of a
  captured reference, so tests inject a small-capacity limiter without touching the
  startup path; `create_app()` builds its limiter from `get_settings()`, which the
  dependency override does not reach, hence the state hook. Keeping `limits.py` free of
  FastAPI makes the pure token-bucket property test trivial to drive with a fake clock.
## 2026-11-23 · Task E2.8 (guest mode)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules.
- **Produced:** `GuestSandbox` model + migration `0004_guests`, `auth/tokens.py` (HMAC
  guest token), `services/guests.py` (sandboxes, quotas, cleanup), `schemas/guests.py`,
  `api/guests.py` (`POST /v1/guest`), `optional_guest` dependency, guest binding and quota
  checks in projects and builds, eight new settings, a lifespan cleanup task,
  `tests/test_guests.py`.
- **Results:** 10 new tests, including an end-to-end guest build of a corpus example
  through the fake runner; migration round-trip verified on SQLite; full suite 114 pass,
  26 skip locally; ruff clean. `NE1104` on quota and on an expired/missing sandbox.
- **Lessons:** a stateless signed token needs no store to recognise a guest, but loading
  the sandbox row anyway lets a token outliving its deleted sandbox fail cleanly as
  `NE1104`. A `reserve(session)` callback passed into `BuildManager.submit` keeps the
  quota check in the same transaction as the build row without the manager importing the
  guest service. No JWT library yet: a tiny HMAC token keeps E2.8 dependency-free and E3
  reuses the same secret.
## 2026-11-23 · Task E2.9 (OpenAPI export and generated SDK)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules.
- **Produced:** `tools/export-openapi.py` (export + pure-Python TS generator),
  `spec/api/openapi.json`, `packages/sdk-ts` (`package.json`, `tsconfig.json`, `README.md`,
  `.gitignore`, generated `src/schema.ts`, thin `src/index.ts`), a CI drift-check step in
  the `core` job.
- **Results:** `--check` passes on the committed files and fails on a deliberate edit;
  `tsc --noEmit` compiles the generated types and the client cleanly under strict; core
  suite unchanged (114 pass, 26 skip). Types are generated, never hand-written.
- **Lessons:** `ErrorBody` never appears in a route response, so FastAPI leaves it out of
  the components; adding it from `model_json_schema()` keeps it generated. A stdlib-only
  generator keeps CI Node-free and offline (the user's constraint) while still satisfying
  "generated, CI fails on drift"; pinning `info.version` stops the document churning with
  the package version.

## 2026-11-23 · Task E3.1 (Users, argon2 hashes, login, JWT access tokens, bootstrap CLI)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules
  (python-style, conventions), E2 "Results of" paragraphs for layout and the token secret.
- **Produced:** `auth/passwords.py` (argon2id), JWT helpers in `auth/tokens.py`,
  `services/users.py`, `schemas/auth.py`, `api/auth.py` (`POST /v1/auth/login`), `cli.py`
  (console script `synapse-core create-engineer`), model `User`, migration `0005_users`,
  settings for the token TTL and argon2 cost, `argon2-cffi` and `pyjwt` deps, regenerated
  `spec/api/openapi.json` and `packages/sdk-ts/src/schema.ts`.
- **Results:** 16 new tests; full suite 151 pass, 4 skipped locally; ruff clean; OpenAPI
  drift check green. Login is opaque `NE1105` with a dummy argon2 verify for absent users;
  JWT is HS256 on `SYNAPSE_JWT_SECRET`, 15-minute TTL; the bootstrap command only ever
  creates the first engineer. No secret or token reaches the logs (asserted).
- **Lessons:** calling a command's `asyncio.run` from inside pytest-asyncio deadlocks, so
  the DB bootstrap test drives the async body on the running loop instead of `cli.main`.
  Reusing E2.8's `token_secret()` for JWTs keeps one secret; argon2 cost in settings lets
  tests pick the cheapest parameters while production keeps real cost.

## 2026-11-23 · Task E3.2 (Permission matrix, RBAC route dependencies, guest isolation)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules,
  E3.1 "Results of" for the principal/token surface.
- **Produced:** `auth/permissions.py` (matrix, `DEVICE_PERMISSIONS`, `has_permission`),
  reworked `api/deps.py` (unified `Principal`, `optional_principal`, `require_permission`,
  `require_any_permission`), `services/users.principal_from_access_token`, guest scoping in
  `services/projects.py`, permission deps on the project/build/device routes,
  `tests/test_permissions.py` and `tests/test_rbac_routes.py`, an `engineer_client` conftest
  fixture, regenerated OpenAPI/SDK.
- **Results:** 13 new tests (2 Hypothesis properties); full suite 163 pass, 4 skipped;
  ruff clean; drift check green. Anonymous/under-privileged requests are `NE1106`; a guest
  is scoped to its own sandbox (others' projects are 404, not 403); device list/read need
  `device.read`, the catalog stays public; a role read from the DB row means a disable takes
  effect on the next request.
- **Lessons:** the loose "engineer ⊇ ... ⊇ guest" phrasing does not hold and should not —
  the guest is a separate, sandbox-scoped role; the property tests assert the spec's actual
  criteria (user-chain monotonicity + guest device-isolation). Resolving the principal
  without a DB session when there is no token keeps public routes working with no database
  and avoids a 503 masking a 403.

## 2026-11-23 · Task E3.3 (Service tokens: hashed at rest, revocable, engineer-only issue)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules,
  E3.1/E3.2 "Results of" for the principal resolver.
- **Produced:** model `ServiceToken`, migration `0006_service_tokens`,
  `services/service_tokens.py`, `schemas/service_tokens.py`, `api/service_tokens.py`
  (`POST`/`GET`/`DELETE /v1/service-tokens`, engineer-only), a third branch in the principal
  resolver, regenerated OpenAPI/SDK.
- **Results:** 7 new tests; full suite 170 pass, 4 skipped; ruff clean; drift green. The
  token is a 256-bit `svc_` secret shown once; only its SHA-256 is stored; revoked/unknown
  tokens are `NE1105`; a daemon token is cell-bound, a twin token is not.
- **Lessons:** a plaintext prefix (`svc_`, no dot) is a cheap, reliable way to route three
  token kinds through one resolver without trial-decoding each. Storing only the hash keeps
  the "shown once" guarantee honest; the issue route being `user.manage` stops a service
  principal from minting more tokens.

## 2026-11-23 · Task E3.4 (Append-only audit log and the engineer read route)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, workspace rules,
  the E3.1–E3.3 "Results of" for the principal and session surfaces.
- **Produced:** model `AuditLog`, migration `0007_audit_log` (Postgres INSERT/SELECT grant),
  `services/audit.py`, `schemas/audit.py`, `api/audit.py` (`GET /v1/audit`, engineer-only),
  audit writes on login/token issue+revoke/project+version+build create, `SessionMakerDep`
  and `principal_identity` in `api/deps.py`, regenerated OpenAPI/SDK.
- **Results:** 10 new tests; full suite 180 pass, 4 skipped; ruff clean; drift green. One row
  per sensitive action; `detail` is scrubbed of secret-looking fields; the read route is
  engineer-only and there is no update/delete route.
- **Lessons:** writing audit in its own session and swallowing its errors keeps auditing off
  the critical path of the audited action. A Postgres table owner bypasses its own grants,
  so the append-only grant is honest only with a non-owner app role (noted for E7); the
  code-level guarantee (no UPDATE/DELETE path) is what holds today.

## 2026-11-23 · Task E4.1 (Relay core: WebSocket endpoint, lifecycle, validation, roles)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, the EXISTING relay
  contract (`spec/protocols/relay.md`, schemas in `spec/schemas/relay/`, examples in
  `spec/protocols/examples/`), workspace rules, and the E2/E3 "Results of" paragraphs for the
  principal, token and session surfaces that E4.1 reuses.
- **Produced:** package `synapse_core/relay/` (`validate.py` schema validation with
  `jsonschema`; `roles.py` `ROLE_MESSAGES` from `relay.md`; `protocol.py` server-frame
  builders and `RelayCode`; `auth.py` token→roles reusing E3; `hub.py` session registry;
  `session.py` `RelaySession` lifecycle; `starlette.py` transport adapter), route
  `api/relay.py` (`WS /v1/relay`), registered in `main.py`; `jsonschema` dependency.
- **Results:** 15 new tests; full suite 195 pass, 4 skipped locally; ruff clean; OpenAPI/SDK
  drift green. Handshake (`hello`→`welcome`), schema validation of every frame, the role table
  checked against `relay.md`, fatal vs non-fatal error codes, ping/`pong` and idle-ping/silent
  close all covered. No new `NE` code (relay uses its own protocol error codes).
- **Lessons:** a WebSocket endpoint must inject settings via `Depends(get_settings)`, not call
  the cached singleton, or test dependency overrides (and the per-test database) are ignored.
  Making the session transport-agnostic with injectable timeouts let the lifecycle be tested
  in-process with Starlette's `TestClient` and short timeouts, no network and no real clock
  wait. `BaseHTTPMiddleware` skips WebSocket scopes, so the relay's control-message rate limit
  belongs inside the session (E4.2), not in the HTTP limiter.

## 2026-11-23 · Task E4.2 (Relay subscriptions, snapshot cache, fan-out, control rate limit)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, the relay contract and
  its example messages (`spec/protocols/examples/`), the E2.7 token-bucket limiter, and the
  E4.1 "Results of" for the hub and session surfaces this builds on.
- **Produced:** `relay/hub.py` grown with `Subscription`, `BoundedOutbox` (drop policy),
  per-cell snapshot cache and `publish` fan-out with channel/device matching; `relay/session.py`
  with an outbox writer task, real `subscribe`/`unsubscribe` (ack + cached snapshot), publisher
  fan-out, and a per-session 20/s control-message rate limit reusing the E2.7 bucket.
- **Results:** 12 new tests (9 DB-free hub/property, 3 WebSocket fan-out); full suite 207 pass,
  4 skipped; ruff clean; drift green. The Hypothesis property validates every example message
  against the JSON Schema and round-trips it unchanged, then routes publisher examples.
- **Lessons:** routing every post-`welcome` send through one bounded outbox and a single writer
  task gives ordered delivery, a uniform drop policy and no concurrent writes to the socket.
  The drop policy must keep control/snapshot/replay frames even past the soft bound, so the
  queue is not a hard cap for those; only `telemetry`/`trace` are ever dropped. Making the
  control rate injectable let the limit be hit deterministically in a test without real time.

## 2026-11-23 · Task E4.3 (Daemon pairing, daemons/devices tables, heartbeat timeout)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, the relay contract
  (`deployment_status`), the E3.3 service-token primitive reused for the daemon token, and the
  E4.1/E4.2 "Results of" for the hub and session surfaces.
- **Produced:** models `PairingCode`/`Daemon`/`Device` + migration `0008`, `services/pairing.py`
  (issue/claim/unpair/heartbeat/status), routes `api/pairing.py` (`POST /v1/pairing`, public
  `POST /v1/pairing/claim`, `GET /v1/daemons`, `DELETE /v1/daemons/{id}`), relay `on_heartbeat`
  hook, hub `close_by_key` and session `stop` event, three new audit actions.
- **Results:** 8 new tests (7 DB pairing flow, 1 offline migration/model check); full suite 215
  pass, 4 skipped; ruff clean; drift green. `NE1111` reused for a bad claim; no new code.
- **Lessons:** closing a relay session from outside needs both the outbox closed (stops the
  writer) and a `stop` event the reader races against its receive, or the socket lingers until
  the peer disconnects. An offline SQLite migration/model column check catches drift that the
  local `create_all` schema would otherwise hide until the Postgres CI round trip.

## 2026-11-23 · Task E4.4 (Telemetry ingest into plain Postgres with a storage cap)
- **Kiro features:** spec-driven task from `.kiro/specs/core-services/`, the relay `telemetry`
  frame shape, and the E4.1–E4.3 relay/session surfaces (the publisher-frame path and hooks).
- **Produced:** models `TelemetrySample`/`Telemetry1m` + migration `0009`, `telemetry/ingest.py`
  (`_rows_from_frame`, `ingest_telemetry` with the `SYNAPSE_TELEMETRY_MAX_ROWS` cap), a relay
  `on_telemetry` hook that persists frames with `simulated` set from the publisher role, and the
  telemetry settings (cap and the retention windows E4.5 uses).
- **Results:** 4 new tests; full suite 219 pass, 4 skipped; ruff clean; drift green. The sample
  `id` is split into device/tag; a dot-less id is skipped; the cap drops a whole batch.
- **Lessons:** SQLite only autoincrements an `INTEGER PRIMARY KEY`, so a `BigInteger` PK needs a
  `with_variant(Integer, "sqlite")` for the local bulk-insert tests to get ids while Postgres
  keeps `BIGINT`. An out-of-enum quality cannot reach ingest (the JSON Schema rejects it at the
  socket), so the normalisation is a defensive fallback, not a path a wire frame can take.
