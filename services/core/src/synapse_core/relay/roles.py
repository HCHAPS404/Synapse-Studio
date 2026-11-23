"""The relay role table and message→permission mapping (E4.1).

`ROLE_MESSAGES` is the "Roles and what each may send" table of `spec/protocols/relay.md`.
A test (`test_relay_roles.py`) parses that markdown table and asserts it equals this dict,
so the code and the contract cannot drift. A session may hold several roles (the relay
`hello` grants roles from the Core API principal); a message is allowed when at least one
held role may send it, otherwise the relay answers `forbidden`.

The relay reuses the REST permission matrix (`auth/permissions.py`) only indirectly: the
roles here are the protocol roles of `relay.md` (`guest`, `viewer`, `operator`, `twin`,
`daemon`), which the session derives from the authenticated principal.
"""

from __future__ import annotations

# The five messages every session role may send ("the common ones" in relay.md).
_COMMON: frozenset[str] = frozenset({"hello", "ping", "pong", "subscribe", "unsubscribe"})

_OPERATOR_EXTRA: frozenset[str] = frozenset(
    {
        "sim_start",
        "sim_pause",
        "sim_step",
        "sim_set_input",
        "sim_cut_link",
        "sim_speed",
        "replay_play",
        "advisory_estop_request",
    }
)

_TWIN_EXTRA: frozenset[str] = frozenset(
    {
        "cell_snapshot",
        "telemetry",
        "trace",
        "sim_state",
        "replay_header",
        "replay_events",
        "replay_end",
        "ack",
        "error",
    }
)

_DAEMON_EXTRA: frozenset[str] = frozenset(
    {"telemetry", "trace", "deployment_status", "estop_state", "ack", "error"}
)

ROLE_MESSAGES: dict[str, frozenset[str]] = {
    "guest": _COMMON,
    "viewer": _COMMON,
    "operator": _COMMON | _OPERATOR_EXTRA,
    "twin": _COMMON | _TWIN_EXTRA,
    "daemon": _COMMON | _DAEMON_EXTRA,
}

# Messages the server sends and never accepts from a client (relay.md: "The server sends…").
# They are not in any role's row, so a client sending one is `forbidden`.
SERVER_ONLY: frozenset[str] = frozenset({"welcome"})


def may_send(roles: frozenset[str], message_type: str) -> bool:
    """Whether a session holding `roles` may send `message_type` (any role suffices)."""
    return any(message_type in ROLE_MESSAGES.get(role, frozenset()) for role in roles)
