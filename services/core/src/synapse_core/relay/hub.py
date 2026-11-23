"""In-memory relay hub (E4.1 core; subscriptions, cache and fan-out added in E4.2).

One instance per process, as the design states: a deploy drops the sockets and clients
reconnect (the protocol has no resume). The hub tracks live sessions so later tasks can
fan out publisher frames to subscribers (E4.2) and close a revoked daemon's sockets (E4.3).
E4.1 registers and unregisters sessions and assigns session ids.
"""

from __future__ import annotations

import itertools
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from typing import Any

# A sink sends one already-built frame (a dict) to a connected peer.
Sink = Callable[[dict[str, Any]], Awaitable[None]]


@dataclass
class Session:
    """A live relay connection."""

    id: str
    roles: frozenset[str]
    key: str
    sink: Sink
    cell_id: str | None = None
    capabilities: list[str] = field(default_factory=list)


class Hub:
    """Registry of live sessions. Not safe to share across processes (one instance)."""

    def __init__(self) -> None:
        self._sessions: dict[str, Session] = {}
        self._counter = itertools.count(1)

    def next_session_id(self) -> str:
        """A monotonically increasing session id, `s-0001` style."""
        return f"s-{next(self._counter):04d}"

    def register(self, session: Session) -> None:
        """Adds a session to the registry."""
        self._sessions[session.id] = session

    def unregister(self, session_id: str) -> None:
        """Removes a session (idempotent)."""
        self._sessions.pop(session_id, None)

    def sessions(self) -> list[Session]:
        """A snapshot of the live sessions."""
        return list(self._sessions.values())

    def count(self) -> int:
        """How many sessions are live."""
        return len(self._sessions)
