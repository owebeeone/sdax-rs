#!/usr/bin/env python3
"""Hash the source and manifests that determine harness library behavior."""

import hashlib
import pathlib
import sys


def main() -> None:
    default_root = pathlib.Path(__file__).resolve().parent.parent
    if len(sys.argv) == 1:
        root = default_root
    elif len(sys.argv) == 3 and sys.argv[1] == "--repo":
        root = pathlib.Path(sys.argv[2]).resolve()
    else:
        raise SystemExit("usage: performance-source-revision.py [--repo DIR]")
    files = [root / "Cargo.toml", root / "Cargo.lock"]
    files.extend(sorted((root / "crates").glob("*/Cargo.toml")))
    files.extend(sorted((root / "crates").glob("*/src/**/*.rs")))
    digest = hashlib.sha256()
    for path in sorted(files):
        relative = path.relative_to(root).as_posix()
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    print(digest.hexdigest())


if __name__ == "__main__":
    main()
