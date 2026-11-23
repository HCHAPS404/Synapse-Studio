"""Builders for the relay frames the daemon sends (Neuron Wire, relay.md v1).

Every frame is an object with `"v": 1` and `"type"`. The daemon sends the common frames
(`hello`, `ping`, `pong`) plus its publisher frames (`telemetry`, `trace`,
`deployment_status`, `estop_state`). Field names and shapes follow
`spec/protocols/examples/`. The schema in `spec/schemas/relay/message.schema.json` is the
source of truth; these builders produce frames that validate against it.
"""

from __future__ import annotations

from typing import Any

from synapse_daemon.drivers.base import Sample

PROTOCOL_VERSION = 1


def hello(
    *,
    token: str | None,
    client_version: str,
    capabilities: list[str] | None = None,
) -> dict[str, Any]:
    """The first frame the daemon sends. `client` is `daemon`; an absent token is omitted."""
    frame: dict[str, Any] = {
        "v": PROTOCOL_VERSION,
        "type": "hello",
        "client": "daemon",
        "protocol": PROTOCOL_VERSION,
        "client_version": client_version,
        "capabilities": capabilities or [],
    }
    if token:
        frame["token"] = token
    return frame


def ping(id_: int, t_ns: int) -> dict[str, Any]:
    """A liveness ping carrying the sender's own clock."""
    return {"v": PROTOCOL_VERSION, "type": "ping", "id": id_, "t_ns": t_ns}


def pong(id_: int, t_ping_ns: int, t_ns: int) -> dict[str, Any]:
    """The answer to a server `ping`."""
    return {
        "v": PROTOCOL_VERSION,
        "type": "pong",
        "id": id_,
        "t_ping_ns": t_ping_ns,
        "t_ns": t_ns,
    }


def telemetry(cell: str, samples: list[Sample]) -> dict[str, Any]:
    """A batch of samples. A `bad`/`stale` sample carries its quality; no guessed values."""
    out: list[dict[str, Any]] = []
    for s in samples:
        entry: dict[str, Any] = {"id": s.tag, "t_ns": s.t_ns, "quality": s.quality.value}
        if s.value is not None:
            entry["value"] = s.value
        out.append(entry)
    return {"v": PROTOCOL_VERSION, "type": "telemetry", "cell": cell, "samples": out}


def trace(cell: str, events: list[dict[str, Any]]) -> dict[str, Any]:
    """A batch of trace events already shaped as `{device, t_ns, kind, name, value}`."""
    return {"v": PROTOCOL_VERSION, "type": "trace", "cell": cell, "events": events}


def deployment_status(cell: str, devices: list[dict[str, Any]]) -> dict[str, Any]:
    """The daemon heartbeat and per-device online/build status (read-only)."""
    return {
        "v": PROTOCOL_VERSION,
        "type": "deployment_status",
        "cell": cell,
        "devices": devices,
    }


def estop_state(
    cell: str, state: str, t_ns: int, request_id: str | int | None = None
) -> dict[str, Any]:
    """The reported E-STOP circuit state. `state` is reported by the daemon only.

    ADVISORY path: this frame reports the state the daemon read from its input; it never
    invents a state. The hardwired E-STOP is the authority. `request_id` echoes a request.
    """
    frame: dict[str, Any] = {
        "v": PROTOCOL_VERSION,
        "type": "estop_state",
        "cell": cell,
        "state": state,
        "t_ns": t_ns,
    }
    if request_id is not None:
        frame["id"] = request_id
    return frame
