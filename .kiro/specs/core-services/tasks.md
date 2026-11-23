# Tasks: services (Phase E)
Order: each task ends with tests green, a `docs/kiro-log.md` entry, the roadmap row, a
Conventional Commit with `-s`, and CI checked. One task per commit. Phase D is implemented
first; then E2, E3, E4, E5, E6, E7 in that order. "Needs hardware" is `no` when the task is
tested against simulated peers.
| Id | Task | Requirements | Needs hardware | Status |
|---|---|---|---|---|
| E2.1 | Service layout, settings, structured logging with redaction, `ErrorBody` and `NE11xx` handlers, SQLAlchemy async session, Alembic baseline, `docs/errors/services.md`; CI Postgres container | E-R1, E-R19 | no | done |
| E2.2 | `neuron check`, `build`, `emit --format json` (diagnostics and artifact list) with CLI tests | E-R2 | no | done |
| E2.3 | Compile runner: temp dir, path checks, limits, timeout and process-group kill, environment stripping, network self-test, `NE1101` `NE1102` | E-R2 | no | done |
| E2.4 | Projects, immutable versions and files (tables, routes, content hash, `row_version`, `NE1107`) | E-R1 | no | done |
| E2.5 | Builds API on the runner: queue and semaphore, stored diagnostics and artifacts with SHA-256, `GET /v1/capabilities` | E-R1, E-R2 | no | done |
| E2.6 | Device catalog from `devices/` manifests (read-only routes) | E-R1 | no | done |
| E2.7 | Token bucket limiter, per-client buckets, `Retry-After`, `NE1103`; property test of the rate bound | E-R4 | no | done |
| E2.8 | Guest mode: sandboxes, guest token, quotas, expiry cleanup, `NE1104`; a guest builds a corpus example | E-R3 | no | done |
| E2.9 | OpenAPI export, generated `packages/sdk-ts`, CI drift check | E-R5 | no | done |
| E2.10 | Docker image (Rust stage plus Python), `render.yaml` to `runtime: docker`, local `docker compose` for the API and Postgres | E-R2, E-R18 | no | open |
| E3.1 | Users, argon2 hashes, login, JWT access tokens, first-engineer CLI, `NE1105` | E-R6 | no | done |
| E3.2 | Permission matrix, route dependencies, role checks, `NE1106`; monotonicity and guest-isolation properties | E-R7, E-R3 | no | done |
| E3.3 | Service tokens (hashed, revocable) and their issue route | E-R6, E-R10 | no | done |
| E3.4 | Audit log table with `INSERT`/`SELECT` grants, audit writes on every sensitive action, read route for engineers | E-R8 | no | done |
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

## Results of E2.1
Built in `services/core`: `settings.py` (adds pool, log level and JSON switch),
`logging_config.py` (JSON lines, `request_id` context variable, a filter that redacts URL
credentials, JWTs, bearer headers, `key=value` pairs and fields whose name looks secret),
`middleware.py` (accepts a sane `X-Request-ID` or makes one, echoes it), `schemas/errors.py`
(`ErrorBody`), `errors.py` (`ErrorCode` NE1101..NE1112, `ServiceError`, handlers for service,
HTTP, validation and unhandled errors), `db/` (declarative base with a naming convention,
async engine cached per URL with `pool_pre_ping`, `get_session` dependency, `service_meta`
baseline table), Alembic (`alembic.ini`, `migrations/`, revision `0001`, URL only from the
settings or a test override, never from the ini). `GET /health` is unchanged (same model,
same blocking psycopg check in a thread). Docs: `docs/errors/services.md` with all twelve
codes, rows in chapter 10 (plus the `NE11xx` range), embedded in `explain.rs` with a test.
CI: Postgres 16 service container and `SYNAPSE_TEST_DATABASE_URL` for the core job.
Choices: validation errors use `NE1101` (request rejected) and list only locations and
messages, never input values; transport errors without a catalogue entry use `HTTP_<status>`
(the `ErrorBody.code` pattern allows both); 401/403/409/429 map to NE1105/6/7/3. The
catch-all 500 handler runs outside the request-id middleware, so its body has no request id.
Open: `pyright` is not configured (the spec does not ask for it); the Postgres tests
(migration round trip, async query) run in CI only. Tests: 23 pass, 2 skipped locally.

