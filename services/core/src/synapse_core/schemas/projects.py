"""Request and response models of the project routes."""

import uuid
from datetime import datetime
from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field, StringConstraints

ProjectName = Annotated[str, StringConstraints(strip_whitespace=True, min_length=1, max_length=120)]


class ProjectCreate(BaseModel):
    """Body of `POST /v1/projects`."""

    model_config = ConfigDict(extra="forbid")
    name: ProjectName


class ProjectUpdate(BaseModel):
    """Body of `PUT /v1/projects/{id}`; `row_version` is the version the client last saw."""

    model_config = ConfigDict(extra="forbid")
    name: ProjectName
    row_version: int = Field(ge=1)


class ProjectOut(BaseModel):
    """A project."""

    model_config = ConfigDict(from_attributes=True)
    id: uuid.UUID
    name: str
    created_at: datetime
    updated_at: datetime
    row_version: int


class VersionCreate(BaseModel):
    """Body of `POST /v1/projects/{id}/versions`: the complete file set of the snapshot.

    Paths and sizes are checked by the same rules as compile jobs (`NE1101`).
    """

    model_config = ConfigDict(extra="forbid")
    files: dict[str, str]
    message: Annotated[str, StringConstraints(max_length=500)] = ""


class VersionSummary(BaseModel):
    """A version without its file contents."""

    id: uuid.UUID
    project_id: uuid.UUID
    number: int
    content_hash: str = Field(description="SHA-256 over the sorted paths and contents")
    message: str
    created_at: datetime
    file_count: int
    total_bytes: int


class FileOut(BaseModel):
    """One file of a version."""

    path: str
    content: str
    size: int


class VersionOut(VersionSummary):
    """A version with all its files, sorted by path."""

    files: list[FileOut]


class ProjectPage(BaseModel):
    """A page of projects."""

    items: list[ProjectOut]
    total: int
    limit: int
    offset: int


class VersionPage(BaseModel):
    """A page of version summaries, newest first."""

    items: list[VersionSummary]
    total: int
    limit: int
    offset: int
