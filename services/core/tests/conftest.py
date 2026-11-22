"""Shared fixtures. Database tests need SYNAPSE_TEST_DATABASE_URL (a throwaway database,
never production); without it they are skipped."""

import os
from collections.abc import AsyncIterator, Iterator
from pathlib import Path

import pytest
from alembic import command
from alembic.config import Config
from httpx import ASGITransport, AsyncClient
from pydantic import SecretStr
from sqlalchemy import create_engine, delete

from synapse_core.db.base import Base
from synapse_core.db.models import Project as ProjectRow
from synapse_core.db.models import ProjectVersion, VersionFile
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
async def client(schema: None) -> AsyncIterator[AsyncClient]:
    """An HTTP client on the app with the test database; tables are emptied afterwards."""
    assert TEST_URL is not None
    settings = Settings(database_url=SecretStr(TEST_URL))
    app = create_app()
    app.dependency_overrides[get_settings] = lambda: settings
    transport = ASGITransport(app=app, raise_app_exceptions=False)
    async with AsyncClient(transport=transport, base_url="http://test") as http:
        yield http
    await dispose_engines()
    engine = create_engine(sqlalchemy_url(TEST_URL).replace("+aiosqlite", ""))
    with engine.begin() as conn:
        for model in (VersionFile, ProjectVersion, ProjectRow):
            conn.execute(delete(model))
    engine.dispose()
