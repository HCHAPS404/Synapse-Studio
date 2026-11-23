"""Shared fixtures. Database tests need SYNAPSE_TEST_DATABASE_URL (a throwaway database,
never production); without it they are skipped."""

import os
from collections.abc import AsyncIterator, Iterator
from pathlib import Path

import pytest
from alembic import command
from alembic.config import Config
from fastapi import FastAPI
from httpx import ASGITransport, AsyncClient
from pydantic import SecretStr
from sqlalchemy import create_engine, delete

from synapse_core.db.base import Base
from synapse_core.db.models import (
    AuditLog,
    Build,
    BuildArtifact,
    GuestSandbox,
    Project,
    ProjectVersion,
    ServiceToken,
    User,
    VersionFile,
)
from synapse_core.db.session import dispose_engines, sqlalchemy_url
from synapse_core.main import create_app
from synapse_core.settings import Settings, get_settings

TEST_URL = os.environ.get("SYNAPSE_TEST_DATABASE_URL")
needs_db = pytest.mark.skipif(TEST_URL is None, reason="SYNAPSE_TEST_DATABASE_URL is not set")
ROOT = Path(__file__).resolve().parents[1]


@pytest.fixture(scope="session")
def schema() -> Iterator[None]:
    """Creates the schema once: through Alembic on Postgres (so the migrations are the
    thing under test); `create_all` for a SQLite URL (a quick local check only)."""
    if TEST_URL is None:
        yield
        return
    if TEST_URL.startswith("sqlite"):
        engine = create_engine(TEST_URL.replace("+aiosqlite", ""))
        Base.metadata.create_all(engine)
        engine.dispose()
    else:
        config = Config(str(ROOT / "alembic.ini"))
        config.set_main_option("script_location", str(ROOT / "migrations"))
        config.attributes["url"] = TEST_URL
        command.upgrade(config, "head")
    yield


@pytest.fixture
async def app(schema: None) -> AsyncIterator[FastAPI]:
    """The application on the test database; tables are emptied afterwards."""
    assert TEST_URL is not None
    settings = Settings(database_url=SecretStr(TEST_URL))
    application = create_app()
    application.dependency_overrides[get_settings] = lambda: settings
    yield application
    manager = getattr(application.state, "build_manager", None)
    if manager is not None:
        await manager.shutdown()
    await dispose_engines()
    engine = create_engine(sqlalchemy_url(TEST_URL).replace("+aiosqlite", ""))
    with engine.begin() as conn:
        for model in (
            BuildArtifact,
            Build,
            VersionFile,
            ProjectVersion,
            Project,
            GuestSandbox,
            ServiceToken,
            AuditLog,
            User,
        ):
            conn.execute(delete(model))
    engine.dispose()


@pytest.fixture
async def client(app: FastAPI) -> AsyncIterator[AsyncClient]:
    """An HTTP client on `app`."""
    transport = ASGITransport(app=app, raise_app_exceptions=False)
    async with AsyncClient(transport=transport, base_url="http://test") as http:
        yield http


@pytest.fixture
async def engineer_client(app: FastAPI, client: AsyncClient) -> AsyncClient:
    """A client authenticated as an engineer (E3.2): the project/build routes need a role.

    Creates an engineer with the cheapest argon2 parameters, logs in, and sets the bearer
    header on the shared client. Tests of the open-until-E3.2 routes use this so they keep
    exercising the route logic rather than the permission gate.
    """
    from sqlalchemy.ext.asyncio import async_sessionmaker

    from synapse_core.db.session import get_engine
    from synapse_core.services import users as users_svc

    settings = app.dependency_overrides[get_settings]()
    cheap = Settings(
        database_url=settings.database_url,
        argon2_time_cost=1,
        argon2_memory_cost_kib=8,
        argon2_parallelism=1,
    )
    engine = get_engine(settings)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        await users_svc.create_user(
            session, "engineer@test.local", "engineer-password", "engineer", cheap
        )
    resp = await client.post(
        "/v1/auth/login",
        json={"email": "engineer@test.local", "password": "engineer-password"},
    )
    assert resp.status_code == 200, resp.text
    client.headers["Authorization"] = f"Bearer {resp.json()['access_token']}"
    return client
