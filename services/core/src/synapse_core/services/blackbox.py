"""Store black box records uploaded by a daemon (E6.2).

A daemon uploads a frozen fault window (E6.1). The upload is idempotent on
`(cell_id, device, at_ns)`: a daemon that restarts and retries an already-stored record gets
the same id back and nothing is duplicated, so "upload retries survive restarts" holds without
the daemon tracking server ids. The whole record JSON is stored verbatim so the twin can replay
it (`blackbox_to_replay`, daemon side) without a lossy round trip.
"""

from __future__ import annotations

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.db.models import BlackBoxRecord
from synapse_core.schemas.blackbox import BlackBoxRecordIn


async def store_record(
    session: AsyncSession, record: BlackBoxRecordIn, *, cell_id: str
) -> BlackBoxRecord:
    """Store a record, or return the existing one for the same fault (idempotent).

    `cell_id` is the cell the record belongs to (the daemon's token cell when it is bound to
    one, else the record's own `cell`). Identity is `(cell_id, device, at_ns)`: the instant of
    a fault on one device of one cell is unique enough to recognise a retry.
    """
    existing = (
        await session.execute(
            select(BlackBoxRecord).where(
                BlackBoxRecord.cell_id == cell_id,
                BlackBoxRecord.device == record.device,
                BlackBoxRecord.at_ns == record.at_ns,
            )
        )
    ).scalar_one_or_none()
    if existing is not None:
        return existing
    row = BlackBoxRecord(
        cell_id=cell_id,
        device=record.device,
        fault_kind=record.fault_kind.value,
        fault_code=record.fault_code,
        at_ns=record.at_ns,
        record=record.model_dump(mode="json"),
    )
    session.add(row)
    await session.commit()
    await session.refresh(row)
    return row
