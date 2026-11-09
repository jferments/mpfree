#!/usr/bin/env python3
"""Tests for host_emmc_backup.py against a fake MPC. No network, no device.

The real read-only-backup-preflight.sh runs under BusyBox (as on the MPC)
against a small fake eMMC with the same partition structure. Set
MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import hashlib
import os
import random
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
HELPER = os.path.join(TOOLS, "host_emmc_backup.py")
PREFLIGHT = os.path.join(TOOLS, "read-only-backup-preflight.sh")
FAKE_SSH = os.path.join(HERE, "fake_mpc_ssh.py")
APPLETS = ("sh", "cat", "sed", "awk", "dd", "sha256sum", "readlink", "uname", "head")

SECTOR = 512
DISK_SECTORS = 4096
HW_SECTORS = 16
LAYOUT = [  # same names and order as the MPC One, scaled down
    ("mmcblk0p1", "uboot-spl", 64, 128),
    ("mmcblk0p2", "env", 192, 16),
    ("mmcblk0p3", "uboot", 256, 768),
    ("mmcblk0p4", "splash", 1024, 256),
    ("mmcblk0p5", "recoverysplash", 1280, 256),
    ("mmcblk0p6", "rootfs", 1536, 1024),
    ("mmcblk0p7", "content", 2560, 512),
    ("mmcblk0p8", "data", 3072, 991),
]
EXPECTED_COMMANDS = [
    "sh -s hash",
    "dd if=/dev/mmcblk0 bs=1048576",
    "dd if=/dev/mmcblk0boot0 bs=1048576",
    "dd if=/dev/mmcblk0boot1 bs=1048576",
    "sh -s stat",
]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def write(path, text_or_bytes):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    mode = "wb" if isinstance(text_or_bytes, bytes) else "w"
    with open(path, mode) as fh:
        fh.write(text_or_bytes)


class FakeMpc:
    def __init__(self, base):
        self.base = base
        self.root = os.path.join(base, "mpc")
        self.disk = random.Random(19).randbytes(DISK_SECTORS * SECTOR)
        self.zeros = bytes(HW_SECTORS * SECTOR)
        dev = os.path.join(self.root, "dev")
        blk = os.path.join(self.root, "sys/class/block")
        write(os.path.join(dev, "mmcblk0"), self.disk)
        self._sysfs(blk, "mmcblk0", None, DISK_SECTORS, None)
        for i, (name, pname, start, sectors) in enumerate(LAYOUT, 1):
            write(os.path.join(dev, name), self.part(name))
            self._sysfs(blk, name, start, sectors, pname, partn=i)
        for name in ("mmcblk0boot0", "mmcblk0boot1"):
            write(os.path.join(dev, name), self.zeros)
            self._sysfs(blk, name, None, HW_SECTORS, None)
        card = os.path.join(self.root, "sys/devices/mmc_host/mmc0/mmc0:0001")
        write(os.path.join(card, "name"), "DG4008\n")
        os.makedirs(os.path.join(self.root, "sys/block/mmcblk0"))
        os.symlink("../../devices/mmc_host/mmc0/mmc0:0001",
                   os.path.join(self.root, "sys/block/mmcblk0/device"))
        write(os.path.join(self.root, "sys/kernel/debug/mmc0/mmc0:0001/ext_csd"),
              "00" * 179 + "48" + "00" * 332 + "\n")
        write(os.path.join(self.root, "proc/sys/kernel/random/boot_id"),
              "11111111-2222-4333-8444-555555555555\n")

    def _sysfs(self, blk, name, start, sectors, pname, partn=None):
        d = os.path.join(blk, name)
        counters = [7, 0, 224, 16, 0, 0, 0, 0, 0, 4, 16, 0, 0, 0, 0, 0, 0]
        if name in ("mmcblk0", "mmcblk0p7", "mmcblk0p8"):
            counters[4:7] = [34, 11, 256]
        write(os.path.join(d, "stat"), " ".join(f"{v:8d}" for v in counters) + "\n")
        write(os.path.join(d, "size"), f"{sectors}\n")
        uevent = f"MAJOR=179\nMINOR=0\nDEVNAME={name}\n"
        if start is not None:
            write(os.path.join(d, "start"), f"{start}\n")
            uevent += f"DEVTYPE=partition\nPARTN={partn}\nPARTNAME={pname}\n"
        else:
            uevent += "DEVTYPE=disk\n"
        write(os.path.join(d, "uevent"), uevent)

    def part(self, name):
        for n, _, start, sectors in LAYOUT:
            if n == name:
                return self.disk[start * SECTOR:(start + sectors) * SECTOR]
        raise KeyError(name)

    def baseline(self, overrides=None):
        overrides = overrides or {}
        lines = ["state TEST", f"emmc {overrides.get('emmc', 'DG4008')}",
                 f"disk mmcblk0 {DISK_SECTORS}"]
        for name, pname, start, sectors in LAYOUT:
            digest = "-" if name in ("mmcblk0p2", "mmcblk0p7", "mmcblk0p8") else sha(self.part(name))
            digest = overrides.get(name, digest)
            start = overrides.get(name + ".start", start)
            sectors = overrides.get(name + ".sectors", sectors)
            lines.append(f"part {name} {pname} {start} {sectors} {digest}")
        for name in ("mmcblk0boot0", "mmcblk0boot1"):
            lines.append(f"hw {name} {HW_SECTORS} {sha(self.zeros)}")
        path = os.path.join(self.base, "baseline.txt")
        write(path, "\n".join(lines) + "\n")
        return path


class BackupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-backup-test-")
        self.mpc = FakeMpc(self.tmp)
        self.bb = os.path.join(self.tmp, "busybox-bin")
        os.makedirs(self.bb)
        for applet in APPLETS:
            os.symlink(self.busybox, os.path.join(self.bb, applet))
        fakebin = os.path.join(self.tmp, "fakebin")
        os.makedirs(fakebin)
        ssh = os.path.join(fakebin, "ssh")
        write(ssh, f'#!/bin/sh\nexec "{sys.executable}" "{FAKE_SSH}" "$@"\n')
        os.chmod(ssh, 0o755)
        self.path = fakebin + os.pathsep + os.environ.get("PATH", "")
        self.out = os.path.join(self.tmp, "out")
        self.cmdlog = os.path.join(self.tmp, "commands.log")

    def tearDown(self):
        for dirpath, _, files in os.walk(self.tmp):
            os.chmod(dirpath, 0o700)
            for f in files:
                p = os.path.join(dirpath, f)
                if not os.path.islink(p):
                    os.chmod(p, 0o600)
        shutil.rmtree(self.tmp)

    def run_backup(self, baseline=None, **faults):
        env = dict(os.environ, PATH=self.path, FAKE_MPC_ROOT=self.mpc.root,
                   FAKE_MPC_LOG=self.cmdlog, FAKE_MPC_BUSYBOX_PATH=self.bb)
        for key in ("FAKE_MPC_ON_STREAM", "FAKE_MPC_CORRUPT", "FAKE_MPC_TRUNCATE",
                    "FAKE_MPC_DD_RC"):
            env.pop(key, None)
        env.update(faults)
        r = subprocess.run(
            [sys.executable, HELPER, "--ip", "192.0.2.10", "--port", "2222",
             "--user", "root", "--key", "/nonexistent-test-key",
             "--out-root", self.out, "--baseline", baseline or self.mpc.baseline(),
             "--preflight-script", PREFLIGHT, "--tool-version", "test"],
            env=env, capture_output=True, text=True, timeout=120)
        runs = os.listdir(self.out) if os.path.isdir(self.out) else []
        run_dir = os.path.join(self.out, runs[0]) if len(runs) == 1 else None
        with open(self.cmdlog) as fh:
            commands = fh.read().splitlines()
        return r.returncode, r.stdout + r.stderr, run_dir, commands

    def images(self, run_dir):
        return sorted(f for f in os.listdir(run_dir) if ".img" in f)

    def assert_failed_cleanly(self, rc, out, run_dir, text):
        self.assertEqual(rc, 1, out)
        self.assertIn("BACKUP FAILED", out)
        self.assertIn(text, out)
        self.assertEqual(self.images(run_dir), [], "partial images must be deleted")
        self.assertTrue(os.path.exists(os.path.join(run_dir, "MANIFEST-FAILED.txt")))

    def test_verified_backup(self):
        rc, out, run_dir, commands = self.run_backup()
        self.assertEqual(rc, 0, out)
        self.assertIn("BACKUP VERIFIED", out)
        self.assertEqual(commands, EXPECTED_COMMANDS)
        self.assertEqual(self.images(run_dir),
                         ["mmcblk0.img", "mmcblk0boot0.img", "mmcblk0boot1.img"])
        with open(os.path.join(run_dir, "mmcblk0.img"), "rb") as fh:
            self.assertEqual(fh.read(), self.mpc.disk)
        check = subprocess.run(["sha256sum", "-c", "SHA256SUMS"], cwd=run_dir,
                               capture_output=True, text=True)
        self.assertEqual(check.returncode, 0, check.stdout + check.stderr)
        for name in os.listdir(run_dir):
            self.assertEqual(stat.S_IMODE(os.stat(os.path.join(run_dir, name)).st_mode), 0o400)
        with open(os.path.join(run_dir, "MANIFEST.txt")) as fh:
            manifest = fh.read()
        self.assertIn("status OK", manifest)
        self.assertIn("emmc_model DG4008", manifest)
        self.assertIn("ext_csd 00", manifest)
        self.assertIn("mmcblk0p8: no writes during the copy", manifest)
        self.assertIn(sha(self.mpc.part("mmcblk0p6")), manifest)

    def test_data_partition_written_during_copy_is_reported(self):
        rc, out, run_dir, _ = self.run_backup(FAKE_MPC_ON_STREAM="write:mmcblk0p8")
        self.assertEqual(rc, 0, out)
        self.assertIn("mmcblk0p8: 8 sectors written during the copy", out)

    def test_rootfs_write_during_copy_fails(self):
        rc, out, run_dir, _ = self.run_backup(FAKE_MPC_ON_STREAM="write:mmcblk0p6")
        self.assert_failed_cleanly(rc, out, run_dir, "mmcblk0p6 was written or discarded")

    def test_bootloader_discard_during_copy_fails(self):
        rc, out, run_dir, _ = self.run_backup(FAKE_MPC_ON_STREAM="discard:mmcblk0p3")
        self.assert_failed_cleanly(rc, out, run_dir, "mmcblk0p3 was written or discarded")

    def test_reboot_during_copy_fails(self):
        rc, out, run_dir, _ = self.run_backup(FAKE_MPC_ON_STREAM="reboot")
        self.assert_failed_cleanly(rc, out, run_dir, "rebooted during the backup")

    def test_corrupted_bootloader_bytes_fail(self):
        offset = 256 * SECTOR + 1000  # inside p3 (U-Boot)
        rc, out, run_dir, _ = self.run_backup(FAKE_MPC_CORRUPT=f"mmcblk0:{offset}")
        self.assert_failed_cleanly(rc, out, run_dir,
                                   "mmcblk0p3: copied data does not match the MPC's own hash")

    def test_corrupted_partition_table_fails(self):
        rc, out, run_dir, _ = self.run_backup(FAKE_MPC_CORRUPT="mmcblk0:600")
        self.assert_failed_cleanly(rc, out, run_dir, "gpt-head: copied data does not match")

    def test_corrupted_boot0_fails(self):
        rc, out, run_dir, _ = self.run_backup(FAKE_MPC_CORRUPT="mmcblk0boot0:5")
        self.assert_failed_cleanly(rc, out, run_dir, "mmcblk0boot0: copied data does not match")

    def test_truncated_stream_fails(self):
        rc, out, run_dir, _ = self.run_backup(FAKE_MPC_TRUNCATE="mmcblk0:1000000")
        self.assert_failed_cleanly(rc, out, run_dir, "expected 2097152 bytes, received 1000000")

    def test_remote_dd_error_fails(self):
        rc, out, run_dir, _ = self.run_backup(FAKE_MPC_DD_RC="mmcblk0boot1:1")
        self.assert_failed_cleanly(rc, out, run_dir, "remote dd/ssh exit status 1")

    def test_interrupted_copy_deletes_partial_images(self):
        import signal
        import time
        env = dict(os.environ, PATH=self.path, FAKE_MPC_ROOT=self.mpc.root,
                   FAKE_MPC_LOG=self.cmdlog, FAKE_MPC_BUSYBOX_PATH=self.bb,
                   FAKE_MPC_PAUSE="mmcblk0:524288")
        proc = subprocess.Popen(
            [sys.executable, HELPER, "--ip", "192.0.2.10", "--port", "2222",
             "--user", "root", "--key", "/nonexistent-test-key", "--out-root", self.out,
             "--baseline", self.mpc.baseline(), "--preflight-script", PREFLIGHT],
            env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        deadline = time.monotonic() + 30
        partial = None
        while time.monotonic() < deadline and partial is None:
            for dirpath, _, files in os.walk(self.out) if os.path.isdir(self.out) else []:
                for f in files:
                    p = os.path.join(dirpath, f)
                    if f.endswith(".partial") and os.path.getsize(p) >= 524288:
                        partial = p
            time.sleep(0.05)
        self.assertIsNotNone(partial, "copy never started")
        proc.send_signal(signal.SIGTERM)
        out, _ = proc.communicate(timeout=30)
        self.assertEqual(proc.returncode, 130, out)
        self.assertIn("BACKUP FAILED: interrupted", out)
        self.assertFalse(os.path.exists(partial))

    def test_layout_mismatch_copies_nothing(self):
        baseline = self.mpc.baseline({"mmcblk0p8.sectors": 990})
        rc, out, run_dir, commands = self.run_backup(baseline=baseline)
        self.assert_failed_cleanly(rc, out, run_dir, "does not match the baseline")
        self.assertEqual(commands, ["sh -s hash"])

    def test_overlapping_baseline_is_rejected_before_contacting_the_mpc(self):
        baseline = self.mpc.baseline({"mmcblk0p6.start": 1600})
        r = subprocess.run(
            [sys.executable, HELPER, "--ip", "192.0.2.10", "--port", "2222",
             "--user", "root", "--key", "/nonexistent-test-key", "--out-root", self.out,
             "--baseline", baseline, "--preflight-script", PREFLIGHT],
            env=dict(os.environ, PATH=self.path, FAKE_MPC_ROOT=self.mpc.root,
                     FAKE_MPC_LOG=self.cmdlog, FAKE_MPC_BUSYBOX_PATH=self.bb),
            capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 1)
        self.assertIn("BACKUP NOT STARTED", r.stderr)
        self.assertFalse(os.path.exists(self.cmdlog), "no remote command may be sent")

    def test_wrong_emmc_model_copies_nothing(self):
        rc, out, run_dir, commands = self.run_backup(baseline=self.mpc.baseline({"emmc": "OTHER1"}))
        self.assert_failed_cleanly(rc, out, run_dir, "eMMC model DG4008 != OTHER1")
        self.assertEqual(commands, ["sh -s hash"])

    def test_changed_boot_storage_keeps_backup_and_stops(self):
        baseline = self.mpc.baseline({"mmcblk0p1": "0" * 64})
        rc, out, run_dir, _ = self.run_backup(baseline=baseline)
        self.assertEqual(rc, 3, out)
        self.assertIn("BACKUP VERIFIED", out)
        self.assertIn("STOP: boot-critical storage differs", out)
        self.assertEqual(len(self.images(run_dir)), 3)

    def test_preflight_runs_under_busybox_and_reads_geometry(self):
        env = dict(os.environ, MPFREE_TEST_ROOT=self.mpc.root, PATH=self.bb)
        with open(PREFLIGHT, "rb") as fh:
            r = subprocess.run([os.path.join(self.bb, "sh"), "-s", "hash"], stdin=fh,
                               env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stderr, "")
        self.assertIn("GEOM mmcblk0p6 1536 1024 rootfs", r.stdout)
        self.assertIn("GEOM mmcblk0 - 4096 -", r.stdout)
        self.assertIn(f"HASH gpt-head {sha(self.mpc.disk[:64 * SECTOR])}", r.stdout)
        self.assertIn("EMMC DG4008", r.stdout)
        self.assertTrue(r.stdout.rstrip().endswith("END hash"))


class ReferenceBaselineTests(unittest.TestCase):
    def test_reference_baseline_matches_the_boot_map(self):
        sys.path.insert(0, TOOLS)
        try:
            import host_emmc_backup as helper
        finally:
            sys.path.pop(0)
        base = helper.parse_baseline(os.path.join(TOOLS, "baselines/mpc-one-3.6.0-reference.txt"))
        self.assertEqual(base["disk"]["sectors"], 15273600)
        self.assertEqual(base["emmc"], "DG4008")
        parts = {p["dev"]: p for p in base["parts"]}
        self.assertEqual((parts["mmcblk0p1"]["start"], parts["mmcblk0p1"]["sectors"]), (64, 128))
        self.assertEqual(parts["mmcblk0p6"]["sha256"],
                         "fa2ca12bfac82f5ca69238f8b85c1f1df18994486b385086ebdda26ae61b2350")
        self.assertIsNone(parts["mmcblk0p2"]["sha256"])
        last = parts["mmcblk0p8"]["start"] + parts["mmcblk0p8"]["sectors"]
        self.assertEqual(base["disk"]["sectors"] - last, 33, "backup GPT tail")
        zeros = hashlib.sha256(bytes(8192 * SECTOR)).hexdigest()
        self.assertTrue(all(h["sha256"] == zeros for h in base["hw"]))


if __name__ == "__main__":
    unittest.main(verbosity=2)
