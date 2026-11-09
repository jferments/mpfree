"""Host-only checks for a strictly opt-in, volatile MPC Rust smoke runner."""
import importlib.util
from pathlib import Path
import unittest

SOURCE = (Path(__file__).resolve().parents[1] / "tools/mpc-one/host_ram_smoke.py")
spec = importlib.util.spec_from_file_location("mpfree_ram_smoke", SOURCE)
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)

LIVE_REPORT = """MPFREE ARMV7 USERSPACE ABI SURVEY - READ ONLY
=== Exact board model and compatibility (no serial) ===
board_model=Akai Professional MPC One
=== Kernel and CPU ===
Linux mpc-one 6.6.95-az01-2025-06-27-rt55 #1 PREEMPT_RT
=== Operating system release ===
ID=az01
VERSION="5.0.8 (scarthgap)"
=== GNU libc embedded release banner (read-only strings) ===
GNU libc release banner: GNU C Library (GNU libc) stable release version 2.39.
"""
ENV_REPORT = """MPFREE RAM SMOKE ENVIRONMENT READ ONLY
No software is installed, uploaded, started or modified by this survey.
=== /run mount type (exact mount point) ===
run_mount_type=tmpfs
=== Required external utilities ===
tool.awk=present
tool.mktemp=present
tool.cat=present
tool.sha256sum=present
tool.timeout=MISSING
tool.nice=MISSING
tool.chmod=present
tool.rm=present
tool.rmdir=present
tool.systemctl=present
=== Shell CPU-time limit and inherited scheduling (read-only) ===
shell_cpu_limit=present
shell_scheduler_policy=0
=== Factory application (read-only) ===
factory_service_active=active
factory_main_pid=present
=== Interpreter and target kernel (read-only) ===
kernel_machine=armv7l
armhf_interpreter=present
END MPFREE RAM SMOKE ENVIRONMENT
"""

PASS_OUTPUT = """STAGE=VERIFY_SHA256
MPFREE_VOLATILE_STAGE_SHA256_OK
STAGE=EXECUTE_RUST
MPFREE_ARMV7_SMOKE_V1
os=linux
arch=arm
sample_rate_hz=44100
frames=4096
beats=1
nonzero_samples=146
status=PASS
STAGE=RUST_EXECUTED
"""

SESSION_OUTPUT = """STAGE=VERIFY_SHA256
MPFREE_VOLATILE_STAGE_SHA256_OK
STAGE=EXECUTE_RUST
MPFREE_SESSION_V1
os=linux
arch=arm
backend=null
pacing=monotonic
sample_rate_hz=44100
period_frames=128
frames=88200
callback_count=690
deadline_misses=1
late_starts=689
max_callback_us=500
p99_callback_us=300
max_start_lateness_us=700
total_render_us=30000
wall_elapsed_us=2000123
groove_events=12
sampler_notes=4
checksum_fnv1a64=3dc4d6dabbb17fbd
status=PASS
STAGE=RUST_EXECUTED
"""

HEADLESS_OUTPUT = """STAGE=VERIFY_SHA256
MPFREE_VOLATILE_STAGE_SHA256_OK
STAGE=EXECUTE_RUST
MPFREE_HEADLESS_V1
os=linux
arch=arm
backend=null
sample_rate_hz=44100
frames=88200
groove_events=12
sampler_notes=4
groove_voice_steals=0
sampler_voice_steals=0
prepared_bytes=94628
nonzero_samples=102304
peak_abs=0.765504
checksum_fnv1a64=3dc4d6dabbb17fbd
elapsed_us=10000
peak_rss_kib=4320
rss_kib=3992
status=PASS
STAGE=RUST_EXECUTED
"""