## Results of E2.2
`crates/neuron-cli/src/report.rs` (new), `--format human|json` for `check`, `build` and
`emit`, document layout in `docs/cli-json.md` (`"format": 1`: `ok`, `diagnostics` with code,
severity, message, file, line, column, help and notes, `messages`, `output_dir`, sorted
`artifacts` with path and size). Method: the binary shadows `eprint!`, `eprintln!`, `print!`
and `println!` with macros that write to the real streams as before, or to buffers in JSON
mode; after the command, the captured rendered diagnostics (the one layout all stages share:
analyzer, hardware, cell, emitters) are parsed into data, so the human and JSON modes
cannot disagree and no command needed rewriting. The exit code is returned unchanged;
`ok` is "no error diagnostic". Choices: SHA-256 is not computed by the CLI (the runner
hashes artifacts, E2.3/E2.5); stages that print no code (missing tool, usage) become
diagnostics with `code: null`; a usage error in JSON mode still prints a document.
Open: diagnostics are parsed from text, so a new stage that prints a different layout needs
its header recognised in `report::parse`; `import` has no `--format` yet (the spec asks for
check, build and emit only). Tests: 13 integration tests in `tests/e2_json.rs` (success,
errors with position, hardware error, native build artifacts, emit artifacts, policy
refusal, usage errors, human default unchanged, human/JSON agreement, determinism) and 5
unit tests of the parser; all other `neuron-cli` suites still pass; clippy `-D warnings` clean.

## Results of E2.3
Package `services/core/src/synapse_core/compile/`: `models.py` (`CompileJob` with typed
options, `CliDocument`, `CompileResult`, `CompileOutcome` with the artifact bytes),
`paths.py` (path and size checks, `NE1101`), `sandbox.py` (environment, rlimits, process
group kill, network self-test), `runner.py` (`CompileRunner`, `RunnerConfig`,
`runner_config(settings, network)`). Behaviour: files are validated (relative POSIX paths in
normal form, no `..`, no leading dot, no backslash, no case clashes, limits 200 files, 200
KiB each, 2 MiB total) and written into a fresh `mkdtemp` directory (0700, `O_EXCL`, mode
0600) with `src/` and `tmp/`; the argument list is built from typed fields (names must match
`[A-Za-z0-9_][A-Za-z0-9_.-]*`, so no option injection), always with `--format json`, run by
`create_subprocess_exec` (no shell) with `cwd` in `src/`; the environment holds only `PATH`,
the Windows system variables, a private `HOME`/`TMPDIR`, `LANG` and `NO_COLOR` (nothing
`SYNAPSE_*`); POSIX `RLIMIT_CPU/AS/FSIZE/NOFILE/CORE` and a new session per job; a wall-clock
timeout (30 s), an output cap (1 MiB) and a CPU/signal check end in `NE1102` after a
process-group kill (`taskkill /T` on Windows); output files are read without following
links, size-capped and hashed (`NE1101` over the caps); the directory is removed in
`finally`; the job directory name is replaced by `<job>` in all text. A program with errors,
a crash or non-JSON output is a result (`ok: false`), not an exception. A missing compiler
is a 503. Network self-test: `detect_network_sandbox` tries `unshare --net` (then with
`--map-root-user`), runs a probe inside and outside, and reports `isolated` only if the
namespace differs and only `lo` exists; otherwise `unverified` and no prefix. `/health`
gains `sandbox.network` (additive; the web app ignores it). New settings:
`SYNAPSE_NEURON_BIN`, `SYNAPSE_COMPILE_TIMEOUT_S`, `SYNAPSE_COMPILE_WORK_DIR`.
Tests: 55 new Python tests (fake `neuron` and fake `unshare` scripts; a Hypothesis property
that accepted paths never leave the directory); `docs/errors/services.md` text for
`NE1101` and `NE1102` updated to the real causes.
Verified and not verified: all tests pass on Windows (78 pass, 6 skip). The POSIX-only tests
(grandchild kill, CPU limit, link skipping) and the real-binary test (`NEURON_BIN`) are
skipped here and run only on Linux CI (the first three) or where `NEURON_BIN` is set;
Docker was not running, so they have not been run yet. Open: the "socket-denying wrapper"
test of the design needs the real binary and a Linux sandbox, so it belongs to E2.10 (image);
the concurrency semaphore and `NE1103` queue are E2.5; `import` is not offered by the runner
(the CLI has no `--format json` for it); `RLIMIT_AS` of 4 GiB is a guess to tune on Render.

