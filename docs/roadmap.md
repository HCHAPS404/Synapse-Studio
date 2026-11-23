# SYNAPSE Studio · Neuron — Complete roadmap

Version 2026-11-15. Source of truth for phases, assignments, order and status.
Update the **Status** column when a task changes state and note it in
`docs/kiro-log.md`.

## Conventions

| Mark | Meaning |
|---|---|
| ✅ | done |
| ✅🔌 | done and verified without hardware; the acceptance on the real device is pending (a hardware session, You) |
| 🚧 | in progress |
| ⬜ | not started |
| **H** | Hell (lead: language, compiler, backends, std, verification, core services, daemon, hardware, AI) |
| **M** | Martin (engine, digital twin, web, mobile; desktop with Hell) |
| **H+M** | shared; the first initial is the accountable owner |
| **You** | a human action that agents cannot do (forms, hardware, recordings, posts) |

Every task follows the definition of done in `CONTRIBUTING.md`: approved spec, tests
(property-based for invariants), docs, CI green, evidence in `docs/kiro-log.md`.

## Key dates and gates

| Date | Gate | Exit criteria |
|---|---|---|
| 2026-11-14 | G0 | Repository, CI, Kiro setup live ✅ |
| 2026-11-19 | **G1 — language** | Phase A complete: Neuron programs type-check, run in the interpreter and pass tests; edition 2026 frozen |
| 2026-11-21 | **G2 — first device** | A `.neuron` program blinks, prints over UART and handles an interrupt on Arduino UNO R3 |
| 2026-11-24 | G3 — multi-target | Same program on UNO, Mega, Pico and ESP32-S3; Verilog emitted and running on an FPGA; DG-1 desktop decision |
| 2026-11-29 | **G4 — feature freeze** | Routes A–D work in the virtual lab and on hardware; web usable end to end |
| 2026-12-02 | G5 — release freeze | `release/kiroween-1.0` cut; only fixes after this |
| 2026-12-03 | Submission | Devpost form complete (one day of margin) |
| 2026-12-04 17:00 COT | Deadline | 14:00 PT, no extensions |
| 2026-12-16 → 2027-01-29 | Judging | Virtual lab online and free; only urgent fixes |

## Tracks running in parallel

```
Hell   ─ A language ─────────▶ B direct programming ─▶ C translation ─▶ D cell compiler
         (14–19 nov)            (19–28 nov)             (22–29 nov)       (23–28 nov)
                    └─ E services: Core API + Edge Daemon (20–29 nov) ─┘
Martin ─ G1 design system ─▶ F engine + twin ─────────────▶ G web IDE + virtual lab ─▶ G desktop/mobile
         (15–17 nov)           (15–29 nov)                    (17–30 nov)                (25–30 nov)
Both   ─ I hardware sessions (17 nov → 1 dec) · J integration (29 nov–1 dec) · K submission (1–3 dec)
```

### Contracts that let both tracks work without blocking

| Contract | Owner | Needed by | Date |
|---|---|---|---|
| NIR JSON schema (`spec/schemas/nir.schema.json`) | H | Martin (editors, twin) | v1.0 published 15 nov ✅ |
| Engine WASM API (load scene, step, inject signal, read state) | M | Hell (virtual devices) | 17 nov |
| `.scene.json` format | M | both | 18 nov |
| Twin / relay WebSocket protocol | H | Martin (web, mobile) | 20 nov |
| Core API OpenAPI + generated `sdk-ts` | H | Martin | 21 nov |

---

## Phase 0 — Foundation (done)

| ID | Task | Owner | Status |
|---|---|---|---|
| 0.1 | Public repo, Apache-2.0, governance files, branches `main`/`develop` | H | ✅ |
| 0.2 | Kiro steering (product, tech, structure, conventions + file-match style and codegen safety rules) | H | ✅ |
| 0.3 | Kiro hooks: `.kiro` guard, cargo check on save, evidence logging (secret guard moved to CI) | H | ✅ |
| 0.4 | CI: compliance (`.kiro`, license, gitleaks), Rust, Python | H | ✅ |
| 0.5 | Core API skeleton with `/health`; `render.yaml` | H | ✅ |
| 0.6 | README, About text, banner and diagrams | H | ✅ |
| 0.7 | Compliance matrix, team agreement template, human-actions list | H | ✅ |

---

## Phase A — The Neuron language (no hardware) · Hell · 14–19 nov

Goal: a working language on the PC — parse, check, run, test. Everything else depends
on this phase.

