"""Process sandbox of a compile job: environment, limits, process-group kill, network probe."""

import asyncio
import contextlib
import json
import os
import shutil
import signal
import subprocess
import sys
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

IS_POSIX = os.name == "posix"

# Variables a job may inherit. Everything else (SYNAPSE_*, cloud keys, tokens) is dropped.
_INHERITED_ENV = ("PATH", "SYSTEMROOT", "SystemRoot", "WINDIR")

# Run inside the sandbox and outside it: the network namespace and the interfaces seen.
PROBE_CODE = (
    "import json, os\n"
    "ns = os.readlink('/proc/self/ns/net')\n"
    "rows = open('/proc/net/dev').read().splitlines()[2:]\n"
    "print(json.dumps({'ns': ns, 'if': sorted(r.split(':')[0].strip() for r in rows)}))\n"
)


@dataclass(frozen=True)
class Limits:
    """Resource limits of a job (POSIX `setrlimit`; ignored on Windows) and its output caps."""

    timeout_s: float = 30.0
    cpu_s: int = 30
    address_space_bytes: int = 4 * 1024**3
    file_size_bytes: int = 64 * 1024**2
    open_files: int = 256
    max_output_bytes: int = 1024 * 1024
    max_artifacts: int = 200
    max_artifact_bytes: int = 20 * 1024**2
    max_total_artifact_bytes: int = 64 * 1024**2


def build_env(
    job_dir: Path, extra: Mapping[str, str] | None = None, source: Mapping[str, str] | None = None
) -> dict[str, str]:
    """The minimal environment of a job: a few inherited names, a private home and temp."""
    source = os.environ if source is None else source
    env = {k: source[k] for k in _INHERITED_ENV if k in source}
    tmp = str(job_dir / "tmp")
    env.update(
        {
            "HOME": str(job_dir),
            "TMPDIR": tmp,
            "TMP": tmp,
            "TEMP": tmp,
            "LANG": "C.UTF-8",
            "NO_COLOR": "1",
        }
    )
    if extra:
        env.update(extra)
    return env


def rlimit_setter(limits: Limits) -> Callable[[], None] | None:
    """A function for `preexec_fn` that applies the limits in the child; `None` on Windows."""
    if not IS_POSIX:
        return None
    import resource  # noqa: PLC0415  (POSIX only)

    def apply() -> None:
        pairs = [
            (resource.RLIMIT_CPU, limits.cpu_s),
            (resource.RLIMIT_AS, limits.address_space_bytes),
            (resource.RLIMIT_FSIZE, limits.file_size_bytes),
            (resource.RLIMIT_NOFILE, limits.open_files),
            (resource.RLIMIT_CORE, 0),
        ]
        for which, value in pairs:
            _soft, hard = resource.getrlimit(which)
            if hard != resource.RLIM_INFINITY:
                value = min(value, hard)
            with contextlib.suppress(ValueError, OSError):
                resource.setrlimit(which, (value, value))

    return apply


def spawn_options(limits: Limits) -> dict[str, object]:
    """Keyword arguments that put the job in its own process group (or console group)."""
    if IS_POSIX:
        return {"start_new_session": True, "preexec_fn": rlimit_setter(limits)}
    return {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP}  # type: ignore[attr-defined]


async def kill_group(proc: asyncio.subprocess.Process) -> None:
    """Kills the job and everything it started. Safe to call twice."""
    if IS_POSIX:
        with contextlib.suppress(ProcessLookupError, PermissionError):
            os.killpg(proc.pid, signal.SIGKILL)
    else:
        # `taskkill /T` ends the tree; there is no process group kill on Windows.
        with contextlib.suppress(OSError):
            await asyncio.to_thread(
                subprocess.run,
                ["taskkill", "/F", "/T", "/PID", str(proc.pid)],  # noqa: S607
                check=False,
                capture_output=True,
                timeout=10,
            )
    with contextlib.suppress(ProcessLookupError):
        proc.kill()


def reap_group(proc: asyncio.subprocess.Process) -> None:
    """After a normal exit, kills members left in the job's group (POSIX)."""
    if IS_POSIX:
        with contextlib.suppress(ProcessLookupError, PermissionError):
            os.killpg(proc.pid, signal.SIGKILL)


NetworkStatus = Literal["isolated", "unverified"]


@dataclass(frozen=True)
class NetworkSandbox:
    """Result of the start-up self-test. `isolated` only when a probe proved it."""

    status: NetworkStatus
    prefix: tuple[str, ...] = field(default=())
    detail: str = ""


async def _run_probe(argv: Sequence[str], timeout_s: float) -> dict[str, object] | None:
    try:
        proc = await asyncio.create_subprocess_exec(
            *argv,
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
        )
        out, _ = await asyncio.wait_for(proc.communicate(), timeout_s)
    except (OSError, TimeoutError):
        return None
    if proc.returncode != 0:
        return None
    try:
        doc = json.loads(out.decode("utf-8", "replace"))
    except ValueError:
        return None
    return doc if isinstance(doc, dict) else None


async def detect_network_sandbox(
    candidates: Sequence[Sequence[str]] | None = None,
    probe_code: str = PROBE_CODE,
    timeout_s: float = 10.0,
) -> NetworkSandbox:
    """Finds a command prefix that puts a job in an empty network namespace.

    Each candidate (default: `unshare --net`, then with `--map-root-user`) runs a probe
    that prints its network namespace and interfaces. The prefix counts as isolated only
    when the namespace differs from the host's and holds no interface but `lo`. Anything
    else is `unverified` and jobs run without a prefix.
    """
    if candidates is None:
        unshare = shutil.which("unshare") if sys.platform == "linux" else None
        candidates = (
            [(unshare, "--net", "--"), (unshare, "--net", "--map-root-user", "--")]
            if unshare
            else []
        )
    if not candidates:
        return NetworkSandbox("unverified", detail="no network namespace tool on this system")
    probe = [sys.executable, "-c", probe_code]
    host = await _run_probe(probe, timeout_s)
    if host is None:
        return NetworkSandbox("unverified", detail="the probe does not run on the host")
    for prefix in candidates:
        inner = await _run_probe([*prefix, *probe], timeout_s)
        if inner is None:
            continue
        interfaces = inner.get("if")
        if (
            inner.get("ns") != host.get("ns")
            and isinstance(interfaces, list)
            and set(interfaces) <= {"lo"}
        ):
            return NetworkSandbox("isolated", tuple(prefix), "network namespace verified")
    return NetworkSandbox("unverified", detail="the container does not allow a network namespace")
