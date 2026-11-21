---
inclusion: always
---

# Current work and session handoff (updated 2026-11-20)

Read this first in every new session. It records the state, the decisions and the
working rules agreed with Hell (the user) that are not written anywhere else. Update it
when a task finishes; delete sections that become obsolete.

## People and how to work with Hell

- Hell is the lead (language, compiler, services, hardware, AI). Martin is the teammate
  (engine, twin, web, mobile); his guide is `docs/martin.md`.
- Hell writes in Spanish: **answer in Spanish**. Code, comments, docs, commits: English.
- Hell wants speed, quality and few wasted tokens. He gave standing approval to execute
  what is needed; work autonomously, task by task.
- After **each finished task**: tests green, `docs/kiro-log.md` entry, roadmap and README
  status updated, Conventional Commit with `-s`, push to `develop`, check CI.
- Report results with tables when useful. Be honest about what is verified and what is not.

## Repository state

- Working copy: **`C:\dev\Synapse-Studio`** (primary). `E:\PROYECTOS\HACKATHONES\Kiroween 2026`
  is only a backup: a loose USB cable damaged files five times; on 2026-11-19 `chkdsk /f`
  removed 65 tracked files, restored from git. Do not work on E:.
- GitHub `HCHAPS404/Synapse-Studio`. `develop` = `d1c963d` (C1 + C2, CI green); the work
  of C3 to C7 is local and not yet committed or pushed when this was written.
  The C: clone has `git config user.name hchaps404` and `user.email helmut.chs@gmail.com`. `main` = `a04682f`
  (promote `develop` to `main` at the next gate with `tools/promote-main.ps1`, after asking).
  Tag `phase-b-software` = `a04682f`. Martin's branch: `feat/engine-kernel`.
- Toolchains: `%USERPROFILE%\toolchains` (w64devkit gcc 16.2, avr-gcc 7.3, avrdude 6.3,
  arm-none-eabi-gcc 15.2.1). Rust 1.99.1, Python 3.12, uv, Node 24, pnpm 9, gh.
- Agent permissions: `%USERPROFILE%\.kiro\settings\permissions.yaml` (paths point to C:;
  backup `permissions.yaml.bak`). The agent cannot edit it; Hell does.

## Terminal and tools (important)

- The editor terminal often garbles or drops output and returns exit code -1. Run long
  jobs detached and read their logs: `tools/job.ps1 -Cmd "..." -Log name.log` writes
  `out/name.log` ending with `JOB END exit=N`. Run one cargo job at a time (they block on
  the build lock). The file reader can lag a few seconds behind new files.
- **How to launch it (agent):** call the shell tool with `run_in_background: true` and the
  command `powershell -NoProfile -ExecutionPolicy Bypass -File tools\job.ps1 -Cmd "cargo ..." -Log x.log`.
  Do NOT wrap it in `Start-Process ... -WindowStyle Hidden` from the agent shell: on
  2026-11-19 that silently never ran (no log file). Read `out/x.log` with the file reader.
- When the agent shell stops running commands at all (even a one-line `Out-File`),
  Hell has to restart the editor terminal or Kiro. Both times on 2026-11-19 it died right
  after a long foreground wait (`Start-Sleep` loops of minutes); a third time it died
  after a single `Start-Sleep 25` while a cargo job ran. Avoid `Start-Sleep` in the agent
  shell altogether: poll a background job only by reading its log with the file reader.
  The file reader keeps working while the shell is frozen.
- **Memory (2026-11-21):** a full rebuild with one rustc per core ran the PC out of RAM
  and page file (os error 1455, "memory allocation failed") and froze the editor.
  `tools/job.ps1` now sets `CARGO_BUILD_JOBS=2` unless it is already set. Changing a
  feature of a shared dependency (e.g. `serde_json`) rebuilds the whole workspace.
- Sub-agent rule that worked (C5.3 to C7): edit files only with the file tools, never from
  the shell; poll jobs only with the file reader.
