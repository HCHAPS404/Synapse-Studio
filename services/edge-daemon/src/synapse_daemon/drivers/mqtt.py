"""An MQTT driver over `aiomqtt`.

A tag maps to a topic via `cfg.tag_map[tag] = {"topic": str}`. A write publishes the value to
the topic; a read returns the latest payload received on it (MQTT is publish/subscribe, so the
driver caches the last message per subscribed topic). It is tested against a mock client (the
design allows a mock when no in-process broker is available). A tag with no received message is
a `bad`-quality sample; the driver never guesses a value.
"""

from __future__ import annotations

import asyncio
import contextlib
import time
from typing import Any, Protocol

from synapse_daemon.drivers.base import (
    Driver,
    DriverConfig,
    DriverError,
    Quality,
    Sample,
    Value,
)


class MqttClient(Protocol):
    """The subset of `aiomqtt.Client` the driver uses, so a mock can stand in."""

    async def __aenter__(self) -> MqttClient: ...
    async def __aexit__(self, *exc: Any) -> None: ...
    async def publish(self, topic: str, payload: Any) -> None: ...
    async def subscribe(self, topic: str) -> Any: ...
    @property
    def messages(self) -> Any: ...


def _encode(value: Value) -> bytes:
    if isinstance(value, bool):
        return b"1" if value else b"0"
    return str(value).encode("utf-8")


def _decode(payload: bytes | str) -> Value:
    text = payload.decode("utf-8", "replace") if isinstance(payload, bytes) else str(payload)
    text = text.strip()
    try:
        return int(text)
    except ValueError:
        pass
    try:
        return float(text)
    except ValueError:
        return text


class MqttDriver(Driver):
    """MQTT publish/subscribe. `cfg.params`: `host`, `port`; a `client_factory` for tests."""

    name = "mqtt"

    def __init__(self, client_factory: Any | None = None) -> None:
        self._client_factory = client_factory
        self._client: MqttClient | None = None
        self._cfg: DriverConfig | None = None
        self._latest: dict[str, bytes] = {}  # topic -> last payload
        self._reader: asyncio.Task[None] | None = None

    def _new_client(self, cfg: DriverConfig) -> MqttClient:
        if self._client_factory is not None:
            return self._client_factory()
        import aiomqtt

        host = str(cfg.params.get("host", "127.0.0.1"))
        port = int(cfg.params.get("port", 1883))  # type: ignore[arg-type]
        return aiomqtt.Client(hostname=host, port=port)  # type: ignore[return-value]

    def _topic(self, tag: str) -> str:
        spec = self._cfg.tag_map.get(tag) if self._cfg else None
        if not isinstance(spec, dict) or "topic" not in spec:
            raise DriverError(f"tag {tag!r} is not mapped to an MQTT topic")
        return str(spec["topic"])

    async def open(self, cfg: DriverConfig) -> None:
        self._cfg = cfg
        client = self._new_client(cfg)
        try:
            await client.__aenter__()
        except Exception as exc:
            raise DriverError(f"mqtt: cannot connect: {exc}") from exc
        self._client = client
        for tag in cfg.tag_map:
            await client.subscribe(self._topic(tag))
        self._reader = asyncio.create_task(self._read_loop())

    async def _read_loop(self) -> None:
        assert self._client is not None
        with contextlib.suppress(Exception):
            async for message in self._client.messages:
                topic = str(message.topic)
                payload = message.payload
                self._latest[topic] = payload if isinstance(payload, bytes) else _encode(payload)

    async def read(self, tag: str) -> Sample:
        now = time.monotonic_ns()
        if self._client is None or self._cfg is None:
            return Sample.bad(tag, now)
        try:
            topic = self._topic(tag)
        except DriverError:
            return Sample.bad(tag, now)
        # Give the reader a moment to deliver a just-published retained message.
        await asyncio.sleep(0)
        if topic not in self._latest:
            return Sample.bad(tag, now)
        return Sample(tag=tag, value=_decode(self._latest[topic]), quality=Quality.GOOD, t_ns=now)

    async def write(self, tag: str, value: Value) -> None:
        if self._client is None or self._cfg is None:
            raise DriverError("mqtt client is not open")
        if self._cfg.writable and not self._cfg.is_writable(tag):
            raise DriverError(f"tag {tag!r} is not writable")
        topic = self._topic(tag)
        try:
            await self._client.publish(topic, _encode(value))
        except Exception as exc:
            raise DriverError(f"mqtt publish failed: {exc}") from exc

    async def close(self) -> None:
        if self._reader is not None:
            self._reader.cancel()
            with contextlib.suppress(asyncio.CancelledError, Exception):
                await self._reader
            self._reader = None
        if self._client is not None:
            with contextlib.suppress(Exception):
                await self._client.__aexit__(None, None, None)
            self._client = None
