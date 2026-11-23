#!/usr/bin/env python3
"""Tests for host_disktest_mpc.py against a fake test build. No device.

As in test_app_mpc.py, a fake ssh runs the tool's real remote scripts under
BusyBox applets against a fake root folder (setting the scripts' R='' root
prefix to it), accepts only the test build's host key alias, and only while
the fake MPC is "on the test build". Running the disk test is the one other
change: a shell stand-in logs its arguments and prints what the real one
would (or refuses, with exit code 3, as the real one does off the stick).
The streaming survey (read-only-stream-survey.sh) runs for real against a
fake /proc and /sys for the stick, its USB path and the pad player.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import hashlib
import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
HELPER = os.path.join(TOOLS, "host_disktest_mpc.py")
_spec = importlib.util.spec_from_file_location("test_app_mpc", os.path.join(HERE, "test_app_mpc.py"))
th = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(th)

FAKE_SSH = th.FAKE_SSH.replace(
    "program = 'exec \"$R/run/mpfree-app/mpfree\"'\n"
    "if program in script:\n"
    "    script = script.replace(program, 'exec \"$FAKE_PROGRAM\"')",
    "program = 'exec \"$R/run/mpfree-disktest/mpfree-disktest\"'\n"
    "if program in script:\n"
    "    script = script.replace(program, 'exec \"$FAKE_PROGRAM\"')")
assert FAKE_SSH != th.FAKE_SSH, "the fake ssh's program line moved in test_app_mpc.py"

SURVEY = os.path.join(TOOLS, "read-only-stream-survey.sh")
EXTRA_APPLETS = ("head", "sed", "tr", "basename", "cut")
SECRET_SERIAL = "7Q4X9Z0SECRET"
SECRET_VOLUME_ID = "1A2B-3C4D"

FAKE_DISKTEST = r'''#!/bin/sh
echo "disktest: [$*]" >> "$FAKE_LOG"
[ -x ./mpfree-disktest ] || { echo "STOP: not copied"; exit 1; }
if [ "$FAKE_DISKTEST_RC" = 3 ]; then echo "mpfree-disktest: refused: 40.0 MiB free" >&2; exit 3; fi
if [ -n "$FAKE_VM_CHANGE" ]; then echo 33554432 > "$FAKE_ROOT/proc/sys/vm/dirty_bytes"; fi
echo "free before: 900.0 MiB on /run/mpfree-data"
echo "  write: 1488 ops, p50 0.10 ms, p99 31.00 ms, max 412.00 ms; over 50/100/250/500 ms: 9/4/1/0"
echo "free after: 900.0 MiB (the folder is deleted)"
exit "$FAKE_DISKTEST_RC"
'''


def stat_line(pid, comm, policy=0, prio=0, cpu=0):
    """A /proc stat line: fields 39 to 41 are the last CPU, the RT priority and the policy."""
    return " ".join([str(pid), f"({comm})", "S"] + ["0"] * 35 + [str(cpu), str(prio), str(policy), "0", "0"]) + "\n"


def survey_root(r):
    """The stick, its USB path, the pad player and the system, as the survey reads them."""
    def put(rel, text):
        th.write(os.path.join(r, rel), text)

    def link(target, rel):
        path = os.path.join(r, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        if os.path.isdir(path) and not os.path.islink(path):
            os.rmdir(path)
        os.symlink(target, path)

    put("proc/mounts", "/dev/root / ext4 ro,relatime 0 0\ntmpfs /run tmpfs rw,nosuid,nodev,mode=755 0 0\n"
        "/dev/sda4 /run/mpfree-data ext4 rw,nosuid,nodev,noexec,noatime 0 0\n"
        f"/dev/sda2 /media/{SECRET_VOLUME_ID} vfat rw,relatime 0 0\n")
    put("proc/cmdline", f"console=ttyS2 androidboot.serialno={SECRET_SERIAL} isolcpus=2-3 threadirqs rootwait\n")
    put("sys/kernel/realtime", "1\n")
    os.makedirs(f"{r}/run/mpfree-data/library", exist_ok=True)
    usb = "sys/devices/platform/ff500000.usb"
    dev = f"{usb}/usb2/2-1"
    scsi = f"{dev}/2-1:1.0/host0/target0:0:0/0:0:0:0"
    blk = f"{scsi}/block/sda"
    for name, text in (("dev", "8:0"), ("size", "60437492"), ("removable", "1"), ("ro", "0"),
                       ("stat", "  1200  30  96000  4100  800  10  64000  9000  0  7000  13100")):
        put(f"{blk}/{name}", text + "\n")
    for name, text in (("scheduler", "[mq-deadline] none"), ("nr_requests", "2"), ("max_sectors_kb", "120"),
                       ("max_hw_sectors_kb", "120"), ("read_ahead_kb", "128"), ("rotational", "1"),
                       ("write_cache", "write through"), ("logical_block_size", "512")):
        put(f"{blk}/queue/{name}", text + "\n")
    for name, text in (("partition", "4"), ("start", "4194304"), ("size", "2097152"), ("dev", "8:4"),
                       ("stat", "  300  0  24000  900  200  0  16000  2000  0  1500  2900")):
        put(f"{blk}/sda4/{name}", text + "\n")
    for name, text in (("vendor", "SanDisk"), ("model", "Ultra Fit"), ("rev", "1.00"), ("queue_depth", "1")):
        put(f"{scsi}/{name}", text + "\n")
    link("../../../0:0:0:0", f"{blk}/device")
    link(f"../{blk[len('sys/'):]}", "sys/block/sda")
    link(f"../../{blk[len('sys/'):]}/sda4", "sys/class/block/sda4")
    for name, text in (("idVendor", "0781"), ("idProduct", "5583"), ("manufacturer", "SanDisk"),
                       ("product", "Ultra Fit"), ("version", " 2.10"), ("speed", "480"), ("bMaxPower", "224mA"),
                       ("serial", SECRET_SERIAL)):
        put(f"{dev}/{name}", text + "\n")
    for name, text in (("bInterfaceClass", "08"), ("bInterfaceSubClass", "06"), ("bInterfaceProtocol", "50")):
        put(f"{dev}/2-1:1.0/{name}", text + "\n")
    os.makedirs(f"{r}/sys/bus/usb/drivers/usb-storage")
    link("../../../../../bus/usb/drivers/usb-storage", f"{dev}/2-1:1.0/driver")
    put(f"{usb}/usb2/speed", "480\n")
    put(f"{usb}/usb2/version", " 2.00\n")
    os.makedirs(f"{r}/sys/bus/platform/drivers/ehci-platform")
    link("../../../bus/platform/drivers/ehci-platform", f"{usb}/driver")
    for name, text in (("read_ahead_kb", "128"), ("min_ratio", "0"), ("max_ratio", "100"), ("strict_limit", "0")):
        put(f"sys/class/bdi/8:0/{name}", text + "\n")
    for name, text in (("dirty_ratio", "20"), ("dirty_background_ratio", "10"), ("dirty_bytes", "0"),
                       ("dirty_expire_centisecs", "3000"), ("dirty_writeback_centisecs", "500")):
        put(f"proc/sys/vm/{name}", text + "\n")
    put("proc/meminfo", "MemTotal:        2050000 kB\nMemFree:          900000 kB\nMemAvailable:    1500000 kB\n"
        "Cached:           400000 kB\nDirty:                12 kB\nWriteback:             0 kB\nShmem: 1 kB\n")
    put("proc/interrupts", "           CPU0       CPU1       CPU2       CPU3\n"
        " 56:       5120          0          0          0     GIC-0  56 Level     ehci_hcd:usb2\n"
        " 57:         10          0          0          0     GIC-0  57 Level     ohci_hcd:usb3\n"
        " 60:       9000          0          0          0     GIC-0  60 Level     ff890000.i2s\n")
    put("proc/irq/56/smp_affinity_list", "0-3\n")
    put("proc/irq/56/effective_affinity_list", "0\n")
    for pid, comm, policy, prio, cpus in ((100, "irq/56-ehci_hcd:usb2", 1, 50, "0-3"),
                                          (101, "usb-storage", 0, 0, "0-3"), (300, "edisksd", 0, 0, "0-3")):
        put(f"proc/{pid}/comm", comm + "\n")
        put(f"proc/{pid}/stat", stat_line(pid, comm, policy, prio))
        put(f"proc/{pid}/status", f"Name:\t{comm}\nCpus_allowed_list:\t{cpus}\n")
    put("proc/200/comm", "mpfree\n")
    for tid, comm, policy, prio, cpus in ((200, "mpfree", 0, 0, "0-2"), (201, "mpfree-audio", 1, 70, "3"),
                                          (202, "mpfree-ui", 0, 0, "0-2")):
        put(f"proc/200/task/{tid}/comm", comm + "\n")
        put(f"proc/200/task/{tid}/stat", stat_line(tid, comm, policy, prio, cpu=3 if policy else 0))
        put(f"proc/200/task/{tid}/status", f"Name:\t{comm}\nCpus_allowed_list:\t{cpus}\n")
    put("sys/devices/system/cpu/online", "0-3\n")
    put("sys/devices/system/cpu/isolated", "\n")
    for c in range(4):
        for name, text in (("scaling_governor", "performance"), ("scaling_cur_freq", "1608000"),
                           ("scaling_min_freq", "126000"), ("scaling_max_freq", "1608000")):
            put(f"sys/devices/system/cpu/cpu{c}/cpufreq/{name}", text + "\n")
    put("proc/filesystems", "nodev\tsysfs\n\text4\n\tvfat\n\texfat\n")
    put("proc/modules", "exfat 90112 0 - Live 0x0\nsnd_soc_jp07 16384 1 - Live 0x0\n")
    put("lib/systemd/system/edisksd.service", "[Service]\nExecStart=/usr/bin/edisksd\n")
    put("kernel-log", "[    1.0] usb 2-1: new high-speed USB device number 2 using ehci-platform\n"
        f"[    1.1] usb 2-1: SerialNumber: {SECRET_SERIAL}\n"
        "[    1.2] usb-storage 2-1:1.0: USB Mass Storage device detected\n"
        "[    1.3] sd 0:0:0:0: [sda] 60437492 512-byte logical blocks: (30.9 GB/28.8 GiB)\n"
        "[    2.0] EXT4-fs (sda4): mounted filesystem with ordered data mode\n"
        "[    3.0] random: crng init done\n")


class DiskTestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-disktest-test-")
        t = self.tmp
        self.work, self.log, self.stage = os.path.join(t, "work"), os.path.join(t, "log"), os.path.join(t, "stage")
        self.root, self.report = os.path.join(t, "root310"), os.path.join(t, "report.txt")
        th.write(self.stage, "310\n")
        th.write(self.log, "")
        fakebin, remote = os.path.join(t, "fakebin"), os.path.join(t, "remote-bin")
        th.write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        os.makedirs(remote)
        for name in th.APPLETS + EXTRA_APPLETS:
            if not os.path.exists(os.path.join(remote, name)):
                os.symlink(self.busybox, os.path.join(remote, name))
        th.write(os.path.join(remote, "systemctl"), th.FAKE_SYSTEMCTL, 0o755)
        screen, app = os.path.join(t, "screen-state"), os.path.join(t, "app-state")
        th.write(screen, "inactive\n")
        th.write(app, "active\n")
        standin = os.path.join(t, "disktest-standin")
        th.write(standin, FAKE_DISKTEST, 0o755)
        self.env = {"PATH": fakebin + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"), "HOME": t,
                    "MPFREE_APP_TEST": "1", "MPFREE_ACCEPT_DISKTEST": "YES", "FAKE_STAGE": self.stage,
                    "FAKE_LOG": self.log, "FAKE_ROOT": self.root, "FAKE_REMOTE_PATH": remote, "FAKE_SCREEN": screen,
                    "FAKE_APP": app, "FAKE_ACVS": "inactive", "FAKE_PROGRAM": standin, "FAKE_DISKTEST_RC": "0"}
        self.key = os.path.join(t, "key")
        th.write(self.key, "fake key\n", 0o600)
        self.program = os.path.join(t, "build", "mpfree-disktest")
        th.write(self.program, th.arm_elf(), 0o755)
        folder = os.path.join(self.work, "kexec-stick-20260101T000000Z-abc")
        th.write(os.path.join(folder, "stick.img"), b"\x00" * 4096)
        th.write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        th.write(os.path.join(folder, "report.txt"), f"its kernel release: {th.NEW}\n")
        r = self.root
        th.write(f"{r}/proc/sys/kernel/osrelease", th.NEW + "\n")
        th.write(f"{r}/proc/sys/kernel/kexec_load_disabled", "1\n")
        os.makedirs(f"{r}/sys/block/sda")
        th.write(f"{r}/proc/mounts", "/dev/root / ext4 ro,relatime 0 0\ntmpfs /run tmpfs rw,nosuid,nodev,mode=755 0 0\n")
        os.makedirs(f"{r}/run/mpfree-data")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_helper(self, answers="\n" * 2, extra=(), **env):
        cmd = [sys.executable, "-I", HELPER, "--ip", "192.0.2.141", "--key", self.key, "--work", self.work,
               "--report", self.report, "--program", self.program, *extra]
        return subprocess.run(cmd, input=answers, env=dict(self.env, **env), text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, timeout=120)

    def lines(self, prefix):
        with open(self.log) as fh:
            return [ln[len(prefix):].strip() for ln in fh.read().splitlines() if ln.startswith(prefix)]

    def test_copies_the_program_and_runs_it_with_no_arguments(self):
        res = self.run_helper()
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("static ARM EABI5 hard-float", "CHECKS-OK", "/run/mpfree-data",
                     "copied; its hash on the MPC matches", "WRITES test files in /run/mpfree-data/mpfree-disktest",
                     "over 50/100/250/500 ms: 9/4/1/0", "the folder is deleted", "RESULT: the disk test ran",
                     "MPFREE STREAM SURVEY - READ ONLY", "END OF STREAM SURVEY"):
            self.assertIn(text, res.stdout)
        self.assertEqual(self.lines("disktest: "), ["[]"])
        # Read-only on services: only is-active, never a start or stop.
        self.assertTrue(all(c.startswith("is-active ") for c in self.lines("systemctl: ")), self.lines("systemctl: "))
        with open(self.program, "rb") as a, open(f"{self.root}/run/mpfree-disktest/mpfree-disktest", "rb") as b:
            self.assertEqual(hashlib.sha256(a.read()).digest(), hashlib.sha256(b.read()).digest())
        self.assertEqual(os.stat(f"{self.root}/run/mpfree-disktest/mpfree-disktest").st_mode & 0o777, 0o700)
        self.assertEqual(sorted(os.listdir(f"{self.root}/run")), ["mpfree-data", "mpfree-disktest"])
        self.assertEqual(os.listdir(f"{self.root}/run/mpfree-data"), [])
        with open(self.report) as fh:
            self.assertIn("RESULT: the disk test ran", fh.read())

    def test_a_refusal_is_reported_as_nothing_written(self):
        res = self.run_helper(FAKE_DISKTEST_RC="3")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("refused: 40.0 MiB free", res.stdout)
        self.assertIn("RESULT: refused by the disk test; nothing was written on the stick.", res.stdout)

    def test_a_failed_run_says_how_to_clean_up(self):
        res = self.run_helper(FAKE_DISKTEST_RC="1")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("ended with exit code 1", res.stdout)
        self.assertIn("rm -r /run/mpfree-data/mpfree-disktest", res.stdout)

    def test_needs_the_opt_in_and_refuses_stock(self):
        res = self.run_helper(MPFREE_ACCEPT_DISKTEST="")
        self.assertEqual(res.returncode, 1)
        self.assertIn("opt-in required", res.stdout)
        th.write(self.stage, "36\n")
        res = self.run_helper()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("the test build did not answer", res.stdout)
        self.assertEqual(self.lines("disktest: "), [])
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-disktest"))

    def test_stops_on_a_dynamic_program_before_any_ssh(self):
        th.write(self.program, th.arm_elf(interp=True), 0o755)
        res = self.run_helper()
        self.assertEqual(res.returncode, 1)
        self.assertIn("dynamically linked", res.stdout)
        self.assertEqual(self.lines("ssh["), [])

    def test_q_stops_before_the_copy_and_s_skips_the_run(self):
        res = self.run_helper(answers="q\n")
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: stopped by you", res.stdout)
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-disktest"))
        res = self.run_helper(answers="\ns\n")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("skipped: nothing was written on the stick", res.stdout)
        self.assertEqual(self.lines("disktest: "), [])

    def test_survey_only_reads_and_copies_nothing(self):
        survey_root(self.root)
        res = self.run_helper(answers="", extra=["--survey-only"], MPFREE_ACCEPT_DISKTEST="")
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("[1/2] Workstation checks", "[2/2] Checks and the streaming survey", "CHECKS-OK",
                     "MPFREE STREAM SURVEY - READ ONLY", "release: 6.", "real-time kernel (/sys/kernel/realtime): 1",
                     "command line, CPU and interrupt settings only: isolcpus=2-3 threadirqs\n",
                     "mounted: /dev/sda4 type ext4 (rw,nosuid,nodev,noexec,noatime)",
                     "library folder: present", "partition: sda4, number 4, start 4194304, 2097152 sectors",
                     "disk: sda (8:0), 60437492 sectors, removable 1, read-only 0", "  vendor: SanDisk",
                     "  scheduler: [mq-deadline] none", "  nr_requests: 2", "  max_sectors_kb: 120",
                     "  read_ahead_kb: 128", "  write_cache: write through",
                     "  sda: 1200 30 96000 4100 800 10 64000 9000 0 7000 13100",
                     "USB device: 2-1", "  idVendor: 0781", "  speed: 480",
                     "interface 2-1:1.0: class 08, subclass 06, protocol 50, driver usb-storage",
                     "root hub: usb2, speed 480, version  2.00; host controller: ff500000.usb, driver ehci-platform",
                     "  dirty_ratio: 20", "  dirty_expire_centisecs: 3000", "for the stick (/sys/class/bdi/8:0):",
                     "  max_ratio: 100", "MemAvailable: 1500000 kB",
                     "56: 5120 0 0 0 GIC-0 56 Level ehci_hcd:usb2", "IRQ 56: CPUs 0-3 (effective 0)",
                     "thread irq/56-ehci_hcd:usb2: policy FIFO, priority 50, last CPU 0, CPUs 0-3",
                     "usb-storage: policy other, priority 0, last CPU 0, CPUs 0-3",
                     "pad player: pid 200", "mpfree-audio: policy FIFO, priority 70, last CPU 3, CPUs 3",
                     "online: 0-3, isolated: ", "cpu3: governor performance, now 1608000 kHz",
                     "file systems the kernel reads now: vfat exfat ext4\n", "disk modules loaded: exfat\n",
                     "block devices: sda\n", "/dev/sda4 /run/mpfree-data ext4", "/dev/sda2 /media/<uuid> vfat",
                     "Akai's disk service (edisksd): running yes, unit /lib/systemd/system/edisksd.service",
                     "usb-storage 2-1:1.0: USB Mass Storage device detected", "EXT4-fs (sda4): mounted",
                     "SerialNumber: <serial>", "END OF STREAM SURVEY",
                     "RESULT: the survey ran (above); nothing was copied to or run on the MPC."):
            self.assertIn(text, res.stdout)
        for absent in (SECRET_SERIAL, SECRET_VOLUME_ID, "ff890000.i2s", "mpfree-ui", "crng init", "console=",
                       "[3/", "Copy the program"):
            self.assertNotIn(absent, res.stdout)
        self.assertEqual(self.lines("disktest: "), [])
        self.assertFalse(os.path.exists(f"{self.root}/run/mpfree-disktest"))
        # Only read-only scripts went over ssh: the checks, df and the survey.
        self.assertEqual(len(self.lines("ssh[")), 3)

    def test_read_only_runs_the_read_only_test(self):
        survey_root(self.root)
        res = self.run_helper(extra=["--read-only"], MPFREE_ACCEPT_DISKTEST="READ-ONLY")
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("disk test on the test build, read-only", "END OF STREAM SURVEY",
                     "copied; its hash on the MPC matches",
                     "[4/4] The read-only disk test: READS the audio in /run/mpfree-data/library and the backup",
                     "RESULT: the read-only disk test ran (numbers above); nothing was written on the stick."):
            self.assertIn(text, res.stdout)
        self.assertNotIn("WRITES test files", res.stdout)
        self.assertEqual(self.lines("disktest: "), ["[--read-only]"])
        self.assertEqual(sorted(os.listdir(f"{self.root}/run/mpfree-data")), ["library"])
        # YES (the writing run's opt-in) covers the read-only run too.
        res = self.run_helper(extra=["--read-only"], MPFREE_ACCEPT_DISKTEST="YES")
        self.assertEqual(res.returncode, 0, res.stdout)

    def test_each_mode_needs_its_own_opt_in(self):
        res = self.run_helper(extra=["--read-only"], MPFREE_ACCEPT_DISKTEST="")
        self.assertEqual(res.returncode, 1)
        self.assertIn("MPFREE_ACCEPT_DISKTEST=READ-ONLY", res.stdout)
        # The read-only opt-in does not allow the writing run.
        res = self.run_helper(MPFREE_ACCEPT_DISKTEST="READ-ONLY")
        self.assertEqual(res.returncode, 1)
        self.assertIn("writes test files", res.stdout)
        self.assertEqual(self.lines("disktest: "), [])
        self.assertEqual(self.lines("ssh["), [])
        res = self.run_helper(extra=["--survey-only", "--read-only"])
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("not allowed with argument", res.stdout)

    def test_under_load_runs_the_test_under_load(self):
        survey_root(self.root)
        res = self.run_helper(extra=["--under-load"])
        self.assertEqual(res.returncode, 0, res.stdout)
        for text in ("disk test under load on the test build", "END OF STREAM SURVEY",
                     "copied; its hash on the MPC matches",
                     "[4/4] The disk test under load: READS the audio in /run/mpfree-data/library",
                     "WRITES test files in /run/mpfree-data/mpfree-disktest",
                     "keeps the processor busy (it stops at 75 degrees C)",
                     "RESULT: the disk test under load ran (numbers above)."):
            self.assertIn(text, res.stdout)
        self.assertEqual(self.lines("disktest: "), ["[--under-load]"])
        # Only YES allows it: it writes on the stick.
        res = self.run_helper(extra=["--under-load"], MPFREE_ACCEPT_DISKTEST="READ-ONLY")
        self.assertEqual(res.returncode, 1)
        self.assertIn("MPFREE_ACCEPT_DISKTEST=YES python3 tools/mpc-one/host_disktest_mpc.py --under-load",
                      res.stdout)
        self.assertEqual(self.lines("disktest: "), ["[--under-load]"])
        res = self.run_helper(extra=["--read-only", "--under-load"])
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("not allowed with argument", res.stdout)

    def test_recorder_runs_the_recorder_test_and_checks_the_writeback_settings(self):
        survey_root(self.root)
        th.write(f"{self.root}/proc/sys/vm/dirty_bytes", "1048576\n")
        th.write(f"{self.root}/proc/sys/vm/dirty_ratio", "0\n")
        th.write(f"{self.root}/proc/sys/vm/dirty_background_bytes", "51200\n")
        th.write(f"{self.root}/proc/sys/vm/dirty_background_ratio", "0\n")
        res = self.run_helper(extra=["--recorder"])
        self.assertEqual(res.returncode, 0, res.stdout)
        settings = "dirty_bytes=1048576, dirty_ratio=0, dirty_background_bytes=51200, dirty_background_ratio=0"
        for text in ("MPFree recorder test on the test build", "END OF STREAM SURVEY",
                     "writeback settings now: " + settings,
                     "copied; its hash on the MPC matches",
                     "[4/4] The recorder test: READS the audio in /run/mpfree-data/library",
                     "WRITES test takes in /run/mpfree-data/mpfree-disktest",
                     "sets /proc/sys/vm/dirty_bytes to 33554432, then back to the value it found (RAM only)",
                     "writeback settings as before the run: " + settings,
                     "RESULT: the recorder test ran (numbers above)."):
            self.assertIn(text, res.stdout)
        self.assertEqual(self.lines("disktest: "), ["[--recorder]"])
        # Only YES allows it.
        res = self.run_helper(extra=["--recorder"], MPFREE_ACCEPT_DISKTEST="READ-ONLY")
        self.assertEqual(res.returncode, 1)
        self.assertIn("MPFREE_ACCEPT_DISKTEST=YES python3 tools/mpc-one/host_disktest_mpc.py --recorder",
                      res.stdout)
        self.assertEqual(self.lines("disktest: "), ["[--recorder]"])
        res = self.run_helper(extra=["--recorder", "--under-load"])
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("not allowed with argument", res.stdout)

    def test_recorder_says_when_the_writeback_settings_were_not_put_back(self):
        survey_root(self.root)
        th.write(f"{self.root}/proc/sys/vm/dirty_bytes", "1048576\n")
        res = self.run_helper(extra=["--recorder"], FAKE_VM_CHANGE="1")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("WRITEBACK SETTINGS CHANGED: dirty_bytes=33554432", res.stdout)
        self.assertIn("(before: dirty_bytes=1048576", res.stdout)
        self.assertIn("switching the MPC off and on puts them back", res.stdout)

    def test_a_failed_under_load_run_says_how_to_clean_up(self):
        res = self.run_helper(extra=["--under-load"], FAKE_DISKTEST_RC="1")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("ended with exit code 1", res.stdout)
        self.assertIn("rm -r /run/mpfree-data/mpfree-disktest", res.stdout)

    def test_a_failed_read_only_run_says_it_wrote_nothing(self):
        res = self.run_helper(extra=["--read-only"], MPFREE_ACCEPT_DISKTEST="READ-ONLY", FAKE_DISKTEST_RC="1")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("mpfree-disktest --read-only ended with exit code 1; it writes nothing on the stick",
                      res.stdout)
        self.assertNotIn("rm -r", res.stdout)


class SurveyStaticTests(unittest.TestCase):
    def test_survey_only_reads(self):
        with open(SURVEY) as fh:
            text = fh.read()
        self.assertTrue(text.startswith("R=''\n"))
        lines = [line for line in text.splitlines() if not line.lstrip().startswith("#")]
        # Text in single quotes is printed or is a grep or sed pattern, never a command.
        code = "\n".join(re.sub(r"'[^']*'", "''", line) for line in lines)
        for line in code.splitlines():
            stripped = line.replace("2>/dev/null", "")
            self.assertNotIn(">", stripped, f"the survey redirects output: {line}")
        for forbidden in (r"\bmount\b", r"\bumount\b", r"\bblkid\b", r"\bfsck", r"\bmkfs", r"\btune2fs\b",
                          r"\bhdparm\b", r"\bsysctl\b", r"\bsystemctl\b", r"\becho\s+\S+\s*>", r"\btee\b",
                          r"\bdd\b", r"\brm\b", r"\bmv\b", r"\bcp\b", r"\bmkdir\b", r"\btouch\b",
                          r"\bmodprobe\b", r"\binsmod\b", r"\brmmod\b", r"\bkill\b", r"\bchrt\b",
                          r"\btaskset\b", r"\budevadm\b", r"/bind\b", r"/unbind\b", r"dmesg\s+-", r"/dev/sd",
                          r"/dev/mmcblk", r"drop_caches", r"\bsync\b"):
            self.assertIsNone(re.search(forbidden, code), forbidden)

    def test_the_tool_streams_it_in_step_2(self):
        with open(HELPER) as fh:
            text = fh.read()
        self.assertIn('SURVEY = os.path.join(HERE, "read-only-stream-survey.sh")', text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
