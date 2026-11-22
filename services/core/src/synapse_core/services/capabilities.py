"""What this image can compile: the `neuron` binary, cross compilers and limits."""

import asyncio
import contextlib
import shutil

from synapse_core.compile.models import CompileCommand
from synapse_core.compile.runner import EMIT_LANGUAGES, RunnerConfig
from synapse_core.schemas.builds import CapabilitiesOut, CompileLimits, NeuronCapability

TOOLCHAINS = ("gcc", "avr-gcc", "arm-none-eabi-gcc")


async def probe_neuron(cmd: tuple[str, ...], timeout_s: float = 5.0) -> NeuronCapability:
    """Runs `neuron --version`; any failure means unavailable (never an exception)."""
    try:
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            "--version",
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
        )
    except OSError:
        return NeuronCapability(available=False, version=None)
    try:
        out, _ = await asyncio.wait_for(proc.communicate(), timeout_s)
    except TimeoutError:
        with contextlib.suppress(ProcessLookupError):
            proc.kill()
        return NeuronCapability(available=False, version=None)
    if proc.returncode != 0:
        return NeuronCapability(available=False, version=None)
    text = out.decode("utf-8", "replace").strip().splitlines()
    return NeuronCapability(available=True, version=(text[0][:64] if text else None))


def find_toolchains() -> dict[str, bool]:
    """Which cross compilers are on `PATH`."""
    return {name: shutil.which(name) is not None for name in TOOLCHAINS}


async def build_capabilities(
    config: RunnerConfig, concurrency: int, queue_max: int
) -> CapabilitiesOut:
    """The capability document for a runner configuration."""
    neuron = await probe_neuron(config.neuron_cmd)
    toolchains = await asyncio.to_thread(find_toolchains)
    return CapabilitiesOut(
        neuron=neuron,
        commands=[c.value for c in CompileCommand] if neuron.available else [],
        emit_languages=sorted(EMIT_LANGUAGES) if neuron.available else [],
        toolchains=toolchains,
        sandbox_network=config.network.status,
        limits=CompileLimits(
            timeout_s=config.limits.timeout_s,
            max_files=config.inputs.max_files,
            max_file_bytes=config.inputs.max_file_bytes,
            max_total_bytes=config.inputs.max_total_bytes,
            concurrency=concurrency,
            queue_max=queue_max,
        ),
    )
