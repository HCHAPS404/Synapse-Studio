"""Black box upload route `POST /v1/blackbox` (E6.2).

A daemon uploads a fault record it froze (E6.1). The daemon authenticates with its service
token (`deployment.report`, the same permission it reports status with); the record is bound to
the daemon token's cell so a daemon cannot write records for another cell. The upload is
idempotent, so a daemon that restarts and retries does not duplicate a record. The body is
validated against the record shape (`extra="forbid"`) at this trust boundary.
"""

from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends

from synapse_core.api.deps import Principal, require_permission
from synapse_core.auth.permissions import Permission
from synapse_core.db.session import SessionDep
from synapse_core.schemas.blackbox import BlackBoxRecordIn, BlackBoxUploaded
from synapse_core.services import blackbox as svc
from synapse_core.services.service_tokens import ServicePrincipal

router = APIRouter(prefix="/v1", tags=["blackbox"])

ReporterDep = Annotated[Principal, Depends(require_permission(Permission.DEPLOYMENT_REPORT))]


@router.post("/blackbox", response_model=BlackBoxUploaded, status_code=201)
async def upload_blackbox(
    body: BlackBoxRecordIn,
    session: SessionDep,
    principal: ReporterDep,
) -> BlackBoxUploaded:
    """Store a fault record from a daemon. Idempotent on `(cell, device, fault instant)`."""
    # A daemon token is bound to one cell; records are stored under that cell. A twin (the only
    # other holder of deployment.report is the daemon) never reaches here with a cell record.
    cell_id = body.cell
    if isinstance(principal, ServicePrincipal) and principal.cell_id:
        cell_id = principal.cell_id
    row = await svc.store_record(session, body, cell_id=cell_id)
    return BlackBoxUploaded(id=row.id, cell_id=row.cell_id, created_at=row.created_at)
