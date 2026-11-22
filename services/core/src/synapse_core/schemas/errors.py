"""The error body every non-2xx response carries."""

from pydantic import BaseModel, Field


class ErrorBody(BaseModel):
    """`{code, message, request_id}`.

    `code` is an `NE11xx` service code (docs/errors/services.md). Transport-level failures
    that have no catalogue entry (unknown route, bad method, internal error) use
    `HTTP_<status>`.
    """

    code: str = Field(pattern=r"^(NE11\d\d|HTTP_\d{3})$", examples=["NE1106"])
    message: str
    request_id: str | None = None
