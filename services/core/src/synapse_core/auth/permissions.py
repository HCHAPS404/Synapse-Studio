"""The single permission matrix used by REST (E3.2) and, later, the relay (E4.1).

One dict `role -> frozenset[Permission]`. `viewer`, `operator` and `engineer` form a chain
(each a superset of the one before); `guest` is separate and holds no device, daemon or
real-device-telemetry permission; `daemon` and `twin` are service roles. The matrix is the
table in `.kiro/specs/core-services/design.md`; the Hypothesis properties in the tests
check its monotonicity and the guest's isolation so the two cannot drift apart.
"""

from __future__ import annotations

from enum import StrEnum


class Permission(StrEnum):
    """Every permission a route or relay message can require."""

    SANDBOX_WRITE = "sandbox.write"
    BUILD_RUN = "build.run"
    PROJECT_READ = "project.read"
    PROJECT_WRITE = "project.write"
    DEVICE_READ = "device.read"
    TELEMETRY_READ = "telemetry.read"
    SIM_CONTROL = "sim.control"
    ESTOP_ADVISE = "estop.advise"
    DEVICE_COMMAND = "device.command"
    DEPLOY_REQUEST = "deploy.request"
    DEPLOY_CONFIRM = "deploy.confirm"
    PAIRING_ISSUE = "pairing.issue"
    USER_MANAGE = "user.manage"
    AUDIT_READ = "audit.read"
    TELEMETRY_PUBLISH = "telemetry.publish"
    DEPLOYMENT_REPORT = "deployment.report"
    SIM_PUBLISH = "sim.publish"


# Roles. The user roles form the chain viewer < operator < engineer.
ROLE_GUEST = "guest"
ROLE_VIEWER = "viewer"
ROLE_OPERATOR = "operator"
ROLE_ENGINEER = "engineer"
ROLE_DAEMON = "daemon"
ROLE_TWIN = "twin"

USER_ROLE_CHAIN: tuple[str, ...] = (ROLE_VIEWER, ROLE_OPERATOR, ROLE_ENGINEER)

P = Permission

# Build the chain by accumulation so monotonicity holds by construction; the tests still
# assert it, so a hand edit that breaks the chain is caught.
_VIEWER: frozenset[Permission] = frozenset({P.PROJECT_READ, P.DEVICE_READ, P.TELEMETRY_READ})
_OPERATOR: frozenset[Permission] = _VIEWER | {
    P.PROJECT_WRITE,
    P.BUILD_RUN,
    P.SIM_CONTROL,
    P.ESTOP_ADVISE,
    P.DEVICE_COMMAND,
    P.DEPLOY_CONFIRM,
}
_ENGINEER: frozenset[Permission] = _OPERATOR | {
    P.DEPLOY_REQUEST,
    P.PAIRING_ISSUE,
    P.USER_MANAGE,
    P.AUDIT_READ,
}

# A guest acts only inside its own sandbox: it may write and build there, and (per the
# design) read telemetry of virtual devices. It holds no device, daemon or real-device
# permission; guest isolation is a tested property.
_GUEST: frozenset[Permission] = frozenset({P.SANDBOX_WRITE, P.BUILD_RUN})

_DAEMON: frozenset[Permission] = frozenset({P.TELEMETRY_PUBLISH, P.DEPLOYMENT_REPORT})
_TWIN: frozenset[Permission] = frozenset({P.SIM_PUBLISH})

ROLE_PERMISSIONS: dict[str, frozenset[Permission]] = {
    ROLE_GUEST: _GUEST,
    ROLE_VIEWER: _VIEWER,
    ROLE_OPERATOR: _OPERATOR,
    ROLE_ENGINEER: _ENGINEER,
    ROLE_DAEMON: _DAEMON,
    ROLE_TWIN: _TWIN,
}

# Permissions that touch a real device, a daemon, or a real-device telemetry stream. A
# guest must hold none of these (the guest-isolation property checks this set).
DEVICE_PERMISSIONS: frozenset[Permission] = frozenset(
    {
        P.DEVICE_READ,
        P.DEVICE_COMMAND,
        P.SIM_CONTROL,
        P.ESTOP_ADVISE,
        P.TELEMETRY_READ,
        P.DEPLOY_REQUEST,
        P.DEPLOY_CONFIRM,
        P.PAIRING_ISSUE,
        P.TELEMETRY_PUBLISH,
        P.DEPLOYMENT_REPORT,
    }
)


def has_permission(role: str, permission: Permission) -> bool:
    """Whether `role` holds `permission`; an unknown role holds nothing."""
    return permission in ROLE_PERMISSIONS.get(role, frozenset())
