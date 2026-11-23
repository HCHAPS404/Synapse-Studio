"""Response model of the guest route."""

from datetime import datetime

from pydantic import BaseModel


class GuestSessionOut(BaseModel):
    """Response of `POST /v1/guest`: the token to send as `Authorization: Bearer <token>`."""

    token: str
    role: str
    sandbox_id: str
    expires_at: datetime
