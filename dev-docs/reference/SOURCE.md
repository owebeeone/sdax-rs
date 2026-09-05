# Reference: Python sdax Monte Carlo test

`sdax-python-test_monte_carlo.py` is the owner's own randomized stress test from the Python `sdax` package (MIT), fetched by the manager on 2026-09-05 from
`https://raw.githubusercontent.com/owebeeone/sdax/main/tests/test_monte_carlo.py` (sha256 `77e52f68f1e4d438c47aa0675f73cfb84003abf48a70207e0f4bc1e81620f754`, 159 lines).

It is a reference for the shape of the Rust testkit's Monte Carlo suite (random plans, random failures, seed printed on failure), not code to port. The Rust suite drives the pure machine through the scripted driver and checks the contract's invariants, which the Python version could not do.

`sdax-python-test_monte_dag.py` is the DAG-mode Monte Carlo test from the same package, fetched 2026-09-05 from `https://raw.githubusercontent.com/owebeeone/sdax/main/tests/test_monte_dag.py` (sha256 `f40b9d6d49e02f0f5badf13409e5e0d2928ee3e84d371bcfeda8dfba44499e19`). It seeds the generator per case and prints the seed to stderr on failure so a case can be replayed — the discipline the Rust suite follows.
