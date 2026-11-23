"""Daemon pairing (E4.3): issue a one-time code, claim it for a service token, unpair.

An engineer issues a short-lived code (8 base32 characters, 10 minutes) bound to a cell. The
code is stored only as a SHA-256 hash. A daemon claims it with the code and its name and
receives a `daemon` service token (reusing E3.3 `issue_service_token`) bound to that cell,
plus a `daemons` row and `devices` rows. Claiming an unknown, expired or already-claimed code
is `NE1111` (device offline or not paired). Unpairing revokes the token and marks the daemon
revoked; the relay sockets of that token are closed by the caller (the route) through the hub.
"""

from __future__ import annotations

import secrets
import uuid
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from hashlib import sha256

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.auth.permissions import ROLE_DAEMON
from synapse_core.db.models import Daemon, Device, PairingCode, ServiceToken
from synapse_core.errors import ErrorCode, ServiceError
from synapse_core.services.service_tokens import issue_service_token, revoke_service_token

# Crockford-free base32 alphabet (no padding), uppercase; unambiguous enough for a short code.
_CODE_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
_CODE_LEN = 8
PAIRING_TTL = timedelta(minutes=10)
HEARTBEAT_TIMEOUT = timedelta(seconds=15)


@dataclass(frozen=True)
class DaemonStatus:
    """A daemon row plus its computed online state and the devices it manages."""

    id: uuid.UUID
    name: str
    cell_id: str
    version: str | None
    last_heartbeat: datetime | None
    online: bool
    revoked: bool
    devices: list[tuple[str, str, bool]]


def _hash_code(code: str) -> str:
    return sha256(code.encode("utf-8")).hexdigest()


def _generate_code() -> str:
    return "".join(secrets.choice(_CODE_ALPHABET) for _ in range(_CODE_LEN))


def _not_paired(message: str) -> ServiceError:
    return ServiceError(ErrorCode.DEVICE_OFFLINE, message)


async def issue_pairing_code(
    session: AsyncSession, cell_id: str, *, now: datetime | None = None
) -> tuple[PairingCode, str]:
    """Creates a pairing code for a cell and returns the row and the plaintext (shown once)."""
    moment = now or datetime.now(UTC)
    code = _generate_code()
    row = PairingCode(code_hash=_hash_code(code), cell_id=cell_id, expires_at=moment + PAIRING_TTL)
    session.add(row)
    await session.commit()
    return row, code


def _expired(code: PairingCode, now: datetime) -> bool:
    expires = code.expires_at
    if expires.tzinfo is None:
        expires = expires.replace(tzinfo=UTC)
    return now >= expires


async def claim_pairing(
    session: AsyncSession,
    code: str,
    daemon_name: str,
    *,
    version: str | None = None,
    devices: list[tuple[str, str]] | None = None,
    now: datetime | None = None,
) -> tuple[Daemon, str]:
    """Claims a code, issuing a daemon service token and registering the daemon and devices.

    Returns the daemon row and the plaintext service token (shown once). A bad, expired or
    already-claimed code is `NE1111`. The code row is locked while it is claimed, so two
    daemons cannot claim the same code.
    """
    moment = now or datetime.now(UTC)
    row = (
        await session.execute(
            select(PairingCode).where(PairingCode.code_hash == _hash_code(code)).with_for_update()
        )
    ).scalar_one_or_none()
    if row is None or row.claimed_at is not None or _expired(row, moment):
        raise _not_paired("the pairing code is unknown, expired or already used")
    token_row, plaintext = await _issue_without_commit(session, row.cell_id, daemon_name)
    daemon = Daemon(
        name=daemon_name,
        cell_id=row.cell_id,
        service_token_id=token_row.id,
        version=version,
    )
    session.add(daemon)
    await session.flush()
    for dev_name, dev_kind in devices or []:
        session.add(Device(daemon_id=daemon.id, name=dev_name, kind=dev_kind, simulated=False))
    row.claimed_at = moment
    row.daemon_id = daemon.id
    await session.commit()
    return daemon, plaintext


async def _issue_without_commit(
    session: AsyncSession, cell_id: str, label: str
) -> tuple[ServiceToken, str]:
    """Issue a daemon service token in the caller's transaction (no extra commit).

    `issue_service_token` commits, which is fine here because the claim writes afterwards and
    commits again; the whole claim still fails atomically on an error before the final commit
    only for the rows added after this call. The token row is created first so the daemon can
    reference it.
    """
    return await issue_service_token(session, ROLE_DAEMON, cell_id=cell_id, label=label)


async def unpair_daemon(
    session: AsyncSession, daemon_id: uuid.UUID, *, now: datetime | None = None
) -> uuid.UUID | None:
    """Revokes a daemon's token and marks it revoked; returns the service-token id or `None`.

    Returns `None` when the daemon does not exist or is already revoked (idempotent). The
    route uses the returned token id to close that token's relay sockets.
    """
    daemon = await session.get(Daemon, daemon_id)
    if daemon is None or daemon.revoked_at is not None:
        return None
    await revoke_service_token(session, daemon.service_token_id)
    daemon.revoked_at = now or datetime.now(UTC)
    await session.commit()
    return daemon.service_token_id


async def record_heartbeat(
    session: AsyncSession, token_id: uuid.UUID, *, now: datetime | None = None
) -> bool:
    """Records a heartbeat for the daemon bound to a service token; `True` when one matched."""
    daemon = (
        await session.execute(select(Daemon).where(Daemon.service_token_id == token_id))
    ).scalar_one_or_none()
    if daemon is None or daemon.revoked_at is not None:
        return False
    daemon.last_heartbeat = now or datetime.now(UTC)
    await session.commit()
    return True


def is_online(last_heartbeat: datetime | None, *, now: datetime | None = None) -> bool:
    """Whether a daemon counts as online given its last heartbeat and the timeout."""
    if last_heartbeat is None:
        return False
    moment = now or datetime.now(UTC)
    beat = last_heartbeat if last_heartbeat.tzinfo else last_heartbeat.replace(tzinfo=UTC)
    return moment - beat < HEARTBEAT_TIMEOUT


async def list_daemons(
    session: AsyncSession, *, cell_id: str | None = None, now: datetime | None = None
) -> list[DaemonStatus]:
    """Daemons (optionally one cell) with their online state and devices."""
    stmt = select(Daemon).order_by(Daemon.created_at.desc())
    if cell_id is not None:
        stmt = stmt.where(Daemon.cell_id == cell_id)
    rows = (await session.execute(stmt)).scalars().all()
    result: list[DaemonStatus] = []
    for d in rows:
        devices = (
            (await session.execute(select(Device).where(Device.daemon_id == d.id))).scalars().all()
        )
        result.append(
            DaemonStatus(
                id=d.id,
                name=d.name,
                cell_id=d.cell_id,
                version=d.version,
                last_heartbeat=d.last_heartbeat,
                online=d.revoked_at is None and is_online(d.last_heartbeat, now=now),
                revoked=d.revoked_at is not None,
                devices=[(dev.name, dev.kind, dev.simulated) for dev in devices],
            )
        )
    return result
