#!/bin/sh
# Suite (a), the error-code half: extract every `compile_fail` doctest from
# crates/sdax/src/compile_fail.rs and assert the error code each one claims.
#
# `cargo test --doc` already proves the programs do not compile; only rustc can
# say *why*, and the gate inventory's decode rows name specific codes.
#
# Usage: scripts/compile-fail.sh [target-dir]
set -eu
cd "$(dirname "$0")/.."
TARGET="${1:-${CARGO_TARGET_DIR:-target}}"
export CARGO_TARGET_DIR="$TARGET"

echo "== toolchain"
rustc --version
echo "== building the rlib the witnesses link against"
cargo build -p sdax --locked --offline >/dev/null

# Cargo refreshes this public artifact for the just-completed build. The deps
# directory may also contain rlibs from another toolchain; never select by name.
RLIB="$TARGET/debug/libsdax.rlib"
if [ ! -f "$RLIB" ]; then
  echo "sdax rlib not found at $RLIB" >&2
  exit 2
fi

WORK=$(mktemp -d)
# Retain compiler witnesses unless the owner requests cleanup.
python3 - "$WORK" <<'PY'
import pathlib, re, sys
work = pathlib.Path(sys.argv[1])
src = pathlib.Path("crates/sdax/src/compile_fail.rs").read_text().splitlines()
blocks, cur, title = [], None, "?"
for line in src:
    body = line[4:] if line.startswith("//! ") else line[3:] if line.startswith("//!") else None
    if body is None:
        continue
    if body.startswith("## "):
        title = body[3:].split(" —")[0].strip()
    if body.strip() == "```compile_fail":
        cur = []
        continue
    if cur is not None and body.strip() == "```":
        blocks.append((title, cur))
        cur = None
        continue
    if cur is not None:
        cur.append(body)
if not blocks:
    sys.exit("no compile_fail blocks found")
for i, (title, lines) in enumerate(blocks):
    want = "?"
    for l in lines:
        m = re.match(r"\s*// expect: (E\d+)", l)
        if m:
            want = m.group(1)
    name = title.replace(" ", "_").replace("-", "_")
    path = work / f"{i:02d}_{name}.rs"
    path.write_text("fn main() {\n" + "\n".join(lines) + "\n}\n")
    (work / f"{i:02d}_{name}.want").write_text(want)
print(len(blocks))
PY

FAIL=0
COUNT=0
for f in "$WORK"/*.rs; do
  COUNT=$((COUNT + 1))
  want=$(cat "${f%.rs}.want")
  name=$(basename "$f")
  if out=$(rustc --edition 2021 --crate-type bin -L "$TARGET/debug/deps" \
            --extern sdax="$RLIB" -o /dev/null "$f" 2>&1); then
    echo "FAIL  $name: compiled (expected $want)"
    FAIL=1
    continue
  fi
  if [ "$want" = "?" ]; then
    echo "FAIL  $name: no '// expect: Ennnn' line"
    FAIL=1
  elif echo "$out" | grep -q "error\[$want\]"; then
    echo "PASS  $name: rejected with $want"
  else
    echo "FAIL  $name: rejected, but not with $want:"
    echo "$out" | grep -m2 '^error' | sed 's/^/      /'
    FAIL=1
  fi
done
echo "== $COUNT witnesses"
exit $FAIL
