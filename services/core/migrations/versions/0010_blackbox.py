"""Black box records uploaded by daemons (E6.2).

Revision ID: 0010
Revises: 0009
Create Date: 2026-11-23
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "0010"
down_revision: str | None = "0009"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None

_JSON = sa.JSON().with_variant(sa.dialects.postgresql.JSONB(), "postgresql")


def upgrade() -> None:
    op.create_table(
        "blackbox_records",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("cell_id", sa.String(120), nullable=False),
        sa.Column("device", sa.String(120), nullable=False),
        sa.Column("fault_kind", sa.String(16), nullable=False),
        sa.Column("fault_code", sa.String(120), nullable=False),
        sa.Column("at_ns", sa.BigInteger(), nullable=False),
        sa.Column("record", _JSON, nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_blackbox_records")),
    )
    op.create_index("ix_blackbox_records_cell_at", "blackbox_records", ["cell_id", "at_ns"])


def downgrade() -> None:
    op.drop_index("ix_blackbox_records_cell_at", table_name="blackbox_records")
    op.drop_table("blackbox_records")
