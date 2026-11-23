"""Permission matrix (E3.2): the monotonic user-role chain and guest isolation.

The design's correctness criteria are the authoritative statement:
- the user roles form a chain viewer ⊆ operator ⊆ engineer (monotonicity);
- the guest holds none of the device / daemon / real-telemetry permissions (isolation).

The guest is *not* a subset of viewer and viewer is not a subset of the guest: a guest
holds `sandbox.write` (and `build.run`) that viewer lacks, and viewer holds `project.read`,
`device.read`, `telemetry.read` that the guest lacks. The chain is over the user roles only,
exactly as the design says, and the guest's isolation is the separate property.
"""

from hypothesis import given
from hypothesis import strategies as st

from synapse_core.auth.permissions import (
    DEVICE_PERMISSIONS,
    ROLE_ENGINEER,
    ROLE_GUEST,
    ROLE_OPERATOR,
    ROLE_PERMISSIONS,
    ROLE_VIEWER,
    USER_ROLE_CHAIN,
    Permission,
    has_permission,
)

_ALL_PERMISSIONS = list(Permission)


def test_the_user_roles_form_a_superset_chain() -> None:
    viewer = ROLE_PERMISSIONS[ROLE_VIEWER]
    operator = ROLE_PERMISSIONS[ROLE_OPERATOR]
    engineer = ROLE_PERMISSIONS[ROLE_ENGINEER]
    assert viewer < operator < engineer  # strict supersets, so no role is redundant


@given(st.sampled_from(_ALL_PERMISSIONS))
def test_monotonic_over_the_chain(permission: Permission) -> None:
    """For every permission: if a role in the chain holds it, so does every later role."""
    held = [has_permission(role, permission) for role in USER_ROLE_CHAIN]
    # Once True, it stays True down the chain (viewer -> operator -> engineer).
    assert held == sorted(held, key=lambda v: v)  # False (0) before True (1)
    for earlier in range(len(held)):
        if held[earlier]:
            assert all(held[earlier:])


def test_a_guest_holds_no_device_permission() -> None:
    """Guest isolation: the guest touches no device, daemon or real-telemetry permission."""
    guest = ROLE_PERMISSIONS[ROLE_GUEST]
    assert guest.isdisjoint(DEVICE_PERMISSIONS)


@given(st.sampled_from(sorted(DEVICE_PERMISSIONS)))
def test_a_guest_is_denied_every_device_permission(permission: Permission) -> None:
    assert has_permission(ROLE_GUEST, permission) is False


def test_the_guest_only_acts_in_its_own_sandbox() -> None:
    guest = ROLE_PERMISSIONS[ROLE_GUEST]
    assert Permission.SANDBOX_WRITE in guest
    assert Permission.BUILD_RUN in guest
    # No project.read / project.write: the guest reaches its own projects through sandbox
    # scoping in the service layer, not a global project permission.
    assert Permission.PROJECT_READ not in guest
    assert Permission.PROJECT_WRITE not in guest


def test_an_unknown_role_holds_nothing() -> None:
    for permission in _ALL_PERMISSIONS:
        assert has_permission("root", permission) is False
        assert has_permission("", permission) is False
