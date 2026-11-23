"""Black box: ring buffer, fault window, SQLite store and the record schema (E6.1).

The tests drive synthetic events (no hardware): they feed samples and trace events into the
ring buffer, freeze a window around a synthetic fault, assert the pre/post window is kept, and
check the frozen record validates against the committed JSON Schema and round-trips through the
local SQLite store (including the restart case the upload retry of E6.2 relies on).
"""

from __future__ import annotations

import json
from pathlib import Path

import jsonschema
import pytest

from synapse_daemon.blackbox import (
    DEFAULT_PRE_S,
    RECORD_VERSION,
    BlackBoxEvent,
    BlackBoxRecord,
    FaultKind,
    RingBuffer,
    canonical_schema_text,
    record_schema,
)
from synapse_daemon.blackbox_store import BlackBoxStore
from synapse_daemon.config import DaemonSettings
from synapse_daemon.drivers.base import Quality, Sample

_NS = 1_000_000_000
SCHEMA_PATH = Path(__file__).resolve().parents[3] / "spec" / "schemas" / "blackbox.schema.json"


def _sample(tag: str, value: object, t_ns: int, quality: Quality = Quality.GOOD) -> Sample:
    return Sample(tag=tag, value=value, quality=quality, t_ns=t_ns)


def _fixed_clock(value: int):
    return lambda: value


def test_committed_schema_matches_model() -> None:
    committed = SCHEMA_PATH.read_text(encoding="utf-8")
    assert committed == canonical_schema_text(), (
        "spec/schemas/blackbox.schema.json is out of date; run "
        "`uv run python tools/gen_blackbox_schema.py`"
    )
    # The committed schema parses back to the model schema.
    assert json.loads(committed) == record_schema()


def test_ring_buffer_prunes_items_outside_the_window() -> None:
    # A 10 s window; feed samples across 20 s. Only the last 10 s survive.
    buf = RingBuffer(window_s=10.0)
    for i in range(21):
        buf.add_sample(_sample("x", i, i * _NS))
    rec = buf.freeze(
        cell="c", device="plc", fault_kind=FaultKind.FAULT, at_ns=20 * _NS, pre_s=30.0, post_s=0.0
    )
    # Pruned to the window ending at the newest time (20 s): t in [10 s, 20 s].
    kept = [s.t_ns // _NS for s in rec.samples]
    assert kept == list(range(10, 21))


def test_freeze_keeps_only_the_pre_and_post_window() -> None:
    buf = RingBuffer(window_s=60.0)
    for i in range(0, 60):
        buf.add_sample(_sample("x", i, i * _NS))
    buf.add_event(BlackBoxEvent(t_ns=30 * _NS, device="plc", kind="fault", name="E1"))
    rec = buf.freeze(
        cell="c",
        device="plc",
        fault_kind=FaultKind.FAULT,
        at_ns=30 * _NS,
        fault_code="E1",
        pre_s=10.0,
        post_s=5.0,
    )
    times = [s.t_ns // _NS for s in rec.samples]
    # [at - 10 s, at + 5 s] = [20 s, 35 s].
    assert min(times) == 20 and max(times) == 35
    assert rec.pre_ns == 10 * _NS and rec.post_ns == 5 * _NS
    assert rec.events and rec.events[0].name == "E1"


def test_a_synthetic_fault_yields_a_record_with_pre_and_post_windows() -> None:
    # Simulate the runtime: fill the buffer, a fault at t=15 s, keep feeding for the post window.
    buf = RingBuffer(window_s=30.0)
    for i in range(16):  # 0..15 s pre-fault samples
        buf.add_sample(_sample("speed", i * 10, i * _NS))
    fault_ns = 15 * _NS
    buf.add_event(
        BlackBoxEvent(t_ns=fault_ns, device="arm", kind="fault", name="overspeed", value="true")
    )
    for i in range(16, 21):  # 16..20 s post-fault samples
        buf.add_sample(_sample("speed", 0, i * _NS))
    rec = buf.freeze(
        cell="line",
        device="arm",
        fault_kind=FaultKind.FAULT,
        at_ns=fault_ns,
        fault_code="overspeed",
        pre_s=10.0,
        post_s=5.0,
    )
    pre = [s for s in rec.samples if s.t_ns < fault_ns]
    post = [s for s in rec.samples if s.t_ns > fault_ns]
    assert pre and post, "record must carry both the pre and the post window"
    assert rec.record_version == RECORD_VERSION
    assert rec.fault_kind is FaultKind.FAULT and rec.fault_code == "overspeed"


def test_record_validates_against_the_committed_json_schema() -> None:
    buf = RingBuffer(window_s=30.0)
    buf.add_sample(_sample("ok", True, 1 * _NS))
    buf.add_sample(_sample("bad", None, 2 * _NS, quality=Quality.BAD))
    buf.add_event(BlackBoxEvent(t_ns=2 * _NS, device="plc", kind="fault", name="E1"))
    rec = buf.freeze(
        cell="c", device="plc", fault_kind=FaultKind.ESTOP, at_ns=2 * _NS, pre_s=10.0, post_s=5.0
    )
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    doc = json.loads(rec.model_dump_json())
    jsonschema.validate(doc, schema)  # raises on a mismatch


def test_store_round_trip_and_restart_keeps_pending(tmp_path: Path) -> None:
    db = tmp_path / "blackbox.db"
    store = BlackBoxStore(db)
    buf = RingBuffer(window_s=30.0)
    buf.add_sample(_sample("x", 1, 1 * _NS))
    rec = buf.freeze(
        cell="c", device="plc", fault_kind=FaultKind.WATCHDOG, at_ns=1 * _NS, pre_s=10.0, post_s=5.0
    )
    rid = store.store(rec)
    assert store.count() == 1 and store.count(pending_only=True) == 1
    assert [p.id for p in store.pending()] == [rid]
    got = store.get(rid)
    assert got is not None and got.record == rec and got.uploaded is False
    store.close()

    # A restart: a new store over the same file still sees the record as pending (E6.2 retry).
    store2 = BlackBoxStore(db)
    assert [p.id for p in store2.pending()] == [rid]
    store2.mark_uploaded(rid)
    assert store2.count(pending_only=True) == 0
    assert store2.pending() == []
    store2.close()


def test_settings_default_blackbox_path_and_window() -> None:
    s = DaemonSettings(work_dir="daemon-data")
    assert s.blackbox_path().name == "blackbox.db"
    assert s.blackbox_window_s == 30.0
    assert s.blackbox_pre_s == DEFAULT_PRE_S


def test_bad_record_json_is_rejected_by_the_model() -> None:
    with pytest.raises(ValueError):
        BlackBoxRecord.model_validate_json('{"record_version": "1"}')  # missing required fields
