"""E5.10: heartbeat, the read-only E-STOP input, estop_state, advisory handling, safe state."""

from __future__ import annotations

import asyncio
from typing import Any

from synapse_daemon.drivers.base import DriverConfig
from synapse_daemon.drivers.simulated import SimulatedDriver
from synapse_daemon.estop import (
    EstopController,
    EstopState,
    Heartbeat,
    SafeOutputs,
    SimulatedEstopInput,
)


async def _driver(writable: set[str]) -> SimulatedDriver:
    drv = SimulatedDriver()
    await drv.open(DriverConfig(name="plc", writable=frozenset(writable)))
    return drv


def _collector() -> tuple[list[dict[str, Any]], Any]:
    sent: list[dict[str, Any]] = []

    async def sink(frame: dict[str, Any]) -> None:
        sent.append(frame)

    return sent, sink


# --- E-STOP input (read-only) ----------------------------------------------------------


async def test_estop_reports_released_then_tripped() -> None:
    inp = SimulatedEstopInput(EstopState.RELEASED)
    ctrl = EstopController("cell", inp, SafeOutputs({}))
    assert await ctrl.read_state() is EstopState.RELEASED
    inp.set(EstopState.TRIPPED)
    assert await ctrl.read_state() is EstopState.TRIPPED


async def test_estop_unreadable_input_is_unknown() -> None:
    inp = SimulatedEstopInput(EstopState.RELEASED)
    inp.fault()
    ctrl = EstopController("cell", inp, SafeOutputs({}))
    frame = await ctrl.estop_state_frame(request_id="r-1")
    assert frame["type"] == "estop_state" and frame["state"] == "unknown" and frame["id"] == "r-1"


# --- advisory request drives safe outputs and reports the READ state -------------------


async def test_advisory_request_drives_safe_state_and_reports_read_state() -> None:
    drv = await _driver({"motor", "valve"})
    await drv.write("motor", 100)
    await drv.write("valve", True)
    safe = SafeOutputs({"motor": (drv, 0), "valve": (drv, False)})
    inp = SimulatedEstopInput(EstopState.RELEASED)
    ctrl = EstopController("cell", inp, safe)
    sent, sink = _collector()

    state = await ctrl.handle_advisory_request("spill", "r-9", sink)

    # Outputs were driven to their safe value.
    assert (await drv.read("motor")).value == 0
    assert (await drv.read("valve")).value is False
    # The reply is the state the daemon READS (released), not a synthesised "tripped".
    assert state is EstopState.RELEASED
    assert sent[-1]["type"] == "estop_state" and sent[-1]["state"] == "released"
    assert sent[-1]["id"] == "r-9"


async def test_advisory_never_reports_tripped_unless_input_says_so() -> None:
    drv = await _driver({"motor"})
    safe = SafeOutputs({"motor": (drv, 0)})
    inp = SimulatedEstopInput(EstopState.TRIPPED)  # the circuit is actually tripped
    ctrl = EstopController("cell", inp, safe)
    _, sink = _collector()
    assert await ctrl.handle_advisory_request("x", None, sink) is EstopState.TRIPPED


# --- safe state on relay loss ----------------------------------------------------------


async def test_apply_safe_state_on_loss() -> None:
    drv = await _driver({"heater"})
    await drv.write("heater", 1)
    safe = SafeOutputs({"heater": (drv, 0)})
    ctrl = EstopController("cell", SimulatedEstopInput(), safe)
    done = await ctrl.apply_safe_state()
    assert done == ["heater"] and (await drv.read("heater")).value == 0


async def test_one_failing_output_does_not_block_the_rest() -> None:
    good = await _driver({"a"})

    class Failing(SimulatedDriver):
        async def write(self, tag: str, value: Any) -> None:
            raise RuntimeError("driver down")

    bad = Failing()
    await bad.open(DriverConfig(name="bad", writable=frozenset({"b"})))
    safe = SafeOutputs({"b": (bad, 0), "a": (good, 0)})
    ctrl = EstopController("cell", SimulatedEstopInput(), safe)
    done = await ctrl.apply_safe_state()
    assert "a" in done and "b" not in done  # the good output still reached safe state


# --- heartbeat -------------------------------------------------------------------------


async def test_heartbeat_sends_deployment_status() -> None:
    sent, sink = _collector()
    hb = Heartbeat(
        "cell",
        sink,
        devices=lambda: [{"device": "mcu1", "online": True}],
        interval_s=0.05,
    )
    await hb.beat_once()
    assert sent[-1]["type"] == "deployment_status" and sent[-1]["cell"] == "cell"
    assert sent[-1]["devices"][0]["device"] == "mcu1"


async def test_heartbeat_loop_repeats_then_stops() -> None:
    sent, sink = _collector()
    hb = Heartbeat("cell", sink, devices=lambda: [], interval_s=0.02)
    hb.start()
    await asyncio.sleep(0.1)
    await hb.stop()
    count = len(sent)
    assert count >= 2
    await asyncio.sleep(0.05)
    assert len(sent) == count  # no beats after stop
