"""Telemetry query route `GET /v1/telemetry` (E4.5).

A user with `telemetry.read` (viewer and up) may query any device; a guest may query only
virtual devices (`simulated`), as the design allows. An anonymous request or any other
principal is `NE1106`.
"""

from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Query

from synapse_core.api.deps import OptionalPrincipalDep
from synapse_core.auth.permissions import Permission, has_permission
from synapse_core.db.session import SessionDep
from synapse_core.errors import ErrorCode, ServiceError
from synapse_core.schemas.telemetry import TelemetryPage, TelemetrySampleOut
from synapse_core.services.guests import GuestPrincipal
from synapse_core.telemetry.query import MAX_LIMIT, query_samples

router = APIRouter(prefix="/v1", tags=["telemetry"])

Limit = Annotated[int, Query(ge=1, le=MAX_LIMIT)]


def _denied() -> ServiceError:
    return ServiceError(
        ErrorCode.PERMISSION_DENIED, "your role does not have permission for this action"
    )


@router.get("/telemetry", response_model=TelemetryPage)
async def get_telemetry(
    session: SessionDep,
    principal: OptionalPrincipalDep,
    cell: Annotated[str, Query(min_length=1, max_length=120)],
    device: Annotated[str | None, Query(max_length=120)] = None,
    tag: Annotated[str | None, Query(max_length=200)] = None,
    from_ns: int | None = None,
    to_ns: int | None = None,
    limit: Limit = 1000,
) -> TelemetryPage:
    """Telemetry samples for a cell, newest first. Guests see only virtual devices."""
    if principal is None:
        raise _denied()
    is_guest = isinstance(principal, GuestPrincipal)
    if not is_guest and not has_permission(principal.role, Permission.TELEMETRY_READ):
        raise _denied()
    rows = await query_samples(
        session,
        cell_id=cell,
        device=device,
        tag=tag,
        from_ns=from_ns,
        to_ns=to_ns,
        limit=limit,
        virtual_only=is_guest,
    )
    return TelemetryPage(
        items=[TelemetrySampleOut(**vars(r)) for r in rows], cell_id=cell, limit=limit
    )
