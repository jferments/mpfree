#!/usr/bin/env python3
"""Tests for devtest_report.py on made-up reports in the tools' own shapes. No device."""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "devtest_report.py")


def status(underruns, periods, sampler, synth, late=0, cores="two"):
    """The app's status block, as the workflows' runs print it (indented by the host tool's stream)."""
    return [
        f"    status: 10 hits (0 from the screen), 0 repeats, {underruns} underruns in {periods} periods, at most 4 "
        f"sounds at once, peak -12.0 dBFS, 0 periods clipped; pad to sound -; note engine dropped 0 events, overflow 0; "
        f"sampler: render mean {sampler} us, max {sampler * 3} us (1.0 % and 3.0 % of a period), 0 steals, 0 events "
        f"dropped, at most 0 grains (0 skipped); time: fine",
        f"    status: synth: Poly (Init), render mean {synth} us, max {synth * 4} us (1.0 % and 4.0 % of a period), "
        f"3 notes, at most 3 voices (16 set), 0 notes dropped, 0 resets",
        f"    status: sound cores {cores}: drum voices 10 parts on core 2, {late} late; mixer 5 stages on core 2, 0 late",
    ]


def run(name, cores, body, rc=0):
    return ([f"  ----- workflow {name} (sound cores: {cores}) -----"] + body
            + [f"  ----- end of workflow {name} (sound cores: {cores}): exit code {rc} -----"])


def beat(underruns_in_take=0, saw_rec=True):
    body = ['    script: "A beat", 9 steps, 12.0 s',
            "    script: line 3 at 1.000 s: mark steps"]
    body += status(0, 344, 100, 50)
    body += ["    script: line 4 at 1.000 s: button STEP SEQ down",
             "    STEP SEQ: the pads are pad A1's steps, page 1",
             "    script: line 5 at 1.100 s: expect STEP SEQ: the pads are pad A1's steps",
             "    control: slow period at +1.500 s: 3000 us outside the engines, most in screen actions (x)",
             "    script: line 6 at 4.000 s: mark take"]
    body += status(0, 1378, 120, 50)
    body += ["    script: line 7 at 4.000 s: button REC down"]
    if saw_rec:
        body += ["    rec: ready, PLAY records Track 1"]
    body += ["    script: line 8 at 4.100 s: expect rec: ready, PLAY records Track 1",
             "    script: line 9 at 4.200 s: expect-not refused",
             "    script: line 10: no control header/tab \"Nope\" on Play; it has header/tab \"Play\"",
             "    script: done"]
    body += status(underruns_in_take, 4134, 140, 60)
    body += ["    screen: 600 frames, mean 8.0 ms, slowest 30.0 ms to draw and copy; 3 touches"]
    return body


def heavy(cores, underruns, late, slow=0):
    body = ['    script: "Heavy", 3 steps, 40.0 s', "    script: line 2 at 1.000 s: mark steady"]
    body += status(0, 344, 300, 200, 0, cores)
    body += ["    control: slow period at +2.0 s: x"] * slow
    body += ["    script: done"]
    body += status(underruns, 344 + 13780, 400, 250, late, cores)
    return body


def bench_report():
    case = lambda name, p99: {"name": name, "module": "synth", "mean_ns": p99 / 2, "p99_ns": p99,
                              "p999_ns": p99, "max_ns": p99, "mem_bytes": 0}
    neon = {"schema": "mpfree-bench/1", "cases": [case("Poly Supersaw, 8 voices", 300000),
                                                  case("Wind Clarinet, 8 voices", 200000)]}
    plain = {"schema": "mpfree-bench/1", "cases": [case("Poly Supersaw, 8 voices", 500000),
                                                   case("Wind Clarinet, 8 voices", 201000)]}
    lines = ["[4/6] The default build: the reference kernels, every engine, then one core vs two (only computes)",
             "    mpfree-bench --afford on mpc-one, build default",
             "    engine                                fixed us  us / voice  p99 us at each voice count"
             "                worst/mean    budget  voices that fit",
             "    " + f"{'Wind Clarinet':<36} {12.5:>9.1f} {'7.25':>11}  {'1:20 4:41 8:70':<40}  {1.4:>10.1f}  {580.0:>4.0f} us  77",
             "    " + f"{'Poly Supersaw':<36} {18.4:>9.1f} {'7.40':>11}  {'1:26 4:48 8:78':<40}  {1.2:>10.1f}  {580.0:>4.0f} us  16+ (75)",
             "    Grain budget (R7 step A6), on the MPC: a grain 0.80 us, a looping sample voice 3.10 us (about 3.9 grains; "
             "the engine's VOICE_COST is 4).",
             "    grain budget: 180 grains fit in the sampler's 725 us beside 32 sample voices (GRAIN_BUDGET is 256 now)",
             "    Keys Tines                                   120.0      10.0       12.0          6.88      130.0       12.0"
             "  over the synth budget",
             "  ----- default.json -----"]
    lines += ["  " + ln for ln in json.dumps(neon, indent=1).splitlines()]
    lines += ["  ----- end of default.json -----", "  ----- default-plain.json -----"]
    lines += ["  " + ln for ln in json.dumps(plain, indent=1).splitlines()]
    lines += ["  ----- end of default-plain.json -----"]
    return lines


