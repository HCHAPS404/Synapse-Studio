"""E5.11: the LAN PWA server and offline operation."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from pathlib import Path

import pytest

from synapse_daemon.config import DaemonSettings
from synapse_daemon.lan import LanServer, run_mode, safe_join


def test_safe_join_rejects_traversal(tmp_path: Path) -> None:
    assert safe_join(tmp_path, "/../../etc/passwd") is None
    assert safe_join(tmp_path, "/app.js") == tmp_path / "app.js"
    assert safe_join(tmp_path, "/") == tmp_path / "index.html"


async def _get(host: str, port: int, path: str) -> tuple[int, bytes, dict[str, str]]:
    reader, writer = await asyncio.open_connection(host, port)
    writer.write(f"GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n".encode())
    await writer.drain()
    raw = await reader.read()
    writer.close()
    head, _, body = raw.partition(b"\r\n\r\n")
    lines = head.decode("latin-1").split("\r\n")
    status = int(lines[0].split()[1])
    headers = dict(h.split(": ", 1) for h in lines[1:] if ": " in h)
    return status, body, headers


@pytest.fixture
async def pwa(tmp_path: Path) -> Path:
    (tmp_path / "index.html").write_text("<html>SYNAPSE</html>")
    (tmp_path / "app.js").write_text("console.log('hi')")
    return tmp_path


@pytest.fixture
async def server(pwa: Path) -> AsyncIterator[LanServer]:
    mode = run_mode(DaemonSettings())  # no relay/token -> local (offline) mode
    srv = LanServer(pwa, mode)
    await srv.start("127.0.0.1", 0)
    try:
        yield srv
    finally:
        await srv.stop()


async def test_serves_index(server: LanServer) -> None:
    status, body, headers = await _get("127.0.0.1", server.port, "/")
    assert status == 200 and b"SYNAPSE" in body
    assert headers["Content-Type"].startswith("text/html")


async def test_serves_asset(server: LanServer) -> None:
    status, body, _ = await _get("127.0.0.1", server.port, "/app.js")
    assert status == 200 and b"console.log" in body


async def test_spa_fallback_to_index(server: LanServer) -> None:
    status, body, _ = await _get("127.0.0.1", server.port, "/some/route")
    assert status == 200 and b"SYNAPSE" in body  # unknown route falls back to index.html


async def test_health_reports_offline_mode(server: LanServer) -> None:
    import json

    status, body, headers = await _get("127.0.0.1", server.port, "/health")
    assert status == 200 and headers["Content-Type"] == "application/json"
    data = json.loads(body)
    assert data["status"] == "ok" and data["mode"] == "local" and data["offline"] is True


async def test_traversal_is_forbidden(server: LanServer) -> None:
    status, _, _ = await _get("127.0.0.1", server.port, "/../../secret")
    assert status in (403, 404)  # normalised and refused; never serves outside the root


async def test_post_is_rejected(server: LanServer) -> None:
    reader, writer = await asyncio.open_connection("127.0.0.1", server.port)
    writer.write(b"POST / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
    await writer.drain()
    raw = await reader.read()
    writer.close()
    assert b"405" in raw.split(b"\r\n")[0]


async def test_health_without_pwa_dir() -> None:
    mode = run_mode(DaemonSettings())
    srv = LanServer(None, mode)  # no PWA configured
    await srv.start("127.0.0.1", 0)
    try:
        status, _, _ = await _get("127.0.0.1", srv.port, "/health")
        assert status == 200  # health still works offline
        status, _, _ = await _get("127.0.0.1", srv.port, "/index.html")
        assert status == 404
    finally:
        await srv.stop()
