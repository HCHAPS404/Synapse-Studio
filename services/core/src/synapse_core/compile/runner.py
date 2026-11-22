"""The compile runner: one fresh directory and one sandboxed `neuron` process per job."""

import asyncio
import contextlib
import hashlib
import logging
import os
import shutil
import stat
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path

from fastapi import HTTPException
from pydantic import ValidationError

from synapse_core.compile.models import (
    ArtifactInfo,
    CliDocument,
    CompileCommand,
    CompileJob,
    CompileOutcome,
    CompileResult,
    Diagnostic,
)
from synapse_core.compile.paths import InputLimits, check_files, check_name, reject
from synapse_core.compile.sandbox import (
    IS_POSIX,
    Limits,
    NetworkSandbox,
    build_env,
    kill_group,
    reap_group,
    spawn_options,
)
from synapse_core.errors import ErrorCode, ServiceError
from synapse_core.settings import Settings

log = logging.getLogger(__name__)

_EMIT_LANGUAGES = frozenset({"st", "scl", "plcopen", "spel", "python", "kernel", "c"})
_LOG_TAIL_BYTES = 4096
# Signals that mean a resource limit or the kernel ended the job (SIGXCPU, SIGKILL, SIGXFSZ).
_LIMIT_SIGNALS = frozenset({9, 24, 25})


@dataclass(frozen=True)
class RunnerConfig:
    """Everything the runner needs. `neuron_cmd` is an argument list, never a shell string."""

    neuron_cmd: tuple[str, ...] = ("neuron",)
    inputs: InputLimits = field(default_factory=InputLimits)
    limits: Limits = field(default_factory=Limits)
    network: NetworkSandbox = field(default_factory=lambda: NetworkSandbox("unverified"))
    extra_env: dict[str, str] = field(default_factory=dict)
    work_root: Path | None = None


def _argv(job: CompileJob, src: Path, out: Path) -> list[str]:
    """The `neuron` arguments of a job, built from validated fields only."""
    args: list[str] = [job.command.value]
    if job.command is CompileCommand.EMIT:
        if job.language not in _EMIT_LANGUAGES:
            raise reject("`language` must be one of " + ", ".join(sorted(_EMIT_LANGUAGES)))
        args.append(job.language)
    elif job.language is not None:
        raise reject("`language` is only for emit")
    if job.command is CompileCommand.BUILD:
        if job.native and job.host:
            raise reject("`native` and `host` cannot be combined")
        if job.board is not None:
            args += ["--board", check_name(job.board, "board")]
        args += ["--native"] * job.native + ["--host"] * job.host
        args += ["--out", str(out)]
    elif job.board is not None or job.native or job.host:
        raise reject("`board`, `native` and `host` are only for build")
    if job.command is CompileCommand.EMIT:
        if job.profile is not None:
            args += ["--profile", check_name(job.profile, "profile")]
        if job.robot is not None:
            args += ["--robot", check_name(job.robot, "robot")]
        if job.policy is not None:
            args += ["--policy", job.policy]
        args += ["-o", str(out)]
    elif job.profile is not None or job.robot is not None or job.policy is not None:
        raise reject("`profile`, `robot` and `policy` are only for emit")
    if job.command is CompileCommand.CHECK:
        if job.cpu_hz is not None:
            args += ["--cpu-hz", str(job.cpu_hz)]
        if job.ram is not None:
            args += ["--ram", str(job.ram)]
    elif job.cpu_hz is not None or job.ram is not None:
        raise reject("`cpu_hz` and `ram` are only for check")
    # The package is the source directory; the process runs inside it.
    return [*args, "--format", "json", str(src)]


def _write_inputs(src: Path, files: dict[str, bytes]) -> None:
    """Writes the files into a directory this runner just created (so no link exists)."""
    root = src.resolve()
    for rel, data in files.items():
        target = (root / rel).resolve()
        if not target.is_relative_to(root):  # defence in depth after `check_files`
            raise reject("a file path leaves the job directory")
        target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as handle:
            handle.write(data)


