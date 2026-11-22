"""Projects, immutable versions and their files (E2.4).

Revision ID: 0002
Revises: 0001
Create Date: 2026-11-23
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "0002"
down_revision: str | None = "0001"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    op.create_table(
        "projects",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("owner_id", sa.Uuid(), nullable=True),
        sa.Column("sandbox_id", sa.Uuid(), nullable=True),
        sa.Column("name", sa.String(120), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("row_version", sa.Integer(), nullable=False),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_projects")),
    )
    op.create_index(op.f("ix_projects_owner_id"), "projects", ["owner_id"])
    op.create_index(op.f("ix_projects_sandbox_id"), "projects", ["sandbox_id"])
    op.create_table(
        "project_versions",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("project_id", sa.Uuid(), nullable=False),
        sa.Column("number", sa.Integer(), nullable=False),
        sa.Column("content_hash", sa.String(64), nullable=False),
        sa.Column("message", sa.String(500), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.ForeignKeyConstraint(
            ["project_id"],
            ["projects.id"],
            name=op.f("fk_project_versions_project_id_projects"),
            ondelete="CASCADE",
        ),
        sa.PrimaryKeyConstraint("id", name=op.f("pk_project_versions")),
        sa.UniqueConstraint("project_id", "number", name=op.f("uq_project_versions_project_id")),
    )
    op.create_index(op.f("ix_project_versions_project_id"), "project_versions", ["project_id"])
    op.create_table(
        "version_files",
        sa.Column("version_id", sa.Uuid(), nullable=False),
        sa.Column("path", sa.String(200), nullable=False),
        sa.Column("content", sa.Text(), nullable=False),
        sa.Column("size", sa.Integer(), nullable=False),
        sa.ForeignKeyConstraint(
            ["version_id"],
            ["project_versions.id"],
            name=op.f("fk_version_files_version_id_project_versions"),
            ondelete="CASCADE",
        ),
        sa.PrimaryKeyConstraint("version_id", "path", name=op.f("pk_version_files")),
    )


def downgrade() -> None:
    op.drop_table("version_files")
    op.drop_index(op.f("ix_project_versions_project_id"), table_name="project_versions")
    op.drop_table("project_versions")
    op.drop_index(op.f("ix_projects_sandbox_id"), table_name="projects")
    op.drop_index(op.f("ix_projects_owner_id"), table_name="projects")
    op.drop_table("projects")
