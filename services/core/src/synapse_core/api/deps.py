"""Dependencies shared by routers: the current principal and permission checks (E3.2)."""

from collections.abc import Awaitable, Callable
from typing import Annotated

from fastapi import Depends, HTTPException, Request
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from synapse_core.auth.permissions import Permission, has_permission
from synapse_core.compile.runner import CompileRunner, runner_config
from synapse_core.compile.sandbox import NetworkSandbox
from synapse_core.db.session import get_engine
from synapse_core.errors import ErrorCode, ServiceError
from synapse_core.services.builds import BuildManager
from synapse_core.services.guests import GuestPrincipal, principal_from_token
from synapse_core.services.service_tokens import (
    ServicePrincipal,
    looks_like_service_token,
    principal_from_service_token,
)
from synapse_core.services.users import AuthenticatedUser, principal_from_access_token
from synapse_core.settings import Settings, get_settings

# A principal is a logged-in user, a service (daemon/twin), or a guest sandbox; anonymous
# requests have none.
Principal = AuthenticatedUser | ServicePrincipal | GuestPrincipal


def _bearer_token(request: Request) -> str | None:
    """The token of an `Authorization: Bearer <token>` header, if present and well formed."""
    header = request.headers.get("Authorization", "")
    scheme, _, value = header.partition(" ")
    if scheme.lower() == "bearer" and value:
        return value.strip()
    return None


def _looks_like_jwt(token: str) -> bool:
    """A JWT has three dot-separated parts; a guest token (E2.8) has two."""
    return token.count(".") == 2


async def optional_principal(
    request: Request,
    settings: Annotated[Settings, Depends(get_settings)],
) -> Principal | None:
    """Resolves the caller from the bearer token, or `None` when there is none.

    A three-part token is a user JWT (`NE1105` on failure); a two-part token is a guest
    token (`NE1104` on failure). The resolved key is recorded on `request.state.principal_id`
    so the rate limiter keys on the principal rather than the shared client IP. No token
    means no database access, so a public route (and a denied anonymous request) never
    needs a configured database.
    """
    token = _bearer_token(request)
    if token is None:
        return None
    engine = get_engine(settings)
    if engine is None:
        raise HTTPException(status_code=503, detail="database is not configured")
    maker = async_sessionmaker(engine, expire_on_commit=False)
    async with maker() as session:
        principal: Principal
        if looks_like_service_token(token):
            principal = await principal_from_service_token(session, token)
        elif _looks_like_jwt(token):
            principal = await principal_from_access_token(session, token, settings)
        else:
            principal = await principal_from_token(session, token, settings)
    request.state.principal_id = principal.key
    return principal


OptionalPrincipalDep = Annotated["Principal | None", Depends(optional_principal)]


async def optional_guest(
    principal: OptionalPrincipalDep,
) -> GuestPrincipal | None:
    """The principal when it is a guest, else `None` (routes that treat guests specially)."""
    return principal if isinstance(principal, GuestPrincipal) else None


OptionalGuestDep = Annotated["GuestPrincipal | None", Depends(optional_guest)]


def _role_of(principal: Principal) -> str:
    return principal.role


def _denied() -> ServiceError:
    return ServiceError(
        ErrorCode.PERMISSION_DENIED,
        "your role does not have permission for this action",
    )


def require_permission(permission: Permission) -> Callable[..., Awaitable[Principal]]:
    """A dependency that admits only a principal whose role holds `permission` (`NE1106`).

    An anonymous request (no token) is also `NE1106`: these routes are not public.
    """

    async def dependency(principal: OptionalPrincipalDep) -> Principal:
        if principal is None or not has_permission(_role_of(principal), permission):
            raise _denied()
        return principal

    return dependency


def require_any_permission(
    *permissions: Permission,
) -> Callable[..., Awaitable[Principal]]:
    """A dependency that admits a principal holding at least one of `permissions` (`NE1106`).

    Used where a user role and a guest reach the same route through different permissions
    (for example `project.write` for a user, `sandbox.write` for a guest).
    """

    async def dependency(principal: OptionalPrincipalDep) -> Principal:
        if principal is None:
            raise _denied()
        role = _role_of(principal)
        if not any(has_permission(role, p) for p in permissions):
            raise _denied()
        return principal

    return dependency


def get_build_manager(
    request: Request, settings: Annotated[Settings, Depends(get_settings)]
) -> BuildManager:
    """The process-wide build manager, created on first use from the settings.

    Creation has no `await`, so two requests cannot both create one.
    """
    manager: BuildManager | None = getattr(request.app.state, "build_manager", None)
    if manager is None:
        engine = get_engine(settings)
        if engine is None:
            raise HTTPException(status_code=503, detail="database is not configured")
        network: NetworkSandbox = getattr(
            request.app.state, "network_sandbox", NetworkSandbox("unverified")
        )
        manager = BuildManager(
            CompileRunner(runner_config(settings, network)),
            async_sessionmaker(engine, expire_on_commit=False),
            concurrency=settings.compile_concurrency,
            queue_max=settings.compile_queue_max,
        )
        request.app.state.build_manager = manager
    return manager


BuildManagerDep = Annotated[BuildManager, Depends(get_build_manager)]


def get_sessionmaker(
    settings: Annotated[Settings, Depends(get_settings)],
) -> async_sessionmaker[AsyncSession]:
    """A session factory for work outside the request transaction (e.g. audit writes).

    503 when no database is configured.
    """
    engine = get_engine(settings)
    if engine is None:
        raise HTTPException(status_code=503, detail="database is not configured")
    return async_sessionmaker(engine, expire_on_commit=False)


SessionMakerDep = Annotated["async_sessionmaker[AsyncSession]", Depends(get_sessionmaker)]


def principal_identity(principal: Principal) -> tuple[str, str]:
    """`(actor, role)` for the audit log from any principal kind."""
    return principal.key, principal.role