def _prepare(files: dict[str, bytes], work_root: Path | None) -> Path:
    """Makes the job directory (mode 0700) with `src/` filled and an empty `tmp/`."""
    if work_root is not None:
        work_root.mkdir(parents=True, exist_ok=True)
    job_dir = Path(tempfile.mkdtemp(prefix="synapse-job-", dir=work_root)).resolve()
    try:
        if IS_POSIX:
            job_dir.chmod(0o700)
        for name in ("src", "tmp"):
            (job_dir / name).mkdir(mode=0o700)
        _write_inputs(job_dir / "src", files)
    except BaseException:
        shutil.rmtree(job_dir, ignore_errors=True)
        raise
    return job_dir


async def _drain(stream: asyncio.StreamReader, cap: int, overflow: asyncio.Event) -> bytes:
    """Reads a stream to the end, keeping at most `cap` bytes and flagging an overflow."""
    kept = bytearray()
    while chunk := await stream.read(65536):
        room = cap - len(kept)
        if room > 0:
            kept += chunk[:room]
        if len(chunk) > room:
            overflow.set()
    return bytes(kept)


def _scrub(text: str, job_dir: Path) -> str:
    """Hides the temporary directory so results do not depend on where the job ran."""
    for variant in {str(job_dir), str(job_dir.resolve()), job_dir.as_posix()}:
        text = text.replace(variant, "<job>")
    return text


def _tail(data: bytes, job_dir: Path) -> str:
    return _scrub(data[-_LOG_TAIL_BYTES:].decode("utf-8", "replace"), job_dir)


def _clean_file(name: str | None, job_dir: Path) -> str | None:
    if name is None:
        return None
    name = _scrub(name, job_dir).replace("\\", "/")
    for prefix in ("<job>/src/", "<job>/", "./"):
        if name.startswith(prefix):
            name = name[len(prefix) :]
    return name


def _collect(out: Path, limits: Limits) -> tuple[list[ArtifactInfo], dict[str, bytes]]:
    """Reads the regular files under `out` (never following links), hashed and size-capped."""
    infos: list[ArtifactInfo] = []
    blobs: dict[str, bytes] = {}
    if not out.is_dir():
        return infos, blobs
    total = 0
    root = out.resolve()
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        dirnames.sort()
        for name in sorted(filenames):
            path = Path(dirpath) / name
            meta = path.lstat()
            if not stat.S_ISREG(meta.st_mode):
                continue
            if len(infos) >= limits.max_artifacts:
                raise ServiceError(
                    ErrorCode.JOB_REJECTED, f"more than {limits.max_artifacts} output files"
                )
            if meta.st_size > limits.max_artifact_bytes:
                raise ServiceError(
                    ErrorCode.JOB_REJECTED,
                    f"an output file is larger than {limits.max_artifact_bytes} bytes",
                )
            total += meta.st_size
            if total > limits.max_total_artifact_bytes:
                raise ServiceError(
                    ErrorCode.JOB_REJECTED,
                    f"the output is larger than {limits.max_total_artifact_bytes} bytes",
                )
            flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
            with os.fdopen(os.open(path, flags), "rb") as handle:
                data = handle.read(limits.max_artifact_bytes + 1)
            rel = path.relative_to(root).as_posix()
            blobs[rel] = data
            infos.append(
                ArtifactInfo(name=rel, size=len(data), sha256=hashlib.sha256(data).hexdigest())
            )
    return infos, blobs


def _result_from(
    code: int, stdout: bytes, stderr: bytes, job_dir: Path, elapsed_ms: float
) -> CompileResult:
    """Builds the result from the CLI document, or from the raw output when there is none."""
    tail = _tail(stderr, job_dir)
    try:
        doc = CliDocument.model_validate_json(stdout)
    except ValidationError:
        message = f"neuron exited with code {code} and printed no valid JSON document"
        return CompileResult(
            ok=False,
            exit_code=code,
            diagnostics=[Diagnostic(severity="error", message=message)],
            log_tail=tail or _tail(stdout, job_dir),
            duration_ms=elapsed_ms,
        )
    diagnostics = [
        d.model_copy(
            update={
                "file": _clean_file(d.file, job_dir),
                "message": _scrub(d.message, job_dir),
                "help": _scrub(d.help, job_dir) if d.help else None,
            }
        )
        for d in doc.diagnostics
    ]
    return CompileResult(
        ok=code == 0 and doc.ok,
        exit_code=code,
        diagnostics=diagnostics,
        messages=[_scrub(m, job_dir) for m in doc.messages],
        log_tail=tail,
        binary_version=doc.neuron_version,
        duration_ms=elapsed_ms,
    )


