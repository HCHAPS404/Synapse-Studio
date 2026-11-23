"""E5.3: the command allowlist, per-command Pydantic args, NE1109 and NE1111."""

from __future__ import annotations

import pytest

from synapse_daemon.commands import ALLOWED_COMMANDS, CommandDispatcher
from synapse_daemon.drivers.base import Driver, DriverConfig, Quality
from synapse_daemon.drivers.simulated import SimulatedDriver
from synapse_daemon.errors import DaemonError, ErrorCode


class Registry:
    def __init__(self, drivers: dict[str, Driver]) -> None:
        self._drivers = drivers

    def driver_for(self, device: str) -> Driver | None:
        return self._drivers.get(device)


async def _dispatcher_with_mcu() -> CommandDispatcher:
    drv = SimulatedDriver()
    await drv.open(
        DriverConfig(name="mcu1", params={"values": {"temp": 21}}, writable=frozenset({"relay"}))
    )
    return CommandDispatcher(Registry({"mcu1": drv}))


async def test_read_tag_allowed() -> None:
    disp = await _dispatcher_with_mcu()
    sample = await disp.run("read_tag", {"device": "mcu1", "tag": "temp"})
    assert sample.value == 21 and sample.quality is Quality.GOOD


async def test_write_tag_allowed_on_writable() -> None:
    disp = await _dispatcher_with_mcu()
    await disp.run("write_tag", {"device": "mcu1", "tag": "relay", "value": True})
    assert (await disp.run("read_tag", {"device": "mcu1", "tag": "relay"})).value is True


async def test_unknown_command_is_ne1109() -> None:
    disp = await _dispatcher_with_mcu()
    with pytest.raises(DaemonError) as exc:
        await disp.run("run_shell", {"cmd": "rm -rf /"})
    assert exc.value.code is ErrorCode.COMMAND_NOT_ALLOWED


async def test_bad_arguments_are_ne1109() -> None:
    disp = await _dispatcher_with_mcu()
    with pytest.raises(DaemonError) as exc:
        await disp.run("read_tag", {"device": "mcu1"})  # missing tag
    assert exc.value.code is ErrorCode.COMMAND_NOT_ALLOWED


async def test_extra_argument_is_ne1109() -> None:
    disp = await _dispatcher_with_mcu()
    with pytest.raises(DaemonError) as exc:
        await disp.run("read_tag", {"device": "mcu1", "tag": "temp", "sneaky": 1})
    assert exc.value.code is ErrorCode.COMMAND_NOT_ALLOWED


async def test_command_for_offline_device_is_ne1111() -> None:
    disp = CommandDispatcher(Registry({}))  # no drivers: every device is offline
    with pytest.raises(DaemonError) as exc:
        await disp.run("read_tag", {"device": "ghost", "tag": "x"})
    assert exc.value.code is ErrorCode.DEVICE_OFFLINE


async def test_status_without_device_reports_daemon() -> None:
    disp = await _dispatcher_with_mcu()
    assert (await disp.run("status", {}))["online"] is True


async def test_status_reports_offline_device() -> None:
    disp = await _dispatcher_with_mcu()
    assert (await disp.run("status", {"device": "ghost"}))["online"] is False


def test_allowlist_has_no_dangerous_command() -> None:
    # The allowlist never contains a shell, an allowlist edit or an E-STOP command.
    assert set(ALLOWED_COMMANDS) == {"read_tag", "write_tag", "flash", "status", "reset_driver"}
    for banned in ("shell", "exec", "estop", "set_allowlist", "eval"):
        assert banned not in ALLOWED_COMMANDS
