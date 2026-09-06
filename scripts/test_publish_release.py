"""Publication tests inject registry responses and a command recorder."""

import hashlib
import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch
from urllib.error import HTTPError, URLError

from publish_release import Cargo, publish_release
from registry_release import Registry, request
from release_selection import ReleaseError, resolve
from test_package_archives import tar_bytes
from test_release_selection import GitFixture


class RegistryTests(unittest.TestCase):
    def test_only_confirmed_404_is_absent(self):
        for status in (401, 403, 429, 500, 503):
            with self.subTest(status=status), patch("registry_release.urlopen", side_effect=
                    HTTPError("https://crates.io/fixture", status, "error", {}, None)):
                with self.assertRaises(ReleaseError):
                    request("https://crates.io/fixture", absent_ok=True)
        with patch("registry_release.urlopen", side_effect=
                   HTTPError("https://crates.io/fixture", 404, "absent", {}, None)):
            self.assertIsNone(request("https://crates.io/fixture", absent_ok=True))
            with self.assertRaises(ReleaseError):
                request("https://crates.io/fixture")
        with patch("registry_release.urlopen", side_effect=URLError("offline")):
            with self.assertRaises(ReleaseError):
                request("https://crates.io/fixture", absent_ok=True)

    def test_existing_version_requires_checksum_clean_provenance_and_identical_source(self):
        from release_selection import Selection
        selected = Selection("v1.2.3", "a" * 40, "a" * 40)
        for crate in ("sdax", "sdax-tokio"):
            original = {
                "Cargo.toml": f'[package]\nname="{crate}"\nversion="1.2.3"\n'.encode(),
                "src/lib.rs": b"// expected code",
                "Cargo.lock": b'[[package]]\nname="sdax"\nversion="1.2.3"\n',
                ".cargo_vcs_info.json": json.dumps({"git": {"sha1": selected.sha},
                                                    "path_in_vcs": f"crates/{crate}"}).encode(),
            }
            expected = tar_bytes(original, f"{crate}-1.2.3")
            registry = Registry({crate: expected})
            for case in ("valid", "checksum", "sha", "dirty", "path", "source", "missing", "yanked", "lock"):
                with self.subTest(crate=crate, case=case):
                    files = dict(original)
                    provenance = json.loads(files[".cargo_vcs_info.json"])
                    if case == "sha":
                        provenance["git"]["sha1"] = "b" * 40
                    if case == "dirty":
                        provenance["git"]["dirty"] = True
                    if case == "path":
                        provenance["path_in_vcs"] = "elsewhere"
                    files[".cargo_vcs_info.json"] = json.dumps(provenance).encode()
                    if case == "source":
                        files["src/lib.rs"] = b"// different code claiming the same SHA"
                    if case == "missing":
                        del files["src/lib.rs"]
                    if case == "lock":
                        files["Cargo.lock"] = files["Cargo.lock"].replace(b"1.2.3", b"1.2.4")
                    archive = tar_bytes(files, f"{crate}-1.2.3")
                    checksum = hashlib.sha256(archive).hexdigest()
                    metadata = json.dumps({"version": {"crate": crate, "num": "1.2.3",
                        "checksum": "0" * 64 if case == "checksum" else checksum,
                        "yanked": case == "yanked"}}).encode()
                    with patch("registry_release.request", side_effect=[metadata, archive]):
                        if case == "valid" or (case == "lock" and crate == "sdax"):
                            self.assertEqual(registry.existing(crate, selected), checksum)
                        else:
                            with self.assertRaises(ReleaseError):
                                registry.existing(crate, selected)

    def test_index_wait_requires_the_verified_core_checksum(self):
        registry = Registry({})
        good = {"vers": "1.2.3", "cksum": "a" * 64, "yanked": False}
        with patch("registry_release.request", return_value=None):
            self.assertFalse(registry.core_indexed("1.2.3", "a" * 64))
        with patch("registry_release.request", return_value=json.dumps(good).encode()):
            self.assertTrue(registry.core_indexed("1.2.3", "a" * 64))
            with self.assertRaises(ReleaseError):
                registry.core_indexed("1.2.3", "b" * 64)


