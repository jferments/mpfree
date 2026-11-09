"""M12 mock-only checks; NEVER connects to the MPC or opens input devices."""
from pathlib import Path
import importlib.util
import subprocess
import sys
import tempfile
import unittest

BASE = Path(__file__).resolve().parents[1] / "tools" / "mpc-one"
sys.path.insert(0, str(BASE))
spec = importlib.util.spec_from_file_location("mpfree_m12_capture_test", BASE / "host_touch_capture.py")
cap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cap)


def counters(prefix, bump=None):
    lines = []
    for p in range(1, 7):
        w = 100 + p + (1 if bump == p else 0)
        lines.append(f"{prefix}emmc_p{p}_writes={w} {prefix}emmc_p{p}_sectors_written={w * 8}")
    return "\n".join(lines)


BASELINE = {(p, k): (100 + p) * (8 if k == "sectors_written" else 1)
            for p in range(1, 7) for k in ("writes", "sectors_written")}


def good_stdout(events=3, bump=None, status="PASS", touches=1, pre_bump=None):
    body = [
        "STAGE=CHECK_RUN_TMPFS", counters("pre_", pre_bump),
        "MPFREE_TOUCH_CAPTURE_STAGE_SHA256_OK",
        cap.HEADER, "device=event0", cap.ARMED,
    ]
    body += [f"E 1 {i} 3 57 {i}" for i in range(min(touches, events))]
    body += [f"E 1 {i} 3 53 {i}" for i in range(events - min(touches, events))]
    body += [
        f"{cap.END} events={events} drops=0 reads=1 elapsed_ms=9000 status={status}",
        "STAGE=CAPTURE_EXITED status=0", "STAGE_DIR_REMOVED",
        "STAGE=EMMC_COUNTERS_AFTER", counters("post_", bump), "FACTORY_ACTIVE_AFTER",
    ]
    return "\n".join(body) + "\n"


def fake_remote(events_by_call):
    """A stand-in for ssh: prints one good step run per call, in order."""
    script = (
        "import sys,time\n"
        "sys.stdin.buffer.read()\n"
        "n = int(open(sys.argv[1]).read()) if __import__('os').path.exists(sys.argv[1]) else 0\n"
        "open(sys.argv[1], 'w').write(str(n + 1))\n"
        f"touches = {events_by_call!r}[n]\n"
        "out = sys.argv[2].replace('@@T@@', str(touches))\n"
        "head, rest = out.split('@@SPLIT@@')\n"
        "sys.stdout.write(head); sys.stdout.flush(); time.sleep(0.3)\n"
        "lines = rest.replace('@@EVENTS@@', ''.join(f'E 1 {i} 3 57 {i}\\n' for i in range(touches)))\n"
        "sys.stdout.write(lines.replace('events=@@N@@', f'events={touches}'))\n"
        f"print('{cap.WATCHDOG}', file=sys.stderr)\n"
    )
    template = "\n".join([
        "STAGE=CHECK_RUN_TMPFS", counters("pre_"), "MPFREE_TOUCH_CAPTURE_STAGE_SHA256_OK",
        cap.HEADER, "device=event0", cap.ARMED, "@@SPLIT@@@@EVENTS@@"
        f"{cap.END} events=@@N@@ drops=0 reads=1 elapsed_ms=9000 status=PASS",
        "STAGE=CAPTURE_EXITED status=0", "STAGE_DIR_REMOVED",
        "STAGE=EMMC_COUNTERS_AFTER", counters("post_"), "FACTORY_ACTIVE_AFTER", "",
    ])
    return script, template


