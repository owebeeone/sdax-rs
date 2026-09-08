"""Mechanical reduction selection, without model calls or fixture-specific hints."""
import importlib.util
from pathlib import Path
import unittest
spec = importlib.util.spec_from_file_location("prepare_freeze", Path(__file__).with_name("prepare_freeze.py"))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)

class PrepareTests(unittest.TestCase):
    def test_first_two_sections_preserved_whole_for_any_heading_names(self):
        for titles in [("Values and ownership", "Resources and effects", "Components"),
                       ("Types and ownership", "Retry, effects, and services", "Complete resource-bearing composition")]:
            first=f"## {titles[0]}\nComplete first section.\n```rust\nlet x = 1;\n```\n"
            second=f"## {titles[1]}\nComplete second section.\n"
            source="Intro\n"+first+second+f"## {titles[2]}\nOmitted third section.\n"
            self.assertEqual(prepare.reduce_context(source),first.rstrip()+"\n\n"+second)
    def test_insufficient_sections_are_rejected(self):
        with self.assertRaises(ValueError):
            prepare.reduce_context("Intro\n## Only one\nContent\n")

if __name__ == "__main__":
    unittest.main()
