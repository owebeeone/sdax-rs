#!/bin/sh
# The quote gate: a `docs/` snippet is a *checked quotation* of a real test.
#
# A Markdown fence is a second copy of the code, so a test can evolve and the
# page silently drift — the failure the guide tests exist to prevent. The tests
# are the source of truth; the quote is checked, not trusted.
#
# For every fence in `docs/**/*.md` whose info string carries a `guide:<name>`
# attribute, the fence body must equal the whole file
# `crates/sdax-tokio/tests/guide/<name>.rs` (trailing newline normalised, and
# nothing else). A `guide:<name>` with no such file fails. A scenario file no
# page quotes is fine — it still runs as a test.
#
# Usage: scripts/check-guide-quotes.sh
set -eu
cd "$(dirname "$0")/.."

DOCS="${1:-docs}"
SCEN="${2:-crates/sdax-tokio/tests/guide}"

echo "== fences in $DOCS/**/*.md against $SCEN/"
python3 - "$DOCS" "$SCEN" <<'PY'
import pathlib
import re
import sys

docs = pathlib.Path(sys.argv[1])
scen = pathlib.Path(sys.argv[2])

OPEN = re.compile(r"^(?P<indent> {0,3})(?P<mark>`{3,}|~{3,})(?P<info>.*)$")
NAME = re.compile(r"(?:^|,)\s*guide:(?P<name>[A-Za-z0-9_]+)\s*(?:,|$)")

fences = []  # (page, line, name, info, body)
for md in sorted(docs.rglob("*.md")):
    lines = md.read_text().split("\n")
    i, close = 0, None
    while i < len(lines):
        line = lines[i]
        if close is None:
            m = OPEN.match(line)
            if m and not m.group("info").startswith(m.group("mark")[0]):
                close = re.compile(
                    r"^ {0,3}%s{%d,}\s*$" % (re.escape(m.group("mark")[0]), len(m.group("mark")))
                )
                start, info = i, m.group("info").strip()
                body = []
        else:
            if close.match(line):
                hit = NAME.search(info)
                if hit:
                    fences.append((md, start + 1, hit.group("name"), info, body))
                close = None
            else:
                body.append(line)
        i += 1
    if close is not None:
        print("FAIL  %s: fence opened at line %d is never closed" % (md, start + 1))
        sys.exit(1)

fail = 0
quoted = set()
for page, line, name, info, body in fences:
    where = "%s:%d" % (page, line)
    path = scen / ("%s.rs" % name)
    quoted.add(name)
    # The plan's fence grammar: `rust` first so every renderer still
    # highlights it, the `guide:` attribute after.
    if info.split(",")[0].strip() != "rust":
        print("FAIL  %s  info string is %r, not `rust,guide:%s`" % (where, info, name))
        print("      a fence opening `guide:%s` alone renders as grey text" % name)
        fail = 1
        continue
    if not path.is_file():
        print("FAIL  %s  guide:%s names no scenario: %s is missing" % (where, name, path))
        print("      write the test, or drop the `guide:` attribute from the fence")
        fail = 1
        continue
    want = path.read_text().rstrip("\n").split("\n")
    got = [ln.rstrip("\r") for ln in body]
    while got and got[-1] == "":
        got.pop()
    if got == want:
        print("PASS  %s  guide:%s == %s" % (where, name, path))
        continue
    fail = 1
    print("FAIL  %s  guide:%s != %s" % (where, name, path))
    n = min(len(got), len(want))
    at = next((k for k in range(n) if got[k] != want[k]), n)
    print("      first difference at fence line %d (%s:%d):" % (at + 1, path, at + 1))
    print("        page: %s" % (got[at] if at < len(got) else "<fence ends here>"))
    print("        file: %s" % (want[at] if at < len(want) else "<file ends here>"))
    print("      the test is the source of truth: copy the file into the fence")

extra = sorted(p.stem for p in scen.glob("*.rs") if p.stem not in quoted)
for name in extra:
    print("note  %s/%s.rs is quoted by no page (allowed; it still runs)" % (scen, name))

print("== %d fence(s), %d scenario(s), %d unquoted" % (len(fences), len(list(scen.glob("*.rs"))), len(extra)))
if fail:
    print("GUIDE QUOTE GATE FAILED")
    sys.exit(1)
print("GUIDE QUOTE GATE PASSED")
PY
