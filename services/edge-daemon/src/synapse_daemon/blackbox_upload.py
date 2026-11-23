"""Upload black box records to the Core API, with a restart-safe retry (E6.2).

Records are stored locally first (E6.1); this uploads the ones that are still pending to
`POST /v1/blackbox` with the daemon's service token. A record is marked uploaded only after the
API accepts it, so a crash or a network outage leaves it pending and `flush_pending` retries it
on the next call, including after a restart (the SQLite store keeps the pending rows). The API
upload is idempotent on `(cell, device, fault instant)`, so retrying a record the API already
has is harmless.
"""

from __future__ import annotations

import logging
from typing import Any, Protocol

from synapse_daemon.blackbox_store import BlackBoxStore

log = logging.getLogger(__name__)


class HttpResponse(Protocol):
    status_code: int

    def json(self) -> dict[str, Any]: ...


class HttpClient(Protocol):
    """A minimal async HTTP client (the subset of `httpx.AsyncClient` the daemon uses)."""

    async def post(
        self, url: str, json: dict[str, Any], headers: dict[str, str]
    ) -> HttpResponse: ...


async def upload_record(
    http: HttpClient, api_base_url: str, *, token: str, record_json: dict[str, Any]
) -> bool:
    """POST one record. Returns True when the API accepted it (2xx), False otherwise.

    Never raises on an HTTP error status or a transport failure: an upload failure just leaves
    the record pending for the next retry. The token is sent as a bearer header, never logged.
    """
    url = api_base_url.rstrip("/") + "/v1/blackbox"
    headers = {"Authorization": f"Bearer {token}"}
    try:
        resp = await http.post(url, json=record_json, headers=headers)
    except Exception as exc:  # noqa: BLE001 - a transport failure is a retry, not a crash
        log.warning("black box upload failed: %s", type(exc).__name__)
        return False
    if 200 <= resp.status_code < 300:
        return True
    log.warning("black box upload rejected: HTTP %s", resp.status_code)
    return False


async def flush_pending(
    http: HttpClient,
    api_base_url: str,
    *,
    token: str,
    store: BlackBoxStore,
    limit: int = 100,
) -> int:
    """Upload every pending record, oldest fault first. Returns how many were uploaded.

    Stops at the first failure so order is kept and the API is not hammered while it is down;
    the remaining records stay pending and are retried on the next call. On a daemon restart the
    store still holds the unuploaded rows, so this resumes where it left off.
    """
    uploaded = 0
    for row in store.pending(limit=limit):
        ok = await upload_record(
            http, api_base_url, token=token, record_json=row.record.model_dump(mode="json")
        )
        if not ok:
            break
        store.mark_uploaded(row.id)
        uploaded += 1
    return uploaded
