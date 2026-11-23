"""Request and response models of the service-token routes (E3.3)."""

import uuid
from datetime import datetime
from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, StringConstraints

CellId = Annotated[str, StringConstraints(strip_whitespace=True, min_length=1, max_length=120)]
Label = Annotated[str, StringConstraints(max_length=120)]


class ServiceTokenCreate(BaseModel):
    """Body of `POST /v1/service-tokens`. A daemon token needs a `cell_id`; a twin token
    must not have one."""

    model_config = ConfigDict(extra="forbid")
    role: Literal["daemon", "twin"]
    cell_id: CellId | None = None
    label: Label = ""


class ServiceTokenInfo(BaseModel):
    """A service token's metadata (never its secret)."""

    model_config = ConfigDict(from_attributes=True)
    id: uuid.UUID
    role: str
    cell_id: str | None
    label: str
    created_at: datetime
    revoked_at: datetime | None


class ServiceTokenIssued(ServiceTokenInfo):
    """The response of issuing a token; `token` is shown once and never stored in clear."""

    token: str = Field(description="The secret, shown only now; store it, it cannot be recovered")


class ServiceTokenList(BaseModel):
    """Every service token's metadata."""

    items: list[ServiceTokenInfo]