## Results of E2.4
Migration `0002_projects` (reversible; a test compares the models with it through Alembic's
`compare_metadata`: no difference) adds `projects` (`row_version`, nullable `owner_id` and
`sandbox_id` without foreign keys until E3 and E2.8), `project_versions` (unique
`(project_id, number)`) and `version_files` (primary key `(version_id, path)`), cascading
deletes. Code: `db/models.py`, `schemas/projects.py`, `services/projects.py` (logic),
`api/projects.py` (routes only). Routes: `POST|GET /v1/projects`, `GET|PUT
/v1/projects/{id}`, `POST|GET /v1/projects/{id}/versions`, `GET /v1/versions/{id}`, with
`limit` (1..100) and `offset`. Choices: a version is the full file set, validated by the
compile runner's `check_files` (so `NE1101` and the same limits); the content hash is
SHA-256 over paths sorted by UTF-8 bytes with 8-byte length prefixes, independent of file
order; the project row is locked (`FOR UPDATE`) while the number is taken and the unique
constraint turns a lost race into `NE1107`; a rename is one `UPDATE ... WHERE row_version =
:seen`, so a stale write is `NE1107` and nothing else can win. Versions are immutable by
construction: no route writes them (PUT, PATCH, DELETE, POST on a version give 405, tested);
there is no database trigger. Not done: deleting a project, and any ownership or permission
check: the routes are open until E3.2 and the guest sandbox (E2.8) bind them.
Tests: 12 (3 Hypothesis properties of the hash, 9 route tests). The route tests need
`SYNAPSE_TEST_DATABASE_URL`; Docker is not running here, so they were run against SQLite
(`uv run --with aiosqlite`, schema from `create_all`) only as a local check. They have not
run against Postgres or through the migration: that happens in CI, where the fixture applies
the Alembic migrations. `tests/conftest.py` holds the fixtures.

## Results of E2.5
Migration `0003_builds` (reversible; the model/migration diff is empty on SQLite and the
offline SQL emits `JSONB` and `BYTEA` on Postgres) adds `builds` (status, args, diagnostics,
messages, artifacts as JSON -> JSONB, `log_tail`, `binary_version`, `exit_code`,
`error_code`, timestamps) and `build_artifacts` (`(build_id, name)` primary key, SHA-256,
size, bytes). Code: `services/builds.py` (`BuildManager`), `services/capabilities.py`,
`api/builds.py`, `api/deps.py`, `schemas/builds.py`, a `runner.validate` method and an
exported `EMIT_LANGUAGES`, two new settings (`compile_concurrency` 2, `compile_queue_max`
8). Routes: `POST /v1/versions/{id}/builds` (202), `GET /v1/versions/{id}/builds`,
`GET /v1/builds/{id}`, `GET /v1/builds/{id}/artifacts/{name}` (octet-stream, `X-Content-SHA256`,
`ETag`, `nosniff`), `GET /v1/capabilities`.
Behaviour: a build is validated with the runner (`NE1101` for bad options or files; nothing
is stored), persisted `queued`, then run in a background task. A single `BuildManager` per
process holds an `asyncio.Semaphore(concurrency)`; it admits `concurrency + queue_max`
in-flight builds and rejects the rest with `NE1103` and `Retry-After`. The result is stored
with the binary version so it is reproducible: `succeeded`/`failed` from the program,
`error` with `NE1101`/`NE1102` from the runner. The manager and the limiter are in memory
(one instance, per the design); on start-up the lifespan marks builds a stopped process
left `queued`/`running` as `error` (`interrupt_unfinished`). `/v1/capabilities` probes
`neuron --version` and `PATH` for `gcc`, `avr-gcc`, `arm-none-eabi-gcc`, and reports the
sandbox network status and the limits; it needs no database and is cached per process.
Choices: artifact bytes live in a table (simple, one Render instance, caps already bound
them) rather than object storage; downloads set no caching beyond the ETag. Open: no rate
limiter on the API verbs yet (E2.7), no guest quota on builds (E2.8), no auth (E3); artifact
storage may move off Postgres if sizes grow.
Tests: 13 in `tests/test_builds.py` (capabilities without a compiler/database, a 503 when
unconfigured, success with stored and downloadable artifacts, a program-error `failed`
build, an `NE1102` timeout, option rejection before storage, bounded concurrency with an
`NE1103` full queue and a drained-queue retry, restart marking). A gated fake runner proves
the peak concurrency. Like E2.4 they ran on SQLite locally; Postgres through Alembic runs
in CI.

