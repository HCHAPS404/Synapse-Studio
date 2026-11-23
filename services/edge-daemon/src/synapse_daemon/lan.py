"""Local LAN mode: the daemon works with no cloud.

When no cloud relay is configured the daemon runs on the LAN only. E5.1 defines the mode and
its description; the LAN HTTP server that serves the PWA and the offline relay socket are
built in E5.11. This module keeps the mode decision in one place so the CLI and the runtime
agree on it.
"""

from __future__ import annotations

from dataclasses import dataclass

from synapse_daemon.config import DaemonSettings


@dataclass(frozen=True, slots=True)
class RunMode:
    """Whether the daemon relays to the cloud or serves the LAN only."""

    local: bool
    relay_url: str | None
    lan_host: str
    lan_port: int

    @property
    def description(self) -> str:
        if self.local:
            return f"local LAN mode on {self.lan_host}:{self.lan_port} (no cloud relay)"
        return f"cloud relay to {self.relay_url} plus LAN on {self.lan_host}:{self.lan_port}"


def run_mode(settings: DaemonSettings) -> RunMode:
    """Decide the run mode from the settings."""
    return RunMode(
        local=settings.is_local_mode(),
        relay_url=settings.relay_url,
        lan_host=settings.lan_host,
        lan_port=settings.lan_port,
    )
