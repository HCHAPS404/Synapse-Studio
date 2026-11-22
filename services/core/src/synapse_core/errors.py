"""NE11xx catalogue and the exception handlers that turn failures into `ErrorBody`."""

import logging
from enum import StrEnum

from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from starlette.exceptions import HTTPException as StarletteHTTPException

from synapse_core.logging_config import request_id_var
from synapse_core.schemas.errors import ErrorBody

log = logging.getLogger(__name__)


class ErrorCode(StrEnum):
    """Codes of docs/errors/services.md."""

    JOB_REJECTED = "NE1101"
    JOB_TIMEOUT = "NE1102"
    RATE_LIMIT = "NE1103"
    GUEST_QUOTA = "NE1104"
    AUTH_FAILED = "NE1105"
    PERMISSION_DENIED = "NE1106"
    VERSION_CONFLICT = "NE1107"
    SIGNATURE_INVALID = "NE1108"
    COMMAND_NOT_ALLOWED = "NE1109"
    DEPLOYMENT_BLOCKED = "NE1110"
    DEVICE_OFFLINE = "NE1111"
    PROFILE_INVALID = "NE1112"


DEFAULT_STATUS: dict[ErrorCode, int] = {
    ErrorCode.JOB_REJECTED: 422,
    ErrorCode.JOB_TIMEOUT: 408,
    ErrorCode.RATE_LIMIT: 429,
    ErrorCode.GUEST_QUOTA: 403,
    ErrorCode.AUTH_FAILED: 401,
    ErrorCode.PERMISSION_DENIED: 403,
    ErrorCode.VERSION_CONFLICT: 409,
    ErrorCode.SIGNATURE_INVALID: 422,
    ErrorCode.COMMAND_NOT_ALLOWED: 403,
    ErrorCode.DEPLOYMENT_BLOCKED: 409,
    ErrorCode.DEVICE_OFFLINE: 409,
    ErrorCode.PROFILE_INVALID: 422,
}

_HTTP_TO_CODE: dict[int, ErrorCode] = {
    401: ErrorCode.AUTH_FAILED,
    403: ErrorCode.PERMISSION_DENIED,
    409: ErrorCode.VERSION_CONFLICT,
    429: ErrorCode.RATE_LIMIT,
}


class ServiceError(Exception):
    """A failure with a catalogue code; raised by services, rendered by the handler."""

    def __init__(
        self,
        code: ErrorCode,
        message: str,
        *,
        status_code: int | None = None,
        headers: dict[str, str] | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.status_code = status_code or DEFAULT_STATUS[code]
        self.headers = headers or {}


def _response(
    status: int, code: str, message: str, headers: dict[str, str] | None = None
) -> JSONResponse:
    body = ErrorBody(code=code, message=message, request_id=request_id_var.get())
    return JSONResponse(body.model_dump(), status_code=status, headers=headers)


async def _service_error(_: Request, exc: Exception) -> JSONResponse:
    assert isinstance(exc, ServiceError)
    return _response(exc.status_code, exc.code.value, exc.message, exc.headers)


async def _http_error(_: Request, exc: Exception) -> JSONResponse:
    assert isinstance(exc, StarletteHTTPException)
    mapped = _HTTP_TO_CODE.get(exc.status_code)
    code = mapped.value if mapped else f"HTTP_{exc.status_code}"
    headers = dict(exc.headers) if exc.headers else None
    return _response(exc.status_code, code, str(exc.detail), headers)


async def _validation_error(_: Request, exc: Exception) -> JSONResponse:
    assert isinstance(exc, RequestValidationError)
    # Only locations and messages: the input values may hold user data or secrets.
    parts = [f"{'.'.join(str(p) for p in e['loc'])}: {e['msg']}" for e in exc.errors()[:5]]
    return _response(422, ErrorCode.JOB_REJECTED.value, "invalid request: " + "; ".join(parts))


async def _unhandled(_: Request, exc: Exception) -> JSONResponse:
    log.error("unhandled error", exc_info=exc)
    return _response(500, "HTTP_500", "internal error")


def install_error_handlers(app: FastAPI) -> None:
    """Registers every handler on the application."""
    app.add_exception_handler(ServiceError, _service_error)
    app.add_exception_handler(StarletteHTTPException, _http_error)
    app.add_exception_handler(RequestValidationError, _validation_error)
    app.add_exception_handler(Exception, _unhandled)
