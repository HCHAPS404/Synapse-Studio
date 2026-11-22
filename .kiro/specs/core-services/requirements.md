# Requirements: services (Phase E, tasks E2 to E7)
Status: v1.0 draft, for review by Hell. Parent specs: `neuron-cell` (device manifests and
timing profiles), `web-skeleton`. Normative text: `spec/protocols/relay.md` and
`spec/schemas/relay/` (E4 contract, done), `README.md` "Safety and verification".
Roadmap rows E2 to E7. E1 (WASM and PyO3 bindings) is done and not repeated here.
## Context
Today `services/core` only answers `GET /health` and checks the Neon database. The web
IDE, the desktop and Android apps, the digital twin and the lab hardware need a backend:
projects and builds stored in a database, users and roles, a live relay between twin,
daemon and clients, and an Edge Daemon that talks to real devices. Phase E builds these
and deploys the judging environment.
Phase D is implemented first; Phase E follows in the order E2, E3, E4, E5, E6, E7.
Everything runs without hardware. Daemon drivers are tested against simulated servers;
acceptance on real devices waits for sessions I1 and I4 to I7. AI is not part of Phase E.
## User stories
- As a judge, I want to open the public URL and build a project through the API without
  an account, and never reach a device.
- As an integrator, I want projects with immutable versions and reproducible builds.
- As an operator, I want live telemetry and simulation control in the web and Android
  apps, and a supplementary stop request that never replaces the hardwired E-STOP.
- As an engineer, I want to deploy a signed package to a lab device only after
  verification and two confirmations, with every step in an audit log.
