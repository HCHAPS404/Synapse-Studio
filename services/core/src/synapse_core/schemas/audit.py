"""Response models of the audit read route (E3.4)."""

import uuid
from datetime import datetime
from typing import Any

from pydantic import BaseModel


class AuditEntryOut(BaseModel):
    """One audit log row."""

    id: uuid.UUID
    at: datetime
    actor: str
    role: str
    action: str
    target: str | None
    request_id: str | None
    result: str
    detail: dict[str, Any]


class AuditPage(BaseModel):
    """A page of audit rows, newest first."""

    items: list[AuditEntryOut]
    total: int
    limit: int
    offset: int
