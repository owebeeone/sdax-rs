#!/usr/bin/env python3
"""Cut an sdax-rs release off ``main``, per RELEASE.md.

Automates:

  1. Gate the tree with the shared offline ``scripts/check_all.py`` inventory.
  2. Bump ``version`` in the workspace and every crate manifest, and refresh
     ``Cargo.lock``.
  3. Gate the bumped tree, then commit on ``main``: ``chore(release): sdax X.Y.Z``
     (through ``gwz`` when this repository is a workspace member).
  4. Tag that commit ``vX.Y.Z`` (lightweight). An existing tag is NEVER moved.
  5. Optionally ``--push`` main + tag atomically, and ``--github-release``
     (creates the GitHub Release that triggers crates.io publish).

Requires a clean working tree. The commit is skipped when every manifest
already carries the target version. Re-running after a successful release is
an idempotent no-op (and will create the tag if a prior run stopped before
tagging). Pushing is left to you unless ``--push`` is given.

This operates on your LOCAL ``main`` ref and does not fetch; it warns if
``main`` is behind its upstream.

Usage:
    python3 scripts/release.py vX.Y.Z
    python3 scripts/release.py vX.Y.Z --push
    python3 scripts/release.py vX.Y.Z --push --github-release
    python3 scripts/release.py vX.Y.Z --no-test
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

from release_selection import ReleaseError, TAG_PATTERN, resolve, validate_versions

REPO = Path(__file__).resolve().parent.parent
MANIFESTS = (
    REPO / "Cargo.toml",
    REPO / "crates" / "sdax" / "Cargo.toml",
    REPO / "crates" / "sdax-tokio" / "Cargo.toml",
    REPO / "crates" / "sdax-testkit" / "Cargo.toml",
)
GITHUB_REPO = "owebeeone/sdax-rs"


def fail(message: str) -> None:
    print(f"release: error: {message}", file=sys.stderr)
    raise SystemExit(1)


def log(message: str) -> None:
    print(f"release: {message}")


def run(
    command: list[object],
    *,
    cwd: Path | None = None,
    capture: bool = False,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    argv = [str(part) for part in command]
    log("$ " + " ".join(argv))
    result = subprocess.run(
        argv,
        cwd=str(cwd) if cwd is not None else None,
        capture_output=capture,
        text=True,
    )
    if check and result.returncode:
        if capture and result.stdout:
            print(result.stdout)
        if capture and result.stderr:
            print(result.stderr, file=sys.stderr)
        fail(f"command failed ({result.returncode}): {' '.join(argv)}")
    return result


def git(args: list[str], **kwargs) -> subprocess.CompletedProcess[str]:
    return run(["git", "-C", REPO, *args], **kwargs)


def require_tools(*names: str) -> None:
    missing = [name for name in names if shutil.which(name) is None]
    if missing:
        fail("missing required tools: " + ", ".join(missing))


def current_branch() -> str:
    branch = git(["branch", "--show-current"], capture=True).stdout.strip()
    if not branch:
        fail("detached HEAD -- switch to main before releasing")
    return branch


def warn_if_behind_upstream(branch: str) -> None:
    upstream = git(
        ["rev-parse", "--abbrev-ref", "--symbolic-full-name", f"{branch}@{{u}}"],
        capture=True,
        check=False,
    )
    if upstream.returncode != 0 or not upstream.stdout.strip():
        return
    name = upstream.stdout.strip()
    behind = git(
        ["rev-list", "--count", f"{branch}..{name}"],
        capture=True,
        check=False,
    ).stdout.strip()
    if behind and behind != "0":
        log(
            f"WARNING: local {branch} is {behind} commit(s) behind {name} "
            f"-- releasing local {branch}"
        )


def working_tree_clean() -> None:
    workspace = workspace_root()
    if workspace:
        require_tools("gwz")
        result = run(["gwz", "--root", workspace, "status", "--json"], capture=True)
        status = json.loads(result.stdout)
        if status.get("errors") or status.get("workspace_git_status", {}).get("clean") is not True:
            fail("gwz workspace must be clean before releasing")
        return
    status = git(["status", "--porcelain"], capture=True).stdout
    if status.strip():
        fail("working tree is not clean -- commit or stash changes first:\n" + status.rstrip())


def read_package_version() -> str:
    text = (REPO / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'(?m)^version = "([^"]+)"', text)
    if match is None:
        fail('no `version = "..."` found in workspace Cargo.toml')
    return match.group(1)


def bump_versions(old: str, new: str) -> bool:
    """Replace every ``version = "<old>"`` in workspace manifests. Returns True if any file changed."""
    changed = False
    needle = f'version = "{old}"'
    replacement = f'version = "{new}"'
    for path in MANIFESTS:
        text = path.read_text(encoding="utf-8")
        if needle not in text:
            if f'version = "{new}"' not in text:
                fail(f"{path.relative_to(REPO)} has neither {old} nor {new}")
            continue
        path.write_text(text.replace(needle, replacement), encoding="utf-8", newline="\n")
        log(f"bumped {path.relative_to(REPO)} {old} -> {new}")
        changed = True
    return changed


def refresh_cargo_lock() -> bool:
    lock = REPO / "Cargo.lock"
    before = lock.read_text(encoding="utf-8") if lock.is_file() else ""
    run(["cargo", "generate-lockfile", "--offline"], cwd=REPO)
    after = lock.read_text(encoding="utf-8")
    if after == before:
        log("Cargo.lock already matches Cargo.toml")
        return False
    log("refreshed Cargo.lock from Cargo.toml")
    return True


def run_gates(*, no_test: bool, allow_dirty: bool = False) -> None:
    command = [sys.executable, "-B", REPO / "scripts/check_all.py"]
    if no_test:
        command.append("--no-test")
    if allow_dirty:
        command.append("--allow-dirty")
    run(command, cwd=REPO)


def validate_tree_versions(version: str) -> None:
    try:
        validate_versions(lambda path: (REPO / path).read_text(), version)
    except (ReleaseError, KeyError, ValueError) as error:
        fail(str(error))


def workspace_root() -> Path | None:
    return next((path for path in REPO.parents if (path / "gwz.conf/gwz.yml").is_file()), None)


def commit_release(paths: list[str], version: str) -> None:
    message = f"chore(release): sdax {version}"
    workspace = workspace_root()
    if workspace:
        require_tools("gwz")
        run(["gwz", "--root", workspace, "add", *[REPO / path for path in paths]])
        # Include the root so GWZ commits its generated lock/integrity changes
        # alongside the member. Otherwise a successful release leaves staged
        # workspace metadata behind and its clean-tree retry refuses.
        run(["gwz", "--root", workspace, "--target", str(REPO.relative_to(workspace)),
             "--target", "@root", "commit", "-m", message])
    else:
        git(["add", *paths])
        git(["commit", "-m", message])


def ensure_tag(tag: str, target: str) -> None:
    existing = git(
        ["rev-parse", "-q", "--verify", f"refs/tags/{tag}^{{commit}}"],
        capture=True,
        check=False,
    )
    if existing.returncode == 0:
        if existing.stdout.strip() == target:
            log(f"tag {tag} already points at {target[:10]} -- leaving it")
            return
        fail(
            f"tag {tag} already exists at {existing.stdout.strip()[:10]}, not the "
            f"release commit {target[:10]} -- refusing to move a release tag"
        )
    git(["tag", tag, target])
    log(f"created tag {tag} -> {target[:10]}")


def push_release(branch: str, tag: str, *, expected_head: str) -> None:
    try:
        selected = resolve(REPO, tag)
    except ReleaseError as error:
        fail(str(error))
    if selected.sha != expected_head:
        fail("local release tag no longer points at the checked release commit")
    result = run(
        [
            "git",
            "-C",
            REPO,
            "push",
            "--atomic",
            "origin",
            f"{expected_head}:refs/heads/{branch}",
            f"{selected.tag_object}:refs/tags/{tag}",
        ],
        capture=True,
        check=False,
    )
    if result.returncode != 0:
        if result.stderr:
            print(result.stderr, file=sys.stderr)
        fail(
            f"atomic push of {expected_head[:10]} to {branch} and {tag} failed -- "
            "remote left unchanged"
        )
    log(f"pushed {branch} + {tag} to origin (atomic)")


def create_github_release(tag: str) -> None:
    require_tools("gh")
    existing = run(
        ["gh", "release", "view", tag, "--repo", GITHUB_REPO],
        capture=True,
        check=False,
    )
    if existing.returncode == 0:
        log(f"GitHub Release {tag} already exists")
        return
    notes = f"sdax {tag}. Publishing this release triggers the crates.io workflow."
    run(
        [
            "gh",
            "release",
            "create",
            tag,
            "--verify-tag",
            "--repo",
            GITHUB_REPO,
            "--title",
            tag,
            "--notes",
            notes,
        ]
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag", help="release tag, e.g. v0.1.0")
    parser.add_argument("--branch", default="main", help="branch to release from (default: main)")
    parser.add_argument("--no-test", action="store_true", help="skip `cargo test --workspace --locked`")
    parser.add_argument("--push", action="store_true", help="push the branch + tag to origin atomically")
    parser.add_argument(
        "--github-release",
        action="store_true",
        help="create the GitHub Release (requires --push); triggers crates.io publish",
    )
    args = parser.parse_args()

    if args.github_release and not args.push:
        fail("--github-release requires --push")
    if not TAG_PATTERN.fullmatch(args.tag):
        fail(f"tag must look like vX.Y.Z, got {args.tag!r}")
    version = args.tag[1:]

    require_tools("git", "cargo")
    branch = current_branch()
    if branch != args.branch:
        fail(f"on branch {branch!r} but releases are cut from {args.branch!r} -- switch first")

    warn_if_behind_upstream(args.branch)
    working_tree_clean()

    head = git(["rev-parse", "HEAD"], capture=True).stdout.strip()
    existing = git(
        ["rev-parse", "-q", "--verify", f"refs/tags/{args.tag}^{{commit}}"],
        capture=True,
        check=False,
    )
    already_cut = existing.returncode == 0
    if already_cut:
        if existing.stdout.strip() != head:
            fail(
                f"tag {args.tag} already exists at {existing.stdout.strip()[:10]} but "
                f"{args.branch} HEAD is {head[:10]}"
            )
        current = read_package_version()
        if current != version:
            fail(
                f"tag {args.tag} already points at HEAD but Cargo.toml version is "
                f"{current}, not {version}"
            )
        log(f"{args.tag} already exists at {args.branch} HEAD ({head[:10]}); release already cut")
        validate_tree_versions(version)
        run_gates(no_test=args.no_test)
        if args.push:
            push_release(args.branch, args.tag, expected_head=head)
        if args.github_release:
            create_github_release(args.tag)
        return

    validate_tree_versions(read_package_version())
    run_gates(no_test=args.no_test)

    current = read_package_version()
    if current != version:
        if not bump_versions(current, version):
            fail(f"Cargo.toml version is {current}, expected a bump to {version}")
        refresh_cargo_lock()
        validate_tree_versions(version)
        run_gates(no_test=args.no_test, allow_dirty=True)
        rels = [str(path.relative_to(REPO)) for path in MANIFESTS]
        rels.append("Cargo.lock")
        commit_release(rels, version)
        head = git(["rev-parse", "HEAD"], capture=True).stdout.strip()
        log(f"release commit -> {head[:10]}  (sdax {version})")
    else:
        log(f"{args.branch} already at version {version}; no new commit needed")

    ensure_tag(args.tag, head)
    if args.push:
        push_release(args.branch, args.tag, expected_head=head)
        if args.github_release:
            create_github_release(args.tag)
    else:
        log("next step (not done without --push):")
        log(f"  git -C {REPO} push origin {args.branch} {args.tag}")


if __name__ == "__main__":
    main()