| ID | Task | Deliverable | Acceptance | Depends | Dates | Status |
|---|---|---|---|---|---|---|
| A1 | Language specification and grammar | `spec/language/` (16 chapters), `spec/grammar/neuron.ebnf` | Draft reviewed by Hell and Martin | — | 14 nov | ✅ v0.4 draft · review pending (You) |
| A2 | Lexer | `neuron-syntax::lexer` | Properties LEX-1, LEX-2; all spec blocks lex | A1 | 14 nov | ✅ v0.2 |
| A3 | AST and parser (8 dialects) | `neuron-syntax::parser`, `outline`, golden files | Every complete spec program parses; error recovery; parser totality property; golden outlines | A2 | 14 nov | ✅ v0.2 |
| A4 | Name resolution and modules | `neuron-sema` crate: scopes, `use`, `pub`, packages, `neuron.toml` | NE0101–NE0105 with tests; multi-file packages | A3 | 14–15 nov | ✅ |
| A5 | Type system and units | Types, inference, quantities with dimensions, affine units, conversions, generics (checked once, monomorphized in A9), classes and interfaces, `Option`/`Result`/`?` | Property UNIT-1; NE0201–NE0208 (and NE0209–NE0217) | A4 | 15–16 nov | ✅ |
| A6 | Dialect rules and effects | Legal constructs per dialect (NE0501), `mut` aliasing (NE0303), bounded loops (NE0403), no recursion (NE0302), non-blocking bodies (NE0404) | One positive and one negative test per rule | A5 | 16 nov | ✅ |
| A7 | Time and concurrency analysis | Tasks/interrupts graph, resource ceilings (SRP), `signal` single writer, WCET model and response-time analysis, static memory and stack budget | SRP and MEM-1 properties on the computed model; NE0301, NE0304, NE0306, NE0401, NE0402, NE0604. LET-1 and MEM-2 on the interpreter (A10) | A6 | 16–17 nov | ✅ |
| A8 | Safety checks | `safe_state` completeness (NE0802), `on fault` restrictions (NE0803), `verify` model checking for `fsm`/`sequence` (NE0901–NE0904) with counterexample traces | Counterexamples are shortest paths. SAFE-1/2 on the interpreter (A10) | A7 | 17 nov | ✅ |
| A9 | NIR (canonical IR) | `neuron-nir` crate, lowering → NIR, JSON Schema (`spec/schemas/nir.schema.json`), printer NIR → `.neuron`, `neuron emit-nir` / `print-nir`, spec chapter 16 | Properties NIR-1 and P-RT (round trip); schema published for Martin | A5–A8 | 15 nov | ✅ |
| A10 | Reference interpreter | `neuron-interp` crate: discrete-event executor with simulated time (tasks, interrupts, signals with LET, `fsm`, `sequence`, `run`/BUSY/DONE/TIMEOUT, PLC scan, faults and safe state), function blocks, `test` blocks; `neuron run`, `neuron test`; spec chapter 17 | Properties MOC-R1, LET-1, SAFE-1; every example and valid spec program runs; corpus programs run (A13) | A9 | 16 nov | ✅ |
| A11 | Tooling | Formatter (`neuron fmt`, property P-FMT), `neuron explain NE####`, `docs/errors/`, LSP basics (diagnostics, hover, go to definition) | Formatter idempotent on corpus; LSP works in VS Code/Kiro | A3–A9 | 18–19 nov | ✅ formatter, explain and LSP done; VS Code client in `editors/vscode` awaits a manual install test (You) |
| A12 | Standard library, base layer | `lib/std`: `core`, `units`, `time`, `logic` (TON/TOF/TP/CTU/CTD/R_TRIG/F_TRIG/SR/RS), `control` (PID with anti-windup, low-pass, moving average, hysteresis, debounce, ramp), `test` | Each module tested in the interpreter | A10 | 18–19 nov | ✅ 13 modules in `lib/`, 30 tests, differential and property tests against the native blocks, layering rule `NE0106` |
| A13 | Validation corpus and freeze | ~50 programs across the 8 dialects in `examples/corpus/`; usability review; tag `spec-v1.0` | Gate **G1** | A1–A12 | 19 nov | ✅ 50 programs and the usability review done, tag `spec-v1.0-rc1`; the final `spec-v1.0` needs an independent reviewer (You) |

---

## Phase B — Direct programming of devices · Hell · 19–30 nov

