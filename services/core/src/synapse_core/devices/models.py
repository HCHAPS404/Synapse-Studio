"""Pydantic models of a device or medium manifest.

They mirror `crates/neuron-cell/src/manifest.rs`: the fields of `[device]`, `[io]`,
`[timing]`, `[safety]` and the optional `[robot]` table, and the `[medium]` table of
`devices/media/*.toml`. Durations and rates stay as the strings the manifest holds (for
example `"1 ms"`), because this catalog is read-only and descriptive; the Rust side owns
the authoritative numeric parsing used by `neuron cell check`.
"""

from enum import StrEnum
from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field

Name = Annotated[str, Field(min_length=1)]


class Kind(StrEnum):
    """Device class; also the folder name under `devices/` (via `folder`)."""

    PLC = "plc"
    MCU = "mcu"
    FPGA = "fpga"
    ROBOT = "robot"
    HOST = "host"

    @property
    def folder(self) -> str:
        return {
            Kind.PLC: "plcs",
            Kind.MCU: "controllers",
            Kind.FPGA: "fpgas",
            Kind.ROBOT: "robots",
            Kind.HOST: "hosts",
        }[self]


class Backend(StrEnum):
    """The emitter or backend that builds a device's program."""

    ST = "st"
    SCL = "scl"
    PLCOPEN = "plcopen"
    SPEL = "spel"
    PYTHON = "python"
    C = "c"
    NATIVE = "native"
    VERILOG = "verilog"


class Trust(StrEnum):
    """How far a manifest's numbers can be believed."""

    GENERIC = "generic"
    MEASURED = "measured"
    CONFIRMED = "confirmed"


class Family(StrEnum):
    """How a medium carries values."""

    REGISTER = "register"
    FRAME = "frame"
    PUBSUB = "pubsub"
    ZERO_HOP = "zero_hop"


class IoFamily(BaseModel):
    """One address family of a device (`%I`, pins)."""

    model_config = ConfigDict(extra="forbid")
    prefix: Name
    volts: float | None = None
    kind: str | None = None
    max_ma: int | None = None


class MediumTiming(BaseModel):
    """A device's timing on one medium (strings as written in the manifest)."""

    model_config = ConfigDict(extra="forbid")
    transport: str
    jitter: str


class RobotLimits(BaseModel):
    """The `[robot]` table (robots only). Mirrors `neuron_emit::robot::Robot`."""

    model_config = ConfigDict(extra="forbid")
    name: Name
    kind: str
    reach_mm: float
    shoulder_height_mm: float
    z_min_mm: float
    z_max_mm: float
    joint_limits_deg: list[list[float]]


class DeviceManifest(BaseModel):
    """A validated device manifest (`devices/<class>/*.toml`)."""

    model_config = ConfigDict(extra="forbid")
    name: Name
    kind: Kind
    dialects: list[str] = Field(min_length=1)
    backend: Backend
    profile: str = "generic"
    trust: Trust
    vendor_class: str = ""
    note: str = ""
    target: str | None = None
    board: str | None = None
    io: dict[str, IoFamily] = Field(default_factory=dict)
    scan_min: str | None = None
    timing: dict[str, MediumTiming] = Field(default_factory=dict)
    safe_state_time: str
    robot: RobotLimits | None = None


class MediumManifest(BaseModel):
    """A validated medium manifest (`devices/media/*.toml`)."""

    model_config = ConfigDict(extra="forbid")
    name: Name
    family: Family
    bitrate: str | None = None
    frame_overhead_bytes: int = 0
    transport_worst: str
    isolated: bool = False
    event_driven: bool = False
    speaks: list[Kind] = Field(min_length=1)
    options: list[str] = Field(default_factory=list)
    isolating: list[str] = Field(default_factory=list)
    trust: Trust
    note: str = ""


class DeviceSummary(BaseModel):
    """A device without its I/O and timing detail (list route)."""

    name: str
    kind: Kind
    backend: Backend
    profile: str
    trust: Trust
    vendor_class: str


class DeviceCatalog(BaseModel):
    """Response of `GET /v1/devices/catalog`."""

    devices: list[DeviceManifest]
    media: list[MediumManifest]


class DeviceList(BaseModel):
    """Response of `GET /v1/devices`: summaries only."""

    items: list[DeviceSummary]
