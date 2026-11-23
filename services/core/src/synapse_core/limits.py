"""Token-bucket rate limiting (`NE1103`).

A bucket holds up to `capacity` tokens and refills at `rate` tokens per second. Each
admitted request takes one token; when none is available the request is refused and the
caller is told how long to wait (`Retry-After`). Buckets are kept in memory per client and
per scope (one process instance, as the design states).

Rate bound (the property the tests assert): with a bucket that starts full, the number of
requests admitted over any window of length `elapsed` never exceeds `capacity + rate *
elapsed`. A full bucket admits at most `capacity` immediately; after that every further
admission needs the bucket to refill, and it refills at `rate` per second, so at most
`rate * elapsed` more can pass in the window.
"""

from __future__ import annotations

import threading
import time
from collections.abc import Callable
from dataclasses import dataclass


@dataclass(frozen=True)
class BucketConfig:
    """Shape of one token bucket: a burst of `capacity`, refilled at `rate` per second."""

    capacity: float
    rate: float

    def __post_init__(self) -> None:
        if self.capacity <= 0:
            raise ValueError("capacity must be positive")
        if self.rate <= 0:
            raise ValueError("rate must be positive")


@dataclass
class _Bucket:
    """Mutable state of one client's bucket. Guarded by the limiter lock."""

    tokens: float
    updated: float


@dataclass
class Decision:
    """The outcome of one `check`: admitted, or refused with a wait in seconds."""

    allowed: bool
    retry_after_s: float = 0.0

    @property
    def retry_after_header(self) -> str:
        """`Retry-After` value in whole seconds, at least 1 when refused."""
        return str(max(1, int(self.retry_after_s + 0.999)))


class RateLimiter:
    """In-memory token-bucket limiter, keyed by `(scope, client)`.

    The clock is injectable so tests advance time deterministically. Access is guarded by a
    lock because FastAPI runs request handlers on a thread pool and background tasks here.
    """

    def __init__(
        self,
        scopes: dict[str, BucketConfig],
        *,
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        self._scopes = dict(scopes)
        self._clock = clock
        self._buckets: dict[tuple[str, str], _Bucket] = {}
        self._lock = threading.Lock()

    def config(self, scope: str) -> BucketConfig:
        """The bucket shape of a scope."""
        return self._scopes[scope]

    def _refill(self, bucket: _Bucket, cfg: BucketConfig, now: float) -> None:
        elapsed = now - bucket.updated
        if elapsed > 0:
            bucket.tokens = min(cfg.capacity, bucket.tokens + elapsed * cfg.rate)
            bucket.updated = now

    def check(self, scope: str, client: str, *, cost: float = 1.0) -> Decision:
        """Takes `cost` tokens of `(scope, client)` if available, else refuses with a wait."""
        cfg = self._scopes[scope]
        now = self._clock()
        with self._lock:
            bucket = self._buckets.get((scope, client))
            if bucket is None:
                bucket = _Bucket(tokens=cfg.capacity, updated=now)
                self._buckets[(scope, client)] = bucket
            self._refill(bucket, cfg, now)
            if bucket.tokens >= cost:
                bucket.tokens -= cost
                return Decision(allowed=True)
            missing = cost - bucket.tokens
            return Decision(allowed=False, retry_after_s=missing / cfg.rate)

    def reset(self) -> None:
        """Drops every bucket (test helper)."""
        with self._lock:
            self._buckets.clear()


@dataclass
class LimiterSettings:
    """The three scopes the design names: API calls, compile jobs and guest creation."""

    api_capacity: float = 120.0
    api_rate: float = 60.0
    compile_capacity: float = 20.0
    compile_rate: float = 10.0
    guest_capacity: float = 5.0
    guest_rate: float = 1.0 / 60.0  # five at once, then one more a minute

    def scopes(self) -> dict[str, BucketConfig]:
        return {
            "api": BucketConfig(self.api_capacity, self.api_rate),
            "compile": BucketConfig(self.compile_capacity, self.compile_rate),
            "guest": BucketConfig(self.guest_capacity, self.guest_rate),
        }


# Paths that are never rate limited (liveness and schema), checked by prefix or exact match.
EXEMPT_PATHS: frozenset[str] = frozenset(
    {"/health", "/openapi.json", "/docs", "/redoc", "/docs/oauth2-redirect"}
)


def client_key(*, principal: str | None, ip: str | None) -> str:
    """The bucket key for a request: the authenticated principal if any, else the IP.

    E3 passes a principal (user id, guest sandbox id or service token id); until then the
    IP is used, so a caller behind one address shares one bucket.
    """
    if principal:
        return f"p:{principal}"
    return f"ip:{ip or 'unknown'}"


def scope_for(method: str, path: str) -> str | None:
    """Which bucket a request draws from, or `None` when it is exempt.

    Guest creation (`POST /v1/guest`) and build submission (`POST .../builds`) have their
    own scopes so a flood of cheap reads cannot exhaust them and vice versa.
    """
    if path in EXEMPT_PATHS:
        return None
    if method == "POST" and path == "/v1/guest":
        return "guest"
    if method == "POST" and path.endswith("/builds"):
        return "compile"
    return "api"
