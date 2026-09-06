"""The shared bar must propagate failure and preserve non-test release checks."""

import subprocess
import contextlib
import io
import unittest

from check_all import checks, run_checks
from release_selection import ReleaseError


class GateTests(unittest.TestCase):
    def test_inventory_and_skip_tests_only(self):
        full = {check.name: check for check in checks()}
        self.assertEqual(set(full), {"lockfile", "fmt", "clippy", "tests", "architecture",
                                   "compile-fail", "guide-quotes", "rustdoc", "script-tests",
                                   "consumer", "archives"})
        skipped = {check.name for check in checks(no_test=True)}
        self.assertEqual(skipped, set(full) - {"tests"})
        self.assertEqual(full["rustdoc"].env["RUSTDOCFLAGS"], "-D warnings")
        self.assertNotIn("--allow-dirty", full["archives"].command)
        dirty = {check.name: check for check in checks(allow_dirty=True)}
        self.assertIn("--allow-dirty", dirty["archives"].command)
        msrv = checks(msrv_only=True)
        self.assertEqual(len(msrv), 2)
        self.assertTrue(all(check.command[:3] == ["cargo", "+1.75", "build"] for check in msrv))
        self.assertTrue(all("--locked" in check.command and "--offline" in check.command for check in msrv))

    def test_every_failure_stops_subsequent_gates(self):
        inventory = checks()
        for failing in range(len(inventory)):
            with self.subTest(gate=inventory[failing].name):
                called = []

                def runner(command, **kwargs):
                    called.append(command)
                    return subprocess.CompletedProcess(command, 17 if len(called) == failing + 1 else 0, "fixture")

                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()), \
                     self.assertRaisesRegex(ReleaseError, inventory[failing].name):
                    run_checks(inventory, runner=runner)
                self.assertEqual(called, [check.command for check in inventory[:failing + 1]])


if __name__ == "__main__":
    unittest.main()
