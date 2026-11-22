"""Runtime configuration loaded from environment variables (never hard-coded)."""

from functools import lru_cache

from pydantic import Field, SecretStr
from pydantic_settings import BaseSettings, SettingsConfigDict


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
    log_level: str = Field(default="INFO", description="DEBUG | INFO | WARNING | ERROR")
    log_json: bool = Field(default=True, description="Structured JSON logs")


@lru_cache
def get_settings() -> Settings:
    """Cached settings instance."""
    return Settings()
