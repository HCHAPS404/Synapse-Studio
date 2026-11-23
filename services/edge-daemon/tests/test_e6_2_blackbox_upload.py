"""Convert a record to the replay format and upload with a restart-safe retry (E6.2, daemon).

Synthetic events only: a frozen record is converted to the twin's replay format and validated
against `spec/schemas/replay.schema.json`; the uploader marks a record uploaded only on success,
stops at the first failure, and resumes pending records after a restart of the store.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from synapse_daemon.blackbox import (
    BlackBoxEvent,
    FaultKind,
    RingBuffer,
    blackbox_to_replay,
)
from synapse_daemon.blackbox_store import BlackBoxStore
from synapse_daemon.blackbox_upload import flush_pending, upload_record
from synapse_daemon.drivers.base import Quality, Sample

_NS = 1_000_000_000
REPLAY_SCHEMA = Path(__file__).resolve().parents[3] / "spec" / "schemas" / "replay.schema.json"


def _record_with_window() -> Any:
    buf = RingBuffer(window_s=30.0)
    buf.add_sample(Sample(tag="speed", value=120, quality=Quality.GOOD, t_ns=5 * _NS))
    buf.add_sample(Sample(tag="stop", value=True, quality=Quality.GOOD, t_ns=8 * _NS))
    buf.add_sample(Sample(tag="lost", value=None, quality=Quality.BAD, t_ns=9 * _NS))
    buf.add_event(
        BlackBoxEvent(t_ns=10 * _NS, device="arm", kind="fault", name="overspeed", value="true")
    )
    buf.add_event(
        BlackBoxEvent(t_ns=11 * _NS, device="arm", kind="output", name="brake", value="1")
    )
    return buf.freeze(
        cell="line",
        device="arm",
        fault_kind=FaultKind.FAULT,
        at_ns=10 * _NS,
        fault_code="overspeed",
        pre_s=10.0,
        post_s=5.0,
    )


def test_blackbox_to_replay_validates_against_the_replay_schema() -> None:
    replay = blackbox_to_replay(_record_with_window())
    schema = json.loads(REPLAY_SCHEMA.read_text(encoding="utf-8"))
    jsonschema.validate(replay, schema)  # raises on a mismatch
    assert replay["replay_version"] == "1"
    assert replay["kind"] == "cosimulation"
    assert replay["cell"] == "line"
    # Events are time-ordered and end_ns is the last one.
    times = [e["t_ns"] for e in replay["events"]]
    assert times == sorted(times)
    assert replay["end_ns"] == times[-1]
    # A tag sample became a `signal` event; a bool renders as text.
    signal = next(e for e in replay["events"] if e["name"] == "stop")
    assert signal["kind"] == "signal" and signal["value"] == "true"
    # A fault trace event keeps its kind.
    fault = next(e for e in replay["events"] if e["name"] == "overspeed")
    assert fault["kind"] == "fault"


def test_blackbox_to_replay_maps_unknown_kind_to_signal() -> None:
    buf = RingBuffer(window_s=30.0)
    buf.add_event(BlackBoxEvent(t_ns=1 * _NS, device="d", kind="weird", name="x"))
    rec = buf.freeze(
        cell="c", device="d", fault_kind=FaultKind.WATCHDOG, at_ns=1 * _NS, pre_s=10.0, post_s=5.0
    )
    replay = blackbox_to_replay(rec)
    assert replay["events"][0]["kind"] == "signal"


class _FakeResponse:
    def __init__(self, status_code: int, body: dict[str, Any] | None = None) -> None:
        self.status_code = status_code
        self._body = body or {}

    def json(self) -> dict[str, Any]:
        return self._body


class _FakeHttp:
    """Records POSTs and replies with a scripted sequence of status codes."""

    def __init__(self, statuses: list[int]) -> None:
        self._statuses = list(statuses)
        self.calls: list[dict[str, Any]] = []

    async def post(self, url: str, json: dict[str, Any], headers: dict[str, str]) -> _FakeResponse:
        self.calls.append({"url": url, "json": json, "headers": headers})
        code = self._statuses.pop(0) if self._statuses else 201
        return _FakeResponse(code, {"id": "rec-1"})


@pytest.mark.asyncio
async def test_upload_record_reports_success_and_failure() -> None:
    http_ok = _FakeHttp([201])
    assert await upload_record(http_ok, "https://api", token="svc_x", record_json={"a": 1}) is True
    assert http_ok.calls[0]["url"] == "https://api/v1/blackbox"
    assert http_ok.calls[0]["headers"]["Authorization"] == "Bearer svc_x"

    http_bad = _FakeHttp([500])
    assert await upload_record(http_bad, "https://api", token="t", record_json={}) is False


class _RaisingHttp:
    async def post(self, url: str, json: dict[str, Any], headers: dict[str, str]) -> _FakeResponse:
        raise ConnectionError("down")


@pytest.mark.asyncio
async def test_upload_record_treats_transport_failure_as_retry() -> None:
    assert await upload_record(_RaisingHttp(), "https://api", token="t", record_json={}) is False


@pytest.mark.asyncio
async def test_flush_pending_marks_uploaded_and_retries_after_restart(tmp_path: Path) -> None:
    db = tmp_path / "bb.db"
    store = BlackBoxStore(db)
    store.store(_record_with_window())
    store.store(_record_with_window())  # same fault instant -> still two local rows
    assert store.count(pending_only=True) == 2

    # First flush: the API rejects after the first record, so only one is uploaded.
    http = _FakeHttp([201, 500])
    uploaded = await flush_pending(http, "https://api", token="svc", store=store)
    assert uploaded == 1
    assert store.count(pending_only=True) == 1
    # The uploaded record was sent as the full replayable record JSON (has samples/events).
    assert "samples" in http.calls[0]["json"]
    store.close()

    # A restart: a new store over the same file still sees the pending record and retries it.
    store2 = BlackBoxStore(db)
    assert store2.count(pending_only=True) == 1
    http2 = _FakeHttp([201])
    uploaded2 = await flush_pending(http2, "https://api", token="svc", store=store2)
    assert uploaded2 == 1
    assert store2.count(pending_only=True) == 0
    store2.close()
