"""Select an immutable tag and validate its source before release operations."""

import argparse
from dataclasses import dataclass
import os
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TAG_PATTERN = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
CRATES = ("sdax", "sdax-tokio", "sdax-testkit")


class ReleaseError(RuntimeError):
    """A release prerequisite was not established; publication must stop."""


@dataclass(frozen=True)
class Selection:
    tag: str
    sha: str
    tag_object: str

    def __post_init__(self):
        if not TAG_PATTERN.fullmatch(self.tag):
            raise ReleaseError("tag must be vX.Y.Z without leading zeroes")
        for value in (self.sha, self.tag_object):
            if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", value):
                raise ReleaseError("expected a full Git object ID")

    @property
    def version(self):
        return self.tag[1:]


def git(root, *args):
    result = subprocess.run(["git", "-C", str(root), *args], text=True,
                            capture_output=True, timeout=60)
    if result.returncode:
        raise ReleaseError(f"git {args[0]} failed: {result.stderr.strip()}")
    return result.stdout.strip()


def validate_versions(read, version):
    """Read manifests through a caller-selected tree, never an implicit HEAD."""
    workspace = tomllib.loads(read("Cargo.toml"))
    if workspace["workspace"]["package"]["version"] != version:
        raise ReleaseError("workspace version does not match the release tag")
    for name in CRATES:
        manifest = tomllib.loads(read(f"crates/{name}/Cargo.toml"))
        if manifest["package"]["name"] != name or manifest["package"]["version"] != version:
            raise ReleaseError(f"{name} package version does not match the release tag")
        if name == "sdax-tokio":
            dep = manifest["dependencies"]["sdax"]
            if dep.get("version") not in (version, "=" + version):
                raise ReleaseError("sdax-tokio's sdax dependency version differs from release")


def resolve(root, tag):
    if not TAG_PATTERN.fullmatch(tag):
        raise ReleaseError("tag must be vX.Y.Z without leading zeroes")
    ref = f"refs/tags/{tag}"
    selected = Selection(tag, git(root, "rev-parse", "--verify", f"{ref}^{{commit}}"),
                         git(root, "rev-parse", "--verify", ref))
    validate_versions(lambda path: git(root, "show", f"{selected.sha}:{path}"), selected.version)
    return selected


def verify_tag(root, selected, remote=None):
    ref = f"refs/tags/{selected.tag}"
    if remote:
        rows = git(root, "ls-remote", "--exit-code", remote, ref, ref + "^{}")
        refs = dict((name, oid) for oid, name in (row.split() for row in rows.splitlines()))
        tag_object = refs.get(ref)
        sha = refs.get(ref + "^{}", tag_object)
    else:
        tag_object = git(root, "rev-parse", "--verify", ref)
        sha = git(root, "rev-parse", "--verify", f"{ref}^{{commit}}")
    if (sha, tag_object) != (selected.sha, selected.tag_object):
        raise ReleaseError("release tag moved or disappeared after selection")


def check_checkout(root, selected):
    if git(root, "rev-parse", "HEAD") != selected.sha:
        raise ReleaseError("checkout is not the selected release commit")
    if git(root, "status", "--porcelain", "--untracked-files=all"):
        raise ReleaseError("release requires a clean checkout")
    validate_versions(lambda path: (Path(root) / path).read_text(), selected.version)


def from_environment():
    return Selection(os.environ["RELEASE_TAG"], os.environ["RELEASE_SHA"],
                     os.environ["RELEASE_TAG_OBJECT"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("select", "verify"))
    args = parser.parse_args()
    try:
        if args.action == "select":
            selected = resolve(ROOT, os.environ["RELEASE_TAG"])
            output = f"sha={selected.sha}\ntag_object={selected.tag_object}\n"
            with open(os.environ["GITHUB_OUTPUT"], "a") as destination:
                destination.write(output)
            print(output, end="")
        else:
            selected = from_environment()
            check_checkout(ROOT, selected)
            verify_tag(ROOT, selected, remote="origin")
            print(f"Verified {selected.tag} at {selected.sha}")
    except (ReleaseError, KeyError, ValueError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f"release selection: {error}\n")


if __name__ == "__main__":
    main()
