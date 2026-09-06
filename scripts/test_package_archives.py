"""Archive controls operate on explicit synthetic fixtures, never package outputs."""

import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from package_archives import package, validate_archive
from release_selection import ReleaseError


def tar_bytes(files, prefix):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as archive:
        for name, data in files.items():
            member = tarfile.TarInfo(f"{prefix}/{name}")
            member.size = len(data)
            archive.addfile(member, io.BytesIO(data))
    return output.getvalue()


class ArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sdax-archive-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.crate = self.root / "crates/sdax"
        self.crate.mkdir(parents=True)
        self.files = {
            "LICENSE-MIT": b"canonical MIT",
            "LICENSE-APACHE-2.0": b"canonical Apache",
            "README.md": b"[Guide](https://github.com/owebeeone/sdax-rs/blob/main/docs/README.md)\n",
            "Cargo.toml": b'[package]\nname="sdax"\nversion="1.2.3"\nlicense="MIT OR Apache-2.0"\n',
            "src/lib.rs": b"// source\n",
        }
        self.files["Cargo.toml.orig"] = self.files["Cargo.toml"]
        for name in ("LICENSE-MIT", "LICENSE-APACHE-2.0", "README.md"):
            (self.root / name).write_bytes(self.files[name])
        (self.crate / "Cargo.toml").write_bytes(self.files["Cargo.toml.orig"])

    def validate(self, files):
        return validate_archive(tar_bytes(files, "sdax-1.2.3"), self.root, "sdax", "1.2.3")

    def test_canonical_contents_pass(self):
        self.assertEqual(self.validate(self.files), self.files)

    def test_missing_or_changed_licenses_and_readme_refuse(self):
        for name in ("LICENSE-MIT", "LICENSE-APACHE-2.0", "README.md"):
            for missing in (False, True):
                with self.subTest(name=name, missing=missing):
                    files = dict(self.files)
                    if missing:
                        del files[name]
                    else:
                        files[name] = b"wrong"
                    with self.assertRaises(ReleaseError):
                        self.validate(files)

    def test_wrong_package_version_and_relative_readme_links_refuse(self):
        files = dict(self.files)
        files["Cargo.toml"] = files["Cargo.toml"].replace(b"1.2.3", b"1.2.4")
        with self.assertRaises(ReleaseError):
            self.validate(files)
        self.files["README.md"] = b"[Guide](docs/README.md)"
        (self.root / "README.md").write_bytes(self.files["README.md"])
        with self.assertRaisesRegex(ReleaseError, "link"):
            self.validate(self.files)

    def test_failed_cargo_cannot_reuse_an_existing_archive(self):
        stale = self.root / "target/package/sdax-1.2.3.crate"
        stale.parent.mkdir(parents=True)
        stale.write_bytes(tar_bytes(self.files, "sdax-1.2.3"))
        original = stale.read_bytes()
        commands = []

        def failed(command, **kwargs):
            commands.append(command)
            raise ReleaseError("Cargo failed")

        with patch("package_archives.run", side_effect=failed):
            with self.assertRaisesRegex(ReleaseError, "Cargo failed"):
                package(self.root, allow_dirty=True)
        self.assertIn("--allow-dirty", commands[0])
        self.assertNotEqual(commands[0][commands[0].index("--target-dir") + 1],
                            str(self.root / "target"))
        self.assertEqual(stale.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
