"""FastAPI application entry point."""

import asyncio
import time
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Annotated, Literal

from fastapi import Depends, FastAPI
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel

from synapse_core import __version__
from synapse_core.db.session import dispose_engines
from synapse_core.errors import install_error_handlers
from synapse_core.logging_config import configure_logging
from synapse_core.middleware import HEADER as REQUEST_ID_HEADER
from synapse_core.middleware import RequestIdMiddleware
from synapse_core.settings import Settings, get_settings

SettingsDep = Annotated[Settings, Depends(get_settings)]


class ComponentStatus(BaseModel):
    """Health of one dependency."""

    status: Literal["ok", "unconfigured", "error"]
    latency_ms: float | None = None
    detail: str | None = None


class HealthResponse(BaseModel):
    """Response of GET /health."""

    status: Literal["ok", "degraded"]
    version: str
    environment: str
    database: ComponentStatus


def _check_database_sync(url: str, timeout_s: float) -> ComponentStatus:
    import psycopg  # imported lazily so the service starts without a database

    started = time.perf_counter()
    try:
        with psycopg.connect(url, connect_timeout=max(1, int(timeout_s))) as conn:
            conn.execute("SELECT 1")
        return ComponentStatus(status="ok", latency_ms=(time.perf_counter() - started) * 1000)
    except Exception as exc:  # noqa: BLE001 - report any failure as status, never crash health
        # Never echo the connection string; only the exception type.
        return ComponentStatus(status="error", detail=type(exc).__name__)


async def check_database(settings: Settings) -> ComponentStatus:
    """Runs `SELECT 1` against the configured database, if any."""
    if settings.database_url is None:
        return ComponentStatus(status="unconfigured")
    return await asyncio.to_thread(
        _check_database_sync,
        settings.database_url.get_secret_value(),
        settings.database_timeout_s,
    )


@asynccontextmanager
async def _lifespan(_: FastAPI) -> AsyncIterator[None]:
    yield
    await dispose_engines()


def create_app() -> FastAPI:
    """Application factory."""
    settings = get_settings()
    configure_logging(settings.log_level, json_output=settings.log_json)
    app = FastAPI(title="SYNAPSE Core API", version=__version__, lifespan=_lifespan)
    install_error_handlers(app)
    app.add_middleware(RequestIdMiddleware)
    app.add_middleware(
        CORSMiddleware,
        allow_origins=settings.cors_origins,
        allow_methods=["GET", "POST", "PUT", "DELETE"],
        allow_headers=["*"],
        expose_headers=[REQUEST_ID_HEADER, "Retry-After"],
    )

    @app.get("/health", response_model=HealthResponse)
    async def health(settings: SettingsDep) -> HealthResponse:
        database = await check_database(settings)
        status: Literal["ok", "degraded"] = "degraded" if database.status == "error" else "ok"
        return HealthResponse(
            status=status,
            version=__version__,
            environment=settings.environment,
            database=database,
        )

    return app


app = create_app()
