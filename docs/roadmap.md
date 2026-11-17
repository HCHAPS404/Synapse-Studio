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
| `neuron test examples` on the whole package | H | m06, p03 and p09 fail only when the package is tested as a whole; each corpus folder passes (CI tests per folder). Find the interaction |
| G2 on the UNO, I2 boards, I3 FPGA, ESP32-S3 TFT dashboard | You | Commands in `docs/phase-b-prerequisites.md` |

---

## Phase C — Translation between languages · Hell · 22–29 nov

| ID | Task | Deliverable | Acceptance | Depends | Dates | Status |
|---|---|---|---|---|---|---|
| C1 | Emitter framework | Plugin API, fidelity report (exact/approximate/unsupported), package with code + config + wiring report | Golden files per emitter | A9 | 22 nov | ⬜ |
| C2 | IEC 61131-3 emitters | ST (generic and Yaskawa dialect), SCL, PLCopen XML (Ladder/FBD/SFC) | `matiec` compiles ST; import check in the vendor tool during I5 | C1 | 23–25 nov | ⬜ |
| C3 | Robot emitter | SPEL+ program + command-interpreter slave; workspace checks (NE0804) | Runs on the virtual robot; checked on the arm in I6 | C1 | 25–27 nov | ⬜ |
| C4 | Python / ROS 2 emitter | `host` dialect → Python nodes | Runs on a Raspberry Pi / PC | C1 | 26–28 nov | ⬜ |
| C5 | Importers | ST and PLCopen Ladder → Neuron; SPEL+ subset; C subset; Verilog subset; fidelity per construct | Round trip by traces (import → run in interpreter → equal outputs) | A9, A10 | 22–29 nov | ⬜ |
| C6 | Micro-Kernel hot reload | Small interpreters for PLC/robot/MCU that accept state tables from the daemon | Sequence changed without the vendor IDE in I5/I6 | C2, E5 | 27–29 nov | ⬜ |

---

## Phase D — Cell compiler (multi-device) · Hell · 23–28 nov

| ID | Task | Deliverable | Acceptance | Depends | Dates | Status |
|---|---|---|---|---|---|---|
| D1 | Device manifests and capabilities | Manifest schema, trust levels, capability palette, topologies (same class other vendor, replicas, heterogeneous) | Same module placed on two PLC types | B1 | 23 nov | ⬜ |
| D2 | Partitioner | `place` and automatic assignment; paradigm lowering per target | Bottling cell partitions into 4 programs | D1, C1 | 23–24 nov | ⬜ |
| D3 | Links and timing | Standard Time Model: link synthesis (both ends), worst-case latency (NE0701), Latch+ACK, BUSY/DONE/TIMEOUT, timestamps, cell watchdog; uses measured timing profiles | 200 µs on Modbus rejected with alternative proposed | D2, I1 | 24–26 nov | ⬜ |
| D4 | Safety across the cell | Mutex zones, workspace bounds, electrical compatibility (NE0801), cell `verify` | Counterexample replayed in the twin | D3, A8 | 26–27 nov | ⬜ |
| D5 | Technical dossier | Wiring, I/O list, pin tables, timing diagrams, BOM, test report (PDF) | Generated for the demo cell | D4 | 27–28 nov | ⬜ |

---

## Phase E — Services · Hell · 20–29 nov

| ID | Task | Deliverable | Depends | Dates | Status |
|---|---|---|---|---|---|
| E1 | Compiler bindings | PyO3 module and WASM build of the compiler (browser diagnostics without server) | A9 | 20–21 nov | ⬜ |
| E2 | Core API | Projects, versions, builds, devices; Neon + Alembic; guest mode for judges; rate limits | E1 | 21–24 nov | ⬜ |
| E3 | Auth, RBAC, audit | viewer / operator / engineer roles; audit log | E2 | 24–25 nov | ⬜ |
| E4 | Relay and twin gateway | WebSocket protocol, daemon pairing, telemetry (TimescaleDB) | E2 | 24–26 nov | ⬜ |
| E5 | Edge Daemon | Drivers: serial, Modbus TCP/RTU, S7, TCP sockets, MQTT, CAN; flashing tools; signed packages; heartbeat and local E-STOP; serves the PWA on the LAN | B4 | 21–29 nov | ⬜ |
| E6 | Black box and calibration | Records around faults; timing profiles fed back to D3 and the twin | E5, I1 | 26–29 nov | ⬜ |
| E7 | Deployment | Render (judging environment), Neon branches, Vercel; budget until 2027-01-29 | E2 | 25–28 nov | ⬜ |

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
