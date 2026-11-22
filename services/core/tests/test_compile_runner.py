"""Compile runner tests. They use a fake `neuron` (tests/fake_neuron.py), never the Rust build."""

import asyncio
import hashlib
import json
import os
import sys
import time
from dataclasses import replace
from pathlib import Path, PurePosixPath

import pytest
from fastapi import HTTPException
from hypothesis import given
from hypothesis import strategies as st

from synapse_core.compile.models import CompileCommand, CompileJob
from synapse_core.compile.paths import InputLimits, check_files, check_relative_path
from synapse_core.compile.runner import CompileRunner, RunnerConfig
from synapse_core.compile.sandbox import IS_POSIX, Limits, NetworkSandbox, build_env
from synapse_core.errors import ErrorCode, ServiceError

HERE = Path(__file__).parent
REPO = HERE.parents[2]
FAKE = str(HERE / "fake_neuron.py")
posix_only = pytest.mark.skipif(not IS_POSIX, reason="needs POSIX process groups and rlimits")


def config(work: Path, **changes: object) -> RunnerConfig:
    base = RunnerConfig(
        neuron_cmd=(sys.executable, FAKE),
        limits=Limits(timeout_s=20),
        work_root=work,
    )
    return replace(base, **changes)  # type: ignore[arg-type]


def job(
    mode: str = "ok", command: CompileCommand = CompileCommand.BUILD, **kw: object
) -> CompileJob:
    files = {"a.neuron": "module a for host\n", "mode.txt": mode}
    files.update(kw.pop("extra_files", {}))  # type: ignore[arg-type]
    return CompileJob(command=command, files=files, **kw)  # type: ignore[arg-type]


def leftovers(work: Path) -> list[str]:
    return [p.name for p in work.iterdir()]


# ---- inputs ------------------------------------------------------------------------


@pytest.mark.parametrize(
    "path",
    [
        "",
        "../x.neuron",
        "a/../../x",
        "/etc/passwd",
        "C:/x.neuron",
        "a\\b.neuron",
        ".hidden",
        "a/./b.neuron",
        "a//b.neuron",
        "a/",
        "bad\x00name",
        "bad\nname",
        "-flag.neuron",
        "x" * 300,
        "a b ",
        "a$(id).neuron",
        "a;b.neuron",
    ],
)
async def test_bad_paths_are_rejected_before_anything_runs(tmp_path: Path, path: str) -> None:
    runner = CompileRunner(config(tmp_path))
    bad = CompileJob(command=CompileCommand.CHECK, files={path: "x", "ok.neuron": "x"})
    with pytest.raises(ServiceError) as info:
        await runner.run(bad)
    assert info.value.code is ErrorCode.JOB_REJECTED
    assert leftovers(tmp_path) == []


def test_limits_on_count_size_and_total() -> None:
    limits = InputLimits(max_files=2, max_file_bytes=10, max_total_bytes=15)
    check_files({"a": "x", "b": "y"}, limits)
    for files in (
        {},
        {"a": "x", "b": "y", "c": "z"},
        {"a": "x" * 11},
        {"a": "x" * 9, "b": "y" * 9},
        {"a": "é" * 6},  # 12 bytes: the limit counts bytes, not characters
    ):
        with pytest.raises(ServiceError) as info:
            check_files(files, limits)
        assert info.value.code is ErrorCode.JOB_REJECTED


def test_case_and_file_directory_clashes_are_rejected() -> None:
    for files in ({"A.neuron": "x", "a.neuron": "y"}, {"a": "x", "a/b": "y"}):
        with pytest.raises(ServiceError):
            check_files(files, InputLimits())


@given(st.text(max_size=120))
def test_accepted_paths_never_leave_the_directory(path: str) -> None:
    root = Path("/jobs/one").resolve()
    try:
        accepted = check_relative_path(path, InputLimits())
    except ServiceError:
        return
    parts = PurePosixPath(accepted).parts
    assert ".." not in parts
    assert not PurePosixPath(accepted).is_absolute()
    assert (root / accepted).resolve().is_relative_to(root)


@pytest.mark.parametrize(
    "changes",
    [
        {"command": CompileCommand.CHECK, "board": "uno"},
        {"command": CompileCommand.CHECK, "native": True},
        {"command": CompileCommand.BUILD, "board": "--evil"},
        {"command": CompileCommand.BUILD, "board": "a;b"},
        {"command": CompileCommand.BUILD, "native": True, "host": True},
        {"command": CompileCommand.BUILD, "language": "st"},
        {"command": CompileCommand.EMIT},
        {"command": CompileCommand.EMIT, "language": "rm -rf"},
        {"command": CompileCommand.EMIT, "language": "st", "profile": "../x"},
        {"command": CompileCommand.EMIT, "language": "st", "board": "x"},
        {"command": CompileCommand.BUILD, "policy": "exact"},
        {"command": CompileCommand.BUILD, "cpu_hz": 100},
    ],
)
async def test_option_combinations_are_validated(tmp_path: Path, changes: dict) -> None:
    runner = CompileRunner(config(tmp_path))
    bad = CompileJob(files={"a.neuron": "x", "mode.txt": "ok"}, **changes)
    with pytest.raises(ServiceError) as info:
        await runner.run(bad)
    assert info.value.code is ErrorCode.JOB_REJECTED
    assert leftovers(tmp_path) == []


