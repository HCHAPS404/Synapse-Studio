"""Resolve a relay `hello.token` to the protocol roles of a session (E4.1).

Reuses the E3 token resolution exactly: a user JWT, a service token (`daemon`/`twin`) or a
guest token. The rules fixed by relay.md:

- No token -> the `guest` role (the judges' virtual lab, read-only).
- An invalid or expired token -> `unauthenticated` (the server never falls back to guest).
- A daemon or twin service token -> the `daemon` or `twin` role.

A user maps to a protocol role: `viewer` -> `viewer`; `operator` and `engineer` -> the
protocol `operator` role (relay.md has no engineer row, and an engineer may do everything an
operator may on the relay). The caller turns a failure into a relay `unauthenticated` error.
"""

from __future__ import annotations

from dataclasses import dataclass

from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.errors import ServiceError
from synapse_core.services.guests import GuestPrincipal, principal_from_token
from synapse_core.services.service_tokens import (
    ServicePrincipal,
    looks_like_service_token,
    principal_from_service_token,
)
from synapse_core.services.users import AuthenticatedUser, principal_from_access_token
from synapse_core.settings import Settings


class RelayAuthError(Exception):
    """The token in `hello` is invalid or expired (maps to `unauthenticated`)."""


@dataclass(frozen=True)
class RelayIdentity:
    """The resolved session: its protocol roles, a principal key, and an optional cell bind."""

    roles: frozenset[str]
    key: str
    cell_id: str | None = None


def _looks_like_jwt(token: str) -> bool:
    return token.count(".") == 2


def _user_protocol_role(role: str) -> str:
    """Map a user role to a relay protocol role (engineer acts as operator on the relay)."""
    if role in ("operator", "engineer"):
        return "operator"
    return "viewer"


async def identify(
    session: AsyncSession | None, token: str | None, settings: Settings
) -> RelayIdentity:
    """Resolve a `hello.token` to a `RelayIdentity`, or raise `RelayAuthError`.

    `session` may be `None` only when there is no token (the guest path needs no database).
    """
    if token is None:
        return RelayIdentity(roles=frozenset({"guest"}), key="guest:anonymous")
    if session is None:  # pragma: no cover - the endpoint always passes a session with a token
        raise RelayAuthError("a database session is required to validate a token")
    try:
        if looks_like_service_token(token):
            svc: ServicePrincipal = await principal_from_service_token(session, token)
            return RelayIdentity(roles=frozenset({svc.role}), key=svc.key, cell_id=svc.cell_id)
        if _looks_like_jwt(token):
            user: AuthenticatedUser = await principal_from_access_token(session, token, settings)
            return RelayIdentity(roles=frozenset({_user_protocol_role(user.role)}), key=user.key)
        guest: GuestPrincipal = await principal_from_token(session, token, settings)
        return RelayIdentity(roles=frozenset({"guest"}), key=guest.key)
    except ServiceError as exc:
        raise RelayAuthError("the token is invalid or expired") from exc
