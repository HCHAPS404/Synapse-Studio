"""Response models for `GET /v1/telemetry` (E4.5)."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel


class TelemetrySampleOut(BaseModel):
    """One telemetry sample."""

    device: str
    tag: str
    t_ns: int
    value: Any
    quality: str
    simulated: bool


class TelemetryPage(BaseModel):
    """A page of samples, newest first."""

    items: list[TelemetrySampleOut]
    cell_id: str
    limit: int
