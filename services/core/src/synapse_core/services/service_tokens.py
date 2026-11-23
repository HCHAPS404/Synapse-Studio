"""Service tokens for the `daemon` and `twin` principals (E3.3).

A service token is a random 256-bit secret shown once at issue; only its SHA-256 is stored,
so a database read cannot recover a usable token. Tokens are revocable (`revoked_at`). The
plaintext begins with `svc_` and contains no dot, so the principal resolver tells it apart
from a user JWT (three dots) and a guest token (one dot). Resolution failures are `NE1105`
(authentication failed), the same opaque code as a bad login.
"""

from __future__ import annotations

import secrets
import uuid
from dataclasses import dataclass
from datetime import UTC, datetime
from hashlib import sha256

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.auth.permissions import ROLE_DAEMON, ROLE_TWIN
from synapse_core.db.models import ServiceToken
from synapse_core.errors import ErrorCode, ServiceError

_PREFIX = "svc_"
SERVICE_ROLES: frozenset[str] = frozenset({ROLE_DAEMON, ROLE_TWIN})


@dataclass(frozen=True)
class ServicePrincipal:
    """The caller behind a valid service token."""

    token_id: uuid.UUID
    role: str
    cell_id: str | None

    @property
    def key(self) -> str:
        """The rate-limiter / audit key for this service principal."""
        return f"service:{self.token_id}"


def looks_like_service_token(token: str) -> bool:
    """A service token starts with `svc_`."""
    return token.startswith(_PREFIX)


def hash_token(token: str) -> str:
    """The SHA-256 (hex) of a token value; this is what is stored and compared."""
    return sha256(token.encode("utf-8")).hexdigest()


def _auth_failed() -> ServiceError:
    return ServiceError(ErrorCode.AUTH_FAILED, "authentication failed")


async def issue_service_token(
    session: AsyncSession, role: str, *, cell_id: str | None = None, label: str = ""
) -> tuple[ServiceToken, str]:
    """Creates a service token and returns the row and the plaintext (shown once).

    Raises `ValueError` for a role that is not a service role, or for a `daemon` token
    without a `cell_id` (a daemon token is always bound to one cell). The twin token has no
    cell.
    """
    if role not in SERVICE_ROLES:
        raise ValueError(f"role must be one of {sorted(SERVICE_ROLES)}")
    if role == ROLE_DAEMON and not cell_id:
        raise ValueError("a daemon token must be bound to a cell_id")
    if role == ROLE_TWIN and cell_id:
        raise ValueError("a twin token is not bound to a cell")
    plaintext = _PREFIX + secrets.token_urlsafe(32)
    row = ServiceToken(role=role, cell_id=cell_id, token_hash=hash_token(plaintext), label=label)
    session.add(row)
    await session.commit()
    return row, plaintext


async def principal_from_service_token(session: AsyncSession, token: str) -> ServicePrincipal:
    """Resolves a live (unrevoked) service token; any failure is `NE1105`.

    The lookup is by the stored hash, so the plaintext is never compared in the database.
    """
    row = (
        await session.execute(
            select(ServiceToken).where(ServiceToken.token_hash == hash_token(token))
        )
    ).scalar_one_or_none()
    if row is None or row.revoked_at is not None:
        raise _auth_failed()
    return ServicePrincipal(token_id=row.id, role=row.role, cell_id=row.cell_id)


async def revoke_service_token(session: AsyncSession, token_id: uuid.UUID) -> bool:
    """Marks a token revoked; returns `False` when it does not exist or was already revoked."""
    row = await session.get(ServiceToken, token_id)
    if row is None or row.revoked_at is not None:
        return False
    row.revoked_at = datetime.now(UTC)
    await session.commit()
    return True


async def list_service_tokens(session: AsyncSession) -> list[ServiceToken]:
    """Every service token row (metadata only; the plaintext is not stored)."""
    return list(
        (await session.execute(select(ServiceToken).order_by(ServiceToken.created_at.desc())))
        .scalars()
        .all()
    )
