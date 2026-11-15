# 11 · Interchange standard

Neuron is the hub through which programs, devices and third-party languages meet. The
**Neuron Interchange Standard** has six parts that every tool (importer, emitter,
compiler, simulator, daemon, editor) shares.

| Part | What it standardizes | Artifact |
|---|---|---|
| NIR | The canonical program representation | `.nir.json` |
| Tag dictionary | One definition per signal, with its address on every device | `tags.json` (generated) |
| Neuron Wire | The message format between devices Neuron controls on both ends | wire frames |
| Fidelity report | What a translation preserved and what it did not | `fidelity.json` |
| Source map | Generated code line ↔ `.neuron` span ↔ graphical element | `*.map.json` |
| Deployment package | What is shipped to a device and how it is verified | `*.npkg` |

## 11.1 NIR (Neuron Intermediate Representation)

NIR is the program after parsing, name resolution and type and unit checking,
serialized as canonical JSON. It is the contract between the compiler and every other
tool: editors and graphical views, the digital twin, emitters and importers.

The NIR is defined in **[chapter 16](16-nir.md)**: data model, stable ids, canonical
form, the encoding of types, units and exact numbers, the printer back to `.neuron`,
the properties NIR-1 and P-RT, and versioning. The normative JSON Schema is
[`spec/schemas/nir.schema.json`](../schemas/nir.schema.json), generated from the
implementation and checked in CI.

In short:

- `nir_version` is semantic (`1.0`); readers accept any `1.x` and ignore fields they
  do not know.
- The same program always serializes to the same bytes.
- Numbers are JSON strings with their exact value, so no precision is lost between
  languages.
- References use stable ids (`m0.i3`), never positions. Diagram layout lives in
  `.layout.json`, never in NIR.

## 11.2 Tag dictionary

A **tag** is a named signal visible across devices. Each tag has exactly one
definition, and the compiler generates the address of that tag on every device that
uses it. This replaces hand-mapping registers, data blocks and pins.

| Field | Meaning |
|---|---|
| `name` | Unique in the cell |
| `type`, `unit` | Value type and physical unit (`f32`, `mm/s`) |
| `range` | Valid values, used for scaling and checks |
| `rate` | Update period, used for latency analysis |
| `domain` | Clock domain of the producer |
| `owner` | The single device that writes it |
| `access` | `read`, `write` or `read_write` per consumer |
| `safe` | Value consumers assume when the link is lost |
| `locations` | The address on each device (table below) |

| Medium | Location syntax | Example |
|---|---|---|
| IEC / PLC memory | `%` direct address | `plc.%MD20` |
| PLC data block (S7) | `db(n).offset` | `plc.db(1).x0` |
| Modbus | `hr(n)`, `ir(n)`, `coil(n)`, `di(n)` | `plc.hr(40001)` |
| Microcontroller | symbol or pin | `ctl.speed`, `ctl.PA5` |
| FPGA | port or pin | `fpga.led_out`, `fpga.W5` |
| OPC UA | node id | `opcua("ns=2;s=Line.Speed")` |
| MQTT | topic | `mqtt("line/speed")` |
| ROS 2 | topic and type | `ros2("/line/speed", "std_msgs/Float32")` |
| Neuron Wire | tag id (u16) assigned by the compiler | `wire` |

Rules: a tag mapped with incompatible types or units on two devices is `NE0703`;
two writers are an error; a tag without a `safe` value on a link that can fail is a
warning for inputs and an error for actuator commands.

## 11.3 Neuron Wire

Neuron Wire is the message format used when Neuron generates **both ends** of a link
(UART, CAN, TCP, MQTT, USB-CDC). When one end is third-party equipment, an adapter
maps tags to its native protocol (Modbus, S7, OPC UA) and Neuron Wire is not used.

### Frame

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 1 | `ver_flags` | high nibble version (1); bit 0 ack requested, bit 1 timestamped, bit 2 authenticated |
| 1 | 1 | `type` | see message types |
| 2 | 2 | `session` | random at link start; ties frames to one connection |
| 4 | 4 | `seq` | per-sender counter, wraps |
| 8 | 0 or 8 | `ts` | sender clock in ns, present if timestamped |
| next | 2 | `len` | payload length |
| next | `len` | `payload` | little-endian values, no padding |
| next | 0 or 8 | `mac` | truncated HMAC-SHA-256, present if authenticated |
| last | 2 | `crc` | CRC-16/CCITT-FALSE over all previous bytes |

Serial links wrap the frame with COBS and end it with a `0x00` delimiter. TCP links
prefix a 2-byte length. CAN splits the frame into 8-byte fragments with a one-byte
fragment header.

### Message types

