"""In-memory relay hub: sessions, subscriptions, snapshot cache and fan-out (E4.1, E4.2).

One instance per process, as the design states: a deploy drops the sockets and clients
reconnect (the protocol has no resume). E4.1 added the session registry and ids. E4.2 adds:

- subscriptions: a session subscribes to a cell (optionally one device) and a set of
  channels; the hub keeps who wants what per cell;
- a `cell_snapshot` cache per cell, replayed to a new subscriber of the `cell` channel;
- fan-out: a publisher frame (`telemetry`, `trace`, `sim_state`, `deployment_status`,
  `estop_state`, replay framing) is delivered to the matching subscribers;
- a bounded per-session outbound queue with a drop policy: when the queue is full the
  oldest `telemetry`/`trace` frames are dropped first, never a control reply, a snapshot or
  replay framing (relay.md "Ordering and backpressure").
"""

from __future__ import annotations

import asyncio
import itertools
import logging
from collections import deque
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from typing import Any

log = logging.getLogger(__name__)

# A sink sends one already-built frame (a dict) to a connected peer.
Sink = Callable[[dict[str, Any]], Awaitable[None]]

# Which message types a channel carries (the `Channel` enum of the schema).
CHANNEL_MESSAGES: dict[str, frozenset[str]] = {
    "cell": frozenset({"cell_snapshot", "sim_state"}),
    "telemetry": frozenset({"telemetry"}),
    "trace": frozenset({"trace"}),
    "deployment": frozenset({"deployment_status"}),
    "replay": frozenset({"replay_header", "replay_events", "replay_end"}),
    "safety": frozenset({"estop_state"}),
}

# Frames that may be dropped for a slow subscriber (oldest first). Everything else — control
# replies, snapshots and replay framing — is never dropped.
_DROPPABLE: frozenset[str] = frozenset({"telemetry", "trace"})


@dataclass
class Subscription:
    """One session's interest in a cell: the channels, and an optional single device."""

    cell: str
    channels: frozenset[str]
    device: str | None = None


class BoundedOutbox:
    """A bounded FIFO of outbound frames with the relay drop policy.

    When full, `put` drops the oldest droppable (`telemetry`/`trace`) frame to make room; if
    none can be dropped and the incoming frame is itself droppable, the incoming frame is
    dropped; a non-droppable frame (control, snapshot, replay framing) is always kept, even
    past the soft bound, so the protocol's guarantee holds. `dropped` counts losses.
    """

    def __init__(self, maxsize: int) -> None:
        self._maxsize = maxsize
        self._items: deque[dict[str, Any]] = deque()
        self._event = asyncio.Event()
        self._closed = False
        self.dropped = 0

    def put(self, frame: dict[str, Any]) -> None:
        droppable = frame.get("type") in _DROPPABLE
        if len(self._items) >= self._maxsize:
            if not self._drop_one_droppable():
                if droppable:
                    self.dropped += 1
                    return  # queue full of control frames and this one is droppable: drop it
                # else: keep the control/snapshot/replay frame even past the bound
        self._items.append(frame)
        self._event.set()

    def _drop_one_droppable(self) -> bool:
        for i, item in enumerate(self._items):
            if item.get("type") in _DROPPABLE:
                del self._items[i]
                self.dropped += 1
                return True
        return False

    async def get(self) -> dict[str, Any]:
        """Waits for and returns the next frame; raises `_OutboxClosed` when closed empty."""
        while not self._items:
            if self._closed:
                raise _OutboxClosed()
            self._event.clear()
            await self._event.wait()
        return self._items.popleft()

    def close(self) -> None:
        self._closed = True
        self._event.set()

    def __len__(self) -> int:
        return len(self._items)


class _OutboxClosed(Exception):
    """The outbox was closed and is empty (the writer task ends)."""


@dataclass
class Session:
    """A live relay connection and its subscriptions."""

    id: str
    roles: frozenset[str]
    key: str
    outbox: BoundedOutbox
    cell_id: str | None = None
    capabilities: list[str] = field(default_factory=list)
    subscriptions: dict[tuple[str, str | None], Subscription] = field(default_factory=dict)

    def enqueue(self, frame: dict[str, Any]) -> None:
        """Puts a frame on the outbox (subject to the drop policy)."""
        self.outbox.put(frame)


