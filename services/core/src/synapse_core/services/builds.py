"""Build jobs: accept, queue, run on the compile runner, store the result.

A build is accepted (row `queued`) and runs in a background task of this process. At most
`concurrency` jobs run at once; at most `queue_max` more may wait. Beyond that a request is
`NE1103` with `Retry-After`. The queue lives in memory (one instance, see the design), so a
restart marks unfinished builds as `error` (`interrupt_unfinished`).
"""

import asyncio
import logging
import uuid
from datetime import UTC, datetime

from fastapi import HTTPException
from sqlalchemy import func, select, update
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from synapse_core.compile.models import CompileJob, CompileOutcome
from synapse_core.compile.runner import CompileRunner
from synapse_core.db.models import Build, BuildArtifact, ProjectVersion, VersionFile
from synapse_core.errors import ErrorCode, ServiceError
from synapse_core.schemas.builds import BuildOut, BuildPage, BuildRequest

log = logging.getLogger(__name__)

RETRY_AFTER_S = 5


def _not_found(what: str) -> HTTPException:
    return HTTPException(status_code=404, detail=f"{what} not found")


def build_out(build: Build) -> BuildOut:
    """The response model of a stored build."""
    return BuildOut.model_validate(
        {
            "id": build.id,
            "version_id": build.version_id,
            "kind": build.kind,
            "args": build.args,
            "status": build.status,
            "diagnostics": build.diagnostics,
            "messages": build.messages,
            "artifacts": build.artifacts,
            "log_tail": build.log_tail,
            "binary_version": build.binary_version,
            "exit_code": build.exit_code,
            "error_code": build.error_code,
            "error_message": build.error_message,
            "created_at": build.created_at,
            "started_at": build.started_at,
            "finished_at": build.finished_at,
        }
    )


