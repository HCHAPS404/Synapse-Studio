"""Route `POST /v1/auth/login`: password login that returns a JWT access token (E3.1)."""

from typing import Annotated

from fastapi import APIRouter, Depends, Request

from synapse_core.db.session import SessionDep
from synapse_core.schemas.auth import LoginRequest, TokenOut
from synapse_core.services import users as svc
from synapse_core.settings import Settings, get_settings

router = APIRouter(prefix="/v1/auth", tags=["auth"])

SettingsDep = Annotated[Settings, Depends(get_settings)]


@router.post("/login", response_model=TokenOut)
async def login(
    request: Request, body: LoginRequest, session: SessionDep, settings: SettingsDep
) -> TokenOut:
    """Verifies credentials and issues a short-lived access token (`NE1105` on failure).

    The audit entry for the login is written by E3.4; the token is never logged.
    """
    token, claims = await svc.authenticate(session, body.email, body.password, settings)
    request.state.principal_id = f"user:{claims.subject}"
    return TokenOut(access_token=token, role=claims.role, expires_at=claims.expires_at)
