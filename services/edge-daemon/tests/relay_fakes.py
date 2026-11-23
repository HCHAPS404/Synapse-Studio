"""In-memory relay peer used by the daemon tests (no network)."""

from __future__ import annotations

import asyncio
import json
from typing import Any

from synapse_daemon.relay.client import Connection, ConnectionClosed


class FakeConnection(Connection):
    """An in-memory relay peer.

    `outbox` collects frames the client sent (as parsed dicts). `feed` queues frames the
    client will receive. `drop` makes the next `recv` raise `ConnectionClosed`, so a test
    can exercise reconnect without a network.
    """

    def __init__(self) -> None:
        self.outbox: list[dict[str, Any]] = []
        self._incoming: asyncio.Queue[str | None] = asyncio.Queue()
        self.closed = False

    async def send(self, data: str) -> None:
        if self.closed:
            raise ConnectionClosed("closed")
        self.outbox.append(json.loads(data))

    async def recv(self) -> str:
        item = await self._incoming.get()
        if item is None:
            raise ConnectionClosed("dropped")
        return item

    async def close(self) -> None:
        self.closed = True

    def feed(self, frame: dict[str, Any]) -> None:
        self._incoming.put_nowait(json.dumps(frame))

    def drop(self) -> None:
        self._incoming.put_nowait(None)
