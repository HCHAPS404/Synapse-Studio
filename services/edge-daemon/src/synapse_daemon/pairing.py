"""The daemon side of pairing (E4.3): claim a one-time code, receive a service token.

An engineer issues a code in the Core API (`POST /v1/pairing`, E4.3). The daemon calls
`POST /v1/pairing/claim` with the code and its name and receives a `daemon` service token
(E3.3) bound to a cell. The token is a secret: it is returned to the caller to store in the
environment or a file the operator controls, never logged.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Any, Protocol

log = logging.getLogger(__name__)


@dataclass(frozen=True, slots=True)
class PairingResult:
    """The outcome of a successful claim: the cell, the token (once) and the daemon id."""

    cell_id: str
    service_token: str
    daemon_id: str


class HttpResponse(Protocol):
    status_code: int

    def json(self) -> dict[str, Any]: ...


class HttpClient(Protocol):
    """A minimal async HTTP client (the subset of `httpx.AsyncClient` the daemon uses)."""

    async def post(self, url: str, json: dict[str, Any]) -> HttpResponse: ...


class PairingError(Exception):
    """The claim was refused (bad or expired code) or the API was unreachable."""

    def __init__(self, message: str, *, code: str | None = None) -> None:
        super().__init__(message)
        self.code = code


async def claim_pairing(
    http: HttpClient, api_base_url: str, *, code: str, name: str
) -> PairingResult:
    """Claim a pairing code and return the service token once.

    A bad, expired or already-claimed code is `NE1111` from the API; this raises
    `PairingError` carrying that code. The token is never written to the log.
    """
    url = api_base_url.rstrip("/") + "/v1/pairing/claim"
    resp = await http.post(url, json={"code": code, "name": name})
    body = resp.json()
    if resp.status_code >= 400:
        raise PairingError(body.get("message", "pairing claim failed"), code=body.get("code"))
    token = body.get("token") or body.get("service_token")
    cell_id = body.get("cell_id") or body.get("cell")
    daemon_id = body.get("daemon_id") or body.get("id")
    if not (token and cell_id and daemon_id):
        raise PairingError("pairing response missing token, cell or daemon id")
    log.info("paired with cell=%s daemon=%s", cell_id, daemon_id)  # token never logged
    return PairingResult(cell_id=str(cell_id), service_token=str(token), daemon_id=str(daemon_id))
