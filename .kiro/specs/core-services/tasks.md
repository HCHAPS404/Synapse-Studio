# Tasks: services (Phase E)
Order: each task ends with tests green, a `docs/kiro-log.md` entry, the roadmap row, a
Conventional Commit with `-s`, and CI checked. One task per commit. Phase D is implemented
first; then E2, E3, E4, E5, E6, E7 in that order. "Needs hardware" is `no` when the task is
tested against simulated peers.
| Id | Task | Requirements | Needs hardware | Status |
|---|---|---|---|---|
| E2.1 | Service layout, settings, structured logging with redaction, `ErrorBody` and `NE11xx` handlers, SQLAlchemy async session, Alembic baseline, `docs/errors/services.md`; CI Postgres container | E-R1, E-R19 | no | open |
| E2.2 | `neuron check`, `build`, `emit --format json` (diagnostics and artifact list) with CLI tests | E-R2 | no | open |
| E2.3 | Compile runner: temp dir, path checks, limits, timeout and process-group kill, environment stripping, network self-test, `NE1101` `NE1102` | E-R2 | no | open |
| E2.4 | Projects, immutable versions and files (tables, routes, content hash, `row_version`, `NE1107`) | E-R1 | no | open |
| E2.5 | Builds API on the runner: queue and semaphore, stored diagnostics and artifacts with SHA-256, `GET /v1/capabilities` | E-R1, E-R2 | no | open |
| E2.6 | Device catalog from `devices/` manifests (read-only routes) | E-R1 | no | open |
| E2.7 | Token bucket limiter, per-client buckets, `Retry-After`, `NE1103`; property test of the rate bound | E-R4 | no | open |
| E2.8 | Guest mode: sandboxes, guest token, quotas, expiry cleanup, `NE1104`; a guest builds a corpus example | E-R3 | no | open |
| E2.9 | OpenAPI export, generated `packages/sdk-ts`, CI drift check | E-R5 | no | open |
| E2.10 | Docker image (Rust stage plus Python), `render.yaml` to `runtime: docker`, local `docker compose` for the API and Postgres | E-R2, E-R18 | no | open |
| E3.1 | Users, argon2 hashes, login, JWT access tokens, first-engineer CLI, `NE1105` | E-R6 | no | open |
| E3.2 | Permission matrix, route dependencies, role checks, `NE1106`; monotonicity and guest-isolation properties | E-R7, E-R3 | no | open |
| E3.3 | Service tokens (hashed, revocable) and their issue route | E-R6, E-R10 | no | open |
| E3.4 | Audit log table with `INSERT`/`SELECT` grants, audit writes on every sensitive action, read route for engineers | E-R8 | no | open |
| E4.1 | Relay core: `WS /v1/relay`, lifecycle (`hello`, `welcome`, ping, timeouts), schema validation, role table checked against `relay.md`, error codes | E-R9, E-R7 | no | open |
| E4.2 | Subscriptions, `cell_snapshot` cache, fan-out with bounded queues and drop policy, control rate limit; round-trip property over all examples | E-R9 | no | open |
| E4.3 | Daemon pairing (code, claim, token, unpair), `daemons` and `devices` tables, `deployment_status` and heartbeat timeout | E-R10 | no | open |
| E4.4 | Telemetry ingest from relay frames, tables, indexes, storage cap | E-R11 | no | open |
| E4.5 | Retention and 1-minute roll-up task with advisory lock; `GET /v1/telemetry`; property test of retention | E-R11 | no | open |
| E4.6 | Advisory E-STOP path and sim control routing (twin only, simulated devices only), audit entries | E-R9, E-R15, E-R8 | no | open |
| E5.1 | Daemon package: config, `Driver` protocol, simulated driver, relay client with reconnect, local LAN mode, pairing client | E-R12, E-R10 | no | open |
| E5.2 | Signed packages: manifest, Ed25519 sign (API side) and verify (daemon), quarantine, `NE1108` | E-R13 | no | open |
| E5.3 | Command allowlist with per-command Pydantic arguments, `NE1109`, `NE1111` for offline devices | E-R13 | no | open |
| E5.4 | Deployment state machine in the API and the daemon, `NE1110`, property test over event sequences | E-R14 | no | open |
| E5.5 | Serial driver and `Flasher` (avrdude, UF2) with fake tools and `loop://` | E-R12 | no | open |
| E5.6 | Modbus TCP and RTU drivers against a pymodbus server and a loopback serial pair | E-R12 | no | open |
| E5.7 | S7 driver against the snap7 server or a mock | E-R12 | no | open |
| E5.8 | TCP socket and MQTT drivers against a local server and an in-process broker | E-R12 | no | open |
| E5.9 | CAN driver on the python-can virtual bus | E-R12 | no | open |
| E5.10 | Heartbeat, local E-STOP input (read-only), `estop_state`, advisory request handling, safe state outputs | E-R15 | no (simulated input) | open |
| E5.11 | Serve the PWA on the LAN and offline operation; `docs/daemon.md` with the safety wording | E-R12, E-R20 | no | open |
| E5.12 | OPC UA driver against an in-process `asyncua` server (stretch, after E5.1 to E5.11) | E-R12 | no | open |
| E5-HW | Daemon flashes the UNO and talks to a PLC and a robot controller in the lab; Python `host` node on the Pi 5 | E-R12, E-R14 | **yes** | pending (I4 to I7) |
| E6.1 | Black box: ring buffer, fault window, local SQLite store, record JSON Schema | E-R16 | no | open |
| E6.2 | Upload route and table, retry on restart, conversion to the replay format | E-R16 | no | open |
| E6.3 | Calibration pipeline and profile TOML; `neuron cell check --profile`; a synthetic profile changes an `NE0701` result; `NE1112` | E-R17 | no (generic and synthetic numbers) | open |
| E6-HW | Measure the lab devices (I1), commit measured profiles, mark manifests `measured`, rerun the D3 budgets | E-R17 | **yes** | pending (I1) |
| E7.1 | Alembic on deploy (`preDeployCommand`), migration round-trip CI job, secrets list in `render.yaml` | E-R18 | no | open |
| E7.2 | Neon `dev` branch for CI and local use; per-environment settings; `docs/deploy.md` | E-R18 | no (needs Hell for the branch if MCP is revoked) | open |
| E7.3 | `tools/live-smoke.py` and scheduled GitHub Action; production check of guest, build and relay | E-R18, E-R3 | no | open |
| E7.4 | Judge path: public URL configuration in the web app, CORS, guest flow end to end (depends on G6) | E-R3, E-R18 | no | open |
| E7.5 | Monitoring and runbook until 2027-01-29: uptime monitor, alerts, `docs/runbook.md`, shutdown plan | E-R18 | no | open |
| E7-H | Spending caps and alerts on Render and Neon; external uptime monitor account | E-R18 | human action (Hell) | pending |
Ordering note. E2.1 to E2.3 unblock everything else; E2.2 touches `neuron-cli` (Rust) and
runs before the runner. E2.10 comes before the first production deploy of E2.5, so Render
never runs a build the image cannot make. E3.2 must land before any route that is not
public. E4.1 needs E3.1 and E3.3 (tokens). E5.4 needs E3.2, E3.4 and E4.6. E5.5 to E5.9
are independent and can be reordered. E6.3 needs the Phase D `neuron cell check` command.
E7 tasks start with E7.1 as soon as E2.1 has a migration; E7.3 to E7.5 close the phase.
Notes. Hardware rows stay pending until their session. The daemon, the relay and every
screen text describe the software E-STOP as supplementary, and nothing claims IEC 61508
or ISO 13849.
