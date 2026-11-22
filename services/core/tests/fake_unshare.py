"""A stand-in for `unshare --net --`: marks the environment, then runs the rest."""

import os
import subprocess
import sys


def main() -> int:
    args = sys.argv[1:]
    if args and args[0] == "--fail":
        return 1
    rest = args[args.index("--") + 1 :] if "--" in args else args
    env = dict(os.environ)
    if "--shared" not in args:
        env["FAKE_NS"] = "isolated"
    return subprocess.run(rest, env=env, check=False).returncode  # noqa: S603


if __name__ == "__main__":
    sys.exit(main())
