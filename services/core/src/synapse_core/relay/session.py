"""Per-connection relay session: lifecycle, validation, roles, subscriptions (E4.1, E4.2).

Transport-agnostic so it can be driven by a real Starlette WebSocket or an in-process fake
in tests. The lifecycle follows `spec/protocols/relay.md`:

1. the first frame must be `hello` within `hello_timeout_s`; a bad or missing first frame,
   or a wrong protocol version, is fatal and closes the socket;
2. the server answers `welcome` (granted roles) or a fatal `unauthenticated`;
3. the server pings after `idle_ping_s` of silence and closes after `silence_close_s`;
4. every inbound frame is validated against the JSON Schema; a role may send only the
   messages of its row in the table, otherwise `forbidden`.

E4.2 adds: subscriptions (`subscribe`/`unsubscribe` with an `ack` and the cached
`cell_snapshot`), fan-out of publisher frames through the hub, a bounded per-session outbox
with the drop policy (a writer task drains it to the transport), and a control-message rate
limit of 20 per second per session (`rate_limited`), built on the E2.7 token bucket.
"""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass
from typing import Any, Protocol

from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from synapse_core.limits import BucketConfig, RateLimiter
from synapse_core.relay import protocol as proto
from synapse_core.relay.auth import RelayAuthError, identify
from synapse_core.relay.hub import Hub, Session, Subscription, _OutboxClosed
from synapse_core.relay.roles import may_send
from synapse_core.relay.validate import (
    FrameError,
    VersionError,
    message_type,
    validate_message,
)
from synapse_core.relay.validate import (
    decode_frame as _decode_frame,
)
from synapse_core.settings import Settings

log = logging.getLogger(__name__)

# Channels a client may subscribe to (the `Channel` enum of the schema).
VALID_CHANNELS: frozenset[str] = frozenset(
    {"cell", "telemetry", "trace", "deployment", "replay", "safety"}
)

# Control-message rate limit (relay.md: fewer than 20 control messages per second).
_CONTROL_SCOPE = "control"


class Transport(Protocol):
    """The minimal WebSocket surface the session needs."""

    async def accept(self) -> None: ...
    async def receive_text(self) -> str: ...
    async def send_json(self, data: dict[str, Any]) -> None: ...
    async def close(self, code: int = 1000) -> None: ...


class TransportClosed(Exception):
    """The peer closed the connection (the session ends cleanly)."""


@dataclass(frozen=True)
class SessionTimeouts:
    """Lifecycle timeouts; short values are injected in tests."""

    hello_timeout_s: float = 10.0
    idle_ping_s: float = 15.0
    silence_close_s: float = 45.0


# Client requests that carry an optional `id` and get an `ack` (sim/replay control). They are
# forwarded to the twin/daemon of the cell by the fan-out once E5 connects real publishers;
# here they are validated, role-checked, rate-limited and acknowledged.
_SIM_CONTROL: frozenset[str] = frozenset(
    {
        "sim_start",
        "sim_pause",
        "sim_step",
        "sim_set_input",
        "sim_cut_link",
        "sim_speed",
        "replay_play",
    }
)

# Publisher frames routed by the hub to the cell's subscribers.
_PUBLISHER: frozenset[str] = frozenset(
    {
        "telemetry",
        "trace",
        "sim_state",
        "deployment_status",
        "estop_state",
        "cell_snapshot",
        "replay_header",
        "replay_events",
        "replay_end",
    }
)

# Messages that count against the control-message rate limit: client-originated frames other
# than the liveness `pong` (which is a mandated response, not client traffic).
_RATE_EXEMPT: frozenset[str] = frozenset({"pong"})


