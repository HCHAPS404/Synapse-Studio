"""Calibration pipeline: synthetic round trips to a timing profile TOML (E6.3, daemon side).

Synthetic samples only; real numbers come from session I1. The tests check the reduction
(transport = p99.9 - clock offset, jitter = p99.9 - median) and that the TOML is in the
manifest `[timing]` shape the Rust `neuron cell check --profile` reads.
"""

from __future__ import annotations

import tomllib

import pytest

from synapse_daemon.calibrate import CalibrationProfile, reduce_round_trips


def test_reduce_round_trips_computes_transport_and_jitter() -> None:
    # 100 samples around 2 ms with a tail at 3 ms.
    us = 1_000
    samples = [2000 * us] * 99 + [3000 * us]
    prof = reduce_round_trips(
        "generic_s7_1200", "modbus_tcp", samples, clock_offset_ns=0, measured_at="2026-11-24"
    )
    assert prof.samples == 100
    # p99.9 of the sorted list is the last element (the tail).
    assert prof.transport_ns == 3000 * us
    # jitter = p99.9 - median (2 ms).
    assert prof.jitter_ns == 1000 * us
    assert prof.trust == "measured"
    assert prof.device == "generic_s7_1200" and prof.medium == "modbus_tcp"


def test_clock_offset_is_subtracted_from_transport() -> None:
    samples = [5_000_000] * 50
    prof = reduce_round_trips("d", "m", samples, clock_offset_ns=1_000_000)
    # p99.9 = 5 ms, minus a 1 ms offset -> 4 ms one-way worst case; jitter 0 (no spread).
    assert prof.transport_ns == 4_000_000
    assert prof.jitter_ns == 0


def test_transport_never_goes_negative() -> None:
    prof = reduce_round_trips("d", "m", [1_000_000] * 40, clock_offset_ns=5_000_000)
    assert prof.transport_ns == 0


def test_to_toml_is_a_valid_profile_table() -> None:
    prof = reduce_round_trips(
        "generic_s7_1200",
        "modbus_tcp",
        [2_000_000] * 1999 + [2_500_000],
        measured_at="2026-11-24",
        tool="synapse-daemon 0.1.0",
    )
    text = prof.to_toml()
    doc = tomllib.loads(text)
    p = doc["profile"]
    assert p["device"] == "generic_s7_1200"
    assert p["medium"] == "modbus_tcp"
    assert p["trust"] == "measured"
    assert p["samples"] == 2000
    # Durations are strings with a unit, as the manifest `[timing]` uses.
    assert p["transport"].endswith((" us", " ms", " s", " ns"))
    assert p["jitter"].endswith((" us", " ms", " s", " ns"))
    assert p["measured_at"] == "2026-11-24"
    assert p["tool"] == "synapse-daemon 0.1.0"


def test_duration_formatting_uses_whole_units() -> None:
    prof = CalibrationProfile("d", "m", samples=30, transport_ns=2_000_000, jitter_ns=500_000)
    doc = tomllib.loads(prof.to_toml())["profile"]
    assert doc["transport"] == "2 ms"
    assert doc["jitter"] == "500 us"


def test_no_samples_is_an_error() -> None:
    with pytest.raises(ValueError):
        reduce_round_trips("d", "m", [])


def test_negative_round_trip_is_an_error() -> None:
    with pytest.raises(ValueError):
        reduce_round_trips("d", "m", [1_000, -1])