# ---- a normal run ----------------------------------------------------------------------


async def test_build_result_artifacts_and_cleanup(tmp_path: Path) -> None:
    runner = CompileRunner(config(tmp_path))
    outcome = await runner.run(job("ok", board="arduino_uno_r3", native=True))
    result = outcome.result
    assert result.ok
    assert result.binary_version == "9.9.9-fake"
    assert [a.name for a in result.artifacts] == ["app.hex", "sub/app.lst"]
    for artifact in result.artifacts:
        data = outcome.files[artifact.name]
        assert artifact.size == len(data)
        assert artifact.sha256 == hashlib.sha256(data).hexdigest()
    assert result.diagnostics[0].file == "a.neuron"  # the job directory is not in the name
    assert leftovers(tmp_path) == []


async def test_program_errors_are_a_result_not_an_exception(tmp_path: Path) -> None:
    runner = CompileRunner(config(tmp_path))
    outcome = await runner.run(job("errors", CompileCommand.CHECK))
    result = outcome.result
    assert not result.ok
    assert result.exit_code == 1
    assert result.diagnostics[0].code == "NE0201"
    assert result.diagnostics[0].file == "a.neuron"
    assert str(tmp_path) not in result.model_dump_json()
    assert outcome.files == {}
    assert leftovers(tmp_path) == []


async def test_arguments_are_built_from_fields_only(tmp_path: Path) -> None:
    runner = CompileRunner(config(tmp_path))
    outcome = await runner.run(job("argv", board="arduino_uno_r3", native=True))
    argv = json.loads(outcome.files["argv.json"])["argv"]
    assert argv[0] == "build"
    assert argv[argv.index("--board") + 1] == "arduino_uno_r3"
    assert "--native" in argv
    assert argv[argv.index("--format") + 1] == "json"
    emit = await runner.run(
        job("argv", CompileCommand.EMIT, language="st", policy="equivalent", profile="yaskawa")
    )
    argv = json.loads(emit.files["argv.json"])["argv"]
    assert argv[:2] == ["emit", "st"]
    assert argv[argv.index("--policy") + 1] == "equivalent"
    assert argv[argv.index("--profile") + 1] == "yaskawa"


async def test_files_with_spaces_and_directories_are_written(tmp_path: Path) -> None:
    runner = CompileRunner(config(tmp_path))
    extra = {"lib/my unit.neuron": "x", "neuron.toml": "[package]\n"}
    outcome = await runner.run(job("ok", extra_files=extra))
    assert outcome.result.ok


# ---- environment ----------------------------------------------------------------------


