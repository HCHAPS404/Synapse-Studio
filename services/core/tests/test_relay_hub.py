"""E4.2 unit tests: the bounded outbox drop policy, hub fan-out matching, and a Hypothesis
round-trip property over every example in `spec/protocols/examples/` (decode→validate→route).

No database and no WebSocket: these exercise the hub and validation directly.
"""

from __future__ import annotations

import json
from pathlib import Path

from hypothesis import given
from hypothesis import strategies as st

from synapse_core.relay.hub import BoundedOutbox, Hub, Session, Subscription
from synapse_core.relay.validate import message_type, validate_frame

_EXAMPLES = Path(__file__).resolve().parents[3] / "spec" / "protocols" / "examples"
_EXAMPLE_FILES = sorted(p for p in _EXAMPLES.glob("*.json") if p.name != "replay_file.json")


def _session(sid: str) -> Session:
    return Session(id=sid, roles=frozenset({"viewer"}), key="k", outbox=BoundedOutbox(256))


# -- bounded outbox drop policy ---------------------------------------------------


def test_outbox_drops_oldest_telemetry_when_full() -> None:
    box = BoundedOutbox(maxsize=3)
    for i in range(3):
        box.put({"type": "telemetry", "n": i})
    box.put({"type": "telemetry", "n": 99})  # full: drop the oldest telemetry (n=0)
    remaining = [box._items[i]["n"] for i in range(len(box))]  # noqa: SLF001
    assert remaining == [1, 2, 99]
    assert box.dropped == 1


def test_outbox_never_drops_control_frames() -> None:
    box = BoundedOutbox(maxsize=2)
    box.put({"type": "ack", "id": "a"})
    box.put({"type": "error", "code": "forbidden"})
    # Full of non-droppable frames; a control frame is still kept (past the soft bound).
    box.put({"type": "estop_state"})
    assert len(box) == 3
    assert box.dropped == 0
    # A telemetry frame now has nothing droppable to evict, so it is the one dropped.
    box.put({"type": "telemetry"})
    assert len(box) == 3
    assert box.dropped == 1


def test_outbox_drops_telemetry_to_make_room_for_control() -> None:
    box = BoundedOutbox(maxsize=2)
    box.put({"type": "telemetry", "n": 1})
    box.put({"type": "trace"})
    box.put({"type": "ack", "id": "x"})  # evicts the oldest droppable (telemetry n=1)
    kinds = [box._items[i]["type"] for i in range(len(box))]  # noqa: SLF001
    assert kinds == ["trace", "ack"]
    assert box.dropped == 1


# -- hub fan-out matching ---------------------------------------------------------


def test_publish_reaches_only_matching_subscribers() -> None:
    hub = Hub()
    a = _session("s-1")
    b = _session("s-2")
    hub.register(a)
    hub.register(b)
    hub.subscribe(a, Subscription(cell="bottling", channels=frozenset({"telemetry"})))
    hub.subscribe(b, Subscription(cell="other", channels=frozenset({"telemetry"})))
    frame = {"v": 1, "type": "telemetry", "cell": "bottling", "samples": []}
    delivered = hub.publish(frame, "bottling")
    assert delivered == 1
    assert len(a.outbox) == 1 and len(b.outbox) == 0


def test_publish_respects_channel_filter() -> None:
    hub = Hub()
    s = _session("s-1")
    hub.register(s)
    hub.subscribe(s, Subscription(cell="c", channels=frozenset({"trace"})))
    # A telemetry frame is not on the `trace` channel, so it is not delivered.
    assert hub.publish({"type": "telemetry", "cell": "c", "samples": []}, "c") == 0
    assert hub.publish({"type": "trace", "cell": "c", "events": []}, "c") == 1


def test_device_scoped_subscription_filters_by_device() -> None:
    hub = Hub()
    s = _session("s-1")
    hub.register(s)
    hub.subscribe(s, Subscription(cell="c", channels=frozenset({"telemetry"}), device="plc1"))
    mine = {"type": "telemetry", "cell": "c", "samples": [{"id": "plc1.level"}]}
    theirs = {"type": "telemetry", "cell": "c", "samples": [{"id": "mcu1.temp"}]}
    assert hub.publish(mine, "c") == 1  # concerns plc1: delivered
    assert hub.publish(theirs, "c") == 0  # concerns mcu1 only: filtered out
    assert len(s.outbox) == 1


def test_snapshot_is_cached_and_replayed() -> None:
    hub = Hub()
    publisher = _session("s-pub")
    hub.register(publisher)
    snap = {"v": 1, "type": "cell_snapshot", "cell": "c", "description": {}}
    hub.publish(snap, "c")  # cached even with no subscriber
    assert hub.cached_snapshot("c") == snap
    late = _session("s-late")
    hub.register(late)
    hub.subscribe(late, Subscription(cell="c", channels=frozenset({"cell"})))
    cached = hub.cached_snapshot("c")
    assert cached is not None and cached["type"] == "cell_snapshot"


# -- message round trip (E-R9, "every example validates; messages round trip") ----


def test_every_example_file_exists() -> None:
    assert _EXAMPLE_FILES, "no relay example files found"


@given(st.sampled_from(_EXAMPLE_FILES))
def test_examples_decode_validate_and_route(path: Path) -> None:
    """Each example message decodes, validates against the schema, and re-serialises to the
    same JSON; a publisher example routes to a matching subscriber."""
    text = path.read_text(encoding="utf-8")
    original = json.loads(text)
    frame = validate_frame(text)
    assert frame == original  # validation does not mutate
    # Re-encode and re-validate: the message round-trips through the schema unchanged.
    assert validate_frame(json.dumps(frame)) == original
    mtype = message_type(frame)
    cell = frame.get("cell")
    if mtype in {"telemetry", "trace", "deployment_status", "estop_state", "sim_state"} and cell:
        hub = Hub()
        sub = _session("s-1")
        hub.register(sub)
        channel = {
            "telemetry": "telemetry",
            "trace": "trace",
            "deployment_status": "deployment",
            "estop_state": "safety",
            "sim_state": "cell",
        }[mtype]
        hub.subscribe(sub, Subscription(cell=str(cell), channels=frozenset({channel})))
        assert hub.publish(frame, str(cell)) == 1
