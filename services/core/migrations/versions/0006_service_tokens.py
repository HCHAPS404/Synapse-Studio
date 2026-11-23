"""Service tokens for the daemon and twin principals (E3.3).

Revision ID: 0006
Revises: 0005
Create Date: 2026-11-23
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "0006"
down_revision: str | None = "0005"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    op.create_table(
        "service_tokens",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("role", sa.String(16), nullable=False),
        sa.Column("cell_id", sa.String(120), nullable=True),
        sa.Column("token_hash", sa.String(64), nullable=False),
        sa.Column("label", sa.String(120), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("revoked_at", sa.DateTime(timezone=True), nullable=True),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_service_tokens")),
        sa.UniqueConstraint("token_hash", name=op.f("uq_service_tokens_token_hash")),
    )
    op.create_index(op.f("ix_service_tokens_cell_id"), "service_tokens", ["cell_id"])


def downgrade() -> None:
    op.drop_index(op.f("ix_service_tokens_cell_id"), table_name="service_tokens")
    op.drop_table("service_tokens")
