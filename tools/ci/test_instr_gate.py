#!/usr/bin/env python3
"""Tests for instr_gate.py that need no valgrind: reading callgrind dumps,
comparing with a baseline, and writing and reading the baseline file."""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import instr_gate  # noqa: E402

DUMP = """# callgrind format
version: 1
creator: callgrind-3.22.0
pid: 4242
cmd:  mpfree-bench --count --blocks 200
part: 3

desc: I1 cache:
desc: D1 cache:
desc: LL cache:

desc: Timerange: Basic block 1 - 2
desc: Trigger: Client Request: mpfree-bench case: Cloud reverb (Plate, Eco)

positions: line
events: Ir
summary: 47009200

fn=(1) main
0 47009200

totals: 47009200
"""

FINAL = DUMP.replace(
    "desc: Trigger: Client Request: mpfree-bench case: Cloud reverb (Plate, Eco)",
    "desc: Trigger: Program termination",
)


class ParseDump(unittest.TestCase):
    def test_a_case_dump_gives_its_name_and_count(self):
        self.assertEqual(instr_gate.parse_dump(DUMP), ("Cloud reverb (Plate, Eco)", 47009200))

    def test_the_final_dump_is_not_a_case(self):
        self.assertIsNone(instr_gate.parse_dump(FINAL))

    def test_another_programs_request_is_not_a_case(self):
        other = DUMP.replace("mpfree-bench case: ", "someone else: ")
        self.assertIsNone(instr_gate.parse_dump(other))


class Compare(unittest.TestCase):
    def test_a_rise_past_the_tolerance_fails_and_a_small_one_does_not(self):
        rows, rises = instr_gate.compare({"a": 1000, "b": 1000}, {"a": 1051, "b": 1049}, 5.0)
        self.assertEqual(rises, 1)
        self.assertTrue(rows[0].lstrip().startswith("RISE"), rows[0])
        self.assertTrue(rows[1].lstrip().startswith("ok"), rows[1])

    def test_a_drop_is_reported_but_never_fails(self):
        rows, rises = instr_gate.compare({"a": 1000}, {"a": 800}, 5.0)
        self.assertEqual(rises, 0)
        self.assertTrue(rows[0].lstrip().startswith("DROP"), rows[0])

    def test_new_and_gone_cases_are_named_and_do_not_fail(self):
        rows, rises = instr_gate.compare({"old": 10}, {"new": 10}, 5.0)
        self.assertEqual(rises, 0)
        self.assertEqual(sorted(r.split()[0] for r in rows), ["GONE", "NEW"])


class Baseline(unittest.TestCase):
    def test_what_is_blessed_reads_back(self):
        counts = {"Sampler, 20 pads": 60487, "Poly Init, 8 voices": 37133}
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "instructions.txt")
            instr_gate.write_baseline(path, counts, 200, "rustc 1.97.0 (test)")
            base, header = instr_gate.read_baseline(path)
        self.assertEqual(base, counts)
        self.assertTrue(any("rustc 1.97.0 (test)" in h for h in header))
        self.assertTrue(any("Blocks counted per case: 200" in h for h in header))


if __name__ == "__main__":
    unittest.main()
