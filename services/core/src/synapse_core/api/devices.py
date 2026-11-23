"""Read-only device catalog routes. Public (the catalog has no secrets); a guest may
read it, but no other `/v1/devices/*` route (E-R3, enforced in E3.2)."""

from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException

from synapse_core.devices.catalog import get_catalog
from synapse_core.devices.models import DeviceCatalog, DeviceList, DeviceManifest, DeviceSummary
from synapse_core.settings import Settings, get_settings

router = APIRouter(prefix="/v1/devices", tags=["devices"])

SettingsDep = Annotated[Settings, Depends(get_settings)]


@router.get("/catalog", response_model=DeviceCatalog)
async def device_catalog(settings: SettingsDep) -> DeviceCatalog:
    """The whole catalog: every device and medium manifest."""
    return get_catalog(settings)


@router.get("", response_model=DeviceList)
async def list_devices(settings: SettingsDep) -> DeviceList:
    """Device summaries only."""
    catalog = get_catalog(settings)
    return DeviceList(items=[DeviceSummary.model_validate(d.model_dump()) for d in catalog.devices])


@router.get("/{name}", response_model=DeviceManifest)
async def get_device(name: str, settings: SettingsDep) -> DeviceManifest:
    """One device manifest by name, or a 404."""
    for device in get_catalog(settings).devices:
        if device.name == name:
            return device
    raise HTTPException(status_code=404, detail="device not found")
