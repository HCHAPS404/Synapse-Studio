"""The daemon's relay client: connect, `hello`/`welcome`, send frames, reconnect.

The client is transport-agnostic through a `Connection` protocol so tests drive it with an
in-memory fake and CI needs no network. The default transport is a `websockets` client. On
any disconnect the client reconnects with exponential backoff between `reconnect_min_s` and
`reconnect_max_s`. It answers server `ping` with `pong` and ignores unknown frames (the
protocol evolves additively).
"""

from __future__ import annotations

import asyncio
import contextlib
import json
import logging
import time
from collections.abc import Awaitable, Callable
from typing import Any, Protocol

from synapse_daemon.relay import frames

log = logging.getLogger(__name__)

# A handler the daemon registers for inbound frames of a given `type`.
type FrameHandler = Callable[[dict[str, Any]], Awaitable[None]]


class Connection(Protocol):
    """One open WebSocket. `recv` raises `ConnectionClosed` when the peer goes away."""

    async def send(self, data: str) -> None: ...
    async def recv(self) -> str: ...
    async def close(self) -> None: ...


class ConnectionClosed(Exception):
    """The transport closed; the client should reconnect."""


# A factory that opens one connection to the relay URL.
type Connect = Callable[[str], Awaitable[Connection]]


class RelayClient:
    """Keeps a session to the relay alive and lets the daemon publish frames."""

    def __init__(
        self,
        url: str,
        token: str | None,
        *,
        client_version: str,
        connect: Connect,
        capabilities: list[str] | None = None,
        reconnect_min_s: float = 1.0,
        reconnect_max_s: float = 30.0,
    ) -> None:
        self._url = url
        self._token = token
        self._client_version = client_version
        self._connect = connect
        self._capabilities = capabilities or []
        self._reconnect_min = reconnect_min_s
        self._reconnect_max = reconnect_max_s
        self._conn: Connection | None = None
        self._handlers: dict[str, FrameHandler] = {}
        self._connected = asyncio.Event()
        self._stop = asyncio.Event()
        self.session: str | None = None
        self.roles: list[str] = []

    def on(self, frame_type: str, handler: FrameHandler) -> None:
        """Register a handler for one inbound frame type."""
        self._handlers[frame_type] = handler

    @property
    def is_connected(self) -> bool:
        return self._connected.is_set()

    async def send(self, frame: dict[str, Any]) -> None:
        """Send one frame. Does nothing when not connected (telemetry is a view, not a log)."""
        conn = self._conn
        if conn is None or not self._connected.is_set():
            return
        try:
            await conn.send(json.dumps(frame, separators=(",", ":")))
        except Exception as exc:  # the connection died mid-send; the run loop reconnects
            raise ConnectionClosed(str(exc)) from exc

    async def run(self) -> None:
        """Connect and keep the session alive until `stop()`, reconnecting on loss."""
        backoff = self._reconnect_min
        while not self._stop.is_set():
            try:
                await self._session_once()
                backoff = self._reconnect_min
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                log.warning("relay session ended: %s", exc)
            finally:
                self._connected.clear()
                self._conn = None
            if self._stop.is_set():
                break
            await self._sleep_or_stop(backoff)
            backoff = min(backoff * 2, self._reconnect_max)

    async def _session_once(self) -> None:
        conn = await self._connect(self._url)
        self._conn = conn
        try:
            await conn.send(
                json.dumps(
                    frames.hello(
                        token=self._token,
                        client_version=self._client_version,
                        capabilities=self._capabilities,
                    ),
                    separators=(",", ":"),
                )
            )
            welcome = json.loads(await conn.recv())
            if welcome.get("type") != "welcome":
                raise ConnectionClosed(f"expected welcome, got {welcome.get('type')}")
            self.session = welcome.get("session")
            self.roles = list(welcome.get("roles", []))
            self._connected.set()
            log.info("relay connected session=%s roles=%s", self.session, self.roles)
            await self._read_loop(conn)
        finally:
            with contextlib.suppress(Exception):
                await conn.close()

    async def _read_loop(self, conn: Connection) -> None:
        while not self._stop.is_set():
            raw = await conn.recv()
            try:
                frame = json.loads(raw)
            except json.JSONDecodeError:
                continue
            ftype = frame.get("type")
            if ftype == "ping":
                await self.send(frames.pong(frame.get("id", 0), frame.get("t_ns", 0), _now_ns()))
                continue
            handler = self._handlers.get(ftype)
            if handler is not None:
                await handler(frame)

    async def _sleep_or_stop(self, seconds: float) -> None:
        with contextlib.suppress(TimeoutError):
            await asyncio.wait_for(self._stop.wait(), timeout=seconds)

    async def wait_connected(self, within_s: float | None = None) -> None:
        """Block until the first `welcome` arrives (used by tests and startup)."""
        await asyncio.wait_for(self._connected.wait(), timeout=within_s)

    def stop(self) -> None:
        """Ask the run loop to finish; it closes the connection and returns."""
        self._stop.set()


def _now_ns() -> int:
    return time.time_ns()


async def websocket_connect(url: str) -> Connection:  # pragma: no cover - needs a network
    """Default transport: a `websockets` client wrapped in the `Connection` protocol."""
    import websockets

    ws = await websockets.connect(url, max_size=2**20)

    class _WS:
        async def send(self, data: str) -> None:
            await ws.send(data)

        async def recv(self) -> str:
            try:
                msg = await ws.recv()
            except Exception as exc:
                raise ConnectionClosed(str(exc)) from exc
            return msg if isinstance(msg, str) else msg.decode("utf-8")

        async def close(self) -> None:
            await ws.close()

    return _WS()
