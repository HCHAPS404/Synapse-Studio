"""Routes `/v1/projects` and `/v1/versions`.

Until E3 and E2.8 these routes are open and projects have no owner; the permission
dependencies land with E3.2.
"""

import uuid
from typing import Annotated

from fastapi import APIRouter, Depends, Query

from synapse_core.api.deps import OptionalGuestDep
from synapse_core.compile.paths import InputLimits
from synapse_core.db.session import SessionDep
from synapse_core.schemas.projects import (
    ProjectCreate,
    ProjectOut,
    ProjectPage,
    ProjectUpdate,
    VersionCreate,
    VersionOut,
    VersionPage,
)
from synapse_core.services import projects as svc
from synapse_core.settings import Settings, get_settings

router = APIRouter(prefix="/v1", tags=["projects"])

Limit = Annotated[int, Query(ge=1, le=100)]
Offset = Annotated[int, Query(ge=0)]
SettingsDep = Annotated[Settings, Depends(get_settings)]


@router.post("/projects", response_model=ProjectOut, status_code=201)
async def create_project(
    body: ProjectCreate,
    session: SessionDep,
    settings: SettingsDep,
    guest: OptionalGuestDep,
) -> ProjectOut:
    """Creates an empty project. A guest's project is bound to its sandbox and counts
    against the sandbox project quota (`NE1104`)."""
    if guest is not None:
        project = await svc.create_guest_project(session, body.name, guest.sandbox_id, settings)
    else:
        project = await svc.create_project(session, body.name)
    return ProjectOut.model_validate(project)


@router.get("/projects", response_model=ProjectPage)
async def list_projects(session: SessionDep, limit: Limit = 50, offset: Offset = 0) -> ProjectPage:
    """Lists projects, newest first."""
    return await svc.list_projects(session, limit, offset)


@router.get("/projects/{project_id}", response_model=ProjectOut)
async def get_project(project_id: uuid.UUID, session: SessionDep) -> ProjectOut:
    """Reads one project."""
    return ProjectOut.model_validate(await svc.get_project(session, project_id))


@router.put("/projects/{project_id}", response_model=ProjectOut)
async def update_project(
    project_id: uuid.UUID, body: ProjectUpdate, session: SessionDep
) -> ProjectOut:
    """Renames a project; a stale `row_version` is `NE1107`."""
    project = await svc.rename_project(session, project_id, body.name, body.row_version)
    return ProjectOut.model_validate(project)


@router.post("/projects/{project_id}/versions", response_model=VersionOut, status_code=201)
async def create_version(
    project_id: uuid.UUID,
    body: VersionCreate,
    session: SessionDep,
    settings: SettingsDep,
    guest: OptionalGuestDep,
) -> VersionOut:
    """Stores a new immutable version holding exactly the files sent.

    A guest gets tighter file limits (`NE1101` over them): fewer files and smaller files.
    """
    limits = None
    if guest is not None:
        limits = InputLimits(
            max_files=settings.guest_max_files,
            max_file_bytes=settings.guest_max_file_bytes,
        )
    return await svc.create_version(session, project_id, body.files, body.message, limits)


@router.get("/projects/{project_id}/versions", response_model=VersionPage)
async def list_versions(
    project_id: uuid.UUID, session: SessionDep, limit: Limit = 50, offset: Offset = 0
) -> VersionPage:
    """Lists the versions of a project, newest first, without file contents."""
    return await svc.list_versions(session, project_id, limit, offset)


@router.get("/versions/{version_id}", response_model=VersionOut)
async def get_version(version_id: uuid.UUID, session: SessionDep) -> VersionOut:
    """Reads a version with its files."""
    return await svc.get_version(session, version_id)
