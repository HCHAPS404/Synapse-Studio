"""Device catalog (E2.6). Reads the real `devices/` manifests; needs no database."""

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from synapse_core.devices.catalog import (
    _REPO_ROOT,
    CatalogError,
    devices_root,
    get_catalog,
    load_device,
    load_medium,
)
from synapse_core.devices.models import Kind
from synapse_core.main import create_app
from synapse_core.settings import Settings, get_settings

DEVICES = _REPO_ROOT / "devices"


def _client(settings: Settings | None = None, *, as_viewer: bool = False) -> TestClient:
    app = create_app()
    app.dependency_overrides[get_settings] = lambda: settings or Settings(database_url=None)
    if as_viewer:
        # The device list/read routes need `device.read` (E3.2); inject a viewer principal
        # so these catalog-shape tests do not need a database or a real login.
        import uuid

        from synapse_core.api.deps import optional_principal
        from synapse_core.services.users import AuthenticatedUser

        app.dependency_overrides[optional_principal] = lambda: AuthenticatedUser(
            user_id=uuid.uuid4(), role="viewer"
        )
    return TestClient(app)


def test_default_devices_dir_is_the_repository_folder() -> None:
    assert devices_root(Settings()) == DEVICES
    assert (DEVICES / "controllers" / "arduino_uno_r3.toml").exists()


def _device_files() -> list[Path]:
    return [p for k in Kind for p in sorted((DEVICES / k.folder).glob("*.toml"))]


def _media_files() -> list[Path]:
    return sorted((DEVICES / "media").glob("*.toml"))


@pytest.mark.parametrize("path", _device_files(), ids=lambda p: p.stem)
def test_every_device_manifest_loads(path: Path) -> None:
    manifest = load_device(path)
    assert manifest.name == path.stem
    assert manifest.kind.folder == path.parent.name
    assert manifest.safe_state_time
    if manifest.kind is Kind.ROBOT:
        assert manifest.robot is not None and manifest.robot.name == manifest.name


@pytest.mark.parametrize("path", _media_files(), ids=lambda p: p.stem)
def test_every_medium_manifest_loads(path: Path) -> None:
    medium = load_medium(path)
    assert medium.name == path.stem
    assert medium.speaks


def test_catalog_has_every_file() -> None:
    catalog = get_catalog(Settings())
    assert len(catalog.devices) == len(_device_files())
    assert len(catalog.media) == len(_media_files())
    assert [d.name for d in catalog.devices] == sorted(d.name for d in catalog.devices)
    arduino = next(d for d in catalog.devices if d.name == "arduino_uno_r3")
    assert arduino.kind is Kind.MCU
    assert arduino.backend.value == "c"
    assert arduino.io["gpio"].volts == 5.0
    assert arduino.timing["zero_hop"].transport == "10 us"
    assert arduino.scan_min == "1 ms"
    scara = next(d for d in catalog.devices if d.name == "generic_scara")
    assert scara.robot is not None
    assert len(scara.robot.joint_limits_deg) == 4
    assert scara.timing["modbus_tcp"].transport == "3 ms"


# ---- routes ------------------------------------------------------------------------


def test_catalog_route() -> None:
    body = _client().get("/v1/devices/catalog").json()
    assert len(body["devices"]) == len(_device_files())
    assert len(body["media"]) == len(_media_files())
    assert {m["name"] for m in body["media"]} >= {"modbus_tcp", "s7", "zero_hop"}


def test_list_and_get_device_routes() -> None:
    client = _client(as_viewer=True)
    items = client.get("/v1/devices").json()["items"]
    assert len(items) == len(_device_files())
    assert "io" not in items[0]  # summary only
    one = client.get("/v1/devices/generic_s7_1200")
    assert one.status_code == 200
    assert one.json()["io"]["digital_in"]["prefix"] == "%I"
    assert client.get("/v1/devices/does_not_exist").status_code == 404


def test_devices_dir_setting_overrides_the_default(tmp_path: Path) -> None:
    (tmp_path / "plcs").mkdir()
    (tmp_path / "plcs" / "demo_plc.toml").write_text(
        '[device]\nname="demo_plc"\nkind="plc"\ndialects=["plc"]\nbackend="st"\n'
        'trust="generic"\nnote="x"\n[timing]\nscan_min="1 ms"\n[safety]\nsafe_state_time="1 ms"\n',
        encoding="utf-8",
    )
    body = (
        _client(Settings(database_url=None, devices_dir=str(tmp_path)))
        .get("/v1/devices/catalog")
        .json()
    )
    assert [d["name"] for d in body["devices"]] == ["demo_plc"]
    assert body["media"] == []


# ---- validation --------------------------------------------------------------------


def test_bad_manifests_are_rejected(tmp_path: Path) -> None:
    plcs = tmp_path / "plcs"
    plcs.mkdir()

    def write(name: str, text: str) -> Path:
        path = plcs / name
        path.write_text(text, encoding="utf-8")
        return path

    def head(name: str) -> str:
        return (
            f'[device]\nname="{name}"\nkind="plc"\ndialects=["plc"]\n'
            'backend="st"\ntrust="generic"\nnote="x"\n'
        )

    safety = '[safety]\nsafe_state_time="1 ms"\n'
    with pytest.raises(CatalogError):  # no [safety]
        load_device(write("a.toml", head("a") + "[timing]\n"))
    with pytest.raises(CatalogError):  # name does not match the file stem
        load_device(write("b.toml", head("c") + "[timing]\n" + safety))
    with pytest.raises(CatalogError):  # unknown field
        load_device(write("d.toml", head("d") + "extra=1\n[timing]\n" + safety))
