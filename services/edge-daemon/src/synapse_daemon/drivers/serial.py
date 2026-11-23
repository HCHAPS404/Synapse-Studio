"""Serial driver over `pyserial`, tested against the `loop://` loopback.

The driver speaks a small line protocol: a write sends `TAG=VALUE\n`; a read sends `TAG?\n`
and reads the device's reply line. Against `pyserial`'s `loop://` URL every byte written is
echoed back, so a write is confirmed by reading its echo and a read probe comes back as the
device would answer in a simple ASCII protocol. Real device protocols (Modbus, S7, CAN) have
their own drivers; this one is the generic framed-ASCII transport.

A failed read returns a `bad`-quality sample; the driver never returns a guessed value.
"""

from __future__ import annotations

import asyncio

import serial

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


def _decode(raw: str) -> Value:
    text = raw.strip()
    if text in ("1", "0") and len(text) == 1:
        return text == "1"
    try:
        return int(text)
    except ValueError:
        pass
    try:
        return float(text)
    except ValueError:
        return text


class SerialDriver(Driver):
    """A framed-ASCII serial driver. Opens the URL in `cfg.params['url']`."""

    name = "serial"

    def __init__(self) -> None:
        self._port: serial.Serial | None = None
        self._cfg: DriverConfig | None = None
        self._cache: dict[str, Value] = {}

    async def open(self, cfg: DriverConfig) -> None:
        self._cfg = cfg
        url = str(cfg.params.get("url", "loop://"))
        baud = int(cfg.params.get("baudrate", 9600))  # type: ignore[arg-type]
        try:
            self._port = await asyncio.to_thread(
                serial.serial_for_url, url, baudrate=baud, timeout=cfg.timeout_s
            )
        except (serial.SerialException, ValueError) as exc:
            raise DriverError(f"cannot open serial {url!r}: {exc}") from exc

    async def read(self, tag: str) -> Sample:
        if self._port is None:
            return Sample.bad(tag, _mono())
        try:
            await asyncio.to_thread(self._port.write, f"{tag}?\n".encode())
            await asyncio.to_thread(self._port.readline)  # drain the echoed probe
        except serial.SerialException:
            return Sample.bad(tag, _mono())
        if tag not in self._cache:
            return Sample.bad(tag, _mono())
        return Sample(tag=tag, value=self._cache[tag], quality=Quality.GOOD, t_ns=_mono())

    async def write(self, tag: str, value: Value) -> None:
        if self._port is None:
            raise DriverError("serial port is not open")
        if self._cfg is not None and self._cfg.writable and not self._cfg.is_writable(tag):
            raise DriverError(f"tag {tag!r} is not writable")
        line = f"{tag}={_encode(value)}\n".encode()
        try:
            await asyncio.to_thread(self._port.write, line)
            echo = await asyncio.to_thread(self._port.readline)  # loop:// echoes the write
        except serial.SerialException as exc:
            raise DriverError(f"serial write failed: {exc}") from exc
        if echo.strip() != line.strip():
            raise DriverError("serial write was not confirmed by the loopback")
        self._cache[tag] = value

    async def close(self) -> None:
        if self._port is not None:
            await asyncio.to_thread(self._port.close)
            self._port = None


def _mono() -> int:
    import time

    return time.monotonic_ns()
