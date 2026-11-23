"""E5.7: the S7 driver against an in-process snap7 server, with a mock fallback."""

from __future__ import annotations

import asyncio
import contextlib
import ctypes
import socket
from typing import Any

import pytest

from synapse_daemon.drivers.base import DriverConfig, DriverError, Quality
from synapse_daemon.drivers.s7 import S7Driver


def _free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def _native_available() -> bool:
    try:
        from snap7.server import Server

        Server()  # loads the native library
        return True
    except Exception:
        return False


NATIVE = _native_available()


# --- real snap7 server round trip ------------------------------------------------------


@pytest.mark.skipif(not NATIVE, reason="snap7 native library not available")
async def test_s7_round_trip_against_server() -> None:
    from snap7.server import Server
    from snap7.type import SrvArea

    port = _free_port()
    server = Server()
    db_size = 16
    buffer = (ctypes.c_char * db_size)()
    server.register_area(SrvArea.DB, 1, buffer)
    started = await asyncio.to_thread(server.start, port)
    assert started == 0 or started is None
    try:
        drv = S7Driver()
        await drv.open(
            DriverConfig(
                name="plc",
                params={"host": "127.0.0.1", "rack": 0, "slot": 1, "port": port},
                tag_map={
                    "speed": {"db": 1, "start": 0, "kind": "int"},
                    "run": {"db": 1, "start": 4, "kind": "bool", "bit": 0},
                },
                writable=frozenset({"speed", "run"}),
                timeout_s=2.0,
            )
        )
        await drv.write("speed", 1234)
        assert (await drv.read("speed")).value == 1234
        await drv.write("run", True)
        assert (await drv.read("run")).value is True
        await drv.close()
    finally:
        with contextlib.suppress(Exception):
            server.stop()
        with contextlib.suppress(Exception):
            server.destroy()


# --- mock client (always runs; the design allows a mock when the lib is absent) --------


class MockS7Client:
    """A tiny in-memory S7 client: typed DB reads/writes over a dict of DB buffers."""

    def __init__(self) -> None:
        self._int: dict[tuple[int, int], int] = {}
        self._real: dict[tuple[int, int], float] = {}
        self._bool: dict[tuple[int, int, int], bool] = {}
        self.connected = False

    def connect(self, host: str, rack: int, slot: int, port: int = 102) -> Any:
        self.connected = True
        return self

    def db_read_int(self, db: int, start: int) -> int:
        return self._int.get((db, start), 0)

    def db_write_int(self, db: int, start: int, value: int) -> None:
        self._int[(db, start)] = value

    def db_read_real(self, db: int, start: int) -> float:
        return self._real.get((db, start), 0.0)

    def db_write_real(self, db: int, start: int, value: float) -> None:
        self._real[(db, start)] = value

    def db_read_bool(self, db: int, start: int, bit: int) -> bool:
        return self._bool.get((db, start, bit), False)

    def db_write_bool(self, db: int, start: int, bit: int, value: bool) -> None:
        self._bool[(db, start, bit)] = value

    def disconnect(self) -> None:
        self.connected = False


def _mock_cfg() -> DriverConfig:
    return DriverConfig(
        name="plc",
        params={"host": "127.0.0.1"},
        tag_map={
            "speed": {"db": 1, "start": 0, "kind": "int"},
            "temp": {"db": 1, "start": 2, "kind": "real"},
            "run": {"db": 1, "start": 4, "kind": "bool", "bit": 0},
        },
        writable=frozenset({"speed", "temp", "run"}),
    )


async def test_s7_mock_int_real_bool_round_trip() -> None:
    drv = S7Driver(client_factory=MockS7Client)
    await drv.open(_mock_cfg())
    await drv.write("speed", 900)
    await drv.write("temp", 36.5)
    await drv.write("run", True)
    assert (await drv.read("speed")).value == 900
    assert (await drv.read("temp")).value == pytest.approx(36.5)
    assert (await drv.read("run")).value is True
    await drv.close()


async def test_s7_mock_non_writable_refused() -> None:
    cfg = _mock_cfg()
    cfg.writable = frozenset({"speed"})
    drv = S7Driver(client_factory=MockS7Client)
    await drv.open(cfg)
    with pytest.raises(DriverError):
        await drv.write("run", True)
    await drv.close()


async def test_s7_mock_unmapped_read_is_bad() -> None:
    drv = S7Driver(client_factory=MockS7Client)
    await drv.open(_mock_cfg())
    assert (await drv.read("ghost")).quality is Quality.BAD
    await drv.close()


async def test_s7_read_before_open_is_bad() -> None:
    drv = S7Driver(client_factory=MockS7Client)
    assert (await drv.read("speed")).quality is Quality.BAD


async def test_s7_connect_failure_raises() -> None:
    class Failing:
        def connect(self, *a: Any, **k: Any) -> None:
            raise RuntimeError("no route to host")

    drv = S7Driver(client_factory=Failing)
    with pytest.raises(DriverError):
        await drv.open(_mock_cfg())
