"""Dependencies shared by routers."""

from typing import Annotated

from fastapi import Depends, HTTPException, Request
from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core.compile.runner import CompileRunner, runner_config
from synapse_core.compile.sandbox import NetworkSandbox
from synapse_core.db.session import SessionDep, get_engine
from synapse_core.services.builds import BuildManager
from synapse_core.services.guests import GuestPrincipal, principal_from_token
from synapse_core.settings import Settings, get_settings


def _bearer_token(request: Request) -> str | None:
    """The token of an `Authorization: Bearer <token>` header, if present and well formed."""
    header = request.headers.get("Authorization", "")
    scheme, _, value = header.partition(" ")
    if scheme.lower() == "bearer" and value:
        return value.strip()
    return None


async def optional_guest(
    request: Request,
    session: SessionDep,
    settings: Annotated[Settings, Depends(get_settings)],
) -> GuestPrincipal | None:
    """Resolves a guest token when one is sent; a bad or expired token is `NE1104`.

    When there is no token the request is anonymous (the open routes still work until E3
    binds ownership). The principal is recorded on `request.state.principal_id` so the rate
    limiter keys on the guest instead of the shared IP.
    """
    token = _bearer_token(request)
    if token is None:
        return None
    principal = await principal_from_token(session, token, settings)
    request.state.principal_id = principal.key
    return principal


OptionalGuestDep = Annotated["GuestPrincipal | None", Depends(optional_guest)]


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