class CaptureChecks(unittest.TestCase):
    def check(self, stdout, stderr=cap.WATCHDOG + "\n", code=0):
        return cap.check_execution(stdout, stderr, code, BASELINE)

    def test_good_run_passes(self):
        fields, capture = self.check(good_stdout())
        self.assertEqual(fields["events"], "3")
        self.assertEqual(capture[0], cap.HEADER)
        self.assertTrue(capture[-1].startswith(cap.END))
        self.assertEqual(cap.count_touches(capture), 1)

    def test_emmc_write_during_or_before_step_fails_closed(self):
        with self.assertRaisesRegex(RuntimeError, "after this step differ"):
            self.check(good_stdout(bump=6))
        # A write between the preflight and this step also fails closed.
        with self.assertRaisesRegex(RuntimeError, "before this step differ"):
            self.check(good_stdout(pre_bump=1, bump=1))

    def test_zero_events_is_not_a_check_failure(self):
        fields, capture = self.check(good_stdout(events=0, touches=0))
        self.assertEqual(fields["events"], "0")
        self.assertEqual(cap.count_touches(capture), 0)

    def test_missing_watchdog_or_marker_or_bad_status_fails(self):
        with self.assertRaises(RuntimeError):
            self.check(good_stdout(), "")
        with self.assertRaises(RuntimeError):
            self.check(good_stdout().replace("STAGE_DIR_REMOVED\n", ""))
        with self.assertRaises(RuntimeError):
            self.check(good_stdout(status="EVENT_CAP_REACHED"))
        with self.assertRaises(RuntimeError):
            self.check(good_stdout(), code=1)

    def test_footer_count_must_match_streamed_events(self):
        bad = good_stdout().replace("events=3 ", "events=4 ")
        with self.assertRaisesRegex(RuntimeError, "does not match"):
            self.check(bad)

    def test_count_touches_ignores_lifts_and_other_axes(self):
        capture = ["E 1 0 3 57 12", "E 1 0 3 53 57", "E 1 1 3 57 -1", "E 1 2 3 57 13",
                   "E 1 3 1 57 1", "E 1 4 3 47 57"]
        self.assertEqual(cap.count_touches(capture), 2)

    def test_preflight_markers_counters_and_stale_dirs(self):
        pre = "\n".join([
            "MPFREE_TOUCH_CAPTURE_PREFLIGHT_V1", counters(""), "stale_stage_dirs=0",
            "touch_driver=ili2117", "touch_event=event0",
            "factory_service_active=active", "device_node_opened=no",
            "END MPFREE_TOUCH_CAPTURE_PREFLIGHT",
        ])
        self.assertEqual(cap.parse_preflight(pre)[(6, "writes")], 106)
        self.assertEqual(cap.stale_stage_dirs(pre), 0)
        self.assertEqual(cap.stale_stage_dirs(pre.replace("dirs=0", "dirs=2")), 2)
        with self.assertRaises(RuntimeError):
            cap.stale_stage_dirs(pre.replace("stale_stage_dirs=0", ""))
        with self.assertRaises(RuntimeError):
            cap.parse_preflight(pre.replace("device_node_opened=no", ""))
        with self.assertRaises(RuntimeError):
            cap.parse_preflight(pre.replace("emmc_p3_writes", "emmc_p9_writes"))

    def test_remote_script_is_fixed_and_rejects_bad_hash(self):
        script = cap.remote_script("a" * 64)
        self.assertIn('"$stage/probe" --capture', script)
        self.assertIn("ulimit -f 0", script)
        self.assertIn("/run/mpfree-touch-capture.", script)
        # SIGPIPE ignored before anything else, so a dropped link still runs
        # the EXIT trap that deletes the /run copy.
        self.assertTrue(script.startswith("set -eu\n# If the SSH link"))
        self.assertLess(script.index("trap '' PIPE"), script.index("mktemp"))
        for banned in ("EVIOCGRAB", "event1", "dd ", "mount ", "systemctl stop",
                       "systemctl restart", "/dev/mmcblk", "fw_setenv"):
            self.assertNotIn(banned, script)
        with self.assertRaises(ValueError):
            cap.remote_script("not-a-hash")
        result = subprocess.run(["sh", "-n"], input=script.encode(), check=False)
        self.assertEqual(result.returncode, 0)

    def test_step_names_match_the_analyzer(self):
        src = (BASE.parents[1] / "rust/mpfree-touch/src/calibrate.rs").read_text()
        block = src[src.index("pub const STEP_NAMES"):].split("];", 1)[0]
        rust_names = [l.strip().strip('",') for l in block.splitlines()[1:] if l.strip()]
        self.assertEqual(rust_names, [s[0] for s in cap.STEPS])
        rust_cap = (BASE.parents[1] / "rust/mpfree-touch/src/bin/capture.rs").read_text()
        self.assertIn(f"const CAPTURE_SECONDS: u64 = {cap.STEP_SECONDS};", rust_cap)
        self.assertIn(f'const HEADER: &str = "{cap.HEADER}";', rust_cap)
        self.assertIn(f'const WATCHDOG_MARKER: &str = "{cap.WATCHDOG}";', rust_cap)
        self.assertEqual([s for s in cap.STEPS if s[2]], [cap.STEPS[-1]])

    def test_run_step_counts_down_then_go_then_stop(self):
        fake = (
            "import sys,time\n"
            "sys.stdin.buffer.read()\n"
            f"print('{cap.HEADER}'); print('{cap.ARMED}'); sys.stdout.flush()\n"
            "time.sleep(0.5)\n"
            "print('E 1 2 3 57 7')\n"
            f"print('{cap.END} events=1 drops=0 reads=1 elapsed_ms=500 status=PASS')\n"
            f"print('{cap.WATCHDOG}', file=sys.stderr)\n"
        )
        seen = []
        code, out, err = cap.run_step(
            [sys.executable, "-c", fake], b"binary", 10, "do it", seen.append, countdown=0)
        self.assertEqual(code, 0)
        self.assertIn("E 1 2 3 57 7", out)
        self.assertIn(cap.WATCHDOG, err)
        self.assertEqual(seen, [f"GO: do it  ({cap.STEP_SECONDS} s)", "STOP. Lift your fingers."])

    def test_run_step_deadline_kills_a_hung_remote(self):
        hung = "import sys,time; sys.stdin.buffer.read(); time.sleep(30)"
        seen = []
        code, out, err = cap.run_step(
            [sys.executable, "-c", hung], b"", 1, "x", seen.append, countdown=0)
        self.assertNotEqual(code, 0)
        self.assertIn("step exceeded 1 s; ssh closed", seen)

    def test_run_step_kills_ssh_on_ctrl_c(self):
        hung = ("import sys,time; sys.stdin.buffer.read(); "
                f"print('{cap.HEADER}'); print('{cap.ARMED}'); sys.stdout.flush(); time.sleep(30)")
        procs = []
        real_popen = subprocess.Popen

        def spy(*a, **k):
            procs.append(real_popen(*a, **k))
            return procs[-1]

        def interrupt(text):
            if text.startswith("GO"):
                raise KeyboardInterrupt
        cap.subprocess.Popen = spy
        try:
            with self.assertRaises(KeyboardInterrupt):
                cap.run_step([sys.executable, "-c", hung], b"", 20, "x", interrupt, countdown=0)
        finally:
            cap.subprocess.Popen = real_popen
        self.assertIsNotNone(procs[0].poll(), "ssh stand-in still running after Ctrl-C")
        procs[0].stdout.close()
        procs[0].stderr.close()

    def run_steps(self, touches_per_call, answers):
        script, template = fake_remote(touches_per_call)
        seen, asked = [], []
        replies = iter(answers)

        def ask(prompt):
            asked.append(prompt)
            return next(replies)
        with tempfile.TemporaryDirectory() as d:
            counter = Path(d) / "calls"
            log = Path(d) / "r.events.log"
            cmd = [sys.executable, "-c", script, str(counter), template]
            kept = cap.run_steps(cmd, b"bin", BASELINE, log, seen.append, ask,
                                 deadline_s=10, countdown=0)
            return kept, log.read_text(), seen, asked

    def test_run_steps_waits_for_enter_and_writes_labelled_log(self):
        steps = len(cap.STEPS)
        fingers = [s[1] for s in cap.STEPS]
        kept, log, seen, asked = self.run_steps(fingers, [""] * steps)
        self.assertEqual(kept, steps)
        self.assertEqual(len(asked), steps)
        self.assertTrue(asked[-1].endswith("(or type s then Enter to skip): "))
        lines = log.splitlines()
        self.assertEqual(lines[0], cap.LOG_HEADER)
        self.assertEqual(lines[-1], f"{cap.LOG_END} steps={steps}")
        self.assertEqual([l[5:] for l in lines if l.startswith("STEP ")],
                         [s[0] for s in cap.STEPS])
        # Only the device run goes in the log, not the STAGE/eMMC lines.
        self.assertFalse(any(l.startswith(("STAGE", "pre_", "post_")) for l in lines))
        self.assertIn(f"step five_fingers: events=5 touches=5 drops=0; eMMC p1-p6 unchanged, "
                      "/run copy removed, factory app active", seen)

    def test_run_steps_offers_redo_on_wrong_touch_count_and_skip_for_optional(self):
        # centre: 2 touches (redo), then 1. Everything else right; skip ten.
        fingers = [2] + [s[1] for s in cap.STEPS[:-1]]
        answers = ["", "", ""] + [""] * (len(cap.STEPS) - 2) + ["s"]
        kept, log, seen, asked = self.run_steps(fingers, answers)
        self.assertEqual(kept, len(cap.STEPS) - 1)
        self.assertIn("Expected 1 touch(es), saw 2.", seen)
        self.assertTrue(asked[1].startswith("Press Enter to redo this step"))
        self.assertIn("step ten_fingers: skipped", seen)
        self.assertEqual(log.count("STEP centre"), 1)
        self.assertNotIn("STEP ten_fingers", log)

    def test_run_steps_can_keep_a_mismatched_step(self):
        fingers = [s[1] for s in cap.STEPS[:-1]] + [7]
        answers = [""] * len(cap.STEPS) + ["k"]
        kept, log, seen, asked = self.run_steps(fingers, answers)
        self.assertEqual(kept, len(cap.STEPS))
        self.assertIn("Expected 10 touch(es), saw 7.", seen)


if __name__ == "__main__":
    unittest.main()
