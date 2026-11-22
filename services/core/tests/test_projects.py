"""Projects, versions and files (E2.4). Route tests need a database (see conftest)."""

import random
import uuid

import pytest
from conftest import needs_db
from httpx import AsyncClient
from hypothesis import given
from hypothesis import strategies as st

from synapse_core.services.projects import content_hash

# ---- content hash (no database) ---------------------------------------------------

_files = st.dictionaries(
    st.from_regex(r"[a-z][a-z0-9_]{0,8}(/[a-z][a-z0-9_]{0,8})?\.neuron", fullmatch=True),
    st.text(max_size=40).map(lambda s: s.encode()),
    min_size=1,
    max_size=6,
)


@given(_files, st.randoms())
def test_hash_does_not_depend_on_file_order(files: dict[str, bytes], rnd: random.Random) -> None:
    keys = list(files)
    rnd.shuffle(keys)
    assert content_hash({k: files[k] for k in keys}) == content_hash(files)


@given(_files, st.text(min_size=1, max_size=5))
def test_hash_changes_with_content(files: dict[str, bytes], extra: str) -> None:
    key = next(iter(files))
    changed = {**files, key: files[key] + extra.encode()}
    assert content_hash(changed) != content_hash(files)


def test_hash_separates_path_from_content() -> None:
    assert content_hash({"a": b"bc"}) != content_hash({"ab": b"c"})
    assert content_hash({"a": b"", "b": b"x"}) != content_hash({"a": b"x", "b": b""})


# ---- routes ------------------------------------------------------------------------


def _plain(doc: dict[str, object]) -> dict[str, object]:
    """Drops timestamps: SQLite (local check) returns them without a zone, Postgres with one."""
    return {k: v for k, v in doc.items() if not k.endswith("_at")}


FILES = {"main.neuron": "module m for host\n", "lib/util.neuron": "module u for host\n"}


async def _project(client: AsyncClient, name: str = "demo") -> dict[str, object]:
    response = await client.post("/v1/projects", json={"name": name})
    assert response.status_code == 201, response.text
    return response.json()


@needs_db
async def test_create_get_and_list_projects(client: AsyncClient) -> None:
    created = await _project(client, "  first  ")
    assert created["name"] == "first"
    assert created["row_version"] == 1
    got = await client.get(f"/v1/projects/{created['id']}")
    assert _plain(got.json()) == _plain(created)
    for n in range(2):
        await _project(client, f"p{n}")
    page = (await client.get("/v1/projects", params={"limit": 2})).json()
    assert page["total"] == 3
    assert len(page["items"]) == 2
    rest = (await client.get("/v1/projects", params={"limit": 2, "offset": 2})).json()
    assert len(rest["items"]) == 1
    seen = {p["id"] for p in page["items"]} | {p["id"] for p in rest["items"]}
    assert len(seen) == 3


@needs_db
async def test_bad_input_and_unknown_ids(client: AsyncClient) -> None:
    assert (await client.post("/v1/projects", json={"name": "  "})).status_code == 422
    assert (await client.post("/v1/projects", json={"name": "x", "extra": 1})).status_code == 422
    missing = await client.get(f"/v1/projects/{uuid.uuid4()}")
    assert missing.status_code == 404
    assert missing.json()["code"] == "HTTP_404"
    assert (await client.get(f"/v1/versions/{uuid.uuid4()}")).status_code == 404
    assert (await client.get("/v1/projects", params={"limit": 0})).status_code == 422


@needs_db
async def test_rename_uses_row_version(client: AsyncClient) -> None:
    project = await _project(client)
    url = f"/v1/projects/{project['id']}"
    ok = await client.put(url, json={"name": "renamed", "row_version": 1})
    assert ok.status_code == 200
    assert ok.json()["name"] == "renamed"
    assert ok.json()["row_version"] == 2
    stale = await client.put(url, json={"name": "other", "row_version": 1})
    assert stale.status_code == 409
    assert stale.json()["code"] == "NE1107"
    assert (await client.get(url)).json()["name"] == "renamed"
    unknown = await client.put(f"/v1/projects/{uuid.uuid4()}", json={"name": "x", "row_version": 1})
    assert unknown.status_code == 404


@needs_db
async def test_versions_are_numbered_hashed_and_complete(client: AsyncClient) -> None:
    project = await _project(client)
    base = f"/v1/projects/{project['id']}/versions"
    v1 = await client.post(base, json={"files": FILES, "message": "first"})
    assert v1.status_code == 201, v1.text
    body = v1.json()
    assert body["number"] == 1
    assert body["file_count"] == 2
    assert body["total_bytes"] == sum(len(t.encode()) for t in FILES.values())
    assert [f["path"] for f in body["files"]] == ["lib/util.neuron", "main.neuron"]
    assert body["content_hash"] == content_hash({k: v.encode() for k, v in FILES.items()})
    reordered = dict(reversed(list(FILES.items())))
    v2 = (await client.post(base, json={"files": reordered})).json()
    assert v2["number"] == 2
    assert v2["content_hash"] == body["content_hash"]
    fetched = (await client.get(f"/v1/versions/{body['id']}")).json()
    assert _plain(fetched) == _plain(body)
    listing = (await client.get(base)).json()
    assert listing["total"] == 2
    assert [v["number"] for v in listing["items"]] == [2, 1]
    assert "files" not in listing["items"][0]
    assert listing["items"][1]["file_count"] == 2


@needs_db
async def test_version_rejects_bad_files_with_ne1101(client: AsyncClient) -> None:
    project = await _project(client)
    base = f"/v1/projects/{project['id']}/versions"
    for files in ({}, {"../x.neuron": "a"}, {"/abs": "a"}, {"A.neuron": "a", "a.neuron": "b"}):
        response = await client.post(base, json={"files": files})
        assert response.status_code == 422, files
        assert response.json()["code"] == "NE1101"
    big = {"big.neuron": "x" * (200 * 1024 + 1)}
    assert (await client.post(base, json={"files": big})).json()["code"] == "NE1101"
    assert (await client.get(base)).json()["total"] == 0
    unknown = await client.post(f"/v1/projects/{uuid.uuid4()}/versions", json={"files": FILES})
    assert unknown.status_code == 404


@needs_db
@pytest.mark.parametrize("method", ["PUT", "PATCH", "DELETE", "POST"])
async def test_versions_cannot_be_modified(client: AsyncClient, method: str) -> None:
    project = await _project(client)
    version = (
        await client.post(f"/v1/projects/{project['id']}/versions", json={"files": FILES})
    ).json()
    response = await client.request(
        method, f"/v1/versions/{version['id']}", json={"files": {"a.neuron": "x"}}
    )
    assert response.status_code == 405
    again = (await client.get(f"/v1/versions/{version['id']}")).json()
    assert _plain(again) == _plain(version)
