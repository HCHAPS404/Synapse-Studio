"""Shared fixtures for the daemon tests."""

from __future__ import annotations

from typing import Any

import pytest


@pytest.fixture
def welcome_frame() -> dict[str, Any]:
    return {
        "v": 1,
        "type": "welcome",
        "session": "s-test",
        "server_time_ns": 1,
        "roles": ["daemon"],
        "protocol": 1,
    }
