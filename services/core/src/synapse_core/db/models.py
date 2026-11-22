"""ORM models. Tables of later tasks are added here with their own Alembic revisions."""

import uuid
from datetime import UTC, datetime

from sqlalchemy import DateTime, ForeignKey, Integer, String, Text, UniqueConstraint, Uuid
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


class VersionFile(Base):
    """One file of a version (UTF-8 text)."""

    __tablename__ = "version_files"

    version_id: Mapped[uuid.UUID] = mapped_column(
        ForeignKey("project_versions.id", ondelete="CASCADE"), primary_key=True
    )
    path: Mapped[str] = mapped_column(String(200), primary_key=True)
    content: Mapped[str] = mapped_column(Text, nullable=False)
    size: Mapped[int] = mapped_column(Integer, nullable=False)
