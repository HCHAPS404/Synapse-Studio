"""Black box upload route `POST /v1/blackbox` (E6.2).

A daemon (service token) uploads a fault record; the upload is idempotent so a daemon that
restarts and retries does not duplicate the record. A principal without `deployment.report`
(an engineer, a guest) is `NE1106`. The body is validated at the trust boundary.
"""

from __future__ import annotations

from typing import Any

from conftest import needs_db
from httpx import AsyncClient

pytestmark = needs_db


def _record(*, device: str = "arm", at_ns: int = 10_000_000_000) -> dict[str, Any]:
    return {
        "record_version": "1",
        "cell": "bottling",
        "device": device,
        "fault_kind": "fault",
        "fault_code": "overspeed",
        "at_ns": at_ns,
        "pre_ns": 10_000_000_000,
        "post_ns": 5_000_000_000,
        "daemon_version": "0.1.0",
        "package_hash": "",
        "samples": [
            {"tag": "speed", "value": 120, "quality": "good", "t_ns": at_ns - 1},
            {"tag": "lost", "value": None, "quality": "bad", "t_ns": at_ns},
        ],
        "events": [
            {"t_ns": at_ns, "device": device, "kind": "fault", "name": "overspeed", "value": "true"}
        ],
    }


async def _daemon_token(engineer_client: AsyncClient, cell: str = "bottling") -> str:
    issued = await engineer_client.post("/v1/pairing", json={"cell_id": cell})
    assert issued.status_code == 201, issued.text
    code = issued.json()["code"]
    claim = await engineer_client.post(
        "/v1/pairing/claim",
        json={"code": code, "daemon_name": "lab-daemon", "devices": [{"name": "arm"}]},
        headers={"Authorization": ""},
    )
    assert claim.status_code == 201, claim.text
    return claim.json()["token"]


async def test_daemon_uploads_a_record(engineer_client: AsyncClient) -> None:
    token = await _daemon_token(engineer_client)
    resp = await engineer_client.post(
        "/v1/blackbox", json=_record(), headers={"Authorization": f"Bearer {token}"}
    )
    assert resp.status_code == 201, resp.text
    body = resp.json()
    assert body["cell_id"] == "bottling"
    assert "id" in body and "created_at" in body


async def test_upload_is_idempotent_on_retry(engineer_client: AsyncClient) -> None:
    token = await _daemon_token(engineer_client)
    headers = {"Authorization": f"Bearer {token}"}
    first = await engineer_client.post("/v1/blackbox", json=_record(), headers=headers)
    second = await engineer_client.post("/v1/blackbox", json=_record(), headers=headers)
    assert first.status_code == 201 and second.status_code == 201
    # A retry of the same fault returns the same id (no duplicate row).
    assert first.json()["id"] == second.json()["id"]
    # A different fault instant is a new record.
    third = await engineer_client.post(
        "/v1/blackbox", json=_record(at_ns=20_000_000_000), headers=headers
    )
    assert third.json()["id"] != first.json()["id"]


async def test_record_is_bound_to_the_daemon_cell(engineer_client: AsyncClient) -> None:
    token = await _daemon_token(engineer_client, cell="bottling")
    # The body claims another cell; the daemon token's cell wins.
    body = _record()
    body["cell"] = "someone-elses-cell"
    resp = await engineer_client.post(
        "/v1/blackbox", json=body, headers={"Authorization": f"Bearer {token}"}
    )
    assert resp.status_code == 201, resp.text
    assert resp.json()["cell_id"] == "bottling"


async def test_engineer_cannot_upload(engineer_client: AsyncClient) -> None:
    # The engineer_client's own bearer is a user JWT, which lacks deployment.report.
    resp = await engineer_client.post("/v1/blackbox", json=_record())
    assert resp.status_code == 403
    assert resp.json()["code"] == "NE1106"


async def test_unknown_field_is_rejected(engineer_client: AsyncClient) -> None:
    token = await _daemon_token(engineer_client)
    body = _record()
    body["sneaky"] = 1
    resp = await engineer_client.post(
        "/v1/blackbox", json=body, headers={"Authorization": f"Bearer {token}"}
    )
    assert resp.status_code == 422 or resp.json().get("code") == "NE1101"
