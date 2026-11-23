"""Black box: a ring buffer that keeps a window around a fault and a local SQLite store (E6.1).

The daemon keeps the last N seconds (default 30 s) of tag samples and trace events in a ring
buffer. A fault (a `FAULT` trace, an E-STOP `tripped`, a driver loss or a watchdog) freezes a
window of `pre_s` seconds before and `post_s` seconds after the fault. The frozen window
becomes a `BlackBoxRecord`: a header (cell, device, fault code, `at_ns`, versions, package
hash), the `samples` and the `events`. The record is JSON validated against
`spec/schemas/blackbox.schema.json` (generated from these Pydantic models, like the other
schemas). It is written to a local SQLite file first so it survives a restart; the upload and
the conversion to the replay format are E6.2.

Nothing here reads or decides a safety state; the E-STOP authority stays the hardwired circuit
(see `estop.py`). A black box is a diagnostic record, not a safety function.
"""

from __future__ import annotations

import time
from collections import deque
from collections.abc import Callable, Iterable
from enum import StrEnum

from pydantic import BaseModel, Field

from synapse_daemon.drivers.base import Quality, Sample, Value

# Format version of a black box record. Bump only on an incompatible change.
RECORD_VERSION = "1"

# Default ring-buffer window and the window frozen around a fault (design E6).
DEFAULT_WINDOW_S = 30.0
DEFAULT_PRE_S = 10.0
DEFAULT_POST_S = 5.0

_NS_PER_S = 1_000_000_000


class FaultKind(StrEnum):
    """What froze the window. Mirrors the fault sources in the design (E6)."""

    FAULT = "fault"
    ESTOP = "estop"
    DRIVER_LOSS = "driver_loss"
    WATCHDOG = "watchdog"


class BlackBoxSample(BaseModel):
    """One tag reading kept in the ring buffer. A `bad`/`stale` read carries no value."""

    model_config = {"extra": "forbid"}

    tag: str = Field(description="Cell tag name")
    value: Value | None = Field(default=None, description="Reading, or null for a failed read")
    quality: Quality = Field(description="Sample quality: good, stale or bad")
    t_ns: int = Field(ge=0, description="Cell-epoch time of the reading, in nanoseconds")

    @classmethod
    def from_sample(cls, s: Sample) -> BlackBoxSample:
        """Build from a driver `Sample`."""
        return cls(tag=s.tag, value=s.value, quality=s.quality, t_ns=s.t_ns)


class BlackBoxEvent(BaseModel):
    """One trace event kept in the ring buffer (`{device, t_ns, kind, name, value}`)."""

    model_config = {"extra": "forbid"}

    t_ns: int = Field(ge=0, description="Cell-epoch time of the event, in nanoseconds")
    device: str = Field(description="Device instance the event belongs to")
    kind: str = Field(description="Event kind as used on the trace channel (e.g. output, fault)")
    name: str = Field(default="", description="Signal, output, fault or machine name")
    value: str = Field(default="", description="Value as text, empty when there is none")


class BlackBoxRecord(BaseModel):
    """A frozen window around a fault: a header plus the samples and events of the window.

    The header identifies the cell and the device that faulted, the fault code and instant,
    and the versions and package hash in force so a record can be tied back to what was running.
    The window is `[at_ns - pre_ns, at_ns + post_ns]`; `samples` and `events` fall inside it,
    ordered by `t_ns`.
    """

    model_config = {"extra": "forbid"}

    record_version: str = Field(description='Format version, "1"')
    cell: str = Field(description="Cell name")
    device: str = Field(description="Device instance that faulted")
    fault_kind: FaultKind = Field(description="What froze the window")
    fault_code: str = Field(default="", description="Fault code or reason text")
    at_ns: int = Field(ge=0, description="Instant of the fault, in nanoseconds")
    pre_ns: int = Field(ge=0, description="Window kept before the fault, in nanoseconds")
    post_ns: int = Field(ge=0, description="Window kept after the fault, in nanoseconds")
    daemon_version: str = Field(default="", description="Daemon version that wrote the record")
    package_hash: str = Field(default="", description="Hash of the deployed package, when known")
    samples: list[BlackBoxSample] = Field(default_factory=list)
    events: list[BlackBoxEvent] = Field(default_factory=list)


