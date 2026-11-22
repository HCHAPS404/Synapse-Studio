"""ORM models. Tables of later tasks are added here with their own Alembic revisions."""

import uuid
from datetime import UTC, datetime
from typing import Any

from sqlalchemy import (
    JSON,
    DateTime,
    ForeignKey,
    Integer,
    LargeBinary,
    String,
    Text,
    UniqueConstraint,
    Uuid,
)
from sqlalchemy.dialects.postgresql import JSONB
from sqlalchemy.orm import Mapped, mapped_column

from synapse_core.db.base import Base


def _now() -> datetime:
    return datetime.now(UTC)


class ServiceMeta(Base):
    """Key/value markers about the schema (baseline table, revision 0001)."""

    __tablename__ = "service_meta"

    key: Mapped[str] = mapped_column(String(64), primary_key=True)
    value: Mapped[str] = mapped_column(Text, nullable=False)


class Project(Base):
    """A named project. `row_version` is the optimistic-concurrency counter (`NE1107`).

    `owner_id` and `sandbox_id` are plain columns until E3 (users) and E2.8 (guest
    sandboxes) add the tables they point to.
    """

    __tablename__ = "projects"

    id: Mapped[uuid.UUID] = mapped_column(Uuid, primary_key=True, default=uuid.uuid4)
    owner_id: Mapped[uuid.UUID | None] = mapped_column(Uuid, nullable=True, index=True)
    sandbox_id: Mapped[uuid.UUID | None] = mapped_column(Uuid, nullable=True, index=True)
    name: Mapped[str] = mapped_column(String(120), nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_now
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_now
    )
    row_version: Mapped[int] = mapped_column(Integer, nullable=False, default=1)


class ProjectVersion(Base):
    """An immutable snapshot of all files of a project. No route or service updates a row."""

    __tablename__ = "project_versions"
    __table_args__ = (UniqueConstraint("project_id", "number"),)

    id: Mapped[uuid.UUID] = mapped_column(Uuid, primary_key=True, default=uuid.uuid4)
    project_id: Mapped[uuid.UUID] = mapped_column(
        ForeignKey("projects.id", ondelete="CASCADE"), nullable=False, index=True
    )
    number: Mapped[int] = mapped_column(Integer, nullable=False)
    content_hash: Mapped[str] = mapped_column(String(64), nullable=False)
    message: Mapped[str] = mapped_column(String(500), nullable=False, default="")
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_now
    )


Json = JSON().with_variant(JSONB(), "postgresql")


class Build(Base):
    """One run of `neuron check`, `build` or `emit` on a version.

    `status`: `queued`, `running`, `succeeded`, `failed` (the program has errors) or
    `error` (the job was rejected, timed out, or the service failed: see `error_code`).
    """

    __tablename__ = "builds"

    id: Mapped[uuid.UUID] = mapped_column(Uuid, primary_key=True, default=uuid.uuid4)
    version_id: Mapped[uuid.UUID] = mapped_column(
        ForeignKey("project_versions.id", ondelete="CASCADE"), nullable=False, index=True
    )
    kind: Mapped[str] = mapped_column(String(16), nullable=False)
    args: Mapped[dict[str, Any]] = mapped_column(Json, nullable=False, default=dict)
    status: Mapped[str] = mapped_column(String(16), nullable=False, default="queued")
    diagnostics: Mapped[list[Any]] = mapped_column(Json, nullable=False, default=list)
    messages: Mapped[list[Any]] = mapped_column(Json, nullable=False, default=list)
    artifacts: Mapped[list[Any]] = mapped_column(Json, nullable=False, default=list)
    log_tail: Mapped[str] = mapped_column(Text, nullable=False, default="")
    binary_version: Mapped[str | None] = mapped_column(String(64), nullable=True)
    exit_code: Mapped[int | None] = mapped_column(Integer, nullable=True)
    error_code: Mapped[str | None] = mapped_column(String(8), nullable=True)
    error_message: Mapped[str | None] = mapped_column(Text, nullable=True)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_now
    )
    started_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    finished_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)


class BuildArtifact(Base):
    """The bytes of one output file of a build; its hash and size are also in `Build.artifacts`."""

    __tablename__ = "build_artifacts"

    build_id: Mapped[uuid.UUID] = mapped_column(
        ForeignKey("builds.id", ondelete="CASCADE"), primary_key=True
    )
    name: Mapped[str] = mapped_column(String(300), primary_key=True)
    sha256: Mapped[str] = mapped_column(String(64), nullable=False)
    size: Mapped[int] = mapped_column(Integer, nullable=False)
    content: Mapped[bytes] = mapped_column(LargeBinary, nullable=False)


class VersionFile(Base):
    """One file of a version (UTF-8 text)."""

    __tablename__ = "version_files"

    version_id: Mapped[uuid.UUID] = mapped_column(
        ForeignKey("project_versions.id", ondelete="CASCADE"), primary_key=True
    )
    path: Mapped[str] = mapped_column(String(200), primary_key=True)
    content: Mapped[str] = mapped_column(Text, nullable=False)
    size: Mapped[int] = mapped_column(Integer, nullable=False)
