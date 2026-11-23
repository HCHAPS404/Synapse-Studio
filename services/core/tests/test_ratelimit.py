"""Rate limiter (E2.7): the token bucket, the scope map, and the middleware over the app."""

from fastapi.testclient import TestClient
from hypothesis import given
from hypothesis import settings as hyp_settings
from hypothesis import strategies as st

from synapse_core.limits import (
    BucketConfig,
    RateLimiter,
    client_key,
    scope_for,
)
from synapse_core.main import create_app
from synapse_core.settings import Settings, get_settings


class FakeClock:
    """A clock the tests drive by hand."""

    def __init__(self) -> None:
        self.now = 0.0

    def __call__(self) -> float:
        return self.now


# ---- the bucket --------------------------------------------------------------------


def test_a_full_bucket_admits_its_capacity_then_refuses() -> None:
    clock = FakeClock()
    limiter = RateLimiter({"api": BucketConfig(capacity=3, rate=1)}, clock=clock)
    assert [limiter.check("api", "c").allowed for _ in range(3)] == [True, True, True]
    refused = limiter.check("api", "c")
    assert refused.allowed is False
    assert refused.retry_after_s > 0
    assert int(refused.retry_after_header) >= 1


def test_tokens_refill_at_the_configured_rate() -> None:
    clock = FakeClock()
    limiter = RateLimiter({"api": BucketConfig(capacity=2, rate=2)}, clock=clock)
    assert limiter.check("api", "c").allowed
    assert limiter.check("api", "c").allowed
    assert not limiter.check("api", "c").allowed
    clock.now += 0.5  # 2 tokens/s * 0.5 s = 1 token
    assert limiter.check("api", "c").allowed
    assert not limiter.check("api", "c").allowed


def test_buckets_are_independent_per_client_and_scope() -> None:
    clock = FakeClock()
    limiter = RateLimiter({"api": BucketConfig(1, 1), "guest": BucketConfig(1, 1)}, clock=clock)
    assert limiter.check("api", "a").allowed
    assert not limiter.check("api", "a").allowed
    assert limiter.check("api", "b").allowed  # another client, own bucket
    assert limiter.check("guest", "a").allowed  # another scope, own bucket


def test_retry_after_is_long_enough_to_pass() -> None:
    clock = FakeClock()
    limiter = RateLimiter({"api": BucketConfig(1, 2)}, clock=clock)
    assert limiter.check("api", "c").allowed
    refused = limiter.check("api", "c")
    assert not refused.allowed
    clock.now += refused.retry_after_s
    assert limiter.check("api", "c").allowed


# ---- the rate bound, as a property -------------------------------------------------


@hyp_settings(max_examples=300)
@given(
    capacity=st.integers(min_value=1, max_value=50),
    rate=st.floats(min_value=0.1, max_value=50, allow_nan=False, allow_infinity=False),
    steps=st.lists(
        st.floats(min_value=0.0, max_value=5.0, allow_nan=False, allow_infinity=False),
        min_size=1,
        max_size=60,
    ),
)
def test_admitted_never_exceeds_capacity_plus_rate_times_elapsed(
    capacity: int, rate: float, steps: list[float]
) -> None:
    """The invariant of E-R4: over any window the admitted count stays within the bound.

    Start the bucket full, advance the clock by each step and attempt one request after
    each advance. The number admitted must never exceed `capacity + rate * elapsed`, where
    `elapsed` is the total time advanced so far.
    """
    clock = FakeClock()
    limiter = RateLimiter({"s": BucketConfig(capacity=capacity, rate=rate)}, clock=clock)
    admitted = 0
    elapsed = 0.0
    # The initial attempt at t=0.
    if limiter.check("s", "c").allowed:
        admitted += 1
    for step in steps:
        clock.now += step
        elapsed += step
        if limiter.check("s", "c").allowed:
            admitted += 1
        # +1e-6 absorbs floating-point error; the bound holds exactly in exact arithmetic.
        assert admitted <= capacity + rate * elapsed + 1e-6


# ---- the scope map -----------------------------------------------------------------


def test_scope_map_sorts_requests_into_buckets() -> None:
    assert scope_for("GET", "/health") is None
    assert scope_for("GET", "/openapi.json") is None
    assert scope_for("POST", "/v1/guest") == "guest"
    assert scope_for("POST", "/v1/versions/abc/builds") == "compile"
    assert scope_for("GET", "/v1/projects") == "api"
    assert scope_for("POST", "/v1/projects") == "api"


def test_client_key_prefers_the_principal() -> None:
    assert client_key(principal="user-1", ip="1.2.3.4") == "p:user-1"
    assert client_key(principal=None, ip="1.2.3.4") == "ip:1.2.3.4"
    assert client_key(principal=None, ip=None) == "ip:unknown"


# ---- the middleware over the app ---------------------------------------------------


def _app(**overrides: float) -> TestClient:
    base = {"database_url": None, "ratelimit_api_capacity": 3, "ratelimit_api_rate": 0.001}
    base.update(overrides)
    settings = Settings(**base)  # type: ignore[arg-type]
    app = create_app()
    app.dependency_overrides[get_settings] = lambda: settings
    app.state.rate_limiter = RateLimiter(settings.limiter.scopes())
    return TestClient(app)


def test_health_is_never_rate_limited() -> None:
    client = _app()
    for _ in range(20):
        assert client.get("/health").status_code == 200


def test_api_requests_over_the_bucket_get_ne1103_with_retry_after() -> None:
    client = _app()
    codes = [client.get("/v1/capabilities").status_code for _ in range(5)]
    assert codes[:3] == [200, 200, 200]
    assert 429 in codes
    refused = client.get("/v1/capabilities")
    assert refused.status_code == 429
    assert refused.json()["code"] == "NE1103"
    assert int(refused.headers["retry-after"]) >= 1
    assert refused.json()["request_id"]  # the request id survives a refusal
