# Martin's guide: role, tasks and integration points

This page is for Martin. It lists what he owns, in which order, what he can reuse
from Hell's finished work, and how to deliver. The task IDs and dates come from
`docs/roadmap.md`, which stays the source of truth for status.

## Role

Martin owns the **Synapse Engine** (Rust, compiled to WASM), the **digital twin**, the
**web IDE and virtual lab**, the **Android app**, and shares the **desktop app** with
Hell. The product must be fully usable without hardware, so the twin and the virtual
lab are what the judges will use first.

Responsibilities:

- Keep the contracts below stable once published; announce breaking changes in the
  daily sync and bump their version.
- Write a spec in `.kiro/specs/<feature>/` (requirements, design, tasks) before each
  task, and append an evidence entry to `docs/kiro-log.md` after it.
- Keep the judge path (no login, "Try the virtual lab") working on every commit of
  `develop` from G6 on.
- Update the **Status** column of his rows in `docs/roadmap.md`.

## Where Hell's side stands (2026-11-17)

Phase A (language) and Phase B (direct programming) are done. What the twin can reuse
today:

| Need in the twin | Crate or file | Entry point |
|---|---|---|
| Program model | `spec/schemas/nir.schema.json`, `crates/neuron-nir` | `neuron emit-nir <pkg>` writes `.nir.json` |
| Run a program without a device model | `crates/neuron-interp` | discrete-event interpreter in simulated time, used as the oracle |
| Virtual AVR MCU (UNO) running the real firmware | `crates/neuron-sim-avr` | cycle-counted ATmega328P: GPIO, UART, timers, external interrupts |
| Virtual RP2040 running the real firmware | `crates/neuron-sim-thumb` | ARMv6-M core, SysTick, NVIC, RP2040 GPIO/UART/clocks |
| Virtual displays | `crates/neuron-ui` (`render`, `emu`) | `render` gives the reference image; `emu::replay` rebuilds the screen from the firmware's I2C/SPI bus traffic (SSD1306, TFT, HD44780) |
| Virtual FPGA | `crates/neuron-codegen-hdl` | `neuron emit-verilog <pkg>` writes Verilog-2005 plus a source map |
| Source maps for G5 inspectors | `app.map.json` (C backend), the Verilog map | map generated lines back to `.neuron` spans |
| Diagnostics in the editor | `crates/neuron-lsp`, `neuron check`, `neuron explain NE####` | WASM build of the compiler comes with E1 |

All of these are Rust crates with `#![forbid(unsafe_code)]`, deterministic output and
no I/O in the core, so they compile to `wasm32-unknown-unknown` for the engine.

## Contracts

| Contract | Owner | Due | Notes |
|---|---|---|---|
| NIR JSON schema | Hell | ✅ 15 nov | `spec/schemas/nir.schema.json`; CI fails on drift |
| **Engine WASM API** (load scene, step, inject signal, read state, snapshot/rewind) | **Martin** | **17 nov** | Hell plugs the virtual MCUs (F7) into it; publish as `spec/engine/api.md` plus a Rust trait |
| **`.scene.json` format** | **Martin** | **18 nov** | JSON Schema in `spec/schemas/scene.schema.json`; scenes live in `scenes/` |
| Twin / relay WebSocket protocol | Hell | 20 nov | Martin consumes it in web and mobile |
| Core API OpenAPI + `packages/sdk-ts` | Hell | 21 nov | generated types, never hand-written |

Suggested shape of the engine API, so the virtual MCUs fit without glue:

```rust
pub trait VirtualDevice {
    fn id(&self) -> &str;
    /// Advance the device to `t_ns` of simulated time.
    fn step_to(&mut self, t_ns: u64);
    /// Drive an input pin or signal from the scene.
    fn set_input(&mut self, port: &str, value: Value);
    /// Read outputs changed since the last call.
    fn drain_outputs(&mut self) -> Vec<(String, Value)>;
}
```

`neuron-sim-avr` and `neuron-sim-thumb` already step by cycles and expose pin levels,
so Hell will write the adapter once this trait is fixed.

## Tasks, in order

Dates are targets. "Accept" is what must be true to mark the row ✅.

### Phase G1 · design system (15–17 nov)
- Tokens, light/dark theme, shadcn/ui components, Storybook, `next-intl` with `en`
  default and `es` secondary, WCAG AA contrast.
- Accept: Storybook builds in CI; no hard-coded UI strings outside the `en` messages.

