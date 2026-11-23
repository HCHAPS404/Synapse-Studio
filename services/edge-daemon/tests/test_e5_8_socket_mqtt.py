"""E5.8: the TCP socket driver against an echo server, the MQTT driver against a mock broker."""

from __future__ import annotations

import asyncio
import contextlib
from collections.abc import AsyncIterator
from typing import Any

import pytest

from synapse_daemon.drivers.base import DriverConfig, DriverError, Quality
from synapse_daemon.drivers.mqtt import MqttDriver
from synapse_daemon.drivers.socket_tcp import TcpSocketDriver

# --- TCP echo-and-remember server ------------------------------------------------------


class _EchoServer:
    """A device that remembers `TAG=VALUE` writes and answers `TAG?` with `TAG=VALUE`."""

    def __init__(self) -> None:
        self.values: dict[str, str] = {}
        self._server: asyncio.AbstractServer | None = None
        self.port = 0

    async def start(self) -> None:
        self._server = await asyncio.start_server(self._handle, "127.0.0.1", 0)
        self.port = self._server.sockets[0].getsockname()[1]

    async def _handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        with contextlib.suppress(Exception):
            while not reader.at_eof():
                line = await reader.readline()
                if not line:
                    break
                text = line.decode().strip()
                if text.endswith("?"):
                    tag = text[:-1]
                    if tag in self.values:
                        writer.write(f"{tag}={self.values[tag]}\n".encode())
                        await writer.drain()
                elif "=" in text:
                    tag, _, value = text.partition("=")
                    self.values[tag] = value
        writer.close()

    async def stop(self) -> None:
        if self._server is not None:
            self._server.close()
            with contextlib.suppress(Exception):
                await self._server.wait_closed()


@pytest.fixture
async def echo() -> AsyncIterator[_EchoServer]:
    server = _EchoServer()
    await server.start()
    try:
        yield server
    finally:
        await server.stop()


async def test_tcp_write_then_read(echo: _EchoServer) -> None:
    drv = TcpSocketDriver()
    await drv.open(
        DriverConfig(
            name="dev",
            params={"host": "127.0.0.1", "port": echo.port},
            writable=frozenset({"sp"}),
            timeout_s=2.0,
        )
    )
    await drv.write("sp", 55)
    assert (await drv.read("sp")).value == 55
    await drv.close()


async def test_tcp_read_unknown_is_bad(echo: _EchoServer) -> None:
    drv = TcpSocketDriver()
    await drv.open(
        DriverConfig(name="dev", params={"host": "127.0.0.1", "port": echo.port}, timeout_s=0.5)
    )
    assert (await drv.read("never")).quality is Quality.BAD
    await drv.close()


async def test_tcp_non_writable_refused(echo: _EchoServer) -> None:
    drv = TcpSocketDriver()
    await drv.open(
        DriverConfig(
            name="dev",
            params={"host": "127.0.0.1", "port": echo.port},
            writable=frozenset({"a"}),
        )
    )
    with pytest.raises(DriverError):
        await drv.write("b", 1)
    await drv.close()


async def test_tcp_connect_failure_raises() -> None:
    drv = TcpSocketDriver()
    with pytest.raises(DriverError):
        await drv.open(
            DriverConfig(name="dev", params={"host": "127.0.0.1", "port": 1}, timeout_s=0.5)
        )


# --- MQTT mock broker ------------------------------------------------------------------


class _Message:
    def __init__(self, topic: str, payload: bytes) -> None:
        self.topic = topic
        self.payload = payload


class _MockBroker:
    """A shared in-process broker: a publish is delivered to every subscribed client."""

    def __init__(self) -> None:
        self.clients: list[_MockClient] = []

    def deliver(self, topic: str, payload: bytes) -> None:
        for client in self.clients:
            if topic in client.subscriptions:
                client.inbox.put_nowait(_Message(topic, payload))


class _MockClient:
    def __init__(self, broker: _MockBroker) -> None:
        self._broker = broker
        self.subscriptions: set[str] = set()
        self.inbox: asyncio.Queue[_Message] = asyncio.Queue()

    async def __aenter__(self) -> _MockClient:
        self._broker.clients.append(self)
        return self

    async def __aexit__(self, *exc: Any) -> None:
        with contextlib.suppress(ValueError):
            self._broker.clients.remove(self)

    async def subscribe(self, topic: str) -> None:
        self.subscriptions.add(topic)

    async def publish(self, topic: str, payload: Any) -> None:
        data = payload if isinstance(payload, bytes) else str(payload).encode()
        self._broker.deliver(topic, data)

    @property
    async def messages(self) -> Any:  # async generator
        while True:
            yield await self.inbox.get()


def _mqtt_cfg() -> DriverConfig:
    return DriverConfig(
        name="sensor",
        params={"host": "127.0.0.1"},
        tag_map={"temp": {"topic": "cell/temp"}, "sp": {"topic": "cell/sp"}},
        writable=frozenset({"sp"}),
    )


async def test_mqtt_publish_is_received_as_read() -> None:
    broker = _MockBroker()
    # Two clients on the same broker: a publisher and the driver (subscriber).
    pub = _MockClient(broker)
    await pub.__aenter__()
    drv = MqttDriver(client_factory=lambda: _MockClient(broker))
    await drv.open(_mqtt_cfg())
    await pub.publish("cell/temp", b"21.5")
    await asyncio.sleep(0.05)  # let the driver's reader deliver the message
    s = await drv.read("temp")
    assert s.value == pytest.approx(21.5) and s.quality is Quality.GOOD
    await drv.close()
    await pub.__aexit__(None, None, None)


async def test_mqtt_write_publishes_to_own_subscription() -> None:
    broker = _MockBroker()
    drv = MqttDriver(client_factory=lambda: _MockClient(broker))
    await drv.open(_mqtt_cfg())
    await drv.write("sp", 42)  # the driver is subscribed to cell/sp, so it reads it back
    await asyncio.sleep(0.05)
    assert (await drv.read("sp")).value == 42
    await drv.close()


async def test_mqtt_unseen_tag_is_bad() -> None:
    broker = _MockBroker()
    drv = MqttDriver(client_factory=lambda: _MockClient(broker))
    await drv.open(_mqtt_cfg())
    assert (await drv.read("temp")).quality is Quality.BAD  # nothing published yet
    await drv.close()


async def test_mqtt_non_writable_refused() -> None:
    broker = _MockBroker()
    drv = MqttDriver(client_factory=lambda: _MockClient(broker))
    await drv.open(_mqtt_cfg())
    with pytest.raises(DriverError):
        await drv.write("temp", 1)  # temp is not in the writable set
    await drv.close()