| Code | Name | Purpose |
|---|---|---|
| `0x01` | HELLO | Version, device id, hash of the tag table, features |
| `0x02` | DATA | One or more tag values: `tag_id (u16)` then the value |
| `0x03` | ACK | Acknowledges a `seq`; optionally carries the receiver's timestamp |
| `0x04` | HEARTBEAT | State (`run`, `safe`, `fault`) and uptime |
| `0x05` | TIMESYNC_REQ | Starts a clock-offset measurement |
| `0x06` | TIMESYNC_RSP | Answers with receive and send timestamps |
| `0x07` | FAULT | Fault code and the tag involved |
| `0x08` | CMD | Commands from the supervision plane (allowlist only) |
| `0x09` | NACK | Rejected frame with a reason |

### Reliability classes

- **Fire and forget:** no acknowledgement (telemetry).
- **Acknowledged:** retransmit with back-off up to N times, then raise `LinkLost`.
- **Latched:** the sender holds the output until the receiver acknowledges that it
  has *sampled* the value in its own cycle (Latch + ACK).

Duplicates are discarded by `(session, seq)`; a frame older than half the `seq` space
is discarded as stale.

### Heartbeat and watchdog

Each side sends HEARTBEAT every `T_hb` (default 10 ms). A peer is lost after five
missed heartbeats, which gives the default cell watchdog of 50 ms; the device then
enters its safe state.

### Clock synchronization

With timestamps `t1` (request sent), `t2` (received), `t3` (response sent) and `t4`
(response received):

```text
offset = ((t2 - t1) + (t3 - t4)) / 2
delay  = (t4 - t1) - (t3 - t2)
```

The sample with the smallest `delay` among the last N wins; drift is the slope of a
linear fit over successive offsets. The reported accuracy bound is `delay / 2` plus
the clock resolution. Where the hardware supports PTP it is used instead.

### Values

`bool` 1 byte; unsigned and signed integers little-endian at their width; `f32` and
`f64` IEEE 754 little-endian; `fixed` as its underlying integer; `duration` as signed
64-bit nanoseconds; arrays concatenated; structs in declaration order.

> **Property WIRE-1.** `decode(encode(m)) == m` for every message.
> **Property WIRE-2.** Any single corrupted frame is rejected by the CRC (and, when
> authenticated, by the MAC).
> **Property WIRE-3.** A frame is applied at most once per `(session, seq)`.

### Security

Over IP, links use the authenticated flag with a per-link key and reject frames
outside a replay window. Industrial protocols (Modbus, S7) are never bridged to
the public internet; the relay speaks Neuron Wire over TLS.

## 11.4 Fidelity report

Every import and every emission produces `fidelity.json`: one entry per construct.

| Class | Meaning |
|---|---|
| `exact` | Same observable behavior by construction |
| `equivalent` | Same observable behavior up to a documented difference (for example scan period) |
| `approximate` | Close; the difference is quantified in the entry |
| `modeled` | Represented as a black box with a declared contract |
| `unsupported` | Cannot be represented; the construct is reported and not translated |

```json
{
  "tool": "emit-scl", "version": "0.1.0", "target": "plc.generic",
  "summary": { "exact": 41, "equivalent": 3, "approximate": 1, "modeled": 0, "unsupported": 0 },
  "entries": [
    { "span": [301, 356], "class": "equivalent",
      "note": "task period 10 ms mapped to cyclic OB with 10 ms interval" }
  ]
}
```

A tool never downgrades silently: anything not `exact` appears in the report, and the
build fails if the project's policy forbids `approximate` or worse.

## 11.5 Source maps

Each emitter writes a map from generated code to the program:

```json
{
  "version": 1,
  "generated": "conveyor.scl",
  "mappings": [
    { "gen": [12, 0, 12, 34], "src": { "file": "conveyor.neuron", "span": [301, 356] },
      "view": "rung:run_motor" }
  ]
}
```

`gen` is `[line, column, end_line, end_column]`. The inspector uses it to highlight the
running line, the debugger to place breakpoints, and the editor to open the graphical
element named in `view`.

## 11.6 Importer and emitter contracts

| | Input | Output |
|---|---|---|
| **Importer** | Source text of another language and options | NIR, `fidelity.json`, diagnostics |
| **Emitter** | NIR, target and board descriptions | Source files, configuration, wiring report, `fidelity.json`, source map |

- Both are pure and deterministic: the same input yields byte-identical output.
- **Correctness criterion:** for a stimulus set `S`, the observable trace of the
  translated program equals that of the original within the tolerance stated in the
  fidelity report (chapter 14, §14.10).
- Plugins declare their capabilities (languages, targets, constructs) in a manifest;
  adding one never changes the core compiler.

## 11.7 Deployment package

```text
build/blink.npkg
  manifest.json      target, tool versions, NIR hash, audit hash, manifest trust levels
  code/              generated sources and binaries
  config/            pin plans, constraints, tables
  wiring.md          what to connect, voltage levels, isolation required
  fidelity.json
  audit.json         trusted code, assumptions, disabled checks
  *.map.json
  SIGNATURE          Ed25519 signature over the manifest hash
```

The Edge Daemon refuses unsigned or altered packages, packages built from manifests
below the *verified* trust level, and packages that did not pass the deployment gates
(chapter 08, §8.5).
