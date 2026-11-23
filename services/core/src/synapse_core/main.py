"""FastAPI application entry point."""

import asyncio
import logging
import time
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Annotated, Literal

from fastapi import Depends, FastAPI, Request
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel
from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core import __version__
from synapse_core.api.audit import router as audit_router
from synapse_core.api.auth import router as auth_router
from synapse_core.api.builds import router as builds_router
from synapse_core.api.devices import router as devices_router
from synapse_core.api.guests import router as guests_router
from synapse_core.api.projects import router as projects_router
from synapse_core.api.service_tokens import router as service_tokens_router
from synapse_core.compile.sandbox import NetworkSandbox, detect_network_sandbox
from synapse_core.db.session import dispose_engines, get_engine
from synapse_core.errors import install_error_handlers
from synapse_core.limits import RateLimiter
from synapse_core.logging_config import configure_logging
from synapse_core.middleware import HEADER as REQUEST_ID_HEADER
from synapse_core.middleware import RequestIdMiddleware
from synapse_core.ratelimit import RateLimitMiddleware
from synapse_core.services.builds import interrupt_unfinished
from synapse_core.services.guests import cleanup_expired
from synapse_core.settings import Settings, get_settings

log = logging.getLogger(__name__)

SettingsDep = Annotated[Settings, Depends(get_settings)]


class ComponentStatus(BaseModel):
    """Health of one dependency."""

    status: Literal["ok", "unconfigured", "error"]
    latency_ms: float | None = None
    detail: str | None = None


class SandboxStatus(BaseModel):
    """Result of the compile sandbox self-test."""

    network: Literal["isolated", "unverified"]


class HealthResponse(BaseModel):
    """Response of GET /health."""

    status: Literal["ok", "degraded"]
    version: str
    environment: str
    database: ComponentStatus
    sandbox: SandboxStatus


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


async def _interrupt_unfinished_builds() -> None:
    """Builds a stopped process left queued or running cannot finish: mark them `error`."""
    engine = get_engine(get_settings())
    if engine is None:
        return
    try:
        async with async_sessionmaker(engine)() as session:
            count = await interrupt_unfinished(session)
        if count:
            log.warning("marked unfinished builds as error", extra={"count": count})
    except Exception as exc:  # noqa: BLE001 - e.g. migrations not applied yet; never block start-up
        log.warning("cannot check unfinished builds", extra={"error": type(exc).__name__})


async def _guest_cleanup_loop(interval_s: float) -> None:
    """Deletes expired guest sandboxes and their data on a timer."""
    while True:
        await asyncio.sleep(interval_s)
        engine = get_engine(get_settings())
        if engine is None:
            continue
        try:
            async with async_sessionmaker(engine)() as session:
                removed = await cleanup_expired(session)
            if removed:
                log.info("removed expired guest sandboxes", extra={"count": removed})
        except Exception as exc:  # noqa: BLE001 - never let the loop die
            log.warning("guest cleanup failed", extra={"error": type(exc).__name__})


@asynccontextmanager
async def _lifespan(app: FastAPI) -> AsyncIterator[None]:
    # Self-test of the compile sandbox; the result is reported by /health.
    app.state.network_sandbox = await detect_network_sandbox()
    log.info(
        "compile sandbox",
        extra={
            "network": app.state.network_sandbox.status,
            "detail": app.state.network_sandbox.detail,
        },
    )
    await _interrupt_unfinished_builds()
    cleanup = asyncio.create_task(_guest_cleanup_loop(get_settings().guest_cleanup_interval_s))
    yield
    cleanup.cancel()
    try:
        await cleanup
    except asyncio.CancelledError:
        pass
    manager = getattr(app.state, "build_manager", None)
    if manager is not None:
        await manager.shutdown()
    await dispose_engines()


def create_app() -> FastAPI:
    """Application factory."""
    settings = get_settings()
    configure_logging(settings.log_level, json_output=settings.log_json)
    app = FastAPI(title="SYNAPSE Core API", version=__version__, lifespan=_lifespan)
    install_error_handlers(app)
    limiter = RateLimiter(settings.limiter.scopes())
    app.state.rate_limiter = limiter
    # Middleware runs in reverse add order: request-id outermost, then the limiter, so a
    # refused request still carries a request id and is logged, then CORS headers are added.
    app.add_middleware(RateLimitMiddleware, limiter=limiter)
    app.add_middleware(RequestIdMiddleware)
    app.add_middleware(
        CORSMiddleware,
        allow_origins=settings.cors_origins,
        allow_methods=["GET", "POST", "PUT", "DELETE"],
        allow_headers=["*"],
        expose_headers=[REQUEST_ID_HEADER, "Retry-After"],
    )

    app.include_router(auth_router)
    app.include_router(projects_router)
    app.include_router(builds_router)
    app.include_router(devices_router)
    app.include_router(guests_router)
    app.include_router(service_tokens_router)
    app.include_router(audit_router)

    @app.get("/health", response_model=HealthResponse)
    async def health(request: Request, settings: SettingsDep) -> HealthResponse:
        database = await check_database(settings)
        status: Literal["ok", "degraded"] = "degraded" if database.status == "error" else "ok"
        probed: NetworkSandbox | None = getattr(request.app.state, "network_sandbox", None)
        return HealthResponse(
            status=status,
            version=__version__,
            environment=settings.environment,
            database=database,
            sandbox=SandboxStatus(network=probed.status if probed else "unverified"),
        )

    return app


app = create_app()
