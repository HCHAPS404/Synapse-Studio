"""Calibration pipeline: measured round trips to a timing profile (E6.3, requirement E-R17).

`synapse-daemon calibrate` sends probe messages through a driver and records round-trip times.
This module turns those samples into a profile file in the manifest `[timing]` shape, one per
device and medium:

    [profile]
    device = "generic_s7_1200"
    medium = "modbus_tcp"
    trust = "measured"
    samples = 2000
    transport = "2 ms"
    jitter = "1 ms"
    measured_at = "2026-11-24"
    tool = "synapse-daemon 0.1.0"

`neuron cell check --profile file.toml` (Rust, E6.3) applies it, overriding the manifest timing
for that device and medium. The reduction is: `transport` = p99.9 round trip minus the clock
offset estimate (a one-way worst case), `jitter` = p99.9 minus the median. Profiles need a
minimum number of samples to be `measured` (the Rust side enforces `NE1112`); the numbers here
are synthetic until session I1 measures real devices, and a real profile is a data-only commit
to `devices/**`.
"""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from datetime import UTC, datetime

from synapse_daemon import __version__

# p99.9: the near-worst sample, robust to one outlier. Fewer than this many samples has no
# meaningful tail, so the Rust side rejects a `measured` profile below MIN_MEASURED_SAMPLES (30).
_P = 0.999


def _percentile(sorted_ns: Sequence[int], p: float) -> int:
    """The p-th percentile of a non-empty sorted sequence (nearest-rank)."""
    n = len(sorted_ns)
    idx = min(n - 1, max(0, round(p * (n - 1))))
    return sorted_ns[idx]


def _median(sorted_ns: Sequence[int]) -> int:
    n = len(sorted_ns)
    mid = n // 2
    if n % 2 == 1:
        return sorted_ns[mid]
    return (sorted_ns[mid - 1] + sorted_ns[mid]) // 2


def _fmt_ns(ns: int) -> str:
    """Render nanoseconds as a manifest duration with the largest unit keeping whole steps."""
    for div, unit in ((1_000_000_000, "s"), (1_000_000, "ms"), (1_000, "us")):
        if ns >= div and ns % div == 0:
            return f"{ns // div} {unit}"
    return f"{ns} ns"


@dataclass(frozen=True, slots=True)
class CalibrationProfile:
    """The reduced profile for one device on one medium."""

    device: str
    medium: str
    samples: int
    transport_ns: int
    jitter_ns: int
    trust: str = "measured"
    measured_at: str = ""
    tool: str = ""

    def to_toml(self) -> str:
        """The profile TOML in the manifest `[timing]` shape (what `--profile` reads)."""
        lines = [
            "[profile]",
            f'device = "{self.device}"',
            f'medium = "{self.medium}"',
            f'trust = "{self.trust}"',
            f"samples = {self.samples}",
            f'transport = "{_fmt_ns(self.transport_ns)}"',
            f'jitter = "{_fmt_ns(self.jitter_ns)}"',
        ]
        if self.measured_at:
            lines.append(f'measured_at = "{self.measured_at}"')
        if self.tool:
            lines.append(f'tool = "{self.tool}"')
        return "\n".join(lines) + "\n"


def reduce_round_trips(
    device: str,
    medium: str,
    round_trip_ns: Sequence[int],
    *,
    clock_offset_ns: int = 0,
    measured_at: str | None = None,
    tool: str | None = None,
) -> CalibrationProfile:
    """Reduce measured round-trip times to a `(transport, jitter)` profile.

    `transport` is the p99.9 round trip minus the estimated clock offset (a one-way worst case,
    never below zero). `jitter` is the p99.9 minus the median. The sample count is recorded so
    the Rust side can reject a thin `measured` profile (`NE1112`).

    Raises `ValueError` on no samples or a negative round-trip time (a measurement error).
    """
    if not round_trip_ns:
        raise ValueError("no round-trip samples to reduce")
    if any(x < 0 for x in round_trip_ns):
        raise ValueError("a round-trip time cannot be negative")
    s = sorted(round_trip_ns)
    p999 = _percentile(s, _P)
    transport = max(0, p999 - max(0, clock_offset_ns))
    jitter = max(0, p999 - _median(s))
    return CalibrationProfile(
        device=device,
        medium=medium,
        samples=len(s),
        transport_ns=transport,
        jitter_ns=jitter,
        measured_at=measured_at or datetime.now(UTC).date().isoformat(),
        tool=tool or f"synapse-daemon {__version__}",
    )
