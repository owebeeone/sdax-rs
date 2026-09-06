#!/usr/bin/env python3
"""Test the documented dependency recipe outside the Cargo workspace, offline.

Requires Python 3.11+. Only SDAX Git source coordinates are replaced with local
paths. Tokio is taken verbatim from the docs; the negative controls then remove
its direct dependency or test-util feature. No dependency is silently supplied.
"""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
QUICKSTART = ROOT / "docs/QuickStart.md"
GUIDE_SRC = ROOT / "crates/sdax-tokio/tests/guide/simple_hold.rs"
HEADER = '[package]\nname = "sdax-guide-consumer-check"\nversion = "0.1.0"\nedition = "2021"\n\n'


def build_local_manifest():
    match = re.search(r"^```toml\n(.*?)^```", QUICKSTART.read_text(), re.M | re.S)
    if not match:
        raise RuntimeError("QuickStart has no TOML dependency recipe")
    recipe = match[1]
    parsed = tomllib.loads(recipe)
    sources = []
    for name in ("sdax", "sdax-tokio"):
        dependency = parsed["dependencies"][name]
        if not isinstance(dependency, dict) or "git" not in dependency:
            raise RuntimeError(f"Expected an unpublished Git dependency for {name}")
        sources.append({k: dependency[k] for k in ("git", "rev", "tag", "branch") if k in dependency})
        pattern = rf"^{re.escape(name)}\s*=\s*\{{([^\n]*)\}}\s*$"

        def replace_source(match):
            fields = re.sub(r'\b(?:git|rev|tag|branch)\s*=\s*"[^"\n]*"\s*,?\s*', '', match[1])
            path = json.dumps(str(ROOT / "crates" / name))
            return f"{name} = {{ path = {path}, {fields.strip()} }}"

        recipe, count = re.subn(pattern, replace_source, recipe, flags=re.M)
        if count != 1:
            raise RuntimeError(f"Expected one inline dependency table for {name}")
    if sources[0] != sources[1]:
        raise RuntimeError("SDAX dependencies must use the same Git source and revision")
    local = tomllib.loads(recipe)
    expected = parsed.copy()
    expected["dependencies"] = {name: value.copy() if isinstance(value, dict) else value
                                for name, value in parsed["dependencies"].items()}
    for name in ("sdax", "sdax-tokio"):
        dep = expected["dependencies"][name]
        for key in ("git", "rev", "tag", "branch"):
            dep.pop(key, None)
        dep["path"] = str(ROOT / "crates" / name)
    if local != expected:
        raise RuntimeError("Local substitution altered the documented dependency recipe")
    return HEADER + recipe


def without_tokio(manifest):
    changed, count = re.subn(r"^tokio\s*=.*\n", "", manifest, flags=re.M)
    if count != 1:
        raise RuntimeError("Expected one direct inline Tokio dependency for negative control")
    return changed


def without_test_util(manifest):
    def remove_feature(match):
        return re.sub(r'"test-util"\s*,?\s*', '', match[0])
    changed = re.sub(r"^tokio\s*=.*$", remove_feature, manifest, flags=re.M)
    if changed == manifest:
        raise RuntimeError("Expected documented test-util feature for negative control")
    return changed


def build_git_manifest(manifest, repository, revision):
    expected = tomllib.loads(manifest)
    for name in ("sdax", "sdax-tokio"):
        pattern = rf"^({re.escape(name)}\s*=\s*\{{)\s*path\s*=\s*\"[^\"\n]*\""
        source = f"git = {json.dumps(repository.as_uri())}, rev = {json.dumps(revision)}"
        manifest, count = re.subn(pattern, lambda match: match[1] + source, manifest, flags=re.M)
        if count != 1:
            raise RuntimeError(f"Expected a local {name} dependency")
        dependency = expected["dependencies"][name]
        dependency.pop("path")
        dependency.update(git=repository.as_uri(), rev=revision)
    if tomllib.loads(manifest) != expected:
        raise RuntimeError("Git substitution altered the documented dependency recipe")
    return manifest


