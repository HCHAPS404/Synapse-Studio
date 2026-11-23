"""Builders for the frames the relay server sends, and the grant from principal to roles.

The server only ever sends `welcome`, `error`, `ack`, `pong`, `cell_snapshot` and forwards
publisher frames (relay.md). These builders produce the exact JSON those messages require;
every outbound frame is still validated against the schema in tests. Error codes are the
relay codes of `relay.md` (`bad_message`, `unsupported_version`, `unauthenticated`,
`forbidden`, `unknown_target`, `rate_limited`, `not_ready`, `internal`), which are distinct
from the REST `NE11xx` catalogue.
"""

from __future__ import annotations

import time
from enum import StrEnum
from typing import Any

PROTOCOL_VERSION = 1


class RelayCode(StrEnum):
    """Error codes of the relay protocol (relay.md `## Error codes`)."""

    BAD_MESSAGE = "bad_message"
    UNSUPPORTED_VERSION = "unsupported_version"
    UNAUTHENTICATED = "unauthenticated"
    FORBIDDEN = "forbidden"
    UNKNOWN_TARGET = "unknown_target"
    RATE_LIMITED = "rate_limited"
    NOT_READY = "not_ready"
    INTERNAL = "internal"


# Codes that close the socket (relay.md: fatal before `welcome`, plus the two `hello` codes).
FATAL_CODES: frozenset[RelayCode] = frozenset(
    {RelayCode.UNSUPPORTED_VERSION, RelayCode.UNAUTHENTICATED}
)


def now_ns() -> int:
    """The server clock in nanoseconds since the Unix epoch (for `welcome`/`pong`)."""
    return time.time_ns()


def welcome(
    session_id: str, roles: list[str], *, capabilities: list[str] | None = None
) -> dict[str, Any]:
    """A `welcome` frame answering a `hello`."""
    frame: dict[str, Any] = {
        "v": PROTOCOL_VERSION,
        "type": "welcome",
        "session": session_id,
        "server_time_ns": now_ns(),
        "roles": roles,
        "protocol": PROTOCOL_VERSION,
    }
    if capabilities:
        frame["capabilities"] = capabilities
    return frame


def error(code: RelayCode, message: str, *, request_id: str | None = None) -> dict[str, Any]:
    """An `error` frame. `message` is English and never carries a token or secret."""
    frame: dict[str, Any] = {
        "v": PROTOCOL_VERSION,
        "type": "error",
        "code": code.value,
        "message": message,
    }
    if request_id is not None:
        frame["id"] = request_id
    return frame


def ack(request_id: str, request_type: str) -> dict[str, Any]:
    """An `ack` frame confirming a request that has no other reply."""
    return {
        "v": PROTOCOL_VERSION,
        "type": "ack",
        "id": request_id,
        "request": request_type,
    }


def pong(ping_id: int, t_ping_ns: int) -> dict[str, Any]:
    """A `pong` answering a `ping`; `t_ns` is the server clock when it received the ping."""
    return {
        "v": PROTOCOL_VERSION,
        "type": "pong",
        "id": ping_id,
        "t_ping_ns": t_ping_ns,
        "t_ns": now_ns(),
    }


def is_fatal(code: RelayCode) -> bool:
    """Whether an error of this code must be followed by closing the socket."""
    return code in FATAL_CODES
