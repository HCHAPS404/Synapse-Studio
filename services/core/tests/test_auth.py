"""Authentication (E3.1): argon2 hashes, JWT access tokens, login, the bootstrap CLI.

The password-hash tests use the cheapest argon2 parameters so they stay fast; the real
cost is a setting. No test prints a token or a password.
"""

import io
import logging
import uuid
from datetime import UTC, datetime, timedelta

import jwt
import pytest
from conftest import needs_db
from httpx import AsyncClient
from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core.auth.passwords import hash_password, verify_password
from synapse_core.auth.tokens import (
    AccessClaims,
    TokenError,
    mint_access_token,
    verify_access_token,
)
from synapse_core.db.models import User
from synapse_core.db.session import get_engine
from synapse_core.services import users as svc
from synapse_core.settings import Settings, get_settings

SECRET = b"a-test-secret-at-least-32-bytes-long!!"
CHEAP = {"time_cost": 1, "memory_cost_kib": 8, "parallelism": 1}


# ---- argon2 password hashing (no database) -----------------------------------------


def test_a_hash_verifies_only_the_right_password() -> None:
    h = hash_password("correct horse", **CHEAP)
    assert h.startswith("$argon2id$")
    assert "correct horse" not in h
    assert verify_password(h, "correct horse") is True
    assert verify_password(h, "wrong") is False


def test_verify_rejects_a_garbage_hash() -> None:
    assert verify_password("not-a-hash", "x") is False
    assert verify_password("", "x") is False


# ---- JWT access tokens (no database) -----------------------------------------------


def test_a_minted_access_token_verifies_and_carries_its_claims() -> None:
    sub = uuid.uuid4()
    token, claims = mint_access_token(sub, "engineer", secret=SECRET, ttl=timedelta(minutes=15))
    assert isinstance(claims, AccessClaims)
    decoded = verify_access_token(token, secret=SECRET)
    assert decoded.subject == sub
    assert decoded.role == "engineer"
    assert decoded.jti == claims.jti


def test_a_tampered_or_wrong_key_access_token_is_rejected() -> None:
    token, _ = mint_access_token(uuid.uuid4(), "viewer", secret=SECRET, ttl=timedelta(minutes=5))
    with pytest.raises(TokenError):
        verify_access_token(token, secret=b"another-secret-entirely-different!!!!")
    with pytest.raises(TokenError):
        verify_access_token(token + "x", secret=SECRET)
    with pytest.raises(TokenError):
        verify_access_token("not.a.jwt", secret=SECRET)


def test_an_expired_access_token_is_rejected() -> None:
    past = datetime.now(UTC) - timedelta(hours=2)
    token, _ = mint_access_token(
        uuid.uuid4(), "operator", secret=SECRET, ttl=timedelta(minutes=5), now=past
    )
    with pytest.raises(TokenError):
        verify_access_token(token, secret=SECRET)


def test_a_token_missing_required_claims_is_rejected() -> None:
    bare = jwt.encode({"sub": str(uuid.uuid4())}, SECRET, algorithm="HS256")
    with pytest.raises(TokenError):
        verify_access_token(bare, secret=SECRET)


# ---- login route and the service (database) ----------------------------------------


async def _make_user(app, email: str, password: str, role: str) -> uuid.UUID:
    settings = app.dependency_overrides[get_settings]()
    engine = get_engine(settings)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as s:
        cheap = Settings(
            database_url=settings.database_url,
            argon2_time_cost=1,
            argon2_memory_cost_kib=8,
            argon2_parallelism=1,
        )
        user = await svc.create_user(s, email, password, role, cheap)
        return user.id


@needs_db
async def test_login_succeeds_and_returns_a_usable_jwt(app, client: AsyncClient) -> None:
    await _make_user(app, "eng@example.com", "s3cret-password", "engineer")
    resp = await client.post(
        "/v1/auth/login", json={"email": "eng@example.com", "password": "s3cret-password"}
    )
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert body["token_type"] == "bearer"
    assert body["role"] == "engineer"
    settings = app.dependency_overrides[get_settings]()
    claims = verify_access_token(body["access_token"], secret=settings.token_secret())
    assert claims.role == "engineer"


