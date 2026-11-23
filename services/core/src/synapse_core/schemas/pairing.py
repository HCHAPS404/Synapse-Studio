"""Request and response models for daemon pairing (E4.3)."""

from __future__ import annotations

import uuid
from datetime import datetime

from pydantic import BaseModel, Field


class PairingCreate(BaseModel):
    """Engineer asks for a pairing code bound to a cell."""

    model_config = {"extra": "forbid"}

    cell_id: str = Field(min_length=1, max_length=120)


class PairingIssued(BaseModel):
    """The issued pairing code; the plaintext is shown once."""

    id: uuid.UUID
    cell_id: str
    code: str
    expires_at: datetime


class DeviceClaim(BaseModel):
    """One device a claiming daemon declares."""

    model_config = {"extra": "forbid"}

    name: str = Field(min_length=1, max_length=120)
    kind: str = Field(default="", max_length=64)


class PairingClaim(BaseModel):
    """A daemon claims a code with its name, version and the devices it manages."""

    model_config = {"extra": "forbid"}

    code: str = Field(min_length=1, max_length=32)
    daemon_name: str = Field(min_length=1, max_length=120)
    version: str | None = Field(default=None, max_length=64)
    devices: list[DeviceClaim] = Field(default_factory=list, max_length=200)


class DaemonClaimed(BaseModel):
    """The result of a claim: the daemon id and the service token (shown once)."""

    daemon_id: uuid.UUID
    cell_id: str
    token: str


class DeviceInfo(BaseModel):
    """A device managed by a daemon."""

    name: str
    kind: str
    simulated: bool


class DaemonInfo(BaseModel):
    """A daemon's status (`deployment_status` view) and the devices it manages."""

    id: uuid.UUID
    name: str
    cell_id: str
    version: str | None
    last_heartbeat: datetime | None
    online: bool
    revoked: bool
    devices: list[DeviceInfo]


class DaemonList(BaseModel):
    """A list of daemons."""

    items: list[DaemonInfo]
