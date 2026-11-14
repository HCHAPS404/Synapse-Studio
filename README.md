# SYNAPSE Studio

> **Design once. Simulate in 3D. Compile to any machine.**

SYNAPSE Studio is a software-defined industrial automation framework built around
**Neuron**, a new programming language and canonical intermediate representation
(IR). Neuron lets you:

1. **Program devices directly.** Compile `.neuron` source to native firmware for
   microcontrollers (STMicroelectronics, Texas Instruments, Arduino, Raspberry Pi
   Pico, ESP32 and more).
2. **Translate between ecosystems.** Import and emit languages such as IEC 61131-3
   (ST, SCL, Ladder, FBD, SFC), SPEL+, C/C++, Python, Verilog and assembly, so one
   logic can run on a PLC, a robot arm, an FPGA or an MCU.
3. **Verify before you energize.** Check mutual exclusion, workspace bounds,
   electrical compatibility, timing and logic (model checking) before deployment.
4. **Simulate everything in a digital twin.** A from-scratch 3D engine with
   physics, kinematics and virtual devices that run the generated code, so the
   whole system can be evaluated without any physical hardware.
5. **Deploy to real hardware** through the Edge Daemon, and control cells from web,
   desktop and mobile.

Submitted to **Kiroween 2026**, category **Frankenstein**.

## Status

Early development. The project is being built in public during the Kiroween 2026
submission period (30 October to 4 December 2026). See [`docs/`](docs/) for the
architecture decisions and [`.kiro/`](.kiro/) for the specs, steering and hooks
used to build it with Kiro.

## Try it (judges)

> The virtual lab (no account, no hardware required) will be linked here once
> deployed. Until then, follow the developer quick start below.

## Developer quick start

Prerequisites: Git, Rust (stable), Node.js 24+, pnpm, Python 3.12+, uv, Docker.

```powershell
# Rust workspace (language, compiler, engine)
cargo test --workspace

# Core API service (Python)
cd services/core
uv sync
uv run pytest
uv run uvicorn synapse_core.main:app --reload
```

A `Taskfile.yml` wraps common commands (`task --list`). Install Task with
`winget install Task.Task` if you want to use it.

## Repository layout

```
.kiro/        Kiro specs, steering, hooks, agents and MCP settings
crates/       Rust workspace: Neuron language, compiler, backends, engine
lib/          Neuron standard library (written in Neuron)
services/     Core API (FastAPI) and Edge Daemon (Python)
apps/         Web (Next.js PWA), desktop (Tauri), mobile (Kotlin Multiplatform)
packages/     Shared TypeScript packages
spec/         Language and format specifications, JSON Schemas
conformance/  Golden files and conformance vectors
devices/      Device manifests and board profiles
targets/      Architecture and MCU target descriptions
docs/         Architecture decision records, guides, compliance notes
tools/        Repository tooling scripts
```

## Safety notice

SYNAPSE Studio is an engineering **design aid**. It is not a safety-certified tool
(it does not claim compliance with IEC 61508 or ISO 13849). A hardwired emergency
stop is always the authority; the software emergency stop is supplementary. Never
run generated code on real machinery without independent validation, dry runs and
reduced-speed commissioning.

## License

Apache License 2.0. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).
Vendor and product names are trademarks of their respective owners and are used
only to describe compatibility.
