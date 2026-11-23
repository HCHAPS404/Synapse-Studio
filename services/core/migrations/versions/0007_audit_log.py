"""Append-only audit log with INSERT/SELECT-only grants (E3.4).

Revision ID: 0007
Revises: 0006
Create Date: 2026-11-23

The table is append-only by grant: on Postgres the application role keeps only INSERT and
SELECT on it (UPDATE and DELETE are revoked), so no code path can change or remove a row.
That grant is the one thing this migration does not reverse (the design says so): `downgrade`
drops the table, which removes the grant with it, but it does not re-grant UPDATE/DELETE.
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "0007"
down_revision: str | None = "0006"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    op.create_table(
        "audit_log",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("actor", sa.String(120), nullable=False),
        sa.Column("role", sa.String(16), nullable=False),
        sa.Column("action", sa.String(64), nullable=False),
        sa.Column("target", sa.String(300), nullable=True),
        sa.Column("request_id", sa.String(64), nullable=True),
        sa.Column("result", sa.String(16), nullable=False),
        sa.Column(
            "detail",
            sa.JSON().with_variant(sa.dialects.postgresql.JSONB(), "postgresql"),
            nullable=False,
        ),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_audit_log")),
    )
    op.create_index(op.f("ix_audit_log_at"), "audit_log", ["at"])
    op.create_index(op.f("ix_audit_log_action"), "audit_log", ["action"])
    bind = op.get_bind()
    if bind.dialect.name == "postgresql":
        # Append-only: the application role may insert and read, never update or delete.
        # CURRENT_USER is the role running the migration, which is the application role in
        # this single-service deployment.
        op.execute("REVOKE UPDATE, DELETE, TRUNCATE ON audit_log FROM CURRENT_USER")
        op.execute("GRANT INSERT, SELECT ON audit_log TO CURRENT_USER")


def downgrade() -> None:
    op.drop_index(op.f("ix_audit_log_action"), table_name="audit_log")
    op.drop_index(op.f("ix_audit_log_at"), table_name="audit_log")
    op.drop_table("audit_log")