class BuildManager:
    """Owns the compile runner, the concurrency limit and the background tasks."""

    def __init__(
        self,
        runner: CompileRunner,
        sessions: async_sessionmaker[AsyncSession],
        *,
        concurrency: int = 2,
        queue_max: int = 8,
    ) -> None:
        self.runner = runner
        self.concurrency = concurrency
        self.queue_max = queue_max
        self._sessions = sessions
        self._semaphore = asyncio.Semaphore(concurrency)
        self._pending = 0  # accepted and not finished: waiting plus running
        self._tasks: set[asyncio.Task[None]] = set()

    @property
    def pending(self) -> int:
        """Builds accepted and not yet finished."""
        return self._pending

    async def submit(
        self, session: AsyncSession, version_id: uuid.UUID, request: BuildRequest
    ) -> Build:
        """Validates and stores a build as `queued`, then starts it in the background.

        Raises `NE1101` for bad options or files and `NE1103` when the queue is full;
        in both cases nothing is stored.
        """
        if await session.get(ProjectVersion, version_id) is None:
            raise _not_found("version")
        rows = (
            (await session.execute(select(VersionFile).where(VersionFile.version_id == version_id)))
            .scalars()
            .all()
        )
        options = request.model_dump(exclude={"kind"})
        job = CompileJob(command=request.kind, files={r.path: r.content for r in rows}, **options)
        self.runner.validate(job)
        if self._pending >= self.concurrency + self.queue_max:
            raise ServiceError(
                ErrorCode.RATE_LIMIT,
                "the build queue is full; retry shortly",
                headers={"Retry-After": str(RETRY_AFTER_S)},
            )
        build = Build(
            version_id=version_id,
            kind=request.kind.value,
            args={k: v for k, v in options.items() if v not in (None, False)},
            status="queued",
        )
        session.add(build)
        await session.commit()
        self._pending += 1
        task = asyncio.create_task(self._run(build.id, job))
        self._tasks.add(task)
        task.add_done_callback(self._tasks.discard)
        return build

    async def _run(self, build_id: uuid.UUID, job: CompileJob) -> None:
        try:
            async with self._semaphore:
                await self._update(build_id, status="running", started_at=datetime.now(UTC))
                await self._finish(build_id, job)
        except asyncio.CancelledError:
            raise
        except Exception:
            log.exception("build bookkeeping failed", extra={"build_id": str(build_id)})
        finally:
            self._pending -= 1

    async def _finish(self, build_id: uuid.UUID, job: CompileJob) -> None:
        try:
            outcome = await self.runner.run(job)
        except ServiceError as exc:
            await self._update(
                build_id,
                status="error",
                error_code=exc.code.value,
                error_message=exc.message,
                finished_at=datetime.now(UTC),
            )
            return
        except HTTPException as exc:
            await self._update(
                build_id,
                status="error",
                error_message=str(exc.detail),
                finished_at=datetime.now(UTC),
            )
            return
        except Exception:
            log.exception("compile job crashed", extra={"build_id": str(build_id)})
            await self._update(
                build_id,
                status="error",
                error_message="internal error",
                finished_at=datetime.now(UTC),
            )
            return
        await self._store(build_id, outcome)

    async def _update(self, build_id: uuid.UUID, **values: object) -> None:
        async with self._sessions() as session:
            await session.execute(update(Build).where(Build.id == build_id).values(**values))
            await session.commit()

    async def _store(self, build_id: uuid.UUID, outcome: CompileOutcome) -> None:
        result = outcome.result
        async with self._sessions() as session:
            session.add_all(
                BuildArtifact(
                    build_id=build_id,
                    name=info.name,
                    sha256=info.sha256,
                    size=info.size,
                    content=outcome.files[info.name],
                )
                for info in result.artifacts
            )
            await session.execute(
                update(Build)
                .where(Build.id == build_id)
                .values(
                    status="succeeded" if result.ok else "failed",
                    diagnostics=[d.model_dump() for d in result.diagnostics],
                    messages=result.messages,
                    artifacts=[a.model_dump() for a in result.artifacts],
                    log_tail=result.log_tail,
                    binary_version=result.binary_version,
                    exit_code=result.exit_code,
                    finished_at=datetime.now(UTC),
                )
            )
            await session.commit()

    async def wait_idle(self) -> None:
        """Waits for every background task (tests, graceful stop)."""
        while self._tasks:
            await asyncio.gather(*list(self._tasks), return_exceptions=True)

    async def shutdown(self) -> None:
        """Cancels running jobs; their builds are marked at the next start."""
        for task in list(self._tasks):
            task.cancel()
        await asyncio.gather(*list(self._tasks), return_exceptions=True)


async def interrupt_unfinished(session: AsyncSession) -> int:
    """Marks builds left `queued` or `running` by a stopped process as `error`."""
    result = await session.execute(
        update(Build)
        .where(Build.status.in_(("queued", "running")))
        .values(
            status="error",
            error_message="the service restarted before the build finished",
            finished_at=datetime.now(UTC),
        )
    )
    await session.commit()
    return int(result.rowcount or 0)  # type: ignore[attr-defined]


async def get_build(session: AsyncSession, build_id: uuid.UUID) -> Build:
    """The build, or a 404."""
    build = await session.get(Build, build_id)
    if build is None:
        raise _not_found("build")
    return build


async def list_builds(
    session: AsyncSession, version_id: uuid.UUID, limit: int, offset: int
) -> BuildPage:
    """Builds of a version, newest first."""
    if await session.get(ProjectVersion, version_id) is None:
        raise _not_found("version")
    total = (
        await session.execute(
            select(func.count()).select_from(Build).where(Build.version_id == version_id)
        )
    ).scalar_one()
    rows = (
        (
            await session.execute(
                select(Build)
                .where(Build.version_id == version_id)
                .order_by(Build.created_at.desc(), Build.id)
                .limit(limit)
                .offset(offset)
            )
        )
        .scalars()
        .all()
    )
    return BuildPage(items=[build_out(b) for b in rows], total=total, limit=limit, offset=offset)


async def get_artifact(session: AsyncSession, build_id: uuid.UUID, name: str) -> BuildArtifact:
    """One stored output file of a build, or a 404."""
    artifact = await session.get(BuildArtifact, (build_id, name))
    if artifact is None:
        raise _not_found("artifact")
    return artifact
