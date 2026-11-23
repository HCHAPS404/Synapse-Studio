"""E5.5: the serial driver over `loop://` and the Flasher with fake tools."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

from synapse_daemon.drivers.base import DriverConfig, DriverError, Quality
from synapse_daemon.drivers.serial import SerialDriver
from synapse_daemon.flasher import Flasher, FlashTarget

HERE = Path(__file__).parent
FAKE_AVRDUDE = str(HERE / "fake_avrdude.py")


# --- serial driver over loop:// --------------------------------------------------------


async def test_serial_write_then_read_round_trips() -> None:
    drv = SerialDriver()
    await drv.open(DriverConfig(name="plc", params={"url": "loop://"}, writable=frozenset({"q"})))
    await drv.write("q", 42)
    s = await drv.read("q")
    assert s.value == 42 and s.quality is Quality.GOOD
    await drv.close()


async def test_serial_read_unknown_tag_is_bad() -> None:
    drv = SerialDriver()
    await drv.open(DriverConfig(name="plc", params={"url": "loop://"}))
    s = await drv.read("never_written")
    assert s.quality is Quality.BAD and s.value is None
    await drv.close()


async def test_serial_write_bool_round_trips() -> None:
    drv = SerialDriver()
    await drv.open(DriverConfig(name="plc", params={"url": "loop://"}, writable=frozenset({"r"})))
    await drv.write("r", True)
    assert (await drv.read("r")).value is True
    await drv.close()


async def test_serial_non_writable_tag_refused() -> None:
    drv = SerialDriver()
    await drv.open(DriverConfig(name="plc", params={"url": "loop://"}, writable=frozenset({"a"})))
    with pytest.raises(DriverError):
        await drv.write("b", 1)
    await drv.close()


async def test_serial_bad_url_raises() -> None:
    drv = SerialDriver()
    with pytest.raises(DriverError):
        await drv.open(DriverConfig(name="plc", params={"url": "not-a-real-scheme://x"}))


# --- Flasher ---------------------------------------------------------------------------


def _avr_target(timeout_s: float = 10.0) -> FlashTarget:
    return FlashTarget(
        device="uno",
        method="avrdude",
        avrdude_cmd=[sys.executable, FAKE_AVRDUDE],
        port="loop://",
        timeout_s=timeout_s,
    )


async def test_flash_avrdude_success(tmp_path: Path) -> None:
    hexfile = tmp_path / "fw.hex"
    hexfile.write_text(":00000001FF\n")
    flasher = Flasher(targets={"uno": _avr_target()})
    result = await flasher.flash("uno", hexfile)
    assert result.ok and result.exit_code == 0 and "fake avrdude" in result.output


async def test_flash_avrdude_nonzero_exit_raises(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setenv("FAKE_AVRDUDE_FAIL", "1")
    hexfile = tmp_path / "fw.hex"
    hexfile.write_text(":00000001FF\n")
    flasher = Flasher(targets={"uno": _avr_target()})
    with pytest.raises(DriverError):
        await flasher.flash("uno", hexfile)


async def test_flash_avrdude_timeout_raises(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setenv("FAKE_AVRDUDE_HANG", "1")
    hexfile = tmp_path / "fw.hex"
    hexfile.write_text(":00000001FF\n")
    flasher = Flasher(targets={"uno": _avr_target(timeout_s=0.5)})
    with pytest.raises(DriverError):
        await flasher.flash("uno", hexfile)


async def test_flash_uf2_copies_to_drive(tmp_path: Path) -> None:
    drive = tmp_path / "RPIBOOT"
    drive.mkdir()
    uf2 = tmp_path / "fw.uf2"
    uf2.write_bytes(b"UF2\n")
    flasher = Flasher(targets={"pico": FlashTarget("pico", "uf2", drive=str(drive))})
    result = await flasher.flash("pico", uf2)
    assert result.ok and (drive / "fw.uf2").read_bytes() == b"UF2\n"


async def test_flash_unknown_device_raises(tmp_path: Path) -> None:
    uf2 = tmp_path / "fw.uf2"
    uf2.write_bytes(b"UF2")
    flasher = Flasher(targets={})
    with pytest.raises(DriverError):
        await flasher.flash("ghost", uf2)


async def test_flash_missing_artifact_raises() -> None:
    flasher = Flasher(targets={"uno": _avr_target()})
    with pytest.raises(DriverError):
        await flasher.flash("uno", "/no/such/file.hex")
