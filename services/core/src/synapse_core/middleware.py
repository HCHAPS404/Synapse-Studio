"""Request id middleware: accepts a sane `X-Request-ID` or makes one, echoes it back."""

import logging
import re
import time
import uuid

from starlette.middleware.base import BaseHTTPMiddleware, RequestResponseEndpoint
from starlette.requests import Request
from starlette.responses import Response

from synapse_core.logging_config import request_id_var

HEADER = "X-Request-ID"
_VALID = re.compile(r"^[A-Za-z0-9._\-]{8,64}$")
log = logging.getLogger("synapse_core.access")


class RequestIdMiddleware(BaseHTTPMiddleware):
    """Binds the request id to a context variable for logs and error bodies."""

    async def dispatch(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        incoming = request.headers.get(HEADER, "")
        request_id = incoming if _VALID.match(incoming) else uuid.uuid4().hex
        token = request_id_var.set(request_id)
        started = time.perf_counter()
        try:
            response = await call_next(request)
        finally:
            request_id_var.reset(token)
        response.headers[HEADER] = request_id
        log.info(
            "request",
            extra={
                "method": request.method,
                "path": request.url.path,
                "status": response.status_code,
                "duration_ms": round((time.perf_counter() - started) * 1000, 2),
                "request_id": request_id,
            },
        )
        return response
