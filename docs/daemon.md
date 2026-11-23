# Edge Daemon (`services/edge-daemon`)

The Edge Daemon connects SYNAPSE Studio to field devices. It speaks the Neuron Wire relay
protocol (`spec/protocols/relay.md`) to the Core API, exposes field data as telemetry and
trace, applies signed deployment packages, and serves the web PWA on the LAN. It runs on a
PC or a Raspberry Pi 5 and works with no cloud.

## Safety (read first)

- The software E-STOP in this product is **supplementary**. A **hardwired E-STOP is the
  authority**. No message, API call or command releases, bypasses or disables the hardwired
  E-STOP or any other safety function.
- The daemon only **reads** the E-STOP circuit state from a configured digital input and
  reports it as `estop_state` (`released`, `tripped`, or `unknown` when it cannot read the
  input). It never decides or invents a state.
- An `advisory_estop_request` is **advisory only**: it asks the daemon to put the outputs it
  controls into their declared safe state and to report the state it reads. It does not trip
  or release the hardwired E-STOP.
- SYNAPSE Studio is a **design aid**. It does **not** claim IEC 61508 or ISO 13849
  compliance and must not be relied on as a certified safety function.

## Running

```
uv sync
uv run synapse-daemon run
```

Configuration comes from environment variables prefixed `SYNAPSE_DAEMON_` (never hard-coded;
secrets such as the service token come from the environment or a file the operator controls):

| Variable | Meaning |
|---|---|
| `SYNAPSE_DAEMON_NAME` | daemon name shown in the cell |
| `SYNAPSE_DAEMON_CELL_ID` | cell the daemon serves (set at pairing) |
| `SYNAPSE_DAEMON_RELAY_URL` | `wss://host/v1/relay`; unset ⇒ local LAN mode |
| `SYNAPSE_DAEMON_API_BASE_URL` | `https://host` for pairing and uploads |
| `SYNAPSE_DAEMON_SERVICE_TOKEN` | daemon service token from pairing (secret) |
| `SYNAPSE_DAEMON_LAN_HOST` / `_LAN_PORT` | LAN server bind (default `0.0.0.0:8787`) |
| `SYNAPSE_DAEMON_PWA_DIR` | folder of the built PWA to serve |
| `SYNAPSE_DAEMON_TRUSTED_KEYS` | base64 Ed25519 public keys for signed packages |
| `SYNAPSE_DAEMON_ESTOP_INPUT` | the digital input the daemon reads the E-STOP from |

## Local LAN mode and offline operation

When no `RELAY_URL` and service token are configured the daemon runs **local LAN mode**: it
serves the PWA and `/health` on `http://<lan>:8787` with no cloud and no network beyond the
LAN. `/health` reports `{"status":"ok","mode":"local","offline":true}`. This is how a judge
or operator reaches the UI with nothing but the daemon running. The static server rejects
path traversal and falls back to `index.html` for single-page-app routes.

## Pairing

An engineer issues a one-time code in the Core API (`POST /v1/pairing`, E4.3). The daemon
claims it (`POST /v1/pairing/claim`) and receives a `daemon` service token bound to one cell.
The token is shown once and stored by the operator; it is never logged. Unpairing in the API
revokes the token and drops the daemon's relay sockets.

## Drivers

Each driver maps the register and frame layout the cell compiler (D3) emits. A failed read
sets the tag to `bad` quality; a driver never returns a guessed value. Every driver is tested
against a simulated peer, never real hardware.

| Driver | Library | Tested against |
|---|---|---|
| serial | `pyserial` | `loop://` loopback |
| Modbus TCP / RTU | `pymodbus` | in-process server; POSIX pty pair for RTU |
| S7 | `python-snap7` | in-process snap7 server or a mock |
| TCP sockets | asyncio streams | a local echo server |
| MQTT | `aiomqtt` | an in-process mock broker |
| CAN | `python-can` | the virtual bus |

## Flashing

A `Flasher` wraps the same tools as `neuron flash` (avrdude, UF2 copy) as an argument list
(never a shell), with a per-device lock and a timeout. A non-zero exit, a timeout or a
missing tool is a failure.

## Signed packages and command allowlist

A deployment package is a zip with `manifest.json`, the artifacts and `manifest.sig` (Ed25519
over the canonical manifest). The daemon applies a package only when the signature matches a
trusted key and every file matches its SHA-256; any failure is **`NE1108`** and the package is
kept only in a quarantine folder. Commands arrive as `{name, args}` and run only if the name
is in the allowlist (`read_tag`, `write_tag` for writable tags, `flash`, `status`,
`reset_driver`) with valid arguments; anything else is **`NE1109`**. A command for an offline
or unpaired device is **`NE1111`**. There is no command that runs a shell, edits the allowlist
or touches an E-STOP.

## Deployment gate

A deployment reaches `applied` only through `requested → verified → dry_run_ok → confirmed_1
→ confirmed_2 → applied`, in that order, each with its precondition, plus a valid package
signature. Any out-of-order step is **`NE1110`**. The state machine is enforced in the Core
API and again on the daemon.

## Heartbeat and E-STOP

The daemon sends `deployment_status` for its cell every 5 seconds (the API marks it offline
after 15 seconds). It reads the E-STOP input and publishes `estop_state`. On relay loss, a
`tripped` read, or an advisory request, it drives the outputs it controls to their declared
safe state. See the safety section above.

## Black box

The daemon keeps a ring buffer of the last `SYNAPSE_DAEMON_BLACKBOX_WINDOW_S` seconds
(default 30) of tag samples and trace events. A fault freezes a window of
`SYNAPSE_DAEMON_BLACKBOX_PRE_S` seconds before and `SYNAPSE_DAEMON_BLACKBOX_POST_S` seconds
after (default 10 and 5) into a record: a header (cell, device, fault kind and code, instant,
versions, package hash) and the samples and events of the window. The record is validated
against `spec/schemas/blackbox.schema.json` and written to a local SQLite file
(`SYNAPSE_DAEMON_BLACKBOX_DB`, default `<work>/blackbox.db`) first, so it survives a restart
and a cloud outage. It is then uploaded to the Core API (`POST /v1/blackbox`) with the daemon
service token; a record is marked uploaded only on success, so a crash or an outage leaves it
pending and the daemon retries it on the next flush, including after a restart. The upload is
idempotent, so a retry never duplicates a record. A record converts to the twin replay format
(`spec/schemas/replay.schema.json`) so the twin can play the window back. A black box is a
diagnostic record only; it reads and decides no safety state.

## Calibration

The calibration pipeline turns measured round trips into a timing profile for the device
manifests. It reduces the samples to `transport` (the p99.9 round trip minus the clock-offset
estimate) and `jitter` (p99.9 minus the median) and writes a profile file in the manifest
`[timing]` shape (`[profile]` table with `device`, `medium`, `trust`, `samples`, `transport`,
`jitter`). `neuron cell check --profile file.toml` applies it (see `docs/cells.md`). An
invalid profile is `NE1112`. The numbers are generic placeholders until the lab session (I1)
measures real devices; a real profile is a data-only commit to `devices/**`.