| ID | Task | Deliverable | Acceptance | Depends | Dates | Status |
|---|---|---|---|---|---|---|
| B1 | Target model | `targets/arch|chips|boards|runtimes/*.toml`, SVD import, pin/alternate-function solver, clock-tree solver | NE0601, NE0603, NE0605, NE0606; files for UNO, Mega, Pico, Pico 2 W, ESP32-S3, XIAO S3, N657 | A7 | 19–20 nov | ✅ `neuron-target`, 8 boards, pin and clock solvers, HW-1/CLK-1/TGT-1 tests, `neuron check` runs the hardware checks. SVD import stays B1b (after G2); N657 file is a placeholder |
| B2 | MIR and C backend | HIR/NIR → MIR (SSA) → portable C; source maps | Golden files; generated C compiles with `-Wall -Werror` | A9, B1 | 19–21 nov | ✅ `neuron-mir` (a structured typed tree; SSA moves to B8) and `neuron-codegen-c`; host builds match the interpreter trace; `match`, enums, arrays, classes, `sequence` and native blocks give NE0610 |
| B3 | Runtime `neuron-rt` (C) | Startup, static SRP scheduler, timers, interrupt dispatch, safe state, watchdog, panic/fault | Runs on host (tests) and on AVR | B2 | 20–21 nov | ✅ `runtime/neuron-rt`: core, host port, AVR port (UNO, Mega); run-to-completion tasks, interrupts with atomic sections |
| B4 | `std.hal` per family + build/flash | `gpio`, `uart`, `timer`, `adc`, `pwm`, `i2c`, `spi`; `neuron build`, `neuron flash` (avrdude, UF2, esptool, probe-rs/OpenOCD) | **G2** on UNO R3 | B3 | 20–21 nov | ✅🔌 `neuron targets`, `emit-c`, `build` (`--host`, `--native`), `emit-verilog`, `flash` with avrdude, the UF2 drive, probe-rs/OpenOCD/st-flash, esptool and openFPGALoader; gpio, uart, adc, pwm, interrupts; I2C and SPI display bus on AVR and ESP-IDF. 🔌 G2: the UNO on USB (You) |
| B5 | More boards (support tiers) | Mega, Pico, ESP32-S3/XIAO (ESP-IDF runtime port), Pico 2 W (CYW43 LED via SDK), N657 (FSBL boot), boards from the second lab | Same blink/UART/IRQ program on each; tier table in docs | B4 | 21–25 nov | ✅🔌 ports for RP2040/RP2350 (bare metal, RAM UF2), STM32F4 (NUCLEO-F446RE) and ESP32-S3 (ESP-IDF project); the RP2040 firmware matches the interpreter in the Thumb simulator; tier table in `docs/targets.md`. N657 stays a placeholder (FSBL boot). 🔌 Same program on each real board (session I2) |
| B6 | ISA simulators (virtual MCU) | AVR and Thumb-2 (M0+) simulators running the real binaries; peripheral models for GPIO/UART/timer | Same trace on simulator and hardware | B2 | 22–26 nov | ✅🔌 `neuron-sim-avr` runs the avr-gcc and the native firmware; `neuron-sim-thumb` runs the RP2040 firmware (RESETS, XOSC, CLOCKS, IO_BANK0 interrupts, UART0 modelled); both match the interpreter. 🔌 Comparison with the real boards (G2, I2) |
| B7 | `hdl` dialect → Verilog + constraints | Synthesizable Verilog, CDC synchronizers, XDC/PCF; verilator/iverilog checks; FPGA flashing (openFPGALoader or vendor tool) | Counter + PWM + UART TX on the lab FPGA; MCU↔FPGA UART link | A9 | 21–23 nov | ✅🔌 `neuron-codegen-hdl`: Verilog-2005, `NE0702` and two-flop synchronizers, PCF/XDC for two generic boards, iverilog/yosys/nextpnr/Vivado/openFPGALoader drivers; corpus goldens, iverilog simulation and yosys synthesis in CI. 🔌 The lab FPGA (session I3) |
| B8 | Native backends | AVR first, then RV32IMC (Pico 2 W RISC-V cores), then Thumb-2; assembler, linker, ELF/HEX | Blink without external compiler on at least one board | B2 | 24–30 nov | ✅🔌 `neuron-native-avr`: assembler, Intel HEX and a code generator for the UNO (tasks, interrupts, fsm, signals, checked arithmetic); `neuron build --native`; images match the interpreter in the AVR simulator. RV32IMC and Thumb-2 native come later. 🔌 Native blink on the real UNO (with G2) |
| B9 | `ui` dialect, first displays | `std.display`: SSD1306 (I2C), ILI9341/ST7789 (SPI), HD44780; render strategy chosen from RAM | Dashboard on ESP32-S3 with a TFT; NE0604 on UNO if it does not fit | B4 | 25–29 nov | ✅🔌 `neuron-ui` and `runtime/neuron-rt/ui`: SSD1306, ILI9341, ST7789, HD44780; label, gauge, bar, slider, led, button; strategy from the RAM and NE0604 (the TFT panel on the UNO); the firmware's bus traffic rebuilds the reference image pixel for pixel. 🔌 The dashboard on the ESP32-S3 TFT (ESP-IDF and the panel) |

### Pending from Phase B

| Item | Owner | Action |
|---|---|---|
| Local check before each push | H | `powershell -NoProfile -ExecutionPolicy Bypass -File tools\phase-check.ps1 [-Message "..."]` runs what CI runs, then commits and pushes (6–10 min warm) |
| G2 on the UNO, I2 boards, I3 FPGA, ESP32-S3 TFT dashboard | You | Commands in `docs/phase-b-prerequisites.md` |

---

## Phase C — Translation between languages · Hell · 22–29 nov

