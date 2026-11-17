#!/usr/bin/env python3
"""Tests for tools/ci/store.py: keeping a run's files on the CI VM (no GitHub artifacts)."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("store", Path(__file__).resolve().parent / "store.py")
store = importlib.util.module_from_spec(spec)
spec.loader.exec_module(store)

DAY = 86400.0
NOW = 1_800_000_000.0


class StoreTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.store = self.root / "store"
        self.store.mkdir()
        self.src = self.root / "src"
        self.src.mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def env(self, run="100", attempt="1", ref="refs/pull/442/merge", ref_name="442/merge", event="pull_request"):
        return {"MPFREE_STORE": str(self.store), "GITHUB_RUN_ID": run, "GITHUB_RUN_ATTEMPT": attempt,
                "GITHUB_REF": ref, "GITHUB_REF_NAME": ref_name, "GITHUB_EVENT_NAME": event,
                "GITHUB_REPOSITORY": "jferments/mpfree", "GITHUB_SHA": "abc123", "GITHUB_WORKFLOW": "kernel"}

    def file(self, name, data=b"data"):
        p = self.src / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        return p

    def keep(self, kind, paths, env, **kw):
        kw.setdefault("min_free_gib", 0)
        return store.keep(kind, [str(p) for p in paths], env, **kw)

    def test_keeps_files_and_a_folder_with_a_record_and_a_link_for_the_pr(self):
        z = self.file("zImage", b"kernel")
        d = self.src / "stick-programs"
        (d / "sub").mkdir(parents=True)
        (d / "mpfree-app").write_bytes(b"app")
        (d / "sub" / "x").write_bytes(b"x")
        result = self.keep("kernel", [z, d], self.env(), now=NOW)
        run = self.store / "kernel" / "runs" / "100.1"
        self.assertEqual(result["folder"], str(run))
        self.assertEqual((run / "zImage").read_bytes(), b"kernel")
        self.assertEqual((run / "stick-programs" / "sub" / "x").read_bytes(), b"x")
        record = json.loads((run / "run.json").read_text())
        self.assertEqual(record["files"]["zImage"]["sha256"], hashlib.sha256(b"kernel").hexdigest())
        self.assertEqual(set(record["files"]), {"zImage", "stick-programs/mpfree-app", "stick-programs/sub/x"})
        self.assertEqual(record["run_url"], "https://github.com/jferments/mpfree/actions/runs/100")
        self.assertEqual(os.readlink(self.store / "kernel" / "pr-442"), "runs/100.1")
        self.assertEqual((self.store / "kernel" / "pr-442" / "zImage").read_bytes(), b"kernel")
        self.assertEqual(oct(run.stat().st_mode & 0o777), "0o755")
        self.assertFalse(list((self.store / "kernel" / "runs").glob(".incoming-*")))

    def test_two_jobs_of_one_run_add_to_it_and_never_replace(self):
        self.keep("creator", [self.file("a", b"1")], self.env(), now=NOW)
        self.keep("creator", [self.file("b", b"2")], self.env(), now=NOW + 60)
        run = self.store / "creator" / "runs" / "100.1"
        self.assertEqual(set(json.loads((run / "run.json").read_text())["files"]), {"a", "b"})
        with self.assertRaises(store.StoreError) as caught:
            self.keep("creator", [self.file("a", b"changed")], self.env(), now=NOW + 120)
        self.assertEqual(caught.exception.status, 5)
        self.assertEqual((run / "a").read_bytes(), b"1")

    def test_no_store_on_this_machine(self):
        env = {**self.env(), "MPFREE_STORE": str(self.root / "missing")}
        with self.assertRaises(store.StoreError) as caught:
            self.keep("kernel", [self.file("z")], env)
        self.assertEqual(caught.exception.status, 3)

    def test_not_enough_free_disk_keeps_nothing(self):
        with self.assertRaises(store.StoreError) as caught:
            self.keep("kernel", [self.file("z")], self.env(), min_free_gib=1e9)
        self.assertEqual(caught.exception.status, 4)
        self.assertFalse((self.store / "kernel").exists())

    def test_ref_names(self):
        self.assertEqual(store.ref_name({"GITHUB_REF": "refs/heads/main", "GITHUB_REF_NAME": "main"}), "main")
        self.assertEqual(store.ref_name({"GITHUB_REF": "refs/pull/7/merge", "GITHUB_REF_NAME": "7/merge"}), "pr-7")
        self.assertEqual(store.ref_name({"GITHUB_REF": "refs/tags/creator-v0.3.0",
                                         "GITHUB_REF_NAME": "creator-v0.3.0"}), "creator-v0.3.0")
        self.assertEqual(store.ref_name({"GITHUB_REF": "refs/heads/claude/x y",
                                         "GITHUB_REF_NAME": "claude/x y"}), "claude-x-y")
        self.assertEqual(store.ref_name({"GITHUB_REF": "refs/heads/runs", "GITHUB_REF_NAME": "runs"}), "ref-runs")
        self.assertEqual(store.ref_name({"GITHUB_REF_NAME": "../.."}), "unnamed")

    def test_old_runs_go_but_the_newest_on_main_stays(self):
        main = dict(ref="refs/heads/main", ref_name="main", event="push")
        self.keep("kernel", [self.file("z1")], self.env(run="1", **main), now=NOW - 40 * DAY)
        self.keep("kernel", [self.file("z2")], self.env(run="2"), now=NOW - 20 * DAY)
        third = self.keep("kernel", [self.file("z3")], self.env(run="3", ref="refs/pull/9/merge"), now=NOW - 1 * DAY)
        self.keep("kernel", [self.file("z4")], self.env(run="4", ref="refs/pull/9/merge"), now=NOW)
        runs = sorted(p.name for p in (self.store / "kernel" / "runs").iterdir())
        self.assertEqual(runs, ["1.1", "3.1", "4.1"])  # 1.1 is main's newest, however old
        self.assertEqual(third["removed_runs"], ["2.1"])
        self.assertFalse((self.store / "kernel" / "pr-442").is_symlink())  # pointed at run 2
        self.assertEqual(os.readlink(self.store / "kernel" / "pr-9"), "runs/4.1")
        self.assertEqual(os.readlink(self.store / "kernel" / "main"), "runs/1.1")

    def test_at_most_max_runs(self):
        for i in range(5):
            self.keep("coverage", [self.file(f"lcov{i}")], self.env(run=str(i)), now=NOW + i, max_runs=3)
        runs = sorted(p.name for p in (self.store / "coverage" / "runs").iterdir())
        self.assertEqual(runs, ["2.1", "3.1", "4.1"])

    def test_refuses_bad_input(self):
        for kind, paths, status in (("Kernel", [self.file("z")], 2),
                                    ("kernel", [self.src / "missing"], 2),
                                    ("kernel", [self.file("run.json")], 2),
                                    ("kernel", [self.file("a"), self.root / "a"], None)):
            if status is None:
                (self.root / "a").write_bytes(b"other")
                status = 2
            with self.assertRaises(store.StoreError) as caught:
                self.keep(kind, paths, self.env())
            self.assertEqual(caught.exception.status, status, (kind, paths))
        with self.assertRaises(store.StoreError):
            self.keep("kernel", [self.file("z")], {k: v for k, v in self.env().items() if k != "GITHUB_RUN_ID"})

    def test_main_writes_the_summary_and_returns_the_status(self):
        summary = self.root / "summary.md"
        env = {**self.env(), "GITHUB_STEP_SUMMARY": str(summary)}
        self.assertEqual(store.main(["keep", "kernel", str(self.file("z")), "--min-free-gib", "0"], env), 0)
        text = summary.read_text()
        self.assertIn("Kept on the CI VM", text)
        self.assertIn(str(self.store / "kernel" / "runs" / "100.1"), text)
        env["MPFREE_STORE"] = str(self.root / "missing")
        self.assertEqual(store.main(["keep", "kernel", str(self.file("y"))], env), 3)


if __name__ == "__main__":
    unittest.main()