def run_case(consumer, manifest, label, diagnostic=None, *, offline=True, env=None):
    (consumer / "Cargo.toml").write_text(manifest)
    result = subprocess.run(
        ["cargo", "test", *(["--offline"] if offline else []), "--manifest-path", str(consumer / "Cargo.toml"),
         "--test", "simple_hold"], cwd=consumer, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env,
    )
    if diagnostic is None:
        passed = result.returncode == 0
    else:
        passed = result.returncode != 0 and diagnostic(result.stdout)
    if not passed:
        raise RuntimeError(f"{label} failed its expected control\n{result.stdout}")
    print(f"PASS  {label}")


def check_git_source(temporary, path_manifest):
    """Use only file:// Git and a directory source; no registry network access.

    Cargo's offline mode refuses an uncached Git revision, even a file:// URL.
    Replace crates.io with an offline-generated vendor directory, then allow
    the local Git fetch into an isolated Cargo home. The Git fixture contains
    the current source, including uncommitted fixes, and never uses an old HEAD.
    """
    repository = temporary / "git-source"
    repository.mkdir()
    for name in ("Cargo.toml", "Cargo.lock", "README.md", "LICENSE-MIT", "LICENSE-APACHE-2.0"):
        shutil.copyfile(ROOT / name, repository / name)
    shutil.copytree(ROOT / "crates", repository / "crates", ignore=shutil.ignore_patterns("target"))
    git = lambda *args: subprocess.run(["git", "-C", str(repository), *args], check=True,
                                      text=True, capture_output=True).stdout.strip()
    git("init", "-b", "main")
    git("config", "user.name", "SDAX consumer fixture")
    git("config", "user.email", "fixture@example.invalid")
    git("config", "commit.gpgsign", "false")
    git("add", ".")
    git("commit", "-m", "Isolated current-source consumer fixture")
    revision = git("rev-parse", "HEAD")
    vendor = temporary / "vendor"
    vendored = subprocess.run(["cargo", "vendor", "--locked", "--offline", str(vendor)],
                             cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if vendored.returncode:
        raise RuntimeError("Failed to prepare offline Git consumer dependencies:\n" + vendored.stderr)
    consumer = temporary / "git-consumer"
    (consumer / "tests").mkdir(parents=True)
    shutil.copyfile(GUIDE_SRC, consumer / "tests/simple_hold.rs")
    (consumer / ".cargo").mkdir()
    (consumer / ".cargo/config.toml").write_text(vendored.stdout + "\n[net]\noffline = false\n")
    env = {**os.environ, "CARGO_HOME": str(temporary / "cargo-home"), "CARGO_NET_OFFLINE": "false"}
    manifest = build_git_manifest(path_manifest, repository, revision)
    run_case(consumer, manifest, "documented guide from pinned local Git source", offline=False, env=env)
    lock = tomllib.loads((consumer / "Cargo.lock").read_text())
    selected = [package["source"] for package in lock["package"] if package["name"] in ("sdax", "sdax-tokio")]
    if len(selected) != 2 or len(set(selected)) != 1 or not selected[0].endswith("#" + revision):
        raise RuntimeError("Git consumer did not select both crates from the same pinned revision")
    run_case(consumer, build_git_manifest(path_manifest, repository, "0" * 40),
             "missing Git revision is rejected", lambda output: "0000000000000000000000000000000000000000" in output,
             offline=False, env=env)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-clean", action="store_true", help="Retain temporary consumer directory")
    args = parser.parse_args()
    consumer = Path(tempfile.mkdtemp(prefix="sdax-guide-consumer-"))
    try:
        (consumer / "tests").mkdir()
        shutil.copyfile(GUIDE_SRC, consumer / "tests/simple_hold.rs")
        manifest = build_local_manifest()
        run_case(consumer, manifest, "documented guide builds and runs")
        run_case(consumer, without_tokio(manifest), "missing direct Tokio is rejected",
                 lambda output: "error[E043" in output and "tokio" in output)
        run_case(consumer, without_test_util(manifest), "missing test-util is rejected",
                 lambda output: "error[E0599]" in output and "start_paused" in output)
        check_git_source(consumer, manifest)
    finally:
        if args.no_clean:
            print(f"consumer directory retained at {consumer}")
        else:
            shutil.rmtree(consumer)


if __name__ == "__main__":
    main()