| ID | Task | Deliverable | Acceptance | Depends | Dates | Status |
|---|---|---|---|---|---|---|
| C1 | Emitter framework | Plugin API, fidelity report (exact/approximate/unsupported), package with code + config + wiring report | Golden files per emitter | A9 | 22 nov | ✅ `neuron-emit`: emitters with manifests, `fidelity.json`, source maps, wiring report, policy (`NE1003`), deterministic output |
| C2 | IEC 61131-3 emitters | ST (generic and Yaskawa dialect), SCL, PLCopen XML (Ladder/FBD/SFC) | `matiec` compiles ST; import check in the vendor tool during I5 | C1 | 23–25 nov | ✅🔌 ST (generic, yaskawa), SCL, PLCopen XML (LD, SFC); run in the virtual PLC of `neuron-st` and equal to the interpreter for the whole `plc` corpus. 🔌 Import into the vendor tools (I5) |
| C3 | Robot emitter | SPEL+ program + command-interpreter slave; workspace checks (NE0804) | Runs on the virtual robot; checked on the arm in I6 | C1 | 25–27 nov | ✅🔌 19 nov: SPEL+-style program, `NeuronLib.prg`, `NeuronSlave.prg`, generic robot manifests; the 6 robot programs run in the virtual controller `neuron-spel` with 68 motions and 23 output changes equal to the interpreter. 🔌 Dialect details on the arm (I6) |
| C4 | Python / ROS 2 emitter | `host` dialect → Python nodes | Runs on a Raspberry Pi / PC | C1 | 26–28 nov | ✅ 19 nov: Python 3.12 and `neuron_rt.py`, trace byte-equal to the interpreter in 9 runs (168 lines). 20 nov (C4b): timers, counters, `pid`, `fsm`, `sequence`, `struct`, `enum`, `match`, `Ok`/`Err`/`none`/`?` and contracts translated (655 more lines byte-equal); `--ros2` wrapper written and checked against a stub `rclpy` only, to confirm in session I4; `class`, `interface`, `mut`, interpolation, lambdas still `NE0610` |
| C5 | Importers | ST and PLCopen Ladder → Neuron; SPEL+ subset; C subset; Verilog subset; fidelity per construct | Round trip by traces (import → run in interpreter → equal outputs) | A9, A10 | 22–29 nov | ✅ 20 nov: `neuron-import` reads ST, SCL, PLCopen XML, SPEL+, a C subset and a Verilog subset; round trips by traces equal for the whole corpus; the rest is `NE1002` |
| C6 | Micro-Kernel hot reload | Small interpreters for PLC/robot/MCU that accept state tables from the daemon | Sequence changed without the vendor IDE in I5/I6 | C2, E5 | 27–29 nov | ✅🔌 20 nov: state tables, word encoding with checksum, ST, C99 and SPEL+ kernels equal to the reference executor (`m06`, `p09`, two more programs, random tables). 🔌 Run on a PLC and a robot (I5, I6); download by the daemon (E5) |
| C7 | CLI and documentation | `neuron emit` and `neuron import`, `docs/translation.md`, CI smoke steps | Every emitter and importer runs from the command line; policy and diagnostics tested | C1–C6 | 29 nov | ✅ 20 nov: `crates/neuron-cli/src/translate.rs`, 15 CLI tests (`tests/c7_translate.rs`), `docs/translation.md`, smoke steps in CI and `tools/phase-check.ps1` |

---

## Phase D — Cell compiler (multi-device) · Hell · 23–28 nov · ✅ software (D1–D7 done 23 nov; D-HW pending sessions I1 and I4–I7)

