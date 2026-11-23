"""Guest sandbox logic (E2.8).

A guest gets a sandbox and a signed token with no login. Everything the guest creates is
bound to the sandbox and counted against its quotas; an expired sandbox (or a token whose
sandbox was deleted) is `NE1104`. A retention task deletes expired sandboxes with their
projects (the cascade carries versions, files, builds and artifacts). Guests never reach a
device route: the guest principal holds no device permission (enforced fully by the E3.2
matrix; until then guest-only routes are the ones mounted here and the open project/build
routes, and the device routes stay closed to the guest role).
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from hashlib import sha256

from sqlalchemy import delete, func, select
from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.auth.tokens import GuestClaims, mint_guest_token, verify_guest_token
from synapse_core.db.models import GuestSandbox, Project
from synapse_core.errors import ErrorCode, ServiceError
from synapse_core.settings import Settings


@dataclass(frozen=True)
class GuestPrincipal:
    """The caller behind a valid guest token, bound to a live sandbox."""

    sandbox_id: uuid.UUID
    expires_at: datetime
    role: str = "guest"

    @property
    def key(self) -> str:
        """The rate-limiter / audit key for this guest."""
        return f"guest:{self.sandbox_id}"


def _quota(message: str) -> ServiceError:
    """The `NE1104` error (quota reached or sandbox expired)."""
    return ServiceError(ErrorCode.GUEST_QUOTA, message)


def hash_ip(ip: str | None, salt: bytes) -> str | None:
    """A one-way salted hash of a client IP; `None` when the IP is unknown."""
    if not ip:
        return None
    return sha256(salt + ip.encode("utf-8")).hexdigest()


async def create_sandbox(
    session: AsyncSession, settings: Settings, *, ip: str | None
) -> tuple[GuestSandbox, str]:
    """Creates a sandbox and returns it with a freshly minted token."""
    now = datetime.now(UTC)
    sandbox = GuestSandbox(
        expires_at=now + timedelta(seconds=settings.guest_token_ttl_s),
        ip_hash=hash_ip(ip, settings.guest_ip_salt_bytes()),
        build_day=now.date().isoformat(),
    )
    session.add(sandbox)
    await session.commit()
    token, _ = mint_guest_token(
        sandbox.id,
        secret=settings.token_secret(),
        ttl=timedelta(seconds=settings.guest_token_ttl_s),
        now=now,
    )
    return sandbox, token


def claims_from_token(token: str, settings: Settings) -> GuestClaims:
    """Verifies a token's signature and expiry; a failure is `NE1104`."""
    from synapse_core.auth.tokens import TokenError

    try:
        return verify_guest_token(token, secret=settings.token_secret())
    except TokenError as exc:
        raise _quota("the guest session is invalid or has expired; start a new one") from exc


async def load_live_sandbox(
    session: AsyncSession, claims: GuestClaims, *, now: datetime | None = None
) -> GuestSandbox:
    """The sandbox named by the token, if it exists and has not expired; else `NE1104`."""
    sandbox = await session.get(GuestSandbox, claims.sandbox_id)
    moment = now or datetime.now(UTC)
    if sandbox is None or _expired(sandbox, moment):
        raise _quota("the guest session is invalid or has expired; start a new one")
    return sandbox


def _expired(sandbox: GuestSandbox, now: datetime) -> bool:
    expires = sandbox.expires_at
    if expires.tzinfo is None:
        expires = expires.replace(tzinfo=UTC)
    return now >= expires


async def principal_from_token(
    session: AsyncSession, token: str, settings: Settings
) -> GuestPrincipal:
    """Resolves a guest token to a live-sandbox principal (`NE1104` otherwise)."""
    claims = claims_from_token(token, settings)
    sandbox = await load_live_sandbox(session, claims)
    return GuestPrincipal(sandbox_id=sandbox.id, expires_at=sandbox.expires_at)


async def reserve_project(session: AsyncSession, sandbox_id: uuid.UUID, settings: Settings) -> None:
    """Checks the project quota and increments the counter; `NE1104` when full.

    The sandbox row is locked so two concurrent creations cannot both pass the check.
    """
    sandbox = (
        await session.execute(
            select(GuestSandbox).where(GuestSandbox.id == sandbox_id).with_for_update()
        )
    ).scalar_one_or_none()
    if sandbox is None or _expired(sandbox, datetime.now(UTC)):
        raise _quota("the guest session is invalid or has expired; start a new one")
    if sandbox.project_count >= settings.guest_max_projects:
        raise _quota(f"a guest may create at most {settings.guest_max_projects} projects")
    sandbox.project_count += 1


async def reserve_build(session: AsyncSession, sandbox_id: uuid.UUID, settings: Settings) -> None:
    """Checks the per-day build quota and increments the counter; `NE1104` when full."""
    sandbox = (
        await session.execute(
            select(GuestSandbox).where(GuestSandbox.id == sandbox_id).with_for_update()
        )
    ).scalar_one_or_none()
    if sandbox is None or _expired(sandbox, datetime.now(UTC)):
        raise _quota("the guest session is invalid or has expired; start a new one")
    today = datetime.now(UTC).date().isoformat()
    if sandbox.build_day != today:
        sandbox.build_day = today
        sandbox.builds_today = 0
    if sandbox.builds_today >= settings.guest_max_builds_per_day:
        raise _quota(f"a guest may run at most {settings.guest_max_builds_per_day} builds a day")
    sandbox.builds_today += 1


async def cleanup_expired(session: AsyncSession, *, now: datetime | None = None) -> int:
    """Deletes expired sandboxes and their data; returns how many were removed.

    Projects of a sandbox cascade-delete their versions, files, builds and artifacts.
    """
    moment = now or datetime.now(UTC)
    expired_ids = (
        (await session.execute(select(GuestSandbox.id).where(GuestSandbox.expires_at <= moment)))
        .scalars()
        .all()
    )
    if not expired_ids:
        return 0
    await session.execute(delete(Project).where(Project.sandbox_id.in_(expired_ids)))
    result = await session.execute(delete(GuestSandbox).where(GuestSandbox.id.in_(expired_ids)))
    await session.commit()
    return int(result.rowcount or 0)  # type: ignore[attr-defined]


async def count_sandboxes(session: AsyncSession) -> int:
    """How many sandboxes exist (test helper)."""
    return (await session.execute(select(func.count()).select_from(GuestSandbox))).scalar_one()
