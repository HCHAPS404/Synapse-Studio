from pathlib import Path

import pytest
from fastapi import FastAPI, HTTPException
from fastapi.testclient import TestClient
from pydantic import BaseModel

from synapse_core.errors import DEFAULT_STATUS, ErrorCode, ServiceError, install_error_handlers
from synapse_core.main import create_app
from synapse_core.middleware import RequestIdMiddleware
from synapse_core.schemas.errors import ErrorBody


class _Item(BaseModel):
    n: int


def _app() -> FastAPI:
    app = FastAPI()
    install_error_handlers(app)
    app.add_middleware(RequestIdMiddleware)

    @app.get("/conflict")
    async def conflict() -> None:
        raise ServiceError(ErrorCode.VERSION_CONFLICT, "stale row_version")

    @app.get("/limited")
    async def limited() -> None:
        raise ServiceError(ErrorCode.RATE_LIMIT, "slow down", headers={"Retry-After": "3"})

    @app.get("/forbidden")
    async def forbidden() -> None:
        raise HTTPException(status_code=403, detail="no")

    @app.get("/boom")
    async def boom() -> None:
        raise RuntimeError("secret internals")

    @app.post("/item")
    async def item(body: _Item) -> _Item:
        return body

    return app


def _client() -> TestClient:
    return TestClient(_app(), raise_server_exceptions=False)


def test_service_error_body_and_request_id() -> None:
    response = _client().get("/conflict", headers={"X-Request-ID": "abcdefgh-1234"})
    assert response.status_code == 409
    body = ErrorBody.model_validate(response.json())
    assert body.code == "NE1107"
    assert body.request_id == "abcdefgh-1234"
    assert response.headers["X-Request-ID"] == "abcdefgh-1234"


def test_bad_request_id_is_replaced() -> None:
    response = _client().get("/conflict", headers={"X-Request-ID": "bad id\t!"})
    assert response.json()["request_id"] != "bad id\t!"
    assert len(response.json()["request_id"]) == 32


def test_retry_after_header() -> None:
    response = _client().get("/limited")
    assert response.status_code == 429
    assert response.headers["Retry-After"] == "3"
    assert response.json()["code"] == "NE1103"


def test_http_exceptions_map_to_codes() -> None:
    client = _client()
    assert client.get("/forbidden").json()["code"] == "NE1106"
    missing = client.get("/nothing")
    assert missing.status_code == 404
    assert missing.json()["code"] == "HTTP_404"


def test_validation_error_does_not_echo_input() -> None:
    response = _client().post("/item", json={"n": "hunter2-secret"})
    assert response.status_code == 422
    assert response.json()["code"] == "NE1101"
    assert "hunter2-secret" not in response.text


def test_unhandled_error_is_generic() -> None:
    response = _client().get("/boom")
    assert response.status_code == 500
    assert response.json()["code"] == "HTTP_500"
    assert "secret internals" not in response.text


def test_every_code_has_a_status_and_is_documented() -> None:
    root = Path(__file__).resolve().parents[3]
    docs = (root / "docs/errors/services.md").read_text(encoding="utf-8")
    catalogue = (root / "spec/language/10-diagnostics.md").read_text(encoding="utf-8")
    assert [c.value for c in ErrorCode] == [f"NE11{n:02d}" for n in range(1, 13)]
    for code in ErrorCode:
        assert code in DEFAULT_STATUS
        assert f"### {code.value} " in docs
        assert f"| {code.value} |" in catalogue


def test_app_error_bodies_match_in_the_real_app() -> None:
    response = TestClient(create_app()).get("/nope")
    assert response.status_code == 404
    assert ErrorBody.model_validate(response.json()).code == "HTTP_404"


@pytest.mark.parametrize("status", [401, 403, 409, 429])
def test_http_status_codes_with_catalogue_entries(status: int) -> None:
    app = FastAPI()
    install_error_handlers(app)

    @app.get("/x")
    async def x() -> None:
        raise HTTPException(status_code=status)

    code = TestClient(app).get("/x").json()["code"]
    assert code.startswith("NE11")