## Results of E2.6
New package `services/core/src/synapse_core/devices/`: `models.py` (Pydantic v2 models that
mirror `crates/neuron-cell/src/manifest.rs`: `Kind`, `Backend`, `Trust`, `Family`,
`IoFamily`, `MediumTiming`, `RobotLimits`, `DeviceManifest`, `MediumManifest`, plus
`DeviceSummary`/`DeviceCatalog`/`DeviceList` for the routes), `catalog.py` (reads the TOML
with `tomllib`, flattens `[timing]` into `scan_min` plus a medium map exactly as the Rust
`RawTiming` does, validates name-matches-file-stem and class-matches-folder, caches per
directory), `api/devices.py`. Routes (read-only, no database): `GET /v1/devices/catalog`
(every device and medium), `GET /v1/devices` (summaries), `GET /v1/devices/{name}`.
The manifest directory comes from a new setting `devices_dir`; its default is the repo
`devices/` resolved relative to the package (`parents[5]`), so the service finds it
whatever the working directory, and an env var overrides it. **The E2.10 Docker image must
copy `devices/` into the image**: Render runs this service with `rootDir: services/core`
(confirmed in `render.yaml`), which does not contain `devices/`; the design already lists
`devices/` among the files stage 2 copies.
Choices: durations and bit rates stay as the manifest strings (`"1 ms"`); this catalog is
descriptive and the Rust side owns the authoritative numeric parsing for `neuron cell
check`, so the two cannot disagree on a number. Models use `extra="forbid"`, matching the
Rust `deny_unknown_fields`. The catalog is public (it has no secrets); E-R3 keeps every
other `/v1/devices/*` route from guests, enforced with the permission matrix in E3.2.
Not done: cross-checks the Rust side also makes (backend fits the class, dialects known,
units valid) are left to `neuron cell check`; the catalog loads what the repo ships, which
the Rust `neuron-cell` tests already validate.
Tests: 20 in `tests/test_devices.py`, including a parametrized case per file under
`devices/` (`test_every_device_manifest_loads`, `test_every_medium_manifest_loads`) so
every shipped manifest is asserted to load, the catalog count and route tests, a
`devices_dir` override, and rejection of a missing `[safety]`, a name/stem mismatch and an
unknown field. These need no database and run everywhere.

