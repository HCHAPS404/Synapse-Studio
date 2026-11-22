"""Request, result and CLI-document models of the compile runner."""

from dataclasses import dataclass, field
from enum import StrEnum
from typing import Literal

from pydantic import BaseModel, Field


class CompileCommand(StrEnum):
    """The `neuron` commands that support `--format json`."""

    CHECK = "check"
    BUILD = "build"
    EMIT = "emit"


class CompileJob(BaseModel):
    """One compile request. Every field is validated again by the runner (`NE1101`).

    `files` maps a relative POSIX path to its UTF-8 text. The options map one to one to
    flags of `neuron`; the runner builds the argument list itself, so no caller text is
    ever an option.
    """

    command: CompileCommand
    files: dict[str, str]
    language: str | None = Field(default=None, description="emit: st, scl, plcopen, ...")
    board: str | None = Field(default=None, description="build: --board")
    native: bool = Field(default=False, description="build: --native")
    host: bool = Field(default=False, description="build: --host")
    profile: str | None = Field(default=None, description="emit: --profile")
    robot: str | None = Field(default=None, description="emit: --robot (a name or a file name)")
    policy: Literal["exact", "equivalent", "approximate"] | None = None
    cpu_hz: int | None = Field(default=None, ge=1, description="check: --cpu-hz")
    ram: int | None = Field(default=None, ge=1, description="check: --ram")


class Diagnostic(BaseModel):
    """A diagnostic as `neuron --format json` reports it (docs/cli-json.md)."""

    code: str | None = None
    severity: str
    message: str
    file: str | None = None
    line: int | None = None
    column: int | None = None
    help: str | None = None
    notes: list[str] = Field(default_factory=list)


class CliArtifact(BaseModel):
    """An entry of the artifact list of the CLI document."""

    path: str
    size: int


class CliDocument(BaseModel):
    """The JSON document of `neuron --format json`, format 1."""

    format: Literal[1]
    command: str
    ok: bool
    neuron_version: str
    diagnostics: list[Diagnostic]
    messages: list[str] = Field(default_factory=list)
    output_dir: str | None = None
    artifacts: list[CliArtifact] = Field(default_factory=list)


class ArtifactInfo(BaseModel):
    """A file the job wrote, as stored with a build."""

    name: str
    size: int
    sha256: str


class CompileResult(BaseModel):
    """What a finished job reports. A program with errors is a result, not an exception."""

    ok: bool
    exit_code: int
    diagnostics: list[Diagnostic]
    messages: list[str] = Field(default_factory=list)
    artifacts: list[ArtifactInfo] = Field(default_factory=list)
    log_tail: str = ""
    binary_version: str | None = None
    duration_ms: float = 0.0


@dataclass(frozen=True)
class CompileOutcome:
    """The result and the bytes of its artifacts (the job directory no longer exists)."""

    result: CompileResult
    files: dict[str, bytes] = field(default_factory=dict)
