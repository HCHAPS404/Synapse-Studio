"""Daemon pairing and daemon-status routes (E4.3).

`POST /v1/pairing` (engineer, `pairing.issue`) returns a one-time code bound to a cell.
`POST /v1/pairing/claim` is public — the daemon has no token yet — and exchanges the code for
a `daemon` service token, registering the daemon and its devices. `GET /v1/daemons` reports
each daemon's `deployment_status` (online/offline by the heartbeat timeout). `DELETE
/v1/daemons/{id}` (engineer) unpairs: it revokes the token and closes that token's relay
sockets. Every step is audited.
"""

from __future__ import annotations

import uuid
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, Query, Request

from synapse_core.api.deps import (
    Principal,
    SessionMakerDep,
    principal_identity,
    require_permission,
)
from synapse_core.auth.permissions import Permission
from synapse_core.db.session import SessionDep
from synapse_core.relay.hub import Hub
from synapse_core.schemas.pairing import (
    DaemonClaimed,
    DaemonInfo,
    DaemonList,
    DeviceInfo,
    PairingClaim,
    PairingCreate,
    PairingIssued,
)
from synapse_core.services import audit
from synapse_core.services import pairing as svc

router = APIRouter(prefix="/v1", tags=["pairing"])

IssuerDep = Annotated[Principal, Depends(require_permission(Permission.PAIRING_ISSUE))]
ReaderDep = Annotated[Principal, Depends(require_permission(Permission.DEVICE_READ))]


@router.post("/pairing", response_model=PairingIssued, status_code=201)
async def issue_pairing(
    body: PairingCreate,
    session: SessionDep,
    sessions: SessionMakerDep,
    principal: IssuerDep,
) -> PairingIssued:
    """Issues a one-time pairing code for a cell (engineer only)."""
    row, code = await svc.issue_pairing_code(session, body.cell_id)
    actor, role = principal_identity(principal)
    await audit.record(
        sessions,
        audit.AuditAction.PAIRING_ISSUE,
        actor=actor,
        role=role,
        result="ok",
        target=body.cell_id,
        detail={"pairing_id": str(row.id)},
    )
    return PairingIssued(id=row.id, cell_id=row.cell_id, code=code, expires_at=row.expires_at)


@router.post("/pairing/claim", response_model=DaemonClaimed, status_code=201)
async def claim_pairing(
    body: PairingClaim,
    session: SessionDep,
    sessions: SessionMakerDep,
) -> DaemonClaimed:
    """Claims a code and returns a daemon service token (public; the daemon has no token yet)."""
    daemon, token = await svc.claim_pairing(
        session,
        body.code,
        body.daemon_name,
        version=body.version,
        devices=[(d.name, d.kind) for d in body.devices],
    )
    await audit.record(
        sessions,
        audit.AuditAction.PAIRING_CLAIM,
        actor=f"daemon:{daemon.id}",
        role="daemon",
        result="ok",
        target=daemon.cell_id,
        detail={"daemon_id": str(daemon.id), "name": daemon.name},
    )
    return DaemonClaimed(daemon_id=daemon.id, cell_id=daemon.cell_id, token=token)


@router.get("/daemons", response_model=DaemonList)
async def list_daemons(
    session: SessionDep,
    _principal: ReaderDep,
    cell_id: Annotated[str | None, Query(max_length=120)] = None,
) -> DaemonList:
    """Daemons and their deployment status (online/offline by the heartbeat timeout)."""
    statuses = await svc.list_daemons(session, cell_id=cell_id)
    return DaemonList(
        items=[
            DaemonInfo(
                id=s.id,
                name=s.name,
                cell_id=s.cell_id,
                version=s.version,
                last_heartbeat=s.last_heartbeat,
                online=s.online,
                revoked=s.revoked,
                devices=[DeviceInfo(name=n, kind=k, simulated=sim) for n, k, sim in s.devices],
            )
            for s in statuses
        ]
    )


@router.delete("/daemons/{daemon_id}", status_code=204)
async def unpair_daemon(
    daemon_id: uuid.UUID,
    request: Request,
    session: SessionDep,
    sessions: SessionMakerDep,
    principal: IssuerDep,
) -> None:
    """Unpairs a daemon: revokes its token and closes its relay sockets (engineer only)."""
    token_id = await svc.unpair_daemon(session, daemon_id)
    if token_id is None:
        raise HTTPException(status_code=404, detail="no active daemon with that id")
    hub: Hub | None = getattr(request.app.state, "relay_hub", None)
    if hub is not None:
        hub.close_by_key(f"service:{token_id}")
    actor, role = principal_identity(principal)
    await audit.record(
        sessions,
        audit.AuditAction.DAEMON_UNPAIR,
        actor=actor,
        role=role,
        result="ok",
        target=str(daemon_id),
    )
