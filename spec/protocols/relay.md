# Relay and twin gateway protocol (v1)

Contract between the relay / twin gateway and its clients: web IDE, desktop, Android,
the 3D twin and the Edge Daemon. Implementation of the types: `crates/synapse-protocol`.
The relay server itself is a later task (roadmap E4 implementation).

**Source of truth.** The Rust types generate the JSON Schemas in `spec/schemas/relay/`:

| File | Root |
|---|---|
| `message.schema.json` | one protocol message (`Message`, tagged by `type`) |
| `replay.schema.json` | a replay file (`ReplayFile`) |

TypeScript types for the web and Android clients are generated from these files later
(CI fails on drift, as for NIR). Do not hand-write them.

Regenerate after changing a type: `cargo run -p synapse-protocol --example export_schema`.
A test fails when the committed schemas differ from the types.

## Safety rule

The hardwired E-STOP is the authority. No message in this protocol trips, releases,
bypasses or disables an E-STOP or any other safety function. The only E-STOP message a
client can send is `advisory_estop_request`. It asks for a stop, and the relay forwards it
to the daemon. The answer is always an `estop_state` carrying the state **reported by the
daemon** (`released`, `tripped` or `unknown` when the daemon is offline). A client must
never show "stopped" because it sent a request, only because it received `tripped`.
This protocol is a design aid and does not claim IEC 61508 or ISO 13849 compliance.

## Transport

- WebSocket, JSON text frames, one message per frame, UTF-8. Binary frames are out of
  scope for v1 and are rejected with `bad_message`.
- Every message is an object with `"v": 1` and `"type": "<name>"`, plus the fields of
  that type. Field names are `snake_case`.
- Times are integer nanoseconds (`*_ns`) since the **cell epoch** (simulated or
  daemon-aligned time of the cell). Exceptions: `welcome.server_time_ns` and the
  `ping`/`pong` clocks are the sender's own clock, so clients can estimate offsets.
- Values (`Sample.value`, `sim_set_input.value`) are JSON numbers, booleans or strings.
  The unit is declared once per tag in the cell description (`TagInfo.unit`), never in
  the value.
- Optional fields are omitted when absent. Receivers ignore unknown fields (additive
  evolution).

## Connection lifecycle

1. Client opens the WebSocket and sends `hello` as the first frame (within 10 s).
2. Server answers `welcome` (session id, granted roles) or `error` with
   `unauthenticated` / `unsupported_version` and closes.
3. Client sends `subscribe` for a cell (optionally one device) and the channels it wants.
   The server replies with `ack`, then a `cell_snapshot` and the streams.
4. Both sides send `ping` at least every 15 s of idleness; the peer answers `pong`.
   After 45 s without any frame the connection is closed.
5. `unsubscribe` ends a subscription. Closing the socket ends the session.

Fatal errors (`unsupported_version`, `unauthenticated`, `bad_message` before `welcome`)
are followed by closing the socket. Others keep the session open.

## Authentication (placeholder)

`hello.token` carries a bearer token from the Core API. The token format and validation
are defined with the Core API (Phase E). Rules fixed now:

- No token: the session gets the `guest` role. This is the judges' virtual lab, with no
  login. Guests are read-only.
- Invalid or expired token: `error` code `unauthenticated`. The server does not fall back
  to guest.
- The Edge Daemon and the twin use service tokens and are granted `daemon` and `twin`.
- Tokens are never logged and never echoed in any message.

## Roles and what each may send

| Role | May send |
|---|---|
| `guest`, `viewer` | `hello`, `ping`, `pong`, `subscribe`, `unsubscribe` |
| `operator` | the above, plus `sim_start`, `sim_pause`, `sim_step`, `sim_set_input`, `sim_cut_link`, `sim_speed`, `replay_play`, `advisory_estop_request` |
| `twin` | the common ones, plus `cell_snapshot`, `telemetry`, `trace`, `sim_state`, `replay_header`, `replay_events`, `replay_end`, `ack`, `error` |
| `daemon` | the common ones, plus `telemetry`, `trace`, `deployment_status`, `estop_state`, `ack`, `error` |

The server sends `welcome`, `error`, `ack`, `pong`, `cell_snapshot` and forwards
publisher messages to subscribers. A message a role may not send is answered with
`error` code `forbidden`. `Role::may_send` in the crate implements this table.

The twin is the authority for **virtual** devices only. Simulation control messages are
meaningful only for devices with `simulated: true`; the server never forwards them to
real hardware.

## Message list

| `type` | Direction | Purpose |
|---|---|---|
| `hello` | client → server | open a session: client kind, protocol, capabilities, token |
| `welcome` | server → client | session id, server time, roles, negotiated protocol |
| `error` | server → client | code, message, optional request id |
| `ack` | server → client | confirms a request that has no other reply |
| `ping` / `pong` | both | liveness and clock offset |
| `subscribe` / `unsubscribe` | client → server | cell or device, channels |
| `cell_snapshot` | server → client | devices, links with budgets, tags, zones |
| `telemetry` | publisher → subscribers | batched samples (`id`, `t_ns`, `value`, `quality`) |
| `trace` | publisher → subscribers | batched trace events with the device |
| `sim_start`, `sim_pause`, `sim_step`, `sim_set_input`, `sim_cut_link`, `sim_speed` | operator → twin | simulation control |
| `sim_state` | twin → subscribers | running, simulated time, speed |
| `replay_play` | operator → twin | play a replay file |
| `replay_header`, `replay_events`, `replay_end` | twin → subscribers | replay stream |
| `deployment_status` | daemon → subscribers | device online/offline, build id, last heartbeat (read-only) |
| `advisory_estop_request` | client → server → daemon | advisory stop request (see the safety rule) |
| `estop_state` | daemon → subscribers | reported E-STOP circuit state; also the answer to a request |

