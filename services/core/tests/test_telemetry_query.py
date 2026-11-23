"""E4.5 `GET /v1/telemetry`: filtering, limit, and guest-sees-only-virtual (needs a database)."""

from __future__ import annotations

from conftest import needs_db
from fastapi import FastAPI
from httpx import AsyncClient

from synapse_core.settings import get_settings

pytestmark = needs_db


async def _insert(app: FastAPI, rows: list[dict]) -> None:
    from sqlalchemy import insert
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.models import TelemetrySample
    from synapse_core.db.session import get_engine

    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        await session.execute(insert(TelemetrySample), rows)
        await session.commit()


def _row(device: str, tag: str, t_ns: int, value: object, simulated: bool) -> dict:
    return {
        "cell_id": "bottling",
        "device": device,
        "tag": tag,
        "t_ns": t_ns,
        "value": value,
        "quality": "good",
        "simulated": simulated,
    }


async def test_user_reads_all_devices(app: FastAPI, engineer_client: AsyncClient) -> None:
    await _insert(
        app,
        [
            _row("plc1", "level", 10, 1.0, True),
            _row("mcu1", "temp", 20, 2.0, False),
        ],
    )
    resp = await engineer_client.get("/v1/telemetry", params={"cell": "bottling"})
    assert resp.status_code == 200, resp.text
    items = resp.json()["items"]
    assert {i["device"] for i in items} == {"plc1", "mcu1"}
    # Newest first.
    assert items[0]["t_ns"] >= items[-1]["t_ns"]


async def test_filter_by_device_tag_and_range(app: FastAPI, engineer_client: AsyncClient) -> None:
    await _insert(
        app,
        [
            _row("plc1", "level", 10, 1.0, True),
            _row("plc1", "level", 100, 2.0, True),
            _row("plc1", "mode", 50, "auto", True),
        ],
    )
    resp = await engineer_client.get(
        "/v1/telemetry",
        params={"cell": "bottling", "device": "plc1", "tag": "level", "from_ns": 50, "to_ns": 200},
    )
    items = resp.json()["items"]
    assert len(items) == 1 and items[0]["t_ns"] == 100


async def test_guest_sees_only_virtual_devices(app: FastAPI, client: AsyncClient) -> None:
    await _insert(
        app,
        [
            _row("plc1", "level", 10, 1.0, True),
            _row("mcu1", "temp", 20, 2.0, False),
        ],
    )
    guest = (await client.post("/v1/guest")).json()["token"]
    resp = await client.get(
        "/v1/telemetry",
        params={"cell": "bottling"},
        headers={"Authorization": f"Bearer {guest}"},
    )
    assert resp.status_code == 200, resp.text
    items = resp.json()["items"]
    assert items and all(i["simulated"] is True for i in items)
    assert all(i["device"] == "plc1" for i in items)


async def test_anonymous_is_denied(client: AsyncClient) -> None:
    resp = await client.get("/v1/telemetry", params={"cell": "bottling"})
    assert resp.status_code == 403 and resp.json()["code"] == "NE1106"
