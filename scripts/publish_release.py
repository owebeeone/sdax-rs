"""Workflow publication executor; the person-operated entry point is release.py.

Invoke only downstream of the selected commit's complete checks and MSRV jobs.
Tests inject registry and Cargo implementations and never contact crates.io.
"""

import argparse
from pathlib import Path
import subprocess
import tempfile
import time

from package_archives import package, run, validate_archive
from registry_release import Registry
from release_selection import (ROOT, ReleaseError, check_checkout, from_environment, verify_tag)


def publish_release(root, selected, registry, cargo, *, remote="origin", sleep=time.sleep, attempts=12):
    def identity():
        check_checkout(root, selected)
        verify_tag(root, selected, remote=remote)

    identity()
    # Preflight both names before any write: an incompatible existing adapter
    # must not be discovered only after publishing the core.
    present = {crate: registry.existing(crate, selected) for crate in ("sdax", "sdax-tokio")}
    if present["sdax"] is None:
        cargo.package("sdax")
        identity()
        cargo.publish("sdax")

    for attempt in range(attempts):
        identity()
        checksum = registry.existing("sdax", selected)
        if checksum is not None and registry.core_indexed(selected.version, checksum):
            break
        if attempt + 1 < attempts:
            sleep(5)
    else:
        raise ReleaseError("core version did not become resolvable within the bounded wait")

    # A real, non-staged registry package/build is mandatory even for a retry.
    cargo.package("sdax-tokio")
    identity()
    if present["sdax-tokio"] is None:
        cargo.publish("sdax-tokio")


class Cargo:
    def __init__(self, root, selected):
        self.root = root
        self.selected = selected

    def package(self, crate):
        with tempfile.TemporaryDirectory(prefix="sdax-registry-package-") as target:
            run(["cargo", "package", "-p", crate, "--locked", "--registry", "crates-io",
                 "--target-dir", target], cwd=self.root)
            path = Path(target) / "package" / f"{crate}-{self.selected.version}.crate"
            validate_archive(path.read_bytes(), self.root, crate, self.selected.version)
        print(f"Verified real registry package: {crate}", flush=True)

    def publish(self, crate):
        subprocess.run(["cargo", "publish", "-p", crate, "--locked", "--registry", "crates-io"],
                       cwd=self.root, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    try:
        selected = from_environment()
        check_checkout(ROOT, selected)
        verify_tag(ROOT, selected, remote="origin")
        registry = Registry(package(ROOT))
        publish_release(ROOT, selected, registry, Cargo(ROOT, selected))
    except (ReleaseError, KeyError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f"publication refused: {error}\n")


if __name__ == "__main__":
    main()
