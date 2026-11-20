# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
"""Prints the path of each CPython named on the command line, as 3.14 or
3.14t, from those actions/setup-python installed on a macOS or Windows
runner, one per line, and fails if any is missing.

Each candidate is asked what it is. Going by names on PATH isn't enough:
on macOS the free-threaded installer's python3.14 shadows the ordinary
one, so maturin's --find-interpreter built 3.14t twice and 3.14 never.
"""

import glob
import os
import subprocess
import sys

ASK = (
    "import sys, sysconfig; "
    "t = 't' if sysconfig.get_config_var('Py_GIL_DISABLED') else ''; "
    "print(f'{sys.version_info[0]}.{sys.version_info[1]}{t}')"
)


def candidates():
    cache = os.environ.get("RUNNER_TOOL_CACHE", "")
    patterns = [
        os.path.join(cache, "Python", "*", "*", "python.exe"),
        os.path.join(cache, "Python", "*", "*", "bin", "python3"),
        "/Library/Frameworks/Python*.framework/Versions/*/bin/python3.*",
    ]
    for pattern in patterns:
        for path in sorted(glob.glob(pattern)):
            if not path.endswith("-config") and os.access(path, os.X_OK):
                yield path


def main(wanted):
    found = {}
    for path in candidates():
        try:
            what = subprocess.run(
                [path, "-c", ASK], capture_output=True, text=True, timeout=60
            ).stdout.strip()
        except (OSError, subprocess.SubprocessError):
            continue
        found.setdefault(what, path)
    missing = [v for v in wanted if v not in found]
    if missing:
        print(f"no interpreter for {', '.join(missing)}; found {found}", file=sys.stderr)
        return 1
    for v in wanted:
        print(f"{v}: {found[v]}", file=sys.stderr)
        print(found[v])
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