def runner_config(settings: Settings, network: NetworkSandbox) -> RunnerConfig:
    """The runner configuration for the service settings and the network self-test."""
    return RunnerConfig(
        neuron_cmd=(settings.neuron_bin,),
        limits=Limits(timeout_s=settings.compile_timeout_s),
        network=network,
        work_root=Path(settings.compile_work_dir) if settings.compile_work_dir else None,
    )


class CompileRunner:
    """Runs compile jobs. One instance is shared; each `run` is independent."""

    def __init__(self, config: RunnerConfig) -> None:
        self.config = config

    async def run(self, job: CompileJob) -> CompileOutcome:
        """Runs one job.

        Raises `ServiceError` with `NE1101` (rejected input or oversized output) or
        `NE1102` (timeout, output flood, or ended by a resource limit). A program with
        errors is not an exception: it is a result with `ok == False`.
        """
        files = check_files(job.files, self.config.inputs)
        _argv(job, Path("src"), Path("out"))  # rejects bad options before any disk work
        job_dir = await asyncio.to_thread(_prepare, files, self.config.work_root)
        try:
            src, out = job_dir / "src", job_dir / "out"
            argv = [
                *self.config.network.prefix,
                *self.config.neuron_cmd,
                *_argv(job, src, out),
            ]
            return await self._execute(argv, src, job_dir, out)
        finally:
            await asyncio.to_thread(shutil.rmtree, job_dir, True)

    async def _execute(
        self, argv: list[str], cwd: Path, job_dir: Path, out: Path
    ) -> CompileOutcome:
        limits = self.config.limits
        env = build_env(job_dir, self.config.extra_env)
        started = time.perf_counter()
        try:
            proc = await asyncio.create_subprocess_exec(
                *argv,
                stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                cwd=cwd,
                env=env,
                **spawn_options(limits),  # type: ignore[arg-type]
            )
        except OSError as exc:
            log.error("cannot start the compiler", extra={"error": type(exc).__name__})
            raise HTTPException(status_code=503, detail="the compiler is not available") from exc
        assert proc.stdout is not None
        assert proc.stderr is not None
        overflow = asyncio.Event()
        readers = asyncio.gather(
            _drain(proc.stdout, limits.max_output_bytes, overflow),
            _drain(proc.stderr, limits.max_output_bytes, overflow),
        )
        flood = asyncio.create_task(overflow.wait())
        waiter = asyncio.create_task(proc.wait())
        try:
            done, _ = await asyncio.wait(
                {waiter, flood}, timeout=limits.timeout_s, return_when=asyncio.FIRST_COMPLETED
            )
            if not done:
                await kill_group(proc)
                raise ServiceError(
                    ErrorCode.JOB_TIMEOUT, f"the job ran longer than {limits.timeout_s:g} s"
                )
            if flood in done and waiter not in done:
                await kill_group(proc)
                raise ServiceError(
                    ErrorCode.JOB_TIMEOUT,
                    f"the job printed more than {limits.max_output_bytes} bytes",
                )
            reap_group(proc)
            stdout, stderr = await readers
        except BaseException:
            await kill_group(proc)
            raise
        finally:
            for task in (flood, waiter):
                task.cancel()
            await asyncio.gather(flood, waiter, return_exceptions=True)
            if not readers.done():
                readers.cancel()
                await asyncio.gather(readers, return_exceptions=True)
            with contextlib.suppress(ProcessLookupError):
                await proc.wait()
        code = proc.returncode if proc.returncode is not None else -1
        if overflow.is_set():
            raise ServiceError(
                ErrorCode.JOB_TIMEOUT,
                f"the job printed more than {limits.max_output_bytes} bytes",
            )
        if code < 0 and -code in _LIMIT_SIGNALS:
            raise ServiceError(
                ErrorCode.JOB_TIMEOUT, "the job was ended by a resource limit (CPU, memory or size)"
            )
        elapsed_ms = (time.perf_counter() - started) * 1000
        result = _result_from(code, stdout, stderr, job_dir, elapsed_ms)
        infos, blobs = await asyncio.to_thread(_collect, out, limits)
        return CompileOutcome(result=result.model_copy(update={"artifacts": infos}), files=blobs)
