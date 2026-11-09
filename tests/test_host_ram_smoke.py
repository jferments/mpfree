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
PASS_OUTPUT = """MPFREE_VOLATILE_STAGE_SHA256_OK
MPFREE_ARMV7_SMOKE_V1
os=linux
arch=arm
frames=4096
beats=1
nonzero_samples=146
status=PASS
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

    def test_remote_script_is_fixed_tmpfs_only_and_checks_checksum(self):
        digest = "a" * 64
        program = smoke.remote_run_script(digest)
        for marker in (
            '/run', 'tmpfs', "mktemp -d /run/mpfree-smoke.",
            'trap cleanup 0', 'timeout 5 nice -n 10', "sha256sum", digest,
            'chmod 0500', 'cat > "$stage/probe"', 'ulimit -c 0',
            'ulimit -f 0', "systemctl is-active acvs.service",
        ):
            self.assertIn(marker, program)
        for forbidden in ("systemctl stop", "systemctl restart", "mount -o", "dd if=", "/dev/snd",
                          "/dev/dri", "/dev/input", "reboot", "fw_setenv"):
            self.assertNotIn(forbidden, program)
        for bad in ("", "a" * 63, "a" * 65, "'; reboot", "not-a-digest"):
            with self.assertRaises(ValueError):
                smoke.remote_run_script(bad)

    def test_must_see_hash_arch_frames_and_pass(self):
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