def effects():
    """effects.mpw's busy part: a mark (which prints the status block, "0 underruns" in it), then expect-not."""
    body = ['    script: "Effects", 4 steps, 14.0 s', "    script: line 16 at 1.000 s: mark busy"]
    body += status(0, 344, 300, 200)
    body += ["    script: line 38 at 11.000 s: expect-not underrun",
             "    script: line 39 at 11.000 s: mark remove"]
    body += status(0, 3790, 310, 200)
    body += ["    script: done"]
    body += status(0, 4134, 310, 200)
    return body


class ReportTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-devtest-report-")

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def write(self, name, lines):
        path = os.path.join(self.tmp, name)
        with open(path, "w") as fh:
            fh.write("\n".join(lines) + "\n")
        return path

    def report(self, *paths):
        out = os.path.join(self.tmp, "out.md")
        res = subprocess.run([sys.executable, "-I", TOOL, *paths, "--out", out], text=True,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        return res

    def test_an_expect_not_is_not_met_by_the_status_block_a_mark_prints(self):
        devtest = ["MPFree device round workflows on the test build"] + run("effects", "default", effects())
        res = self.report(self.write("05-devtest.log", devtest))
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("| effects | 0 | yes | 1 of 1 | 0 | 0 | 0 | 0 |", res.stdout)
        self.assertNotIn("was seen", res.stdout)

    def test_workflows_their_parts_checks_and_the_sound_core_pick(self):
        devtest = (["MPFree device round workflows on the test build"] + run("beat", "default", beat())
                   + ["  your answer: a click in the take"]
                   + run("heavy-load", "shared", heavy("shared", 3, 0))
                   + run("heavy-load", "two", heavy("two", 0, 2))
                   + run("heavy-load", "three", heavy("three", 0, 0, slow=1)))
        res = self.report(self.write("05-devtest.log", devtest))
        self.assertEqual(res.returncode, 1, res.stdout)  # a step not done and underruns on shared
        out = res.stdout
        self.assertIn("| beat | 0 | yes | 3 of 3 | 1 | 0 | 1 | 0 |", out)
        self.assertIn("Your listening answer: a click in the take", out)
        self.assertIn('- line 10: no control header/tab "Nope" on Play', out)
        # The steps part: 344 to 1378 periods, sampler mean over the part (120*1378 - 100*344) / 1034.
        self.assertIn("| steps | 0 | 1034 | 1 | 0 | 127 | 50 |", out)
        self.assertIn("| take | 0 | 2756 | 0 | 0 | 150 | 65 |", out)
        self.assertIn("Worst period of the run: sampler 420 µs, synth 240 µs.", out)
        self.assertIn("Screen: 600 frames, mean 8.0 ms, slowest 30.0 ms.", out)
        self.assertIn("| shared | 3 | 0 | 0 |", out)
        self.assertIn("The numbers favour **three**", out)
        with open(os.path.join(self.tmp, "out.md")) as fh:
            self.assertEqual(fh.read(), out)

    def test_a_missed_expect_is_named_and_a_clean_run_passes(self):
        res = self.report(self.write("a.log", run("beat", "default", beat(saw_rec=False))))
        self.assertEqual(res.returncode, 1)
        self.assertIn("| beat | 0 | yes | 2 of 3 |", res.stdout)
        self.assertIn("- line 8: expect 'rec: ready, PLAY records Track 1' never showed", res.stdout)
        clean = [ln for ln in beat() if "no control" not in ln]
        res = self.report(self.write("b.log", run("beat", "default", clean)))
        self.assertEqual(res.returncode, 0, res.stdout)
        res = self.report(self.write("c.log", run("beat", "default", clean, rc=101)))
        self.assertEqual(res.returncode, 1)
        cut = [ln for ln in clean if "script: done" not in ln][:8]
        res = self.report(self.write("d.log", run("beat", "default", cut)))
        self.assertIn("| beat | 0 | NO |", res.stdout)

    def test_the_bench_report_gives_neon_voice_costs_and_the_grain_budget(self):
        res = self.report(self.write("03-bench.log", bench_report()))
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertIn("2 engines timed both ways; with the NEON kernels 1 are faster", out)
        self.assertIn("| Poly Supersaw, 8 voices | 300.0 | 500.0 | 0.60 |", out)
        self.assertIn("Verdict: **keep NEON: faster where it differs, nowhere slower**.", out)
        self.assertIn("| default | Wind Clarinet | 12.5 | 7.25 | 580 | 77 |", out)
        self.assertNotIn("| default | Poly Supersaw |", out)  # not one of the round's asks
        self.assertIn("set `VOICE_COST` to **4** (now 4)", out)
        self.assertIn("set `GRAIN_BUDGET` to **180** (now 256)", out)
        self.assertIn("- Keys Tines", out)

    def test_nothing_to_read(self):
        res = self.report(self.write("empty.log", ["hello"]))
        self.assertEqual(res.returncode, 2)
        self.assertIn("no workflow runs or benchmark reports", res.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
