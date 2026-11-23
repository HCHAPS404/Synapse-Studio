"""Heartbeat, the read-only E-STOP input, `estop_state` and the advisory request handler.

Safety (E-R15, E-R20, relay.md): the software E-STOP is SUPPLEMENTARY. A hardwired E-STOP is
the authority. The daemon only READS the E-STOP circuit state from a configured digital input;
nothing here releases, bypasses or disables any safety function. An `advisory_estop_request`
makes the daemon drive the outputs it controls to their declared safe state and report the
state it reads; it never sets the E-STOP state itself and never reports `tripped`/`released`
unless the input says so. Nothing claims IEC 61508 or ISO 13849.
"""

from __future__ import annotations

import asyncio
import contextlib
import logging
import time
from collections.abc import Awaitable, Callable, Mapping
from enum import StrEnum
from typing import Protocol

from synapse_daemon.drivers.base import Driver, Value

log = logging.getLogger(__name__)


class EstopState(StrEnum):
    """The reported E-STOP circuit state. `UNKNOWN` when the input cannot be read."""

    RELEASED = "released"
    TRIPPED = "tripped"
    UNKNOWN = "unknown"


class EstopInput(Protocol):
    """A read-only source of the E-STOP circuit state (a digital input)."""

    async def read(self) -> EstopState:
        """Read the current state. Returns `UNKNOWN` when the input cannot be read."""
        ...


class SimulatedEstopInput:
    """A test/offline E-STOP input whose state is set in memory (read-only to the daemon)."""

    def __init__(self, state: EstopState = EstopState.RELEASED, *, readable: bool = True) -> None:
        self._state = state
        self._readable = readable

    def set(self, state: EstopState) -> None:
        """Set the state as the physical circuit would; the daemon still only reads it."""
        self._state = state

    def fault(self, readable: bool = False) -> None:
        """Make the input unreadable, so the daemon reports `unknown`."""
        self._readable = readable

    async def read(self) -> EstopState:
        return self._state if self._readable else EstopState.UNKNOWN


# A sink that sends one relay frame (e.g. the relay client's `send`).
type FrameSink = Callable[[dict[str, object]], Awaitable[None]]


class SafeOutputs:
    """The outputs the daemon controls and the safe value each takes on a stop or a loss.

    `drive_safe` writes every declared safe value through its driver. A write failure is logged
    and does not stop the others, so one dead driver cannot prevent the rest reaching safe state.
    """

    def __init__(self, entries: Mapping[str, tuple[Driver, Value]]) -> None:
        # tag -> (driver, safe value)
        self._entries = dict(entries)

    @property
    def tags(self) -> list[str]:
        return list(self._entries)

    async def drive_safe(self) -> list[str]:
        """Drive every controlled output to its safe value. Returns the tags set."""
        done: list[str] = []
        for tag, (driver, value) in self._entries.items():
            try:
                await driver.write(tag, value)
                done.append(tag)
            except Exception as exc:  # one failing output must not block the others
                log.warning("could not drive %s to safe state: %s", tag, exc)
        return done


class EstopController:
    """Reads the E-STOP input, publishes `estop_state`, and handles advisory requests.

    The controller never decides a state: it reports what `EstopInput.read` returns. On an
    advisory request it drives the safe outputs and then reports the read state. On relay loss
    the runtime calls `apply_safe_state` directly.
    """

    def __init__(
        self,
        cell_id: str,
        estop_input: EstopInput,
        safe_outputs: SafeOutputs,
        *,
        clock: Callable[[], int] = time.monotonic_ns,
    ) -> None:
        self._cell = cell_id
        self._input = estop_input
        self._safe = safe_outputs
        self._clock = clock

    async def read_state(self) -> EstopState:
        """Read the current E-STOP state (read-only)."""
        try:
            return await self._input.read()
        except Exception:  # an input that errors is reported as unknown, never as a state
            return EstopState.UNKNOWN

    async def estop_state_frame(self, request_id: str | int | None = None) -> dict[str, object]:
        """Build the `estop_state` frame carrying the state the daemon reads."""
        from synapse_daemon.relay import frames

        state = await self.read_state()
        return frames.estop_state(self._cell, state.value, self._clock(), request_id)

    async def handle_advisory_request(
        self, reason: str, request_id: str | int | None, sink: FrameSink
    ) -> EstopState:
        """Handle an `advisory_estop_request`: drive safe outputs, then report the read state.

        ADVISORY ONLY. The request does not trip or release the hardwired E-STOP and does not
        change what the input reads; it asks the daemon to put the outputs it controls into
        their safe state. The reply is the state the daemon reads, never a synthesised one.
        """
        log.info("advisory E-STOP request for cell %s: %s", self._cell, reason)
        await self._safe.drive_safe()
        state = await self.read_state()
        await sink(await self.estop_state_frame(request_id))
        return state

    async def apply_safe_state(self) -> list[str]:
        """Drive the controlled outputs to safe state (on relay loss or a read of `tripped`)."""
        return await self._safe.drive_safe()


class Heartbeat:
    """Sends `deployment_status` for the cell on a fixed interval (the daemon heartbeat)."""

    def __init__(
        self,
        cell_id: str,
        sink: FrameSink,
        devices: Callable[[], list[dict[str, object]]],
        *,
        interval_s: float = 5.0,
        clock: Callable[[], int] = time.monotonic_ns,
    ) -> None:
        self._cell = cell_id
        self._sink = sink
        self._devices = devices
        self._interval = interval_s
        self._clock = clock
        self._task: asyncio.Task[None] | None = None
        self._stop = asyncio.Event()

    def frame(self) -> dict[str, object]:
        """One `deployment_status` frame with the current device list."""
        from synapse_daemon.relay import frames

        return frames.deployment_status(self._cell, self._devices())

    async def beat_once(self) -> None:
        """Send a single heartbeat (used by tests and the run loop)."""
        await self._sink(self.frame())

    def start(self) -> None:
        self._stop.clear()
        self._task = asyncio.create_task(self._run())

    async def _run(self) -> None:
        while not self._stop.is_set():
            with contextlib.suppress(Exception):
                await self.beat_once()
            with contextlib.suppress(TimeoutError):
                await asyncio.wait_for(self._stop.wait(), timeout=self._interval)

    async def stop(self) -> None:
        self._stop.set()
        if self._task is not None:
            with contextlib.suppress(asyncio.CancelledError, Exception):
                await self._task
            self._task = None
