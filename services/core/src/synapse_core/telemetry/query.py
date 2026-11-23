"""Telemetry range query for `GET /v1/telemetry` (E4.5).

Filters by cell, optional device and tag, and an optional `[from_ns, to_ns)` range, newest
first, capped at `limit` (max 5000). A guest may read only virtual devices (`simulated`),
enforced by the caller passing `virtual_only=True`; the design keeps guests off real-device
telemetry.
"""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.db.models import TelemetrySample

MAX_LIMIT = 5000


@dataclass(frozen=True)
class SampleRow:
    """A telemetry sample as returned by the query."""

    device: str
    tag: str
    t_ns: int
    value: object
    quality: str
    simulated: bool


async def query_samples(
    session: AsyncSession,
    *,
    cell_id: str,
    device: str | None = None,
    tag: str | None = None,
    from_ns: int | None = None,
    to_ns: int | None = None,
    limit: int = 1000,
    virtual_only: bool = False,
) -> Sequence[SampleRow]:
    """Returns matching samples, newest first, capped at `min(limit, MAX_LIMIT)`."""
    limit = max(1, min(limit, MAX_LIMIT))
    stmt = select(TelemetrySample).where(TelemetrySample.cell_id == cell_id)
    if device is not None:
        stmt = stmt.where(TelemetrySample.device == device)
    if tag is not None:
        stmt = stmt.where(TelemetrySample.tag == tag)
    if from_ns is not None:
        stmt = stmt.where(TelemetrySample.t_ns >= from_ns)
    if to_ns is not None:
        stmt = stmt.where(TelemetrySample.t_ns < to_ns)
    if virtual_only:
        stmt = stmt.where(TelemetrySample.simulated.is_(True))
    stmt = stmt.order_by(TelemetrySample.t_ns.desc()).limit(limit)
    rows = (await session.execute(stmt)).scalars().all()
    return [
        SampleRow(
            device=r.device,
            tag=r.tag,
            t_ns=r.t_ns,
            value=r.value,
            quality=r.quality,
            simulated=r.simulated,
        )
        for r in rows
    ]
