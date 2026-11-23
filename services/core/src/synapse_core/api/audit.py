"""Audit read route (E3.4): engineers read the append-only log.

Only the engineer role holds `audit.read`; everyone else is `NE1106`. There is no write,
update or delete route: rows are written by the actions themselves and the table grant
forbids changing them.
"""

from typing import Annotated

from fastapi import APIRouter, Depends, Query

from synapse_core.api.deps import Principal, require_permission
from synapse_core.auth.permissions import Permission
from synapse_core.db.session import SessionDep
from synapse_core.schemas.audit import AuditEntryOut, AuditPage
from synapse_core.services import audit as svc

router = APIRouter(prefix="/v1/audit", tags=["audit"])

EngineerDep = Annotated[Principal, Depends(require_permission(Permission.AUDIT_READ))]
Limit = Annotated[int, Query(ge=1, le=500)]
Offset = Annotated[int, Query(ge=0)]


@router.get("", response_model=AuditPage)
async def read_audit(
    session: SessionDep,
    _principal: EngineerDep,
    action: str | None = None,
    actor: str | None = None,
    limit: Limit = 100,
    offset: Offset = 0,
) -> AuditPage:
    """Audit rows newest first, optionally filtered by `action` or `actor`."""
    entries, total = await svc.query(
        session, action=action, actor=actor, limit=limit, offset=offset
    )
    return AuditPage(
        items=[AuditEntryOut(**vars(e)) for e in entries],
        total=total,
        limit=limit,
        offset=offset,
    )
