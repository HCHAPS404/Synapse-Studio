"""Request and response models of the build and capability routes."""

import uuid
from datetime import datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

from synapse_core.compile.models import ArtifactInfo, CompileCommand, Diagnostic

BuildStatus = Literal["queued", "running", "succeeded", "failed", "error"]


class BuildRequest(BaseModel):
    """Body of `POST /v1/versions/{id}/builds`. Options map to flags of `neuron`; which
    ones fit which command is checked when the build is accepted (`NE1101`)."""

    model_config = ConfigDict(extra="forbid")
    kind: CompileCommand
    language: str | None = Field(default=None, description="emit: st, scl, plcopen, ...")
    board: str | None = None
    native: bool = False
    host: bool = False
    profile: str | None = None
    robot: str | None = None
    policy: Literal["exact", "equivalent", "approximate"] | None = None
    cpu_hz: int | None = Field(default=None, ge=1)
    ram: int | None = Field(default=None, ge=1)


class BuildOut(BaseModel):
    """A build. While `status` is `queued` or `running`, the result fields are empty."""

    id: uuid.UUID
    version_id: uuid.UUID
    kind: str
    args: dict[str, object]
    status: BuildStatus
    diagnostics: list[Diagnostic]
    messages: list[str]
    artifacts: list[ArtifactInfo]
    log_tail: str
    binary_version: str | None
    exit_code: int | None
    error_code: str | None = Field(description="NE1101 or NE1102 when status is `error`")
    error_message: str | None
    created_at: datetime
    started_at: datetime | None
    finished_at: datetime | None


class BuildPage(BaseModel):
    """A page of builds, newest first."""

    items: list[BuildOut]
    total: int
    limit: int
    offset: int


class NeuronCapability(BaseModel):
    """The `neuron` binary of this image."""

    available: bool
    version: str | None


class CompileLimits(BaseModel):
    """Limits a job runs under."""

    timeout_s: float
    max_files: int
    max_file_bytes: int
    max_total_bytes: int
    concurrency: int
    queue_max: int


class CapabilitiesOut(BaseModel):
    """Response of `GET /v1/capabilities`: what this image can compile."""

    neuron: NeuronCapability
    commands: list[str]
    emit_languages: list[str]
    toolchains: dict[str, bool] = Field(description="Cross compilers found on PATH")
    sandbox_network: Literal["isolated", "unverified"]
    limits: CompileLimits
