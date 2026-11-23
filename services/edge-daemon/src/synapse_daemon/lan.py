"""Local LAN mode: the daemon works with no cloud.

When no cloud relay is configured the daemon runs on the LAN only. `run_mode` keeps the mode
decision in one place; `LanServer` serves the built PWA from a folder and a small `/health`
endpoint over plain asyncio (no web framework, no cloud, no network beyond the LAN), so a
judge or operator can reach the UI with nothing but the daemon running.
"""

from __future__ import annotations

import asyncio
import json
import mimetypes
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import unquote, urlsplit

from synapse_daemon.config import DaemonSettings


@dataclass(frozen=True, slots=True)
class RunMode:
    """Whether the daemon relays to the cloud or serves the LAN only."""

    local: bool
    relay_url: str | None
    lan_host: str
    lan_port: int

    @property
    def description(self) -> str:
        if self.local:
            return f"local LAN mode on {self.lan_host}:{self.lan_port} (no cloud relay)"
        return f"cloud relay to {self.relay_url} plus LAN on {self.lan_host}:{self.lan_port}"


def run_mode(settings: DaemonSettings) -> RunMode:
    """Decide the run mode from the settings."""
    return RunMode(
        local=settings.is_local_mode(),
        relay_url=settings.relay_url,
        lan_host=settings.lan_host,
        lan_port=settings.lan_port,
    )


def safe_join(root: Path, url_path: str) -> Path | None:
    """Resolve a URL path under `root`, rejecting traversal. Returns None when unsafe."""
    raw = unquote(url_path).lstrip("/")
    if raw in ("", "."):
        raw = "index.html"
    # Reject traversal before normalising, so a leading "/" cannot collapse "..".
    parts = [p for p in raw.replace("\\", "/").split("/") if p not in ("", ".")]
    if any(p == ".." for p in parts):
        return None
    candidate = root.joinpath(*parts)
    try:
        candidate.relative_to(root)
    except ValueError:
        return None
    return candidate


class LanServer:
    """A minimal static-file HTTP server for the PWA plus `/health`, over asyncio.

    It serves `pwa_dir` on the LAN and reports the run mode on `/health`. It is deliberately
    tiny (no framework) so the daemon can serve the UI offline. SPA routing: a path with no
    file falls back to `index.html` when that exists.
    """

    def __init__(self, pwa_dir: str | Path | None, mode: RunMode) -> None:
        self._root = Path(pwa_dir).resolve() if pwa_dir else None
        self._mode = mode
        self._server: asyncio.AbstractServer | None = None
        self.port = 0

    async def start(self, host: str, port: int) -> None:
        self._server = await asyncio.start_server(self._handle, host, port)
        self.port = self._server.sockets[0].getsockname()[1]

    async def stop(self) -> None:
        if self._server is not None:
            self._server.close()
            try:
                await self._server.wait_closed()
            except Exception:  # noqa: S110 - already closed
                pass
            self._server = None

    async def _handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        try:
            request_line = await reader.readline()
            while True:  # drain the rest of the headers
                line = await reader.readline()
                if line in (b"\r\n", b"", b"\n"):
                    break
            if not request_line:
                return
            parts = request_line.decode("latin-1").split()
            if len(parts) < 2 or parts[0] != "GET":
                await self._respond(writer, 405, b"method not allowed", "text/plain")
                return
            path = urlsplit(parts[1]).path
            status, body, ctype = self._route(path)
            await self._respond(writer, status, body, ctype)
        finally:
            writer.close()

    def _route(self, path: str) -> tuple[int, bytes, str]:
        if path == "/health":
            payload = {
                "status": "ok",
                "mode": "local" if self._mode.local else "cloud",
                "offline": self._mode.local,
            }
            return 200, json.dumps(payload).encode(), "application/json"
        if self._root is None:
            return 404, b"no PWA directory configured", "text/plain"
        target = safe_join(self._root, path)
        if target is None:
            return 403, b"forbidden", "text/plain"
        if not target.is_file():
            index = self._root / "index.html"  # SPA fallback
            if index.is_file():
                target = index
            else:
                return 404, b"not found", "text/plain"
        ctype = mimetypes.guess_type(str(target))[0] or "application/octet-stream"
        return 200, target.read_bytes(), ctype

    async def _respond(
        self, writer: asyncio.StreamWriter, status: int, body: bytes, ctype: str
    ) -> None:
        reason = {200: "OK", 403: "Forbidden", 404: "Not Found", 405: "Method Not Allowed"}.get(
            status, "OK"
        )
        headers = (
            f"HTTP/1.1 {status} {reason}\r\n"
            f"Content-Type: {ctype}\r\n"
            f"Content-Length: {len(body)}\r\n"
            "Cache-Control: no-cache\r\n"
            "Connection: close\r\n\r\n"
        ).encode("latin-1")
        writer.write(headers + body)
        try:
            await writer.drain()
        except Exception:  # noqa: S110 - client may have gone
            pass
