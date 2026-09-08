#!/usr/bin/env python3
"""Run one command and write its wall time and exit status as JSON."""

import json
import os
import subprocess
import sys
import time


def main() -> int:
    if len(sys.argv) < 4 or sys.argv[2] != "--":
        print("usage: time-command.py OUTPUT.json -- COMMAND ...", file=sys.stderr)
        return 2
    output = sys.argv[1]
    command = sys.argv[3:]
    started = time.monotonic_ns()
    result = subprocess.run(command, check=False)
    elapsed = time.monotonic_ns() - started
    record = {
        "command": command,
        "cwd": os.getcwd(),
        "elapsed_ns": elapsed,
        "exit_code": result.returncode,
    }
    with open(output, "w", encoding="utf-8") as handle:
        json.dump(record, handle, indent=2)
        handle.write("\n")
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
