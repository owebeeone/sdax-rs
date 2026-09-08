#!/usr/bin/env python3
"""Capture Linux child-process CPU and peak RSS from native getrusage counters."""

import pathlib
import resource
import subprocess
import sys
import time


def main() -> None:
    if len(sys.argv) != 6:
        raise SystemExit(
            "usage: resident-process-probe.py NATIVE STDOUT STDERR BINARY SECONDS"
        )
    native, stdout_path, stderr_path, binary, seconds = sys.argv[1:]
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.perf_counter()
    with pathlib.Path(stdout_path).open("wb") as stdout, pathlib.Path(stderr_path).open(
        "wb"
    ) as stderr:
        process = subprocess.Popen(
            [binary, "resident-probe", "--seconds", seconds],
            stdout=stdout,
            stderr=stderr,
        )
        exit_code = process.wait()
    elapsed = time.perf_counter() - started
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    if exit_code != 0:
        raise SystemExit(exit_code)
    pathlib.Path(native).write_text(
        "".join(
            (
                f"wall_seconds={elapsed:.9f}\n",
                f"user_cpu_seconds={after.ru_utime - before.ru_utime:.9f}\n",
                f"system_cpu_seconds={after.ru_stime - before.ru_stime:.9f}\n",
                f"maximum_resident_set_kib={after.ru_maxrss}\n",
            )
        ),
        encoding="ascii",
    )


if __name__ == "__main__":
    main()