async def test_environment_is_stripped(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    secrets = {
        "SYNAPSE_DATABASE_URL": "postgresql://u:topsecret@h/db",
        "SYNAPSE_JWT_SECRET": "jwt-secret-value",
        "AWS_SECRET_ACCESS_KEY": "aws-secret-value",
        "GITHUB_TOKEN": "gh-secret-value",
    }
    for key, value in secrets.items():
        monkeypatch.setenv(key, value)
    runner = CompileRunner(config(tmp_path))
    outcome = await runner.run(job("env"))
    env = json.loads(outcome.files["env.json"])
    text = json.dumps(env)
    for key, value in secrets.items():
        assert key not in env
        assert value not in text
    allowed = {"PATH", "SYSTEMROOT", "SystemRoot", "WINDIR", "HOME", "TMPDIR", "TMP", "TEMP"}
    allowed |= {"LANG", "NO_COLOR", "SYSTEMDRIVE", "COMSPEC", "PATHEXT"}  # Windows adds some
    assert set(env) - allowed == set(), set(env) - allowed


def test_build_env_has_a_private_home_and_extras() -> None:
    env = build_env(
        Path("/jobs/x"), {"NEURON_ROOT": "/opt/neuron"}, source={"PATH": "/bin", "X": "1"}
    )
    assert env["PATH"] == "/bin"
    assert "X" not in env
    assert Path(env["HOME"]) == Path("/jobs/x")
    assert env["NEURON_ROOT"] == "/opt/neuron"


async def test_the_process_runs_inside_the_package_directory(tmp_path: Path) -> None:
    outcome = await CompileRunner(config(tmp_path)).run(job("argv"))
    cwd = json.loads(outcome.files["argv.json"])["cwd"]
    assert Path(cwd).name == "src"


# ---- timeout, floods, limits ---------------------------------------------------------


async def test_timeout_kills_the_job_with_ne1102(tmp_path: Path) -> None:
    runner = CompileRunner(config(tmp_path, limits=Limits(timeout_s=1)))
    started = time.perf_counter()
    with pytest.raises(ServiceError) as info:
        await runner.run(job("sleep"))
    assert info.value.code is ErrorCode.JOB_TIMEOUT
    assert time.perf_counter() - started < 15
    assert leftovers(tmp_path) == []


def _alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


@posix_only
async def test_timeout_kills_the_whole_process_group(tmp_path: Path) -> None:
    pidfile = tmp_path.parent / f"{tmp_path.name}-child.pid"
    runner = CompileRunner(config(tmp_path, limits=Limits(timeout_s=2)))
    with pytest.raises(ServiceError) as info:
        await runner.run(job("spawn", extra_files={"pidfile.txt": str(pidfile)}))
    assert info.value.code is ErrorCode.JOB_TIMEOUT
    child = int(pidfile.read_text(encoding="utf-8"))
    for _ in range(50):
        if not _alive(child):
            break
        await asyncio.sleep(0.1)
    assert not _alive(child), "the grandchild survived the kill"


async def test_an_output_flood_is_stopped(tmp_path: Path) -> None:
    runner = CompileRunner(config(tmp_path, limits=Limits(timeout_s=20, max_output_bytes=100_000)))
    started = time.perf_counter()
    with pytest.raises(ServiceError) as info:
        await runner.run(job("flood"))
    assert info.value.code is ErrorCode.JOB_TIMEOUT
    assert "printed more than" in info.value.message
    assert time.perf_counter() - started < 15


@posix_only
async def test_cpu_limit_ends_a_busy_job(tmp_path: Path) -> None:
    runner = CompileRunner(config(tmp_path, limits=Limits(timeout_s=30, cpu_s=1)))
    with pytest.raises(ServiceError) as info:
        await runner.run(job("cpu"))
    assert info.value.code is ErrorCode.JOB_TIMEOUT
    assert "resource limit" in info.value.message


async def test_garbage_output_is_reported_as_a_failed_result(tmp_path: Path) -> None:
    outcome = await CompileRunner(config(tmp_path)).run(job("garbage"))
    assert not outcome.result.ok
    assert outcome.result.exit_code == 3
    assert "no valid JSON" in outcome.result.diagnostics[0].message


async def test_crash_output_hides_the_job_directory(tmp_path: Path) -> None:
    outcome = await CompileRunner(config(tmp_path)).run(job("crash"))
    assert outcome.result.exit_code == 101
    assert "panicked" in outcome.result.log_tail
    assert str(tmp_path) not in outcome.result.log_tail
    assert "<job>" in outcome.result.log_tail


# ---- artifacts ------------------------------------------------------------------------


@posix_only
async def test_links_in_the_output_are_not_followed(tmp_path: Path) -> None:
    outcome = await CompileRunner(config(tmp_path)).run(job("links"))
    assert sorted(outcome.files) == ["real.txt"]


async def test_artifact_caps_are_ne1101(tmp_path: Path) -> None:
    big = config(tmp_path, limits=Limits(timeout_s=20, max_artifact_bytes=100_000))
    with pytest.raises(ServiceError) as info:
        await CompileRunner(big).run(job("bigout"))
    assert info.value.code is ErrorCode.JOB_REJECTED
    many = config(tmp_path, limits=Limits(timeout_s=20, max_artifacts=10))
    with pytest.raises(ServiceError) as info:
        await CompileRunner(many).run(job("manyout"))
    assert info.value.code is ErrorCode.JOB_REJECTED
    total = config(tmp_path, limits=Limits(timeout_s=20, max_total_artifact_bytes=200_000))
    with pytest.raises(ServiceError):
        await CompileRunner(total).run(job("bigout"))
    assert leftovers(tmp_path) == []


async def test_a_missing_compiler_is_a_503(tmp_path: Path) -> None:
    runner = CompileRunner(config(tmp_path, neuron_cmd=(str(tmp_path / "no-such-binary"),)))
    with pytest.raises(HTTPException) as info:
        await runner.run(job("ok"))
    assert info.value.status_code == 503
    assert leftovers(tmp_path) == []


# ---- the real binary, when the environment provides one ---------------------------------


@pytest.mark.skipif("NEURON_BIN" not in os.environ, reason="NEURON_BIN is not set")
async def test_real_binary_checks_a_corpus_example(tmp_path: Path) -> None:
    source = (REPO / "examples/corpus/plc/p01_start_stop.neuron").read_text(encoding="utf-8")
    runner = CompileRunner(RunnerConfig(neuron_cmd=(os.environ["NEURON_BIN"],), work_root=tmp_path))
    outcome = await runner.run(
        CompileJob(command=CompileCommand.CHECK, files={"p01_start_stop.neuron": source})
    )
    assert outcome.result.ok, outcome.result
    assert outcome.result.binary_version


def test_network_status_type_is_exposed() -> None:
    assert NetworkSandbox("unverified").prefix == ()
