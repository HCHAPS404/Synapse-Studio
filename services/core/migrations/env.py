"""Alembic environment (synchronous psycopg 3 connection)."""

from logging.config import fileConfig

from alembic import context
from sqlalchemy import create_engine, pool

from synapse_core.db import models  # noqa: F401  (registers tables on the metadata)
from synapse_core.db.base import Base
from synapse_core.db.session import sqlalchemy_url
from synapse_core.settings import get_settings

config = context.config
if config.config_file_name is not None:
    fileConfig(config.config_file_name, disable_existing_loggers=False)

target_metadata = Base.metadata


def _url() -> str:
    """A URL passed by a test (`config.attributes['url']`) or the service settings."""
    override = config.attributes.get("url")
    if override:
        return sqlalchemy_url(str(override))
    database_url = get_settings().database_url
    if database_url is None:
        raise RuntimeError("SYNAPSE_DATABASE_URL is not set")
    return sqlalchemy_url(database_url.get_secret_value())


def run_migrations_offline() -> None:
    """Emits SQL without a connection."""
    context.configure(url=_url(), target_metadata=target_metadata, literal_binds=True)
    with context.begin_transaction():
        context.run_migrations()


def run_migrations_online() -> None:
    """Runs against the database."""
    engine = create_engine(_url(), poolclass=pool.NullPool)
    with engine.connect() as connection:
        context.configure(connection=connection, target_metadata=target_metadata)
        with context.begin_transaction():
            context.run_migrations()
    engine.dispose()


if context.is_offline_mode():
    run_migrations_offline()
else:
    run_migrations_online()
