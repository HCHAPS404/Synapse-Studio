"""Guest mode (E2.8): tokens, sandbox creation, quotas, expiry cleanup, and an end-to-end
guest build of a corpus example through the fake runner (never the real Rust binary)."""

import sys
import time
import uuid
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest
from conftest import needs_db
from fastapi import FastAPI
from httpx import AsyncClient
from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core.api.deps import get_build_manager
from synapse_core.auth.tokens import TokenError, mint_guest_token, verify_guest_token
from synapse_core.compile.runner import CompileRunner, RunnerConfig
from synapse_core.compile.sandbox import Limits
from synapse_core.db.models import GuestSandbox, Project
from synapse_core.db.session import get_engine
from synapse_core.services import guests as svc
from synapse_core.services.builds import BuildManager
from synapse_core.settings import Settings, get_settings

HERE = Path(__file__).parent
FAKE = str(HERE / "fake_neuron.py")
CORPUS = HERE.parents[2] / "examples" / "corpus" / "host" / "h01_gateway_counter.neuron"
FINAL = {"succeeded", "failed", "error"}
SECRET = b"a-test-secret-at-least-32-bytes-long!!"


# ---- tokens (no database) ----------------------------------------------------------


def test_a_minted_token_verifies_and_carries_its_sandbox() -> None:
    sbx = uuid.uuid4()
    token, expires = mint_guest_token(sbx, secret=SECRET, ttl=timedelta(hours=24))
    claims = verify_guest_token(token, secret=SECRET)
    assert claims.sandbox_id == sbx
    assert claims.role == "guest"
    assert abs((claims.expires_at - expires).total_seconds()) < 1


def test_a_tampered_or_wrong_key_token_is_rejected() -> None:
    token, _ = mint_guest_token(uuid.uuid4(), secret=SECRET, ttl=timedelta(hours=1))
    with pytest.raises(TokenError):
        verify_guest_token(token, secret=b"another-secret-entirely-different!!!!")
    payload, sig = token.split(".")
    with pytest.raises(TokenError):
        verify_guest_token(f"{payload}x.{sig}", secret=SECRET)
    with pytest.raises(TokenError):
        verify_guest_token("not-a-token", secret=SECRET)


def test_an_expired_token_is_rejected() -> None:
    past = datetime.now(UTC) - timedelta(hours=2)
    token, _ = mint_guest_token(uuid.uuid4(), secret=SECRET, ttl=timedelta(hours=1), now=past)
    with pytest.raises(TokenError):
        verify_guest_token(token, secret=SECRET)


def test_ip_hash_is_one_way_and_stable() -> None:
    salt = b"salt"
    assert svc.hash_ip(None, salt) is None
    one = svc.hash_ip("1.2.3.4", salt)
    assert one == svc.hash_ip("1.2.3.4", salt)
    assert one != svc.hash_ip("1.2.3.5", salt)
    assert one is not None and "1.2.3.4" not in one  # the IP never appears


# ---- helpers for the DB tests ------------------------------------------------------


def _config(work: Path) -> RunnerConfig:
    return RunnerConfig(
        neuron_cmd=(sys.executable, FAKE), limits=Limits(timeout_s=20), work_root=work
    )


def _manager(app: FastAPI, work: Path, **kw: int) -> BuildManager:
    settings = app.dependency_overrides[get_settings]()
    engine = get_engine(settings)
    assert engine is not None
    manager = BuildManager(
        CompileRunner(_config(work)), async_sessionmaker(engine, expire_on_commit=False), **kw
    )
    app.dependency_overrides[get_build_manager] = lambda: manager
    app.state.build_manager = manager
    return manager


async def _guest(client: AsyncClient) -> dict[str, str]:
    created = await client.post("/v1/guest")
    assert created.status_code == 201, created.text
    return created.json()


def _auth(token: str) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


async def _wait(
    client: AsyncClient,
    build_id: str,
    headers: dict[str, str] | None = None,
    timeout_s: float = 30,
) -> dict[str, object]:
    deadline = time.monotonic() + timeout_s
    while True:
        body = (await client.get(f"/v1/builds/{build_id}", headers=headers)).json()
        if body["status"] in FINAL:
            return body
        assert time.monotonic() < deadline
        import asyncio

        await asyncio.sleep(0.05)


# ---- the route and the end-to-end build --------------------------------------------