## Results of E2.7
Built `services/core/src/synapse_core/limits.py` (pure token-bucket core, no FastAPI import):
`BucketConfig` (capacity = burst, rate = refill/s), `_Bucket`, `Decision` (with a
`retry_after_header` rounded up to at least 1 s), `RateLimiter` keyed by `(scope, client)`
with an injectable clock and a lock, `LimiterSettings` (three scopes: `api`, `compile`,
`guest`), `EXEMPT_PATHS`, `client_key(principal, ip)` and `scope_for(method, path)`.
`ratelimit.py` holds `RateLimitMiddleware` (ASGI middleware, the design allows middleware or
a dependency; middleware keeps routes clean); it reads the limiter from
`app.state.rate_limiter` on each request (one instance per process; tests swap it), refuses
an empty bucket with `NE1103` and a `Retry-After` header, and never reaches the handler.
`settings.py` gains six scalar `ratelimit_*` fields and a `limiter` property. `main.py`
builds the limiter and adds the middleware inside the request-id middleware (so a refusal
still carries a request id) and outside CORS.
Choices: `/health` and the schema paths (`/openapi.json`, `/docs`, `/redoc`) are exempt;
`POST /v1/guest` draws from the `guest` scope and `POST .../builds` from `compile`, so a
flood of cheap reads cannot drain either; everything else is `api`. Keying is by principal
when E3 sets `request.state.principal_id`, else by client IP. The in-memory build queue of
E2.5 still raises its own `NE1103` when full; that is orthogonal to the per-client rate.
Open: the per-session WebSocket control-message rate of E-R4 lands with the relay (E4.2);
real principals arrive with E3. Tests: 9 in `tests/test_ratelimit.py`, including a
Hypothesis property that the admitted count over any window never exceeds
`capacity + rate * elapsed` (full suite: 110 pass, 21 skip; ruff clean).

