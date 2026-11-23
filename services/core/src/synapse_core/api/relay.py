"""The relay WebSocket endpoint `WS /v1/relay` (E4.1).

One process-wide `Hub` lives on `app.state.relay_hub`. Each connection runs a `RelaySession`
that performs the handshake, validates frames against the JSON Schema and enforces the role
table. Authentication reuses the E3 token resolution through a session maker.
"""

from __future__ import annotations

import uuid
from typing import Annotated, Any

from fastapi import APIRouter, Depends, WebSocket
from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core.db.session import get_engine
from synapse_core.relay.hub import Hub
from synapse_core.relay.session import RelaySession, SessionTimeouts
from synapse_core.relay.starlette import StarletteTransport
from synapse_core.services.pairing import record_heartbeat
from synapse_core.settings import Settings, get_settings
from synapse_core.telemetry.ingest import ingest_telemetry

router = APIRouter()


def get_hub(websocket: WebSocket) -> Hub:
    """The process-wide relay hub, created on first use (no `await`, so no race)."""
    hub: Hub | None = getattr(websocket.app.state, "relay_hub", None)
    if hub is None:
        hub = Hub()
        websocket.app.state.relay_hub = hub
    return hub


@router.websocket("/v1/relay")
async def relay(
    websocket: WebSocket,
    settings: Annotated[Settings, Depends(get_settings)],
) -> None:
    """Runs one relay session on the connection."""
    engine = get_engine(settings)
    sessions = async_sessionmaker(engine, expire_on_commit=False) if engine is not None else None
    timeouts: SessionTimeouts | None = getattr(websocket.app.state, "relay_timeouts", None)
    control_rate: float = getattr(websocket.app.state, "relay_control_rate", 20.0)

    async def heartbeat(principal_key: str) -> None:
        """Records a daemon heartbeat from its `deployment_status` frame (best effort)."""
        if sessions is None or not principal_key.startswith("service:"):
            return
        try:
            token_id = uuid.UUID(principal_key.split(":", 1)[1])
        except ValueError:
            return
        async with sessions() as db:
            await record_heartbeat(db, token_id)

    async def telemetry(frame: dict[str, Any], simulated: bool) -> None:
        """Persists a telemetry frame into the raw table (best effort, off the fan-out path)."""
        if sessions is None:
            return
        async with sessions() as db:
            await ingest_telemetry(
                db, frame, simulated=simulated, max_rows=settings.telemetry_max_rows
            )

    session = RelaySession(
        StarletteTransport(websocket),
        get_hub(websocket),
        settings,
        sessions,
        timeouts=timeouts,
        control_rate=control_rate,
        on_heartbeat=heartbeat,
        on_telemetry=telemetry,
    )
    await session.run()
