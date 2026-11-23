"""A local SQLite store for black box records (E6.1).

A record is written here first, as one row holding the record JSON, so it survives a daemon
restart and a cloud outage. The `uploaded_at` column is null until the upload to the Core API
succeeds (E6.2): on restart the daemon re-reads the rows that are still unuploaded and retries,
so no fault record is lost. Access is synchronous `sqlite3` guarded by a lock; the runtime
calls it from a worker thread so the event loop is not blocked.

Writing to disk here is deliberate: the black box must outlive the process that produced it.
"""

from __future__ import annotations

import sqlite3
import threading
import uuid
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from synapse_daemon.blackbox import BlackBoxRecord

_SCHEMA = """
CREATE TABLE IF NOT EXISTS blackbox_records (
    id           TEXT PRIMARY KEY,
    cell         TEXT NOT NULL,
    device       TEXT NOT NULL,
    fault_kind   TEXT NOT NULL,
    fault_code   TEXT NOT NULL DEFAULT '',
    at_ns        INTEGER NOT NULL,
    record_json  TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    uploaded_at  TEXT
);
CREATE INDEX IF NOT EXISTS ix_blackbox_unuploaded
    ON blackbox_records (uploaded_at, at_ns);
"""


@dataclass(frozen=True, slots=True)
class StoredRecord:
    """A row of the store: the record and its upload state."""

    id: str
    cell: str
    device: str
    fault_code: str
    at_ns: int
    record: BlackBoxRecord
    uploaded: bool


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


class BlackBoxStore:
    """A SQLite-backed store of black box records on the daemon's local disk."""

    def __init__(self, path: str | Path) -> None:
        self._path = Path(path)
        self._path.parent.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()
        # check_same_thread=False so a worker thread can use the one connection under the lock.
        self._conn = sqlite3.connect(str(self._path), check_same_thread=False)
        self._conn.row_factory = sqlite3.Row
        with self._lock:
            self._conn.executescript(_SCHEMA)
            self._conn.commit()

    def store(self, record: BlackBoxRecord) -> str:
        """Persist a record, unuploaded. Returns its new id."""
        record_id = str(uuid.uuid4())
        with self._lock:
            self._conn.execute(
                "INSERT INTO blackbox_records "
                "(id, cell, device, fault_kind, fault_code, at_ns, record_json, created_at, "
                " uploaded_at) "
                "VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)",
                (
                    record_id,
                    record.cell,
                    record.device,
                    record.fault_kind.value,
                    record.fault_code,
                    record.at_ns,
                    record.model_dump_json(),
                    _now_iso(),
                ),
            )
            self._conn.commit()
        return record_id

    def mark_uploaded(self, record_id: str) -> None:
        """Mark a record as uploaded so it is not retried."""
        with self._lock:
            self._conn.execute(
                "UPDATE blackbox_records SET uploaded_at = ? WHERE id = ?",
                (_now_iso(), record_id),
            )
            self._conn.commit()

    def _row_to_stored(self, row: sqlite3.Row) -> StoredRecord:
        return StoredRecord(
            id=row["id"],
            cell=row["cell"],
            device=row["device"],
            fault_code=row["fault_code"],
            at_ns=row["at_ns"],
            record=BlackBoxRecord.model_validate_json(row["record_json"]),
            uploaded=row["uploaded_at"] is not None,
        )

    def pending(self, limit: int = 100) -> list[StoredRecord]:
        """Unuploaded records, oldest fault first: what the upload retry (E6.2) sends."""
        with self._lock:
            rows = self._conn.execute(
                "SELECT * FROM blackbox_records WHERE uploaded_at IS NULL "
                "ORDER BY at_ns ASC LIMIT ?",
                (limit,),
            ).fetchall()
        return [self._row_to_stored(r) for r in rows]

    def get(self, record_id: str) -> StoredRecord | None:
        """One record by id, or None."""
        with self._lock:
            row = self._conn.execute(
                "SELECT * FROM blackbox_records WHERE id = ?", (record_id,)
            ).fetchone()
        return self._row_to_stored(row) if row is not None else None

    def count(self, *, pending_only: bool = False) -> int:
        """Rows in the store, optionally only the unuploaded ones."""
        sql = "SELECT COUNT(*) FROM blackbox_records"
        if pending_only:
            sql += " WHERE uploaded_at IS NULL"
        with self._lock:
            return int(self._conn.execute(sql).fetchone()[0])

    def close(self) -> None:
        """Close the connection. Safe to call more than once."""
        with self._lock:
            self._conn.close()