def record_schema() -> dict[str, object]:
    """The JSON Schema of a `BlackBoxRecord`. The committed copy is the file under spec/schemas."""
    return BlackBoxRecord.model_json_schema()


def canonical_schema_text() -> str:
    """Canonical JSON text of the record schema (sorted keys, two-space indent, final newline).

    The committed `spec/schemas/blackbox.schema.json` equals this; a test fails on drift.
    """
    import json

    return json.dumps(record_schema(), indent=2, sort_keys=True) + "\n"


class RingBuffer:
    """A time-bounded ring buffer of samples and events, used to freeze a fault window.

    New items are appended with their `t_ns`; items older than `window_ns` behind the newest
    seen time are dropped. `freeze` returns the items within `[at_ns - pre_ns, at_ns + post_ns]`,
    so a caller feeds the post-fault samples in before freezing (the runtime keeps feeding the
    buffer for `post_s` after a fault, then freezes).
    """

    def __init__(
        self,
        *,
        window_s: float = DEFAULT_WINDOW_S,
        clock: Callable[[], int] = time.monotonic_ns,
    ) -> None:
        self._window_ns = int(window_s * _NS_PER_S)
        self._clock = clock
        self._samples: deque[BlackBoxSample] = deque()
        self._events: deque[BlackBoxEvent] = deque()
        self._newest_ns = 0

    def _now_ns(self) -> int:
        return self._clock()

    def _prune(self) -> None:
        cutoff = self._newest_ns - self._window_ns
        while self._samples and self._samples[0].t_ns < cutoff:
            self._samples.popleft()
        while self._events and self._events[0].t_ns < cutoff:
            self._events.popleft()

    def add_sample(self, sample: Sample) -> None:
        """Append one driver sample, pruning anything now outside the window."""
        self._newest_ns = max(self._newest_ns, sample.t_ns)
        self._samples.append(BlackBoxSample.from_sample(sample))
        self._prune()

    def add_samples(self, samples: Iterable[Sample]) -> None:
        """Append a batch of samples."""
        for s in samples:
            self.add_sample(s)

    def add_event(self, event: BlackBoxEvent) -> None:
        """Append one trace event, pruning anything now outside the window."""
        self._newest_ns = max(self._newest_ns, event.t_ns)
        self._events.append(event)
        self._prune()

    def freeze(
        self,
        *,
        cell: str,
        device: str,
        fault_kind: FaultKind,
        at_ns: int,
        fault_code: str = "",
        pre_s: float = DEFAULT_PRE_S,
        post_s: float = DEFAULT_POST_S,
        daemon_version: str = "",
        package_hash: str = "",
    ) -> BlackBoxRecord:
        """Freeze the window `[at_ns - pre_ns, at_ns + post_ns]` into a record.

        Items are taken in `t_ns` order and only within the window, so a record never carries
        data from outside the frozen window even if the buffer holds more.
        """
        pre_ns = int(pre_s * _NS_PER_S)
        post_ns = int(post_s * _NS_PER_S)
        lo = at_ns - pre_ns
        hi = at_ns + post_ns
        samples = sorted((s for s in self._samples if lo <= s.t_ns <= hi), key=lambda s: s.t_ns)
        events = sorted((e for e in self._events if lo <= e.t_ns <= hi), key=lambda e: e.t_ns)
        return BlackBoxRecord(
            record_version=RECORD_VERSION,
            cell=cell,
            device=device,
            fault_kind=fault_kind,
            fault_code=fault_code,
            at_ns=at_ns,
            pre_ns=pre_ns,
            post_ns=post_ns,
            daemon_version=daemon_version,
            package_hash=package_hash,
            samples=samples,
            events=events,
        )
