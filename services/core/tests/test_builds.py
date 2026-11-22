"""Builds API and capabilities (E2.5). The compiler is the fake `neuron` of the runner tests."""

import asyncio
import hashlib
import sys
import time
import uuid
from pathlib import Path

from conftest import needs_db
from fastapi import FastAPI
from fastapi.testclient import TestClient
from httpx import AsyncClient
from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core.api.deps import get_build_manager
from synapse_core.compile.models import CompileJob, CompileOutcome, CompileResult
from synapse_core.compile.runner import CompileRunner, RunnerConfig
from synapse_core.compile.sandbox import Limits
from synapse_core.db.models import Build
from synapse_core.db.session import get_engine
from synapse_core.main import create_app
from synapse_core.services.builds import BuildManager, interrupt_unfinished
from synapse_core.services.capabilities import build_capabilities
from synapse_core.settings import Settings, get_settings

HERE = Path(__file__).parent
FAKE = str(HERE / "fake_neuron.py")
FINAL = {"succeeded", "failed", "error"}


def _config(work: Path, timeout_s: float = 20) -> RunnerConfig:
    return RunnerConfig(
        neuron_cmd=(sys.executable, FAKE), limits=Limits(timeout_s=timeout_s), work_root=work
    )


# ---- capabilities (no database) ------------------------------------------------------


async def test_capabilities_with_the_fake_compiler(tmp_path: Path) -> None:
    caps = await build_capabilities(_config(tmp_path), concurrency=2, queue_max=8)
    assert caps.neuron.available
    assert caps.neuron.version == "neuron 9.9.9-fake"
    assert caps.commands == ["check", "build", "emit"]
    assert "st" in caps.emit_languages
    assert set(caps.toolchains) == {"gcc", "avr-gcc", "arm-none-eabi-gcc"}
    assert caps.sandbox_network == "unverified"
    assert caps.limits.concurrency == 2
    assert caps.limits.max_files == 200


def test_capabilities_route_without_compiler_or_database() -> None:
    settings = Settings(database_url=None, neuron_bin="synapse-no-such-binary")
    app = create_app()
    app.dependency_overrides[get_settings] = lambda: settings
    response = TestClient(app).get("/v1/capabilities")
    assert response.status_code == 200
    body = response.json()
    assert body["neuron"] == {"available": False, "version": None}
    assert body["commands"] == []
    assert body["limits"]["timeout_s"] == settings.compile_timeout_s


def test_build_routes_need_a_database() -> None:
    app = create_app()
    app.dependency_overrides[get_settings] = lambda: Settings(database_url=None)
    response = TestClient(app).post(f"/v1/versions/{uuid.uuid4()}/builds", json={"kind": "check"})
    assert response.status_code == 503


# ---- builds -------------------------------------------------------------------------


class GatedRunner(CompileRunner):
    """A runner that waits for a gate and records how many jobs run at once."""

    def __init__(self, config: RunnerConfig) -> None:
        super().__init__(config)
        self.gate = asyncio.Event()
        self.running = 0
        self.peak = 0

    async def run(self, job: CompileJob) -> CompileOutcome:
        self.running += 1
        self.peak = max(self.peak, self.running)
        try:
            await self.gate.wait()
        finally:
            self.running -= 1
        return CompileOutcome(result=CompileResult(ok=True, exit_code=0, diagnostics=[]))


def _manager(app: FastAPI, runner: CompileRunner, **kw: int) -> BuildManager:
    settings = app.dependency_overrides[get_settings]()
    engine = get_engine(settings)
    assert engine is not None
    manager = BuildManager(runner, async_sessionmaker(engine, expire_on_commit=False), **kw)
    app.dependency_overrides[get_build_manager] = lambda: manager
    app.state.build_manager = manager
    return manager


async def _version(client: AsyncClient, mode: str = "ok") -> str:
    project = (await client.post("/v1/projects", json={"name": "p"})).json()
    files = {"a.neuron": "module a for host\n", "mode.txt": mode}
    response = await client.post(f"/v1/projects/{project['id']}/versions", json={"files": files})
    assert response.status_code == 201, response.text
    return str(response.json()["id"])


async def _wait(client: AsyncClient, build_id: str, timeout_s: float = 30) -> dict[str, object]:
    deadline = time.monotonic() + timeout_s
    while True:
        body = (await client.get(f"/v1/builds/{build_id}")).json()
        if body["status"] in FINAL:
            return body
        assert time.monotonic() < deadline, f"build still {body['status']}"
        await asyncio.sleep(0.05)


