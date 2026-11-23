"""Ingest relay `telemetry` frames into `telemetry_samples` (E4.4).

A `telemetry` frame carries a cell and a batch of samples (`id`, `t_ns`, `value`, `quality`).
The sample `id` is `<device>.<tag>` (the cell description's tag id), so it is split into a
device and a tag. `simulated` is set by the publisher kind: the twin publishes virtual
devices (`True`), a daemon publishes real devices (`False`), so a later query can keep a guest
to virtual data. Samples are inserted in one batch (`insert().values([...])`, an executemany
under the driver). A storage cap (`SYNAPSE_TELEMETRY_MAX_ROWS`) protects the Neon quota: when
the table already holds the cap, new samples are dropped and a counter is logged instead of
filling the database.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Any

from sqlalchemy import func, insert, select
from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.db.models import TelemetrySample

log = logging.getLogger(__name__)

_QUALITIES = frozenset({"good", "stale", "bad"})


@dataclass(frozen=True)
class IngestResult:
    """How a batch was handled."""

    accepted: int
    dropped: int


def _rows_from_frame(frame: dict[str, Any], *, simulated: bool) -> list[dict[str, Any]]:
    """Builds insert rows from a validated `telemetry` frame.

    A sample whose `id` has no `.` is skipped (it names no device/tag pair); an unknown
    quality is normalised to `bad` (a value that cannot be trusted).
    """
    cell = str(frame["cell"])
    rows: list[dict[str, Any]] = []
    for sample in frame.get("samples", []):
        raw_id = str(sample.get("id", ""))
        device, dot, tag = raw_id.partition(".")
        if not dot or not device or not tag:
            continue
        quality = sample.get("quality", "good")
        rows.append(
            {
                "cell_id": cell,
                "device": device,
                "tag": tag,
                "t_ns": int(sample["t_ns"]),
                "value": sample.get("value"),
                "quality": quality if quality in _QUALITIES else "bad",
                "simulated": simulated,
            }
        )
    return rows


async def ingest_telemetry(
    session: AsyncSession, frame: dict[str, Any], *, simulated: bool, max_rows: int
) -> IngestResult:
    """Inserts the samples of a `telemetry` frame, honouring the storage cap.

    When the table already holds `max_rows` or more, the whole batch is dropped (the cap is a
    guard against filling the Neon quota, not a precise limit). Returns the accepted and
    dropped counts.
    """
    rows = _rows_from_frame(frame, simulated=simulated)
    if not rows:
        return IngestResult(accepted=0, dropped=0)
    current = (
        await session.execute(select(func.count()).select_from(TelemetrySample))
    ).scalar_one()
    if current >= max_rows:
        log.warning(
            "telemetry storage cap reached; dropping samples",
            extra={"cap": max_rows, "dropped": len(rows)},
        )
        return IngestResult(accepted=0, dropped=len(rows))
    await session.execute(insert(TelemetrySample), rows)
    await session.commit()
    return IngestResult(accepted=len(rows), dropped=0)
