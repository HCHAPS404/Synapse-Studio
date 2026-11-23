"""User and authentication logic (E3.1).

Login verifies an argon2id password hash and, on success, mints a short-lived JWT access
token. There is no self-registration: `create_user` is called by the bootstrap CLI (first
engineer) and, from E3.2, by an engineer-only route. A failed login is `NE1105` and the
message never says which part failed (missing user, wrong password, disabled account).
Passwords and tokens are never logged.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta

from sqlalchemy import func, select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.auth.passwords import hash_password, verify_password
from synapse_core.auth.tokens import AccessClaims, mint_access_token
from synapse_core.db.models import User
from synapse_core.errors import ErrorCode, ServiceError
from synapse_core.settings import Settings

USER_ROLES: frozenset[str] = frozenset({"viewer", "operator", "engineer"})


@dataclass(frozen=True)
class AuthenticatedUser:
    """The principal behind a valid user access token."""

    user_id: uuid.UUID
    role: str

    @property
    def key(self) -> str:
        """The rate-limiter / audit key for this user."""
        return f"user:{self.user_id}"


def _auth_failed() -> ServiceError:
    """`NE1105`; the same opaque message for every authentication failure."""
    return ServiceError(ErrorCode.AUTH_FAILED, "authentication failed")


def _normalize_email(email: str) -> str:
    return email.strip().lower()


async def create_user(
    session: AsyncSession, email: str, password: str, role: str, settings: Settings
) -> User:
    """Creates a user with an argon2id hash; a duplicate email or bad role is rejected.

    Commits the new row. Raises `ValueError` for an unknown role or an empty password so a
    CLI or route surfaces a clear message; a duplicate email raises `ValueError` too (the
    unique constraint is the authority, checked after the insert).
    """
    if role not in USER_ROLES:
        raise ValueError(f"role must be one of {sorted(USER_ROLES)}")
    if not password:
        raise ValueError("password must not be empty")
    user = User(
        email=_normalize_email(email),
        password_hash=hash_password(
            password,
            time_cost=settings.argon2_time_cost,
            memory_cost_kib=settings.argon2_memory_cost_kib,
            parallelism=settings.argon2_parallelism,
        ),
        role=role,
    )
    session.add(user)
    try:
        await session.commit()
    except IntegrityError as exc:
        await session.rollback()
        raise ValueError("a user with this email already exists") from exc
    return user


async def count_users(session: AsyncSession) -> int:
    """How many users exist (used by the bootstrap command)."""
    return (await session.execute(select(func.count()).select_from(User))).scalar_one()


async def principal_from_access_token(
    session: AsyncSession, token: str, settings: Settings
) -> AuthenticatedUser:
    """Resolves a user JWT to a live, enabled user; any failure is `NE1105`.

    The signature and expiry are checked first, then the user is loaded so a token for a
    deleted or disabled account is rejected. The role is read from the database row, not the
    token claim, so a role change takes effect on the next request without waiting for the
    old token to expire.
    """
    from synapse_core.auth.tokens import TokenError, verify_access_token

    try:
        claims = verify_access_token(token, secret=settings.token_secret())
    except TokenError as exc:
        raise _auth_failed() from exc
    user = await session.get(User, claims.subject)
    if user is None or user.disabled:
        raise _auth_failed()
    return AuthenticatedUser(user_id=user.id, role=user.role)


async def authenticate(
    session: AsyncSession, email: str, password: str, settings: Settings
) -> tuple[str, AccessClaims]:
    """Verifies the credentials and returns `(token, claims)`; `NE1105` on any failure.

    The password is always verified (against the user's hash, or a dummy hash when there is
    no such user) so the response time does not reveal whether the email exists.
    """
    user = (
        await session.execute(select(User).where(User.email == _normalize_email(email)))
    ).scalar_one_or_none()
    if user is None or user.disabled:
        # Spend time on a dummy verification to avoid a user-enumeration timing signal.
        verify_password(_dummy_hash(), password)
        raise _auth_failed()
    if not verify_password(user.password_hash, password):
        raise _auth_failed()
    token, claims = mint_access_token(
        user.id,
        user.role,
        secret=settings.token_secret(),
        ttl=timedelta(seconds=settings.access_token_ttl_s),
        now=datetime.now(UTC),
    )
    return token, claims


_DUMMY_HASH_CACHE: str | None = None


def _dummy_hash() -> str:
    """A valid Argon2id hash of a throwaway value, computed once, used only to equalise
    login timing against a non-existent or disabled user (anti-enumeration)."""
    global _DUMMY_HASH_CACHE
    if _DUMMY_HASH_CACHE is None:
        _DUMMY_HASH_CACHE = hash_password("x", time_cost=1, memory_cost_kib=8, parallelism=1)
    return _DUMMY_HASH_CACHE
