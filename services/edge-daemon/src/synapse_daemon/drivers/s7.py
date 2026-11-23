"""S7 driver over `python-snap7`.

A tag maps to a DB location via
`cfg.tag_map[tag] = {"db": int, "start": int, "kind": "int"|"real"|"bool", "bit": int}`
(`bit` only for `kind == "bool"`). The driver reads and writes with snap7's typed DB helpers.
It is tested against an in-process snap7 `Server` (or a mock when the native library cannot
bind). A failed read is a `bad`-quality sample; a failed write raises `DriverError`; no value
is ever guessed.
"""

from __future__ import annotations

import asyncio
import time
from typing import Any

from synapse_daemon.drivers.base import (
    Driver,
    DriverConfig,
    DriverError,
    Quality,
    Sample,
    Value,
)


def _entry(cfg: DriverConfig, tag: str) -> dict[str, Any]:
    spec = cfg.tag_map.get(tag)
    if not isinstance(spec, dict) or "db" not in spec or "kind" not in spec:
        raise DriverError(f"tag {tag!r} is not mapped to an S7 DB location")
    return spec


class S7Driver(Driver):
    """Siemens S7 over ISO-on-TCP. `cfg.params`: `host`, `rack`, `slot`, `port`."""

    name = "s7"

    def __init__(self, client_factory: Any | None = None) -> None:
        # `client_factory` lets a test inject a mock when the native snap7 lib is absent.
        self._client_factory = client_factory
        self._client: Any | None = None
        self._cfg: DriverConfig | None = None

    def _new_client(self) -> Any:
        if self._client_factory is not None:
            return self._client_factory()
        from snap7.client import Client

        return Client()

    async def open(self, cfg: DriverConfig) -> None:
        self._cfg = cfg
        host = str(cfg.params.get("host", "127.0.0.1"))
        rack = int(cfg.params.get("rack", 0))  # type: ignore[arg-type]
        slot = int(cfg.params.get("slot", 1))  # type: ignore[arg-type]
        port = int(cfg.params.get("port", 102))  # type: ignore[arg-type]
        client = self._new_client()
        try:
            await asyncio.to_thread(client.connect, host, rack, slot, port)
        except Exception as exc:  # snap7 raises Snap7Exception
            raise DriverError(f"s7: cannot connect to {host}:{port}: {exc}") from exc
        self._client = client

    async def read(self, tag: str) -> Sample:
        now = time.monotonic_ns()
        if self._client is None or self._cfg is None:
            return Sample.bad(tag, now)
        try:
            spec = _entry(self._cfg, tag)
            value = await asyncio.to_thread(self._read_value, spec)
        except Exception:  # a failed read is bad quality, never a guess
            return Sample.bad(tag, now)
        return Sample(tag=tag, value=value, quality=Quality.GOOD, t_ns=now)

    def _read_value(self, spec: dict[str, Any]) -> Value:
        client = self._client
        db = int(spec["db"])
        start = int(spec.get("start", 0))
        kind = str(spec["kind"])
        if kind == "int":
            return int(client.db_read_int(db, start))
        if kind == "real":
            return float(client.db_read_real(db, start))
        if kind == "bool":
            return bool(client.db_read_bool(db, start, int(spec.get("bit", 0))))
        raise DriverError(f"unknown S7 kind {kind!r}")

    async def write(self, tag: str, value: Value) -> None:
        if self._client is None or self._cfg is None:
            raise DriverError("s7 client is not open")
        if self._cfg.writable and not self._cfg.is_writable(tag):
            raise DriverError(f"tag {tag!r} is not writable")
        spec = _entry(self._cfg, tag)
        try:
            await asyncio.to_thread(self._write_value, spec, value)
        except DriverError:
            raise
        except Exception as exc:
            raise DriverError(f"s7 write of {tag!r} failed: {exc}") from exc

    def _write_value(self, spec: dict[str, Any], value: Value) -> None:
        client = self._client
        db = int(spec["db"])
        start = int(spec.get("start", 0))
        kind = str(spec["kind"])
        if kind == "int":
            client.db_write_int(db, start, int(value))  # type: ignore[arg-type]
        elif kind == "real":
            client.db_write_real(db, start, float(value))  # type: ignore[arg-type]
        elif kind == "bool":
            client.db_write_bool(db, start, int(spec.get("bit", 0)), bool(value))
        else:
            raise DriverError(f"unknown S7 kind {kind!r}")

    async def close(self) -> None:
        if self._client is not None:
            try:
                await asyncio.to_thread(self._client.disconnect)
            except Exception:  # noqa: S110 - closing an already-closed client is fine
                pass
            self._client = None