- `tools/phase-check.ps1 [-MessageFile f]`: everything CI runs (CLI checks on examples,
  corpus and lib, smoke runs including `neuron emit` and `neuron import`), then
  `tools/ship.ps1` (fmt, clippy, tests, commit -s, push).
- `tools/verify-worktree.ps1`: compares every tracked file with git (disk damage check).
- Windows App Control may block fresh test executables (os error 4551): rerun.
- If `target/` gets damaged, delete it or set `CARGO_INCREMENTAL=0`.

## Phase status

| Phase | Status |
|---|---|
| 0, A, B | done. B4–B9 verified without hardware; device steps pending (G2 UNO, I2 boards, I3 FPGA, ESP32-S3 TFT) |
| C | **done** (C1–C7, 2026-11-20; 517 workspace tests). Open: hardware sessions I5 (yaskawa profile, vendor import, ST kernel on a PLC) and I6 (SPEL+ dialect, `NeuronSlave.prg`, `NeuronLib.prg`), `--ros2`, timers/counters/fsm in the Python emitter |
| E1 | **done** (2026-11-21): `crates/neuron-wasm` (JSON API in `src/api.rs`: check, format, hover, definition, run, test, emit, importSource, version; wasm-bindgen exports on wasm32; CI builds `wasm32-unknown-unknown`), `crates/neuron-py` (PyO3, maturin, excluded from the workspace: `cargo test --workspace` never builds it), usage in `packages/wasm-glue/README.md`. Martin's G3 is unblocked. The E4 relay contract (`spec/protocols/relay.md`) is also done |
| D, E (E2–E7), H | not started (D1–D2 next) |
| F, G | Martin, not started; his F2 engine WASM API and `.scene.json` are overdue |

Phase B facts: `neuron test examples` passes 18/18 since A10.12 (each test runs its module
and what it refers to). Open spec items: independent review for `spec-v1.0`, VS Code
extension manual test.

## Phase C: done

Spec: `.kiro/specs/neuron-translate/` (requirements T1–T9, design, tasks with a "Results of"
paragraph per task). User guide: `docs/translation.md`.

- Crates: `neuron-emit` (framework, `iec`, `spel`, `python`, `kernel`), `neuron-st`,
  `neuron-spel` (virtual PLC and robot controller), `neuron-import` (`st`, `scl`, `plcopen`,
  `spel`, `c`, `verilog`), and in `neuron-cli` the module `src/translate.rs` with
  `neuron emit <st|scl|plcopen|spel|python|kernel> ... [-o dir] [--profile] [--robot]
  [--policy]` and `neuron import <st|scl|plcopen|spel|c|verilog> <file> [-o out.neuron]`
  (tests: `crates/neuron-cli/tests/c7_translate.rs`, 15 tests).
- CI and `tools/phase-check.ps1` run smoke steps: emit st, spel, python, kernel, then
  import of the emitted ST and `neuron check` of the result. CI sets `NEURON_REQUIRE_PYTHON=1`.
- Not exact, by design (the fidelity reports say it): IEC timers (`equivalent`, one scan),
  sequences (two scans), SPEL+ motion time and `blend`, 16-bit kernel arithmetic.
- Open: see the Phase status row (I5, I6, `--ros2`, Python timers/fsm), the
  the SPEL+ kernel's integer I/O uses modelled `InW`/`OutW` (real statements: I6), no C emitter for `core` functions.

Closed after C7 (2026-11-20): the Python emitter covers blocks, fsm, sequence, types,
contracts and a ROS 2 node (stub-tested); `neuron emit c` (C99 for `core`); the SPEL+
kernel is checked on integer ports (`OutW`/`InW`, to confirm in I6).

## Phase D: spec accepted (2026-11-20), in progress

