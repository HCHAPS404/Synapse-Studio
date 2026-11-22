"""ORM models. Tables of later tasks are added here with their own Alembic revisions."""

from sqlalchemy import String, Text
from sqlalchemy.orm import Mapped, mapped_column

from synapse_core.db.base import Base


class ServiceMeta(Base):
    """Key/value markers about the schema (baseline table, revision 0001)."""

    __tablename__ = "service_meta"

    key: Mapped[str] = mapped_column(String(64), primary_key=True)
    value: Mapped[str] = mapped_column(Text, nullable=False)