- As the lead, I want measured timing from the lab to change the cell timing checks.
## Requirements
| Id | Requirement | Acceptance |
|---|---|---|
| E-R1 | Core API resources: projects, immutable versions (content hash), files, builds, device catalog, under `/v1`. Explicit response models, pagination, optimistic concurrency, errors as `{code, message, request_id}` | API tests for every route; a version cannot be modified; a stale write is `NE1107` |
| E-R2 | Compile jobs: `check`, `build`, `emit`, `import` through the `neuron` binary as a subprocess with timeout, size limits, a temporary directory per job, no shell, no network, and the diagnostics returned as data | a job over a limit is `NE1101`; a hung job is killed (`NE1102`); no secret reaches the job environment (test) |
| E-R3 | Guest mode for judges: no login; a sandbox of projects and builds with quotas and expiry; a guest can never list, read or command devices | a guest builds a corpus example through the API; the same guest gets `NE1106` on every device route |
| E-R4 | Rate limits per client (IP, guest id, user) on API, compile jobs and WebSocket control messages | property test: accepted requests in any window never exceed burst plus rate times the window; over the limit is `NE1103` |
| E-R5 | OpenAPI is exported from FastAPI to `spec/api/openapi.json`; `packages/sdk-ts` types are generated from it, never hand-written; CI fails on drift | CI step regenerates both and diffs; a changed model without regeneration fails |
| E-R6 | Authentication: users with argon2 password hashes, short-lived JWT access tokens signed with a secret from the environment, service tokens for the daemon and the twin, guest tokens | login, expiry, tampered token, wrong password and revoked service token tests; secrets and tokens never in logs |
| E-R7 | Roles `viewer`, `operator`, `engineer` plus `guest` and the service roles `daemon` and `twin`; one permission matrix used by REST and WebSocket | tests deny every cell of the matrix that is not granted; property: permissions grow monotonically viewer, operator, engineer; the relay table equals `relay.md` |
| E-R8 | Audit log: append-only record of logins, role changes, builds, device commands, deployments, confirmations and E-STOP requests, with actor, role, request id and result | each audited action writes one row; the engineer can read it; no update or delete path exists |
| E-R9 | Relay server: a WebSocket endpoint in the same FastAPI service that implements `spec/protocols/relay.md` v1: lifecycle, roles, subscriptions, snapshots, ping, backpressure, error codes | every example in `spec/protocols/examples/` validates; messages round trip through the JSON Schemas; a role sending a forbidden message gets `forbidden`; the relay never forwards `sim_*` to real devices |
| E-R10 | Daemon pairing: a one-time code issued to an engineer binds a daemon to a cell and gives it a service token; unpairing revokes it | pairing, reuse of a code, expired code and revoked token tests; the daemon shows as `deployment_status` |
| E-R11 | Telemetry in plain Postgres tables on Neon with time-based retention (TimescaleDB is not assumed); query API by device, tag and time range | ingest of 500-sample batches, range queries, retention job removes only expired rows (property test) |
| E-R12 | Edge Daemon (`services/edge-daemon`): drivers for serial, Modbus TCP and RTU, S7, TCP sockets, MQTT and CAN behind one driver interface; flashing through the existing tools; serves the PWA on the LAN; works offline | each driver passes its tests against a simulated peer; the daemon starts and serves with no cloud |
| E-R13 | Signed packages and command allowlist: the daemon applies only Ed25519-signed packages from a trusted key and executes only allowlisted commands | tampered package `NE1108`; a command outside the allowlist `NE1109`; unsigned and wrong-key cases |
| E-R14 | Safe commissioning: a deployment needs a passing build with verification, a dry run, reduced speed and two recorded human confirmations; otherwise it is blocked | deploy without any of the four gives `NE1110`; the state machine admits no shortcut (property test over event sequences) |
| E-R15 | Heartbeat and local E-STOP: the daemon reports heartbeat and the E-STOP state it reads from an input; the software E-STOP is supplementary. No message or API releases, bypasses or disables a safety function | tests: request forwarding, `estop_state` comes only from the daemon, daemon offline gives `unknown`; review of the "never claims" wording |
| E-R16 | Black box: the daemon keeps a ring buffer and, on a fault, stores a record of the window around it; records upload to the API and replay in the twin format | a simulated fault yields a record with the pre and post window; the record validates against its JSON Schema and converts to `replay.schema.json` |
| E-R17 | Calibration: a pipeline turns measured round trips into a timing profile file for the device manifests (`trust = "measured"`), with generic numbers until session I1 | a profile built from synthetic samples changes an `NE0701` result of `neuron cell check`; an invalid profile is `NE1112` |
| E-R18 | Deployment: Render (Docker), Neon branches, Vercel, migrations on deploy, health, monitoring and a spending plan until 2027-01-29; the public URL works with no login | live smoke script passes against production; a migration failure stops the deploy |
| E-R19 | Engineering rules: FastAPI async handlers, dependency injection, Pydantic v2 at every boundary, SQLAlchemy 2 and Alembic, parameterized queries, structured logging, secrets from the environment, `pyright` and `ruff` clean | CI; a test greps that no secret value appears in logs |
| E-R20 | The product works without hardware; hardware items are marked for their session; no claim of IEC 61508 or ISO 13849 | review of the docs and UI strings |
## Correctness criteria
- **Permission monotonicity.** For every permission, if `viewer` has it then `operator` and
  `engineer` have it; if `operator` has it, `engineer` has it.
- **Guest isolation.** No permission of `guest` touches a device, a daemon or a real-device
  telemetry stream.
- **Rate bound.** Per client and window `W`, accepted requests are at most `burst + rate * W`.
- **Message fidelity.** Every valid relay message parses, validates against the JSON Schema
  and serializes to the same JSON.
- **Deployment gate.** A deployment reaches `applied` only through
  `verified, dry_run_ok, confirmed_1, confirmed_2`, in that order, with a valid signature.
- **Determinism.** The same project version and the same `neuron` binary give the same
  build artifacts and diagnostics.
## New error codes (family NE11xx, services)
`NE1101` job rejected (size or limit), `NE1102` job timed out or killed, `NE1103` rate limit,
`NE1104` guest quota or sandbox expired, `NE1105` authentication failed, `NE1106` permission
denied, `NE1107` version conflict, `NE1108` package signature invalid, `NE1109` command not
allowed, `NE1110` deployment blocked, `NE1111` device offline or not paired, `NE1112`
timing profile invalid. Each gets an entry in `docs/errors/services.md`.
## Out of scope
Drones and MAVLink (session I9); AI features; multi-instance relay (one Render instance);
user self-registration with email; OAuth providers; TimescaleDB; certified safety claims;
a PDF dossier (Phase D); wasm or PyO3 compilation in the API.
