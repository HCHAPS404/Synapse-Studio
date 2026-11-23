"""E4.5 retention property: nothing older than the window survives, and the one-minute
roll-up preserves the count and sum of each bucket (needs a database)."""

from __future__ import annotations

from datetime import UTC, datetime

from conftest import needs_db
from fastapi import FastAPI
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

from synapse_core.settings import get_settings
from synapse_core.telemetry.retention import NS_PER_MINUTE, run_retention

pytestmark = needs_db

# A fixed "now": samples are placed relative to it.
_NOW = datetime(2026, 11, 23, 12, 0, 0, tzinfo=UTC)
_NOW_NS = int(_NOW.timestamp() * 1e9)
_RAW_RETENTION_S = 7 * 24 * 3600


def _engine(app: FastAPI):  # noqa: ANN202
    from synapse_core.db.session import get_engine

    return get_engine(app.dependency_overrides[get_settings]())


async def _reset(app: FastAPI) -> None:
    from sqlalchemy import delete
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.models import Telemetry1m, TelemetrySample

    engine = _engine(app)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        await session.execute(delete(TelemetrySample))
        await session.execute(delete(Telemetry1m))
        await session.commit()


@settings(
    max_examples=25, deadline=None, suppress_health_check=[HealthCheck.function_scoped_fixture]
)
@given(
    # Ages in seconds before _NOW: a mix of expired (> retention) and fresh samples.
    ages=st.lists(st.integers(min_value=0, max_value=20 * 24 * 3600), min_size=1, max_size=40),
    values=st.lists(st.floats(min_value=-1e6, max_value=1e6), min_size=1, max_size=40),
)
async def test_retention_removes_only_expired_and_preserves_bucket_sums(
    app: FastAPI, ages: list[int], values: list[float]
) -> None:
    from sqlalchemy import func, select
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.models import Telemetry1m, TelemetrySample

    await _reset(app)
    n = min(len(ages), len(values))
    ages, values = ages[:n], values[:n]
    rows = [
        {
            "cell_id": "c",
            "device": "d",
            "tag": "t",
            "t_ns": _NOW_NS - age * 1_000_000_000,
            "value": val,
            "quality": "good",
            "simulated": True,
        }
        for age, val in zip(ages, values, strict=True)
    ]
    engine = _engine(app)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        from sqlalchemy import insert

        await session.execute(insert(TelemetrySample), rows)
        await session.commit()

    raw_cutoff = _NOW_NS - _RAW_RETENTION_S * 1_000_000_000
    expired = [r for r in rows if r["t_ns"] < raw_cutoff]
    # Expected per-minute sums over the expired rows (what the roll-up must preserve).
    expected: dict[int, tuple[int, float]] = {}
    for r in expired:
        minute = r["t_ns"] // NS_PER_MINUTE
        c, s = expected.get(minute, (0, 0.0))
        expected[minute] = (c + 1, s + r["value"])

    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        await run_retention(
            session,
            raw_retention_s=_RAW_RETENTION_S,
            rollup_retention_s=10 * 365 * 24 * 3600,  # keep all roll-ups for the check
            now=_NOW,
        )

    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        # No raw sample older than the window survives.
        oldest = (await session.execute(select(func.min(TelemetrySample.t_ns)))).scalar_one()
        if oldest is not None:
            assert oldest >= raw_cutoff
        # Every fresh sample survives.
        remaining = (
            await session.execute(select(func.count()).select_from(TelemetrySample))
        ).scalar_one()
        assert remaining == len(rows) - len(expired)
        # Each roll-up bucket preserves the count and sum of its expired samples.
        buckets = (await session.execute(select(Telemetry1m))).scalars().all()
        got = {b.minute: (b.count, b.mean_value * b.count) for b in buckets}
        assert set(got) == set(expected)
        for minute, (count, total) in expected.items():
            g_count, g_sum = got[minute]
            assert g_count == count
            assert abs(g_sum - total) <= 1e-3 * max(1.0, abs(total))