| ID | Task | Deliverable | Acceptance | Depends | Dates | Status |
|---|---|---|---|---|---|---|
| D1 | Device manifests and capabilities | Manifest schema, trust levels, capability palette, topologies (same class other vendor, replicas, heterogeneous) | Same module placed on two PLC types | B1 | 23 nov | ✅ 22 nov: crate `neuron-cell` (schema, loader, trust levels, media, `NE0607`); manifests for every device of the corpus cells and the bottling example; `conveyor_plc` builds for the `s7` (SCL) and `iec_st` (ST) PLC classes |
| D2 | Partitioner | `place` and automatic assignment; paradigm lowering per target | Bottling cell partitions into 4 programs | D1, C1 | 23–24 nov | ✅ 22 nov: partitioner (`NE0704`, `NE0705`), the bottling cell partitions into 4 programs; build per device through the existing backends (a cell with one device of each class builds to SCL, C, SPEL+, Verilog with XDC and Python). Open: the bottling example itself does not build (C backend lacks function blocks; `pick` is six-axis on a SCARA) |
| D3 | Links and timing | Standard Time Model: link synthesis (both ends), worst-case latency (NE0701), Latch+ACK, BUSY/DONE/TIMEOUT, timestamps, cell watchdog; uses measured timing profiles | 200 µs on Modbus rejected with alternative proposed | D2, I1 | 24–26 nov | ✅ 23 nov: latency per link (spec 14.8), `NE0701` with alternatives, link table (a 200 µs Modbus link proposes zero_hop); link plans, register maps, endpoint modules as Neuron source in every device program, Neuron Wire codec (WIRE-1..3); cell watchdog (50 ms, `LinkLost`) in the receiver endpoints and the MOC-C2 bound in the plan (robot watchdog logic and the co-simulation proof are open: I6, D6.1); tags with `NE0703` and the single-writer rule |
| D4 | Safety across the cell | Mutex zones, workspace bounds, electrical compatibility (NE0801), cell `verify` | Counterexample replayed in the twin | D3, A8 | 26–27 nov | ✅ 22 nov D4.2: electrical compatibility of wired links, `NE0801`. 23 nov D4.1: zones and mutexes with a generated lock (owner arbiter, request/grant on each party), robot targets in zones, `NE0805`; the user's sequences are not rewritten (acquire/release is a contract in `zones.json`). 23 nov D4.3: cell `verify:` over the cell's machines and the placed modules' machines, links as lossless bounded-delay channels (delay from the `within` budget), shortest counterexample as plain data (`NE0901`); the mutex of x02 is proved with the model checker at its real lock delay, and the two-phase variant is refuted with a trace. The replay of the counterexample in the twin is D6 |
| D5 | Technical dossier | Wiring, I/O list, pin tables, timing diagrams, BOM, test report (PDF) | Generated for the demo cell | D4 | 27–28 nov | ✅ 23 nov D5.1: dossier in Markdown and one self-contained HTML (inline SVG timing diagram, wiring and I/O lists, register map and Wire tags, link table with budgets, BOM, safety report, co-simulation test report), byte-identical for the same inputs; generated for the five-class `Lab` cell and, with a per-device build status (`NE0610`, `NE0804`), for the bottling cell. 23 nov D6 and D7: co-simulation of all devices on one cell clock with link latency and `--cut` (D6.1), replay format and JSON Schema for the twin (D6.2), and the command line `neuron cell check\|build\|sim\|dossier` with the guide [`docs/cells.md`](cells.md) and CI smoke steps; `neuron check` also runs the cell checks. PDF is printed from the HTML by the user |
| D-HW | Timing profiles and the cell on real devices | Measured numbers in the manifests (trust `measured`); the bottling cell on a PLC, a board, an FPGA and an arm | Same trace in the twin and on the bench | D5, I1, I4–I7 | sessions | ⬜ pending: every number is `generic` until I1; the bottling example does not build for all devices (C function blocks, six-axis on SCARA); robot watchdog logic (I6) |

---

## Phase E — Services · Hell · 20–29 nov

