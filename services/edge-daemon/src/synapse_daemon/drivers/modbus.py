"""Modbus TCP and RTU drivers over `pymodbus`.

A tag maps to a Modbus object through
`cfg.tag_map[tag] = {"type": "coil"|"holding", "address": int}`.
`ModbusTcpDriver` connects to `host:port`; `ModbusRtuDriver` opens a serial URL (tested over a
loopback serial pair against a pymodbus RTU server). Both share the read/write logic. A failed
read is a `bad`-quality sample; a failed write raises `DriverError`. The daemon never returns a
guessed value.
"""

from __future__ import annotations

import time
from typing import Any

from pymodbus.client import AsyncModbusSerialClient, AsyncModbusTcpClient
from pymodbus.client.base import ModbusBaseClient

from synapse_daemon.drivers.base import (
    Driver,
    DriverConfig,
    DriverError,
    Quality,
    Sample,
    Value,
)

_COIL = "coil"
_HOLDING = "holding"


def _entry(cfg: DriverConfig, tag: str) -> tuple[str, int]:
    spec = cfg.tag_map.get(tag)
    if not isinstance(spec, dict) or "type" not in spec or "address" not in spec:
        raise DriverError(f"tag {tag!r} is not mapped to a Modbus object")
    obj_type = str(spec["type"])
    if obj_type not in (_COIL, _HOLDING):
        raise DriverError(f"tag {tag!r} has unknown Modbus type {obj_type!r}")
    return obj_type, int(spec["address"])  # type: ignore[arg-type]


class _ModbusDriver(Driver):
    """Shared Modbus logic; subclasses build the client and set `name`."""

    name = "modbus"

    def __init__(self) -> None:
        self._client: ModbusBaseClient | None = None
        self._cfg: DriverConfig | None = None
        self._device_id = 1

    def _make_client(self, cfg: DriverConfig) -> ModbusBaseClient:  # pragma: no cover
        raise NotImplementedError

    async def open(self, cfg: DriverConfig) -> None:
        self._cfg = cfg
        self._device_id = int(cfg.params.get("device_id", 1))  # type: ignore[arg-type]
        client = self._make_client(cfg)
        ok = await client.connect()
        if not ok:
            raise DriverError(f"{self.name}: cannot connect")
        self._client = client

    async def read(self, tag: str) -> Sample:
        now = time.monotonic_ns()
        if self._client is None or self._cfg is None:
            return Sample.bad(tag, now)
        try:
            obj_type, address = _entry(self._cfg, tag)
            if obj_type == _COIL:
                rr = await self._client.read_coils(address, count=1, device_id=self._device_id)
            else:
                rr = await self._client.read_holding_registers(
                    address, count=1, device_id=self._device_id
                )
            if rr.isError():
                return Sample.bad(tag, now)
            value: Value = bool(rr.bits[0]) if obj_type == _COIL else int(rr.registers[0])
        except Exception:  # a failed read is bad quality, never a guess
            return Sample.bad(tag, now)
        return Sample(tag=tag, value=value, quality=Quality.GOOD, t_ns=now)

    async def write(self, tag: str, value: Value) -> None:
        if self._client is None or self._cfg is None:
            raise DriverError("modbus client is not open")
        if self._cfg.writable and not self._cfg.is_writable(tag):
            raise DriverError(f"tag {tag!r} is not writable")
        obj_type, address = _entry(self._cfg, tag)
        try:
            if obj_type == _COIL:
                rr = await self._client.write_coil(address, bool(value), device_id=self._device_id)
            else:
                rr = await self._client.write_register(
                    address,
                    int(value),
                    device_id=self._device_id,  # type: ignore[arg-type]
                )
        except Exception as exc:  # pymodbus raises its own exception hierarchy
            raise DriverError(f"modbus write failed: {exc}") from exc
        if rr.isError():
            raise DriverError(f"modbus write of {tag!r} returned an error")

    async def close(self) -> None:
        if self._client is not None:
            self._client.close()
            self._client = None


class ModbusTcpDriver(_ModbusDriver):
    """Modbus TCP. `cfg.params`: `host`, `port` (default 502), `device_id`."""

    name = "modbus_tcp"

    def _make_client(self, cfg: DriverConfig) -> ModbusBaseClient:
        params: dict[str, Any] = cfg.params
        host = str(params.get("host", "127.0.0.1"))
        port = int(params.get("port", 502))  # type: ignore[arg-type]
        return AsyncModbusTcpClient(host, port=port, timeout=cfg.timeout_s)


class ModbusRtuDriver(_ModbusDriver):
    """Modbus RTU over a serial port. `cfg.params`: `port` (serial URL), `baudrate`."""

    name = "modbus_rtu"

    def _make_client(self, cfg: DriverConfig) -> ModbusBaseClient:
        params: dict[str, Any] = cfg.params
        port = str(params.get("port", "loop://"))
        baud = int(params.get("baudrate", 9600))  # type: ignore[arg-type]
        return AsyncModbusSerialClient(port, baudrate=baud, timeout=cfg.timeout_s)
