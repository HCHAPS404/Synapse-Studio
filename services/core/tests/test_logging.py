import io
import json
import logging

from hypothesis import given
from hypothesis import strategies as st

from synapse_core.logging_config import (
    REDACTED,
    JsonFormatter,
    RedactingFilter,
    redact_text,
    request_id_var,
)

# The prefix keeps a generated secret from being a substring of the text around it.
_SAFE = st.text(alphabet="abcdefghijklmnopqrstuvwxyz0123456789", min_size=8, max_size=24).map(
    lambda s: "Zq9" + s
)


@given(user=_SAFE, password=_SAFE)
def test_url_credentials_are_removed(user: str, password: str) -> None:
    text = f"connecting to postgresql://{user}:{password}@db.example.com:5432/app now"
    out = redact_text(text)
    assert password not in out
    assert "db.example.com" in out


@given(value=_SAFE)
def test_sensitive_key_values_are_removed(value: str) -> None:
    for key in ("password", "token", "api_key", "SYNAPSE_JWT_SECRET"):
        assert value not in redact_text(f"{key}={value}")
        assert value not in redact_text(f'{{"{key}": "{value}"}}')


def test_bearer_and_jwt_are_removed() -> None:
    jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl"
    assert jwt not in redact_text(f"got {jwt} here")
    assert "abc.def" not in redact_text("Authorization: Bearer abc.def")


def test_plain_text_is_unchanged() -> None:
    assert redact_text("build finished in 12 ms") == "build finished in 12 ms"


def _capture() -> tuple[logging.Logger, io.StringIO]:
    stream = io.StringIO()
    handler = logging.StreamHandler(stream)
    handler.addFilter(RedactingFilter())
    handler.setFormatter(JsonFormatter())
    logger = logging.getLogger("test.logging.capture")
    logger.handlers = [handler]
    logger.propagate = False
    logger.setLevel(logging.INFO)
    return logger, stream


def test_json_record_has_request_id_and_redacted_extras() -> None:
    logger, stream = _capture()
    token = request_id_var.set("req-12345678")
    try:
        logger.info(
            "login %s", "password=hunter22hunter", extra={"auth_token": "xyz", "path": "/v1"}
        )
    finally:
        request_id_var.reset(token)
    record = json.loads(stream.getvalue())
    assert record["request_id"] == "req-12345678"
    assert record["auth_token"] == REDACTED
    assert record["path"] == "/v1"
    assert "hunter22hunter" not in stream.getvalue()


def test_exception_text_is_redacted() -> None:
    logger, stream = _capture()
    try:
        raise RuntimeError("cannot reach postgresql://u:s3cretpass@h/db")
    except RuntimeError:
        logger.exception("failed")
    assert "s3cretpass" not in stream.getvalue()
