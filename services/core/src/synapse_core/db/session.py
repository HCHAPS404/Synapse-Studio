"""Async SQLAlchemy engine and session dependency (driver: psycopg 3)."""

from collections.abc import AsyncIterator
from typing import Annotated

from fastapi import Depends, HTTPException
from sqlalchemy.engine import make_url
from sqlalchemy.ext.asyncio import (
    AsyncEngine,
    AsyncSession,
    async_sessionmaker,
    create_async_engine,
)

from synapse_core.settings import Settings, get_settings

_engines: dict[str, AsyncEngine] = {}


def sqlalchemy_url(url: str) -> str:
    """Rewrites a plain `postgresql://` URL to the psycopg 3 dialect SQLAlchemy needs."""
    parsed = make_url(url)
    if parsed.drivername in ("postgresql", "postgres"):
        parsed = parsed.set(drivername="postgresql+psycopg")
    return parsed.render_as_string(hide_password=False)


def get_engine(settings: Settings) -> AsyncEngine | None:
    """The engine for the configured database, created once per URL; `None` without one."""
    if settings.database_url is None:
        return None
    url = sqlalchemy_url(settings.database_url.get_secret_value())
    engine = _engines.get(url)
    if engine is None:
        engine = create_async_engine(
            url,
            pool_size=settings.db_pool_size,
            pool_pre_ping=True,  # Neon computes suspend; drop dead connections
            pool_recycle=settings.db_pool_recycle_s,
        )
        _engines[url] = engine
    return engine


async def dispose_engines() -> None:
    """Closes every pool (application shutdown, tests)."""
    for engine in list(_engines.values()):
        await engine.dispose()
    _engines.clear()


async def get_session(
    settings: Annotated[Settings, Depends(get_settings)],
) -> AsyncIterator[AsyncSession]:
    """Request-scoped session; rolls back on error. 503 when no database is configured."""
    engine = get_engine(settings)
    if engine is None:
        raise HTTPException(status_code=503, detail="database is not configured")
    maker = async_sessionmaker(engine, expire_on_commit=False)
    async with maker() as session:
        try:
            yield session
        except Exception:
            await session.rollback()
            raise


SessionDep = Annotated[AsyncSession, Depends(get_session)]
