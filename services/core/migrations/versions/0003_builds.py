"""Builds and their artifacts (E2.5).

Revision ID: 0003
Revises: 0002
Create Date: 2026-11-23
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op
from sqlalchemy.dialects.postgresql import JSONB

revision: str = "0003"
down_revision: str | None = "0002"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None

JSON_TYPE = sa.JSON().with_variant(JSONB(), "postgresql")


def upgrade() -> None:
    op.create_table(
        "builds",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("version_id", sa.Uuid(), nullable=False),
        sa.Column("kind", sa.String(16), nullable=False),
        sa.Column("args", JSON_TYPE, nullable=False),
        sa.Column("status", sa.String(16), nullable=False),
        sa.Column("diagnostics", JSON_TYPE, nullable=False),
        sa.Column("messages", JSON_TYPE, nullable=False),
        sa.Column("artifacts", JSON_TYPE, nullable=False),
        sa.Column("log_tail", sa.Text(), nullable=False),
        sa.Column("binary_version", sa.String(64), nullable=True),
        sa.Column("exit_code", sa.Integer(), nullable=True),
        sa.Column("error_code", sa.String(8), nullable=True),
        sa.Column("error_message", sa.Text(), nullable=True),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("started_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("finished_at", sa.DateTime(timezone=True), nullable=True),
        sa.ForeignKeyConstraint(
            ["version_id"],
            ["project_versions.id"],
            name=op.f("fk_builds_version_id_project_versions"),
            ondelete="CASCADE",
        ),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_builds")),
    )
    op.create_index(op.f("ix_builds_version_id"), "builds", ["version_id"])
    op.create_table(
        "build_artifacts",
        sa.Column("build_id", sa.Uuid(), nullable=False),
        sa.Column("name", sa.String(300), nullable=False),
        sa.Column("sha256", sa.String(64), nullable=False),
        sa.Column("size", sa.Integer(), nullable=False),
        sa.Column("content", sa.LargeBinary(), nullable=False),
        sa.ForeignKeyConstraint(
            ["build_id"],
            ["builds.id"],
            name=op.f("fk_build_artifacts_build_id_builds"),
            ondelete="CASCADE",
        ),
        sa.PrimaryKeyConstraint("build_id", "name", name=op.f("pk_build_artifacts")),
    )


def downgrade() -> None:
    op.drop_table("build_artifacts")
    op.drop_index(op.f("ix_builds_version_id"), table_name="builds")
    op.drop_table("builds")
