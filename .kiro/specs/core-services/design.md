# Design: services (Phase E)
## Decisions accepted
Accepted by Hell on 2026-11-22:
1. **Compilation in the API.** A Docker image on Render with a Rust build stage that builds
   `neuron-cli`, plus Python. The API runs the binary as a subprocess with a timeout, a
   size limit, a temporary directory per job, no shell and no network. The PyO3 wheel is
   not used. `render.yaml` moves to `runtime: docker`.
2. **Authentication (E3).** Own and minimal: users with argon2 hashes, short-lived JWT
   access tokens signed with a secret from the environment, roles viewer, operator,
   engineer. Guest mode without login for judges: a guest creates and builds projects in a
   sandbox with rate limits and expiry, and never reaches devices. Audit log table.
3. **Telemetry (E4).** Plain Postgres tables on Neon with time-based retention. TimescaleDB
   is not assumed.
4. **Relay (E4).** A WebSocket endpoint inside the same FastAPI service (one Render service).
5. **Order.** Phase D first, then E2, E3, E4, E5, E6, E7.
## Layout
```
services/core/src/synapse_core/
  main.py          app factory, router registration, lifespan (retention task)
  settings.py      env settings (SecretStr for secrets)
  api/             routers only: parse, call a service, return a response model
  schemas/         Pydantic v2 request and response models, ErrorBody
  services/        business logic: projects, builds, guests, deployments, pairing
  db/              SQLAlchemy 2 models, async session, repositories
  auth/            passwords, tokens, permission matrix, dependencies
  compile/         job runner (subprocess sandbox), diagnostics parser
  relay/           hub, sessions, validation, role table
  telemetry/       ingest, query, retention
  limits.py        token bucket
  errors.py        NE11xx catalogue and exception handlers
services/core/migrations/        Alembic
services/edge-daemon/            package synapse_daemon (own pyproject, uv workspace member)
spec/api/openapi.json            exported OpenAPI (committed, CI-checked)
packages/sdk-ts/                 generated types and a thin typed client
```
Routes hold no logic. Handlers are `async`; dependencies inject settings, a database
session, the current principal and the rate limiter. The blocking `psycopg` health check
stays in a thread; all other database access uses SQLAlchemy async with `postgresql+psycopg`.
Errors return `{code, message, request_id}` (`ErrorBody`); `code` is an `NE11xx` string.
Logging is structured JSON with `request_id`; a filter redacts tokens, passwords and URLs
with credentials.
## E2 Core API
Tables (Alembic revision per task):
| Table | Main columns |
|---|---|
| `projects` | id, owner_id (null for guest), sandbox_id (null for users), name, created_at, updated_at, row_version |
| `project_versions` | id, project_id, number, content_hash, created_at, message; immutable |
| `version_files` | version_id, path, content (text), size; unique (version_id, path) |
| `builds` | id, version_id, kind (`check`,`build`,`emit`,`import`), args, status, diagnostics (JSONB), artifacts (JSONB: name, sha256, size), log_tail, binary_version, started_at, finished_at |
| `device_catalog` | read from `devices/` manifests at startup; not a table in E2 |
| `guest_sandboxes` | id, created_at, expires_at, ip_hash, quota counters |
Main routes: `POST /v1/guest` (new sandbox, returns a guest token), `GET|POST /v1/projects`,
`POST /v1/projects/{id}/versions`, `GET /v1/versions/{id}`, `POST /v1/versions/{id}/builds`,
`GET /v1/builds/{id}`, `GET /v1/builds/{id}/artifacts/{name}`, `GET /v1/devices/catalog`,
`GET /v1/capabilities` (which `neuron` commands and toolchains exist in this image).
A new version is a snapshot of all files; the hash covers sorted paths and contents. A
project update carries `row_version`; a mismatch is `NE1107`.
**Compile runner.** The CLI gets `--format json` on `check`, `build` and `emit` (diagnostics
with code, severity, file, line, column, message; artifact list). The runner:
1. writes the version files to a fresh `tempfile` directory (mode 0700), rejecting paths
   with `..`, absolute paths, links and more than the file count or byte limits (`NE1101`);
