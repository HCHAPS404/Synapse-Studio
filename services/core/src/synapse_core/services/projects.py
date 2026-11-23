"""Project and version logic. Routes call these functions and hold no logic themselves."""

import hashlib
import uuid
from datetime import UTC, datetime

from fastapi import HTTPException
from sqlalchemy import func, select, update
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import AsyncSession

from synapse_core.compile.paths import InputLimits, check_files
from synapse_core.db.models import Project, ProjectVersion, VersionFile
from synapse_core.errors import ErrorCode, ServiceError
from synapse_core.schemas.projects import (
    FileOut,
    ProjectOut,
    ProjectPage,
    VersionOut,
    VersionPage,
    VersionSummary,
)
from synapse_core.services import guests
from synapse_core.services.guests import GuestPrincipal
from synapse_core.settings import Settings


def content_hash(files: dict[str, bytes]) -> str:
    """SHA-256 of a file set, independent of the order the files are given in.

    Every path and content is length-prefixed, so two different sets never produce the
    same byte stream.
    """
    digest = hashlib.sha256()
    for path in sorted(files, key=lambda p: p.encode("utf-8")):
        raw_path = path.encode("utf-8")
        digest.update(len(raw_path).to_bytes(8, "big") + raw_path)
        digest.update(len(files[path]).to_bytes(8, "big") + files[path])
    return digest.hexdigest()


def _not_found(what: str) -> HTTPException:
    return HTTPException(status_code=404, detail=f"{what} not found")


async def create_project(session: AsyncSession, name: str) -> Project:
    """Creates a project at `row_version` 1 (anonymous/user; not bound to a sandbox)."""
    project = Project(name=name)
    session.add(project)
    await session.commit()
    return project


async def create_guest_project(
    session: AsyncSession, name: str, sandbox_id: uuid.UUID, settings: Settings
) -> Project:
    """Creates a project bound to a guest sandbox, reserving its project quota first.

    The quota check locks the sandbox row and the whole thing is one transaction, so two
    concurrent creations cannot both pass a quota of one remaining slot.
    """
    await guests.reserve_project(session, sandbox_id, settings)
    project = Project(name=name, sandbox_id=sandbox_id)
    session.add(project)
    await session.commit()
    return project


async def get_project(
    session: AsyncSession, project_id: uuid.UUID, guest: GuestPrincipal | None = None
) -> Project:
    """The project, or a 404. A guest only ever sees its own sandbox's projects.

    When a guest asks for a project of another sandbox (or a user's project) the answer is
    404, not 403, so the guest cannot even learn the project exists.
    """
    project = await session.get(Project, project_id)
    if project is None or not _visible(project, guest):
        raise _not_found("project")
    return project


def _visible(project: Project, guest: GuestPrincipal | None) -> bool:
    """A guest sees only its own sandbox's projects; a user sees every project."""
    if guest is None:
        return True
    return project.sandbox_id == guest.sandbox_id


async def list_projects(
    session: AsyncSession, limit: int, offset: int, guest: GuestPrincipal | None = None
) -> ProjectPage:
    """A page of projects, newest first; a guest sees only its own sandbox's projects."""
    base = select(Project)
    count = select(func.count()).select_from(Project)
    if guest is not None:
        base = base.where(Project.sandbox_id == guest.sandbox_id)
        count = count.where(Project.sandbox_id == guest.sandbox_id)
    total = (await session.execute(count)).scalar_one()
    rows = (
        (
            await session.execute(
                base.order_by(Project.created_at.desc(), Project.id).limit(limit).offset(offset)
            )
        )
        .scalars()
        .all()
    )
    return ProjectPage(
        items=[ProjectOut.model_validate(r) for r in rows], total=total, limit=limit, offset=offset
    )


async def rename_project(
    session: AsyncSession,
    project_id: uuid.UUID,
    name: str,
    row_version: int,
    guest: GuestPrincipal | None = None,
) -> Project:
    """Renames a project if `row_version` is still current; otherwise `NE1107`.

    The check and the write are one `UPDATE ... WHERE row_version = :seen`, so two
    concurrent writers cannot both succeed. A guest may rename only its own project (a 404
    otherwise, checked before the write).
    """
    await get_project(session, project_id, guest)
    result = await session.execute(
        update(Project)
        .where(Project.id == project_id, Project.row_version == row_version)
        .values(name=name, row_version=row_version + 1, updated_at=datetime.now(UTC))
        .execution_options(synchronize_session=False)
    )
    if result.rowcount == 0:  # type: ignore[attr-defined]
        await session.rollback()
        exists = await session.get(Project, project_id)
        if exists is None:
            raise _not_found("project")
        raise ServiceError(
            ErrorCode.VERSION_CONFLICT,
            "the project changed since you read it; reload and retry",
        )
    await session.commit()
    session.expire_all()
    return await get_project(session, project_id)


