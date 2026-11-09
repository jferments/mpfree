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


def good_stdout(events=3, bump=None, status="PASS"):
    body = [
        "STAGE=CHECK_RUN_TMPFS", counters("pre_"),
        "MPFREE_TOUCH_CAPTURE_STAGE_SHA256_OK",
        cap.HEADER, "device=event0", cap.ARMED,
    ]
    body += [f"E 1 {i} 3 53 {i}" for i in range(events)]
    body += [
        f"{cap.END} events={events} drops=0 reads=1 elapsed_ms=60000 status={status}",
        "STAGE=CAPTURE_EXITED status=0", "STAGE_DIR_REMOVED",
        "STAGE=EMMC_COUNTERS_AFTER", counters("post_", bump), "FACTORY_ACTIVE_AFTER",
    ]
    return "\n".join(body) + "\n"


class CaptureChecks(unittest.TestCase):
    def test_good_run_passes(self):
        fields = cap.check_execution(good_stdout(), cap.WATCHDOG + "\n", 0)
        self.assertEqual(fields["events"], "3")

    def test_emmc_write_during_capture_fails_closed(self):
        with self.assertRaisesRegex(RuntimeError, "eMMC boot-chain counters changed"):
            cap.check_execution(good_stdout(bump=6), cap.WATCHDOG + "\n", 0)

    def test_zero_events_fails_closed_with_grab_hint(self):
        with self.assertRaisesRegex(RuntimeError, "exclusive"):
            cap.check_execution(good_stdout(events=0), cap.WATCHDOG + "\n", 0)

    def test_missing_watchdog_or_marker_or_bad_status_fails(self):
        with self.assertRaises(RuntimeError):
            cap.check_execution(good_stdout(), "", 0)
        with self.assertRaises(RuntimeError):
            cap.check_execution(good_stdout().replace("STAGE_DIR_REMOVED\n", ""), cap.WATCHDOG, 0)
        with self.assertRaises(RuntimeError):
            cap.check_execution(good_stdout(status="EVENT_CAP_REACHED"), cap.WATCHDOG, 0)
        with self.assertRaises(RuntimeError):
            cap.check_execution(good_stdout(), cap.WATCHDOG, 1)

    def test_footer_count_must_match_streamed_events(self):
        bad = good_stdout().replace("events=3 ", "events=4 ")
        with self.assertRaisesRegex(RuntimeError, "does not match"):
            cap.check_execution(bad, cap.WATCHDOG, 0)

    def test_preflight_markers_and_counters(self):
        pre = "\n".join([
            "MPFREE_TOUCH_CAPTURE_PREFLIGHT_V1", counters(""),
            "touch_driver=ili2117", "touch_event=event0",
            "factory_service_active=active", "device_node_opened=no",
            "END MPFREE_TOUCH_CAPTURE_PREFLIGHT",
        ])
        self.assertEqual(cap.parse_preflight(pre)[(6, "writes")], 106)
        with self.assertRaises(RuntimeError):
            cap.parse_preflight(pre.replace("device_node_opened=no", ""))
        with self.assertRaises(RuntimeError):
            cap.parse_preflight(pre.replace("emmc_p3_writes", "emmc_p9_writes"))

    def test_remote_script_is_fixed_and_rejects_bad_hash(self):
        script = cap.remote_script("a" * 64)
        self.assertIn('"$stage/probe" --capture', script)
        self.assertIn("ulimit -f 0", script)
        self.assertIn("/run/mpfree-touch-capture.", script)
        for banned in ("EVIOCGRAB", "event1", "dd ", "mount ", "systemctl stop",
                       "systemctl restart", "/dev/mmcblk", "fw_setenv"):
            self.assertNotIn(banned, script)
        with self.assertRaises(ValueError):
            cap.remote_script("not-a-hash")
        result = subprocess.run(["sh", "-n"], input=script.encode(), check=False)
        self.assertEqual(result.returncode, 0)

    def test_streaming_writes_log_and_prompts_after_armed(self):
        fake = (
            "import sys,time\n"
            "sys.stdin.buffer.read()\n"
            f"print('{cap.HEADER}'); print('{cap.ARMED}'); sys.stdout.flush()\n"
            "time.sleep(1.3)\n"
            "print('E 1 2 3 53 7')\n"
            f"print('{cap.END} events=1 drops=0 reads=1 elapsed_ms=300 status=PASS')\n"
            f"print('{cap.WATCHDOG}', file=sys.stderr)\n"
        )
        seen = []
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "x.events.log"
            code, out, err = cap.stream_capture(
                [sys.executable, "-c", fake], b"binary", 10, log,
                ((0, "first"), (1, "second"), (30, "never")), seen.append)
            self.assertEqual(code, 0)
            self.assertIn("E 1 2 3 53 7", log.read_text())
            self.assertIn(cap.WATCHDOG, err)
        self.assertIn("Capture armed: 60 s window starts now.", seen)
        self.assertIn("[ 0s] first", seen)
        self.assertIn("[ 1s] second", seen)
        self.assertNotIn("[30s] never", seen)

    def test_stream_deadline_kills_a_hung_remote(self):
        hung = "import sys,time; sys.stdin.buffer.read(); time.sleep(30)"
        with tempfile.TemporaryDirectory() as d:
            code, out, err = cap.stream_capture(
                [sys.executable, "-c", hung], b"", 1, Path(d) / "h.log", (), lambda s: None)
        self.assertNotEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