| ID | Task | Deliverable | Depends | Dates | Status |
|---|---|---|---|---|---|
| E1 | Compiler bindings | PyO3 module and WASM build of the compiler (browser diagnostics without server) | A9 | 20–21 nov | ✅ 21 nov: `crates/neuron-wasm` (JSON API `check`, `format`, `hover`, `definition`, `run`, `test`, `emit`, `importSource`, `version`; wasm-bindgen exports on wasm32; CI builds `wasm32-unknown-unknown`), `crates/neuron-py` (PyO3 module `neuron_py`, built with maturin, outside the workspace), usage in `packages/wasm-glue/README.md` |
| E2 | Core API | Projects, versions, builds, devices; Neon + Alembic; guest mode for judges; rate limits | E1 | 21–24 nov | 🟡 23 nov: E2.1 done (layout, settings, logging, error handlers, async session, Alembic baseline, NE11xx docs, CI Postgres); E2.2 done (`--format json` for check, build, emit); E2.3 done (compile runner with limits, group kill, network self-test; POSIX tests wait for Linux CI); E2.4 done (projects, immutable versions, files, `NE1107`); E2.5 done (builds API, queue + semaphore, `NE1103`, SHA-256 artifacts, `/v1/capabilities`); E2.6 done (device catalog from the `devices/` manifests, read-only routes); E2.7 done (token-bucket rate limiter, per-client buckets, `NE1103` + `Retry-After`, property test of the rate bound); E2.8 done (guest mode: sandboxes, HMAC guest token, quotas, expiry cleanup, `NE1104`, end-to-end guest build); E2.9 done (OpenAPI export to `spec/api/openapi.json`, generated `packages/sdk-ts` types, CI drift check); E2.10 open |
| E3 | Auth, RBAC, audit | viewer / operator / engineer roles; audit log | E2 | 24–25 nov | ✅ 23 nov: E3.1 done (users, argon2id hashes, `POST /v1/auth/login`, HS256 JWT access tokens, bootstrap CLI, `NE1105`); E3.2 done (permission matrix, route RBAC deps, guest sandbox isolation, device routes gated by `device.read` with the catalog public, `NE1106`, properties); E3.3 done (service tokens hashed at rest and revocable, engineer-only `POST`/`GET`/`DELETE /v1/service-tokens`); E3.4 done (append-only `audit_log` with Postgres INSERT/SELECT grant, audit writes on login/token/project/version/build, engineer-only `GET /v1/audit`). E3 complete |
| E4 | Relay and twin gateway | WebSocket protocol, daemon pairing, telemetry (TimescaleDB) | E2 | 24–26 nov | 🟡 contract done (`spec/protocols/relay.md`, crate `synapse-protocol`, schemas in `spec/schemas/relay/`); E4.1 done (relay core `WS /v1/relay`); E4.2 done (subscriptions, `cell_snapshot` cache, fan-out with bounded per-subscriber queues and the drop policy, per-session 20/s control-message rate limit, Hypothesis round-trip over every example message); E4.3 done (daemon pairing: engineer-issued code, public claim exchanging it for a cell-bound `daemon` service token, `daemons`/`devices` tables, `deployment_status`/heartbeat with a 15 s offline timeout, unpair that revokes and drops the relay sockets); E4.4 done (telemetry ingest from relay frames into plain Postgres `telemetry_samples` with the `(cell,device,tag,t_ns)` index and the `SYNAPSE_TELEMETRY_MAX_ROWS` cap; `simulated` per sample from the publisher role); E4.5 done (time-based retention and a 1-minute roll-up under a Postgres advisory lock, `GET /v1/telemetry` with guests limited to virtual devices, a Hypothesis property that retention removes only expired rows and the roll-up preserves each bucket's count/sum); E4.6 done (advisory E-STOP forwarded to the daemon and answered with its reported state or `unknown`, audited; `sim_*`/`replay_play` routed to the twin only and refused for real devices; the software E-STOP is supplementary and the hardwired one is the authority). **E4 relay server complete (E4.1–E4.6)** |
| E5 | Edge Daemon | Drivers: serial, Modbus TCP/RTU, S7, TCP sockets, MQTT, CAN; flashing tools; signed packages; heartbeat and local E-STOP; serves the PWA on the LAN | B4 | 21–29 nov | 🟡 23 nov: E5.1 done (new `services/edge-daemon` package: config with local LAN vs cloud mode, the `Driver` protocol + `SimulatedDriver`, relay frame builders and a reconnecting `RelayClient`, a pairing client reusing E4.3/E3.3; hardware-free, network-free CI job); E5.2 done (signed packages: Ed25519 sign on the API via `SYNAPSE_SIGNING_KEY`, verify + quarantine on the daemon with a shared canonical manifest, `NE1108` on any tamper); E5.3 done (command allowlist with per-command Pydantic args, `NE1109` for a bad/unknown command and `NE1111` for an offline device; no shell/estop command); E5.4 done (deployment state machine in the API and the daemon with `NE1110` on any out-of-order step; a Hypothesis property proves no shortcut to `applied` and terminal states stay terminal); E5.5 done (serial driver over `pyserial` `loop://` and a `Flasher` for avrdude and UF2 with fake tools, per-device lock and timeout); E5.6 done (Modbus TCP and RTU drivers sharing one read/write path, tested against an in-process pymodbus TCP server and a POSIX pty loopback pair). Open: E5.7–E5.12 |
| E6 | Black box and calibration | Records around faults; timing profiles fed back to D3 and the twin | E5, I1 | 26–29 nov | ⬜ |
| E7 | Deployment | Render (judging environment), Neon branches, Vercel; budget until 2027-01-29 | E2 | 25–28 nov | 🟡 22 nov: Neon project (Ohio), Render service `synapse-core` (Starter, Oregon; region TODO in `render.yaml`) with `/health` and database ok; placeholder `apps/web` on Vercel (https://synapsest.vercel.app) verified end to end from the public URL; CORS set to that origin. Open: Neon `dev` branch, spending caps, the judge-facing app (needs E2 and G6) |

---

## Phase F — Synapse Engine and digital twin · Martin · 15–29 nov

| ID | Task | Deliverable | Depends | Dates | Status |
|---|---|---|---|---|---|
| F1 | Engine kernel (Rust → WASM) | Fixed-step deterministic kernel, ECS, snapshots and rewind, co-simulation signal bus | — | 15–17 nov | ⬜ |
| F2 | Engine API and scene format | WASM API contract, `.scene.json` | F1 | 17–18 nov | ⬜ |
| F3 | Renderer | Three.js behind the engine API (wgpu renderer post-hackathon); gizmos, overlays (zones, paths, signals, latency) | F2 | 16–22 nov | ⬜ |
| F4 | Kinematics | 6-axis and SCARA: FK, IK (analytic + damped numeric), joint limits, trapezoid/S profiles, PTP/linear/arc | F1 | 18–21 nov | ⬜ |
| F5 | Physics | Rigid bodies with contacts and friction, conveyors, grippers; Rapier as test oracle only | F1 | 19–24 nov | ⬜ |
| F6 | Pneumatics, drives, sensors, electrical | Cylinders/valves/reed switches, DC/stepper/servo, optical/inductive/ADC sensors, rule-based electrical model | F1 | 22–25 nov | ⬜ |
| F7 | Virtual devices (with Hell) | PLC (ST subset), robot (SPEL+ core), FPGA (Verilog subset), MCU (B6 simulators), bus; fidelity badges | F2, A10, B6, C2, C3 | 24–29 nov | ⬜ |
| F8 | Scenarios | Bottling cell (route A), legacy Ladder import (B), FPGA master + robot (C), bare-metal debug (D); free builder; fault injection | F3–F7 | 23–28 nov | ⬜ |
| F9 | Recording and replay | Replays of real lab sessions on top of the twin | F8, E6 | 27–29 nov | ⬜ |

---

## Phase G — Client applications · Martin (desktop with Hell) · 15–30 nov

| ID | Task | Deliverable | Depends | Dates | Status |
|---|---|---|---|---|---|
| G1 | Design system | Tokens, theme, components (shadcn/ui), Storybook, English-first i18n, accessibility | — | 15–17 nov | ⬜ |
| G2 | IDE shell (web PWA) | Docking panels, project explorer, command palette, problems panel, offline PWA | G1 | 17–20 nov | ⬜ |
| G3 | Text editor | Monaco with Neuron grammar, diagnostics from the WASM compiler, formatter | G2, E1 | 19–21 nov | ⬜ |
| G4 | Graphical views | Blocks, FSM/GRAFCET, Ladder (bidirectional with text, property VIEW-1), chip/pin view, clock tree | G3, A9, B1 | 20–26 nov | ⬜ |
| G5 | Inspectors | Generated code with source maps (running line highlighted), bus monitor, scope/plotter, timing diagram | G4, F7 | 24–28 nov | ⬜ |
| G6 | Virtual lab for judges | "Try the virtual lab" without login, guided 3-minute tour, live-hardware indicator, offline | F8, G5 | 26–30 nov | ⬜ |
| G7 | Desktop app | DG-1 (Tauri vs Qt) on 24 nov, then USB/serial flashing and embedded daemon; Windows/Linux installers | G2, E5 | 24–30 nov | ⬜ |
| G8 | Android remote control | Kotlin Multiplatform: cells, telemetry, settings, supplementary E-STOP, 3D viewer; APK in Releases | E4 | 26–30 nov | ⬜ |

---

## Phase H — AI (optional, never in the safety path) · Hell · 25–30 nov

| ID | Task | Deliverable | Depends | Dates | Status |
|---|---|---|---|---|---|
| H1 | Provider layer | Classic (no AI) default; assistant; bring-your-own (local or own key, encrypted client-side); public demo on an open-weights model with spending cap and cache | E2 | 25–27 nov | ⬜ |
| H2 | Assistant | Tool-restricted agent (schema, verifier, block catalogue); proposals accepted by the user and verified | H1, A8 | 26–28 nov | ⬜ |
| H3 | AI Driver Builder (MCP) | Machine-readable sources first (SVD, pin CSV, EDS/GSDML), AI fills gaps; manifests marked "AI-generated, unreviewed" | H1, D1 | 27–30 nov | ⬜ |
| H4 | Kiro Crew app | Separate repository (same GitHub account), e.g. a crew that audits Neuron programs and onboards devices | — | 27–30 nov | ⬜ |

---

## Phase I — Hardware lab sessions · Hell (+ Martin for twin calibration)

Order requested by the team: MCU → FPGA and MCU↔FPGA → MCU + SoC + FPGA → PLC →
robot arms → laboratory plants (pneumatic/hydraulic benches and the bottling station).

| ID | Session | What happens | Needs | Dates | Status |
|---|---|---|---|---|---|
| I1 | Reconnaissance and measurement | Inventory with exact models and firmware; which tool programs each device; measure PLC input filter and scan, robot command latency, valve switching time with logic analyzer/oscilloscope; photos | Nothing from Neuron | 17 nov | ⬜ (You) |
| I2 | MCUs | Gate G2 then G3 boards (UNO, Mega, Pico, ESP32-S3, XIAO, Pico 2 W, N657, second-lab boards) | B4, B5 | 20–22 nov | ⬜ |
| I3 | FPGA and MCU↔FPGA | Verilog from `hdl`; UART/GPIO link with Latch+ACK | B7 | 22–23 nov | ⬜ |
| I4 | MCU + SoC + FPGA | Raspberry Pi / Jetson `host` node with MCU and FPGA in one cell | C4, D3 | 24–25 nov | ⬜ |
| I5 | PLCs | ST/SCL import into vendor tools; Micro-Kernel; Modbus/S7 links | C2, C6, E5 | 25–26 nov | ⬜ |
| I6 | Robot arms | SPEL+ program and slave interpreter; workspace and zone checks at reduced speed | C3, D4, E5 | 27–28 nov | ⬜ |
| I7 | Laboratory plants | Pneumatic/hydraulic bench and bottling station in the full cell; mixed real/virtual mode | D, E, F7 | 28–29 nov | ⬜ |
| I8 | Video recording | Real hardware alongside the twin; logos covered | I2–I7 | 30 nov–1 dec | ⬜ (You) |
| I9 | Drone (added 20 nov) | Hexacopter with a Pixhawk 4 flight controller: inventory (firmware PX4 or ArduPilot, version, radio, GPS), bench tests with the propellers off, then a manifest and a link adapter. No software exists for it yet (MAVLink adapter, drone device manifest); see the note below | D1 (manifests), E5 (daemon links), a MAVLink adapter not yet specified | not scheduled | ⬜ (You) |

Hell's plan (20 nov): tonight, the Arduino boards and the Raspberry Pi 5 with the other
boards of I2 and I4 (the host node); next week, the PLCs (I5), the robot arms (I6), the
FPGAs (I3) and the more complex cells (I7). The software side of every session is done
and checked without hardware; each session only confirms it on the device.

Drone note: the roadmap listed drones only as future work. A Pixhawk 4 flies with PX4 or
ArduPilot and talks MAVLink, which no crate of this repository speaks. Before I9 can do
more than inventory, someone has to write the spec for a MAVLink link, a drone
manifest and the safety rules (arming, geofence, failsafe, propellers off on the bench).
Until then it is an inventory and bench item only, and the project makes no flight claim.

Lab safety for every session: isolated network, hardwired E-STOP, optocouplers/level
shifters between 3.3 V and 24 V, dry run first, reduced speed for motion.

---

## Phase J — Integration and polish · H+M · 29 nov–1 dec

| ID | Task | Owner |
|---|---|---|
| J1 | End-to-end tests of routes A–D (Playwright "judge path" on every commit) | M |
| J2 | Security review (secrets, auth, signed packages, command allowlist) | H |
| J3 | Performance (twin frame time, load time < 5 s), accessibility, browsers (Chrome, Edge, Firefox, Safari fallback) | M |
| J4 | Resilience (network loss, daemon down, lab offline → twin) | H |
| J5 | Clean-machine install of web, desktop and APK; docs pass | H+M |

## Phase K — Submission and Kiro evidence · H+M · continuous, final 1–3 dec

| ID | Task | Owner |
|---|---|---|
| K1 | `docs/kiro-log.md` kept current (specs, steering, hooks, MCP, agents, PBT, powers, IDE/CLI/Crew comparison) | H+M |
| K2 | "How we used Kiro" write-up | H |
| K3 | Devpost text, testing instructions with any credentials (English) | H |
| K4 | Demo video < 3 min, English, no third-party trademarks or copyrighted music | You |
| K5 | Blog post (mentions Kiroween 2026) and social post starting with `#KiroweenHackathon26`, written by the team | You |
| K6 | Student details for both members; team agreement signed | You |
| K7 | `release/kiroween-1.0`, final compliance check (license, `.kiro`, repo public) | H |
| K8 | Judging operations until 2027-01-29 (monitoring, budget, urgent fixes only) | H |

## Phase L — After the minimum viable product (backlog, executed as time allows)

wgpu renderer · VHDL, URScript, RAPID, KRL, G-code importers/emitters · full PLCopen ·
MSP430 native backend · full Z3 integration and placement optimizer · OPC UA server ·
real-time collaboration (Yjs) · VS Code extension and plugin SDK · iOS app · macOS
daemon · Yosys/nextpnr flow · drones and sensor networks · community manifest library.

---

## Human actions (You)

| When | Action |
|---|---|
| Now | Review the language spec (`spec/language/`) |
| Now | Martin's GitHub username (collaborator + CODEOWNERS) |
| Now | Protect `main`/`develop`; enable private vulnerability reporting |
| Now | Check drive E: (`chkdsk E: /scan`) and exclude the project from sync/antivirus scanning |
| Before 19 nov | Install `avr-gcc` and `avrdude` (or the Arduino IDE) and a host C compiler; see `docs/phase-b-prerequisites.md` |
| Before 17 nov | Lab preparation and exact device inventory (both labs) |
| 17 nov | Session I1 (measurements) |
| By 21 nov | Neon project, Render service (paid), Vercel project |
| By 25 nov | Review AI provider terms; one development account |
| Any time | Send the three questions to the hackathon manager (`docs/compliance.md`) |
| 30 nov–3 dec | Video, blog, social post, Devpost form |

## Critical path

A4 → A5 → A6 → A7 → A9 → A10 → B1/B2 → B3 → B4 (**G2**) → B5/B7 → C2/C3 → D3 →
F7 → G6 → J → K. A delay in Phase A moves everything; Phase A has priority over all
other Hell tasks until G1.

## Daily rhythm

15-minute sync between Hell and Martin; update this file's statuses; one PR per task
into `develop`; `main` receives `develop` at each gate.
