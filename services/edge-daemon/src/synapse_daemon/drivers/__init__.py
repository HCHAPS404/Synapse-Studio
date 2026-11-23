"""Device drivers behind one `Driver` interface.

Every driver maps the register and frame layout the cell compiler (D3) emits, is configured
with timeouts/retries/backoff, and on failure sets its tags to `bad` quality rather than a
guessed value. Each driver is tested against a simulated peer (loopback, in-process server
or virtual bus), never real hardware.
"""

from synapse_daemon.drivers.base import (
    Driver,
    DriverConfig,
    DriverError,
    Quality,
    Sample,
    Value,
)
from synapse_daemon.drivers.simulated import SimulatedDriver

__all__ = [
    "Driver",
    "DriverConfig",
    "DriverError",
    "Quality",
    "Sample",
    "SimulatedDriver",
    "Value",
]
