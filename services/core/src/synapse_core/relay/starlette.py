"""Adapt a Starlette/FastAPI `WebSocket` to the relay `Transport` protocol (E4.1)."""

from __future__ import annotations

from typing import Any

from starlette.websockets import WebSocket, WebSocketDisconnect, WebSocketState

from synapse_core.relay.session import TransportClosed


class StarletteTransport:
    """Wraps a `WebSocket`, turning a disconnect into `TransportClosed`.

    Binary frames are rejected by the relay (text only); `receive_text` raises on a
    disconnect so the session loop ends cleanly.
    """

    def __init__(self, websocket: WebSocket) -> None:
        self._ws = websocket

    async def accept(self) -> None:
        await self._ws.accept()

    async def receive_text(self) -> str:
        try:
            return await self._ws.receive_text()
        except WebSocketDisconnect as exc:
            raise TransportClosed() from exc
        except (RuntimeError, KeyError) as exc:
            # Starlette raises RuntimeError after a disconnect, KeyError for a binary frame.
            raise TransportClosed() from exc

    async def send_json(self, data: dict[str, Any]) -> None:
        try:
            await self._ws.send_json(data)
        except (WebSocketDisconnect, RuntimeError) as exc:
            raise TransportClosed() from exc

    async def close(self, code: int = 1000) -> None:
        if self._ws.application_state != WebSocketState.DISCONNECTED:
            try:
                await self._ws.close(code=code)
            except RuntimeError:
                pass
