"""E5.6: Modbus TCP against an in-process pymodbus server; RTU over a loopback pair."""

from __future__ import annotations

import asyncio
import contextlib
import socket
import sys
from collections.abc import AsyncIterator

import pytest
from pymodbus.datastore import (
    ModbusDeviceContext,
    ModbusSequentialDataBlock,
    ModbusServerContext,
)
from pymodbus.server import ModbusTcpServer

from synapse_daemon.drivers.base import DriverConfig, DriverError, Quality
from synapse_daemon.drivers.modbus import ModbusRtuDriver, ModbusTcpDriver


def _free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def _context() -> ModbusServerContext:
    # Holding registers and coils seeded with known values at low addresses.
    # pymodbus data blocks are 1-based internally; client address 0 maps to the first value.
    device = ModbusDeviceContext(
        co=ModbusSequentialDataBlock(1, [False] * 16),
        hr=ModbusSequentialDataBlock(1, [111, 222, 333] + [0] * 13),
    )
    return ModbusServerContext(devices=device, single=True)


@pytest.fixture
async def tcp_server() -> AsyncIterator[int]:
    port = _free_port()
    server = ModbusTcpServer(_context(), address=("127.0.0.1", port))
    task = asyncio.create_task(server.serve_forever())
    await asyncio.sleep(0.2)  # let it bind
    try:
        yield port
    finally:
        await server.shutdown()
        task.cancel()
        with contextlib.suppress(asyncio.CancelledError, Exception):
            await task


def _tcp_cfg(port: int) -> DriverConfig:
    return DriverConfig(
        name="plc",
        params={"host": "127.0.0.1", "port": port, "device_id": 1},
        tag_map={
            "level": {"type": "holding", "address": 0},
            "mode": {"type": "holding", "address": 1},
            "valve": {"type": "coil", "address": 0},
        },
        writable=frozenset({"mode", "valve"}),
        timeout_s=2.0,
    )


async def test_tcp_read_holding_register(tcp_server: int) -> None:
    drv = ModbusTcpDriver()
    await drv.open(_tcp_cfg(tcp_server))
    s = await drv.read("level")
    assert s.value == 111 and s.quality is Quality.GOOD
    await drv.close()


async def test_tcp_write_then_read_register(tcp_server: int) -> None:
    drv = ModbusTcpDriver()
    await drv.open(_tcp_cfg(tcp_server))
    await drv.write("mode", 7)
    assert (await drv.read("mode")).value == 7
    await drv.close()


async def test_tcp_write_then_read_coil(tcp_server: int) -> None:
    drv = ModbusTcpDriver()
    await drv.open(_tcp_cfg(tcp_server))
    await drv.write("valve", True)
    assert (await drv.read("valve")).value is True
    await drv.close()


async def test_tcp_non_writable_tag_refused(tcp_server: int) -> None:
    drv = ModbusTcpDriver()
    await drv.open(_tcp_cfg(tcp_server))
    with pytest.raises(DriverError):
        await drv.write("level", 5)  # not in the writable set
    await drv.close()


async def test_tcp_unmapped_tag_read_is_bad(tcp_server: int) -> None:
    drv = ModbusTcpDriver()
    await drv.open(_tcp_cfg(tcp_server))
    assert (await drv.read("ghost")).quality is Quality.BAD
    await drv.close()


async def test_tcp_connect_failure_raises() -> None:
    drv = ModbusTcpDriver()
    # Nothing is listening on this port.
    cfg = _tcp_cfg(_free_port())
    with pytest.raises(DriverError):
        await drv.open(cfg)


# --- RTU over a loopback serial pair ---------------------------------------------------


@pytest.mark.skipif(sys.platform == "win32", reason="pty loopback pair is POSIX-only")
async def test_rtu_round_trip_over_pty_pair() -> None:
    import os

    from pymodbus.server import ModbusSerialServer

    # A real loopback serial pair: the two ends of a pty. The server owns one end, the
    # client the other. Using the /dev/fd paths keeps both ends open for the duration.
    server_fd, client_fd = os.openpty()
    server_port = os.ttyname(server_fd)
    client_port = os.ttyname(client_fd)

    server = ModbusSerialServer(_context(), port=server_port, baudrate=9600)
    server_task = asyncio.create_task(server.serve_forever())
    await asyncio.sleep(0.3)
    try:
        drv = ModbusRtuDriver()
        await drv.open(
            DriverConfig(
                name="plc",
                params={"port": client_port, "baudrate": 9600, "device_id": 1},
                tag_map={"level": {"type": "holding", "address": 0}},
                writable=frozenset({"level"}),
                timeout_s=2.0,
            )
        )
        s = await drv.read("level")
        assert s.value == 111 and s.quality is Quality.GOOD
        await drv.close()
    finally:
        with contextlib.suppress(Exception):
            await server.shutdown()
        server_task.cancel()
        with contextlib.suppress(asyncio.CancelledError, Exception):
            await server_task
        os.close(server_fd)
        os.close(client_fd)


async def test_rtu_offline_read_is_bad() -> None:
    drv = ModbusRtuDriver()
    cfg = DriverConfig(
        name="plc",
        params={"port": "loop://"},
        tag_map={"x": {"type": "holding", "address": 0}},
        timeout_s=0.5,
    )
    with contextlib.suppress(DriverError):
        await drv.open(cfg)
    # No RTU server answers on the loopback, so a read is bad quality, never a guess.
    assert (await drv.read("x")).quality is Quality.BAD
    await drv.close()


async def test_rtu_driver_builds_serial_client() -> None:
    drv = ModbusRtuDriver()
    client = drv._make_client(DriverConfig(name="p", params={"port": "loop://", "baudrate": 19200}))
    assert client is not None and drv.name == "modbus_rtu"