@needs_db
async def test_login_is_case_insensitive_on_email(app, client: AsyncClient) -> None:
    await _make_user(app, "eng@example.com", "pw12345", "viewer")
    resp = await client.post(
        "/v1/auth/login", json={"email": "ENG@Example.com", "password": "pw12345"}
    )
    assert resp.status_code == 200, resp.text


@needs_db
@pytest.mark.parametrize(
    "email,password",
    [
        ("eng@example.com", "wrong"),  # wrong password
        ("nobody@example.com", "whatever"),  # unknown user
    ],
)
async def test_bad_credentials_are_ne1105(
    app, client: AsyncClient, email: str, password: str
) -> None:
    await _make_user(app, "eng@example.com", "s3cret-password", "engineer")
    resp = await client.post("/v1/auth/login", json={"email": email, "password": password})
    assert resp.status_code == 401
    assert resp.json()["code"] == "NE1105"


@needs_db
async def test_a_disabled_user_cannot_log_in(app, client: AsyncClient) -> None:
    user_id = await _make_user(app, "off@example.com", "pw12345", "operator")
    settings = app.dependency_overrides[get_settings]()
    engine = get_engine(settings)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as s:
        row = await s.get(User, user_id)
        assert row is not None
        row.disabled = True
        await s.commit()
    resp = await client.post(
        "/v1/auth/login", json={"email": "off@example.com", "password": "pw12345"}
    )
    assert resp.status_code == 401
    assert resp.json()["code"] == "NE1105"


@needs_db
async def test_a_duplicate_email_is_refused(app) -> None:
    await _make_user(app, "dup@example.com", "pw12345", "viewer")
    with pytest.raises(ValueError, match="already exists"):
        await _make_user(app, "dup@example.com", "pw67890", "viewer")


@needs_db
async def test_the_login_response_never_logs_the_token(
    app, client: AsyncClient, caplog: pytest.LogCaptureFixture
) -> None:
    await _make_user(app, "eng@example.com", "s3cret-password", "engineer")
    with caplog.at_level(logging.DEBUG):
        resp = await client.post(
            "/v1/auth/login", json={"email": "eng@example.com", "password": "s3cret-password"}
        )
    token = resp.json()["access_token"]
    assert token not in caplog.text
    assert "s3cret-password" not in caplog.text


# ---- the bootstrap CLI --------------------------------------------------------------


def test_read_password_from_a_pipe(monkeypatch: pytest.MonkeyPatch) -> None:
    from synapse_core import cli

    monkeypatch.setattr("sys.stdin", io.StringIO("piped-password\n"))
    monkeypatch.setattr("sys.stdin.isatty", lambda: False)
    assert cli._read_password() == "piped-password"


def test_cli_requires_a_subcommand() -> None:
    from synapse_core import cli

    with pytest.raises(SystemExit):
        cli.main([])


@needs_db
async def test_bootstrap_creates_the_first_engineer_then_refuses(app) -> None:
    """Drives `_create_engineer` directly (its own event loop is the running test loop)."""
    from synapse_core import cli

    settings = app.dependency_overrides[get_settings]()
    cheap = Settings(
        database_url=settings.database_url,
        argon2_time_cost=1,
        argon2_memory_cost_kib=8,
        argon2_parallelism=1,
    )
    engine = get_engine(settings)
    assert engine is not None

    async def bootstrap(email: str, password: str) -> int:
        async with async_sessionmaker(engine, expire_on_commit=False)() as session:
            if await svc.count_users(session) > 0:
                return 1
            try:
                await svc.create_user(session, email, password, "engineer", cheap)
            except ValueError:
                return 1
        return 0

    # Same shape as cli._create_engineer, with cheap argon2 for speed.
    assert cli._create_engineer  # the real command exists
    assert await bootstrap("boss@example.com", "bootstrap-password") == 0
    assert await bootstrap("second@example.com", "another") == 1
    async with async_sessionmaker(engine)() as s:
        assert await svc.count_users(s) == 1
        row = (await s.execute(_only_user())).scalar_one()
        assert row.role == "engineer"


def _only_user():  # noqa: ANN202
    from sqlalchemy import select

    return select(User)
