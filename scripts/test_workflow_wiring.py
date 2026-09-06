"""Static workflow regression guards; these do not execute GitHub Actions."""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[1]


def jobs(source):
    start = source.index("\njobs:\n")
    return {match[1]: match[2] for match in re.finditer(
        r"^  ([a-z_]+):\n(.*?)(?=^  [a-z_]+:\n|\Z)", source[start:], re.M | re.S)}


def verify(ci, publish):
    ci_jobs, release_jobs = jobs(ci), jobs(publish)
    if "python3 -B scripts/check_all.py\n" not in ci_jobs["test"]:
        raise ValueError("CI must invoke the complete shared checks")
    if "python3 -B scripts/check_all.py --msrv-only" not in ci_jobs["msrv"]:
        raise ValueError("CI must invoke shared MSRV builds")
    select = release_jobs["select"]
    for token in ("sha: ${{ steps.resolve.outputs.sha }}", "tag_object: ${{ steps.resolve.outputs.tag_object }}",
                  "fetch-depth: 0", "id: resolve", "python3 -B scripts/release_selection.py select"):
        if token not in select:
            raise ValueError("missing source selection output or full tag history")
    for job in ("test", "msrv", "publish"):
        body = release_jobs[job]
        if "ref: ${{ needs.select.outputs.sha }}" not in body:
            raise ValueError(f"{job} must check out the resolved SHA")
        if "RELEASE_SHA: ${{ needs.select.outputs.sha }}" not in body:
            raise ValueError(f"{job} must pass the resolved SHA")
        if "RELEASE_TAG_OBJECT: ${{ needs.select.outputs.tag_object }}" not in body:
            raise ValueError(f"{job} must pass the original tag object")
        if "python3 -B scripts/release_selection.py verify" not in body:
            raise ValueError(f"{job} must verify source identity")
    if "needs: [select, test, msrv]" not in release_jobs["publish"]:
        raise ValueError("publish must depend on both full checks and MSRV")
    if "python3 -B scripts/check_all.py\n" not in release_jobs["test"]:
        raise ValueError("release must invoke the complete shared checks")
    if "python3 -B scripts/check_all.py --msrv-only" not in release_jobs["msrv"]:
        raise ValueError("release must invoke shared MSRV builds")
    if "python3 -B scripts/publish_release.py" not in release_jobs["publish"]:
        raise ValueError("publish must use the verified publication executor")
    if "cargo publish" in publish or "--no-test" in publish or "--allow-dirty" in publish:
        raise ValueError("workflow bypasses the verified publication path")
    for line in publish.splitlines():
        if "${{" in line and ("inputs.tag" in line or "github.event.release.tag_name" in line):
            if not line.strip().startswith("RELEASE_TAG:"):
                raise ValueError("event input must be passed through an environment value")


class WorkflowTests(unittest.TestCase):
    def test_shared_bar_and_selected_commit_wiring(self):
        verify((ROOT / ".github/workflows/ci.yml").read_text(),
               (ROOT / ".github/workflows/publish.yml").read_text())

    def test_missing_gate_or_source_binding_is_rejected(self):
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        publish = (ROOT / ".github/workflows/publish.yml").read_text()
        for before, after in (("needs: [select, test, msrv]", "needs: [select, test]"),
                              ("ref: ${{ needs.select.outputs.sha }}", "ref: main"),
                              ("python3 -B scripts/check_all.py\n", "cargo test\n"),
                              ("python3 -B scripts/release_selection.py verify", "true")):
            with self.subTest(before=before), self.assertRaises(ValueError):
                verify(ci, publish.replace(before, after))


if __name__ == "__main__":
    unittest.main()
