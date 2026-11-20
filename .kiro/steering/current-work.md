---
inclusion: always
---

# Current work and session handoff (updated 2026-11-19)

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
- GitHub `HCHAPS404/Synapse-Studio`. `develop` = `d1c963d` (C1 + C2, CI green).
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
  after a long foreground wait (`Start-Sleep` loops of minutes). Keep foreground commands
  short (at most `Start-Sleep 30`), and poll a background job by reading its log with the
  file reader instead of waiting in the shell.
- `tools/phase-check.ps1 [-MessageFile f]`: everything CI runs (CLI checks on examples,
  corpus and lib, smoke runs), then `tools/ship.ps1` (fmt, clippy, tests, commit -s, push).
- `tools/verify-worktree.ps1`: compares every tracked file with git (disk damage check).
- Windows App Control may block fresh test executables (os error 4551): rerun.
- If `target/` gets damaged, delete it or set `CARGO_INCREMENTAL=0`.

## Phase status

| Phase | Status |
|---|---|
| 0, A, B | done. B4–B9 verified without hardware; device steps pending (G2 UNO, I2 boards, I3 FPGA, ESP32-S3 TFT) |
| C | **in progress** (below) |
| D, E, H | not started. After C: E1 (WASM + PyO3 compiler bindings, overdue, blocks Martin's G3) and the E4 protocol contract (`spec/protocols/relay.md`) come first |
| F, G | Martin, not started; his F2 engine WASM API and `.scene.json` are overdue |

Phase B facts: `neuron test examples` passes 18/18 since A10.12 (each test runs its module
and what it refers to). Workspace tests: 408 passed before Phase C. Open spec items:
independent review for `spec-v1.0`, VS Code extension manual test.

## Phase C: where it stands

Spec: `.kiro/specs/neuron-translate/` (requirements T1–T9, design, tasks C1.1–C7). Read it.

Done (C1, C2, committed with "feat(emit,st): IEC 61131-3 emitters ..."):
- `crates/neuron-st`: ST lexer, parser, AST and virtual PLC (`Plc`, `Clock::ScanCounted`
  and `Clock::WallClock`, std FBs TON TOF TP CTU CTD CTUD R_TRIG F_TRIG SR RS, user FBs and
  functions, SCL spelling `#x` and `"x"`).
- `crates/neuron-emit`: framework (`Emitter`, `Package`, `Fidelity`, `Policy` → NE1003,
  `Writer` with source maps, `nirx` helpers) and `iec/` (`gen.rs` shared ST/SCL lowering,
  `helpers.rs` std.control FBs, `mod.rs` St and Scl emitters, `plcopen.rs`).
- `crates/neuron-emit/tests/c2_iec.rs`: 9 tests pass (exact equality with the interpreter
  for every `plc` corpus program with scan-counted timers; wall-clock within one scan,
  sequences within two; flasher period 1020 ms in the reference, +1 scan per timer on the
  PLC; policy; determinism and maps; yaskawa; SCL equals ST; PLCopen well formed; NE0610).
- NE10xx in chapter 10, `docs/errors/translate.md` (in `neuron explain`), catalogue test
  scans `neuron-emit` and `neuron-st`.

Next, in order:
1. NE0804 becomes implemented by C3 (update `docs/errors/reserved.md` when C3 lands).
2. C5.1 importers ST and PLCopen (new crate `neuron-import`, roxmltree), round trip by traces.
4. C3 starts with interpreter gaps found on 2026-11-19 (no test covered them):
   `point`/`frame`/`tool` are opaque (`pallet.xyz(...)` drops its arguments, `p.above` is
   missing); a `for` runs in one step, so `move`/`wait`/`until` inside a loop never wait
   (r01, r02, r05); sequence locals (`let p`, loop variable) are cleared at each activation;
   `until part_ok` on an `io` input may never be true (check). Sequences tick every 1 ms,
   a motion takes `move_ns` = 1 s. Add a motion observable to the trace. Then:
   C3: robot manifests `devices/robots/*.toml` (generic models), NE0804, SPEL+-style emitter
   and slave, crate `neuron-spel` (virtual controller), differential tests; C5.2 importer.
5. C4: Python emitter + `neuron_rt.py`, tests run with Python in simulated time.
6. C5.3: C subset and Verilog subset importers.
7. C6: state tables `.nst.json` + word encoding, reference executor, ST/C99/SPEL+ kernels.
8. C7: CLI `neuron emit <st|scl|plcopen|spel|python|kernel>` and `neuron import <...>`
   (put it in a new `crates/neuron-cli/src/translate.rs`), `docs/translation.md`, CI steps,
   roadmap rows C1–C6, README status, kiro-log entry.

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
- Robot (planned): SPEL+-style mapping in the design doc; mark vendor details to confirm in
  session I6. No vendor manuals copied, names only descriptive.

## Human actions still open (Hell)

Add Martin as collaborator and protect `main`/`develop`; Neon, Render (paid) and Vercel
accounts before 21 nov; session I1 (inventory, measurements); G2 with the UNO; spec review.
