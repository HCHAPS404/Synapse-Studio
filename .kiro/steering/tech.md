# Technology stack

- **Rust** (workspace in `crates/`): Neuron language, compiler, backends, MCU ISA
  simulators, virtual devices and the Synapse Engine. Built to CLI, WASM (web and
  desktop), PyO3 (Python services) and JNI (Android).
- **Python 3.12**: Core API (FastAPI, Pydantic v2, SQLAlchemy 2, Alembic), Edge
  Daemon (pymodbus, python-snap7, pyserial, asyncua), AI gateway, manifest
  generation, reference interpreter (oracle). Tooling: `uv`, `ruff`, `pyright`,
  `pytest`, `hypothesis`.
- **Web**: Next.js (App Router), React, strict TypeScript, Tailwind, shadcn/ui,
  Zustand, TanStack Query, Monaco, dockview, Serwist (PWA), next-intl.
- **Desktop**: Tauri 2 (decision gate DG-1 vs Qt/PySide6 on 2026-11-24).
- **Mobile**: Kotlin Multiplatform + Compose Multiplatform (Android first).
- **Data**: Neon Postgres (TimescaleDB for telemetry), SQLite/Parquet in daemon.
- **Infra**: Render (backend, paid, always on), Vercel (web), GitHub Actions,
  Docker Compose, dev containers.
- **Orchestration**: cargo workspace, pnpm workspace, uv workspace, `Taskfile.yml`.

## Principles

- Single sources of truth: Rust/Pydantic models export JSON Schema; TypeScript
  types are generated from it. CI fails on schema drift.
- Determinism: fixed-step simulation, fixed evaluation order, seeded randomness.
- Property-based tests (proptest, Hypothesis, fast-check) for invariants.
- Prefer small, reviewable PRs. Spec first, then implementation.
- Never hard-code secrets. Use environment variables and secret stores.
