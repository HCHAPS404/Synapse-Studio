"""Password hashing with argon2id (E3.1).

Hashes use `argon2-cffi` (Argon2id). The cost parameters live in settings so they can be
tuned per environment; the salt is random per hash and stored inside the PHC string, so
nothing else needs to be kept. Passwords and hashes are never logged.
"""

from __future__ import annotations

from argon2 import PasswordHasher
from argon2.exceptions import InvalidHashError, VerifyMismatchError


def _hasher(time_cost: int, memory_cost_kib: int, parallelism: int) -> PasswordHasher:
    return PasswordHasher(time_cost=time_cost, memory_cost=memory_cost_kib, parallelism=parallelism)


def hash_password(password: str, *, time_cost: int, memory_cost_kib: int, parallelism: int) -> str:
    """Returns an Argon2id PHC hash string for `password` (salt embedded)."""
    return _hasher(time_cost, memory_cost_kib, parallelism).hash(password)


def verify_password(stored_hash: str, password: str) -> bool:
    """`True` when `password` matches `stored_hash`; `False` on any mismatch or bad hash.

    Uses a fresh hasher so verification does not depend on the current cost settings
    (the parameters are read from the stored PHC string).
    """
    try:
        return PasswordHasher().verify(stored_hash, password)
    except (VerifyMismatchError, InvalidHashError, ValueError):
        return False
