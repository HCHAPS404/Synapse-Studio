"""Routes `/v1/projects` and `/v1/versions`.

Until E3 and E2.8 these routes are open and projects have no owner; the permission
dependencies land with E3.2.
"""

import uuid
from typing import Annotated

from fastapi import APIRouter, Query

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

router = APIRouter(prefix="/v1", tags=["projects"])

Limit = Annotated[int, Query(ge=1, le=100)]
Offset = Annotated[int, Query(ge=0)]


@router.post("/projects", response_model=ProjectOut, status_code=201)
async def create_project(body: ProjectCreate, session: SessionDep) -> ProjectOut:
    """Creates an empty project."""
    return ProjectOut.model_validate(await svc.create_project(session, body.name))


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
    project_id: uuid.UUID, body: VersionCreate, session: SessionDep
) -> VersionOut:
    """Stores a new immutable version holding exactly the files sent."""
    return await svc.create_version(session, project_id, body.files, body.message)


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
