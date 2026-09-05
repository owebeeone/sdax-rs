#!/bin/sh
# The architecture gate (LBT-001…005, adoption A2).
#
# Reads `cargo metadata` rather than the source: renames, optional deps,
# target-specific deps and dev-dependencies are all visible there, and none of
# them can be hidden by a `use` statement. A source scan cannot say that.
#
# Usage: scripts/check-architecture.sh
set -eu
cd "$(dirname "$0")/.."

CHECKER=$(mktemp -t sdax-arch)
trap 'rm -f "$CHECKER"' EXIT

cat > "$CHECKER" <<'PYEOF'
import json
import sys

md = json.load(sys.stdin)
members = set(md["workspace_members"])
pkgs = {p["id"]: p for p in md["packages"]}
local = {p["name"]: p for pid, p in pkgs.items() if pid in members}

# LBT-001: every workspace library has a reviewed role; an unknown one fails.
ROLES = {
    "sdax": "pure/contract",
    "sdax-tokio": "implementation",
    "sdax-testkit": "harness",
}
# LBT-003/004: the normal (non-dev, non-build) dependencies each role may have.
ALLOWED_NORMAL = {
    "sdax": set(),
    "sdax-tokio": {"sdax", "tokio", "tokio-util"},
    "sdax-testkit": {"sdax"},
}

fail = []


def check(cond, msg):
    if not cond:
        fail.append(msg)


for name in sorted(local):
    check(name in ROLES, "LBT-001: crate %r has no recorded role" % name)
for name in sorted(ROLES):
    check(name in local, "LBT-001: crate %r is recorded but not in the workspace" % name)

for name, pkg in sorted(local.items()):
    normal = set(d["name"] for d in pkg["dependencies"] if d["kind"] is None)
    extra = normal - ALLOWED_NORMAL.get(name, set())
    check(not extra, "LBT-003: %s has unexpected normal dependencies: %s" % (name, sorted(extra)))
    if name == "sdax":
        check(
            not normal,
            "LBT-003: the pure core must have zero normal dependencies, has %s" % sorted(normal),
        )

# LBT-005: the harness is publishable nowhere and reachable from no normal path.
testkit = local.get("sdax-testkit")
if testkit is None:
    fail.append("LBT-005: sdax-testkit is missing")
else:
    check(testkit.get("publish") == [], "LBT-005: sdax-testkit must set publish = false")
for name, pkg in sorted(local.items()):
    if name == "sdax-testkit":
        continue
    for d in pkg["dependencies"]:
        if d["name"] == "sdax-testkit":
            check(
                d["kind"] == "dev",
                "LBT-005: %s depends on the harness as a %s dependency"
                % (name, d["kind"] or "normal"),
            )

# LBT-003: the library must not acquire a Glade dependency, in any kind, under
# any name, anywhere in the resolved graph.
for pkg in pkgs.values():
    check(
        not pkg["name"].startswith("glade-"),
        "LBT-003: %s is in the dependency graph" % pkg["name"],
    )
for name, pkg in sorted(local.items()):
    for d in pkg["dependencies"]:
        renamed = d.get("rename") or ""
        check(
            not d["name"].startswith("glade-") and not renamed.startswith("glade"),
            "LBT-003: %s declares a glade dependency (%s)" % (name, d["name"]),
        )

# LBT-004: only the adapter may see tokio on a normal path.
for name, pkg in sorted(local.items()):
    if name == "sdax-tokio":
        continue
    for d in pkg["dependencies"]:
        if d["name"].startswith("tokio") and d["kind"] is None:
            fail.append("LBT-004: %s has a normal dependency on %s" % (name, d["name"]))

for name in sorted(local):
    print("  %-14s role: %s" % (name, ROLES.get(name, "???")))

if fail:
    print("")
    print("ARCHITECTURE GATE FAILED")
    for f in fail:
        print("  - %s" % f)
    sys.exit(1)
print("")
print("ARCHITECTURE GATE PASSED")
PYEOF

cargo metadata --format-version 1 --offline | python3 "$CHECKER"
