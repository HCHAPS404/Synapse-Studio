"""Network self-test and its use by the runner and /health. Fake `unshare`, no privileges."""

import json
import sys
from dataclasses import replace
from pathlib import Path

from fastapi.testclient import TestClient

from synapse_core.compile.models import CompileCommand, CompileJob
from synapse_core.compile.runner import CompileRunner, RunnerConfig
from synapse_core.compile.sandbox import Limits, NetworkSandbox, detect_network_sandbox
from synapse_core.main import create_app

HERE = Path(__file__).parent
PY = sys.executable
FAKE_UNSHARE = str(HERE / "fake_unshare.py")
FAKE_NEURON = str(HERE / "fake_neuron.py")

# The probe of the tests reads the marker the fake sets instead of /proc.
PROBE = (
    "import json, os\nprint(json.dumps({'ns': os.environ.get('FAKE_NS', 'host'), 'if': ['lo']}))\n"
)
PROBE_WITH_NIC = (
    "import json, os\n"
    "print(json.dumps({'ns': os.environ.get('FAKE_NS', 'host'), 'if': ['lo', 'eth0']}))\n"
)


async def test_no_candidate_means_unverified() -> None:
    result = await detect_network_sandbox(candidates=[], probe_code=PROBE)
    assert result.status == "unverified"
    assert result.prefix == ()


async def test_a_namespace_that_differs_is_isolated() -> None:
    prefix = (PY, FAKE_UNSHARE, "--")
    result = await detect_network_sandbox(candidates=[prefix], probe_code=PROBE)
    assert result.status == "isolated"
    assert result.prefix == prefix


async def test_the_same_namespace_is_not_isolated() -> None:
    prefix = (PY, FAKE_UNSHARE, "--shared", "--")
    result = await detect_network_sandbox(candidates=[prefix], probe_code=PROBE)
    assert result.status == "unverified"
    assert result.prefix == ()


async def test_a_namespace_with_a_network_card_is_not_isolated() -> None:
    result = await detect_network_sandbox(
        candidates=[(PY, FAKE_UNSHARE, "--")], probe_code=PROBE_WITH_NIC
    )
    assert result.status == "unverified"


async def test_a_failing_tool_falls_through_to_the_next_candidate() -> None:
    failing = (PY, FAKE_UNSHARE, "--fail")
    working = (PY, FAKE_UNSHARE, "--")
    only_failing = await detect_network_sandbox(candidates=[failing], probe_code=PROBE)
    assert only_failing.status == "unverified"
    both = await detect_network_sandbox(candidates=[failing, working], probe_code=PROBE)
    assert both.status == "isolated"
    assert both.prefix == working


async def test_a_missing_tool_is_unverified() -> None:
    result = await detect_network_sandbox(
        candidates=[("/no/such/unshare", "--net")], probe_code=PROBE
    )
    assert result.status == "unverified"


async def test_the_default_probe_never_claims_more_than_it_proves() -> None:
    result = await detect_network_sandbox()
    assert result.status in ("isolated", "unverified")
    if result.status == "unverified":
        assert result.prefix == ()


async def test_the_runner_puts_the_prefix_before_the_compiler(tmp_path: Path) -> None:
    network = NetworkSandbox("isolated", (PY, FAKE_UNSHARE, "--"))
    config = RunnerConfig(
        neuron_cmd=(PY, FAKE_NEURON),
        limits=Limits(timeout_s=20),
        network=network,
        work_root=tmp_path,
    )
    files = {"a.neuron": "x", "mode.txt": "argv"}
    outcome = await CompileRunner(config).run(CompileJob(command=CompileCommand.BUILD, files=files))
    assert json.loads(outcome.files["argv.json"])["fake_ns"] == "isolated"
    plain = replace(config, network=NetworkSandbox("unverified"))
    outcome = await CompileRunner(plain).run(CompileJob(command=CompileCommand.BUILD, files=files))
    assert json.loads(outcome.files["argv.json"])["fake_ns"] is None


def test_health_reports_the_sandbox_status() -> None:
    client = TestClient(create_app())
    body = client.get("/health").json()
    assert body["sandbox"]["network"] == "unverified"  # no start-up self-test ran yet
    assert body["status"] == "ok"
    with TestClient(create_app()) as started:
        body = started.get("/health").json()
    assert body["sandbox"]["network"] in ("isolated", "unverified")
    assert set(body) >= {"status", "version", "environment", "database"}
