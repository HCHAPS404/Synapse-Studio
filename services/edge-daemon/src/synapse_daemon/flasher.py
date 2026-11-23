"""Flashing firmware to a device through the same tools as `neuron flash`.

A `Flasher` runs avrdude or copies a UF2 file to a mounted drive, as an argument list (never
a shell), under a per-device lock and a timeout. Tools are resolved from the configuration so
tests inject fake tools; the daemon runs with no real hardware. A non-zero exit, a timeout or
a missing tool is a `DriverError`.
"""

from __future__ import annotations

import asyncio
import shutil
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

from synapse_daemon.drivers.base import DriverError


@dataclass(slots=True)
class FlashTarget:
    """How to flash one device.

    `method` is `avrdude` or `uf2`. For avrdude: `avrdude_bin`, `programmer`, `mcu`, `port`.
    For uf2: `drive` is the mounted bootloader volume the UF2 file is copied to. `timeout_s`
    bounds the operation.
    """

    device: str
    method: str
    avrdude_cmd: list[str] = field(default_factory=lambda: ["avrdude"])
    programmer: str = "arduino"
    mcu: str = "atmega328p"
    port: str = ""
    drive: str = ""
    timeout_s: float = 60.0


@dataclass(slots=True)
class FlashResult:
    """The outcome of a flash: the tool, its exit code and captured output tail."""

    device: str
    method: str
    ok: bool
    exit_code: int
    output: str


@dataclass(slots=True)
class Flasher:
    """Runs flashing tools with a per-device lock so two flashes never overlap."""

    targets: dict[str, FlashTarget] = field(default_factory=dict)
    _locks: dict[str, asyncio.Lock] = field(default_factory=lambda: defaultdict(asyncio.Lock))

    async def flash(self, device: str, artifact: str | Path) -> FlashResult:
        """Flash `artifact` (a .hex or .uf2) to `device`. Raises `DriverError` on failure."""
        target = self.targets.get(device)
        if target is None:
            raise DriverError(f"no flash target configured for device {device!r}")
        art = Path(artifact)
        if not await asyncio.to_thread(art.is_file):
            raise DriverError(f"artifact {art} does not exist")
        async with self._locks[device]:
            if target.method == "avrdude":
                return await self._avrdude(target, art)
            if target.method == "uf2":
                return await self._uf2(target, art)
            raise DriverError(f"unknown flash method {target.method!r}")

    async def _avrdude(self, target: FlashTarget, artifact: Path) -> FlashResult:
        argv = [
            *target.avrdude_cmd,
            "-c",
            target.programmer,
            "-p",
            target.mcu,
            "-P",
            target.port,
            "-U",
            f"flash:w:{artifact}:i",
        ]
        code, out = await self._run(argv, target.timeout_s)
        if code != 0:
            raise DriverError(f"avrdude failed (exit {code}): {out[-200:]}")
        return FlashResult(target.device, "avrdude", True, code, out)

    async def _uf2(self, target: FlashTarget, artifact: Path) -> FlashResult:
        drive = Path(target.drive)
        if not await asyncio.to_thread(drive.is_dir):
            raise DriverError(f"UF2 drive {drive} is not mounted")
        try:
            await asyncio.to_thread(shutil.copy, str(artifact), str(drive / artifact.name))
        except OSError as exc:
            raise DriverError(f"UF2 copy failed: {exc}") from exc
        return FlashResult(target.device, "uf2", True, 0, f"copied {artifact.name} to {drive}")

    async def _run(self, argv: list[str], timeout_s: float) -> tuple[int, str]:
        try:
            proc = await asyncio.create_subprocess_exec(
                *argv,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.STDOUT,
            )
        except FileNotFoundError as exc:
            raise DriverError(f"flash tool not found: {argv[0]}") from exc
        try:
            stdout, _ = await asyncio.wait_for(proc.communicate(), timeout=timeout_s)
        except TimeoutError as exc:
            proc.kill()
            await proc.wait()
            raise DriverError(f"flash timed out after {timeout_s}s") from exc
        return proc.returncode or 0, stdout.decode("utf-8", "replace")
