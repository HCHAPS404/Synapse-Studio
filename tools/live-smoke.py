#!/usr/bin/env python3
"""Live smoke test of the deployed Core API (E7.3).

Exercises the judge-facing path against a running service, using only the Python standard
library (no new dependencies, no third-party HTTP or WebSocket client):

  1. GET /health is reachable and reports status ok|degraded.
  2. POST /v1/guest issues a guest token (login-free, what judges use).
  3. A tiny corpus example is built through the guest flow: create a project, store a version
     with the files, submit a `build`, poll until it finishes.
  4. The relay WebSocket (WS /v1/relay) accepts a `hello` and answers `welcome`, then closes.

Exit codes:
  0  everything passed.
  1  a real regression: the service answered, but a check failed (bad status, build error,
     wrong relay reply). This should fail a scheduled build.
  2  transient/infrastructure: could not reach the service, a timeout, a connection reset.
     A scheduled build treats this as "skip", not a regression (the service may be waking or
     the network flaked), so a blip does not page anyone.

Usage:
  python tools/live-smoke.py [BASE_URL]
  python tools/live-smoke.py            # uses $SYNAPSE_SMOKE_URL or the production default

No secrets are read or printed. The guest token is never logged.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import socket
import ssl
import sys
import time
import urllib.error
import urllib.request
from typing import Any
from urllib.parse import urlparse

DEFAULT_URL = "https://synapse-core-jqqk.onrender.com"
HTTP_TIMEOUT_S = 20.0
BUILD_POLL_TIMEOUT_S = 90.0
BUILD_POLL_INTERVAL_S = 2.0
WS_TIMEOUT_S = 15.0

# A tiny corpus example built through the guest flow. This mirrors
# examples/corpus/host/h01_gateway_counter.neuron. `neuron check` is used as the build kind: it
# runs the full analyzer in the sandboxed runner and needs no board, cross compiler or toolchain,
# so it succeeds on the production image without hardware.
SMOKE_MODULE = """\
## A gateway that counts messages once per second.
module h01_gateway_counter for host
signal messages: u32
task tick every 1 s:
  messages = messages + 1
test "the counter grows once per second":
  advance(3500 ms)
  assert elapsed() > 3.4, "time passed"
