"""E5.9: the CAN driver on the python-can virtual bus."""

from __future__ import annotations

import asyncio
import contextlib
import uuid
from collections.abc import AsyncIterator

import can
import pytest

from synapse_daemon.drivers.base import DriverConfig, DriverError, Quality
from synapse_daemon.drivers.can import CanDriver


@pytest.fixture
def channel() -> str:
    # A unique virtual channel per test so buses do not cross-talk.
    return f"synapse-{uuid.uuid4().hex[:8]}"


def _cfg(channel: str) -> DriverConfig:
    return DriverConfig(
        name="motor",
        params={"interface": "virtual", "channel": channel},
        tag_map={
            "rpm": {"arbitration_id": 0x100, "start": 0, "length": 2, "kind": "int"},
            "enable": {"arbitration_id": 0x101, "start": 0, "length": 1, "kind": "bool"},
        },
        writable=frozenset({"rpm", "enable"}),
        timeout_s=1.0,
    )


@pytest.fixture
async def peer(channel: str) -> AsyncIterator[can.BusABC]:
    bus = can.Bus(interface="virtual", channel=channel)
    try:
        yield bus
    finally:
        with contextlib.suppress(Exception):
            bus.shutdown()


async def test_can_read_from_a_received_frame(channel: str, peer: can.BusABC) -> None:
    drv = CanDriver()
    await drv.open(_cfg(channel))
    peer.send(can.Message(arbitration_id=0x100, data=[0x01, 0x2C], is_extended_id=False))  # 300
    await asyncio.sleep(0.2)
    s = await drv.read("rpm")
    assert s.value == 300 and s.quality is Quality.GOOD
    await drv.close()


async def test_can_read_bool_frame(channel: str, peer: can.BusABC) -> None:
    drv = CanDriver()
    await drv.open(_cfg(channel))
    peer.send(can.Message(arbitration_id=0x101, data=[0x01], is_extended_id=False))
    await asyncio.sleep(0.2)
    assert (await drv.read("enable")).value is True
    await drv.close()


async def test_can_write_then_peer_receives(channel: str, peer: can.BusABC) -> None:
    drv = CanDriver()
    await drv.open(_cfg(channel))
    await drv.write("rpm", 1000)
    msg = await asyncio.to_thread(peer.recv, 1.0)
    assert msg is not None and msg.arbitration_id == 0x100
    assert int.from_bytes(bytes(msg.data[:2]), "big") == 1000
    await drv.close()


async def test_can_write_then_read_back(channel: str, peer: can.BusABC) -> None:
    drv = CanDriver()
    await drv.open(_cfg(channel))
    await drv.write("enable", True)
    assert (await drv.read("enable")).value is True
    await drv.close()


async def test_can_unseen_tag_is_bad(channel: str) -> None:
    drv = CanDriver()
    await drv.open(_cfg(channel))
    assert (await drv.read("rpm")).quality is Quality.BAD
    await drv.close()


async def test_can_non_writable_refused(channel: str) -> None:
    cfg = _cfg(channel)
    cfg.writable = frozenset({"rpm"})
    drv = CanDriver()
    await drv.open(cfg)
    with pytest.raises(DriverError):
        await drv.write("enable", True)
    await drv.close()


async def test_can_read_before_open_is_bad() -> None:
    drv = CanDriver()
    assert (await drv.read("rpm")).quality is Quality.BAD
