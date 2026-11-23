"""Signed, stateless guest tokens (E2.8).

A guest token carries the sandbox id, the role (`guest`), the issue time and the expiry. It
is a compact `payload.signature` string: URL-safe base64 of a canonical JSON payload and an
HMAC-SHA256 over that payload with the server secret. No login, no database read is needed
to recognise a guest; the sandbox row is still looked up to enforce quotas and expiry
(`NE1104`), so a token that outlives its deleted sandbox is rejected there.

E3 replaces this with full JWT access tokens signed with the same `SYNAPSE_JWT_SECRET`;
this keeps the surface tiny and dependency-free until then.
"""

from __future__ import annotations

import base64
import hmac
import json
import uuid
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from hashlib import sha256

_ROLE_GUEST = "guest"


class TokenError(Exception):
    """A guest token is missing, malformed, has a bad signature, or has expired."""


@dataclass(frozen=True)
class GuestClaims:
    """The verified contents of a guest token."""

    sandbox_id: uuid.UUID
    issued_at: datetime
    expires_at: datetime
    role: str = _ROLE_GUEST


def _b64encode(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode("ascii")


def _b64decode(text: str) -> bytes:
    padding = "=" * (-len(text) % 4)
    return base64.urlsafe_b64decode(text + padding)


def _sign(payload: bytes, secret: bytes) -> str:
    return _b64encode(hmac.new(secret, payload, sha256).digest())


def mint_guest_token(
    sandbox_id: uuid.UUID, *, secret: bytes, ttl: timedelta, now: datetime | None = None
) -> tuple[str, datetime]:
    """Returns a signed guest token and its expiry time."""
    issued = now or datetime.now(UTC)
    expires = issued + ttl
    payload = {
        "sbx": str(sandbox_id),
        "role": _ROLE_GUEST,
        "iat": int(issued.timestamp()),
        "exp": int(expires.timestamp()),
    }
    raw = json.dumps(payload, separators=(",", ":"), sort_keys=True).encode("utf-8")
    encoded = _b64encode(raw)
    return f"{encoded}.{_sign(raw, secret)}", expires


def verify_guest_token(token: str, *, secret: bytes, now: datetime | None = None) -> GuestClaims:
    """Parses and verifies a guest token, or raises `TokenError`.

    The signature is checked before the payload is trusted, and in constant time.
    """
    parts = token.split(".")
    if len(parts) != 2:
        raise TokenError("malformed token")
    encoded, signature = parts
    try:
        raw = _b64decode(encoded)
    except (ValueError, TypeError) as exc:
        raise TokenError("malformed token") from exc
    if not hmac.compare_digest(signature, _sign(raw, secret)):
        raise TokenError("bad signature")
    try:
        payload = json.loads(raw)
        sandbox_id = uuid.UUID(payload["sbx"])
        issued = datetime.fromtimestamp(int(payload["iat"]), UTC)
        expires = datetime.fromtimestamp(int(payload["exp"]), UTC)
        role = str(payload["role"])
    except (ValueError, KeyError, TypeError) as exc:
        raise TokenError("malformed payload") from exc
    if role != _ROLE_GUEST:
        raise TokenError("not a guest token")
    moment = now or datetime.now(UTC)
    if moment >= expires:
        raise TokenError("token expired")
    return GuestClaims(sandbox_id=sandbox_id, issued_at=issued, expires_at=expires, role=role)