class Hub:
    """Registry of sessions, their subscriptions and the per-cell snapshot cache."""

    def __init__(self, *, outbox_maxsize: int = 256) -> None:
        self._sessions: dict[str, Session] = {}
        self._counter = itertools.count(1)
        self._snapshots: dict[str, dict[str, Any]] = {}
        self._outbox_maxsize = outbox_maxsize

    # -- sessions ------------------------------------------------------------------

    def next_session_id(self) -> str:
        return f"s-{next(self._counter):04d}"

    def new_outbox(self) -> BoundedOutbox:
        """A bounded outbox sized by the hub's default (one per session)."""
        return BoundedOutbox(self._outbox_maxsize)

    def register(self, session: Session) -> None:
        self._sessions[session.id] = session

    def unregister(self, session_id: str) -> None:
        session = self._sessions.pop(session_id, None)
        if session is not None:
            session.outbox.close()

    def sessions(self) -> list[Session]:
        return list(self._sessions.values())

    def count(self) -> int:
        return len(self._sessions)

    # -- subscriptions -------------------------------------------------------------

    def subscribe(self, session: Session, sub: Subscription) -> None:
        """Records (or replaces) a subscription keyed by `(cell, device)`.

        Returns nothing; the caller sends the cached snapshot afterwards if the `cell`
        channel was requested.
        """
        session.subscriptions[(sub.cell, sub.device)] = sub

    def unsubscribe(self, session: Session, cell: str, device: str | None) -> bool:
        """Removes a subscription; `True` when one existed."""
        return session.subscriptions.pop((cell, device), None) is not None

    def cached_snapshot(self, cell: str) -> dict[str, Any] | None:
        """The last `cell_snapshot` seen for a cell, if any."""
        return self._snapshots.get(cell)

    # -- fan-out -------------------------------------------------------------------

    def publish(self, frame: dict[str, Any], cell: str, *, exclude: str | None = None) -> int:
        """Delivers a publisher frame to every matching subscriber; returns the count.

        A `cell_snapshot` is also cached so a later subscriber receives it. Matching: the
        subscriber is subscribed to this cell, the frame's channel is among its channels,
        and, when the subscriber named a device, the frame concerns that device.
        """
        mtype = frame.get("type")
        if mtype == "cell_snapshot":
            self._snapshots[cell] = frame
        delivered = 0
        for session in self._sessions.values():
            if exclude is not None and session.id == exclude:
                continue
            if self._wants(session, cell, mtype, frame):
                session.enqueue(frame)
                delivered += 1
        return delivered

    def _wants(self, session: Session, cell: str, mtype: str | None, frame: dict[str, Any]) -> bool:
        for (sub_cell, sub_device), sub in session.subscriptions.items():
            if sub_cell != cell:
                continue
            if not any(mtype in CHANNEL_MESSAGES.get(ch, frozenset()) for ch in sub.channels):
                continue
            if sub_device is not None and not _frame_touches_device(frame, sub_device):
                continue
            return True
        return False


def _frame_touches_device(frame: dict[str, Any], device: str) -> bool:
    """Whether a device-scoped subscriber should receive this frame.

    `telemetry` samples carry `id` of the form `<device>.<tag>`; `trace`/`replay_events`
    events carry an explicit `device`; `deployment_status` lists device statuses. A frame
    with no device information (e.g. `sim_state`, `estop_state`) always matches a
    device-scoped subscription of its cell.
    """
    mtype = frame.get("type")
    if mtype == "telemetry":
        return any(
            str(s.get("id", "")).split(".", 1)[0] == device for s in frame.get("samples", [])
        )
    if mtype in ("trace", "replay_events"):
        return any(e.get("device") == device for e in frame.get("events", []))
    if mtype == "deployment_status":
        return any(d.get("device") == device for d in frame.get("devices", []))
    return True
