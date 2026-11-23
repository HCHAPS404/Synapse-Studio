"""A TCP socket driver over asyncio streams with a newline-framed ASCII protocol.

A write sends `TAG=VALUE\n`; a read sends `TAG?\n` and the device answers `TAG=VALUE\n`. The
driver is tested against a local echo-and-remember server. A failed read is a `bad`-quality
sample; a failed write raises `DriverError`; no value is ever guessed.
"""

from __future__ import annotations

import asyncio
import time

from synapse_daemon.drivers.base import (
    Driver,
    DriverConfig,
    DriverError,
    Quality,
    Sample,
    Value,
)


def _encode(value: Value) -> str:
    if isinstance(value, bool):
        return "1" if value else "0"
    return str(value)


def _decode(text: str) -> Value:
    text = text.strip()
    try:
        return int(text)
    except ValueError:
        pass
    try:
        return float(text)
    except ValueError:
        return text


class TcpSocketDriver(Driver):
    """Newline-framed ASCII over a TCP socket. `cfg.params`: `host`, `port`."""

    name = "tcp"

    def __init__(self) -> None:
        self._reader: asyncio.StreamReader | None = None
        self._writer: asyncio.StreamWriter | None = None
        self._cfg: DriverConfig | None = None

    async def open(self, cfg: DriverConfig) -> None:
        self._cfg = cfg
        host = str(cfg.params.get("host", "127.0.0.1"))
        port = int(cfg.params.get("port", 0))  # type: ignore[arg-type]
        try:
            self._reader, self._writer = await asyncio.wait_for(
                asyncio.open_connection(host, port), timeout=cfg.timeout_s
            )
        except (OSError, TimeoutError) as exc:
            raise DriverError(f"tcp: cannot connect to {host}:{port}: {exc}") from exc

    async def read(self, tag: str) -> Sample:
        now = time.monotonic_ns()
        if self._reader is None or self._writer is None:
            return Sample.bad(tag, now)
        try:
            self._writer.write(f"{tag}?\n".encode())
            await self._writer.drain()
            line = await asyncio.wait_for(self._reader.readline(), timeout=self._timeout())
        except (OSError, TimeoutError):
            return Sample.bad(tag, now)
        text = line.decode("utf-8", "replace").strip()
        prefix = f"{tag}="
        if not text.startswith(prefix):
            return Sample.bad(tag, now)
        return Sample(tag=tag, value=_decode(text[len(prefix) :]), quality=Quality.GOOD, t_ns=now)

    async def write(self, tag: str, value: Value) -> None:
        if self._writer is None:
            raise DriverError("tcp socket is not open")
        if self._cfg is not None and self._cfg.writable and not self._cfg.is_writable(tag):
            raise DriverError(f"tag {tag!r} is not writable")
        try:
            self._writer.write(f"{tag}={_encode(value)}\n".encode())
            await self._writer.drain()
        except OSError as exc:
            raise DriverError(f"tcp write failed: {exc}") from exc

    async def close(self) -> None:
        if self._writer is not None:
            self._writer.close()
            try:
                await self._writer.wait_closed()
            except OSError:  # noqa: S110 - already closed
                pass
            self._writer = None
            self._reader = None

    def _timeout(self) -> float:
        return self._cfg.timeout_s if self._cfg is not None else 2.0