Examples: one file per type in `spec/protocols/examples/<type>.json` (and
`replay_file.json`). A test parses each into the Rust type and checks that it
re-serializes to the same JSON.

Quality values: `good`, `stale` (older than the tag's update period), `bad`.

Trace kinds are the interpreter's: `OUT`, `SIG`, `FAULT`, `RESET`, `EVENT`, `STATE`,
`LOG`, `MOVE` (a test checks them against `neuron-interp`).

Requests that expect an answer carry an optional `id`. The server echoes it in `ack`,
`error` or the specific reply (`estop_state`).

## Ordering and backpressure

- Frames of one sender arrive in order. Within a stream, `t_ns` is non-decreasing per
  device.
- Publishers batch telemetry and trace (suggested: at most one frame per 50 ms per cell,
  at most 500 samples per frame).
- Under load the server may **drop telemetry and trace frames** for a slow subscriber
  (oldest first) and may merge batches. It never drops control replies (`welcome`,
  `error`, `ack`, `pong`, `sim_state`, `estop_state`), `cell_snapshot` or replay framing
  (`replay_header`, `replay_end`).
- A client that cannot keep up may be closed with `rate_limited`. Clients should send
  fewer than 20 control messages per second.
- Telemetry is a view; consumers must not treat a gap as a value.

## Clock offset

Each side keeps its own clock. To estimate the offset to the server:

1. Client sends `ping {id, t_ns: t0}` and notes the arrival time `t1` of the `pong`.
2. The server answers `pong {id, t_ping_ns: t0, t_ns: ts}`.
3. `rtt = t1 - t0`, `offset = ts - (t0 + t1) / 2` (server minus client). Keep the sample
   with the smallest `rtt` of the last 8.

`welcome.server_time_ns` gives a first estimate before the first ping.

## Versioning

- `v` is the message-format version and `hello.protocol` the highest version the client
  supports. Version 1 is the only one. A message with another `v` is rejected
  (`unsupported_version` on `hello`, otherwise `bad_message`).
- Inside v1 changes are additive only: new optional fields, new message types, new enum
  values for capabilities. Receivers ignore unknown fields and answer unknown types with
  `bad_message` without closing the session.
- A change that removes or renames a field, changes a meaning or makes a field required
  needs `v: 2`.
- Optional features are negotiated through `capabilities` (strings in `hello` and
  `welcome`); only capabilities present in both are active.

## Error codes

| Code | Meaning | Fatal |
|---|---|---|
| `bad_message` | frame is not a valid message | only before `welcome` |
| `unsupported_version` | `v` or `protocol` not supported | yes |
| `unauthenticated` | token invalid or expired | yes |
| `forbidden` | roles do not allow the message | no |
| `unknown_target` | cell, device, link or tag does not exist | no |
| `rate_limited` | the client sends too much | after repeats |
| `not_ready` | twin or daemon not connected, or a replay is running | no |
| `internal` | server fault | no |

## Replay format

Written by the cell simulation (D6), played by the twin. The same content exists as a
file and as a stream.

File (`*.replay.json`, schema `replay.schema.json`):

```json
{"v":1,
 "header":{"cell":"bottling","devices":["plc1","mcu1"],"start_ns":0,"schema_version":1},
 "events":[{"device":"plc1","t_ns":0,"kind":"OUT","name":"valve","value":"true"}]}
```

- `header.devices` lists every device that appears in `events`.
- `header.start_ns` is the cell-epoch time of the first state; events are in
  non-decreasing `t_ns` order.
- `header.schema_version` is the replay format version (1). Within it, changes are
  additive.
- `ReplayFile::validate` checks the version, the order and the device list.

Stream: `replay_header` (the header), any number of `replay_events` (same event objects,
time order), then `replay_end`. A client can start a replay with `replay_play`, which
carries a whole file.

## Which client uses which messages

| Message | Web | Desktop | Android | Twin | Daemon |
|---|---|---|---|---|---|
| `hello`, `ping`, `pong`, `subscribe`, `unsubscribe` | send | send | send | send | send |
| `welcome`, `error`, `ack` | receive | receive | receive | receive | receive |
| `cell_snapshot` | receive | receive | receive | send (cell it simulates) | not used |
| `telemetry`, `trace` | receive | receive | receive | send (virtual devices) | send (real devices) |
| `sim_*` control | send (operator) | send (operator) | not in v1 UI | receive | not used |
| `sim_state` | receive | receive | receive | send | not used |
| `replay_play` | send (operator) | send (operator) | not used | receive | not used |
| `replay_header`, `replay_events`, `replay_end` | receive | receive | receive | send | not used |
| `deployment_status` | receive | receive | receive | not used | send |
| `advisory_estop_request` | send (operator) | send (operator) | send (operator) | not used | receive |
| `estop_state` | receive | receive | receive | not used | send |

Android's supplementary E-STOP button sends `advisory_estop_request` and shows only the
`estop_state` the server returns.
