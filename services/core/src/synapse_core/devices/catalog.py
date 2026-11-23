"""Load the device catalog from the `devices/` TOML manifests with `tomllib`.

The directory is found through the `devices_dir` setting; its default is the repository's
`devices/` resolved relative to this package, so the service works whatever the process
working directory is. The E2.10 Docker image must copy `devices/` into the image (its
`rootDir` on Render is `services/core`, which does not contain it).
"""

import logging
import tomllib
from functools import lru_cache
from pathlib import Path

from pydantic import ValidationError

from synapse_core.devices.models import (
    DeviceCatalog,
    DeviceManifest,
    Family,
    Kind,
    MediumManifest,
    MediumTiming,
)
from synapse_core.settings import Settings

log = logging.getLogger(__name__)

# …/services/core/src/synapse_core/devices/catalog.py -> the repository root is six up.
_REPO_ROOT = Path(__file__).resolve().parents[5]
_DEFAULT_DEVICES = _REPO_ROOT / "devices"


class CatalogError(RuntimeError):
    """A manifest under `devices/` is missing or does not parse."""


def devices_root(settings: Settings) -> Path:
    """The directory to read manifests from."""
    return Path(settings.devices_dir) if settings.devices_dir else _DEFAULT_DEVICES


def _device_from_toml(path: Path, raw: dict[str, object]) -> DeviceManifest:
    """Turns the TOML tables of a device manifest into the flat model."""
    if "device" not in raw:
        raise CatalogError(f"{path.name}: no [device] table")
    device = dict(raw["device"])  # type: ignore[arg-type]
    timing = dict(raw.get("timing", {}))  # type: ignore[arg-type]
    scan_min = timing.pop("scan_min", None)
    media: dict[str, MediumTiming] = {}
    for medium, value in timing.items():
        if not isinstance(value, dict):
            raise CatalogError(f"{path.name}: timing.{medium} is not a table")
        media[medium] = MediumTiming.model_validate(value)
    safety = raw.get("safety")
    if not isinstance(safety, dict) or "safe_state_time" not in safety:
        raise CatalogError(f"{path.name}: [safety].safe_state_time is required")
    return DeviceManifest.model_validate(
        {
            **device,
            "io": raw.get("io", {}),
            "scan_min": scan_min,
            "timing": media,
            "safe_state_time": safety["safe_state_time"],
            "robot": raw.get("robot"),
        }
    )


def load_device(path: Path) -> DeviceManifest:
    """Reads and validates one device manifest (`CatalogError` on any problem)."""
    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise CatalogError(f"{path.name}: {exc}") from exc
    try:
        manifest = _device_from_toml(path, raw)
    except ValidationError as exc:
        raise CatalogError(f"{path.name}: {exc.errors(include_url=False)}") from exc
    if manifest.name != path.stem:
        raise CatalogError(f"{path.name}: name `{manifest.name}` does not match the file stem")
    if manifest.kind.folder != path.parent.name:
        raise CatalogError(
            f"{path.name}: a {manifest.kind.value} device belongs in `{manifest.kind.folder}/`"
        )
    return manifest


def load_medium(path: Path) -> MediumManifest:
    """Reads and validates one medium manifest (`CatalogError` on any problem)."""
    try:
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise CatalogError(f"{path.name}: {exc}") from exc
    if "medium" not in raw:
        raise CatalogError(f"{path.name}: no [medium] table")
    try:
        medium = MediumManifest.model_validate(raw["medium"])
    except ValidationError as exc:
        raise CatalogError(f"{path.name}: {exc.errors(include_url=False)}") from exc
    if medium.name != path.stem:
        raise CatalogError(f"{path.name}: name `{medium.name}` does not match the file stem")
    if set(medium.isolating) - set(medium.options):
        raise CatalogError(f"{path.name}: `isolating` names must be listed in `options`")
    return medium


def load_catalog(root: Path) -> DeviceCatalog:
    """Loads every manifest under `root` (device folders and `media/`)."""
    if not root.is_dir():
        raise CatalogError(f"the devices directory does not exist: {root}")
    devices: list[DeviceManifest] = []
    for kind in Kind:
        folder = root / kind.folder
        if folder.is_dir():
            devices.extend(load_device(p) for p in sorted(folder.glob("*.toml")))
    media = [load_medium(p) for p in sorted((root / "media").glob("*.toml"))]
    devices.sort(key=lambda d: d.name)
    media.sort(key=lambda m: m.name)
    return DeviceCatalog(devices=devices, media=media)


@lru_cache(maxsize=4)
def _cached(root: str) -> DeviceCatalog:
    return load_catalog(Path(root))


def get_catalog(settings: Settings) -> DeviceCatalog:
    """The catalog for the configured directory, loaded once and cached per directory."""
    return _cached(str(devices_root(settings)))


__all__ = [
    "CatalogError",
    "Family",
    "devices_root",
    "get_catalog",
    "load_catalog",
    "load_device",
    "load_medium",
]
