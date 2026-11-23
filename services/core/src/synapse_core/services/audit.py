"""Audit log writes and the engineer read query (E3.4).

Every sensitive action writes exactly one append-only row: login, service-token issue and
revoke, project and version creation, build creation, user creation and role change. A write
uses its own short-lived session so an audit failure is logged but never breaks the action
it records. `detail` is small and is scrubbed of anything that looks like a secret before it
is stored, so a token or password can never land in the log.
"""

from __future__ import annotations

import logging
import uuid
from collections.abc import Sequence
from dataclasses import dataclass
from datetime import datetime
from enum import StrEnum
from typing import Any

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from synapse_core.db.models import AuditLog
from synapse_core.logging_config import request_id_var

log = logging.getLogger(__name__)

# Field names whose value is replaced by a placeholder before it reaches the table.
_SECRET_KEYS = frozenset({"token", "password", "secret", "access_token", "hash", "token_hash"})
_REDACTED = "***"


class AuditAction(StrEnum):
    """The sensitive actions that are recorded."""

    LOGIN = "login"
    SERVICE_TOKEN_ISSUE = "service_token.issue"  # noqa: S105 - an action name, not a secret
    SERVICE_TOKEN_REVOKE = "service_token.revoke"  # noqa: S105 - an action name, not a secret
    PROJECT_CREATE = "project.create"
    VERSION_CREATE = "version.create"
    BUILD_CREATE = "build.create"
    USER_CREATE = "user.create"
    ROLE_CHANGE = "user.role_change"


@dataclass(frozen=True)
class AuditEntry:
    """A read-model row of the audit log."""

    id: uuid.UUID
    at: datetime
    actor: str
    role: str
    action: str
    target: str | None
    request_id: str | None
    result: str
    detail: dict[str, Any]


def _scrub(detail: dict[str, Any] | None) -> dict[str, Any]:
    """Replaces any secret-looking value so a token or password cannot enter the log."""
    if not detail:
        return {}
    return {k: (_REDACTED if k.lower() in _SECRET_KEYS else v) for k, v in detail.items()}


async def record(
    sessions: async_sessionmaker[AsyncSession],
    action: AuditAction,
    *,
    actor: str,
    role: str,
    result: str,
    target: str | None = None,
    detail: dict[str, Any] | None = None,
) -> None:
    """Writes one audit row in its own session; a failure is logged, never raised.

    The `request_id` is read from the request context so the row correlates with the logs.
    """
    try:
        async with sessions() as session:
            session.add(
                AuditLog(
                    actor=actor,
                    role=role,
                    action=action.value,
                    target=target,
                    request_id=request_id_var.get(),
                    result=result,
                    detail=_scrub(detail),
                )
            )
            await session.commit()
    except Exception as exc:  # noqa: BLE001 - auditing must never break the audited action
        log.warning(
            "audit write failed", extra={"action": action.value, "error": type(exc).__name__}
        )


async def query(
    session: AsyncSession,
    *,
    action: str | None = None,
    actor: str | None = None,
    limit: int = 100,
    offset: int = 0,
) -> tuple[Sequence[AuditEntry], int]:
    """Audit rows newest first, optionally filtered by action or actor, with the total."""
    from sqlalchemy import func

    stmt = select(AuditLog)
    count = select(func.count()).select_from(AuditLog)
    if action is not None:
        stmt = stmt.where(AuditLog.action == action)
        count = count.where(AuditLog.action == action)
    if actor is not None:
        stmt = stmt.where(AuditLog.actor == actor)
        count = count.where(AuditLog.actor == actor)
    total = (await session.execute(count)).scalar_one()
    rows = (
        (
            await session.execute(
                stmt.order_by(AuditLog.at.desc(), AuditLog.id).limit(limit).offset(offset)
            )
        )
        .scalars()
        .all()
    )
    entries = [
        AuditEntry(
            id=r.id,
            at=r.at,
            actor=r.actor,
            role=r.role,
            action=r.action,
            target=r.target,
            request_id=r.request_id,
            result=r.result,
            detail=r.detail,
        )
        for r in rows
    ]
    return entries, total
