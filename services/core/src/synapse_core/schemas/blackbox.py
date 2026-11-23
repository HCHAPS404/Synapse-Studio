"""Request/response models for the black box upload route `POST /v1/blackbox` (E6.2).

These mirror the daemon's `synapse_daemon.blackbox` models and the committed
`spec/schemas/blackbox.schema.json`: a record the daemon uploads is validated here at the
trust boundary before it is stored. `extra="forbid"` matches the schema's
`additionalProperties: false`, so an unknown field is rejected (`NE1101`). The record is stored
verbatim so the twin can replay it without a lossy round trip.
"""

from __future__ import annotations

import uuid
from datetime import datetime
from enum import StrEnum

from pydantic import BaseModel, ConfigDict, Field

# A tag value on the wire: JSON number, boolean or string (relay.md "Values").
type Value = bool | int | float | str


class Quality(StrEnum):
    """Sample quality (relay.md)."""

    GOOD = "good"
    STALE = "stale"
    BAD = "bad"


class FaultKind(StrEnum):
    """What froze the window (daemon E6.1)."""

    FAULT = "fault"
    ESTOP = "estop"
    DRIVER_LOSS = "driver_loss"
    WATCHDOG = "watchdog"


class BlackBoxSample(BaseModel):
    """One tag reading in the record."""

    model_config = ConfigDict(extra="forbid")

    tag: str
    value: Value | None = None
    quality: Quality
    t_ns: int = Field(ge=0)


class BlackBoxEvent(BaseModel):
    """One trace event in the record."""

    model_config = ConfigDict(extra="forbid")

    t_ns: int = Field(ge=0)
    device: str
    kind: str
    name: str = ""
    value: str = ""


class BlackBoxRecordIn(BaseModel):
    """A fault record uploaded by a daemon (the body of `POST /v1/blackbox`)."""

    model_config = ConfigDict(extra="forbid")

    record_version: str
    cell: str = Field(min_length=1, max_length=120)
    device: str = Field(min_length=1, max_length=120)
    fault_kind: FaultKind
    fault_code: str = Field(default="", max_length=120)
    at_ns: int = Field(ge=0)
    pre_ns: int = Field(ge=0)
    post_ns: int = Field(ge=0)
    daemon_version: str = ""
    package_hash: str = ""
    samples: list[BlackBoxSample] = Field(default_factory=list)
    events: list[BlackBoxEvent] = Field(default_factory=list)


class BlackBoxUploaded(BaseModel):
    """The id of a stored record. An idempotent re-upload returns the same id."""

    id: uuid.UUID
    cell_id: str
    created_at: datetime
