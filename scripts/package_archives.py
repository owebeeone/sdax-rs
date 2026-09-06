"""Fresh Cargo archives: verified core and explicitly staged adapter contents.

The adapter is packaged alongside the unpublished core with --no-verify in a
separate temporary target. This is content evidence, not registry resolution.
The publish path separately packages/builds the adapter against crates.io.
"""

import argparse
import io
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
import tempfile
import tomllib

from release_selection import ROOT, ReleaseError

LICENSES = ("LICENSE-MIT", "LICENSE-APACHE-2.0")


def run(command, *, cwd):
    result = subprocess.run(command, cwd=cwd, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT)
    if result.returncode:
        raise ReleaseError(f"{' '.join(command)} failed:\n{result.stdout}")
    return result.stdout


def read_archive(data, crate, version):
    prefix = f"{crate}-{version}/"
    files = {}
    try:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            for member in archive:
                if member.isdir():
                    continue
                if (not member.isfile() or not member.name.startswith(prefix)
                        or ".." in PurePosixPath(member.name).parts):
                    raise ReleaseError(f"invalid archive member: {member.name}")
                name = member.name[len(prefix):]
                if name in files:
                    raise ReleaseError(f"duplicate archive member: {name}")
                files[name] = archive.extractfile(member).read()
    except tarfile.TarError as error:
        raise ReleaseError(f"invalid crate archive: {error}") from error
    return files


def validate_archive(data, root, crate, version):
    files = read_archive(data, crate, version)
    expected = {name: (root / name).read_bytes() for name in (*LICENSES, "README.md")}
    expected["Cargo.toml.orig"] = (root / "crates" / crate / "Cargo.toml").read_bytes()
    for name, contents in expected.items():
        if files.get(name) != contents:
            raise ReleaseError(f"{crate}: missing or altered {name}")
    for target in re.findall(r"\]\(([^)\s]+)", files["README.md"].decode()):
        if not target.startswith(("https://", "#", "mailto:")):
            raise ReleaseError(f"{crate}: README link is not package-portable: {target}")
    try:
        manifest = tomllib.loads(files["Cargo.toml"].decode())
        pkg = manifest["package"]
        if (pkg["name"], pkg["version"], pkg["license"]) != (crate, version, "MIT OR Apache-2.0"):
            raise ReleaseError(f"{crate}: packaged name/version/license mismatch")
        if "src/lib.rs" not in files:
            raise ReleaseError(f"{crate}: packaged library source is missing")
        if crate == "sdax-tokio":
            dependency = manifest["dependencies"]["sdax"]
            if dependency.get("version") not in (version, "=" + version) or "path" in dependency:
                raise ReleaseError("adapter archive has incorrect core dependency")
            lock = tomllib.loads(files["Cargo.lock"].decode())
            cores = [pkg for pkg in lock["package"] if pkg["name"] == "sdax"]
            if len(cores) != 1 or cores[0]["version"] != version:
                raise ReleaseError("adapter archive lock does not resolve the release core version")
    except (KeyError, ValueError) as error:
        raise ReleaseError(f"{crate}: malformed or missing package metadata: {error}") from error
    return files


def version_at(root, crate):
    return tomllib.loads((root / "crates" / crate / "Cargo.toml").read_text())["package"]["version"]


def package(root=ROOT, *, allow_dirty=False):
    """Never inspect an old target/package archive, even when Cargo fails."""
    archives = {}
    dirty = ["--allow-dirty"] if allow_dirty else []
    with tempfile.TemporaryDirectory(prefix="sdax-package-check-") as temporary:
        target = Path(temporary)
        run(["cargo", "package", "-p", "sdax", "--locked", "--offline", *dirty,
             "--target-dir", str(target / "verified-core")], cwd=root)
        version = version_at(root, "sdax")
        core = target / "verified-core/package" / f"sdax-{version}.crate"
        archives["sdax"] = core.read_bytes()
        # Cargo creates its own temporary registry for interdependent packages.
        # No source manifest, lockfile, global cache, or real package output is edited.
        run(["cargo", "package", "-p", "sdax", "-p", "sdax-tokio", "--no-verify",
             "--locked", "--offline", *dirty, "--target-dir", str(target / "staged")], cwd=root)
        version = version_at(root, "sdax-tokio")
        adapter = target / "staged/package" / f"sdax-tokio-{version}.crate"
        archives["sdax-tokio"] = adapter.read_bytes()
        for crate, data in archives.items():
            validate_archive(data, root, crate, version_at(root, crate))
    return archives


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-dirty", action="store_true")
    args = parser.parse_args()
    try:
        package(allow_dirty=args.allow_dirty)
    except (ReleaseError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f"archive check: {error}\n")
    print("PASS  fresh sdax archive: contents and Cargo build verified")
    print("PASS  fresh sdax-tokio staged archive: contents verified; registry build is a release prerequisite")


if __name__ == "__main__":
    main()
