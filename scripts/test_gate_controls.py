"""Negative fixtures for the newly wired architecture, quote, code and doc gates."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ExistingGateControls(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sdax-gate-control-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def execute(self, command, **kwargs):
        return subprocess.run(command, cwd=self.root, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, **kwargs)

    def copy_script(self, name):
        (self.root / "scripts").mkdir(exist_ok=True)
        shutil.copyfile(ROOT / "scripts" / name, self.root / "scripts" / name)

    def minimal_core(self):
        crate = self.root / "crates/sdax"
        (crate / "src").mkdir(parents=True)
        (crate / "src/lib.rs").write_text("pub fn fixture() {}\n")
        (crate / "Cargo.toml").write_text('[package]\nname="sdax"\nversion="0.0.0"\nedition="2021"\n')
        (self.root / "Cargo.toml").write_text('[workspace]\nresolver="2"\nmembers=["crates/sdax"]\n')
        result = self.execute(["cargo", "generate-lockfile", "--offline"])
        self.assertEqual(result.returncode, 0, result.stdout)
        return crate

    def test_architecture_rejects_an_unexpected_normal_dependency(self):
        self.copy_script("check-architecture.sh")
        self.minimal_core()
        for name in ("sdax-tokio", "sdax-testkit", "unexpected"):
            crate = self.root / "crates" / name
            (crate / "src").mkdir(parents=True)
            (crate / "src/lib.rs").write_text("// fixture\n")
            manifest = f'[package]\nname="{name}"\nversion="0.0.0"\nedition="2021"\n'
            if name == "sdax-testkit":
                manifest += "publish=false\n"
            if name == "sdax-tokio":
                manifest += '[dependencies]\nunexpected={path="../unexpected"}\n'
            (crate / "Cargo.toml").write_text(manifest)
        (self.root / "Cargo.toml").write_text('[workspace]\nresolver="2"\nmembers=["crates/*"]\n')
        self.assertEqual(self.execute(["cargo", "generate-lockfile", "--offline"]).returncode, 0)
        result = self.execute(["sh", "scripts/check-architecture.sh"])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("LBT-003: sdax-tokio has unexpected normal dependencies", result.stdout)

    def test_guide_quote_drift_is_rejected(self):
        self.copy_script("check-guide-quotes.sh")
        (self.root / "docs").mkdir()
        (self.root / "guide").mkdir()
        (self.root / "guide/example.rs").write_text("fn example() {}\n")
        (self.root / "docs/example.md").write_text("```rust,guide:example\nfn drifted() {}\n```\n")
        result = self.execute(["sh", "scripts/check-guide-quotes.sh", "docs", "guide"])
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("GUIDE QUOTE GATE FAILED", result.stdout)

    def test_wrong_compile_error_code_is_rejected(self):
        self.copy_script("compile-fail.sh")
        crate = self.minimal_core()
        (crate / "src/compile_fail.rs").write_text(
            "//! ## Wrong code witness\n//! ```compile_fail\n//! // expect: E0308\n"
            "//! no_such_function();\n//! ```\n")
        env = {**os.environ, "CARGO_TARGET_DIR": str(self.root / "target")}
        result = self.execute(["sh", "scripts/compile-fail.sh"], env=env)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("rejected, but not with E0308", result.stdout)

    def test_rustdoc_broken_link_fails_with_warnings_denied(self):
        crate = self.minimal_core()
        (crate / "src/lib.rs").write_text("/// See [ThisTypeDoesNotExist].\npub fn fixture() {}\n")
        result = self.execute(["cargo", "doc", "--workspace", "--no-deps", "--locked", "--offline"],
                              env={**os.environ, "RUSTDOCFLAGS": "-D warnings",
                                   "CARGO_TARGET_DIR": str(self.root / "target")})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unresolved link", result.stdout)


if __name__ == "__main__":
    unittest.main()
