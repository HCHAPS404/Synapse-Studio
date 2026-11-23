"""Telemetry tables in plain Postgres: raw samples and one-minute roll-ups (E4.4, E4.5).

Revision ID: 0009
Revises: 0008
Create Date: 2026-11-23
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "0009"
down_revision: str | None = "0008"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None

_JSON = sa.JSON().with_variant(sa.dialects.postgresql.JSONB(), "postgresql")


def upgrade() -> None:
    op.create_table(
        "telemetry_samples",
        sa.Column("id", sa.BigInteger(), autoincrement=True, nullable=False),
        sa.Column("cell_id", sa.String(120), nullable=False),
        sa.Column("device", sa.String(120), nullable=False),
        sa.Column("tag", sa.String(200), nullable=False),
        sa.Column("t_ns", sa.BigInteger(), nullable=False),
        sa.Column("value", _JSON, nullable=False),
        sa.Column("quality", sa.String(8), nullable=False),
        sa.Column("simulated", sa.Boolean(), nullable=False),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_telemetry_samples")),
    )
    op.create_index(
        "ix_telemetry_samples_series",
        "telemetry_samples",
        ["cell_id", "device", "tag", "t_ns"],
    )
    op.create_table(
        "telemetry_1m",
        sa.Column("id", sa.BigInteger(), autoincrement=True, nullable=False),
        sa.Column("cell_id", sa.String(120), nullable=False),
        sa.Column("device", sa.String(120), nullable=False),
        sa.Column("tag", sa.String(200), nullable=False),
        sa.Column("minute", sa.BigInteger(), nullable=False),
        sa.Column("min_value", sa.Float(), nullable=False),
        sa.Column("max_value", sa.Float(), nullable=False),
        sa.Column("mean_value", sa.Float(), nullable=False),
        sa.Column("count", sa.Integer(), nullable=False),
        sa.Column("simulated", sa.Boolean(), nullable=False),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_telemetry_1m")),
        sa.UniqueConstraint(
            "cell_id", "device", "tag", "minute", name="uq_telemetry_1m_series_minute"
        ),
    )
    op.create_index(
        "ix_telemetry_1m_series", "telemetry_1m", ["cell_id", "device", "tag", "minute"]
    )


def downgrade() -> None:
    op.drop_index("ix_telemetry_1m_series", table_name="telemetry_1m")
    op.drop_table("telemetry_1m")
    op.drop_index("ix_telemetry_samples_series", table_name="telemetry_samples")
    op.drop_table("telemetry_samples")
