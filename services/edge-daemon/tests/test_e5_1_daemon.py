"""E5.1: config, the simulated driver, the relay client with reconnect, pairing.""""""E5.1: config, the simulated driver, the relay client with reconnect, pairing."""

from __future__ import annotations

import asyncio
from typing import Any

import pytest
from relay_fakes import FakeConnection

from synapse_daemon.config import DaemonSettings
from synapse_daemon.drivers import DriverConfig, Quality, SimulatedDriver
from synapse_daemon.drivers.base import Driver, DriverError
from synapse_daemon.lan import run_mode
from synapse_daemon.pairing import PairingError, claim_pairing
from synapse_daemon.relay import frames
from synapse_daemon.relay.client import RelayClient

# --- config and run mode ---------------------------------------------------------------


def test_local_mode_when_no_token() -> None:
    s = DaemonSettings(relay_url="wss://x/v1/relay")  # no token
    assert s.is_local_mode() is True
    assert "local LAN mode" in run_mode(s).description


def test_cloud_mode_with_url_and_token() -> None:
    s = DaemonSettings(relay_url="wss://x/v1/relay", service_token="svc_abc")
    assert s.is_local_mode() is False
    assert "cloud relay" in run_mode(s).description


# --- simulated driver ------------------------------------------------------------------


async def test_simulated_driver_is_a_driver_and_reads_writes() -> None:
    drv = SimulatedDriver()
    assert isinstance(drv, Driver)
    cfg = DriverConfig(name="sim", params={"values": {"level": 42.5}}, writable=frozenset({"q"}))
    await drv.open(cfg)
    s = await drv.read("level")
    assert s.value == 42.5 and s.quality is Quality.GOOD
    await drv.write("q", True)
    assert (await drv.read("q")).value is True
    await drv.close()


async def test_simulated_driver_unknown_tag_is_bad() -> None:
    drv = SimulatedDriver()
    await drv.open(DriverConfig(name="sim"))
    s = await drv.read("missing")
    assert s.quality is Quality.BAD and s.value is None


async def test_simulated_driver_refuses_non_writable() -> None:
    drv = SimulatedDriver()
    await drv.open(DriverConfig(name="sim", writable=frozenset({"a"})))
    with pytest.raises(DriverError):
        await drv.write("b", 1)


# --- frame builders --------------------------------------------------------------------


def test_hello_frame_shape() -> None:
    f = frames.hello(token="svc_x", client_version="0.1.0", capabilities=["replay"])
    assert f["type"] == "hello" and f["client"] == "daemon" and f["token"] == "svc_x"


def test_hello_omits_absent_token() -> None:
    f = frames.hello(token=None, client_version="0.1.0")
    assert "token" not in f


def test_telemetry_omits_value_for_bad_sample() -> None:
    from synapse_daemon.drivers.base import Sample

    f = frames.telemetry("c", [Sample.bad("plc1.level", 5)])
    s0 = f["samples"][0]
    assert s0["quality"] == "bad" and "value" not in s0 and s0["id"] == "plc1.level"


# --- relay client ----------------------------------------------------------------------


def _client(conn: FakeConnection) -> RelayClient:
    async def connect(_: str) -> FakeConnection:
        return conn

    return RelayClient(
        "wss://x/v1/relay",
        "svc_token",
        client_version="0.1.0",
        connect=connect,
        reconnect_min_s=0.01,
        reconnect_max_s=0.02,
    )


async def test_relay_client_handshake_and_send(welcome_frame: dict[str, Any]) -> None:
    conn = FakeConnection()
    conn.feed(welcome_frame)
    client = _client(conn)
    task = asyncio.create_task(client.run())
    await client.wait_connected(within_s=1)
    assert client.session == "s-test" and client.roles == ["daemon"]
    assert conn.outbox[0]["type"] == "hello"
    await client.send(frames.deployment_status("bottling", [{"device": "mcu1", "online": True}]))
    await asyncio.sleep(0.01)
    assert conn.outbox[-1]["type"] == "deployment_status"
    client.stop()
    conn.drop()
    await asyncio.wait_for(task, timeout=1)


async def test_relay_client_answers_ping_with_pong(welcome_frame: dict[str, Any]) -> None:
    conn = FakeConnection()
    conn.feed(welcome_frame)
    conn.feed({"v": 1, "type": "ping", "id": 9, "t_ns": 123})
    client = _client(conn)
    task = asyncio.create_task(client.run())
    await client.wait_connected(within_s=1)
    await asyncio.sleep(0.02)
    pongs = [f for f in conn.outbox if f["type"] == "pong"]
    assert pongs and pongs[0]["id"] == 9 and pongs[0]["t_ping_ns"] == 123
    client.stop()
    conn.drop()
    await asyncio.wait_for(task, timeout=1)


async def test_relay_client_reconnects_after_drop(welcome_frame: dict[str, Any]) -> None:
    conns: list[FakeConnection] = []

    async def connect(_: str) -> FakeConnection:
        c = FakeConnection()
        c.feed(welcome_frame)
        conns.append(c)
        return c

    client = RelayClient(
        "wss://x/v1/relay",
        "svc",
        client_version="0.1.0",
        connect=connect,
        reconnect_min_s=0.01,
        reconnect_max_s=0.02,
    )
    task = asyncio.create_task(client.run())
    await client.wait_connected(within_s=1)
    conns[0].drop()  # first session dies
    await asyncio.sleep(0.1)
    assert len(conns) >= 2  # it opened a new connection
    assert client.is_connected
    client.stop()
    conns[-1].drop()
    await asyncio.wait_for(task, timeout=1)


# --- pairing client --------------------------------------------------------------------


class _Resp:
    def __init__(self, status: int, body: dict[str, Any]) -> None:
        self.status_code = status
        self._body = body

    def json(self) -> dict[str, Any]:
        return self._body


class _Http:
    def __init__(self, resp: _Resp) -> None:
        self._resp = resp
        self.calls: list[tuple[str, dict[str, Any]]] = []

    async def post(self, url: str, json: dict[str, Any]) -> _Resp:
        self.calls.append((url, json))
        return self._resp


async def test_claim_pairing_success() -> None:
    http = _Http(_Resp(200, {"token": "svc_new", "cell_id": "bottling", "daemon_id": "d-1"}))
    result = await claim_pairing(http, "https://api", code="ABCD2345", name="lab")
    assert result.service_token == "svc_new" and result.cell_id == "bottling"
    assert http.calls[0][0] == "https://api/v1/pairing/claim"


async def test_claim_pairing_bad_code_raises_with_code() -> None:
    http = _Http(_Resp(409, {"code": "NE1111", "message": "bad code"}))
    with pytest.raises(PairingError) as exc:
        await claim_pairing(http, "https://api", code="WRONG", name="lab")
    assert exc.value.code == "NE1111"