2. runs `[neuron, <cmd>, ...]` with `asyncio.create_subprocess_exec` (an argument list,
   never a shell), a minimal environment (no `SYNAPSE_*`), `cwd` in the job directory,
   limits `RLIMIT_CPU`, `RLIMIT_AS`, `RLIMIT_FSIZE`, `RLIMIT_NOFILE`, a wall-clock timeout
   and output caps; on timeout it kills the process group (`NE1102`);
3. isolates the network with `unshare --net` when the container allows it. At startup a
   self-test reports `sandbox.network` as `isolated` or `unverified` in `/health` and
   `/v1/capabilities`. The `neuron` binary never opens sockets; a test asserts this by
   running a job under a socket-denying wrapper;
4. collects artifacts (size-capped), hashes them, deletes the directory in `finally`.
A semaphore bounds concurrent jobs (default 2); a full queue is `NE1103`. Builds are stored
with the binary version, so a result can be reproduced.
**Guests.** `POST /v1/guest` issues a guest token (role `guest`, claim `sbx`, expiry 24 h).
Quotas per sandbox: 5 projects, 50 files per version, 200 KiB per file, 100 builds per day.
An expired sandbox returns `NE1104`; a retention task deletes it with its data. Guests
get no route under `/v1/devices/*` other than the public catalog, and no device WebSocket
channel.
**Rate limiter.** A token bucket keyed by principal or IP (`limits.py`, injectable clock),
in memory (one instance). Separate buckets for API calls, compile jobs and guest creation.
Responses carry `Retry-After`.
**OpenAPI and SDK.** `tools/export-openapi.py` writes `spec/api/openapi.json` (sorted keys,
no timestamps). `packages/sdk-ts` is produced by `openapi-typescript` from it
(`pnpm gen`); CI runs both and fails on `git diff`.
## E3 Auth, RBAC, audit
Tables: `users` (id, email, password_hash, role, disabled, created_at), `service_tokens`
(id, role, cell_id, hash, created_at, revoked_at), `audit_log`.
- Passwords: argon2id (`argon2-cffi`), parameters in settings. Login is
  `POST /v1/auth/login`; no self-registration. The first engineer is created by a CLI
  command that reads the password from stdin.
- Access tokens: JWT HS256, 15 minutes, claims `sub`, `role`, `exp`, `iat`, `jti`. The key is
  `SYNAPSE_JWT_SECRET` (at least 32 bytes; startup fails in `production` if missing).
  No refresh token in v1: the client logs in again. Service tokens are random 256-bit
  values shown once; only their SHA-256 is stored.
- Permission matrix in `auth/permissions.py`, one dict `role -> frozenset[Permission]`:
  | Permission | guest | viewer | operator | engineer | daemon | twin |
  |---|---|---|---|---|---|---|
  | `sandbox.write`, `build.run` (own sandbox) | yes | | | | | |
  | `project.read`, `project.write`, `build.run` (own) | | read | yes | yes | | |
  | `device.read`, `telemetry.read` (real) | | yes | yes | yes | | |
  | `sim.control`, `estop.advise`, `device.command` (allowlisted) | | | yes | yes | | |
  | `deploy.request`, `deploy.confirm`, `pairing.issue`, `user.manage`, `audit.read` | | | confirm | yes | | |
  | `telemetry.publish`, `deployment.report` | | | | | yes | |
  | `sim.publish` | | | | | | yes |
  Viewer, operator and engineer form a chain (each a superset of the previous); the guest is
  separate and holds no device permission. Guests may read telemetry of virtual devices.
  The relay uses the same table plus the message table of `relay.md`; a test parses that
  markdown table and compares it with `ROLE_MESSAGES`.
