"""Service tokens (E3.3): hashed at rest, revocable, engineer-only issue route."""

import uuid

import pytest
from conftest import needs_db
from httpx import AsyncClient
from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core.db.models import ServiceToken
from synapse_core.db.session import get_engine
from synapse_core.services import service_tokens as svc
from synapse_core.services import users as users_svc
from synapse_core.settings import Settings, get_settings

CHEAP = {"argon2_time_cost": 1, "argon2_memory_cost_kib": 8, "argon2_parallelism": 1}


# ---- the hash and the plaintext discriminator (no database) ------------------------


def test_the_hash_is_sha256_hex_and_the_prefix_marks_a_service_token() -> None:
    assert len(svc.hash_token("x")) == 64
    assert svc.looks_like_service_token("svc_abc") is True
    assert svc.looks_like_service_token("eyJhbGc.payload.sig") is False
    assert svc.looks_like_service_token("payload.sig") is False


# ---- service helpers against the database ------------------------------------------


def _sessions(app):  # noqa: ANN001, ANN202
    engine = get_engine(app.dependency_overrides[get_settings]())
    assert engine is not None
    return async_sessionmaker(engine, expire_on_commit=False)


@needs_db
async def test_issue_stores_only_the_hash_and_verifies(app) -> None:
    maker = _sessions(app)
    async with maker() as s:
        row, plaintext = await svc.issue_service_token(s, "daemon", cell_id="cell-1", label="pi")
    assert plaintext.startswith("svc_")
    # Only the hash is persisted; the plaintext never appears in the row.
    async with maker() as s:
        stored = await s.get(ServiceToken, row.id)
        assert stored is not None
        assert stored.token_hash == svc.hash_token(plaintext)
        assert plaintext not in (stored.token_hash, stored.label)
    async with maker() as s:
        principal = await svc.principal_from_service_token(s, plaintext)
        assert principal.role == "daemon"
        assert principal.cell_id == "cell-1"


@needs_db
async def test_a_wrong_or_revoked_token_is_rejected(app) -> None:
    maker = _sessions(app)
    async with maker() as s:
        row, plaintext = await svc.issue_service_token(s, "twin")
    async with maker() as s:
        with pytest.raises(Exception):  # noqa: B017 - ServiceError NE1105
            await svc.principal_from_service_token(s, "svc_not-a-real-token")
    async with maker() as s:
        assert await svc.revoke_service_token(s, row.id) is True
    async with maker() as s:
        from synapse_core.errors import ServiceError

        with pytest.raises(ServiceError) as exc:
            await svc.principal_from_service_token(s, plaintext)
        assert exc.value.code.value == "NE1105"
    # Revoking again is a no-op.
    async with maker() as s:
        assert await svc.revoke_service_token(s, row.id) is False
        assert await svc.revoke_service_token(s, uuid.uuid4()) is False


@needs_db
async def test_role_and_cell_binding_rules(app) -> None:
    maker = _sessions(app)
    async with maker() as s:
        with pytest.raises(ValueError, match="daemon.*twin"):
            await svc.issue_service_token(s, "engineer")
        with pytest.raises(ValueError, match="cell_id"):
            await svc.issue_service_token(s, "daemon")
        with pytest.raises(ValueError, match="not bound"):
            await svc.issue_service_token(s, "twin", cell_id="cell-x")


# ---- the engineer-only issue route -------------------------------------------------


async def _token(app, client: AsyncClient, role: str) -> str:
    settings = app.dependency_overrides[get_settings]()
    cheap = Settings(database_url=settings.database_url, **CHEAP)
    email = f"{role}-{uuid.uuid4().hex[:8]}@example.com"
    async with _sessions(app)() as s:
        await users_svc.create_user(s, email, "pw-123456", role, cheap)
    resp = await client.post("/v1/auth/login", json={"email": email, "password": "pw-123456"})
    assert resp.status_code == 200
    return resp.json()["access_token"]


def _auth(token: str) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


@needs_db
async def test_only_an_engineer_can_issue_a_service_token(app, client: AsyncClient) -> None:
    for role in ("viewer", "operator"):
        token = await _token(app, client, role)
        denied = await client.post(
            "/v1/service-tokens", json={"role": "daemon", "cell_id": "c"}, headers=_auth(token)
        )
        assert denied.status_code == 403 and denied.json()["code"] == "NE1106"
    anon = await client.post("/v1/service-tokens", json={"role": "twin"})
    assert anon.status_code == 403 and anon.json()["code"] == "NE1106"


@needs_db
async def test_engineer_issues_lists_and_revokes_a_token(app, client: AsyncClient) -> None:
    eng = await _token(app, client, "engineer")
    issued = await client.post(
        "/v1/service-tokens",
        json={"role": "daemon", "cell_id": "bottling", "label": "lab pi"},
        headers=_auth(eng),
    )
    assert issued.status_code == 201, issued.text
    body = issued.json()
    assert body["token"].startswith("svc_")
    assert body["role"] == "daemon" and body["cell_id"] == "bottling"
    plaintext = body["token"]
    # The listing never carries the secret.
    listed = await client.get("/v1/service-tokens", headers=_auth(eng))
    assert listed.status_code == 200
    item = listed.json()["items"][0]
    assert "token" not in item and item["id"] == body["id"]
    # The issued token authenticates as a daemon (no device.read, so a device route is 403).
    assert (await client.get("/v1/devices", headers=_auth(plaintext))).status_code == 403
    # Revoke it; the engineer-only delete returns 204, then the token stops working.
    revoked = await client.delete(f"/v1/service-tokens/{body['id']}", headers=_auth(eng))
    assert revoked.status_code == 204
    after = await client.get("/v1/devices", headers=_auth(plaintext))
    assert after.status_code == 401 and after.json()["code"] == "NE1105"
    # A second revoke is a 404.
    assert (
        await client.delete(f"/v1/service-tokens/{body['id']}", headers=_auth(eng))
    ).status_code == 404


@needs_db
async def test_a_daemon_token_without_a_cell_is_rejected_by_the_route(
    app, client: AsyncClient
) -> None:
    eng = await _token(app, client, "engineer")
    bad = await client.post("/v1/service-tokens", json={"role": "daemon"}, headers=_auth(eng))
    assert bad.status_code == 422 and bad.json()["code"] == "NE1101"
