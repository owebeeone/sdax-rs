#!/usr/bin/env python3
"""Print a stable content revision for the performance harness."""

import hashlib
import pathlib
import sys


FILES = (
    "Cargo.toml",
    "Cargo.lock",
    "fixtures.csv",
    "src/alloc.rs",
    "src/fixtures.rs",
    "src/lifecycle.rs",
    "src/lifecycle_bench.rs",
    "src/lifecycle_plans.rs",
    "src/main.rs",
    "src/measure.rs",
    "src/tokio_lifecycle.rs",
)


def main() -> None:
    root = pathlib.Path(__file__).resolve().parent.parent
    if len(sys.argv) == 1:
        harness = root / "performance-harness"
    elif len(sys.argv) == 3 and sys.argv[1] == "--harness":
        harness = root / sys.argv[2]
    else:
        raise SystemExit("usage: performance-fixture-revision.py [--harness DIR]")
    digest = hashlib.sha256()
    for relative in FILES:
        label = f"performance-harness/{relative}"
        digest.update(label.encode("utf-8"))
        digest.update(b"\0")
        digest.update((harness / relative).read_bytes())
        digest.update(b"\0")
    print(digest.hexdigest())


if __name__ == "__main__":
    main()