@needs_db
async def test_build_runs_and_stores_artifacts(
    app: FastAPI, client: AsyncClient, tmp_path: Path
) -> None:
    _manager(app, CompileRunner(_config(tmp_path)))
    version = await _version(client)
    accepted = await client.post(f"/v1/versions/{version}/builds", json={"kind": "build"})
    assert accepted.status_code == 202, accepted.text
    assert accepted.json()["status"] in {"queued", "running", "succeeded"}
    done = await _wait(client, accepted.json()["id"])
    assert done["status"] == "succeeded"
    assert done["binary_version"] == "9.9.9-fake"
    assert done["exit_code"] == 0
    assert [d["code"] for d in done["diagnostics"]] == ["NL0402"]  # type: ignore[union-attr]
    names = {a["name"]: a for a in done["artifacts"]}  # type: ignore[union-attr]
    assert set(names) == {"app.hex", "sub/app.lst"}
    # The fake writes text files, so Windows may store CRLF: compare with what is served.
    download = await client.get(f"/v1/builds/{done['id']}/artifacts/sub/app.lst")
    assert download.status_code == 200
    assert download.content.strip() == b"listing"
    digest = hashlib.sha256(download.content).hexdigest()
    assert download.headers["x-content-sha256"] == digest
    assert names["sub/app.lst"]["sha256"] == digest
    assert names["sub/app.lst"]["size"] == len(download.content)
    hexfile = await client.get(f"/v1/builds/{done['id']}/artifacts/app.hex")
    assert hexfile.content.startswith(b":00000001FF")
    assert names["app.hex"]["sha256"] == hashlib.sha256(hexfile.content).hexdigest()
    assert (await client.get(f"/v1/builds/{done['id']}/artifacts/nope")).status_code == 404
    listing = (await client.get(f"/v1/versions/{version}/builds")).json()
    assert listing["total"] == 1
    assert listing["items"][0]["id"] == done["id"]


@needs_db
async def test_program_errors_are_a_failed_build_not_an_error(
    app: FastAPI, client: AsyncClient, tmp_path: Path
) -> None:
    _manager(app, CompileRunner(_config(tmp_path)))
    version = await _version(client, "errors")
    accepted = await client.post(f"/v1/versions/{version}/builds", json={"kind": "check"})
    done = await _wait(client, accepted.json()["id"])
    assert done["status"] == "failed"
    assert done["error_code"] is None
    diagnostic = done["diagnostics"][0]  # type: ignore[index]
    assert diagnostic["code"] == "NE0201"
    assert diagnostic["file"] == "a.neuron"  # the job directory is not exposed
    assert done["artifacts"] == []


@needs_db
async def test_timeout_is_an_error_build_with_ne1102(
    app: FastAPI, client: AsyncClient, tmp_path: Path
) -> None:
    _manager(app, CompileRunner(_config(tmp_path, timeout_s=1)))
    version = await _version(client, "sleep")
    accepted = await client.post(f"/v1/versions/{version}/builds", json={"kind": "check"})
    done = await _wait(client, accepted.json()["id"])
    assert done["status"] == "error"
    assert done["error_code"] == "NE1102"
    assert done["finished_at"] is not None


@needs_db
async def test_bad_options_are_rejected_before_anything_is_stored(
    app: FastAPI, client: AsyncClient, tmp_path: Path
) -> None:
    manager = _manager(app, CompileRunner(_config(tmp_path)))
    version = await _version(client)
    for body in (
        {"kind": "check", "board": "uno"},
        {"kind": "emit"},
        {"kind": "emit", "language": "nope"},
        {"kind": "build", "native": True, "host": True},
        {"kind": "build", "board": "--evil"},
    ):
        response = await client.post(f"/v1/versions/{version}/builds", json=body)
        assert response.status_code == 422, body
        assert response.json()["code"] == "NE1101", body
    assert (
        await client.post(f"/v1/versions/{version}/builds", json={"kind": "rm"})
    ).status_code == 422
    assert (await client.get(f"/v1/versions/{version}/builds")).json()["total"] == 0
    assert manager.pending == 0
    missing = await client.post(f"/v1/versions/{uuid.uuid4()}/builds", json={"kind": "check"})
    assert missing.status_code == 404
    assert (await client.get(f"/v1/builds/{uuid.uuid4()}")).status_code == 404


@needs_db
async def test_concurrency_is_bounded_and_a_full_queue_is_ne1103(
    app: FastAPI, client: AsyncClient, tmp_path: Path
) -> None:
    runner = GatedRunner(_config(tmp_path))
    manager = _manager(app, runner, concurrency=2, queue_max=2)
    version = await _version(client)
    ids = []
    for _ in range(4):
        response = await client.post(f"/v1/versions/{version}/builds", json={"kind": "check"})
        assert response.status_code == 202
        ids.append(response.json()["id"])
    full = await client.post(f"/v1/versions/{version}/builds", json={"kind": "check"})
    assert full.status_code == 429
    assert full.json()["code"] == "NE1103"
    assert int(full.headers["retry-after"]) >= 1
    for _ in range(100):
        if runner.running == 2:
            break
        await asyncio.sleep(0.02)
    assert runner.running == 2
    statuses = []
    for i in ids:
        statuses.append((await client.get(f"/v1/builds/{i}")).json()["status"])
    statuses.sort()
    assert statuses == ["queued", "queued", "running", "running"]
    runner.gate.set()
    await manager.wait_idle()
    for i in ids:
        assert (await _wait(client, i))["status"] == "succeeded"
    assert runner.peak == 2
    assert manager.pending == 0
    again = await client.post(f"/v1/versions/{version}/builds", json={"kind": "check"})
    assert again.status_code == 202  # room again after the queue drained
    await manager.wait_idle()


@needs_db
async def test_interrupted_builds_are_marked_on_restart(
    app: FastAPI, client: AsyncClient, tmp_path: Path
) -> None:
    manager = _manager(app, CompileRunner(_config(tmp_path)))
    version = await _version(client)
    async with manager._sessions() as session:  # noqa: SLF001
        for status in ("queued", "running", "succeeded"):
            session.add(Build(version_id=uuid.UUID(version), kind="check", status=status))
        await session.commit()
        assert await interrupt_unfinished(session) == 2
    items = (await client.get(f"/v1/versions/{version}/builds")).json()["items"]
    assert sorted(b["status"] for b in items) == ["error", "error", "succeeded"]
