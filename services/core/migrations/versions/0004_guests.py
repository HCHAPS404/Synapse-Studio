"""Guest sandboxes (E2.8).

Revision ID: 0004
Revises: 0003
Create Date: 2026-11-23
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "0004"
down_revision: str | None = "0003"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    op.create_table(
        "guest_sandboxes",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("expires_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("ip_hash", sa.String(64), nullable=True),
        sa.Column("project_count", sa.Integer(), nullable=False),
        sa.Column("builds_today", sa.Integer(), nullable=False),
        sa.Column("build_day", sa.String(10), nullable=False),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_guest_sandboxes")),
    )
    op.create_index(op.f("ix_guest_sandboxes_ip_hash"), "guest_sandboxes", ["ip_hash"])


def downgrade() -> None:
    op.drop_index(op.f("ix_guest_sandboxes_ip_hash"), table_name="guest_sandboxes")
    op.drop_table("guest_sandboxes")