## Results of E2.8
Guest mode without login. Model `GuestSandbox` (`id`, `created_at`, `expires_at`, `ip_hash`
nullable, `project_count`, `builds_today`, `build_day`) and migration `0004_guests`
(reversible; upgrade/downgrade/upgrade verified on SQLite). `auth/tokens.py`: a compact
`payload.signature` guest token (URL-safe base64 of canonical JSON, HMAC-SHA256 over the
payload), `mint_guest_token`/`verify_guest_token` checking signature in constant time then
expiry and the `guest` role; no third-party JWT dependency (E3 folds this into full JWTs on
the same `SYNAPSE_JWT_SECRET`). `services/guests.py`: `create_sandbox`, `principal_from_token`
(`NE1104` on a bad/expired token or a missing sandbox), `reserve_project`/`reserve_build`
(row-locked quota check and increment in the caller's transaction), `cleanup_expired`
(deletes expired sandboxes; their projects cascade to versions, files, builds, artifacts),
`hash_ip` (salted one-way). Route `POST /v1/guest` (`api/guests.py`). Dependency
`optional_guest` in `api/deps.py` reads `Authorization: Bearer`, resolves a live sandbox and
records `request.state.principal_id` so the rate limiter keys on the guest. Projects and
builds bind to the sandbox and enforce quotas: `create_guest_project`, guest file limits on
version creation (`guest_max_files`, `guest_max_file_bytes`), a per-day build reservation
passed into `BuildManager.submit`. New settings: `jwt_secret`, `guest_ip_salt`,
`guest_token_ttl_s`, `guest_max_projects` (5), `guest_max_files` (50), `guest_max_file_bytes`
(200 KiB), `guest_max_builds_per_day` (100), `guest_cleanup_interval_s`; `Settings.token_secret()`
requires `SYNAPSE_JWT_SECRET` in production and derives a per-process dev key otherwise. The
lifespan starts a periodic cleanup task.
Choices: the token is stateless so recognising a guest needs no read, but the sandbox row is
still loaded to enforce quotas and expiry, so a token outliving its deleted sandbox is
`NE1104`. Guests never reach a device route: the guest principal holds no device permission
and the device routes stay closed to it (the full matrix lands in E3.2). Quotas are the
design's numbers. Open: read-scoping a guest to only its own projects, and keying device
denial to the matrix, are E3.2; WebSocket guest limits are E4. Tests: 10 in
`tests/test_guests.py` (token mint/verify, tamper, wrong key, expiry, one-way IP hash;
route; an end-to-end guest build of `h01_gateway_counter.neuron` through the fake runner;
project quota `NE1104`; invalid/missing sandbox `NE1104`; expiry cleanup). DB tests ran on
SQLite locally (30 across guests/builds/projects) and run on Postgres through Alembic in CI;
full suite 114 pass, 26 skip locally; ruff clean.

## Results of E2.9
`tools/export-openapi.py` exports the FastAPI document to `spec/api/openapi.json` (canonical:
sorted keys, two-space indent, `info.version` pinned to `1.0.0` so it never drifts with the
package version) and generates `packages/sdk-ts/src/schema.ts` from the same document. The
generator is pure Python (stdlib only), so CI needs no Node and makes no network call. It
maps the OpenAPI 3.1 schema dialect to TypeScript (`$ref`, `anyOf`/`oneOf` unions, `allOf`
intersections, `const`/`enum` literals, arrays, objects with required/optional fields,
`additionalProperties` to `Record`), emitting an `export interface` per object schema and an
`export type` per alias, plus an `ApiPaths` map of route to allowed methods. `ErrorBody` is
produced by the exception handlers, not declared on a route, so FastAPI omits it from the
components; the script adds it from `ErrorBody.model_json_schema()` (still generated, not
hand-written) so clients can type errors. `--check` regenerates in memory and compares,
failing on any difference. `packages/sdk-ts` holds `package.json` (`gen`, `gen:check`,
`typecheck` scripts), `tsconfig.json` (strict), `README.md`, `.gitignore`, the generated
`src/schema.ts`, and a thin hand-written `src/index.ts` (`SynapseClient`, `ApiError`,
`startGuestSession`) that re-exports the generated types; `src/index.ts` is the only
hand-written code and the types it uses all come from `schema.ts`. CI (`core` job) runs
`uv run python ../../tools/export-openapi.py --check` after the tests, so a model change that
is not regenerated fails the build (E-R5). Choices: a Python generator rather than
`openapi-typescript` (the design named it) keeps CI Node-free and offline, as the user asked,
while still generating the types and diffing on drift; the richer per-operation request/
response typing is left to a later pass. Verified: `--check` passes on the committed files
and fails (exit 1) on a deliberate edit; `tsc --noEmit` compiles `schema.ts` and `index.ts`
cleanly under strict mode. No new tests (the drift check is the test); core suite unchanged
(114 pass, 26 skip).

## Results of E3.1
Authentication, kept own and minimal as decision 2 states. New: `auth/passwords.py`
(argon2id via `argon2-cffi`; cost from settings), JWT helpers added to `auth/tokens.py`
(`mint_access_token`/`verify_access_token`, HS256 on `SYNAPSE_JWT_SECRET`, claims `sub`,
`role`, `exp`, `iat`, `jti`, required on decode), `services/users.py` (`create_user`,
`authenticate`, `count_users`; `USER_ROLES = {viewer, operator, engineer}`),
`schemas/auth.py`, `api/auth.py` (`POST /v1/auth/login`), `cli.py` with the console script
`synapse-core create-engineer --email ...` (password from stdin, never an argument),
model `User` (unique email, argon2 hash, role, `disabled`) and migration `0005_users`
(continues the E2 numbering). Deps added to `services/core`: `argon2-cffi`, `pyjwt`.
Choices: login is opaque (`NE1105` with one message for unknown user, wrong password and
disabled account) and always runs a dummy argon2 verify when the user is absent, so timing
does not leak whether an email exists; the access-token TTL is 15 minutes
(`access_token_ttl_s`, 60..3600); emails are normalised to lower case and compared
case-insensitively; the bootstrap command refuses once any user exists, so it only ever
creates the first engineer and is not a back door around the engineer-only creation route
(E3.2); `EmailStr` is not used (would add `email-validator`), a length-bounded `str` is
enough. Secrets and tokens are never logged (a test asserts the token and password are
absent from DEBUG logs). The guest token of E2.8 is unchanged and still signed with the
same `token_secret()`; E3 does not fold it into a JWT (the compact HMAC form stays, both
share the secret, which is what the E2.8 note promised).
Open: route-level role enforcement and the permission matrix are E3.2 (login and the
bootstrap CLI are the only auth surfaces so far; project/build routes stay open until
E3.2 binds them); no refresh token by design (the client logs in again); service tokens
are E3.3; the audit entry for a login is written in E3.4.
Tests: 16 in `tests/test_auth.py` (argon2 round-trip and garbage-hash rejection; JWT
mint/verify, wrong key, tamper, expiry, missing-claim rejection; login success, email
case-insensitivity, wrong password and unknown user both `NE1105`, disabled user `NE1105`,
duplicate email refused, token/password absent from logs; CLI password-from-pipe and
subcommand-required; a DB bootstrap that creates the first engineer then refuses a second).
Full suite 151 pass, 4 skipped locally (the Postgres-only `test_migration_round_trip`
is deselected locally and runs in CI); OpenAPI/SDK regenerated for the login route and the
drift check passes; ruff clean. No new error code (`NE1105` already documented in E2.1).

## Results of E3.2
Permission matrix and route-level RBAC. New: `auth/permissions.py` (`Permission` enum;
`ROLE_PERMISSIONS` built so the user chain accumulates viewer -> operator -> engineer;
`_GUEST` = `{sandbox.write, build.run}`; service roles `daemon`, `twin`; `DEVICE_PERMISSIONS`
set for the isolation property; `has_permission`). `api/deps.py` reworked: a unified
`Principal = AuthenticatedUser | GuestPrincipal`, `optional_principal` (three-part token =
user JWT -> `NE1105`; two-part = guest token -> `NE1104`; no token = no DB access, so public
routes and denied anonymous requests need no database), `require_permission` and
`require_any_permission` dependency factories (anonymous or missing permission is `NE1106`).
`services/users.py` gained `principal_from_access_token` (reads the role from the DB row, not
the token, so a role change or a disable takes effect on the next request). Routes: project
and build routes now carry permission dependencies and are scoped for guests in the service
layer; `services/projects.py` threads a `GuestPrincipal` through `get_project`/`list`/
`rename`/`create_version`/`get_version`/`list_versions` so a guest sees only its own
sandbox's projects (a project of another sandbox or of a user answers 404, not 403, so its
existence does not leak); `api/builds.py` checks the version is visible to the guest before
build create/list/read/artifact. Device routes `GET /v1/devices` and `GET /v1/devices/{name}`
require `device.read` (a guest is `NE1106`); `/v1/devices/catalog` stays public, the single
exception the E2.6 design carved out; `/v1/capabilities` stays public.
Choices and honesty on the monotonicity wording: the spec's correctness criteria are the
authoritative statement, so the property tests check (a) the user chain viewer ⊂ operator ⊂
engineer (strict supersets) and (b) guest isolation (the guest holds none of
`DEVICE_PERMISSIONS`). The literal reading "engineer ⊇ ... ⊇ guest" does NOT hold and should
not: a guest holds `sandbox.write` that no user role has, and viewer holds `project.read`/
`device.read`/`telemetry.read` that the guest must not have. The guest is a separate role
scoped to its own sandbox, exactly as the design says. The relay role table
(`ROLE_MESSAGES`) comparison against `relay.md` is E4.1, not here. Login still writes no
audit entry (E3.4). Service tokens are E3.3.
Tests: `tests/test_permissions.py` (6, two Hypothesis properties: monotonic-over-the-chain,
guest-denied-every-device-permission) and `tests/test_rbac_routes.py` (7 DB tests: anonymous
denied, viewer reads not writes, operator/engineer write, guest denied device routes but
catalog public, guest cannot see another sandbox's or a user's project, a disabled user's
token stops working). Existing E2.4/E2.5 route tests now authenticate through a new
`engineer_client` conftest fixture; the E2.8 guest build test passes the guest token to its
build poll. Full suite 163 pass, 4 skipped locally; ruff clean; OpenAPI/SDK regenerated and
the drift check passes. No new error code (`NE1106` already documented in E2.1).

