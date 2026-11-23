"""A simulated in-memory driver.

It holds a dict of tag values, so the daemon can run, relay telemetry and be tested with no
transport at all. It is also the peer the other drivers' tests compare against. A read of an
unknown tag is a `bad`-quality sample; a write to a non-writable tag raises `DriverError`.
"""

from __future__ import annotations

import time

from synapse_daemon.drivers.base import (
    Driver,
    DriverConfig,
    DriverError,
    Quality,
    Sample,
    Value,
)


class SimulatedDriver(Driver):
    """A driver backed by a dict; no I/O. Seeds its values from `cfg.params['values']`."""

    name = "simulated"

    def __init__(self) -> None:
        self._cfg: DriverConfig | None = None
        self._values: dict[str, Value] = {}
        self._open = False

    async def open(self, cfg: DriverConfig) -> None:
        self._cfg = cfg
        seed = cfg.params.get("values", {})
        if isinstance(seed, dict):
            self._values = {str(k): v for k, v in seed.items()}
        self._open = True

    async def read(self, tag: str) -> Sample:
        now = time.monotonic_ns()
        if not self._open or tag not in self._values:
            return Sample.bad(tag, now)
        return Sample(tag=tag, value=self._values[tag], quality=Quality.GOOD, t_ns=now)

    async def write(self, tag: str, value: Value) -> None:
        if not self._open:
            raise DriverError("driver is not open")
        if self._cfg is not None and self._cfg.writable and not self._cfg.is_writable(tag):
            raise DriverError(f"tag {tag!r} is not writable")
        self._values[tag] = value

    async def close(self) -> None:
        self._open = False

    def snapshot(self) -> dict[str, Value]:
        """A copy of the current values, for tests and the local UI."""
        return dict(self._values)
