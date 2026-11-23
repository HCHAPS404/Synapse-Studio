"""Telemetry retention and the one-minute roll-up (E4.5).

A task started in the lifespan runs on a timer. It is guarded by a Postgres advisory lock
(`pg_try_advisory_lock`) so that, even with more than one process, only one instance rolls up
and deletes at a time (on SQLite the lock is a no-op and the single local process just runs).
Each pass:

1. takes a chunk (10 000) of raw samples older than the raw-retention window;
2. rolls them into `telemetry_1m` first — a bucket is `(cell, device, tag, minute)` and its
   `count`/`mean` are merged with any existing bucket so the running count and sum are
   preserved across passes — then deletes exactly those raw rows (rolled up once);
3. deletes `telemetry_1m` rows older than the roll-up retention window.

Numeric values (numbers and booleans) are aggregated; a string sample contributes to neither
the count nor the stats of a numeric series (min/max/mean are over numbers only).
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from datetime import UTC, datetime

from sqlalchemy import delete, select
from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.db.models import Telemetry1m, TelemetrySample

log = logging.getLogger(__name__)

NS_PER_MINUTE = 60_000_000_000
# A fixed key for the telemetry retention advisory lock (arbitrary but stable).
_ADVISORY_LOCK_KEY = 0x5_7E1E_3E79


@dataclass
class RetentionResult:
    """Counts from one retention pass."""

    rolled_up: int
    raw_deleted: int
    rollup_deleted: int
    locked: bool = True


def _as_number(value: object) -> float | None:
    """A sample value as a float, or `None` when it is not numeric (a string)."""
    if isinstance(value, bool):
        return 1.0 if value else 0.0
    if isinstance(value, (int, float)):
        return float(value)
    return None


async def _try_advisory_lock(session: AsyncSession) -> bool:
    """Takes the Postgres advisory lock if on Postgres; `True` when held (or not Postgres)."""
    if session.bind is None or session.bind.dialect.name != "postgresql":
        return True
    from sqlalchemy import text

    got = (
        await session.execute(text("SELECT pg_try_advisory_lock(:k)"), {"k": _ADVISORY_LOCK_KEY})
    ).scalar_one()
    return bool(got)


async def _release_advisory_lock(session: AsyncSession) -> None:
    if session.bind is None or session.bind.dialect.name != "postgresql":
        return
    from sqlalchemy import text

    await session.execute(text("SELECT pg_advisory_unlock(:k)"), {"k": _ADVISORY_LOCK_KEY})


async def run_retention(
    session: AsyncSession,
    *,
    raw_retention_s: int,
    rollup_retention_s: int,
    now: datetime | None = None,
    chunk: int = 10_000,
) -> RetentionResult:
    """Rolls up then deletes expired raw samples and expired roll-up rows under the lock."""
    if not await _try_advisory_lock(session):
        return RetentionResult(rolled_up=0, raw_deleted=0, rollup_deleted=0, locked=False)
    try:
        now_ns = int((now or datetime.now(UTC)).timestamp() * 1e9)
        raw_cutoff = now_ns - raw_retention_s * 1_000_000_000
        rolled, raw_deleted = await _rollup_and_delete_raw(session, raw_cutoff, chunk)
        rollup_cutoff_minute = (now_ns - rollup_retention_s * 1_000_000_000) // NS_PER_MINUTE
        result = await session.execute(
            delete(Telemetry1m).where(Telemetry1m.minute < rollup_cutoff_minute)
        )
        await session.commit()
        return RetentionResult(
            rolled_up=rolled,
            raw_deleted=raw_deleted,
            rollup_deleted=int(result.rowcount or 0),
        )
    finally:
        await _release_advisory_lock(session)


async def _rollup_and_delete_raw(
    session: AsyncSession, raw_cutoff: int, chunk: int
) -> tuple[int, int]:
    """Rolls a chunk of expired raw samples into `telemetry_1m` and deletes them."""
    rows = (
        (
            await session.execute(
                select(TelemetrySample)
                .where(TelemetrySample.t_ns < raw_cutoff)
                .order_by(TelemetrySample.t_ns)
                .limit(chunk)
            )
        )
        .scalars()
        .all()
    )
    if not rows:
        return 0, 0
    # Aggregate per bucket in memory (the chunk is bounded).
    buckets: dict[tuple[str, str, str, int, bool], list[float]] = {}
    for r in rows:
        number = _as_number(r.value)
        if number is None:
            continue
        key = (r.cell_id, r.device, r.tag, r.t_ns // NS_PER_MINUTE, r.simulated)
        buckets.setdefault(key, []).append(number)
    for (cell, device, tag, minute, simulated), values in buckets.items():
        await _merge_bucket(session, cell, device, tag, minute, simulated, values)
    ids = [r.id for r in rows]
    await session.execute(delete(TelemetrySample).where(TelemetrySample.id.in_(ids)))
    return len(buckets), len(ids)


async def _merge_bucket(
    session: AsyncSession,
    cell: str,
    device: str,
    tag: str,
    minute: int,
    simulated: bool,
    values: list[float],
) -> None:
    """Merges a batch of numeric values into the `(cell, device, tag, minute)` roll-up row."""
    existing = (
        await session.execute(
            select(Telemetry1m).where(
                Telemetry1m.cell_id == cell,
                Telemetry1m.device == device,
                Telemetry1m.tag == tag,
                Telemetry1m.minute == minute,
            )
        )
    ).scalar_one_or_none()
    count = len(values)
    total = sum(values)
    lo = min(values)
    hi = max(values)
    if existing is None:
        session.add(
            Telemetry1m(
                cell_id=cell,
                device=device,
                tag=tag,
                minute=minute,
                min_value=lo,
                max_value=hi,
                mean_value=total / count,
                count=count,
                simulated=simulated,
            )
        )
    else:
        merged_count = existing.count + count
        merged_sum = existing.mean_value * existing.count + total
        existing.min_value = min(existing.min_value, lo)
        existing.max_value = max(existing.max_value, hi)
        existing.mean_value = merged_sum / merged_count
        existing.count = merged_count
