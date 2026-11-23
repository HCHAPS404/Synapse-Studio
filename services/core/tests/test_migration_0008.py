"""E4.3: the 0008 migration and the ORM models agree on the new tables (offline, SQLite).

The shared conftest builds the local test schema with `create_all`, so the pairing tests run
against the models, not the migration. This test applies the Alembic migrations to a throwaway
SQLite file and asserts the `daemons`, `devices` and `pairing_codes` tables have exactly the
columns the models declare, so a migration that drifts from the models is caught locally too
(the full upgrade/downgrade round trip still runs on Postgres in CI).
"""

from __future__ import annotations

from pathlib import Path

from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine, inspect

from synapse_core.db.models import Daemon, Device, PairingCode

ROOT = Path(__file__).resolve().parents[1]
_TABLES = {"daemons": Daemon, "devices": Device, "pairing_codes": PairingCode}


def test_migration_0008_columns_match_models(tmp_path: Path) -> None:
    db_path = tmp_path / "m.db"
    url = f"sqlite:///{db_path}"
    config = Config(str(ROOT / "alembic.ini"))
    config.set_main_option("script_location", str(ROOT / "migrations"))
    config.attributes["url"] = url  # sync sqlite URL (env.py uses create_engine)
    command.upgrade(config, "head")

    engine = create_engine(url)
    try:
        inspector = inspect(engine)
        for table, model in _TABLES.items():
            migrated = {c["name"] for c in inspector.get_columns(table)}
            declared = {c.name for c in model.__table__.columns}
            assert migrated == declared, f"{table}: migration {migrated} vs model {declared}"
    finally:
        engine.dispose()
