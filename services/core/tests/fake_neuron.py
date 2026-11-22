"""A stand-in for the `neuron` executable, so the runner tests do not need the Rust build.

It follows the contract of docs/cli-json.md. What it does is chosen by the file `mode.txt`
in the package directory (the last argument): ok, errors, sleep, spawn, flood, garbage,
crash, env, links, bigout, cpu, argv.
"""

import json
import os
import subprocess
import sys
import time
from pathlib import Path


def option(args: list[str], *names: str) -> str | None:
    for i, a in enumerate(args):
        if a in names and i + 1 < len(args):
            return args[i + 1]
    return None


def document(command: str, ok: bool, diagnostics: list[dict], messages: list[str]) -> str:
    return json.dumps(
        {
            "format": 1,
            "command": command,
            "ok": ok,
            "neuron_version": "9.9.9-fake",
            "language_edition": "2026",
            "diagnostics": diagnostics,
            "messages": messages,
            "output_dir": None,
            "artifacts": [],
        }
    )


def diag(severity: str, code: str | None, message: str, file: str | None = None) -> dict:
    return {
        "code": code,
        "severity": severity,
        "message": message,
        "file": file,
        "line": 1 if file else None,
        "column": 1 if file else None,
        "help": None,
        "notes": [],
    }


def main() -> int:
    args = sys.argv[1:]
    command = args[0]
    src = Path(args[-1])
    out_text = option(args, "--out", "-o")
    out = Path(out_text) if out_text else None
    mode_file = src / "mode.txt"
    mode = mode_file.read_text(encoding="utf-8").strip() if mode_file.exists() else "ok"

    if mode == "sleep":
        time.sleep(120)
    elif mode == "spawn":
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)"])  # noqa: S603
        Path((src / "pidfile.txt").read_text(encoding="utf-8").strip()).write_text(
            str(child.pid), encoding="utf-8"
        )
        time.sleep(120)
    elif mode == "flood":
        chunk = "x" * 65536
        while True:
            sys.stdout.write(chunk)
            sys.stdout.flush()
    elif mode == "garbage":
        print("this is not json")
        return 3
    elif mode == "crash":
        sys.stderr.write(f"thread 'main' panicked at {src}/boom.rs:1\n")
        return 101
    elif mode == "cpu":
        while True:
            pass
    elif mode == "errors":
        d = diag("error", "NE0201", f"type mismatch in {src}/a.neuron", f"{src}/a.neuron")
        sys.stdout.write(document(command, False, [d], ["error: the program has errors"]))
        return 1

    if out is not None:
        out.mkdir(parents=True, exist_ok=True)
        if mode == "env":
            (out / "env.json").write_text(json.dumps(dict(os.environ)), encoding="utf-8")
        elif mode == "argv":
            (out / "argv.json").write_text(
                json.dumps(
                    {"argv": args, "cwd": os.getcwd(), "fake_ns": os.environ.get("FAKE_NS")}
                ),
                encoding="utf-8",
            )
        elif mode == "links":
            (out / "real.txt").write_text("real", encoding="utf-8")
            os.symlink(str(src / "mode.txt"), str(out / "link.txt"))
        elif mode == "bigout":
            (out / "big.bin").write_bytes(b"\0" * 300_000)
        elif mode == "manyout":
            for i in range(30):
                (out / f"f{i:02d}.txt").write_text("x", encoding="utf-8")
        else:
            (out / "app.hex").write_text(":00000001FF\n", encoding="utf-8")
            sub = out / "sub"
            sub.mkdir(exist_ok=True)
            (sub / "app.lst").write_text("listing\n", encoding="utf-8")
    warn = diag("warning", "NL0402", "no timing model", f"{src}/a.neuron")
    sys.stdout.write(document(command, True, [warn], ["built it"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