- Audit: `audit_log(id, at, actor, role, action, target, request_id, result, detail JSONB)`.
  The application role has `INSERT` and `SELECT` only (migration `GRANT`). Details never
  hold tokens or passwords.
## E4 Relay, pairing, telemetry
**Relay (`relay/`).** `WS /v1/relay`. `validate.py` loads `spec/schemas/relay/message.schema.json`
(packaged into the image) and validates every inbound frame with `jsonschema`. The schema is
the single source of truth, so the server works on the validated dict through small typed
accessors and does not duplicate the models. A hub keeps `cell_id -> publishers` and
`subscriptions`. Per subscriber a bounded `asyncio.Queue` (default 256 frames); when full,
the oldest `telemetry` and `trace` frames are dropped, never control replies, snapshots or
replay framing. Rules:
- first frame must be `hello` within 10 s; `hello.token` is validated as in E3, absent token
  gives `guest`, invalid token gives `unauthenticated` (no fallback) and closes;
- `ping` every 15 s of idleness, close after 45 s of silence;
- a role may send only its row in the table; otherwise `forbidden`;
- `sim_*` and `replay_play` go only to the twin of the cell and only for devices with
  `simulated: true`; otherwise `unknown_target` or `not_ready`;
- `advisory_estop_request` is checked for `estop.advise`, written to the audit log and
  forwarded to the daemon of the cell. The answer is the daemon's `estop_state`; with no
  daemon connected, `estop_state` carries `unknown`. The relay never sets a state itself;
