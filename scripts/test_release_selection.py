"""Local Git fixtures: no real tags, registry requests, or publication."""

from pathlib import Path
import subprocess
import tempfile
import unittest

from release_selection import ReleaseError, Selection, check_checkout, resolve, verify_tag


class GitFixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sdax-release-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Release fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n')
        for name in ("sdax", "sdax-tokio", "sdax-testkit"):
            crate = self.root / "crates" / name
            crate.mkdir(parents=True)
            manifest = f'[package]\nname = "{name}"\nversion = "0.1.0"\n'
            if name == "sdax-tokio":
                manifest += '[dependencies]\nsdax = { path = "../sdax", version = "0.1.0" }\n'
            (crate / "Cargo.toml").write_text(manifest)
        (self.root / "marker").write_text("release source")
        self.commit()
        self.sha = self.git("rev-parse", "HEAD")
        self.git("tag", "v0.1.0")

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.root), *args], check=True,
                              text=True, capture_output=True).stdout.strip()

    def commit(self):
        self.git("add", ".")
        self.git("commit", "-m", "isolated fixture")


class SelectionTests(GitFixture):
    def test_lightweight_and_annotated_tags(self):
        selected = resolve(self.root, "v0.1.0")
        self.assertEqual(selected.sha, self.sha)
        self.assertEqual(selected.tag_object, self.sha)
        self.git("tag", "-a", "v0.1.0", "-f", "-m", "annotation")
        annotated = resolve(self.root, "v0.1.0")
        self.assertEqual(annotated.sha, self.sha)
        self.assertNotEqual(annotated.tag_object, self.sha)

    def test_selection_reads_versions_from_tag_not_newer_branch(self):
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "9.0.0"\n')
        self.commit()
        self.assertEqual(resolve(self.root, "v0.1.0").sha, self.sha)

    def test_same_version_wrong_checkout_refuses(self):
        selected = resolve(self.root, "v0.1.0")
        (self.root / "marker").write_text("newer source, same version")
        self.commit()
        with self.assertRaisesRegex(ReleaseError, "checkout"):
            check_checkout(self.root, selected)
        self.git("checkout", "--detach", selected.sha)
        check_checkout(self.root, selected)
        self.assertEqual((self.root / "marker").read_text(), "release source")

    def test_invalid_and_missing_tags_including_branch_names(self):
        self.git("branch", "v2.0.0")
        for tag in ("main", "v1.0.0;echo bad", "v01.0.0", "v1.0.0\n", "v2.0.0", "v3.0.0"):
            with self.subTest(tag=tag), self.assertRaises(ReleaseError):
                resolve(self.root, tag)

    def test_version_or_dependency_mismatch_refuses(self):
        for relative in ("Cargo.toml", "crates/sdax/Cargo.toml",
                         "crates/sdax-tokio/Cargo.toml", "crates/sdax-testkit/Cargo.toml"):
            with self.subTest(relative=relative):
                path = self.root / relative
                before = path.read_text()
                path.write_text(before.replace('version = "0.1.0"', 'version = "0.2.0"'))
                self.commit()
                self.git("tag", "-f", "v0.1.0")
                with self.assertRaisesRegex(ReleaseError, "version"):
                    resolve(self.root, "v0.1.0")
                path.write_text(before)
        path = self.root / "crates/sdax-tokio/Cargo.toml"
        path.write_text(path.read_text().replace('path = "../sdax", version = "0.1.0"',
                                               'path = "../sdax", version = "0.9.0"'))
        self.commit()
        self.git("tag", "-f", "v0.1.0")
        with self.assertRaisesRegex(ReleaseError, "dependency"):
            resolve(self.root, "v0.1.0")

    def test_dirty_checkout_refuses(self):
        selected = resolve(self.root, "v0.1.0")
        (self.root / "marker").write_text("dirty")
        with self.assertRaisesRegex(ReleaseError, "clean"):
            check_checkout(self.root, selected)

    def test_tag_movement_and_deletion_refuse(self):
        selected = resolve(self.root, "v0.1.0")
        (self.root / "marker").write_text("moved")
        self.commit()
        self.git("tag", "-f", "v0.1.0")
        with self.assertRaises(ReleaseError):
            verify_tag(self.root, selected)
        self.git("tag", "-d", "v0.1.0")
        with self.assertRaises(ReleaseError):
            verify_tag(self.root, selected)

    def test_remote_tag_identity_is_checked_without_refreshing_local_tag(self):
        selected = resolve(self.root, "v0.1.0")
        with tempfile.TemporaryDirectory(prefix="sdax-remote-fixture-") as remote:
            subprocess.run(["git", "clone", "--bare", str(self.root), remote],
                           check=True, capture_output=True)
            self.git("remote", "add", "origin", remote)
            verify_tag(self.root, selected, remote="origin")
            self.git("tag", "-a", "v0.1.0", "-f", "-m", "changed annotation")
            self.git("push", "--force", "origin", "refs/tags/v0.1.0")
            self.git("tag", "-f", "v0.1.0", selected.sha)
            with self.assertRaisesRegex(ReleaseError, "moved"):
                verify_tag(self.root, selected, remote="origin")


if __name__ == "__main__":
    unittest.main()