class PublishingTests(GitFixture):
    def setUp(self):
        super().setUp()
        self.selected = resolve(self.root, "v0.1.0")
        self.events = []
        owner = self

        class FakeRegistry:
            present = set()
            indexed = True
            failed = False

            def existing(self, crate, selected):
                if self.failed:
                    raise ReleaseError("registry uncertainty")
                owner.events.append(("lookup", crate))
                return "checksum" if crate in self.present else None

            def core_indexed(self, version, checksum):
                owner.events.append(("index", version))
                return self.indexed

        class RecordingCargo:
            failed = False

            def package(self, crate):
                owner.events.append(("package", crate, owner.git("rev-parse", "HEAD")))
                if self.failed:
                    raise ReleaseError("adapter package failed")

            def publish(self, crate):
                owner.events.append(("publish", crate, owner.git("rev-parse", "HEAD")))
                owner.registry.present.add(crate)

        self.registry = FakeRegistry()
        self.cargo = RecordingCargo()

    def execute(self, **kwargs):
        publish_release(self.root, self.selected, self.registry, self.cargo,
                        remote=None, sleep=lambda _: None, attempts=2, **kwargs)

    def test_tag_source_is_both_tested_and_published_not_same_version_branch(self):
        (self.root / "marker").write_text("wrong same-version branch")
        self.commit()
        with self.assertRaises(ReleaseError):
            self.execute()
        self.assertEqual(self.events, [])
        self.git("checkout", "--detach", self.selected.sha)
        # Execute a source-sensitive check in the actual selected Git checkout.
        tested = subprocess.run([sys.executable, "-c",
            "from pathlib import Path; value = Path('marker').read_text(); "
            "assert value == 'release source'; print(value)"], cwd=self.root,
            check=True, text=True, capture_output=True)
        self.events.append(("tested", tested.stdout.strip(), self.git("rev-parse", "HEAD")))
        self.execute()
        relevant = [event for event in self.events if event[0] in ("tested", "package", "publish")]
        self.assertEqual(relevant, [
            ("tested", "release source", self.sha), ("package", "sdax", self.sha),
            ("publish", "sdax", self.sha), ("package", "sdax-tokio", self.sha),
            ("publish", "sdax-tokio", self.sha)])

    def test_retry_skips_verified_existing_versions(self):
        self.registry.present = {"sdax", "sdax-tokio"}
        self.execute()
        self.assertFalse(any(event[0] == "publish" for event in self.events))

    def test_uncertain_registry_moved_tag_and_failed_package_never_publish(self):
        self.registry.failed = True
        with self.assertRaises(ReleaseError):
            self.execute()
        self.registry.failed = False
        self.cargo.failed = True
        with self.assertRaises(ReleaseError):
            self.execute()
        self.cargo.failed = False
        self.git("tag", "-a", "v0.1.0", "-f", "-m", "changed")
        with self.assertRaises(ReleaseError):
            self.execute()
        self.assertFalse(any(event[0] == "publish" for event in self.events))

    def test_core_wait_is_bounded_and_adapter_not_published_on_timeout(self):
        self.registry.indexed = False
        with self.assertRaisesRegex(ReleaseError, "core.*resolvable"):
            self.execute()
        self.assertEqual([event[1] for event in self.events if event[0] == "publish"], ["sdax"])
        self.assertEqual(sum(event[0] == "index" for event in self.events), 2)

    def test_tag_is_rechecked_after_core_publication(self):
        def move_tag(crate):
            self.registry.present.add(crate)
            self.events.append(("publish", crate))
            self.git("tag", "-a", "v0.1.0", "-f", "-m", "changed")
        self.cargo.publish = move_tag
        with self.assertRaises(ReleaseError):
            self.execute()
        self.assertEqual([event for event in self.events if event[0] == "publish"], [("publish", "sdax")])

    def test_real_adapter_package_failure_stops_adapter_publication(self):
        self.registry.present = {"sdax"}
        self.cargo.failed = True
        with self.assertRaisesRegex(ReleaseError, "adapter package failed"):
            self.execute()
        self.assertFalse(any(event[0] == "publish" for event in self.events))

    def test_cargo_executor_requires_registry_build_and_checks_its_archive(self):
        cargo = Cargo(self.root, self.selected)
        commands = []

        def package_command(command, **kwargs):
            commands.append(command)
            target = Path(command[command.index("--target-dir") + 1])
            (target / "package").mkdir()
            (target / "package/sdax-tokio-0.1.0.crate").write_bytes(b"fresh fixture")

        with patch("publish_release.run", side_effect=package_command), \
             patch("publish_release.validate_archive") as validate, contextlib.redirect_stdout(io.StringIO()):
            cargo.package("sdax-tokio")
            validate.assert_called_once_with(b"fresh fixture", self.root, "sdax-tokio", "0.1.0")
        self.assertEqual(commands[0][:7], ["cargo", "package", "-p", "sdax-tokio", "--locked", "--registry", "crates-io"])
        self.assertFalse({"--allow-dirty", "--no-verify", "--offline"} & set(commands[0]))
        with patch("publish_release.subprocess.run") as command:
            cargo.publish("sdax-tokio")
            command.assert_called_once_with(["cargo", "publish", "-p", "sdax-tokio", "--locked",
                                             "--registry", "crates-io"], cwd=self.root, check=True)


if __name__ == "__main__":
    unittest.main()
