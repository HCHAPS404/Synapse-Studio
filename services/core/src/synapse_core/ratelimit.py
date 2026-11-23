"""Rate-limit middleware: draws from the token buckets of `limits.py` before a handler runs.

Applied as ASGI middleware (the design allows a middleware or a dependency; middleware keeps
the route functions free of limiter logic). `/health` and the schema paths are exempt. A
refused request never reaches its handler: it gets `NE1103` with `Retry-After`.
"""

from __future__ import annotations

from starlette.middleware.base import BaseHTTPMiddleware, RequestResponseEndpoint
from starlette.requests import Request
from starlette.responses import JSONResponse, Response

from synapse_core.errors import ErrorCode
from synapse_core.limits import RateLimiter, client_key, scope_for
from synapse_core.logging_config import request_id_var
from synapse_core.schemas.errors import ErrorBody


def principal_of(request: Request) -> str | None:
    """The authenticated principal of a request, set by the auth dependency (E3).

    Until E3 there is none, so the limiter keys on the client IP.
    """
    return getattr(request.state, "principal_id", None)


def client_ip(request: Request) -> str | None:
    return request.client.host if request.client else None


class RateLimitMiddleware(BaseHTTPMiddleware):
    """Refuses a request with `NE1103` when its bucket is empty.

    The limiter is read from `app.state.rate_limiter` on each request, so tests may swap it
    (and so one instance serves the whole process, as the design states).
    """

    def __init__(self, app: object, limiter: RateLimiter) -> None:
        super().__init__(app)  # type: ignore[arg-type]
        self._fallback = limiter

    async def dispatch(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        scope = scope_for(request.method, request.url.path)
        if scope is None:
            return await call_next(request)
        limiter: RateLimiter = getattr(request.app.state, "rate_limiter", self._fallback)
        key = client_key(principal=principal_of(request), ip=client_ip(request))
        decision = limiter.check(scope, key)
        if not decision.allowed:
            body = ErrorBody(
                code=ErrorCode.RATE_LIMIT.value,
                message="rate limit exceeded; retry after the given delay",
                request_id=request_id_var.get(),
            )
            return JSONResponse(
                body.model_dump(),
                status_code=429,
                headers={"Retry-After": decision.retry_after_header},
            )
        return await call_next(request)
