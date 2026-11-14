---
inclusion: fileMatch
fileMatchPattern: "**/*.py"
---

# Python style

- Python 3.12, full type hints, `pyright` clean, `ruff` for lint and format.
- Pydantic v2 models at every trust boundary; validate all external input.
- FastAPI: async handlers, dependency injection, explicit response models,
  structured logging, no business logic in route functions.
- Database: SQLAlchemy 2 with Alembic migrations; parameterized queries only.
- Tests with `pytest`; invariants with `hypothesis`.
- Secrets only from environment variables; never log secrets or user model keys.
