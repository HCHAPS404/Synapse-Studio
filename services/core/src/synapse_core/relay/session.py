"""Per-connection relay session: lifecycle, validation, role checks (E4.1).

Transport-agnostic so it can be driven by a real Starlette WebSocket or an in-process fake
in tests. The lifecycle follows `spec/protocols/relay.md`:

1. the first frame must be `hello` within `hello_timeout_s`; a bad or missing first frame,
   or a wrong protocol version, is fatal and closes the socket;
2. the server answers `welcome` (granted roles) or a fatal `unauthenticated`;
3. the server pings after `idle_ping_s` of silence and closes after `silence_close_s`;
4. every inbound frame is validated against the JSON Schema; a role may send only the
   messages of its row in the table, otherwise `forbidden`.

Subscriptions, the `cell_snapshot` cache, fan-out and the control-message rate limit are
added in E4.2; E4.1 validates, checks the role and acks requests that carry an `id`.
"""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass
from typing import Any, Protocol

from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from synapse_core.relay import protocol as proto
from synapse_core.relay.auth import RelayAuthError, identify
from synapse_core.relay.hub import Hub, Session
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


# Requests that carry an optional `id` and are confirmed with `ack` when E4.1 has no other
# reply for them yet. subscribe/unsubscribe get real behaviour in E4.2.
_ACKABLE: frozenset[str] = frozenset(
    {
        "subscribe",
        "unsubscribe",
        "sim_start",
        "sim_pause",
        "sim_step",
        "sim_set_input",
        "sim_cut_link",
        "sim_speed",
        "replay_play",
    }
)


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
    ) -> None:
        self._t = transport
        self._hub = hub
        self._settings = settings
        self._sessions = sessions
        self._timeouts = timeouts or SessionTimeouts()
        self._session: Session | None = None

    async def run(self) -> None:
        """Accepts the socket, runs the handshake and the message loop, and cleans up."""
        await self._t.accept()
        try:
            if not await self._handshake():
                return
            await self._loop()
        except TransportClosed:
            pass
        except Exception:  # noqa: BLE001 - never leak a server fault to the socket raw
            log.exception("relay session failed")
            await self._try_send(proto.error(proto.RelayCode.INTERNAL, "server fault"))
        finally:
            if self._session is not None:
                self._hub.unregister(self._session.id)

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
            sink=self._t.send_json,
            cell_id=identity.cell_id,
            capabilities=capabilities,
        )
        self._hub.register(self._session)
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
            # First wait up to the idle window; on timeout send a ping and wait the rest of
            # the silence budget for any frame before closing.
            try:
                text = await asyncio.wait_for(
                    self._t.receive_text(), timeout=self._timeouts.idle_ping_s
                )
            except TimeoutError:
                if pinged:
                    await self._t.close(code=1000)
                    return
                await self._send_server_ping()
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
            # After welcome, a version problem is a non-fatal bad_message (relay.md).
            await self._send(proto.error(proto.RelayCode.BAD_MESSAGE, "unsupported version"))
            return
        except FrameError as exc:
            await self._send(proto.error(proto.RelayCode.BAD_MESSAGE, str(exc)))
            return

        mtype = message_type(frame)
        assert self._session is not None
        request_id = frame.get("id") if isinstance(frame.get("id"), str) else None

        if not may_send(self._session.roles, mtype):
            await self._send(
                proto.error(
                    proto.RelayCode.FORBIDDEN,
                    f"a session with roles {sorted(self._session.roles)} may not send {mtype}",
                    request_id=request_id,
                )
            )
            return

        if mtype == "ping":
            await self._send(proto.pong(int(frame["id"]), int(frame["t_ns"])))
            return
        if mtype == "pong":
            return  # liveness only; nothing to answer
        if mtype == "hello":
            await self._send(proto.error(proto.RelayCode.BAD_MESSAGE, "already greeted"))
            return
        if mtype in _ACKABLE and request_id is not None:
            # E4.1 confirms the request; E4.2 adds subscriptions, cache and forwarding.
            await self._send(proto.ack(request_id, mtype))
            return
        # Publisher frames (telemetry/trace/…) are accepted and fanned out in E4.2.

    # -- helpers -------------------------------------------------------------------

    async def _send_server_ping(self) -> None:
        await self._send(
            {"v": proto.PROTOCOL_VERSION, "type": "ping", "id": 0, "t_ns": proto.now_ns()}
        )

    async def _send(self, frame: dict[str, Any]) -> None:
        await self._t.send_json(frame)

    async def _try_send(self, frame: dict[str, Any]) -> None:
        try:
            await self._t.send_json(frame)
        except Exception:  # noqa: BLE001, S110 - the socket may already be gone; best effort
            pass

    async def _fatal(self, code: proto.RelayCode, message: str) -> None:
        """Sends a fatal error and closes the socket (handshake failures)."""
        await self._try_send(proto.error(code, message))
        await self._t.close(code=1008)
