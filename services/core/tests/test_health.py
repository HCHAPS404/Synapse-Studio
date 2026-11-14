from fastapi.testclient import TestClient
from pydantic import SecretStr

from synapse_core.main import create_app
from synapse_core.settings import Settings, get_settings


def _client(settings: Settings) -> TestClient:
    app = create_app()
    app.dependency_overrides[get_settings] = lambda: settings
    return TestClient(app)


def test_health_without_database_is_ok() -> None:
    response = _client(Settings(database_url=None)).get("/health")
    assert response.status_code == 200
    body = response.json()
    assert body["status"] == "ok"
    assert body["database"]["status"] == "unconfigured"


def test_health_with_unreachable_database_is_degraded_and_hides_url() -> None:
    url = "postgresql://user:supersecret@127.0.0.1:1/db"
    response = _client(Settings(database_url=SecretStr(url), database_timeout_s=1)).get("/health")
    assert response.status_code == 200
    body = response.json()
    assert body["status"] == "degraded"
    assert body["database"]["status"] == "error"
    assert "supersecret" not in response.text
