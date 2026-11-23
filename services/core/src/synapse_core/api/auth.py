"""Route `POST /v1/auth/login`: password login that returns a JWT access token (E3.1)."""

from typing import Annotated

from fastapi import APIRouter, Depends, Request

from synapse_core.api.deps import SessionMakerDep
from synapse_core.db.session import SessionDep
from synapse_core.errors import ServiceError
from synapse_core.schemas.auth import LoginRequest, TokenOut
from synapse_core.services import audit
from synapse_core.services import users as svc
from synapse_core.settings import Settings, get_settings

router = APIRouter(prefix="/v1/auth", tags=["auth"])

SettingsDep = Annotated[Settings, Depends(get_settings)]


@router.post("/login", response_model=TokenOut)
async def login(
    request: Request,
    body: LoginRequest,
    session: SessionDep,
    sessions: SessionMakerDep,
    settings: SettingsDep,
) -> TokenOut:
    """Verifies credentials and issues a short-lived access token (`NE1105` on failure).

    Both the success and the failure are audited (the email is the target); the token and
    the password are never logged or stored in the audit detail.
    """
    try:
        token, claims = await svc.authenticate(session, body.email, body.password, settings)
    except ServiceError:
        await audit.record(
            sessions,
            audit.AuditAction.LOGIN,
            actor=body.email.strip().lower(),
            role="anonymous",
            result="denied",
            target=body.email.strip().lower(),
        )
        raise
    request.state.principal_id = f"user:{claims.subject}"
    await audit.record(
        sessions,
        audit.AuditAction.LOGIN,
        actor=f"user:{claims.subject}",
        role=claims.role,
        result="ok",
        target=body.email.strip().lower(),
    )
    return TokenOut(access_token=token, role=claims.role, expires_at=claims.expires_at)