### Phase F · engine and twin (15–29 nov)
| ID | Do | Accept |
|---|---|---|
| F1 | Fixed-step kernel (`crates/synapse-engine-core`), ECS, snapshots and rewind, signal bus | Property test: same seed and inputs give the same state hash after N steps; rewind then replay gives the same hash |
| F2 | WASM API and `.scene.json` (the two contracts above) | Schema published; a sample scene in `scenes/` loads in a test |
| F3 | Three.js renderer behind the API; gizmos; overlays for zones, paths, signals and latency | 60 fps on the bottling scene on a mid laptop |
| F4 | 6-axis and SCARA kinematics: FK, IK (analytic plus damped numeric), limits, trapezoid and S profiles, PTP/linear/arc | Property test: FK(IK(p)) ≈ p within tolerance inside the workspace |
| F5 | Rigid bodies, contacts, friction, conveyors, grippers (Rapier only as a test oracle) | Bottle on a conveyor stops at a sensor in the same step every run |
| F6 | Pneumatic cylinders, valves, reed switches; DC, stepper and servo drives; optical, inductive and ADC sensors; rule-based electrical model | Each model has a unit test with a hand-computed value |
| F7 | Virtual devices with Hell: MCU (B6 simulators), FPGA (Verilog subset), PLC (ST subset, after C2), robot (SPEL+ core, after C3); fidelity badges | The UNO blink firmware toggles an LED in the scene with the same timing as `neuron run` |
| F8 | Scenarios for routes A–D (bottling cell, legacy Ladder, FPGA master + robot, bare-metal debug), free builder, fault injection | Each route plays end to end in the virtual lab |
| F9 | Recording and replay of lab sessions on the twin | A recorded session replays with the same signal trace |

### Phase G · client apps (17–30 nov)
| ID | Do | Accept |
|---|---|---|
| G2 | IDE shell in `apps/web`: dockview panels, explorer, command palette, problems panel, offline PWA (Serwist) | Works offline after the first load |
| G3 | Monaco with the Neuron grammar (reuse `editors/vscode/syntaxes`), diagnostics from the WASM compiler (E1), format on save | Typing an error shows the same `NE####` as `neuron check` |
| G4 | Graphical views: blocks, FSM/GRAFCET, Ladder (bidirectional with text), chip/pin view (from `targets/`), clock tree | Property VIEW-1: text → view → text gives the same NIR |
| G5 | Inspectors: generated code with the running line highlighted (source maps), bus monitor, scope, timing diagram, virtual display (`neuron-ui::emu`) | Stepping the virtual UNO highlights the matching `.neuron` line |
| G6 | Virtual lab for judges: no login, 3-minute guided tour, live-hardware indicator | Playwright judge path green (J1) |
| G7 | Desktop with Hell: DG-1 Tauri vs Qt on 24 nov, then USB/serial flashing through the embedded daemon; Windows and Linux installers | Installer flashes the UNO via `neuron flash` |
| G8 | Android (Kotlin Multiplatform, Compose): cells, telemetry, settings, supplementary E-STOP (never the authority), 3D viewer; APK in Releases | APK installs on a clean phone and connects to the relay |

### Phase J · integration (29 nov–1 dec)
- **J1:** Playwright "judge path" for routes A–D on every commit.
- **J3:** performance (twin frame time, load < 5 s), accessibility, Chrome, Edge,
  Firefox and a Safari fallback.
- **J5 (with Hell):** clean-machine install of web, desktop and APK.

### Phase K · submission
- K1: keep your entries in `docs/kiro-log.md` current.
- Review the video script for the twin parts (Hell records, K4).

## How to work

- **Branches:** start from `develop`, name `feat/engine-<topic>`, `feat/web-<topic>`,
  `feat/mobile-<topic>`, `spec/<feature>`. One PR per task into `develop`; `main`
  receives `develop` at each gate.
- **Commits:** Conventional Commits with sign-off: `git commit -s -m "feat(engine): fixed-step kernel"`.
- **Language:** code, comments, UI strings and docs in English; Spanish only in the
  `es` messages file.
- **Rust:** `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`. No `unwrap`/`expect` outside tests, documented public
  items, `proptest` for invariants, `BTreeMap`/`IndexMap` where order reaches output.
- **Web:** strict TypeScript, generated types from JSON Schema (never edit them by
  hand), `pnpm lint`, `pnpm test`, `fast-check` for invariants.
- **Assets:** generic 3D models only, no third-party logos or copyrighted music.
- **Safety:** the twin and the apps are design aids. Never claim IEC 61508 or
  ISO 13849 compliance in UI text. The phone E-STOP is supplementary; the hardwired
  E-STOP is the authority.
- **Errors:** new error codes use `NE####` with an entry in `docs/errors/`.
- **Done means:** approved spec, tests, docs, CI green, evidence in
  `docs/kiro-log.md`, roadmap status updated (see `CONTRIBUTING.md`).

## Build and try Hell's side

```powershell
cargo build -p neuron-cli
cargo run -p neuron-cli -- check examples
cargo run -p neuron-cli -- run examples/corpus/mcu/m01_blink.neuron
cargo run -p neuron-cli -- emit-nir examples > out/examples.nir.json
cargo run -p neuron-cli -- build examples/corpus/mcu/m01_blink.neuron --host
cargo run -p neuron-cli -- emit-verilog examples/corpus/hdl/d01_counter.neuron --board artix7_35t_board -o out/hdl
cargo run -p neuron-cli -- targets
```

C toolchains (gcc, avr-gcc, arm-none-eabi-gcc) install with
`tools/install-toolchains.ps1`; tests that need them skip when they are missing,
except in CI.

## Who to ask

- Language, compiler, simulators, targets, services, daemon, hardware: Hell.
- Anything that needs a human (accounts, hardware sessions, recordings, posts): note
  it in the "Human actions" table of `docs/roadmap.md`.
