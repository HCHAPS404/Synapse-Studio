"""Checks of the names and sizes a job brings (`NE1101`). Nothing here touches the disk."""

import re
from dataclasses import dataclass
from pathlib import PurePosixPath

from synapse_core.errors import ErrorCode, ServiceError

_NAME = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.\-]{0,63}$")
_COMPONENT = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.\- ]{0,99}$")
_WINDOWS_DRIVE = re.compile(r"^[A-Za-z]:")


@dataclass(frozen=True)
class InputLimits:
    """Limits of the files of one job."""

    max_files: int = 200
    max_file_bytes: int = 200 * 1024
    max_total_bytes: int = 2 * 1024 * 1024
    max_path_chars: int = 200


def reject(message: str) -> ServiceError:
    """The `NE1101` error."""
    return ServiceError(ErrorCode.JOB_REJECTED, message)


def check_relative_path(path: str, limits: InputLimits) -> str:
    """Returns the normalised relative POSIX path, or raises `NE1101`.

    Rejected: empty, absolute, drive letters, backslashes, `..` or `.` components, hidden
    names, control characters, odd characters, and paths longer than the limit.
    """
    if not path or len(path) > limits.max_path_chars:
        raise reject("a file path is empty or too long")
    if path.startswith("/") or _WINDOWS_DRIVE.match(path):
        raise reject("file paths must be relative")
    if "\\" in path or "\x00" in path or any(ord(c) < 32 or ord(c) == 127 for c in path):
        raise reject("a file path holds a backslash or a control character")
    parts = PurePosixPath(path).parts
    if parts != tuple(path.split("/")):
        raise reject("a file path is not in normal form")
    for part in parts:
        if part in (".", "..") or part.startswith(".") or not _COMPONENT.match(part):
            raise reject("a file path has a component that is not allowed")
        if part != part.rstrip(" "):
            raise reject("a file path component ends with a space")
    return path


def check_files(files: dict[str, str], limits: InputLimits) -> dict[str, bytes]:
    """Validates every path and size; returns path to UTF-8 bytes."""
    if not files:
        raise reject("a job needs at least one file")
    if len(files) > limits.max_files:
        raise reject(f"too many files (limit {limits.max_files})")
    seen: set[str] = set()
    total = 0
    out: dict[str, bytes] = {}
    for raw, text in files.items():
        path = check_relative_path(raw, limits)
        folded = path.casefold()
        if folded in seen:
            raise reject("two file paths differ only in letter case")
        seen.add(folded)
        data = text.encode("utf-8")
        if len(data) > limits.max_file_bytes:
            raise reject(f"a file is larger than {limits.max_file_bytes} bytes")
        total += len(data)
        if total > limits.max_total_bytes:
            raise reject(f"the files are larger than {limits.max_total_bytes} bytes in total")
        out[path] = data
    # A file may not also be a directory of another path.
    for path in out:
        parts = path.split("/")
        for i in range(1, len(parts)):
            if "/".join(parts[:i]) in out:
                raise reject("a file path is also used as a directory")
    return out


def check_name(value: str, what: str) -> str:
    """A bare name (board, language, profile): no slash, no leading dash."""
    if not _NAME.match(value):
        raise reject(f"`{what}` is not a valid name")
    return value
