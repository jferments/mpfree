#!/usr/bin/env python3
"""Offline safety/contract tests for host-side, optional MPFree link benchmark."""
import importlib.util
from pathlib import Path
import types
import unittest

SOURCE = Path(__file__).resolve().parents[1] / "tools/mpc-one/network-bench.py"
spec = importlib.util.spec_from_file_location("mpfree_benchmark", SOURCE)
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


class NetworkBenchContractTests(unittest.TestCase):
    def setUp(self):
        self.args = types.SimpleNamespace(
            ip="10.42.0.141", user="root", iface="eno1np0",
            key="/home/jesse/.ssh/mpc_one_autorun_ed25519", port=2222
        )

    def test_bounded_data(self):
        self.assertEqual(bench.TRANSFER, 4 * 1024 * 1024)
        self.assertLessEqual(bench.TRANSFER * 2, 8 * 1024 * 1024)
        self.assertEqual(bench.CHUNK, 65536)

    def test_units(self):
        self.assertAlmostEqual(bench.goodput_mbit(352800, 1), 2.8224)
        self.assertRaises(ValueError, bench.goodput_mbit, 1, 0)

    def test_auth_and_link_only(self):
        cmd = bench.ssh_prefix(self.args)
        joined = " ".join(cmd)
        self.assertIn("StrictHostKeyChecking=yes", joined)
        self.assertIn("Compression=no", joined)
        self.assertIn("BatchMode=yes", joined)
        self.assertIn("ForwardAgent=no", joined)
        self.assertIn("ClearAllForwardings=yes", joined)
        self.assertIn("root@10.42.0.141", joined)
        self.assertIn("2222", joined)
        self.assertNotIn("StrictHostKeyChecking=no", joined)
        self.assertNotIn("UserKnownHostsFile=/dev/null", joined)

    def test_remote_commands_bounded_and_ephemeral(self):
        # The benchmark's remote command strings are intentionally fixed.
        src = SOURCE.read_text()
        self.assertIn('"dd if=/dev/zero bs=16384 count=256 2>/dev/null"', src)
        self.assertIn('"cat > /dev/null"', src)
        self.assertNotIn("of=/dev/mmcblk", src)
        self.assertNotIn("systemctl stop", src)
        self.assertNotIn("pip install", src)
        self.assertNotIn("ethtool -s", src)


if __name__ == "__main__":
    unittest.main()