class RelaySession:
    """Drives one relay connection to completion."""

    def __init__(
        self,
        transport: Transport,
        hub: Hub,
        settings: Settings,
        sessions: async_sessionmaker[AsyncSession] | None,
        *,
        timeouts: SessionTimeouts | None = None,
        control_rate: float = 20.0,
    ) -> None:
        self._t = transport
        self._hub = hub
        self._settings = settings
        self._sessions = sessions
        self._timeouts = timeouts or SessionTimeouts()
        self._session: Session | None = None
        self._writer: asyncio.Task[None] | None = None
        # One token bucket per session: capacity = burst = the per-second limit, refilled at
        # the same rate (reuses the E2.7 limiter core).
        self._limiter = RateLimiter({_CONTROL_SCOPE: BucketConfig(control_rate, control_rate)})

    async def run(self) -> None:
        """Accepts the socket, runs the handshake and the message loop, and cleans up."""
        await self._t.accept()
        try:
            if not await self._handshake():
                return
            self._writer = asyncio.create_task(self._drain_outbox())
            await self._loop()
        except TransportClosed:
            pass
        except Exception:  # noqa: BLE001 - never leak a server fault to the socket raw
            log.exception("relay session failed")
            await self._try_send(proto.error(proto.RelayCode.INTERNAL, "server fault"))
        finally:
            if self._session is not None:
                self._hub.unregister(self._session.id)
            if self._writer is not None:
                self._writer.cancel()
                try:
                    await self._writer
                except (asyncio.CancelledError, _OutboxClosed):
                    pass

    # -- outbox writer -------------------------------------------------------------

    async def _drain_outbox(self) -> None:
        """Sends queued frames to the transport in order until the outbox closes."""
        assert self._session is not None
        outbox = self._session.outbox
        try:
            while True:
                frame = await outbox.get()
                await self._t.send_json(frame)
        except _OutboxClosed:
            return
        except TransportClosed:
            return

    # -- handshake -----------------------------------------------------------------

    async def _handshake(self) -> bool:
        """Reads the first frame, validates it is a `hello`, authenticates, sends `welcome`.

        Returns `True` when the session is established, `False` when the socket was closed.
        """
        try:
            text = await asyncio.wait_for(
                self._t.receive_text(), timeout=self._timeouts.hello_timeout_s
            )
        except TimeoutError:
            await self._fatal(proto.RelayCode.BAD_MESSAGE, "no hello within the time limit")
            return False

        try:
            frame = _decode_frame(text)
            validate_message(frame)
        except VersionError:
            await self._fatal(proto.RelayCode.UNSUPPORTED_VERSION, "unsupported protocol version")
            return False
        except FrameError as exc:
            await self._fatal(proto.RelayCode.BAD_MESSAGE, str(exc))
            return False

        if message_type(frame) != "hello":
            await self._fatal(proto.RelayCode.BAD_MESSAGE, "the first frame must be hello")
            return False
        if int(frame.get("protocol", 0)) != proto.PROTOCOL_VERSION:
            await self._fatal(proto.RelayCode.UNSUPPORTED_VERSION, "unsupported protocol version")
            return False

        token = frame.get("token")
        try:
            identity = await self._identify(token)
        except RelayAuthError:
            await self._fatal(proto.RelayCode.UNAUTHENTICATED, "the token is invalid or expired")
            return False

        session_id = self._hub.next_session_id()
        capabilities = [c for c in frame.get("capabilities", []) if isinstance(c, str)]
        self._session = Session(
            id=session_id,
            roles=identity.roles,
            key=identity.key,
            outbox=self._hub.new_outbox(),
            cell_id=identity.cell_id,
            capabilities=capabilities,
        )
        self._hub.register(self._session)
        # welcome is sent directly (before the writer starts) so it is always the first frame.
        await self._t.send_json(
            proto.welcome(session_id, sorted(identity.roles), capabilities=capabilities or None)
        )
        return True

    async def _identify(self, token: str | None):  # noqa: ANN202 - returns RelayIdentity
        if token is None:
            return await identify(None, None, self._settings)
        if self._sessions is None:  # pragma: no cover - endpoint provides a session maker
            raise RelayAuthError("no database configured")
        async with self._sessions() as db:
            return await identify(db, token, self._settings)

    # -- message loop --------------------------------------------------------------

    async def _loop(self) -> None:
        """Receives frames until the peer closes or stays silent past the limit."""
        assert self._session is not None
        pinged = False
        while True:
            try:
                text = await asyncio.wait_for(
                    self._t.receive_text(), timeout=self._timeouts.idle_ping_s
                )
            except TimeoutError:
                if pinged:
                    await self._t.close(code=1000)
                    return
                self._enqueue(self._server_ping())
                pinged = True
                try:
                    text = await asyncio.wait_for(
                        self._t.receive_text(),
                        timeout=self._timeouts.silence_close_s - self._timeouts.idle_ping_s,
                    )
                except TimeoutError:
                    await self._t.close(code=1000)
                    return
            pinged = False
            await self._handle(text)

    async def _handle(self, text: str) -> None:
        """Validates and dispatches one inbound frame after `welcome`."""
        try:
            frame = _decode_frame(text)
            validate_message(frame)
        except VersionError:
            self._enqueue(proto.error(proto.RelayCode.BAD_MESSAGE, "unsupported version"))
            return
        except FrameError as exc:
            self._enqueue(proto.error(proto.RelayCode.BAD_MESSAGE, str(exc)))
            return

        mtype = message_type(frame)
        assert self._session is not None
        request_id = frame.get("id") if isinstance(frame.get("id"), str) else None

        if not may_send(self._session.roles, mtype):
            self._enqueue(
                proto.error(
                    proto.RelayCode.FORBIDDEN,
                    f"a session with roles {sorted(self._session.roles)} may not send {mtype}",
                    request_id=request_id,
                )
            )
            return

        if mtype not in _RATE_EXEMPT and not self._allow_control():
            self._enqueue(
                proto.error(
                    proto.RelayCode.RATE_LIMITED,
                    "too many control messages; slow down",
                    request_id=request_id,
                )
            )
            return

        if mtype == "ping":
            self._enqueue(proto.pong(int(frame["id"]), int(frame["t_ns"])))
            return
        if mtype == "pong":
            return
        if mtype == "hello":
            self._enqueue(proto.error(proto.RelayCode.BAD_MESSAGE, "already greeted"))
            return
        if mtype == "subscribe":
            self._on_subscribe(frame, request_id)
            return
        if mtype == "unsubscribe":
            self._on_unsubscribe(frame, request_id)
            return
        if mtype in _SIM_CONTROL:
            # Routed to the cell's twin/daemon by the fan-out; acked to the sender.
            self._fan_out(frame)
            if request_id is not None:
                self._enqueue(proto.ack(request_id, mtype))
            return
        if mtype in _PUBLISHER:
            self._fan_out(frame)
            return

    # -- handlers ------------------------------------------------------------------

    def _on_subscribe(self, frame: dict[str, Any], request_id: str | None) -> None:
        assert self._session is not None
        cell = str(frame["cell"])
        device = frame.get("device")
        device = str(device) if isinstance(device, str) else None
        channels = [c for c in frame.get("channels", []) if c in VALID_CHANNELS]
        self._hub.subscribe(
            self._session, Subscription(cell=cell, channels=frozenset(channels), device=device)
        )
        if request_id is not None:
            self._enqueue(proto.ack(request_id, "subscribe"))
        # Replay the cached cell snapshot to a new `cell`-channel subscriber (relay.md).
        if "cell" in channels:
            snapshot = self._hub.cached_snapshot(cell)
            if snapshot is not None:
                self._enqueue(snapshot)

    def _on_unsubscribe(self, frame: dict[str, Any], request_id: str | None) -> None:
        assert self._session is not None
        cell = str(frame["cell"])
        device = frame.get("device")
        device = str(device) if isinstance(device, str) else None
        self._hub.unsubscribe(self._session, cell, device)
        if request_id is not None:
            self._enqueue(proto.ack(request_id, "unsubscribe"))

    def _fan_out(self, frame: dict[str, Any]) -> None:
        """Delivers a publisher/control frame to the cell's subscribers (not the sender)."""
        assert self._session is not None
        cell = frame.get("cell")
        if isinstance(cell, str):
            self._hub.publish(frame, cell, exclude=self._session.id)

    def _allow_control(self) -> bool:
        assert self._session is not None
        return self._limiter.check(_CONTROL_SCOPE, self._session.id).allowed

    # -- helpers -------------------------------------------------------------------

    def _server_ping(self) -> dict[str, Any]:
        return {"v": proto.PROTOCOL_VERSION, "type": "ping", "id": 0, "t_ns": proto.now_ns()}

    def _enqueue(self, frame: dict[str, Any]) -> None:
        """Queues a frame on the session outbox (subject to the drop policy)."""
        if self._session is not None:
            self._session.enqueue(frame)

    async def _try_send(self, frame: dict[str, Any]) -> None:
        try:
            await self._t.send_json(frame)
        except Exception:  # noqa: BLE001, S110 - the socket may already be gone; best effort
            pass

    async def _fatal(self, code: proto.RelayCode, message: str) -> None:
        """Sends a fatal error and closes the socket (handshake failures)."""
        await self._try_send(proto.error(code, message))
        await self._t.close(code=1008)
