"""Administrative command line for the Core API (E3.1).

`synapse-core create-engineer --email a@b.c` creates the first engineer. The password is
read from stdin (never from an argument, so it does not reach the process table or shell
history) and never printed. The command refuses to run once any user exists, so it cannot
be used to add users behind the engineer-only route (E3.2); it exists only to bootstrap.
"""

from __future__ import annotations

import argparse
import asyncio
import sys

from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core.db.session import get_engine
from synapse_core.services import users as users_svc
from synapse_core.settings import get_settings


async def _create_engineer(email: str, password: str) -> int:
    settings = get_settings()
    engine = get_engine(settings)
    if engine is None:
        print("SYNAPSE_DATABASE_URL is not set", file=sys.stderr)
        return 2
    async with async_sessionmaker(engine, expire_on_commit=False)() as session:
        if await users_svc.count_users(session) > 0:
            print(
                "users already exist; the bootstrap command only creates the first engineer",
                file=sys.stderr,
            )
            return 1
        try:
            user = await users_svc.create_user(session, email, password, "engineer", settings)
        except ValueError as exc:
            print(str(exc), file=sys.stderr)
            return 1
    print(f"created engineer {user.email}")
    return 0


def _read_password() -> str:
    """Reads the password from stdin without echoing it, or from a pipe."""
    if sys.stdin.isatty():
        import getpass

        first = getpass.getpass("Password: ")
        second = getpass.getpass("Repeat password: ")
        if first != second:
            print("passwords do not match", file=sys.stderr)
            raise SystemExit(1)
        return first
    return sys.stdin.readline().rstrip("\n")


def main(argv: list[str] | None = None) -> int:
    """Entry point for the `synapse-core` console script."""
    parser = argparse.ArgumentParser(prog="synapse-core", description="Core API admin commands")
    sub = parser.add_subparsers(dest="command", required=True)
    bootstrap = sub.add_parser("create-engineer", help="create the first engineer user")
    bootstrap.add_argument("--email", required=True, help="the engineer's email")
    args = parser.parse_args(argv)
    if args.command == "create-engineer":
        password = _read_password()
        if not password:
            print("password must not be empty", file=sys.stderr)
            return 1
        return asyncio.run(_create_engineer(args.email, password))
    parser.error(f"unknown command {args.command}")  # pragma: no cover


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
