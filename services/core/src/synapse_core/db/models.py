"""ORM models. Tables of later tasks are added here with their own Alembic revisions."""

import uuid
from datetime import UTC, datetime
from typing import Any

from sqlalchemy import (
    JSON,
    BigInteger,
    Boolean,
    DateTime,
    ForeignKey,
    Index,
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

# A 64-bit autoincrement primary key: BIGINT on Postgres, INTEGER on SQLite (only an
# `INTEGER PRIMARY KEY` autoincrements on SQLite, so the local test schema works too).
_BigId = BigInteger().with_variant(Integer, "sqlite")


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


class AuditLog(Base):
    """An append-only record of a sensitive action (E3.4).

    One row per audited action: who did it (`actor`, `role`), what (`action`, `target`),
    the `request_id` that correlates it with the logs, the `result`, and a small `detail`
    object that never holds a token, a password or a connection string. The application
    database role is granted only `INSERT` and `SELECT` (migration `GRANT`), so there is no
    update or delete path; the table is the authority on what happened.
    """

    __tablename__ = "audit_log"

    id: Mapped[uuid.UUID] = mapped_column(Uuid, primary_key=True, default=uuid.uuid4)
    at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_now, index=True
    )
    actor: Mapped[str] = mapped_column(String(120), nullable=False)
    role: Mapped[str] = mapped_column(String(16), nullable=False)
    action: Mapped[str] = mapped_column(String(64), nullable=False, index=True)
    target: Mapped[str | None] = mapped_column(String(300), nullable=True)
    request_id: Mapped[str | None] = mapped_column(String(64), nullable=True)
    result: Mapped[str] = mapped_column(String(16), nullable=False)
    detail: Mapped[dict[str, Any]] = mapped_column(Json, nullable=False, default=dict)


class PairingCode(Base):
    """A one-time code that pairs a daemon to a cell (E4.3).

    An engineer issues a code for a cell; it is short-lived (10 minutes) and stored only as a
    SHA-256 hash, so a database read cannot replay it. A successful claim sets `claimed_at`,
    so the same code cannot be used twice. `service_token_id` links to the token the claim
    issued (for audit and unpair).
    """

    __tablename__ = "pairing_codes"

    id: Mapped[uuid.UUID] = mapped_column(Uuid, primary_key=True, default=uuid.uuid4)
    code_hash: Mapped[str] = mapped_column(String(64), nullable=False, unique=True)
    cell_id: Mapped[str] = mapped_column(String(120), nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_now
    )
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    claimed_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    daemon_id: Mapped[uuid.UUID | None] = mapped_column(Uuid, nullable=True)


class Daemon(Base):
    """An Edge Daemon paired to a cell (E4.3).

    `last_heartbeat` is updated from the daemon's `deployment_status` frames; the API treats
    the daemon as offline when it has not been heard from within the heartbeat timeout. A
    revoked daemon (`revoked_at`) is unpaired: its service token is revoked and its relay
    sockets are closed.
    """

    __tablename__ = "daemons"

    id: Mapped[uuid.UUID] = mapped_column(Uuid, primary_key=True, default=uuid.uuid4)
    name: Mapped[str] = mapped_column(String(120), nullable=False)
    cell_id: Mapped[str] = mapped_column(String(120), nullable=False, index=True)
    service_token_id: Mapped[uuid.UUID] = mapped_column(Uuid, nullable=False)
    version: Mapped[str | None] = mapped_column(String(64), nullable=True)
    last_heartbeat: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=_now
    )
    revoked_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)


class Device(Base):
    """A device a daemon manages (E4.3). Real devices only (`simulated` defaults to false)."""

    __tablename__ = "devices"

    daemon_id: Mapped[uuid.UUID] = mapped_column(
        ForeignKey("daemons.id", ondelete="CASCADE"), primary_key=True
    )
    name: Mapped[str] = mapped_column(String(120), primary_key=True)
    kind: Mapped[str] = mapped_column(String(64), nullable=False, default="")
    simulated: Mapped[bool] = mapped_column(Boolean, nullable=False, default=False)


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


class TelemetrySample(Base):
    """One telemetry sample ingested from a relay `telemetry` frame (E4.4).

    Plain Postgres (no TimescaleDB). `simulated` flags whether the sample came from the twin
    (virtual device) or a daemon (real device), so a query can keep a guest to virtual data.
    `t_ns` is nanoseconds since the cell epoch. The composite index
    `(cell_id, device, tag, t_ns)` serves the range query of `GET /v1/telemetry` (E4.5).
    Raw rows are kept 7 days (retention, E4.5).
    """

    __tablename__ = "telemetry_samples"
    __table_args__ = (Index("ix_telemetry_samples_series", "cell_id", "device", "tag", "t_ns"),)

    id: Mapped[int] = mapped_column(_BigId, primary_key=True, autoincrement=True)
    cell_id: Mapped[str] = mapped_column(String(120), nullable=False)
    device: Mapped[str] = mapped_column(String(120), nullable=False)
    tag: Mapped[str] = mapped_column(String(200), nullable=False)
    t_ns: Mapped[int] = mapped_column(BigInteger, nullable=False)
    value: Mapped[Any] = mapped_column(Json, nullable=False)
    quality: Mapped[str] = mapped_column(String(8), nullable=False, default="good")
    simulated: Mapped[bool] = mapped_column(Boolean, nullable=False, default=True)


class Telemetry1m(Base):
    """A one-minute roll-up of raw samples for a series (E4.5).

    One row per `(cell_id, device, tag, minute)` with the min, max, mean and count of the raw
    samples in that minute. Kept 30 days. Written by the roll-up task before the raw rows are
    deleted, so a count/sum within a bucket is preserved.
    """

    __tablename__ = "telemetry_1m"
    __table_args__ = (
        UniqueConstraint(
            "cell_id", "device", "tag", "minute", name="uq_telemetry_1m_series_minute"
        ),
        Index("ix_telemetry_1m_series", "cell_id", "device", "tag", "minute"),
    )

    id: Mapped[int] = mapped_column(_BigId, primary_key=True, autoincrement=True)
    cell_id: Mapped[str] = mapped_column(String(120), nullable=False)
    device: Mapped[str] = mapped_column(String(120), nullable=False)
    tag: Mapped[str] = mapped_column(String(200), nullable=False)
    minute: Mapped[int] = mapped_column(BigInteger, nullable=False)  # epoch minute (ns // 60e9)
    min_value: Mapped[float] = mapped_column(nullable=False)
    max_value: Mapped[float] = mapped_column(nullable=False)
    mean_value: Mapped[float] = mapped_column(nullable=False)
    count: Mapped[int] = mapped_column(Integer, nullable=False)
    simulated: Mapped[bool] = mapped_column(Boolean, nullable=False, default=True)


class VersionFile(Base):
    """One file of a version (UTF-8 text)."""

    __tablename__ = "version_files"

    version_id: Mapped[uuid.UUID] = mapped_column(
        ForeignKey("project_versions.id", ondelete="CASCADE"), primary_key=True
    )
    path: Mapped[str] = mapped_column(String(200), primary_key=True)
    content: Mapped[str] = mapped_column(Text, nullable=False)
    size: Mapped[int] = mapped_column(Integer, nullable=False)
