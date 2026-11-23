"""ORM models. Tables of later tasks are added here with their own Alembic revisions."""

import uuid
from datetime import UTC, datetime
from typing import Any

from sqlalchemy import (
    JSON,
    Boolean,
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


class GuestSandbox(Base):
    """A login-free workspace for a judge (E2.8).

    Quotas are enforced against the counters here. `ip_hash` is a salted SHA-256 of the
    client IP (never the IP itself), used only to rate-limit sandbox creation and to spot
    abuse; it is not reversible. `build_day` marks the UTC date the `builds_today` counter
    belongs to; a build on a new day resets it.
    """

    __tablename__ = "guest_sandboxes"

    id: Mapped[uuid.UUID] = mapped_column(Uuid, primary_key=True, default=uuid.uuid4)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_now
    )
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    ip_hash: Mapped[str | None] = mapped_column(String(64), nullable=True, index=True)
    project_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    builds_today: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    build_day: Mapped[str] = mapped_column(String(10), nullable=False, default="")


class User(Base):
    """A registered user with an argon2 password hash and a role (E3.1).

    There is no self-registration: the first engineer is created by a CLI command and later
    users by an engineer. `role` is one of `viewer`, `operator`, `engineer`. The password
    hash is an Argon2id PHC string; the plain password is never stored or logged.
    """

    __tablename__ = "users"

    id: Mapped[uuid.UUID] = mapped_column(Uuid, primary_key=True, default=uuid.uuid4)
    email: Mapped[str] = mapped_column(String(320), nullable=False, unique=True)
    password_hash: Mapped[str] = mapped_column(Text, nullable=False)
    role: Mapped[str] = mapped_column(String(16), nullable=False)
    disabled: Mapped[bool] = mapped_column(Boolean, nullable=False, default=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_now
    )


class ServiceToken(Base):
    """A long-lived token for a service principal (`daemon` or `twin`), E3.3.

    The token value is a random 256-bit secret shown once at issue; only its SHA-256 is
    stored here, so a database read cannot recover a usable token. `cell_id` binds a daemon
    token to one cell (null for the twin). A revoked token has `revoked_at` set and is
    rejected from then on.
    """

    __tablename__ = "service_tokens"

    id: Mapped[uuid.UUID] = mapped_column(Uuid, primary_key=True, default=uuid.uuid4)
    role: Mapped[str] = mapped_column(String(16), nullable=False)
    cell_id: Mapped[str | None] = mapped_column(String(120), nullable=True, index=True)
    token_hash: Mapped[str] = mapped_column(String(64), nullable=False, unique=True)
    label: Mapped[str] = mapped_column(String(120), nullable=False, default="")
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_now
    )
    revoked_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)


class VersionFile(Base):
    """One file of a version (UTF-8 text)."""

    __tablename__ = "version_files"

    version_id: Mapped[uuid.UUID] = mapped_column(
        ForeignKey("project_versions.id", ondelete="CASCADE"), primary_key=True
    )
    path: Mapped[str] = mapped_column(String(200), primary_key=True)
    content: Mapped[str] = mapped_column(Text, nullable=False)
    size: Mapped[int] = mapped_column(Integer, nullable=False)
