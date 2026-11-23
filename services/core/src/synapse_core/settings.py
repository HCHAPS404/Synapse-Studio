"""Runtime configuration loaded from environment variables (never hard-coded)."""

import secrets
from functools import lru_cache

from pydantic import Field, SecretStr
from pydantic_settings import BaseSettings, SettingsConfigDict

from synapse_core.limits import LimiterSettings

# A random key made once per process; development guest tokens do not outlive a restart.
_DEV_TOKEN_SECRET = secrets.token_bytes(32)


class Settings(BaseSettings):
    """Core service settings. Values come from env vars prefixed with SYNAPSE_."""

    model_config = SettingsConfigDict(env_prefix="SYNAPSE_", env_file=".env", extra="ignore")

    environment: str = Field(
        default="development", description="development | judging | production"
    )
    database_url: SecretStr | None = Field(
        default=None, description="Postgres URL (Neon). Optional in development."
    )
    database_timeout_s: float = 3.0
    db_pool_size: int = Field(default=5, ge=1, le=50)
    db_pool_recycle_s: int = Field(default=300, ge=30)
    cors_origins: list[str] = Field(default_factory=lambda: ["http://localhost:3000"])
    neuron_bin: str = Field(default="neuron", description="Path of the `neuron` executable")
    compile_timeout_s: float = Field(default=30.0, gt=0, le=600)
    compile_work_dir: str | None = Field(
        default=None, description="Parent of the per-job directories (default: system temp)"
    )
    compile_concurrency: int = Field(
        default=2, ge=1, le=16, description="Compile jobs that run at the same time"
    )
    compile_queue_max: int = Field(
        default=8, ge=0, le=1000, description="Jobs that may wait; more is NE1103"
    )
    devices_dir: str | None = Field(
        default=None,
        description="Directory of the device manifests; default: the repo `devices/`",
    )
    jwt_secret: SecretStr | None = Field(
        default=None,
        description="HMAC secret for guest tokens (E2.8) and JWTs (E3); required in production",
    )
    access_token_ttl_s: int = Field(
        default=15 * 60, ge=60, le=3600, description="User JWT access-token lifetime"
    )
    argon2_time_cost: int = Field(default=3, ge=1, description="Argon2id iterations")
    argon2_memory_cost_kib: int = Field(default=65536, ge=8, description="Argon2id memory in KiB")
    argon2_parallelism: int = Field(default=4, ge=1, description="Argon2id lanes")
    guest_ip_salt: SecretStr | None = Field(
        default=None, description="Salt for the one-way hash of a guest's IP"
    )
    guest_token_ttl_s: int = Field(
        default=24 * 3600, ge=60, description="Guest token and sandbox lifetime"
    )
    guest_max_projects: int = Field(default=5, ge=1, description="Projects per sandbox")
    guest_max_files: int = Field(default=50, ge=1, description="Files per version for a guest")
    guest_max_file_bytes: int = Field(
        default=200 * 1024, ge=1, description="Bytes per file for a guest"
    )
    guest_max_builds_per_day: int = Field(default=100, ge=1, description="Builds per day")
    guest_cleanup_interval_s: float = Field(
        default=600.0, gt=0, description="How often the expiry cleanup task runs"
    )
    telemetry_max_rows: int = Field(
        default=5_000_000, ge=1000, description="Raw telemetry rows cap; more are dropped"
    )
    telemetry_raw_retention_s: int = Field(
        default=7 * 24 * 3600, ge=60, description="How long raw samples are kept"
    )
    telemetry_1m_retention_s: int = Field(
        default=30 * 24 * 3600, ge=60, description="How long one-minute roll-ups are kept"
    )
    telemetry_rollup_interval_s: float = Field(
        default=60.0, gt=0, description="How often the roll-up and retention task runs"
    )
    signing_key: SecretStr | None = Field(
        default=None,
        description="Base64 Ed25519 32-byte seed for signing deployment packages (E5.2)",
    )
    log_level: str = Field(default="INFO", description="DEBUG | INFO | WARNING | ERROR")
    log_json: bool = Field(default=True, description="Structured JSON logs")

    # Token-bucket rate limits (NE1103). capacity = burst, rate = refill per second.
    ratelimit_api_capacity: float = Field(default=120.0, gt=0)
    ratelimit_api_rate: float = Field(default=60.0, gt=0)
    ratelimit_compile_capacity: float = Field(default=20.0, gt=0)
    ratelimit_compile_rate: float = Field(default=10.0, gt=0)
    ratelimit_guest_capacity: float = Field(default=5.0, gt=0)
    ratelimit_guest_rate: float = Field(default=1.0 / 60.0, gt=0)

    def token_secret(self) -> bytes:
        """The HMAC key for guest tokens.

        In `production` a `SYNAPSE_JWT_SECRET` of at least 32 bytes is required; elsewhere a
        stable per-process development key is derived so local runs and tests work without a
        secret. A development token is never valid in another process, which is the point.
        """
        if self.jwt_secret is not None:
            key = self.jwt_secret.get_secret_value().encode("utf-8")
            if self.environment == "production" and len(key) < 32:
                raise ValueError("SYNAPSE_JWT_SECRET must be at least 32 bytes in production")
            return key
        if self.environment == "production":
            raise ValueError("SYNAPSE_JWT_SECRET is required in production")
        return _DEV_TOKEN_SECRET

    def guest_ip_salt_bytes(self) -> bytes:
        """The salt for hashing a guest IP; a stable development default when unset."""
        if self.guest_ip_salt is not None:
            return self.guest_ip_salt.get_secret_value().encode("utf-8")
        return b"synapse-dev-guest-salt"

    @property
    def limiter(self) -> "LimiterSettings":
        """The rate-limiter configuration assembled from the scalar settings."""
        return LimiterSettings(
            api_capacity=self.ratelimit_api_capacity,
            api_rate=self.ratelimit_api_rate,
            compile_capacity=self.ratelimit_compile_capacity,
            compile_rate=self.ratelimit_compile_rate,
            guest_capacity=self.ratelimit_guest_capacity,
            guest_rate=self.ratelimit_guest_rate,
        )


@lru_cache
def get_settings() -> Settings:
    """Cached settings instance."""
    return Settings()
