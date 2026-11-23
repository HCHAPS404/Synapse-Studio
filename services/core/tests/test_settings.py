"""Per-environment settings behaviour (E7.2).

The deployed environments (`production`, `judging`) must supply a real `SYNAPSE_JWT_SECRET`;
`development` (local, CI) derives a stable per-process key so runs and tests need no secret.
"""

import pytest
from pydantic import SecretStr

from synapse_core.settings import Settings


def _settings(**kwargs: object) -> Settings:
    # Build settings directly so the process environment and .env file never leak in.
    return Settings(_env_file=None, **kwargs)  # type: ignore[call-arg]


def test_development_derives_a_key_without_a_secret() -> None:
    settings = _settings(environment="development")
    assert settings.is_deployed is False
    # No SYNAPSE_JWT_SECRET: a stable per-process key is derived, so local/CI runs work.
    assert len(settings.token_secret()) == 32


@pytest.mark.parametrize("environment", ["production", "judging"])
def test_deployed_requires_a_secret(environment: str) -> None:
    settings = _settings(environment=environment)
    assert settings.is_deployed is True
    with pytest.raises(ValueError, match="SYNAPSE_JWT_SECRET is required"):
        settings.token_secret()


@pytest.mark.parametrize("environment", ["production", "judging"])
def test_deployed_rejects_a_short_secret(environment: str) -> None:
    settings = _settings(environment=environment, jwt_secret=SecretStr("too-short"))
    with pytest.raises(ValueError, match="at least 32 bytes"):
        settings.token_secret()


@pytest.mark.parametrize("environment", ["production", "judging"])
def test_deployed_accepts_a_long_secret(environment: str) -> None:
    secret = "x" * 32
    settings = _settings(environment=environment, jwt_secret=SecretStr(secret))
    assert settings.token_secret() == secret.encode("utf-8")