Spec: `.kiro/specs/neuron-cell/` (requirements D-R1 to D-R11, design, tasks D1.1 to D7).
Hell accepted the three decisions and the order (E4 first, E1 alongside D1-D2, then
D3-D7). Decisions:
1. Link endpoints generated as NIR items added to each device package (every backend
   gets them for free) instead of per-backend endpoint code.
2. Co-simulation runs NIR in the reference interpreter, not the translated code in the
   virtual PLC/controllers (sound because Phase C checked translations against it).
3. Dossier as Markdown plus a self-contained HTML with inline SVG; PDF printed from it.
New codes planned: `NE0704` (placed on a device that cannot run the dialect), `NE0705`
(no device fits). Implemented today: none of NE0701, NE0703, NE0801, NE0805.

Next steps, in order:
1. (done) E4 relay protocol contract and E1 compiler bindings.
2. D1.1, D1.2, D2.1, D2.2.
3. D3.1 to D7, one task per commit. Ask Hell before promoting `develop` to `main`.

## Phase C design decisions (keep them)

- Emitters read NIR only; importers write Neuron **source**, analyzed normally.
- PLC scan order (spec 14.3, matches the interpreter): safety (safety rules, reset, then
  `IF neuron_fault THEN <safe state>; RETURN`), rungs/rules/sequences/fsm in source order,
  then every std FB called once (end of scan) with **shadow input variables**
  (`jam.input = x` → `jam_IN := x;` and `jam(IN := jam_IN, PT := T#3S);`), then time
  accumulators advance by one scan.
- Timers: interpreter counts scans from the first true scan; IEC wall clock lags one scan.
  Fidelity `equivalent`; a timer loop (flasher) gets one scan longer per timer per cycle.
- Sequences: CASE over straight-line blocks with a bounded WHILE (pc, el, go, guard) so a
  wait is checked at once, like the interpreter. Bound: two scans earlier or later.
- `fault F` → `neuron_fault := TRUE; neuron_fault_code := n; <safe state>; RETURN;`
  with the code table in `wiring.md`.
- Profiles: `generic` (PROGRAM + CONFIGURATION) and `yaskawa` (GlobalVars.st, VAR_EXTERNAL,
  no CONFIGURATION; to be confirmed in session I5). SCL: FUNCTION_BLOCK + cyclic interrupt
  OB `Neuron_Cyclic_<name>` + `PLC_tags.csv`; memory lives in the instance DB.
- PLCopen: LD when all rungs are contact/coil networks, SFC when the module is one
  sequence, otherwise an ST body. Deterministic `creationDateTime`.
- Robot: SPEL+-style mapping in the design doc; vendor details to confirm in session I6.
  No vendor manuals copied, names only descriptive.

## Hardware testing plan (Hell, set on 2026-11-20)

Tests are on standby in the repo: software is checked without hardware, and every item
that needs a physical device stays pending until its session.
- **Tonight (20 nov):** Arduino boards (G2 UNO, I2) and the Raspberry Pi 5 (I4 host node).
- **Next week:** PLCs (I5), robot arms (I6), FPGAs (I3) and more complex cells (I7).
- **Drone (new, I9):** hexacopter with a Pixhawk 4. Inventory and bench only. The repo has
  no MAVLink adapter, no drone manifest and no spec for it; the roadmap listed drones
  as future work. Do not claim flight support. Needs a spec first (link, manifest,
  safety: arming, geofence, failsafe, propellers off on the bench).
- Pending because they need a device: yaskawa ST profile and vendor import (I5); SPEL+
  dialect, `NeuronSlave.prg`, `NeuronLib.prg` (I6); ST kernel on a PLC (I5); everything in
  B4-B9 on boards (I2, I3); Python `host` node on the Pi 5 (I4).
- Rule: work that does not need a device is done in the repo, not left pending.

## Human actions still open (Hell)

Add Martin as collaborator and protect `main`/`develop`; Neon, Render (paid) and Vercel
accounts before 21 nov; session I1 (inventory, measurements); G2 with the UNO; spec review.
