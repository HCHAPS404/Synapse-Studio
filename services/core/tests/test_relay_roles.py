"""E4.1: `ROLE_MESSAGES` must equal the role table in `spec/protocols/relay.md`.

The test parses the markdown "Roles and what each may send" table and compares it with the
code, so the two cannot drift (E-R7: "the relay table equals `relay.md`"). The markdown
groups roles ("`guest`, `viewer`") and uses the phrase "the above"/"the common ones" to
inherit; the parser expands both exactly as the table text means.
"""

from __future__ import annotations

import re
from pathlib import Path

from synapse_core.relay.roles import ROLE_MESSAGES

_RELAY_MD = Path(__file__).resolve().parents[3] / "spec" / "protocols" / "relay.md"
_COMMON = {"hello", "ping", "pong", "subscribe", "unsubscribe"}


def _parse_role_table() -> dict[str, frozenset[str]]:
    text = _RELAY_MD.read_text(encoding="utf-8")
    heading = "## Roles and what each may send"
    start = text.index(heading)
    section = text[start:]
    rows: dict[str, frozenset[str]] = {}
    for line in section.splitlines():
        line = line.strip()
        if not line.startswith("|") or line.startswith("| Role") or set(line) <= {"|", "-", " "}:
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if len(cells) != 2:
            continue
        roles_cell, may_cell = cells
        roles = re.findall(r"`([a-z]+)`", roles_cell)
        if not roles:
            continue
        messages = set(re.findall(r"`([a-z_]+)`", may_cell))
        if "the above" in may_cell or "the common ones" in may_cell:
            messages |= _COMMON
        for role in roles:
            rows[role] = frozenset(messages)
    return rows


def test_role_messages_equals_relay_markdown() -> None:
    parsed = _parse_role_table()
    # The `operator` row says "the above, plus …"; "the above" is the guest/viewer row,
    # which is exactly the common set, so the parser's `_COMMON` union matches.
    assert set(parsed) == set(ROLE_MESSAGES), "roles in the table and the code differ"
    for role, messages in parsed.items():
        assert messages == ROLE_MESSAGES[role], f"messages for {role} differ from relay.md"


def test_common_messages_in_every_role() -> None:
    for role, messages in ROLE_MESSAGES.items():
        assert _COMMON <= messages, f"{role} is missing a common message"