- control messages above 20 per second per session give `rate_limited`;
- the relay holds the last `cell_snapshot` of each cell and sends it after `subscribe`.
The hub is in memory. One instance is a stated limit: a deploy drops sockets and clients
reconnect (clients already must, since the protocol has no resume).
**Pairing.** `POST /v1/pairing` (engineer) returns a code of 8 base32 characters valid for
10 minutes, stored hashed. The daemon calls `POST /v1/pairing/claim` with the code and its
name, receives a service token (role `daemon`, bound to a cell) and registers rows in
`daemons` (id, name, cell_id, last_heartbeat, version, revoked_at) and `devices`
(daemon_id, name, kind, simulated=false). `DELETE /v1/daemons/{id}` revokes the token and
closes its sockets.
**Telemetry tables.**
| Table | Columns | Retention |
|---|---|---|
| `telemetry_samples` | cell_id, device, tag, t_ns, value (JSONB), quality | raw 7 days |
| `telemetry_1m` | cell_id, device, tag, minute, min, max, mean, count | 30 days |
| `blackbox_records` (E6) | id, cell_id, device, fault_code, at, record (JSONB) | 90 days |
Index `(cell_id, device, tag, t_ns)`. Ingest takes `telemetry` frames from daemons and twin
(real and virtual flagged by `simulated`), inserted in batches with
`executemany`. A retention task (started in the lifespan, guarded by a Postgres advisory lock)
deletes expired rows in chunks of 10 000 and rolls samples into `telemetry_1m` first.
`GET /v1/telemetry` takes cell, device, tag, `from_ns`, `to_ns`, `limit` (max 5000);
guests may query only virtual devices. A storage cap (`SYNAPSE_TELEMETRY_MAX_ROWS`) drops
new samples with a logged counter instead of filling the Neon quota.
## E5 Edge Daemon
A separate Python package: `synapse_daemon`, asyncio, started as `synapse-daemon run`.
Runs on a PC or the Raspberry Pi 5; works with no cloud (local mode serves the PWA from a
folder on `http://<lan>:8787` and relays on the LAN socket).
```python
class Driver(Protocol):
    name: str
    async def open(self, cfg: DriverConfig) -> None: ...
    async def read(self, tag: str) -> Sample: ...
    async def write(self, tag: str, value: Value) -> None: ...
    async def close(self) -> None: ...
```
| Driver | Library | Tested against |
|---|---|---|
| serial | `pyserial` | `loop://` and a paired pty or `socket://` echo |
| Modbus TCP and RTU | `pymodbus` | pymodbus server on localhost; RTU over a loopback serial pair |
| S7 | `python-snap7` | snap7 `Server` in process, or a mock when the library is not available |
| TCP sockets | `asyncio` streams | a local echo and framing server |
| MQTT | `aiomqtt` | an in-process broker (`amqtt`) or a mock client |
| CAN | `python-can` | virtual bus (`interface="virtual"`) |
| OPC UA (stretch) | `asyncua` | in-process `asyncua` server |
Drivers map the register and frame layouts the cell compiler (D3) emits; the link table of
a cell is the daemon's mapping source. Timeouts, retries and backoff are per driver and
configured; a failed driver sets its tags to `bad` quality, never to a guessed value.
**Flashing.** A `Flasher` wraps the same tools as `neuron flash` (avrdude, UF2 copy,
OpenOCD-style tools when present) as an argument list, with a per-device lock and a timeout.
**Signed packages.** A package is a zip with `manifest.json` (device, build id, files with
SHA-256, `created_at`, `min_daemon`), the artifacts, and `manifest.sig` (Ed25519 over the
canonical manifest bytes, via `cryptography`). The daemon trusts the public keys in its
config; the API signs with a key from `SYNAPSE_SIGNING_KEY` (never committed). Verification
failure is `NE1108` and nothing is written to disk outside a quarantine folder.
**Command allowlist.** Commands arrive as `{name, args}` and run only if `name` is in the
daemon's allowlist (`read_tag`, `write_tag` for tags marked writable, `flash`, `status`,
`reset_driver`); arguments are validated by Pydantic models per command. Anything else is
`NE1109`. There is no command that runs a shell, edits the allowlist or touches an E-STOP.
**Deployment state machine** (in the API, enforced again in the daemon):
`requested -> verified -> dry_run_ok -> confirmed_1 -> confirmed_2 -> applied`, plus
`rejected`, `failed`, `aborted`. `verified` needs a stored build of the version with its
diagnostics free of errors and the cell check passed. `dry_run_ok` needs the daemon to run
the package in simulation at reduced speed (no output drives). Each confirmation is a
separate authenticated call by a user with `deploy.confirm`, audited, with a fresh server
nonce; the second is accepted only after `dry_run_ok`. The default policy does not require
two different people (a lone lab user); `SYNAPSE_REQUIRE_DISTINCT_CONFIRMERS=true` does.
The first run after `applied` is at reduced speed until an operator lifts it. A guest never
reaches this machine.
**Heartbeat and E-STOP.** The daemon sends `deployment_status` with a heartbeat every
5 s; the API marks it offline after 15 s. The daemon reads the E-STOP circuit state from a
configured digital input (read-only) and publishes `estop_state`; when it cannot read the
input it publishes `unknown`. An `advisory_estop_request` makes the daemon put outputs it
controls to their declared safe state and report. The hardwired E-STOP stays the
authority; the UI text says so. This is a design aid, not a certified safety function.
## E6 Black box and calibration
**Black box.** The daemon keeps a ring buffer of the last N seconds (default 30 s) of tag
samples and trace events. A fault (`FAULT` trace, E-STOP `tripped`, driver loss, watchdog)
freezes a window of 10 s before and 5 s after. A record is JSON
(`spec/schemas/blackbox.schema.json`, generated from Pydantic models like the other
schemas): header (cell, device, fault code, `at_ns`, versions, package hash), `samples`
and `events`. It is written to a local SQLite file first, then uploaded
(`POST /v1/blackbox`); upload retries survive restarts. `blackbox_to_replay()` converts
a record to `replay.schema.json` so the twin plays it (F9).
**Calibration.** `synapse_daemon.calibrate` sends probe messages through a driver and
records round-trip times. The pipeline reduces samples to `transport` (p99.9 minus the
clock offset estimate), `jitter` (p99.9 minus median) and a sample count, and writes a
profile file in the manifest `[timing]` format:
```toml
[profile]
device = "generic_s7_1200"
medium = "modbus_tcp"
trust = "measured"          # "generic" for the numbers in the repo
samples = 2000
transport = "2 ms"          # example value; real values come from session I1
jitter = "1 ms"
measured_at = "2026-11-24"
tool = "synapse-daemon 0.1.0"
```
`neuron cell check --profile file.toml` (D-side flag added in E6.3) overrides the manifest
timing for that device and medium and labels the budget `measured`. A profile with fewer
than a minimum number of samples, non-positive values or an unknown unit is `NE1112`. Tests
use synthetic samples and generic numbers. Real values are added after I1; they change
`devices/**/*.toml` in a data-only commit.
## E7 Deployment
| Piece | State and plan |
|---|---|
| Neon | project `synapse-studio`, AWS US East 2, branch `production`. Add branch `dev` (human or MCP), used by CI and local development; `SYNAPSE_DATABASE_URL` differs per environment |
| Render | service `synapse-core`, Starter, Oregon (region TODO, Hell chose to leave it). `render.yaml` becomes `runtime: docker` with `dockerfilePath`, `preDeployCommand: alembic upgrade head`, health check `/health` |
| Vercel | https://synapsest.vercel.app, Root Directory `apps/web`, `NEXT_PUBLIC_API_URL`; the judge path URL depends on Martin's G6 |
| Secrets | `SYNAPSE_DATABASE_URL`, `SYNAPSE_JWT_SECRET`, `SYNAPSE_SIGNING_KEY` in the Render dashboard (`sync: false`) |
**Docker image** (multi-stage): stage 1 `rust:1.99` builds `neuron-cli --release` with a
cargo cache mount; stage 2 `python:3.12-slim` installs `uv`, the locked dependencies, the
binary, `devices/`, `targets/`, `lib/`, `spec/schemas/`, and optionally `gcc-avr` and
`gcc-arm-none-eabi` from apt; runs as a non-root user. `/v1/capabilities` states which
toolchains exist. A smaller image without cross compilers is the fallback if the build
time or size is a problem.
**Migrations.** Alembic runs in `preDeployCommand`; a failure stops the deploy. Every
migration is reversible except the audit grants; CI runs `upgrade head`, `downgrade base`,
`upgrade head` on a throwaway Postgres.
**Monitoring until 2027-01-29.** Render health check and an external uptime check on
`/health` and the judge path every 5 minutes (human sets the monitor); structured logs;
`tools/live-smoke.py` (health, guest, create project, build an example, relay hello and
snapshot) run by a scheduled GitHub Action daily and after each deploy; a runbook
`docs/runbook.md`. Spending: Render Starter fixed plan, Neon usage caps and alerts, set by
Hell (human action); the telemetry cap protects the Neon quota. The service is switched
to a cheaper plan or off after 2027-01-29 by decision of Hell.
## Tests
pytest with `httpx.AsyncClient` and `ASGITransport`; a Postgres service container in CI
(migrated by Alembic); WebSocket tests with Starlette's test client. Hypothesis properties:
permission monotonicity, rate bound, message round trip through the JSON Schemas (strategies
build every example type), deployment state machine over random event sequences, retention
deletes only expired rows, version hash independent of file order. Daemon tests use the
simulated peers of the driver table. CI sets `NEURON_BIN` to the built binary for the
compile runner tests.
## Risks
| Risk | Mitigation |
|---|---|
| Render container cannot create network namespaces | self-test and `unverified` status; the CLI opens no sockets; open question below |
| Docker build with Rust is slow on Render | cache mounts; prebuilt binary from CI as a fallback |
| Neon cold starts and Oregon to Ohio latency | pool with pre-ping, region fix noted in E7 |
| Single instance relay loses sockets on deploy | clients reconnect; documented |
| `python-snap7` or `pymodbus` API changes | versions pinned in `uv.lock`; driver tests catch it |
