"""E4.3 daemon pairing: issue, claim, heartbeat/deployment_status, unpair (needs a database)."""

from __future__ import annotations

import json
from datetime import UTC, datetime, timedelta

from conftest import needs_db
from fastapi import FastAPI
from fastapi.testclient import TestClient
from httpx import AsyncClient

from synapse_core.relay.session import SessionTimeouts
from synapse_core.settings import get_settings

pytestmark = needs_db


def _hello(**extra: object) -> str:
    frame = {"v": 1, "type": "hello", "client": "daemon", "protocol": 1}
    frame.update(extra)
    return json.dumps(frame)


async def test_issue_requires_engineer(client: AsyncClient) -> None:
    resp = await client.post("/v1/pairing", json={"cell_id": "bottling"})
    assert resp.status_code == 403
    assert resp.json()["code"] == "NE1106"


async def test_pair_claim_and_token_authenticates(engineer_client: AsyncClient) -> None:
    issued = await engineer_client.post("/v1/pairing", json={"cell_id": "bottling"})
    assert issued.status_code == 201, issued.text
    code = issued.json()["code"]
    assert len(code) == 8

    # The claim route is public (the daemon has no token yet).
    claim = await engineer_client.post(
        "/v1/pairing/claim",
        json={
            "code": code,
            "daemon_name": "lab-daemon",
            "version": "0.1.0",
            "devices": [{"name": "mcu1", "kind": "generic-mcu"}],
        },
        headers={"Authorization": ""},
    )
    assert claim.status_code == 201, claim.text
    body = claim.json()
    assert body["cell_id"] == "bottling"
    token = body["token"]
    assert token.startswith("svc_")

    # The daemon is listed, offline until a heartbeat, with its device.
    listed = await engineer_client.get("/v1/daemons")
    assert listed.status_code == 200
    daemons = listed.json()["items"]
    assert len(daemons) == 1
    assert daemons[0]["online"] is False
    assert daemons[0]["devices"][0]["name"] == "mcu1"
    assert daemons[0]["devices"][0]["simulated"] is False


async def test_code_cannot_be_reused(engineer_client: AsyncClient) -> None:
    code = (await engineer_client.post("/v1/pairing", json={"cell_id": "c"})).json()["code"]
    first = await engineer_client.post(
        "/v1/pairing/claim", json={"code": code, "daemon_name": "d1"}
    )
    assert first.status_code == 201
    second = await engineer_client.post(
        "/v1/pairing/claim", json={"code": code, "daemon_name": "d2"}
    )
    assert second.status_code == 409
    assert second.json()["code"] == "NE1111"


async def test_unknown_code_is_ne1111(engineer_client: AsyncClient) -> None:
    resp = await engineer_client.post(
        "/v1/pairing/claim", json={"code": "ZZZZZZZZ", "daemon_name": "d"}
    )
    assert resp.status_code == 409 and resp.json()["code"] == "NE1111"


async def test_heartbeat_marks_online_then_timeout(
    app: FastAPI, engineer_client: AsyncClient
) -> None:
    import uuid

    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.models import Daemon
    from synapse_core.db.session import get_engine
    from synapse_core.services import pairing as svc

    code = (await engineer_client.post("/v1/pairing", json={"cell_id": "c"})).json()["code"]
    claim = await engineer_client.post("/v1/pairing/claim", json={"code": code, "daemon_name": "d"})
    daemon_id = claim.json()["daemon_id"]

    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        daemon = await session.get(Daemon, uuid.UUID(daemon_id))
        assert daemon is not None
        ok = await svc.record_heartbeat(session, daemon.service_token_id)
        assert ok is True

    fresh = (await engineer_client.get("/v1/daemons")).json()["items"][0]
    assert fresh["online"] is True
    assert fresh["last_heartbeat"] is not None
    # A heartbeat older than the timeout is offline.
    assert svc.is_online(datetime.now(UTC) - timedelta(seconds=30)) is False


async def test_unpair_revokes_token(app: FastAPI, engineer_client: AsyncClient) -> None:
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.session import get_engine
    from synapse_core.errors import ServiceError
    from synapse_core.services import service_tokens as tok_svc

    code = (await engineer_client.post("/v1/pairing", json={"cell_id": "c"})).json()["code"]
    claim = await engineer_client.post("/v1/pairing/claim", json={"code": code, "daemon_name": "d"})
    daemon_id = claim.json()["daemon_id"]
    token = claim.json()["token"]

    resp = await engineer_client.delete(f"/v1/daemons/{daemon_id}")
    assert resp.status_code == 204

    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        try:
            await tok_svc.principal_from_service_token(session, token)
            raise AssertionError("revoked token should not authenticate")
        except ServiceError as exc:
            assert exc.code.value == "NE1105"
    # Unpairing again is a 404.
    assert (await engineer_client.delete(f"/v1/daemons/{daemon_id}")).status_code == 404


async def test_daemon_deployment_status_updates_heartbeat_over_ws(app: FastAPI) -> None:
    """A daemon that sends `deployment_status` over the relay has its heartbeat recorded."""
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.models import Daemon
    from synapse_core.db.session import get_engine
    from synapse_core.services import pairing as svc

    settings = app.dependency_overrides[get_settings]()
    engine = get_engine(settings)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        _row, code = await svc.issue_pairing_code(session, "bottling")
        daemon, token = await svc.claim_pairing(session, code, "d")
        daemon_id = daemon.id

    app.state.relay_timeouts = SessionTimeouts(
        hello_timeout_s=1.0, idle_ping_s=5.0, silence_close_s=10.0
    )
    with TestClient(app) as c, c.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello(token=token))
        assert ws.receive_json()["type"] == "welcome"
        ws.send_text(
            json.dumps(
                {
                    "v": 1,
                    "type": "deployment_status",
                    "cell": "bottling",
                    "devices": [{"device": "mcu1", "online": True}],
                }
            )
        )
        # Give the heartbeat write a moment by issuing a ping round trip.
        ws.send_text(json.dumps({"v": 1, "type": "ping", "id": 1, "t_ns": 1}))
        assert ws.receive_json()["type"] == "pong"

    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        daemon = await session.get(Daemon, daemon_id)
        assert daemon is not None and daemon.last_heartbeat is not None
