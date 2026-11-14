# Repository structure

```
.kiro/        specs, steering, hooks, agents, settings (never gitignored)
crates/       Rust workspace (neuron-*, synapse-engine-*)
lib/          Neuron standard library (core units hal logic control motion proto
              dsp drivers safety time diag test bsp)
targets/      architecture / MCU target descriptions (.target)
devices/      device manifests and board profiles
apps/         web, desktop, mobile
services/     core (FastAPI), edge-daemon
packages/     shared TypeScript packages (ui-kit, sdk-ts, wasm-glue)
spec/         language and format specs, JSON Schemas
conformance/  golden files and conformance vectors
scenes/       digital-twin scenes (.scene.json)
examples/     Neuron example programs
infra/        deployment (render.yaml, docker, CI helpers)
docs/         ADRs, guides, compliance, kiro-log.md
tools/        repository scripts
```

## File formats

- `.neuron`: Neuron source (human-authored, versioned in git).
- `.nir.json`: canonical IR interchange (validated with JSON Schema).
- `.layout.json`: diagram positions (kept separate to avoid noisy diffs).
- `.scene.json`: digital-twin scene. `.target`: architecture description.
- `neuron.toml` / `neuron.lock`: package manifest and lockfile.

## Branching

`main` (protected, deployed) and `develop` (integration). Short-lived branches:
`feat/<area>-<topic>`, `spec/`, `fix/`, `hw/<device>-<date>`, `docs/`, `chore/`,
`exp/`. Releases: `release/kiroween-1.0`; urgent: `hotfix/`.
