"""Route `POST /v1/guest`: a login-free sandbox and token for judges (E2.8)."""

from typing import Annotated

from fastapi import APIRouter, Depends, Request

from synapse_core.db.session import SessionDep
from synapse_core.schemas.guests import GuestSessionOut
from synapse_core.services import guests as svc
from synapse_core.settings import Settings, get_settings

router = APIRouter(prefix="/v1", tags=["guest"])

SettingsDep = Annotated[Settings, Depends(get_settings)]


@router.post("/guest", response_model=GuestSessionOut, status_code=201)
async def create_guest(
    request: Request, session: SessionDep, settings: SettingsDep
) -> GuestSessionOut:
    """Issues a guest sandbox and a token. Send it as `Authorization: Bearer <token>`."""
    ip = request.client.host if request.client else None
    sandbox, token = await svc.create_sandbox(session, settings, ip=ip)
    return GuestSessionOut(
        token=token,
        role="guest",
        sandbox_id=str(sandbox.id),
        expires_at=sandbox.expires_at,
    )
