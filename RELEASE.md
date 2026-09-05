# Releasing sdax-rs

Cut releases from `main` with `scripts/release.py`. Tags are immutable.
Publishing to crates.io is triggered by creating the GitHub Release for that
tag (`.github/workflows/publish.yml`).

## Prerequisites

- Clean `sdax-rs` worktree on `main`.
- `git`, `cargo`, and (for `--github-release`) `gh`.
- crates.io token in the GitHub `crates-io` environment as
  `CARGO_REGISTRY_TOKEN` (publish workflow only).

When this repo is a gwz member, bump/commit/tag happen **inside** `sdax-rs`.
After a local release commit, refresh the workspace pin with `gwz capture`
from the workspace root before committing the root.

## Cut a release

```sh
# Gate, bump crate versions, commit, tag. Does not push.
python3 scripts/release.py vX.Y.Z

# Also push main + tag atomically.
python3 scripts/release.py vX.Y.Z --push

# Also create the GitHub Release (triggers crates.io publish).
python3 scripts/release.py vX.Y.Z --push --github-release
```

`--github-release` requires `--push`. A pre-existing tag is accepted only when
it already points at the current release commit.

Gates (same bar as CI):

- `cargo fmt --check`
- `cargo metadata --locked`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `cargo package -p sdax --locked`

`sdax-testkit` is not published. `sdax-tokio` is published after `sdax` of the
same version is on crates.io (the workflow does this in order). Packaging
`sdax-tokio` before that first `sdax` publish will fail; do not add it to the
pre-publish gate.

## Skip tests only

```sh
python3 scripts/release.py vX.Y.Z --no-test
```

Still runs fmt, clippy, lock-file check, and `cargo package -p sdax`.
