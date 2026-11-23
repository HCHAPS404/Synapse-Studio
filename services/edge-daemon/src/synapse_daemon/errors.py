"""Daemon-side error codes, a subset of the services catalogue (docs/errors/services.md).

The daemon reuses the `NE11xx` family so a failure on the daemon has the same code as the
matching failure in the Core API. Each code has an entry in `docs/errors/services.md`.
"""

from enum import StrEnum


class ErrorCode(StrEnum):
    """Codes the daemon raises; a subset of the shared services catalogue."""

    SIGNATURE_INVALID = "NE1108"
    COMMAND_NOT_ALLOWED = "NE1109"
    DEPLOYMENT_BLOCKED = "NE1110"
    DEVICE_OFFLINE = "NE1111"


class DaemonError(Exception):
    """A daemon failure carrying a catalogue code."""

    def __init__(self, code: ErrorCode, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message

    def __str__(self) -> str:  # pragma: no cover - trivial
        return f"{self.code.value}: {self.message}"
