"""One offline verification inventory for local work, CI and release preparation."""

import argparse
from dataclasses import dataclass, field
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from release_selection import ROOT, ReleaseError


@dataclass
class Check:
    name: str
    command: list[str]
    env: dict[str, str] = field(default_factory=dict)


def checks(*, no_test=False, allow_dirty=False, msrv_only=False):
    if msrv_only:
        return [Check("msrv-" + crate, ["cargo", "+1.75", "build", "-p", crate, "--locked", "--offline"])
                for crate in ("sdax", "sdax-tokio")]
    python = [sys.executable, "-B"]
    inventory = [
        Check("lockfile", ["cargo", "metadata", "--format-version", "1", "--locked", "--offline"]),
        Check("fmt", ["cargo", "fmt", "--check"]),
        Check("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--offline", "--", "-D", "warnings"]),
        Check("tests", ["cargo", "test", "--workspace", "--locked", "--offline"]),
        Check("architecture", ["sh", "scripts/check-architecture.sh"]),
        Check("compile-fail", ["sh", "scripts/compile-fail.sh"]),
        Check("guide-quotes", ["sh", "scripts/check-guide-quotes.sh"]),
        Check("rustdoc", ["cargo", "doc", "--workspace", "--no-deps", "--locked", "--offline"],
              {"RUSTDOCFLAGS": "-D warnings"}),
        Check("script-tests", [*python, "-m", "unittest", "discover", "-s", "scripts", "-p", "test_*.py"]),
        Check("consumer", [*python, "scripts/check-consumer-guide.py"]),
        Check("archives", [*python, "scripts/package_archives.py", *(["--allow-dirty"] if allow_dirty else [])]),
    ]
    return [check for check in inventory if not (no_test and check.name == "tests")]


def run_checks(inventory, *, root=ROOT, runner=subprocess.run, log_dir=None):
    records = []
    if log_dir:
        log_dir.mkdir(parents=True, exist_ok=True)
    for check in inventory:
        print(f"CHECK {check.name}", flush=True)
        started = time.monotonic()
        result = runner(check.command, cwd=root, text=True, stdout=subprocess.PIPE,
                        stderr=subprocess.STDOUT, env={**os.environ, **check.env})
        records.append({"name": check.name, "command": check.command, "env": check.env,
                        "exit_code": result.returncode, "seconds": round(time.monotonic() - started, 3)})
        if log_dir:
            (log_dir / f"{check.name}.log").write_text(result.stdout)
            (log_dir / "results.json").write_text(json.dumps(records, indent=2) + "\n")
        if result.returncode:
            print(result.stdout, file=sys.stderr)
            raise ReleaseError(f"{check.name} failed (exit {result.returncode})")
        print(f"PASS  {check.name}", flush=True)
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-dirty", action="store_true", help="allow packaging edited local source")
    parser.add_argument("--no-test", action="store_true", help="skip only cargo test; all other gates remain")
    parser.add_argument("--msrv-only", action="store_true", help="build the two libraries using installed Rust 1.75")
    parser.add_argument("--log-dir", type=Path)
    args = parser.parse_args()
    try:
        run_checks(checks(no_test=args.no_test, allow_dirty=args.allow_dirty, msrv_only=args.msrv_only),
                   log_dir=args.log_dir)
    except (ReleaseError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f"checks: {error}\n")


if __name__ == "__main__":
    main()
