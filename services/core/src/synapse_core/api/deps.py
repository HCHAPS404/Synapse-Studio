"""Dependencies shared by routers."""

from typing import Annotated

from fastapi import Depends, HTTPException, Request
from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core.compile.runner import CompileRunner, runner_config
from synapse_core.compile.sandbox import NetworkSandbox
from synapse_core.db.session import get_engine
from synapse_core.services.builds import BuildManager
from synapse_core.settings import Settings, get_settings


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
