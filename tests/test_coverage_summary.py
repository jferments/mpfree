"""Tests for tools/ci/coverage_summary.py (no cargo needed)."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "tools/ci/coverage_summary.py"
spec = importlib.util.spec_from_file_location("coverage_summary", SCRIPT)
cov = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cov)


def entry(name: str, covered: int, count: int) -> dict:
    return {"filename": name, "summary": {"lines": {"covered": covered, "count": count}}}


class CoverageSummaryTests(unittest.TestCase):
    report = {"data": [{"files": [
        entry("/w/mpfree/rust/mpfree-core/src/lib.rs", 90, 100),
        entry("/w/mpfree/rust/mpfree-seq/src/lib.rs", 10, 40),
        entry("/w/mpfree/rust/mpfree-seq/src/step.rs", 0, 10),
        entry("/home/u/.cargo/registry/src/x/lib.rs", 5, 5),  # not ours
    ]}]}

    def test_files_group_by_crate_folder(self):
        self.assertEqual(cov.per_crate(self.report),
                         {"mpfree-core": (90, 100), "mpfree-seq": (10, 50)})

    def test_table_lists_lowest_first_and_untested_crates_as_zero(self):
        text = cov.table(cov.per_crate(self.report), ["mpfree-core", "mpfree-render", "mpfree-seq"])
        rows = [line for line in text.splitlines() if line.startswith("| `")]
        self.assertEqual([r.split("`")[1] for r in rows], ["mpfree-render", "mpfree-seq", "mpfree-core"])
        self.assertIn("| `mpfree-render` | 0 / 0 | 0.0% |", text)
        self.assertIn("| **all** | 100 / 150 | **66.7%** |", text)

    def test_crate_of_ignores_paths_outside_the_workspace(self):
        self.assertIsNone(cov.crate_of("/x/rust/other/src/lib.rs"))
        self.assertEqual(cov.crate_of("rust/mpfree-ui/src/wave.rs"), "mpfree-ui")


if __name__ == "__main__":
    unittest.main()
