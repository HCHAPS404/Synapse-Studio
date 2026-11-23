"""Load and validate relay frames against `spec/schemas/relay/message.schema.json` (E4.1).

The schema is the single source of truth, so the server works on the validated dict through
small typed accessors and does not duplicate the Rust models. `message.schema.json` is a
`oneOf` over every message body plus a required `v`; `validate_frame` first checks the frame
decodes to a JSON object and then validates it against the schema. The schema directory is
found relative to this package (so the working directory does not matter); the E2.10 Docker
image copies `spec/schemas/` into the image, as the design states.
"""

from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator
from jsonschema.exceptions import ValidationError

# …/services/core/src/synapse_core/relay/validate.py -> the repository root is six up.
_REPO_ROOT = Path(__file__).resolve().parents[5]
_DEFAULT_SCHEMA_DIR = _REPO_ROOT / "spec" / "schemas" / "relay"


class FrameError(Exception):
    """A frame is not a valid protocol message (maps to the `bad_message` relay code)."""


class VersionError(Exception):
    """A frame carries a protocol version other than 1 (maps to `unsupported_version`)."""


def schema_dir() -> Path:
    """The directory holding `message.schema.json` (overridable in tests by monkeypatch)."""
    return _DEFAULT_SCHEMA_DIR


@lru_cache(maxsize=1)
def _validator() -> Draft202012Validator:
    """A cached validator built from the packaged message schema."""
    path = schema_dir() / "message.schema.json"
    schema = json.loads(path.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema)


def decode_frame(text: str) -> dict[str, Any]:
    """Parses a text frame into a JSON object, or raises `FrameError`.

    Binary frames and non-object JSON (array, number, string) are rejected: the protocol
    requires one JSON object per text frame.
    """
    try:
        value = json.loads(text)
    except (json.JSONDecodeError, TypeError) as exc:
        raise FrameError("frame is not valid JSON") from exc
    if not isinstance(value, dict):
        raise FrameError("frame is not a JSON object")
    return value


def validate_message(frame: dict[str, Any]) -> None:
    """Validates a decoded frame against the message schema.

    Raises `VersionError` when `v` is present but not 1 (so the caller can answer
    `unsupported_version`), and `FrameError` for any other schema failure (`bad_message`).
    """
    version = frame.get("v")
    if version is not None and version != 1:
        raise VersionError(f"unsupported protocol version {version!r}")
    try:
        _validator().validate(frame)
    except ValidationError as exc:
        # A wrong/missing `v` that slips through as a schema error is still a version issue.
        if "v" in exc.absolute_path or "ProtocolVersion" in str(exc.schema_path):
            raise VersionError("unsupported protocol version") from exc
        raise FrameError(_short_reason(exc)) from exc


def validate_frame(text: str) -> dict[str, Any]:
    """Decodes and validates one text frame, returning the message dict."""
    frame = decode_frame(text)
    validate_message(frame)
    return frame


def message_type(frame: dict[str, Any]) -> str:
    """The `type` tag of a validated frame."""
    value = frame.get("type")
    if not isinstance(value, str):  # pragma: no cover - the schema requires a string type
        raise FrameError("missing message type")
    return value


def _short_reason(exc: ValidationError) -> str:
    """A compact, user-safe reason string (never echoing values that might be secrets)."""
    location = ".".join(str(p) for p in exc.absolute_path) or "<root>"
    return f"invalid message at {location}: {exc.validator} constraint"
