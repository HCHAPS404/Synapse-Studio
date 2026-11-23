"""Command line entry point: `synapse-daemon run` and `synapse-daemon pair`.

The runtime wiring (relay client, drivers, LAN server) is filled in across E5.1 to E5.11.
This CLI resolves the settings and reports the run mode so the daemon can be started and its
configuration checked without hardware and without the cloud.
"""

from __future__ import annotations

import argparse
import sys

from synapse_daemon import __version__
from synapse_daemon.config import get_settings
from synapse_daemon.lan import run_mode


def _run(_: argparse.Namespace) -> int:
    settings = get_settings()
    mode = run_mode(settings)
    print(f"synapse-daemon {__version__}: {mode.description}")
    if settings.is_local_mode():
        print("no service token configured: running offline (LAN only)")
    else:
        print(f"cell={settings.cell_id} relay={settings.relay_url}")
    return 0


def main(argv: list[str] | None = None) -> int:
    """Parse arguments and dispatch. Returns a process exit code."""
    parser = argparse.ArgumentParser(prog="synapse-daemon", description="SYNAPSE Edge Daemon")
    parser.add_argument("--version", action="version", version=__version__)
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run", help="Run the daemon")
    run.set_defaults(func=_run)

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
