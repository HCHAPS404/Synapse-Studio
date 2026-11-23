"""The `Driver` protocol, its shared value types and the mapping a driver reads.

A driver opens a transport to one device, reads and writes tags, and closes. Tags are the
names the cell's link table (D3) assigns; a driver's `tag_map` says which register, address
or frame each tag maps to. A read that fails returns a `Sample` of `bad` quality; a driver
never returns a guessed value.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import StrEnum
from typing import Protocol, runtime_checkable

# A tag value on the wire: JSON number, boolean or string (relay.md "Values").
type Value = bool | int | float | str


class Quality(StrEnum):
    """Sample quality (relay.md): a healthy read, a stale read, or a failed read."""

    GOOD = "good"
    STALE = "stale"
    BAD = "bad"


@dataclass(frozen=True, slots=True)
class Sample:
    """One reading of a tag with its quality and the cell-epoch time it was taken."""

    tag: str
    value: Value | None
    quality: Quality
    t_ns: int

    @classmethod
    def bad(cls, tag: str, t_ns: int) -> Sample:
        """A failed read: no value, `bad` quality. Never a guessed value."""
        return cls(tag=tag, value=None, quality=Quality.BAD, t_ns=t_ns)


class DriverError(Exception):
    """A transport or protocol failure inside a driver."""


@dataclass(slots=True)
class DriverConfig:
    """How to reach one device and how each tag maps to the transport.

    `params` holds transport-specific settings (host, port, serial url, unit id, ...).
    `tag_map` maps a cell tag to a transport address description (register, coil, topic,
    CAN id, ...). `writable` lists the tags the daemon may write (a write to any other tag
    is refused by the command allowlist, E5.3). Timeouts and retries are shared.
    """

    name: str
    params: dict[str, object] = field(default_factory=dict)
    tag_map: dict[str, object] = field(default_factory=dict)
    writable: frozenset[str] = frozenset()
    timeout_s: float = 2.0
    retries: int = 2
    backoff_s: float = 0.1

    def is_writable(self, tag: str) -> bool:
        """Whether the daemon is allowed to write this tag."""
        return tag in self.writable


@runtime_checkable
class Driver(Protocol):
    """One device behind a transport. Implementations are async and hold no global state."""

    name: str

    async def open(self, cfg: DriverConfig) -> None:
        """Open the transport. Raises `DriverError` when the peer cannot be reached."""
        ...

    async def read(self, tag: str) -> Sample:
        """Read one tag. Returns a `bad`-quality sample on failure, never a guess."""
        ...

    async def write(self, tag: str, value: Value) -> None:
        """Write one tag. Raises `DriverError` on failure."""
        ...

    async def close(self) -> None:
        """Close the transport. Safe to call when already closed."""
        ...