@needs_db
async def test_guest_route_issues_a_usable_token(client: AsyncClient) -> None:
    session = await _guest(client)
    assert session["role"] == "guest"
    assert uuid.UUID(session["sandbox_id"])
    # The token verifies with the app's development secret.
    settings = Settings(database_url=None)
    claims = verify_guest_token(session["token"], secret=settings.token_secret())
    assert str(claims.sandbox_id) == session["sandbox_id"]


@needs_db
async def test_a_guest_builds_a_corpus_example_end_to_end(
    app: FastAPI, client: AsyncClient, tmp_path: Path
) -> None:
    _manager(app, tmp_path)
    session = await _guest(client)
    auth = _auth(session["token"])
    project = await client.post("/v1/projects", json={"name": "demo"}, headers=auth)
    assert project.status_code == 201
    # The project is bound to the guest's sandbox.
    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    async with async_sessionmaker(engine)() as s:
        row = await s.get(Project, uuid.UUID(project.json()["id"]))
        assert row is not None and str(row.sandbox_id) == session["sandbox_id"]
    files = {"h01_gateway_counter.neuron": CORPUS.read_text(encoding="utf-8")}
    version = await client.post(
        f"/v1/projects/{project.json()['id']}/versions", json={"files": files}, headers=auth
    )
    assert version.status_code == 201, version.text
    accepted = await client.post(
        f"/v1/versions/{version.json()['id']}/builds", json={"kind": "build"}, headers=auth
    )
    assert accepted.status_code == 202, accepted.text
    done = await _wait(client, accepted.json()["id"], headers=auth)
    assert done["status"] == "succeeded"
    assert {a["name"] for a in done["artifacts"]} == {"app.hex", "sub/app.lst"}  # type: ignore[union-attr]
    # The build counter advanced on the sandbox.
    async with async_sessionmaker(engine)() as s:
        sandbox = await s.get(GuestSandbox, uuid.UUID(session["sandbox_id"]))
        assert sandbox is not None and sandbox.builds_today == 1 and sandbox.project_count == 1


@needs_db
async def test_project_quota_is_ne1104(app: FastAPI, client: AsyncClient) -> None:
    current = app.dependency_overrides[get_settings]()
    settings = Settings(database_url=current.database_url, guest_max_projects=2)
    app.dependency_overrides[get_settings] = lambda: settings
    session = await _guest(client)
    auth = _auth(session["token"])
    assert (await client.post("/v1/projects", json={"name": "a"}, headers=auth)).status_code == 201
    assert (await client.post("/v1/projects", json={"name": "b"}, headers=auth)).status_code == 201
    third = await client.post("/v1/projects", json={"name": "c"}, headers=auth)
    assert third.status_code == 403
    assert third.json()["code"] == "NE1104"


@needs_db
async def test_an_invalid_or_missing_sandbox_token_is_ne1104(
    app: FastAPI, client: AsyncClient
) -> None:
    settings = app.dependency_overrides[get_settings]()
    # A well-signed token for a sandbox that does not exist.
    token, _ = mint_guest_token(
        uuid.uuid4(), secret=settings.token_secret(), ttl=timedelta(hours=1)
    )
    denied = await client.post("/v1/projects", json={"name": "x"}, headers=_auth(token))
    assert denied.status_code == 403
    assert denied.json()["code"] == "NE1104"
    garbage = await client.post("/v1/projects", json={"name": "x"}, headers=_auth("bogus.token"))
    assert garbage.status_code == 403
    assert garbage.json()["code"] == "NE1104"


@needs_db
async def test_expiry_cleanup_deletes_the_sandbox_and_its_projects(
    app: FastAPI, client: AsyncClient
) -> None:
    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    now = datetime.now(UTC)
    async with async_sessionmaker(engine, expire_on_commit=False)() as s:
        live = GuestSandbox(expires_at=now + timedelta(hours=1), build_day="")
        dead = GuestSandbox(expires_at=now - timedelta(minutes=1), build_day="")
        s.add_all([live, dead])
        await s.flush()
        s.add(Project(name="orphan", sandbox_id=dead.id))
        await s.commit()
        live_id, dead_id = live.id, dead.id
    async with async_sessionmaker(engine)() as s:
        removed = await svc.cleanup_expired(s, now=now)
        assert removed == 1
    async with async_sessionmaker(engine)() as s:
        assert await s.get(GuestSandbox, dead_id) is None
        assert await s.get(GuestSandbox, live_id) is not None
        remaining = await svc.count_sandboxes(s)
        assert remaining == 1
