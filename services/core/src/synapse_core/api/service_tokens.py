"""Service-token routes (E3.3): engineer-only issue, list and revoke.

A service token lets a daemon or the twin authenticate. It is shown once at issue; only its
SHA-256 is stored. These routes need `user.manage`, which only the engineer role holds, so a
viewer, operator, guest or service principal is `NE1106`.
"""

import uuid
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException

from synapse_core.api.deps import Principal, require_permission
from synapse_core.auth.permissions import Permission
from synapse_core.db.session import SessionDep
from synapse_core.errors import ErrorCode, ServiceError
from synapse_core.schemas.service_tokens import (
    ServiceTokenCreate,
    ServiceTokenInfo,
    ServiceTokenIssued,
    ServiceTokenList,
)
from synapse_core.services import service_tokens as svc

router = APIRouter(prefix="/v1/service-tokens", tags=["service-tokens"])

EngineerDep = Annotated[Principal, Depends(require_permission(Permission.USER_MANAGE))]


@router.post("", response_model=ServiceTokenIssued, status_code=201)
async def issue_token(
    body: ServiceTokenCreate, session: SessionDep, _principal: EngineerDep
) -> ServiceTokenIssued:
    """Issues a service token. The secret is returned once and never stored in clear."""
    try:
        row, plaintext = await svc.issue_service_token(
            session, body.role, cell_id=body.cell_id, label=body.label
        )
    except ValueError as exc:
        raise ServiceError(ErrorCode.JOB_REJECTED, str(exc)) from exc
    return ServiceTokenIssued(
        id=row.id,
        role=row.role,
        cell_id=row.cell_id,
        label=row.label,
        created_at=row.created_at,
        revoked_at=row.revoked_at,
        token=plaintext,
    )


@router.get("", response_model=ServiceTokenList)
async def list_tokens(session: SessionDep, _principal: EngineerDep) -> ServiceTokenList:
    """Lists every service token's metadata (never a secret), newest first."""
    rows = await svc.list_service_tokens(session)
    return ServiceTokenList(items=[ServiceTokenInfo.model_validate(r) for r in rows])


@router.delete("/{token_id}", status_code=204)
async def revoke_token(token_id: uuid.UUID, session: SessionDep, _principal: EngineerDep) -> None:
    """Revokes a token; a missing or already-revoked token is a 404."""
    if not await svc.revoke_service_token(session, token_id):
        raise HTTPException(status_code=404, detail="no active token with that id")
