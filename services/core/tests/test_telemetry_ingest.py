"""E4.4 telemetry ingest: frame→rows parsing, the storage cap, and persistence from the relay."""

from __future__ import annotations

import json

from conftest import needs_db
from fastapi import FastAPI
from fastapi.testclient import TestClient

from synapse_core.relay.session import SessionTimeouts
from synapse_core.settings import get_settings
from synapse_core.telemetry.ingest import _rows_from_frame

# A valid frame (as it would arrive on the wire, after schema validation).
_FRAME = {
    "v": 1,
    "type": "telemetry",
    "cell": "bottling",
    "samples": [
        {"id": "plc1.level", "t_ns": 1_000, "value": 42.5, "quality": "good"},
        {"id": "plc1.mode", "t_ns": 1_000, "value": "auto", "quality": "stale"},
        {"id": "no_dot", "t_ns": 1_000, "value": 1, "quality": "good"},  # skipped: no device.tag
        {"id": "mcu1.count", "t_ns": 1_000, "value": 12, "quality": "bad"},
    ],
}


def test_rows_from_frame_splits_device_and_tag() -> None:
    rows = _rows_from_frame(_FRAME, simulated=True)
    assert len(rows) == 3  # the id without a dot is skipped
    by_tag = {r["tag"]: r for r in rows}
    assert by_tag["level"]["device"] == "plc1" and by_tag["level"]["value"] == 42.5
    assert by_tag["mode"]["quality"] == "stale"
    assert by_tag["count"]["device"] == "mcu1" and by_tag["count"]["quality"] == "bad"
    assert all(r["cell_id"] == "bottling" and r["simulated"] is True for r in rows)


def test_rows_from_frame_normalises_unknown_quality() -> None:
    # Defensive: even if an out-of-enum quality reached the ingest, it is stored as `bad`.
    frame = {"cell": "c", "samples": [{"id": "d.t", "t_ns": 1, "value": 1, "quality": "weird"}]}
    assert _rows_from_frame(frame, simulated=False)[0]["quality"] == "bad"


async def _twin_token(app: FastAPI) -> str:
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.session import get_engine
    from synapse_core.services import service_tokens as svc

    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        _row, plaintext = await svc.issue_service_token(session, "twin")
    return plaintext


@needs_db
async def test_twin_telemetry_is_persisted_simulated(app: FastAPI) -> None:
    from sqlalchemy import select
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.models import TelemetrySample
    from synapse_core.db.session import get_engine

    token = await _twin_token(app)
    app.state.relay_timeouts = SessionTimeouts(
        hello_timeout_s=1.0, idle_ping_s=5.0, silence_close_s=10.0
    )
    with TestClient(app) as c, c.websocket_connect("/v1/relay") as ws:
        ws.send_text(
            json.dumps({"v": 1, "type": "hello", "client": "twin", "protocol": 1, "token": token})
        )
        assert ws.receive_json()["type"] == "welcome"
        ws.send_text(json.dumps(_FRAME))
        ws.send_text(json.dumps({"v": 1, "type": "ping", "id": 1, "t_ns": 1}))
        reply = ws.receive_json()
        assert reply["type"] == "pong", reply

    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        rows = (await session.execute(select(TelemetrySample))).scalars().all()
    assert len(rows) == 3
    assert {r.device for r in rows} == {"plc1", "mcu1"}
    assert all(r.simulated is True for r in rows)


@needs_db
async def test_storage_cap_drops_new_samples(app: FastAPI) -> None:
    from sqlalchemy import func, select
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.models import TelemetrySample
    from synapse_core.db.session import get_engine
    from synapse_core.telemetry.ingest import ingest_telemetry

    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        first = await ingest_telemetry(session, _FRAME, simulated=True, max_rows=100)
        assert first.accepted == 3 and first.dropped == 0
        # Cap below the current count: the next batch is dropped entirely.
        second = await ingest_telemetry(session, _FRAME, simulated=True, max_rows=2)
        assert second.accepted == 0 and second.dropped == 3
        total = (
            await session.execute(select(func.count()).select_from(TelemetrySample))
        ).scalar_one()
    assert total == 3
