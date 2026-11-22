"""Database tests. Those marked `postgres` need SYNAPSE_TEST_DATABASE_URL (CI service
container); they never touch the production database."""

import os
from pathlib import Path

import pytest
from alembic import command
from alembic.config import Config
from pydantic import SecretStr
from sqlalchemy import text
from sqlalchemy.ext.asyncio import create_async_engine

from synapse_core.db.session import get_engine, sqlalchemy_url
from synapse_core.settings import Settings

TEST_URL = os.environ.get("SYNAPSE_TEST_DATABASE_URL")
postgres = pytest.mark.skipif(TEST_URL is None, reason="SYNAPSE_TEST_DATABASE_URL is not set")
ROOT = Path(__file__).resolve().parents[1]


def test_url_is_rewritten_to_psycopg() -> None:
    assert sqlalchemy_url("postgresql://u:p@h/db").startswith("postgresql+psycopg://")
    assert sqlalchemy_url("postgres://u:p@h/db?sslmode=require").startswith("postgresql+psycopg://")
    assert sqlalchemy_url("postgresql+psycopg://u:p@h/db").startswith("postgresql+psycopg://")


def test_no_engine_without_database_url() -> None:
    assert get_engine(Settings(database_url=None)) is None


def test_engine_is_cached_per_url() -> None:
    settings = Settings(database_url=SecretStr("postgresql://u:p@127.0.0.1:1/db"))
    assert get_engine(settings) is get_engine(settings)


def _alembic() -> Config:
    config = Config(str(ROOT / "alembic.ini"))
    config.set_main_option("script_location", str(ROOT / "migrations"))
    config.attributes["url"] = TEST_URL
    return config


@postgres
def test_migration_round_trip() -> None:
    config = _alembic()
    command.upgrade(config, "head")
    command.downgrade(config, "base")
    command.upgrade(config, "head")


@postgres
async def test_async_session_runs_a_query() -> None:
    assert TEST_URL is not None
    engine = create_async_engine(sqlalchemy_url(TEST_URL))
    try:
        async with engine.connect() as conn:
            assert (await conn.execute(text("SELECT 1"))).scalar_one() == 1
    finally:
        await engine.dispose()
