"""E4.1: relay `hello.token` grants the right protocol roles (needs a database).

Reuses the E3 token kinds: a user JWT (operator), a service token (daemon). The operator may
send `sim_start`; the daemon may publish `telemetry`; a guest token stays `guest`.
"""

from __future__ import annotations

import json

import pytest
from conftest import needs_db
from fastapi import FastAPI
from fastapi.testclient import TestClient
from starlette.websockets import WebSocketDisconnect

from synapse_core.relay.session import SessionTimeouts
from synapse_core.settings import Settings, get_settings

pytestmark = needs_db


def _hello(**extra: object) -> str:
    frame = {"v": 1, "type": "hello", "client": "web", "protocol": 1}
    frame.update(extra)
    return json.dumps(frame)


async def _operator_token(app: FastAPI) -> str:
    from datetime import timedelta

    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.auth.tokens import mint_access_token
    from synapse_core.db.session import get_engine
    from synapse_core.services import users as users_svc

    settings = app.dependency_overrides[get_settings]()
    cheap = Settings(
        database_url=settings.database_url,
        argon2_time_cost=1,
        argon2_memory_cost_kib=8,
        argon2_parallelism=1,
    )
    engine = get_engine(settings)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        user = await users_svc.create_user(session, "operator@test.local", "pw", "operator", cheap)
    token, _ = mint_access_token(
        user.id, "operator", secret=settings.token_secret(), ttl=timedelta(minutes=5)
    )
    return token


async def _daemon_token(app: FastAPI) -> str:
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.session import get_engine
    from synapse_core.services import service_tokens as svc

    settings = app.dependency_overrides[get_settings]()
    engine = get_engine(settings)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        _row, plaintext = await svc.issue_service_token(session, "daemon", cell_id="bottling")
    return plaintext


def _ws_client(app: FastAPI) -> TestClient:
    app.state.relay_timeouts = SessionTimeouts(
        hello_timeout_s=1.0, idle_ping_s=2.0, silence_close_s=4.0
    )
    return TestClient(app)


async def test_operator_token_may_sim_start(app: FastAPI) -> None:
    token = await _operator_token(app)
    with _ws_client(app) as c, c.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello(token=token))
        welcome = ws.receive_json()
        assert welcome["roles"] == ["operator"]
        ws.send_text(json.dumps({"v": 1, "type": "sim_start", "cell": "bottling", "id": "r-2"}))
        assert ws.receive_json()["type"] == "ack"


async def test_daemon_token_grants_daemon_role(app: FastAPI) -> None:
    token = await _daemon_token(app)
    with _ws_client(app) as c, c.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello(client="daemon", token=token))
        welcome = ws.receive_json()
        assert welcome["roles"] == ["daemon"]
        # A daemon may not send a sim control message.
        ws.send_text(json.dumps({"v": 1, "type": "sim_start", "cell": "bottling", "id": "r-9"}))
        err = ws.receive_json()
        assert err["type"] == "error" and err["code"] == "forbidden"


async def test_revoked_service_token_is_unauthenticated(app: FastAPI) -> None:
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.session import get_engine
    from synapse_core.services import service_tokens as svc

    settings = app.dependency_overrides[get_settings]()
    engine = get_engine(settings)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        row, plaintext = await svc.issue_service_token(session, "daemon", cell_id="bottling")
        await svc.revoke_service_token(session, row.id)
    with _ws_client(app) as c, c.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello(client="daemon", token=plaintext))
        err = ws.receive_json()
        assert err["type"] == "error" and err["code"] == "unauthenticated"
        with pytest.raises(WebSocketDisconnect):
            ws.receive_json()
