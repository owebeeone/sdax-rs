"""Exercise release preparation without committing, tagging or publishing."""

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import release
from test_release_selection import GitFixture


class ReleaseHelperTests(unittest.TestCase):
    def test_github_release_creation_cannot_create_a_missing_tag(self):
        commands = []

        def runner(command, **kwargs):
            commands.append(command)
            return subprocess.CompletedProcess(command, 1 if "view" in command else 0, "", "")

        with patch.object(release, "require_tools"), patch.object(release, "run", side_effect=runner):
            release.create_github_release("v0.1.0")
        self.assertIn("--verify-tag", commands[-1])

    def test_shared_checks_forward_dirty_and_no_test(self):
        commands = []
        with patch.object(release, "run", side_effect=lambda command, **kw: commands.append(command)):
            release.run_gates(no_test=True, allow_dirty=True)
        self.assertEqual(len(commands), 1)
        self.assertIn("check_all.py", str(commands[0][2]))
        self.assertIn("--no-test", commands[0])
        self.assertIn("--allow-dirty", commands[0])

    def test_post_bump_failure_stops_before_commit_and_tag(self):
        events = []

        def gates(**kwargs):
            events.append(("gates", kwargs))
            if len(events) == 2:
                raise SystemExit("changed tree failed")

        def git(args, **kwargs):
            if args[0] in ("add", "commit", "tag"):
                self.fail("release mutation occurred before post-bump gates passed")
            return subprocess.CompletedProcess(args, 1 if "-q" in args else 0, "a" * 40, "")

        with patch.object(sys, "argv", ["release.py", "v0.2.0", "--no-test"]), \
             patch.object(release, "require_tools"), patch.object(release, "current_branch", return_value="main"), \
             patch.object(release, "warn_if_behind_upstream"), patch.object(release, "working_tree_clean"), \
             patch.object(release, "git", side_effect=git), \
             patch.object(release, "read_package_version", return_value="0.1.0"), \
             patch.object(release, "bump_versions", return_value=True), \
             patch.object(release, "refresh_cargo_lock"), \
             patch.object(release, "run_gates", side_effect=gates), \
             patch.object(release, "validate_tree_versions"), \
             patch.object(release, "commit_release") as commit, \
             patch.object(release, "ensure_tag") as tag:
            with self.assertRaisesRegex(SystemExit, "changed tree failed"):
                release.main()
            commit.assert_not_called()
            tag.assert_not_called()
        self.assertEqual(events, [("gates", {"no_test": True}),
                                  ("gates", {"no_test": True, "allow_dirty": True})])

    def test_workspace_release_commit_uses_gwz_with_exact_paths(self):
        with tempfile.TemporaryDirectory(prefix="sdax-gwz-release-test-") as temp:
            root = Path(temp)
            repo = root / "sdax-rs"
            repo.mkdir()
            (root / "gwz.conf").mkdir()
            (root / "gwz.conf/gwz.yml").write_text("fixture")
            commands = []
            with patch.object(release, "REPO", repo), patch.object(release, "require_tools"), \
                 patch.object(release, "run", side_effect=lambda command, **kw: commands.append(command)):
                release.commit_release(["Cargo.toml", "Cargo.lock"], "0.2.0")
            self.assertEqual(commands[0], ["gwz", "--root", root, "add", repo / "Cargo.toml", repo / "Cargo.lock"])
            self.assertEqual(commands[1][:9], ["gwz", "--root", root, "--target", "sdax-rs",
                                              "--target", "@root", "commit", "-m"])


class ReleasePushTests(GitFixture):
    def test_annotated_tag_push_and_retry_preserve_the_tag_object(self):
        self.git("tag", "-a", "v0.1.0", "-f", "-m", "annotation")
        tag_object = self.git("rev-parse", "refs/tags/v0.1.0")
        with tempfile.TemporaryDirectory(prefix="sdax-push-fixture-") as remote:
            subprocess.run(["git", "init", "--bare", remote], check=True, capture_output=True)
            self.git("remote", "add", "origin", remote)
            with patch.object(release, "REPO", self.root):
                release.push_release("main", "v0.1.0", expected_head=self.sha)
                release.push_release("main", "v0.1.0", expected_head=self.sha)
            remote_object = subprocess.run(["git", "-C", remote, "rev-parse", "refs/tags/v0.1.0"],
                                           check=True, capture_output=True, text=True).stdout.strip()
            self.assertEqual(remote_object, tag_object)

    def test_changed_local_tag_stops_before_push(self):
        (self.root / "marker").write_text("changed source")
        self.commit()
        self.git("tag", "-f", "v0.1.0")
        with patch.object(release, "REPO", self.root), patch.object(release, "run") as command:
            with self.assertRaises(SystemExit):
                release.push_release("main", "v0.1.0", expected_head=self.sha)
            command.assert_not_called()


if __name__ == "__main__":
    unittest.main()
