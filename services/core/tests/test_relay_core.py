"""E4.1 relay core: handshake, validation, role table, error codes, timeouts.

These tests drive the WebSocket with Starlette's in-process `TestClient` (no network) and a
guest (no token), so they need no database. Token-based role tests live with the DB suite.
The timeouts are shortened through `app.state.relay_timeouts` so the ping/close path is fast.
"""

from __future__ import annotations

import json
from collections.abc import Iterator

import pytest
from fastapi.testclient import TestClient
from pydantic import SecretStr
from starlette.websockets import WebSocketDisconnect

from synapse_core.main import create_app
from synapse_core.relay.session import SessionTimeouts
from synapse_core.settings import Settings, get_settings


def _hello(**extra: object) -> str:
    frame = {"v": 1, "type": "hello", "client": "web", "protocol": 1}
    frame.update(extra)
    return json.dumps(frame)


@pytest.fixture
def client() -> Iterator[TestClient]:
    """A DB-free app (no database configured); the relay guest path needs none."""
    app = create_app()
    app.dependency_overrides[get_settings] = lambda: Settings(database_url=None)
    app.state.relay_timeouts = SessionTimeouts(
        hello_timeout_s=0.3, idle_ping_s=0.2, silence_close_s=0.4
    )
    with TestClient(app) as test_client:
        yield test_client


def test_hello_gets_welcome_with_guest_role(client: TestClient) -> None:
    with client.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello(capabilities=["replay"]))
        welcome = ws.receive_json()
        assert welcome["type"] == "welcome"
        assert welcome["roles"] == ["guest"]
        assert welcome["protocol"] == 1
        assert welcome["capabilities"] == ["replay"]
        assert isinstance(welcome["session"], str) and welcome["session"].startswith("s-")


def test_first_frame_must_be_hello(client: TestClient) -> None:
    with client.websocket_connect("/v1/relay") as ws:
        ws.send_text(json.dumps({"v": 1, "type": "ping", "id": 1, "t_ns": 1}))
        err = ws.receive_json()
        assert err["type"] == "error"
        assert err["code"] == "bad_message"
        with pytest.raises(WebSocketDisconnect):
            ws.receive_json()


def test_bad_json_before_welcome_is_fatal(client: TestClient) -> None:
    with client.websocket_connect("/v1/relay") as ws:
        ws.send_text("{not json")
        err = ws.receive_json()
        assert err["type"] == "error" and err["code"] == "bad_message"
        with pytest.raises(WebSocketDisconnect):
            ws.receive_json()


def test_unsupported_version_is_fatal(client: TestClient) -> None:
    with client.websocket_connect("/v1/relay") as ws:
        ws.send_text(json.dumps({"v": 2, "type": "hello", "client": "web", "protocol": 1}))
        err = ws.receive_json()
        assert err["type"] == "error" and err["code"] == "unsupported_version"
        with pytest.raises(WebSocketDisconnect):
            ws.receive_json()


def test_invalid_token_is_unauthenticated_no_guest_fallback(client: TestClient) -> None:
    # A three-part string is treated as a user JWT; with no DB it fails to resolve.
    app = create_app()
    app.dependency_overrides[get_settings] = lambda: Settings(
        database_url=SecretStr("postgresql://user:pw@localhost:1/none")
    )
    with TestClient(app) as c, c.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello(token="aaa.bbb.ccc"))
        err = ws.receive_json()
        assert err["type"] == "error" and err["code"] == "unauthenticated"
        with pytest.raises(WebSocketDisconnect):
            ws.receive_json()


def test_ping_is_answered_with_pong(client: TestClient) -> None:
    with client.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello())
        ws.receive_json()  # welcome
        ws.send_text(json.dumps({"v": 1, "type": "ping", "id": 7, "t_ns": 1000}))
        pong = ws.receive_json()
        assert pong["type"] == "pong"
        assert pong["id"] == 7
        assert pong["t_ping_ns"] == 1000
        assert isinstance(pong["t_ns"], int)


def test_guest_cannot_send_sim_start_forbidden(client: TestClient) -> None:
    with client.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello())
        ws.receive_json()  # welcome
        ws.send_text(json.dumps({"v": 1, "type": "sim_start", "cell": "bottling", "id": "r-2"}))
        err = ws.receive_json()
        assert err["type"] == "error"
        assert err["code"] == "forbidden"
        assert err["id"] == "r-2"


def test_subscribe_is_acked(client: TestClient) -> None:
    with client.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello())
        ws.receive_json()  # welcome
        ws.send_text(
            json.dumps(
                {"v": 1, "type": "subscribe", "cell": "bottling", "channels": ["cell"], "id": "r-1"}
            )
        )
        ack = ws.receive_json()
        assert ack["type"] == "ack" and ack["id"] == "r-1" and ack["request"] == "subscribe"


def test_bad_message_after_welcome_is_not_fatal(client: TestClient) -> None:
    with client.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello())
        ws.receive_json()  # welcome
        ws.send_text(json.dumps({"v": 1, "type": "subscribe"}))  # missing cell/channels
        err = ws.receive_json()
        assert err["type"] == "error" and err["code"] == "bad_message"
        # Session stays open: a ping is still answered.
        ws.send_text(json.dumps({"v": 1, "type": "ping", "id": 1, "t_ns": 1}))
        assert ws.receive_json()["type"] == "pong"


def test_server_pings_on_idle_then_closes_on_silence(client: TestClient) -> None:
    with client.websocket_connect("/v1/relay") as ws:
        ws.send_text(_hello())
        ws.receive_json()  # welcome
        ping = ws.receive_json()  # server ping after the idle window
        assert ping["type"] == "ping"
        with pytest.raises(WebSocketDisconnect):
            ws.receive_json()  # closed after the silence budget
