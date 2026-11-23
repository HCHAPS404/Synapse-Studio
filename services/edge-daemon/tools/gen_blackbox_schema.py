"""Regenerate `spec/schemas/blackbox.schema.json` from the daemon Pydantic model (E6.1).

Run from `services/edge-daemon`:

    uv run python tools/gen_blackbox_schema.py

The canonical text comes from `synapse_daemon.blackbox.canonical_schema_text` (sorted keys,
two-space indent) so it is byte-stable; the test
`test_e6_1_blackbox.py::test_committed_schema_matches_model` fails when the committed file
drifts from the model.
"""

from __future__ import annotations

from pathlib import Path

from synapse_daemon.blackbox import canonical_schema_text

SCHEMA_PATH = Path(__file__).resolve().parents[3] / "spec" / "schemas" / "blackbox.schema.json"


def main() -> None:
    SCHEMA_PATH.write_text(canonical_schema_text(), encoding="utf-8")
    print(f"wrote {SCHEMA_PATH}")


if __name__ == "__main__":
    main()
