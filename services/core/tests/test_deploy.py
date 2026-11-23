"""E5.4 (API side): the deployment state machine and its invariants (Hypothesis)."""

from __future__ import annotations

import pytest
from hypothesis import given
from hypothesis import strategies as st

from synapse_core.deploy import (
    TERMINAL_STATES,
    DeployEvent,
    DeployState,
    allowed_events,
    is_terminal,
    next_state,
)
from synapse_core.errors import ErrorCode, ServiceError

_EVENTS = list(DeployEvent)
_HAPPY_PATH = [
    DeployEvent.VERIFY,
    DeployEvent.DRY_RUN,
    DeployEvent.CONFIRM_1,
    DeployEvent.CONFIRM_2,
    DeployEvent.APPLY,
]


def test_transition_table_vector() -> None:
    # The same vector the daemon test asserts: the two machines agree.
    table = {s.value: sorted(e.value for e in allowed_events(s)) for s in DeployState}
    assert table == {
        "requested": ["abort", "fail", "reject", "verify"],
        "verified": ["abort", "dry_run", "fail"],
        "dry_run_ok": ["abort", "confirm_1", "fail"],
        "confirmed_1": ["abort", "confirm_2", "fail"],
        "confirmed_2": ["abort", "apply", "fail"],
        "applied": [],
        "rejected": [],
        "failed": [],
        "aborted": [],
    }


def test_happy_path_reaches_applied() -> None:
    state = DeployState.REQUESTED
    for event in _HAPPY_PATH:
        state = next_state(state, event)
    assert state is DeployState.APPLIED


def test_apply_without_confirmations_is_ne1110() -> None:
    with pytest.raises(ServiceError) as exc:
        next_state(DeployState.DRY_RUN_OK, DeployEvent.APPLY)
    assert exc.value.code is ErrorCode.DEPLOYMENT_BLOCKED


@given(st.lists(st.sampled_from(_EVENTS), max_size=30))
def test_no_illegal_transition_and_terminal_stays_terminal(events: list[DeployEvent]) -> None:
    state = DeployState.REQUESTED
    for event in events:
        if is_terminal(state):
            with pytest.raises(ServiceError):
                next_state(state, event)
            assert state in TERMINAL_STATES
            continue
        if event in allowed_events(state):
            state = next_state(state, event)
        else:
            with pytest.raises(ServiceError):
                next_state(state, event)


@given(st.lists(st.sampled_from(_EVENTS), max_size=30))
def test_applied_only_through_the_ordered_path(events: list[DeployEvent]) -> None:
    state = DeployState.REQUESTED
    taken: list[DeployEvent] = []
    for event in events:
        if not is_terminal(state) and event in allowed_events(state):
            state = next_state(state, event)
            taken.append(event)
    if state is DeployState.APPLIED:
        assert taken == _HAPPY_PATH
