"""Request and response models of the auth routes (E3.1)."""

from datetime import datetime

from pydantic import BaseModel, Field


class LoginRequest(BaseModel):
    """Credentials for `POST /v1/auth/login`."""

    email: str = Field(min_length=3, max_length=320)
    password: str = Field(min_length=1, max_length=1024)


class TokenOut(BaseModel):
    """A short-lived JWT access token. Send it as `Authorization: Bearer <token>`."""

    access_token: str
    token_type: str = "bearer"  # noqa: S105 - the literal OAuth2 token type, not a secret
    role: str
    expires_at: datetime
