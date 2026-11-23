"""A CAN driver over `python-can`, tested on the virtual bus (`interface="virtual"`).

A tag maps to a byte slice of a CAN frame:
`cfg.tag_map[tag] = {"arbitration_id": int, "start": int, "length": int, "kind": "int"|"bool",
"signed": bool}`. A read returns the decoded slice from the latest frame seen for that
arbitration id; a write packs the value into a frame (the last frame for that id, or zeros) and
sends it. A tag with no received frame is a `bad`-quality sample; the driver never guesses.
"""

from __future__ import annotations

import asyncio
import contextlib
import time
from typing import Any

import can

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
    if not isinstance(spec, dict) or "arbitration_id" not in spec:
        raise DriverError(f"tag {tag!r} is not mapped to a CAN frame")
    return spec


def _decode(data: bytes, spec: dict[str, Any]) -> Value:
    start = int(spec.get("start", 0))
    length = int(spec.get("length", 1))
    chunk = data[start : start + length]
    if len(chunk) < length:
        chunk = chunk + b"\x00" * (length - len(chunk))
    kind = str(spec.get("kind", "int"))
    if kind == "bool":
        return any(chunk)
    return int.from_bytes(chunk, "big", signed=bool(spec.get("signed", False)))


def _encode_into(base: bytearray, value: Value, spec: dict[str, Any]) -> bytearray:
    start = int(spec.get("start", 0))
    length = int(spec.get("length", 1))
    kind = str(spec.get("kind", "int"))
    if kind == "bool":
        chunk = (b"\x01" if value else b"\x00") * length
    else:
        chunk = int(value).to_bytes(length, "big", signed=bool(spec.get("signed", False)))  # type: ignore[arg-type]
    while len(base) < start + length:
        base.append(0)
    base[start : start + length] = chunk
    return base


class CanDriver(Driver):
    """CAN over python-can. `cfg.params`: `interface` (default `virtual`), `channel`, `bitrate`."""

    name = "can"

    def __init__(self) -> None:
        self._bus: can.BusABC | None = None
        self._cfg: DriverConfig | None = None
        self._latest: dict[int, bytes] = {}  # arbitration_id -> last data
        self._reader: asyncio.Task[None] | None = None
        self._stop = False

    async def open(self, cfg: DriverConfig) -> None:
        self._cfg = cfg
        interface = str(cfg.params.get("interface", "virtual"))
        channel = str(cfg.params.get("channel", "synapse"))
        try:
            kwargs: dict[str, Any] = {"interface": interface, "channel": channel}
            if "bitrate" in cfg.params:
                kwargs["bitrate"] = int(cfg.params["bitrate"])  # type: ignore[arg-type]
            self._bus = await asyncio.to_thread(lambda: can.Bus(**kwargs))
        except Exception as exc:
            raise DriverError(f"can: cannot open bus: {exc}") from exc
        self._stop = False
        self._reader = asyncio.create_task(self._read_loop())

    async def _read_loop(self) -> None:
        bus = self._bus
        assert bus is not None
        while not self._stop:
            msg = await asyncio.to_thread(bus.recv, 0.1)
            if msg is not None:
                self._latest[int(msg.arbitration_id)] = bytes(msg.data)

    async def read(self, tag: str) -> Sample:
        now = time.monotonic_ns()
        if self._bus is None or self._cfg is None:
            return Sample.bad(tag, now)
        try:
            spec = _entry(self._cfg, tag)
        except DriverError:
            return Sample.bad(tag, now)
        arb = int(spec["arbitration_id"])
        if arb not in self._latest:
            return Sample.bad(tag, now)
        return Sample(
            tag=tag, value=_decode(self._latest[arb], spec), quality=Quality.GOOD, t_ns=now
        )

    async def write(self, tag: str, value: Value) -> None:
        if self._bus is None or self._cfg is None:
            raise DriverError("can bus is not open")
        if self._cfg.writable and not self._cfg.is_writable(tag):
            raise DriverError(f"tag {tag!r} is not writable")
        spec = _entry(self._cfg, tag)
        arb = int(spec["arbitration_id"])
        base = bytearray(self._latest.get(arb, b""))
        data = bytes(_encode_into(base, value, spec))
        msg = can.Message(arbitration_id=arb, data=data, is_extended_id=bool(spec.get("extended")))
        try:
            await asyncio.to_thread(self._bus.send, msg)
        except Exception as exc:
            raise DriverError(f"can send failed: {exc}") from exc
        # Remember what we sent so a subsequent read reflects the last known frame.
        self._latest[arb] = data

    async def close(self) -> None:
        self._stop = True
        if self._reader is not None:
            self._reader.cancel()
            with contextlib.suppress(asyncio.CancelledError, Exception):
                await self._reader
            self._reader = None
        if self._bus is not None:
            with contextlib.suppress(Exception):
                await asyncio.to_thread(self._bus.shutdown)
            self._bus = None