## Results of E3.3
Service tokens for the `daemon` and `twin` principals. New: model `ServiceToken`
(`role`, `cell_id`, `token_hash` unique, `label`, `created_at`, `revoked_at`) and migration
`0006_service_tokens`; `services/service_tokens.py` (`issue_service_token` returns the row
and the plaintext once; `principal_from_service_token`; `revoke_service_token`;
`list_service_tokens`; `hash_token`; `looks_like_service_token`; `ServicePrincipal`);
`schemas/service_tokens.py`; `api/service_tokens.py` with `POST /v1/service-tokens`,
`GET /v1/service-tokens` and `DELETE /v1/service-tokens/{id}`, all gated by
`require_permission(user.manage)` (engineer only). The principal resolver now recognises a
third token kind: a service token starts with `svc_` and has no dot, so it is told apart
from a user JWT (three dots) and a guest token (one dot); `Principal` is now
`AuthenticatedUser | ServicePrincipal | GuestPrincipal`.
Choices: the token is `svc_` + `secrets.token_urlsafe(32)` (256 bits); only its SHA-256 is
stored, so a database read cannot recover a usable token; resolution is a lookup by the
stored hash (never a plaintext comparison in SQL). A daemon token must carry a `cell_id`
(bound to one cell); a twin token must not. A revoked or unknown token is `NE1105` (the same
opaque code as a bad login); the engineer-only issue route is `user.manage`, so a daemon or
twin token cannot mint more tokens. Revoke is idempotent at the service layer (`False` the
second time) and the route maps that to a 404; a bad body (daemon without a cell) is `NE1101`
from the route before anything is stored. The pairing code flow that hands a daemon one of
these tokens is E4.3; this task builds the primitive and the engineer-only route.
Tests: 7 in `tests/test_service_tokens.py` (hash shape and the `svc_` discriminator; issue
stores only the hash and verifies; wrong and revoked tokens rejected `NE1105`, idempotent
revoke; role/cell binding rules; engineer-only issue denied to viewer/operator/anonymous
`NE1106`; engineer issues/lists/revokes and the issued token then authenticates and stops
working after revoke; a daemon token without a cell is `NE1101`). Full suite 170 pass, 4
skipped locally; ruff clean; OpenAPI/SDK regenerated for the three routes, drift check green.
No new error code.

