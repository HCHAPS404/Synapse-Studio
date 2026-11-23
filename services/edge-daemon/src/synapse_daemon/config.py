"""Daemon configuration from environment variables (never hard-coded) and a local file.

The daemon runs on a PC or a Raspberry Pi 5. It works with no cloud: when `relay_url` is
unset it runs in local LAN mode, serving the PWA and relaying on the LAN only. Secrets
(the service token) come from the environment or a file the operator controls, never a
literal in code.
"""

from functools import lru_cache
from pathlib import Path

from pydantic import Field, SecretStr
from pydantic_settings import BaseSettings, SettingsConfigDict


class DaemonSettings(BaseSettings):
    """Edge daemon settings. Values come from env vars prefixed with SYNAPSE_DAEMON_."""

    model_config = SettingsConfigDict(env_prefix="SYNAPSE_DAEMON_", env_file=".env", extra="ignore")

    name: str = Field(default="daemon", description="Daemon name shown in the cell")
    cell_id: str = Field(default="", description="Cell this daemon serves; set at pairing")

    # Cloud relay. Unset or empty means local LAN mode (no cloud).
    relay_url: str | None = Field(
        default=None, description="wss URL of the Core API relay, e.g. wss://host/v1/relay"
    )
    api_base_url: str | None = Field(
        default=None, description="https base URL of the Core API for pairing and uploads"
    )
    service_token: SecretStr | None = Field(
        default=None, description="Daemon service token (svc_...) obtained at pairing"
    )

    # Relay client behaviour.
    heartbeat_interval_s: float = Field(default=5.0, gt=0, description="deployment_status period")
    reconnect_min_s: float = Field(default=1.0, gt=0, description="First reconnect backoff")
    reconnect_max_s: float = Field(default=30.0, gt=0, description="Reconnect backoff ceiling")
    ping_interval_s: float = Field(default=15.0, gt=0, description="Client ping when idle")

    # Local LAN server for the PWA and offline relay.
    lan_host: str = Field(default="0.0.0.0", description="Address the LAN server binds")  # noqa: S104
    lan_port: int = Field(default=8787, ge=1, le=65535, description="LAN server port")
    pwa_dir: str | None = Field(
        default=None, description="Folder of the built PWA to serve on the LAN"
    )

    # Signed packages (E5.2) and deployments (E5.4).
    trusted_keys: list[str] = Field(
        default_factory=list, description="Base64 Ed25519 public keys the daemon trusts"
    )
    quarantine_dir: str | None = Field(
        default=None, description="Where rejected packages are kept; default: <work>/quarantine"
    )
    work_dir: str | None = Field(
        default=None, description="Daemon state and package directory; default: ./daemon-data"
    )
    require_distinct_confirmers: bool = Field(
        default=False, description="Two deployment confirmations must be different people"
    )

    # E-STOP (E5.10). The input is read-only; the software E-STOP is supplementary.
    estop_input: str | None = Field(
        default=None, description="Configured digital input the daemon reads the E-STOP from"
    )

    def is_local_mode(self) -> bool:
        """True when no cloud relay is configured: the daemon runs on the LAN only."""
        return not (self.relay_url and self.service_token)

    def work_path(self) -> Path:
        """The daemon state directory, created on demand by callers that write to it."""
        return Path(self.work_dir) if self.work_dir else Path("daemon-data")

    def quarantine_path(self) -> Path:
        """Where a package that fails verification is kept and never applied."""
        if self.quarantine_dir:
            return Path(self.quarantine_dir)
        return self.work_path() / "quarantine"


@lru_cache
def get_settings() -> DaemonSettings:
    """Cached settings instance."""
    return DaemonSettings()
