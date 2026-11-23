"""Routes for builds and capabilities.

Open until E3.2 and the guest sandbox (E2.8) add permissions and quotas.
"""

import uuid
from typing import Annotated
from urllib.parse import quote

from fastapi import APIRouter, Depends, Query, Request, Response
from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.api.deps import BuildManagerDep, OptionalGuestDep
from synapse_core.compile.runner import runner_config
from synapse_core.compile.sandbox import NetworkSandbox
from synapse_core.db.session import SessionDep
from synapse_core.schemas.builds import BuildOut, BuildPage, BuildRequest, CapabilitiesOut
from synapse_core.services import builds as svc
from synapse_core.services import guests as guest_svc
from synapse_core.services.capabilities import build_capabilities
from synapse_core.settings import Settings, get_settings

router = APIRouter(prefix="/v1", tags=["builds"])

SettingsDep = Annotated[Settings, Depends(get_settings)]
Limit = Annotated[int, Query(ge=1, le=100)]
Offset = Annotated[int, Query(ge=0)]


@router.post("/versions/{version_id}/builds", response_model=BuildOut, status_code=202)
async def create_build(
    version_id: uuid.UUID,
    body: BuildRequest,
    session: SessionDep,
    manager: BuildManagerDep,
    settings: SettingsDep,
    guest: OptionalGuestDep,
) -> BuildOut:
    """Accepts a build; it runs in the background. Poll `GET /v1/builds/{id}`.

    A guest's build counts against the sandbox per-day build quota (`NE1104`)."""
    reserve: svc.BuildReservation | None = None
    if guest is not None:
        sandbox_id = guest.sandbox_id

        async def reserve(s: AsyncSession) -> None:
            await guest_svc.reserve_build(s, sandbox_id, settings)

    return svc.build_out(await manager.submit(session, version_id, body, reserve=reserve))


@router.get("/versions/{version_id}/builds", response_model=BuildPage)
async def list_builds(
    version_id: uuid.UUID, session: SessionDep, limit: Limit = 50, offset: Offset = 0
) -> BuildPage:
    """Lists the builds of a version, newest first."""
    return await svc.list_builds(session, version_id, limit, offset)


@router.get("/builds/{build_id}", response_model=BuildOut)
async def get_build(build_id: uuid.UUID, session: SessionDep) -> BuildOut:
    """Reads a build with its diagnostics and artifact list."""
    return svc.build_out(await svc.get_build(session, build_id))


@router.get(
    "/builds/{build_id}/artifacts/{name:path}",
    response_class=Response,
    responses={200: {"content": {"application/octet-stream": {}}}},
)
async def get_artifact(build_id: uuid.UUID, name: str, session: SessionDep) -> Response:
    """Downloads one output file; `X-Content-SHA256` carries its hash."""
    artifact = await svc.get_artifact(session, build_id, name)
    return Response(
        content=artifact.content,
        media_type="application/octet-stream",
        headers={
            "X-Content-SHA256": artifact.sha256,
            "ETag": f'"{artifact.sha256}"',
            "Content-Disposition": f"attachment; filename*=UTF-8''{quote(name.rsplit('/', 1)[-1])}",
            "X-Content-Type-Options": "nosniff",
        },
    )


@router.get("/capabilities", response_model=CapabilitiesOut)
async def capabilities(request: Request, settings: SettingsDep) -> CapabilitiesOut:
    """Which `neuron` commands and toolchains this image has, and the job limits.

    Needs no database. The probe runs once per process and is cached.
    """
    cached: CapabilitiesOut | None = getattr(request.app.state, "capabilities", None)
    if cached is None:
        network: NetworkSandbox = getattr(
            request.app.state, "network_sandbox", NetworkSandbox("unverified")
        )
        cached = await build_capabilities(
            runner_config(settings, network),
            settings.compile_concurrency,
            settings.compile_queue_max,
        )
        request.app.state.capabilities = cached
    return cached
