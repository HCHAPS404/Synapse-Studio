"""E4.2 integration: subscriptions, snapshot cache replay, fan-out and the control-message
rate limit over in-process WebSockets (needs a database for the twin token).
"""

from __future__ import annotations

import json

from conftest import needs_db
from fastapi import FastAPI
from fastapi.testclient import TestClient

from synapse_core.relay.session import SessionTimeouts
from synapse_core.settings import get_settings

pytestmark = needs_db


def _hello(**extra: object) -> str:
    frame = {"v": 1, "type": "hello", "client": "web", "protocol": 1}
    frame.update(extra)
    return json.dumps(frame)


async def _twin_token(app: FastAPI) -> str:
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.session import get_engine
    from synapse_core.services import service_tokens as svc

    settings = app.dependency_overrides[get_settings]()
    engine = get_engine(settings)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        _row, plaintext = await svc.issue_service_token(session, "twin")
    return plaintext


def _ws_client(app: FastAPI) -> TestClient:
    app.state.relay_timeouts = SessionTimeouts(
        hello_timeout_s=1.0, idle_ping_s=5.0, silence_close_s=10.0
    )
    return TestClient(app)


async def test_twin_telemetry_fans_out_to_a_subscriber(app: FastAPI) -> None:
    token = await _twin_token(app)
    with _ws_client(app) as c:
        with c.websocket_connect("/v1/relay") as sub, c.websocket_connect("/v1/relay") as twin:
            sub.send_text(_hello())
            assert sub.receive_json()["type"] == "welcome"
            sub.send_text(
                json.dumps(
                    {"v": 1, "type": "subscribe", "cell": "bottling", "channels": ["telemetry"]}
                )
            )
            twin.send_text(_hello(client="twin", token=token))
            assert twin.receive_json()["type"] == "welcome"
            twin.send_text(
                json.dumps(
                    {
                        "v": 1,
                        "type": "telemetry",
                        "cell": "bottling",
                        "samples": [
                            {"id": "plc1.level", "t_ns": 1, "value": 42.5, "quality": "good"}
                        ],
                    }
                )
            )
            got = sub.receive_json()
            assert got["type"] == "telemetry"
            assert got["samples"][0]["id"] == "plc1.level"


async def test_cached_snapshot_is_sent_after_subscribe(app: FastAPI) -> None:
    token = await _twin_token(app)
    with _ws_client(app) as c:
        with c.websocket_connect("/v1/relay") as twin:
            twin.send_text(_hello(client="twin", token=token))
            assert twin.receive_json()["type"] == "welcome"
            snapshot = {
                "v": 1,
                "type": "cell_snapshot",
                "cell": "bottling",
                "description": {
                    "name": "Bottling line",
                    "devices": [],
                    "links": [],
                    "tags": [],
                    "zones": [],
                },
            }
            twin.send_text(json.dumps(snapshot))
            # A client that subscribes afterwards receives the cached snapshot.
            with c.websocket_connect("/v1/relay") as sub:
                sub.send_text(_hello())
                assert sub.receive_json()["type"] == "welcome"
                sub.send_text(
                    json.dumps(
                        {
                            "v": 1,
                            "type": "subscribe",
                            "cell": "bottling",
                            "channels": ["cell"],
                            "id": "r-1",
                        }
                    )
                )
                first = sub.receive_json()
                assert first["type"] == "ack" and first["id"] == "r-1"
                second = sub.receive_json()
                assert second["type"] == "cell_snapshot" and second["cell"] == "bottling"


async def test_control_message_rate_limit(app: FastAPI) -> None:
    # A low control rate so the burst is exhausted quickly; the next control message is
    # rate_limited. ping counts as control (only pong is exempt).
    app.state.relay_timeouts = SessionTimeouts(
        hello_timeout_s=1.0, idle_ping_s=5.0, silence_close_s=10.0
    )
    app.state.relay_control_rate = 5.0  # small burst so the limit is hit deterministically
    with TestClient(app) as c, c.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello())
        assert ws.receive_json()["type"] == "welcome"
        # Burst is 5; sending more quickly must produce a rate_limited error.
        refused = False
        for i in range(12):
            ws.send_text(json.dumps({"v": 1, "type": "ping", "id": i, "t_ns": i}))
            reply = ws.receive_json()
            if reply["type"] == "error" and reply["code"] == "rate_limited":
                refused = True
                break
        assert refused, "expected a rate_limited error after the control burst"
