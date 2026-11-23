"""The deployment state machine (E5.4), enforced in the API and again in the daemon.

Safe commissioning (E-R14): a deployment reaches `applied` only through, in order,
`verified`, `dry_run_ok`, `confirmed_1`, `confirmed_2`, each with its precondition; any
out-of-order step is `NE1110`. The machine also has the terminal failures `rejected`,
`failed` and `aborted`. This module is a pure transition function with no I/O, so the same
rules run in the API (`synapse_core`) and in the daemon (`synapse_daemon.deploy`), and a
Hypothesis property can drive it over random event sequences.

The software E-STOP is supplementary. A hardwired E-STOP is the authority. Nothing here
claims IEC 61508 or ISO 13849.
"""

from __future__ import annotations

from enum import StrEnum

from synapse_core.errors import ErrorCode, ServiceError


class DeployState(StrEnum):
    """States of a deployment. The last three are terminal."""

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
    """Events that drive a deployment forward or into a terminal state."""

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

# The happy path, each step gated on the previous state. `reject` is only before work starts;
# `fail` and `abort` may end any non-terminal state. No event leaves a terminal state.
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
    """Apply one event. Raises `ServiceError(NE1110)` for an illegal transition."""
    if event not in allowed_events(state):
        raise ServiceError(
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