async def create_version(
    session: AsyncSession,
    project_id: uuid.UUID,
    files: dict[str, str],
    message: str,
    limits: InputLimits | None = None,
    guest: GuestPrincipal | None = None,
) -> VersionOut:
    """Stores a new immutable version of a project (`NE1101` for bad paths or sizes).

    A guest may add a version only to its own project (a 404 otherwise).
    """
    checked = check_files(files, limits or InputLimits())
    await get_project(session, project_id, guest)
    # Lock the project row so concurrent snapshots get distinct numbers.
    locked = await session.execute(
        select(Project.id).where(Project.id == project_id).with_for_update()
    )
    if locked.scalar_one_or_none() is None:
        raise _not_found("project")
    last = (
        await session.execute(
            select(func.max(ProjectVersion.number)).where(ProjectVersion.project_id == project_id)
        )
    ).scalar_one()
    version = ProjectVersion(
        project_id=project_id,
        number=(last or 0) + 1,
        content_hash=content_hash(checked),
        message=message,
    )
    session.add(version)
    try:
        await session.flush()
        session.add_all(
            VersionFile(
                version_id=version.id, path=path, content=files[path], size=len(checked[path])
            )
            for path in checked
        )
        await session.commit()
    except IntegrityError as exc:
        await session.rollback()
        raise ServiceError(
            ErrorCode.VERSION_CONFLICT, "another version was created at the same time; retry"
        ) from exc
    return await get_version(session, version.id)


def _summary(version: ProjectVersion, file_count: int, total_bytes: int) -> VersionSummary:
    return VersionSummary(
        id=version.id,
        project_id=version.project_id,
        number=version.number,
        content_hash=version.content_hash,
        message=version.message,
        created_at=version.created_at,
        file_count=file_count,
        total_bytes=total_bytes,
    )


async def get_version(
    session: AsyncSession, version_id: uuid.UUID, guest: GuestPrincipal | None = None
) -> VersionOut:
    """A version with its files, or a 404. A guest sees only its own project's versions."""
    version = await session.get(ProjectVersion, version_id)
    if version is None:
        raise _not_found("version")
    if guest is not None:
        # Ensure the version's project belongs to this guest (404 otherwise).
        await get_project(session, version.project_id, guest)
    rows = (
        (
            await session.execute(
                select(VersionFile)
                .where(VersionFile.version_id == version_id)
                .order_by(VersionFile.path)
            )
        )
        .scalars()
        .all()
    )
    summary = _summary(version, len(rows), sum(r.size for r in rows))
    return VersionOut(
        **summary.model_dump(),
        files=[FileOut(path=r.path, content=r.content, size=r.size) for r in rows],
    )


async def list_versions(
    session: AsyncSession,
    project_id: uuid.UUID,
    limit: int,
    offset: int,
    guest: GuestPrincipal | None = None,
) -> VersionPage:
    """Version summaries of a project, newest first; a guest only its own project's."""
    await get_project(session, project_id, guest)
    total = (
        await session.execute(
            select(func.count())
            .select_from(ProjectVersion)
            .where(ProjectVersion.project_id == project_id)
        )
    ).scalar_one()
    rows = (
        await session.execute(
            select(
                ProjectVersion,
                func.count(VersionFile.path),
                func.coalesce(func.sum(VersionFile.size), 0),
            )
            .outerjoin(VersionFile, VersionFile.version_id == ProjectVersion.id)
            .where(ProjectVersion.project_id == project_id)
            .group_by(ProjectVersion.id)
            .order_by(ProjectVersion.number.desc())
            .limit(limit)
            .offset(offset)
        )
    ).all()
    return VersionPage(
        items=[_summary(v, int(n), int(b)) for v, n, b in rows],
        total=total,
        limit=limit,
        offset=offset,
    )