class GuardTests(unittest.TestCase):
    def test_exact_live_model_firmware_and_glibc_required(self):
        self.assertIsNone(smoke.verify_live_board(LIVE_REPORT))
        for before, after in (
            ("Akai Professional MPC One", "Akai Professional MPC One+"),
            ('VERSION="5.0.8 (scarthgap)"', 'VERSION="5.1.0"'),
            ("6.6.95-az01-", "6.8.0-different-"),
            ("release version 2.39.", "release version 2.42."),
            ("ID=az01", "ID=debian"),
        ):
            with self.subTest(before=before):
                with self.assertRaises(RuntimeError):
                    smoke.verify_live_board(LIVE_REPORT.replace(before, after))

    def test_strict_ssh_security_flags(self):
        argv = smoke.ssh_command("10.42.0.141", 2222, "root", "/tmp/example", "sh -s")
        for requirement in (
            "StrictHostKeyChecking=yes", "IdentitiesOnly=yes", "BatchMode=yes",
            "ForwardAgent=no", "ClearAllForwardings=yes", "ConnectTimeout=8",
        ):
            self.assertIn(requirement, argv)

    def test_read_only_prerequisites_report_all_ten_utilities(self):
        self.assertIn("PASS", smoke.check_remote_environment(ENV_REPORT))
        self.assertIn("timeout/nice not required", smoke.check_remote_environment(ENV_REPORT))
        for missing in ("mktemp", "cat", "sha256sum", "systemctl"):
            with self.subTest(tool=missing):
                modified = ENV_REPORT.replace(
                    f"tool.{missing}=present", f"tool.{missing}=MISSING"
                )
                with self.assertRaisesRegex(RuntimeError, missing):
                    smoke.check_remote_environment(modified)

    def test_invalid_tmpfs_or_stock_service_never_allows_upload(self):
        for a, b in (
            ("run_mount_type=tmpfs", "run_mount_type=overlay"),
            ("factory_service_active=active", "factory_service_active=inactive"),
            ("factory_main_pid=present", "factory_main_pid=unavailable"),
            ("armhf_interpreter=present", "armhf_interpreter=MISSING"),
            ("shell_cpu_limit=present", "shell_cpu_limit=unsupported"),
            ("shell_scheduler_policy=0", "shell_scheduler_policy=2"),
        ):
            with self.subTest(attribute=a):
                with self.assertRaises(RuntimeError):
                    smoke.check_remote_environment(ENV_REPORT.replace(a, b))

    def test_incomplete_and_duplicate_prerequisite_reports_rejected(self):
        with self.assertRaises(RuntimeError):
            smoke.check_remote_environment(ENV_REPORT.replace(
                "END MPFREE RAM SMOKE ENVIRONMENT", ""
            ))
        with self.assertRaisesRegex(RuntimeError, "duplicate"):
            smoke.check_remote_environment(
                ENV_REPORT.replace(
                    "tool.sha256sum=present",
                    "tool.sha256sum=present\n"
                    "tool.sha256sum=present",
                )
            )

    def test_exit_127_reports_stdout_phase_even_when_stderr_empty(self):
        try:
            smoke.parse_results("STAGE=CHECK_UTILITIES\n", "", 127)
        except RuntimeError as exc:
            self.assertIn("127", str(exc))
            self.assertIn("STAGE=CHECK_UTILITIES", str(exc))
        else:
            self.fail("exit 127 must fail closed and reveal the last phase")

    def test_optional_timeout_and_nice_may_be_missing(self):
        self.assertIn("PASS", smoke.check_remote_environment(ENV_REPORT))
        for name in ("timeout", "nice"):
            report = ENV_REPORT.replace(
                f"tool.{name}=MISSING", f"tool.{name}=present"
            )
            self.assertIn("PASS", smoke.check_remote_environment(report))

    def test_headless_profile_is_allowlisted_separately(self):
        self.assertEqual(
            smoke.get_profile("headless")["binary"], "mpfree-headless"
        )
        self.assertEqual(
            smoke.get_profile("device-smoke")["binary"], "mpfree-device-smoke"
        )
        self.assertNotEqual(
            smoke.get_profile("headless")["watchdog"],
            smoke.get_profile("device-smoke")["watchdog"]
        )
        with self.assertRaises(ValueError):
            smoke.get_profile("unapproved")

    def test_headless_results_require_exact_null_backend_and_pcm_metrics(self):
        stderr = "MPFREE_HEADLESS_RUST_WATCHDOG_5S_ARMED\n"
        d = smoke.parse_results(HEADLESS_OUTPUT, stderr, 0, "headless")
        self.assertEqual(d["frames"], "88200")
        self.assertEqual(d["backend"], "null")
        self.assertEqual(d["rss_kib"], "3992")
        for before, after in (
            ("backend=null", "backend=alsa"),
            ("arch=arm", "arch=x86_64"),
            ("frames=88200", "frames=88199"),
            ("sampler_notes=4", "sampler_notes=5"),
            ("groove_voice_steals=0", "groove_voice_steals=2"),
            ("prepared_bytes=94628", "prepared_bytes=0"),
            ("peak_abs=0.765504", "peak_abs=inf"),
            ("elapsed_us=10000", "elapsed_us=0"),
            ("rss_kib=3992", "rss_kib=oops"),
            ("status=PASS", "status=FAIL"),
        ):
            with self.subTest(field=before):
                with self.assertRaises(RuntimeError):
                    smoke.parse_results(
                        HEADLESS_OUTPUT.replace(before, after),
                        stderr, 0, "headless"
                    )
        with self.assertRaisesRegex(RuntimeError, "watchdog not proven"):
            smoke.parse_results(HEADLESS_OUTPUT, "", 0, "headless")

    def test_headless_duplicate_or_missing_phase_markers_rejected(self):
        stderr = "MPFREE_HEADLESS_RUST_WATCHDOG_5S_ARMED\n"
        with self.assertRaises(RuntimeError):
            smoke.parse_results(
                HEADLESS_OUTPUT.replace(
                    "STAGE=RUST_EXECUTED", "STAGE=EXECUTE_RUST"
                ), stderr, 0, "headless"
            )
        with self.assertRaisesRegex(RuntimeError, "duplicate"):
            smoke.parse_results(
                HEADLESS_OUTPUT.replace(
                    "frames=88200", "frames=88200\nframes=88200"
                ), stderr, 0, "headless"
            )
        # No memory reading may be available on a future kernel,
        # but output fields must still be present and explicit.
        valid = HEADLESS_OUTPUT.replace("peak_rss_kib=4320", "peak_rss_kib=unavailable")
        self.assertEqual(
            smoke.parse_results(valid, stderr, 0, "headless")["peak_rss_kib"],
            "unavailable"
        )

    def test_M7_separate_session_profile_and_paced_null_only_result(self):
        p = smoke.get_profile("session")
        self.assertEqual(p["binary"], "mpfree-session")
        self.assertNotEqual(p["watchdog"], smoke.get_profile("headless")["watchdog"])
        values = smoke.parse_results(
            SESSION_OUTPUT, "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED\n", 0,
            "session"
        )
        self.assertEqual(values["deadline_misses"], "1")  # valid measured data
        self.assertEqual(values["p99_callback_us"], "300")

    def test_M7_rejects_wiped_null_gate_or_corrupt_metrics(self):
        err = "MPFREE_SESSION_RUST_WATCHDOG_7S_ARMED\n"
        for a, b in (
            ("backend=null", "backend=alsa"),
            ("pacing=monotonic", "pacing=offline"),
            ("callback_count=690", "callback_count=689"),
            ("checksum_fnv1a64=3dc4d6dabbb17fbd",
             "checksum_fnv1a64=1234567890123456"),
            ("late_starts=689", "late_starts=691"),
            ("deadline_misses=1", "deadline_misses=691"),
            ("p99_callback_us=300", "p99_callback_us=501"),
            ("max_callback_us=500", "max_callback_us=nan"),
            ("wall_elapsed_us=2000123", "wall_elapsed_us=5"),
            ("wall_elapsed_us=2000123", "wall_elapsed_us=9000000"),
            ("total_render_us=30000", "total_render_us=0"),
        ):
            with self.subTest(field=a), self.assertRaises(RuntimeError):
                smoke.parse_results(SESSION_OUTPUT.replace(a, b), err, 0, "session")
        with self.assertRaisesRegex(RuntimeError, "watchdog"):
            smoke.parse_results(SESSION_OUTPUT, "", 0, "session")

    def test_remote_script_is_fixed_tmpfs_only_and_checks_checksum(self):
        digest = "a" * 64
        program = smoke.remote_run_script(digest)
        for marker in (
            '/run', 'tmpfs', "mktemp -d /run/mpfree-smoke.",
            'trap cleanup 0', 'ulimit -t 3', "sha256sum", digest,
            'chmod 0500', 'cat > "$stage/probe"', 'ulimit -c 0',
            'ulimit -f 0', "systemctl is-active acvs.service",
        ):
            self.assertIn(marker, program)
        for forbidden in ("systemctl stop", "systemctl restart", "timeout 5 nice",
                          "mount -o", "dd if=", "/dev/snd",
                          "/dev/dri", "/dev/input", "reboot", "fw_setenv"):
            self.assertNotIn(forbidden, program)
        for bad in ("", "a" * 63, "a" * 65, "'; reboot", "not-a-digest"):
            with self.assertRaises(ValueError):
                smoke.remote_run_script(bad)

    def test_must_see_hash_arch_frames_and_pass(self):
        smoke.parse_results(PASS_OUTPUT, "WATCHDOG=RUST_5S_ARMED\n", 0)
        with self.assertRaisesRegex(RuntimeError, "watchdog not proven"):
            smoke.parse_results(PASS_OUTPUT, "", 0)
        with self.assertRaises(RuntimeError):
            smoke.parse_results(PASS_OUTPUT, "", 1)
        for marker in ("MPFREE_VOLATILE_STAGE_SHA256_OK", "status=PASS",
                       "frames=4096", "arch=arm"):
            with self.subTest(marker=marker):
                with self.assertRaises(RuntimeError):
                    smoke.parse_results(PASS_OUTPUT.replace(marker, "MISSING"), "", 0)


if __name__ == "__main__":
    unittest.main()