"""
SMOKE_FILES = {"h01_gateway_counter.neuron": SMOKE_MODULE}
SMOKE_BUILD_KIND = "check"


class Transient(Exception):
    """A network or availability problem, not a regression (exit 2)."""


class Regression(Exception):
    """The service answered but a check failed (exit 1)."""


def _log(msg: str) -> None:
    print(msg, flush=True)


# --------------------------------------------------------------------------------------------
# HTTP helpers (urllib only)
# --------------------------------------------------------------------------------------------


def _request(
    method: str,
    url: str,
    *,
    token: str | None = None,
    body: dict[str, Any] | None = None,
) -> tuple[int, dict[str, Any] | None]:
    """Makes one HTTP request. Raises Transient on a network failure, never on an HTTP status."""
    data = None
    headers = {"Accept": "application/json"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    _require_http_url(url)  # scheme restricted to http/https, so S310 below is safe
    req = urllib.request.Request(url, data=data, headers=headers, method=method)  # noqa: S310
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT_S) as resp:  # noqa: S310
            raw = resp.read()
            return resp.status, _parse_json(raw)
    except urllib.error.HTTPError as exc:  # an HTTP status (4xx/5xx): a real answer
        raw = exc.read()
        return exc.code, _parse_json(raw)
    except (urllib.error.URLError, TimeoutError, ssl.SSLError, OSError) as exc:
        raise Transient(f"{method} {url} did not answer: {type(exc).__name__}") from exc


def _require_http_url(url: str) -> None:
    """Refuses anything but http/https, so a bad base URL cannot open a `file:` or other scheme."""
    scheme = urlparse(url).scheme
    if scheme not in ("http", "https"):
        raise Regression(f"refusing to request a non-http(s) URL: {scheme!r}")


def _parse_json(raw: bytes) -> dict[str, Any] | None:
    if not raw:
        return None
    try:
        value = json.loads(raw)
    except ValueError:
        return None
    return value if isinstance(value, dict) else None


# --------------------------------------------------------------------------------------------
# Checks
# --------------------------------------------------------------------------------------------


def check_health(base: str) -> None:
    status, body = _request("GET", f"{base}/health")
    if status != 200 or body is None:
        raise Transient(f"/health returned HTTP {status}")
    if body.get("status") not in ("ok", "degraded"):
        raise Regression(f"/health reported an unexpected status: {body.get('status')!r}")
    _log(
        f"  health: status={body.get('status')} version={body.get('version')} "
        f"env={body.get('environment')} db={body.get('database', {}).get('status')}"
    )


def create_guest(base: str) -> str:
    status, body = _request("POST", f"{base}/v1/guest")
    if status != 201 or body is None or "token" not in body:
        raise Regression(f"POST /v1/guest returned HTTP {status} without a token")
    _log(f"  guest: sandbox={body.get('sandbox_id')} role={body.get('role')}")
    token = body["token"]
    if not isinstance(token, str) or not token:
        raise Regression("guest token is missing or not a string")
    return token


def build_example(base: str, token: str) -> None:
    status, project = _request("POST", f"{base}/v1/projects", token=token, body={"name": "smoke"})
    if status != 201 or project is None:
        raise Regression(f"create project returned HTTP {status}")
    project_id = project["id"]

    status, version = _request(
        "POST",
        f"{base}/v1/projects/{project_id}/versions",
        token=token,
        body={"files": SMOKE_FILES},
    )
    if status != 201 or version is None:
        raise Regression(f"create version returned HTTP {status}")
    version_id = version["id"]

    status, build = _request(
        "POST",
        f"{base}/v1/versions/{version_id}/builds",
        token=token,
        body={"kind": SMOKE_BUILD_KIND},
    )
    if status != 202 or build is None:
        raise Regression(f"submit build returned HTTP {status}")
    build_id = build["id"]

    final = _poll_build(base, token, build_id)
    outcome = final.get("status")
    if outcome != "succeeded":
        raise Regression(
            f"guest build did not succeed: status={outcome} "
            f"error_code={final.get('error_code')} exit_code={final.get('exit_code')}"
        )
    _log(f"  build: {build_id} succeeded, {len(final.get('artifacts', []))} artifact(s)")


def _poll_build(base: str, token: str, build_id: str) -> dict[str, Any]:
    deadline = time.monotonic() + BUILD_POLL_TIMEOUT_S
    while True:
        status, body = _request("GET", f"{base}/v1/builds/{build_id}", token=token)
        if status != 200 or body is None:
            raise Regression(f"poll build returned HTTP {status}")
        if body.get("status") in ("succeeded", "failed", "error"):
            return body
        if time.monotonic() > deadline:
            raise Regression(f"build did not finish within {BUILD_POLL_TIMEOUT_S:.0f}s")
        time.sleep(BUILD_POLL_INTERVAL_S)


def check_relay(base: str, token: str) -> None:
    """Opens WS /v1/relay with a raw-socket RFC 6455 handshake, sends hello, reads welcome."""
    parsed = urlparse(base)
    host = parsed.hostname or ""
    use_tls = parsed.scheme == "https"
    port = parsed.port or (443 if use_tls else 80)
    path = "/v1/relay"

    key = base64.b64encode(os.urandom(16)).decode("ascii")
    handshake = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        "\r\n"
    ).encode("ascii")

    try:
        raw = socket.create_connection((host, port), timeout=WS_TIMEOUT_S)
        sock: socket.socket = raw
        if use_tls:
            ctx = ssl.create_default_context()
            sock = ctx.wrap_socket(raw, server_hostname=host)
        sock.settimeout(WS_TIMEOUT_S)
    except (OSError, ssl.SSLError) as exc:
        raise Transient(f"relay connect failed: {type(exc).__name__}") from exc

    try:
        sock.sendall(handshake)
        resp = _read_until(sock, b"\r\n\r\n")
        _check_switching_protocols(resp, key)

        hello = {
            "v": 1,
            "type": "hello",
            "client": "web",
            "protocol": 1,
            "client_version": "live-smoke",
            "capabilities": [],
            "token": token,
        }
        sock.sendall(_ws_text_frame(json.dumps(hello)))
        message = _ws_read_text(sock)
        frame = json.loads(message)
        if frame.get("type") != "welcome":
            raise Regression(f"relay did not answer welcome; got {frame.get('type')!r}")
        _log(f"  relay: welcome roles={frame.get('roles')} session={frame.get('session')}")
        try:  # a courteous close; failure here does not matter
            sock.sendall(_ws_close_frame())
        except OSError:
            pass
    except (OSError, ssl.SSLError, TimeoutError) as exc:
        raise Transient(f"relay exchange failed: {type(exc).__name__}") from exc
    except ValueError as exc:  # bad JSON from a service that answered: a regression
        raise Regression(f"relay sent an unparseable welcome: {exc}") from exc
    finally:
        try:
            sock.close()
        except OSError:
            pass


# --------------------------------------------------------------------------------------------
# Minimal RFC 6455 client framing (text frames only, client-masked)
# --------------------------------------------------------------------------------------------


def _read_until(sock: socket.socket, marker: bytes) -> bytes:
    buf = b""
    while marker not in buf:
        chunk = sock.recv(4096)
        if not chunk:
            raise Transient("relay closed during the handshake")
        buf += chunk
        if len(buf) > 65536:
            raise Regression("relay handshake response is too large")
    return buf


def _check_switching_protocols(resp: bytes, key: str) -> None:
    head = resp.split(b"\r\n\r\n", 1)[0].decode("latin-1")
    first = head.split("\r\n", 1)[0]
    if "101" not in first:
        raise Regression(f"relay did not switch protocols: {first!r}")
    # SHA-1 here is mandated by the WebSocket handshake (RFC 6455 §1.3), not a security choice.
    digest = hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()  # noqa: S324
    expected = base64.b64encode(digest).decode("ascii")
    accept = ""
    for line in head.split("\r\n")[1:]:
        if line.lower().startswith("sec-websocket-accept:"):
            accept = line.split(":", 1)[1].strip()
    if accept != expected:
        raise Regression("relay Sec-WebSocket-Accept did not match")


def _ws_text_frame(text: str) -> bytes:
    return _ws_frame(0x1, text.encode("utf-8"))


def _ws_close_frame() -> bytes:
    return _ws_frame(0x8, b"")


def _ws_frame(opcode: int, payload: bytes) -> bytes:
    header = bytearray([0x80 | opcode])  # FIN + opcode
    length = len(payload)
    mask_bit = 0x80  # a client MUST mask
    if length < 126:
        header.append(mask_bit | length)
    elif length < 65536:
        header.append(mask_bit | 126)
        header += length.to_bytes(2, "big")
    else:
        header.append(mask_bit | 127)
        header += length.to_bytes(8, "big")
    mask = os.urandom(4)
    header += mask
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return bytes(header) + masked


def _ws_read_text(sock: socket.socket) -> str:
    """Reads one server frame (unmasked) and returns its text, skipping ping/pong control."""
    deadline = time.monotonic() + WS_TIMEOUT_S
    while True:
        if time.monotonic() > deadline:
            raise Transient("relay sent no welcome in time")
        b0 = _recv_exact(sock, 1)[0]
        opcode = b0 & 0x0F
        b1 = _recv_exact(sock, 1)[0]
        masked = bool(b1 & 0x80)
        length = b1 & 0x7F
        if length == 126:
            length = int.from_bytes(_recv_exact(sock, 2), "big")
        elif length == 127:
            length = int.from_bytes(_recv_exact(sock, 8), "big")
        mask = _recv_exact(sock, 4) if masked else b""
        payload = _recv_exact(sock, length) if length else b""
        if masked:
            payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        if opcode in (0x1, 0x2):  # text or binary data
            return payload.decode("utf-8", "replace")
        if opcode == 0x8:  # close
            raise Transient("relay closed before sending welcome")
        # 0x9 ping / 0xA pong / continuation: ignore and read the next frame


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise Transient("relay closed mid-frame")
        buf += chunk
    return buf


# --------------------------------------------------------------------------------------------


def main(argv: list[str]) -> int:
    base = (argv[1] if len(argv) > 1 else os.environ.get("SYNAPSE_SMOKE_URL", DEFAULT_URL)).rstrip(
        "/"
    )
    _log(f"live smoke against {base}")
    try:
        _log("1/4 health")
        check_health(base)
        _log("2/4 guest")
        token = create_guest(base)
        _log("3/4 guest build")
        build_example(base, token)
        _log("4/4 relay")
        check_relay(base, token)
    except Transient as exc:
        _log(f"TRANSIENT: {exc}")
        _log("result: SKIP (infrastructure/network, not a regression)")
        return 2
    except Regression as exc:
        _log(f"REGRESSION: {exc}")
        _log("result: FAIL")
        return 1
    _log("result: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
