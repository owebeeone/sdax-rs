"""Source substitutions must preserve every documented dependency option."""

import importlib.util
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("consumer_check", Path(__file__).with_name("check-consumer-guide.py"))
consumer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(consumer)


class ConsumerSourceTests(unittest.TestCase):
    def test_git_substitution_preserves_recipe_and_pins_both_packages(self):
        path_manifest = consumer.build_local_manifest()
        repo = Path("/tmp/local git fixture")
        manifest = consumer.build_git_manifest(path_manifest, repo, "a" * 40)
        parsed = tomllib.loads(manifest)
        expected = tomllib.loads(path_manifest)
        for crate in ("sdax", "sdax-tokio"):
            dep = expected["dependencies"][crate]
            dep.pop("path")
            dep.update(git=repo.as_uri(), rev="a" * 40)
        self.assertEqual(parsed, expected)

    def test_missing_tokio_is_not_silently_repaired(self):
        with tempfile.TemporaryDirectory() as temporary:
            doc = Path(temporary) / "QuickStart.md"
            doc.write_text(consumer.QUICKSTART.read_text().replace(
                'tokio = { version = "=1.53.1", default-features = false, features = ["rt", "time", "test-util"] }', ""))
            with patch.object(consumer, "QUICKSTART", doc):
                manifest = consumer.build_git_manifest(consumer.build_local_manifest(), Path("/tmp/repo"), "b" * 40)
            self.assertNotIn("tokio", tomllib.loads(manifest).get("dev-dependencies", {}))


if __name__ == "__main__":
    unittest.main()
