"""Audit log (E3.4): a row per sensitive action, the engineer read route, no mutation path."""

import uuid

import pytest
from conftest import needs_db
from httpx import AsyncClient
from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core.db.session import get_engine
from synapse_core.services import audit
from synapse_core.services import users as users_svc
from synapse_core.settings import Settings, get_settings

CHEAP = {"argon2_time_cost": 1, "argon2_memory_cost_kib": 8, "argon2_parallelism": 1}


def test_detail_is_scrubbed_of_secret_looking_values() -> None:
    scrubbed = audit._scrub({"token": "svc_x", "password": "p", "number": 3, "cell_id": "c"})
    assert scrubbed["token"] == "***"
    assert scrubbed["password"] == "***"
    assert scrubbed["number"] == 3 and scrubbed["cell_id"] == "c"
    assert audit._scrub(None) == {}


def _sessions(app) -> async_sessionmaker:  # noqa: ANN001
    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    return async_sessionmaker(engine, expire_on_commit=False)


async def _token(app, client: AsyncClient, role: str) -> tuple[str, str]:
    settings = app.dependency_overrides[get_settings]()
    cheap = Settings(database_url=settings.database_url, **CHEAP)
    email = f"{role}-{uuid.uuid4().hex[:8]}@example.com"
    async with _sessions(app)() as s:
        await users_svc.create_user(s, email, "pw-123456", role, cheap)
    resp = await client.post("/v1/auth/login", json={"email": email, "password": "pw-123456"})
    assert resp.status_code == 200
    return resp.json()["access_token"], email


def _auth(token: str) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


async def _actions(app, action: str) -> list[dict]:
    async with _sessions(app)() as s:
        entries, _ = await audit.query(s, action=action, limit=500)
    return [vars(e) for e in entries]


# ---- writes on each sensitive action -----------------------------------------------


@needs_db
async def test_login_success_and_failure_are_audited(app, client: AsyncClient) -> None:
    _, email = await _token(app, client, "viewer")
    await client.post("/v1/auth/login", json={"email": email, "password": "wrong"})
    rows = await _actions(app, "login")
    results = sorted(r["result"] for r in rows)
    assert results == ["denied", "ok"]
    assert all(r["target"] == email for r in rows)
    # No password ever appears in a detail.
    assert all("wrong" not in str(r["detail"]) for r in rows)


@needs_db
async def test_project_version_and_build_creation_are_audited(
    app, client: AsyncClient, tmp_path
) -> None:
    import sys

    from synapse_core.api.deps import get_build_manager
    from synapse_core.compile.runner import CompileRunner, RunnerConfig
    from synapse_core.compile.sandbox import Limits

    fake = str(__import__("pathlib").Path(__file__).parent / "fake_neuron.py")
    manager_runner = CompileRunner(
        RunnerConfig(
            neuron_cmd=(sys.executable, fake), limits=Limits(timeout_s=20), work_root=tmp_path
        )
    )
    from synapse_core.services.builds import BuildManager

    manager = BuildManager(manager_runner, _sessions(app))
    app.dependency_overrides[get_build_manager] = lambda: manager
    app.state.build_manager = manager

    token, _ = await _token(app, client, "engineer")
    auth = _auth(token)
    project = await client.post("/v1/projects", json={"name": "p"}, headers=auth)
    pid = project.json()["id"]
    version = await client.post(
        f"/v1/projects/{pid}/versions",
        json={"files": {"a.neuron": "module a for host\n"}},
        headers=auth,
    )
    vid = version.json()["id"]
    build = await client.post(f"/v1/versions/{vid}/builds", json={"kind": "check"}, headers=auth)
    assert build.status_code == 202

    assert [r["target"] for r in await _actions(app, "project.create")] == [pid]
    version_rows = await _actions(app, "version.create")
    assert version_rows[0]["target"] == vid and version_rows[0]["detail"]["project_id"] == pid
    assert [r["target"] for r in await _actions(app, "build.create")] == [build.json()["id"]]
    await manager.shutdown()


@needs_db
async def test_service_token_issue_and_revoke_are_audited(app, client: AsyncClient) -> None:
    token, _ = await _token(app, client, "engineer")
    auth = _auth(token)
    issued = await client.post(
        "/v1/service-tokens", json={"role": "twin", "label": "t"}, headers=auth
    )
    tid = issued.json()["id"]
    await client.delete(f"/v1/service-tokens/{tid}", headers=auth)
    issue_rows = await _actions(app, "service_token.issue")
    assert issue_rows[0]["target"] == tid
    # The secret is never in the audit detail.
    assert issued.json()["token"] not in str(issue_rows[0]["detail"])
    assert [r["target"] for r in await _actions(app, "service_token.revoke")] == [tid]


# ---- the engineer read route and the absence of a mutation path --------------------


@needs_db
async def test_only_an_engineer_reads_the_audit_log(app, client: AsyncClient) -> None:
    viewer, _ = await _token(app, client, "viewer")
    denied = await client.get("/v1/audit", headers=_auth(viewer))
    assert denied.status_code == 403 and denied.json()["code"] == "NE1106"
    assert (await client.get("/v1/audit")).status_code == 403  # anonymous
    engineer, _ = await _token(app, client, "engineer")
    ok = await client.get("/v1/audit", headers=_auth(engineer))
    assert ok.status_code == 200
    # The login rows of the two users we just created are visible.
    actions = {item["action"] for item in ok.json()["items"]}
    assert "login" in actions


@needs_db
@pytest.mark.parametrize("method", ["POST", "PUT", "PATCH", "DELETE"])
async def test_the_audit_log_has_no_write_route(app, client: AsyncClient, method: str) -> None:
    engineer, _ = await _token(app, client, "engineer")
    resp = await client.request(method, "/v1/audit", headers=_auth(engineer), json={})
    assert resp.status_code in (404, 405)


@needs_db
async def test_filtering_and_the_request_id_correlation(app, client: AsyncClient) -> None:
    engineer, _ = await _token(app, client, "engineer")
    rid = "abc123-correlate"
    await client.post(
        "/v1/projects", json={"name": "x"}, headers={**_auth(engineer), "X-Request-ID": rid}
    )
    rows = await _actions(app, "project.create")
    assert rows[0]["request_id"] == rid
    # Filtering by actor narrows the result.
    async with _sessions(app)() as s:
        by_actor, total = await audit.query(s, actor=rows[0]["actor"])
    assert total >= 1 and all(e.actor == rows[0]["actor"] for e in by_actor)
