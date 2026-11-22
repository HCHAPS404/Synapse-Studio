"""Structured JSON logging with a request id and redaction of secrets."""

import contextvars
import json
import logging
import re
import sys
from collections.abc import Mapping
from datetime import UTC, datetime
from typing import Any

REDACTED = "[REDACTED]"

request_id_var: contextvars.ContextVar[str | None] = contextvars.ContextVar(
    "request_id", default=None
)

# URLs with credentials: scheme://user:password@host
_URL_CREDENTIALS = re.compile(r"([a-zA-Z][a-zA-Z0-9+.\-]*://)[^\s/@:]*:[^\s/@]*@")
# JSON Web Tokens (three base64url parts, the first two start with eyJ)
_JWT = re.compile(r"eyJ[A-Za-z0-9_\-]{5,}\.[A-Za-z0-9_\-]{5,}\.[A-Za-z0-9_\-]*")
# Authorization schemes
_BEARER = re.compile(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=\-]+")
# key=value or "key": "value" for sensitive keys
_SENSITIVE_KEYS = r"password|passwd|secret|token|api[_-]?key|authorization|signing[_-]?key|jwt"
_KEY_VALUE = re.compile(
    rf"(?i)((?:{_SENSITIVE_KEYS})[\w\-]*[\"']?\s*[=:]\s*)(\"[^\"]*\"|'[^']*'|[^\s,;&\"']+)"
)
_SENSITIVE_FIELD = re.compile(rf"(?i)({_SENSITIVE_KEYS})")

_STANDARD_ATTRS = frozenset(
    logging.LogRecord("", 0, "", 0, "", (), None).__dict__.keys() | {"message", "asctime"}
)


def redact_text(text: str) -> str:
    """Removes credentials, tokens and secret-looking values from free text."""
    text = _URL_CREDENTIALS.sub(rf"\1{REDACTED}@", text)
    text = _JWT.sub(REDACTED, text)
    text = _BEARER.sub(rf"\1 {REDACTED}", text)
    return _KEY_VALUE.sub(rf"\1{REDACTED}", text)


def redact_value(key: str, value: Any) -> Any:
    """Redacts a structured log field by its name or its content."""
    if _SENSITIVE_FIELD.search(key):
        return REDACTED
    if isinstance(value, str):
        return redact_text(value)
    if isinstance(value, Mapping):
        return {str(k): redact_value(str(k), v) for k, v in value.items()}
    if isinstance(value, list | tuple):
        return [redact_value(key, v) for v in value]
    return value


class RedactingFilter(logging.Filter):
    """Redacts the message, its arguments and extra fields of every record."""

    def filter(self, record: logging.LogRecord) -> bool:
        record.msg = redact_text(record.getMessage())
        record.args = None
        for key in list(record.__dict__):
            if key not in _STANDARD_ATTRS:
                record.__dict__[key] = redact_value(key, record.__dict__[key])
        if record.exc_text:
            record.exc_text = redact_text(record.exc_text)
        return True


class JsonFormatter(logging.Formatter):
    """One JSON object per line."""

    def format(self, record: logging.LogRecord) -> str:
        payload: dict[str, Any] = {
            "ts": datetime.fromtimestamp(record.created, UTC).isoformat(timespec="milliseconds"),
            "level": record.levelname,
            "logger": record.name,
            "message": record.getMessage(),
            "request_id": request_id_var.get(),
        }
        for key, value in record.__dict__.items():
            if key not in _STANDARD_ATTRS and key not in payload:
                payload[key] = value
        if record.exc_info:
            payload["exception"] = redact_text(self.formatException(record.exc_info))
        return json.dumps(payload, default=str, sort_keys=True)


def configure_logging(level: str = "INFO", *, json_output: bool = True) -> None:
    """Installs one stream handler on the root logger (idempotent)."""
    handler = logging.StreamHandler(sys.stderr)
    handler.addFilter(RedactingFilter())
    handler.setFormatter(
        JsonFormatter()
        if json_output
        else logging.Formatter("%(asctime)s %(levelname)s %(name)s %(message)s")
    )
    root = logging.getLogger()
    for existing in list(root.handlers):
        if getattr(existing, "_synapse", False):
            root.removeHandler(existing)
    handler._synapse = True  # type: ignore[attr-defined]  # noqa: SLF001
    root.addHandler(handler)
    root.setLevel(level.upper())
