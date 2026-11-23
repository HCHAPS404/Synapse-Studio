"""Daemon pairing: pairing codes, daemons and devices (E4.3).

Revision ID: 0008
Revises: 0007
Create Date: 2026-11-23
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "0008"
down_revision: str | None = "0007"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    op.create_table(
        "pairing_codes",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("code_hash", sa.String(64), nullable=False),
        sa.Column("cell_id", sa.String(120), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("expires_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("claimed_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("daemon_id", sa.Uuid(), nullable=True),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_pairing_codes")),
        sa.UniqueConstraint("code_hash", name=op.f("uq_pairing_codes_code_hash")),
    )
    op.create_table(
        "daemons",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("name", sa.String(120), nullable=False),
        sa.Column("cell_id", sa.String(120), nullable=False),
        sa.Column("service_token_id", sa.Uuid(), nullable=False),
        sa.Column("version", sa.String(64), nullable=True),
        sa.Column("last_heartbeat", sa.DateTime(timezone=True), nullable=True),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("revoked_at", sa.DateTime(timezone=True), nullable=True),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_daemons")),
    )
    op.create_index(op.f("ix_daemons_cell_id"), "daemons", ["cell_id"])
    op.create_table(
        "devices",
        sa.Column("daemon_id", sa.Uuid(), nullable=False),
        sa.Column("name", sa.String(120), nullable=False),
        sa.Column("kind", sa.String(64), nullable=False),
        sa.Column("simulated", sa.Boolean(), nullable=False),
        sa.ForeignKeyConstraint(
            ["daemon_id"],
            ["daemons.id"],
            name=op.f("fk_devices_daemon_id_daemons"),
            ondelete="CASCADE",
        ),
        sa.PrimaryKeyConstraint("daemon_id", "name", name=op.f("pk_devices")),
    )


def downgrade() -> None:
    op.drop_table("devices")
    op.drop_index(op.f("ix_daemons_cell_id"), table_name="daemons")
    op.drop_table("daemons")
    op.drop_table("pairing_codes")