## Results of E3.4
Append-only audit log. New: model `AuditLog` (`at`, `actor`, `role`, `action`, `target`,
`request_id`, `result`, `detail` JSON->JSONB) and migration `0007_audit_log` which, on
Postgres, runs `REVOKE UPDATE, DELETE, TRUNCATE` and `GRANT INSERT, SELECT` on the table
(the one non-reversed step the design allows); `services/audit.py` (`AuditAction` enum;
`record` writes one row in its own session and never raises, so an audit failure cannot
break the audited action; `query` with action/actor filters; `_scrub` replaces any
secret-looking field before storage); `schemas/audit.py`; `api/audit.py` with `GET /v1/audit`
gated by `require_permission(audit.read)` (engineer only). Audit writes were added to: login
(ok and denied, target = the email, no password in the detail), service-token issue and
revoke, project create, version create, build create. Dependencies `SessionMakerDep` and
`principal_identity` were added to `api/deps.py` so a route can write audit rows outside its
request transaction and label them with the caller's key and role. `request_id` is read from
the request context so a row correlates with the structured logs.
Honesty on the grant: the REVOKE/GRANT is applied, and no application code path issues an
UPDATE or DELETE on `audit_log` (there is no such route or service call), so the log is
append-only at the application layer. But Postgres lets a table *owner* bypass its own
privilege grants, and in this single-service deployment the app connects as the owner, so
the grant alone does not stop the owner; truly enforcing it at the database needs a separate
non-owner application role, which is an E7 deployment concern. The code-level guarantee and
the grant together are what E3.4 ships; this limit is recorded here rather than hidden.
`role change` is in the `AuditAction` enum (`user.role_change`) ready for the user-management
route that a later task adds; the actions with a route today all write their row.
Tests: 10 in `tests/test_audit.py` (`_scrub`; login ok+denied audited with no password in
the detail; project/version/build creation audited with targets and detail; service-token
issue+revoke audited with no secret in the detail; engineer-only read, viewer and anonymous
`NE1106`; no POST/PUT/PATCH/DELETE route on `/v1/audit`; actor filter and `X-Request-ID`
correlation). Full suite 180 pass, 4 skipped locally; ruff clean; OpenAPI/SDK regenerated for
the read route, drift check green. No new error code. This completes the E3 row.
