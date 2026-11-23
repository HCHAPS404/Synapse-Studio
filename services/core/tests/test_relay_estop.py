"""E4.6 advisory E-STOP and sim-control routing (twin/simulated only), with audit.

ADVISORY ONLY. The hardwired E-STOP is the authority; these messages never trip, release or
disable it. The relay forwards an advisory request to the daemon and answers with the daemon's
reported `estop_state`, or `unknown` when no daemon is connected — never a decided state.
"""

from __future__ import annotations

import json
from datetime import timedelta

from conftest import needs_db
from fastapi import FastAPI
from fastapi.testclient import TestClient

from synapse_core.relay.session import SessionTimeouts
from synapse_core.settings import Settings, get_settings

pytestmark = needs_db

_SNAPSHOT = {
    "v": 1,
    "type": "cell_snapshot",
    "cell": "bottling",
    "description": {
        "name": "Bottling line",
        "devices": [
            {"id": "plc1", "kind": "generic-plc", "simulated": True},
            {"id": "mcu1", "kind": "generic-mcu", "simulated": False},
        ],
        "links": [],
        "tags": [],
        "zones": [],
    },
}


async def _token(app: FastAPI, role: str, *, cell: str | None = None) -> str:
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.session import get_engine
    from synapse_core.services import service_tokens as svc

    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        _row, plaintext = await svc.issue_service_token(session, role, cell_id=cell)
    return plaintext


async def _operator_token(app: FastAPI) -> str:
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
        user = await users_svc.create_user(session, "op@test.local", "pw", "operator", cheap)
    token, _ = mint_access_token(
        user.id, "operator", secret=settings.token_secret(), ttl=timedelta(minutes=5)
    )
    return token


def _ws(app: FastAPI) -> TestClient:
    app.state.relay_timeouts = SessionTimeouts(
        hello_timeout_s=1.0, idle_ping_s=5.0, silence_close_s=10.0
    )
    return TestClient(app)


def _hello(client: str, **extra: object) -> str:
    frame = {"v": 1, "type": "hello", "client": client, "protocol": 1}
    frame.update(extra)
    return json.dumps(frame)


async def test_advisory_estop_without_daemon_answers_unknown(app: FastAPI) -> None:
    op = await _operator_token(app)
    with _ws(app) as c, c.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello("web", token=op))
        assert ws.receive_json()["roles"] == ["operator"]
        ws.send_text(
            json.dumps(
                {
                    "v": 1,
                    "type": "advisory_estop_request",
                    "cell": "bottling",
                    "reason": "spill",
                    "id": "r-5",
                }
            )
        )
        reply = ws.receive_json()
        assert reply["type"] == "estop_state"
        assert reply["state"] == "unknown"  # relay never decides a state
        assert reply["id"] == "r-5"


async def test_advisory_estop_is_audited(app: FastAPI) -> None:
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.session import get_engine
    from synapse_core.services import audit

    op = await _operator_token(app)
    with _ws(app) as c, c.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello("web", token=op))
        ws.receive_json()
        ws.send_text(
            json.dumps({"v": 1, "type": "advisory_estop_request", "cell": "bottling", "id": "r-1"})
        )
        assert ws.receive_json()["type"] == "estop_state"

    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        rows, _ = await audit.query(session, action="estop.advise")
    assert len(rows) == 1
    assert rows[0].result == "forwarded" and rows[0].target == "bottling"


async def test_advisory_estop_forwarded_to_daemon(app: FastAPI) -> None:
    op = await _operator_token(app)
    daemon = await _token(app, "daemon", cell="bottling")
    with _ws(app) as c:
        with c.websocket_connect("/v1/relay") as dws, c.websocket_connect("/v1/relay") as ows:
            # The daemon connects and publishes a frame so the hub knows it serves the cell.
            dws.send_text(_hello("daemon", token=daemon))
            assert dws.receive_json()["type"] == "welcome"
            dws.send_text(
                json.dumps({"v": 1, "type": "deployment_status", "cell": "bottling", "devices": []})
            )
            ows.send_text(_hello("web", token=op))
            assert ows.receive_json()["type"] == "welcome"
            ows.send_text(
                json.dumps(
                    {"v": 1, "type": "advisory_estop_request", "cell": "bottling", "id": "r-7"}
                )
            )
            # The daemon receives the forwarded request (the relay does not answer itself).
            forwarded = dws.receive_json()
            assert forwarded["type"] == "advisory_estop_request" and forwarded["id"] == "r-7"


async def test_guest_cannot_send_advisory_estop(app: FastAPI) -> None:
    with _ws(app) as c, c.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello("web"))  # no token -> guest
        assert ws.receive_json()["roles"] == ["guest"]
        ws.send_text(
            json.dumps({"v": 1, "type": "advisory_estop_request", "cell": "bottling", "id": "g"})
        )
        err = ws.receive_json()
        assert err["type"] == "error" and err["code"] == "forbidden"


async def test_sim_control_routes_to_twin_and_refuses_real_device(app: FastAPI) -> None:
    op = await _operator_token(app)
    twin = await _token(app, "twin")
    with _ws(app) as c:
        with c.websocket_connect("/v1/relay") as tws, c.websocket_connect("/v1/relay") as ows:
            tws.send_text(_hello("twin", token=twin))
            assert tws.receive_json()["type"] == "welcome"
            # The twin publishes the cell snapshot so the relay knows which devices are simulated.
            tws.send_text(json.dumps(_SNAPSHOT))

            ows.send_text(_hello("web", token=op))
            assert ows.receive_json()["type"] == "welcome"

            # sim_start reaches the twin and is acked.
            ows.send_text(json.dumps({"v": 1, "type": "sim_start", "cell": "bottling", "id": "s1"}))
            assert tws.receive_json()["type"] == "sim_start"
            assert ows.receive_json() == {"v": 1, "type": "ack", "id": "s1", "request": "sim_start"}

            # sim_set_input on the real device mcu1 is refused (unknown_target), never forwarded.
            ows.send_text(
                json.dumps(
                    {
                        "v": 1,
                        "type": "sim_set_input",
                        "cell": "bottling",
                        "device": "mcu1",
                        "name": "x",
                        "value": True,
                        "id": "s2",
                    }
                )
            )
            err = ows.receive_json()
            assert err["type"] == "error" and err["code"] == "unknown_target" and err["id"] == "s2"


async def test_sim_control_without_twin_is_not_ready(app: FastAPI) -> None:
    op = await _operator_token(app)
    with _ws(app) as c, c.websocket_connect("/v1/relay") as ows:
        ows.send_text(_hello("web", token=op))
        ows.receive_json()
        ows.send_text(json.dumps({"v": 1, "type": "sim_pause", "cell": "bottling", "id": "p"}))
        err = ows.receive_json()
        assert err["type"] == "error" and err["code"] == "not_ready"
