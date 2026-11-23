"""The deployment state machine, enforced again on the daemon (E5.4).

The API owns the authoritative machine (`synapse_core.deploy`); the daemon enforces the same
rules so a daemon never applies a package that did not pass `verified`, `dry_run_ok` and both
confirmations in order. The two transition tables are identical and are kept in step by a
shared vector asserted in both test suites. An illegal transition is `NE1110`.

The software E-STOP is supplementary. A hardwired E-STOP is the authority. Nothing here
claims IEC 61508 or ISO 13849.
"""

from __future__ import annotations

from enum import StrEnum

from synapse_daemon.errors import DaemonError, ErrorCode


class DeployState(StrEnum):
    """States of a deployment on the daemon. The last three are terminal."""

    REQUESTED = "requested"
    VERIFIED = "verified"
    DRY_RUN_OK = "dry_run_ok"
    CONFIRMED_1 = "confirmed_1"
    CONFIRMED_2 = "confirmed_2"
    APPLIED = "applied"
    REJECTED = "rejected"
    FAILED = "failed"
    ABORTED = "aborted"


class DeployEvent(StrEnum):
    VERIFY = "verify"
    DRY_RUN = "dry_run"
    CONFIRM_1 = "confirm_1"
    CONFIRM_2 = "confirm_2"
    APPLY = "apply"
    REJECT = "reject"
    FAIL = "fail"
    ABORT = "abort"


TERMINAL_STATES: frozenset[DeployState] = frozenset(
    {DeployState.APPLIED, DeployState.REJECTED, DeployState.FAILED, DeployState.ABORTED}
)

_HAPPY: dict[DeployState, tuple[DeployEvent, DeployState]] = {
    DeployState.REQUESTED: (DeployEvent.VERIFY, DeployState.VERIFIED),
    DeployState.VERIFIED: (DeployEvent.DRY_RUN, DeployState.DRY_RUN_OK),
    DeployState.DRY_RUN_OK: (DeployEvent.CONFIRM_1, DeployState.CONFIRMED_1),
    DeployState.CONFIRMED_1: (DeployEvent.CONFIRM_2, DeployState.CONFIRMED_2),
    DeployState.CONFIRMED_2: (DeployEvent.APPLY, DeployState.APPLIED),
}


def allowed_events(state: DeployState) -> frozenset[DeployEvent]:
    """Every event that is legal from `state`. A terminal state allows none."""
    if state in TERMINAL_STATES:
        return frozenset()
    events: set[DeployEvent] = {DeployEvent.FAIL, DeployEvent.ABORT}
    happy = _HAPPY.get(state)
    if happy is not None:
        events.add(happy[0])
    if state == DeployState.REQUESTED:
        events.add(DeployEvent.REJECT)
    return frozenset(events)


def next_state(state: DeployState, event: DeployEvent) -> DeployState:
    """Apply one event. Raises `DaemonError(NE1110)` for an illegal transition."""
    if event not in allowed_events(state):
        raise DaemonError(
            ErrorCode.DEPLOYMENT_BLOCKED,
            f"deployment event {event.value!r} is not allowed from state {state.value!r}",
        )
    if event == DeployEvent.REJECT:
        return DeployState.REJECTED
    if event == DeployEvent.FAIL:
        return DeployState.FAILED
    if event == DeployEvent.ABORT:
        return DeployState.ABORTED
    return _HAPPY[state][1]


def is_terminal(state: DeployState) -> bool:
    """Whether the state admits no further event."""
    return state in TERMINAL_STATES
