"""Host-only M7 supervisor tests, without any hardware/device execution."""
import importlib.util
from pathlib import Path
import unittest

SRC = Path(__file__).resolve().parents[1] / "tools/mpc-one/host_null_session_supervisor.py"
spec = importlib.util.spec_from_file_location("mpfree_host_session", SRC)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

SESSION = """MPFREE_SESSION_V1
os=linux
arch=x86_64
backend=null
pacing=monotonic
sample_rate_hz=44100
period_frames=128
frames=88200
callback_count=690
deadline_misses=2
late_starts=550
max_callback_us=97
p99_callback_us=50
max_start_lateness_us=211
total_render_us=12000
wall_elapsed_us=2000030
groove_events=12
sampler_notes=4
checksum_fnv1a64=3dc4d6dabbb17fbd
status=PASS
"""


class SupervisorParserTests(unittest.TestCase):
    def test_host_supervisor_validates_real_engine_output(self):
        d = m.parse_session(
            SESSION, "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED\n", 0
        )
        self.assertEqual(d["callback_count"], "690")
        self.assertEqual(d["deadline_misses"], "2")

    def test_reject_wrong_engine_or_pacing_or_hardware_backend(self):
        for old, new in (
            ("backend=null", "backend=alsa"),
            ("pacing=monotonic", "pacing=offline"),
            ("sample_rate_hz=44100", "sample_rate_hz=48000"),
            ("period_frames=128", "period_frames=256"),
            ("groove_events=12", "groove_events=13"),
            ("checksum_fnv1a64=3dc4d6dabbb17fbd", "checksum_fnv1a64=deadbeef"),
            ("status=PASS", "status=FAIL"),
        ):
            with self.subTest(field=old), self.assertRaises(RuntimeError):
                m.parse_session(
                    SESSION.replace(old, new),
                    "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED\n", 0
                )

    def test_rejects_missing_watchdog_nonzero_exit_duplicate_and_bad_metrics(self):
        for out, err, code in (
            (SESSION, "", 0),
            (SESSION, "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED\n", 127),
            (SESSION + "frames=88200\n", "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED\n", 0),
            (SESSION.replace("deadline_misses=2", "deadline_misses=691"),
             "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED\n", 0),
            (SESSION.replace("p99_callback_us=50", "p99_callback_us=200"),
             "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED\n", 0),
            (SESSION.replace("late_starts=550", "late_starts=NaN"),
             "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED\n", 0),
        ):
            with self.subTest(code=code), self.assertRaises(RuntimeError):
                m.parse_session(out, err, code)

    def test_no_remote_transport_embedded_in_host_supervisor(self):
        source = SRC.read_text()
        for forbidden in (
            "ssh -T", "systemctl stop", "systemctl restart",
            "/dev/snd", "/dev/dri", "/dev/input",
            "10.42.0.141", "MPFREE_ACCEPT_HEADLESS_EXEC",
        ):
            self.assertNotIn(forbidden, source)


if __name__ == "__main__":
    unittest.main()
