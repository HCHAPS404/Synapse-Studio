"""Route-level RBAC (E3.2): the permission dependencies on project, build and device routes.

Every denied cell of the matrix is `NE1106`, a guest is scoped to its own sandbox, and a
guest is denied on the non-public device routes (E-R3). DB tests need
SYNAPSE_TEST_DATABASE_URL and are skipped otherwise.
"""

import uuid

from conftest import needs_db
from httpx import AsyncClient
from sqlalchemy.ext.asyncio import async_sessionmaker

from synapse_core.db.models import User
from synapse_core.db.session import get_engine
from synapse_core.services import users as svc
from synapse_core.settings import Settings, get_settings

CHEAP = {"argon2_time_cost": 1, "argon2_memory_cost_kib": 8, "argon2_parallelism": 1}


async def _user_token(app, client: AsyncClient, role: str) -> str:
    """Creates a user of `role` and returns a bearer token for it."""
    settings = app.dependency_overrides[get_settings]()
    cheap = Settings(database_url=settings.database_url, **CHEAP)
    engine = get_engine(settings)
    assert engine is not None
    email = f"{role}-{uuid.uuid4().hex[:8]}@example.com"
    async with async_sessionmaker(engine, expire_on_commit=False)() as s:
        await svc.create_user(s, email, "pw-123456", role, cheap)
    resp = await client.post("/v1/auth/login", json={"email": email, "password": "pw-123456"})
    assert resp.status_code == 200, resp.text
    return resp.json()["access_token"]


def _auth(token: str) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


async def _guest_token(client: AsyncClient) -> str:
    created = await client.post("/v1/guest")
    assert created.status_code == 201
    return created.json()["token"]


# ---- anonymous is denied -----------------------------------------------------------


@needs_db
async def test_anonymous_cannot_touch_projects(client: AsyncClient) -> None:
    denied = [
        await client.get("/v1/projects"),
        await client.post("/v1/projects", json={"name": "x"}),
        await client.get(f"/v1/projects/{uuid.uuid4()}"),
    ]
    for resp in denied:
        assert resp.status_code == 403, resp.text
        assert resp.json()["code"] == "NE1106"


# ---- user roles --------------------------------------------------------------------


@needs_db
async def test_viewer_reads_but_cannot_write(app, client: AsyncClient) -> None:
    eng = await _user_token(app, client, "engineer")
    viewer = await _user_token(app, client, "viewer")
    # Engineer creates a project.
    created = await client.post("/v1/projects", json={"name": "p"}, headers=_auth(eng))
    assert created.status_code == 201
    pid = created.json()["id"]
    # Viewer can read it and list.
    assert (await client.get(f"/v1/projects/{pid}", headers=_auth(viewer))).status_code == 200
    assert (await client.get("/v1/projects", headers=_auth(viewer))).status_code == 200
    # Viewer cannot create or rename.
    denied = await client.post("/v1/projects", json={"name": "q"}, headers=_auth(viewer))
    assert denied.status_code == 403 and denied.json()["code"] == "NE1106"
    rename = await client.put(
        f"/v1/projects/{pid}", json={"name": "r", "row_version": 1}, headers=_auth(viewer)
    )
    assert rename.status_code == 403 and rename.json()["code"] == "NE1106"


@needs_db
async def test_operator_and_engineer_can_write(app, client: AsyncClient) -> None:
    for role in ("operator", "engineer"):
        token = await _user_token(app, client, role)
        created = await client.post("/v1/projects", json={"name": role}, headers=_auth(token))
        assert created.status_code == 201, created.text


# ---- device routes: guest denied, viewer allowed, catalog public -------------------


@needs_db
async def test_guest_is_denied_device_routes_but_catalog_is_public(
    app, client: AsyncClient
) -> None:
    guest = await _guest_token(client)
    # The catalog is public even for a guest.
    assert (await client.get("/v1/devices/catalog", headers=_auth(guest))).status_code == 200
    # Listing and reading a device are denied to a guest (E-R3).
    listed = await client.get("/v1/devices", headers=_auth(guest))
    assert listed.status_code == 403 and listed.json()["code"] == "NE1106"
    one = await client.get("/v1/devices/anything", headers=_auth(guest))
    assert one.status_code == 403 and one.json()["code"] == "NE1106"
    # A viewer may list devices.
    viewer = await _user_token(app, client, "viewer")
    assert (await client.get("/v1/devices", headers=_auth(viewer))).status_code == 200


# ---- guest sandbox isolation on projects -------------------------------------------


@needs_db
async def test_a_guest_cannot_see_another_sandbox_or_a_users_project(
    app, client: AsyncClient
) -> None:
    # A user's project.
    eng = await _user_token(app, client, "engineer")
    user_project = (
        await client.post("/v1/projects", json={"name": "owned"}, headers=_auth(eng))
    ).json()["id"]
    # Guest A creates its own project.
    guest_a = await _guest_token(client)
    a_project = (
        await client.post("/v1/projects", json={"name": "mine"}, headers=_auth(guest_a))
    ).json()["id"]
    # Guest B sees neither the user's project nor guest A's.
    guest_b = await _guest_token(client)
    assert (
        await client.get(f"/v1/projects/{user_project}", headers=_auth(guest_b))
    ).status_code == 404
    assert (
        await client.get(f"/v1/projects/{a_project}", headers=_auth(guest_b))
    ).status_code == 404
    listing = await client.get("/v1/projects", headers=_auth(guest_b))
    assert listing.status_code == 200 and listing.json()["total"] == 0
    # Guest A sees exactly its own one project.
    a_listing = await client.get("/v1/projects", headers=_auth(guest_a))
    assert a_listing.json()["total"] == 1 and a_listing.json()["items"][0]["id"] == a_project


@needs_db
async def test_a_disabled_users_token_stops_working(app, client: AsyncClient) -> None:
    token = await _user_token(app, client, "engineer")
    assert (await client.get("/v1/projects", headers=_auth(token))).status_code == 200
    settings = app.dependency_overrides[get_settings]()
    engine = get_engine(settings)
    assert engine is not None
    async with async_sessionmaker(engine, expire_on_commit=False)() as s:
        user = (await s.execute(_all_users())).scalars().first()
        assert user is not None
        user.disabled = True
        await s.commit()
    after = await client.get("/v1/projects", headers=_auth(token))
    assert after.status_code == 401 and after.json()["code"] == "NE1105"


def _all_users():  # noqa: ANN202
    from sqlalchemy import select

    return select(User)
