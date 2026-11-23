"""The daemon command allowlist and dispatcher.

Commands arrive from the relay as `{name, args}`. The daemon runs a command only when its
name is in the allowlist and its arguments pass the Pydantic model for that command; anything
else is `NE1109`. A command for a device whose driver is not open (offline or not paired) is
`NE1111`. There is no command that runs a shell, edits the allowlist or touches an E-STOP.
"""

from __future__ import annotations

from typing import Any, Protocol

from pydantic import BaseModel, ValidationError

from synapse_daemon.drivers.base import Driver, DriverError, Sample, Value
from synapse_daemon.errors import DaemonError, ErrorCode


class ReadTagArgs(BaseModel):
    """Read one tag from a device."""

    model_config = {"extra": "forbid"}

    device: str
    tag: str


class WriteTagArgs(BaseModel):
    """Write one value to a writable tag. The driver enforces writability too."""

    model_config = {"extra": "forbid"}

    device: str
    tag: str
    value: Value


class FlashArgs(BaseModel):
    """Flash a device from a verified package already on disk."""

    model_config = {"extra": "forbid"}

    device: str
    package_dir: str


class StatusArgs(BaseModel):
    """Report a device's driver status. No device means the whole daemon."""

    model_config = {"extra": "forbid"}

    device: str | None = None


class ResetDriverArgs(BaseModel):
    """Close and reopen a device's driver."""

    model_config = {"extra": "forbid"}

    device: str


# The allowlist: name -> argument model. Any other name is NE1109. Flash is wired to the
# Flasher in E5.5; here it validates and is dispatched to the registered handler.
ALLOWED_COMMANDS: dict[str, type[BaseModel]] = {
    "read_tag": ReadTagArgs,
    "write_tag": WriteTagArgs,
    "flash": FlashArgs,
    "status": StatusArgs,
    "reset_driver": ResetDriverArgs,
}


class DeviceRegistry(Protocol):
    """Resolves a device name to its open driver, or None when offline/unpaired."""

    def driver_for(self, device: str) -> Driver | None: ...


class CommandDispatcher:
    """Validates and runs allowlisted commands against the daemon's drivers."""

    def __init__(self, registry: DeviceRegistry) -> None:
        self._registry = registry

    def validate(self, name: str, args: dict[str, Any]) -> BaseModel:
        """Reject an unknown command or bad arguments with `NE1109`."""
        model = ALLOWED_COMMANDS.get(name)
        if model is None:
            raise DaemonError(ErrorCode.COMMAND_NOT_ALLOWED, f"command {name!r} is not allowed")
        try:
            return model.model_validate(args)
        except ValidationError as exc:
            raise DaemonError(
                ErrorCode.COMMAND_NOT_ALLOWED,
                f"command {name!r} has invalid arguments: {exc.error_count()} error(s)",
            ) from exc

    def _require_driver(self, device: str) -> Driver:
        driver = self._registry.driver_for(device)
        if driver is None:
            raise DaemonError(
                ErrorCode.DEVICE_OFFLINE, f"device {device!r} is offline or not paired"
            )
        return driver

    async def run(self, name: str, args: dict[str, Any]) -> Any:
        """Validate then execute one command. Raises `NE1109`/`NE1111`/driver errors."""
        validated = self.validate(name, args)
        if isinstance(validated, ReadTagArgs):
            return await self._read_tag(validated)
        if isinstance(validated, WriteTagArgs):
            return await self._write_tag(validated)
        if isinstance(validated, StatusArgs):
            return self._status(validated)
        if isinstance(validated, ResetDriverArgs):
            return await self._reset_driver(validated)
        if isinstance(validated, FlashArgs):
            return await self._flash(validated)
        raise DaemonError(ErrorCode.COMMAND_NOT_ALLOWED, "unhandled command")  # pragma: no cover

    async def _read_tag(self, args: ReadTagArgs) -> Sample:
        driver = self._require_driver(args.device)
        return await driver.read(args.tag)

    async def _write_tag(self, args: WriteTagArgs) -> None:
        driver = self._require_driver(args.device)
        # The driver refuses a non-writable tag; a write command never bypasses that.
        await driver.write(args.tag, args.value)

    def _status(self, args: StatusArgs) -> dict[str, Any]:
        if args.device is None:
            return {"online": True}
        driver = self._registry.driver_for(args.device)
        return {"device": args.device, "online": driver is not None}

    async def _reset_driver(self, args: ResetDriverArgs) -> None:
        driver = self._require_driver(args.device)
        await driver.close()
        # Reopening needs the stored config; the runtime registry handles it in E5.10. Here
        # a close is the observable effect and the device must have been online to reset.
        raise_if_no_reopen = getattr(self._registry, "reopen", None)
        if callable(raise_if_no_reopen):
            await raise_if_no_reopen(args.device)

    async def _flash(self, args: FlashArgs) -> None:
        # Flashing is wired to the Flasher in E5.5; the device must be known (paired).
        self._require_driver(args.device)
        flasher = getattr(self._registry, "flash", None)
        if not callable(flasher):
            raise DaemonError(ErrorCode.COMMAND_NOT_ALLOWED, "flashing is not available")
        try:
            await flasher(args.device, args.package_dir)
        except DriverError as exc:
            raise DaemonError(ErrorCode.DEVICE_OFFLINE, str(exc)) from exc
