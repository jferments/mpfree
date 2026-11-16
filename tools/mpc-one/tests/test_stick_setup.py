#!/usr/bin/env python3
"""Tests for the setup scripts of the stick the MPFree USB creator makes (tools/mpc-one/stick/). No device.

- setup-backup.sh, the first-start wizard's read-only eMMC window, runs under dash with BusyBox applets
  against a fake kernel: a fake sysfs whose drivers_probe and unbind files are FIFOs served by a thread, as
  the eMMC controller and udev would answer them, each disk's ro file following its force_ro file as the
  MMC driver's do, and /dev files holding a small fake eMMC. It is rehearsed through every way it can stop.
- setup-format.sh, which makes partition 4's filesystem once, against a fake partition file and a stand-in
  mke2fs.
- Both run with nothing on PATH but the programs their NEEDS line names (the USB creator checks Akai's
  system has each one), so a program missing from Akai's system shows up here: the MPC's BusyBox has no
  blockdev, and its od has no -A or -t (a stand-in od refuses them).
- autorun.sh, the script Akai's script runner runs, against test_boot_hook.py's fake MPC: it runs the real
  hook-install.sh and hook-remove.sh there.

Every script's first line R='' becomes the fake root. Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead
of skip without BusyBox or dash.
"""
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_boot_hook import FAKE_SYSTEMCTL, RELEASE, SCRIPT, HookFixture  # noqa: E402
from test_hook_canary import write as write_text  # noqa: E402

TOOLS = os.path.dirname(HERE)
STICK = os.path.join(TOOLS, "stick")
BACKUP = os.path.join(STICK, "setup-backup.sh")
FORMAT = os.path.join(STICK, "setup-format.sh")
AUTORUN = os.path.join(STICK, "autorun.sh")
BASELINE = os.path.join(TOOLS, "baselines", "mpc-one-3.6.0-reference.txt")
EMMC = "ff0f0000.dwmmc"
SD = "ff0c0000.dwmmc"
# A small eMMC with the MPC One's eight partition names: number, name, start, sectors (None: to the end).
TEST_LAYOUT = ((1, "uboot-spl", 64, 64), (2, "env", 128, 16), (3, "uboot", 144, 112), (4, "splash", 256, 256),
               (5, "recoverysplash", 512, 256), (6, "rootfs", 768, 256), (7, "content", 1024, 512),
               (8, "data", 1536, None))
DISK_SECTORS = 2048
BOOT_SECTORS = 16
STAT = "     120        0     4000      30 {w} 0 {ws} 0        0       40       30 {d} 0 0 0 0 0\n"
DATA_UUID = "6b1f5a3c-9e2d-4c7b-8a10-3f2e1d0c9b8a"
TOKEN = "0123456789abcdef0123456789abcdef"


def write(path, data, mode=0o644):
    if isinstance(data, str):
        return write_text(path, data, mode)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as fh:
        fh.write(data)
    os.chmod(path, mode)


def read_text(path):
    if not os.path.exists(path):
        return ""
    with open(path) as fh:
        return fh.read()


def need_busybox():
    busybox, dash = shutil.which("busybox"), shutil.which("dash")
    if not busybox or not dash:
        if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
            raise RuntimeError("BusyBox and dash are required for these tests")
        raise unittest.SkipTest("BusyBox or dash not installed")
    return busybox, dash


def needs(path):
    """The programs a stick script names in its NEEDS='...' line."""
    with open(path) as fh:
        (line,) = [line for line in fh.read().splitlines() if line.startswith("NEEDS='")]
    return line[len("NEEDS='"):-1].split()


# The MPC's BusyBox od (no CONFIG_DESKTOP) has no -A or -t; this one refuses them too.
OD = ('#!/bin/sh\nfor a; do case "$a" in -A*|-t*) echo "od: invalid option -- ${a#-}" >&2; exit 1 ;; esac; done\n'
      'exec {bb} od "$@"\n')
# Its dd has no conv=, iflag=, oflag=, ibs=, obs= or status= (built without FEATURE_DD_IBS_OBS: the second
# creator stick's format step stopped on conv=notrunc); this one refuses them too.
DD_REFUSES = ('for a; do case "$a" in conv=*|iflag=*|oflag=*|ibs=*|obs=*|status=*) '
              'echo "dd: unknown operand ${a%%=*}" >&2; exit 1 ;; esac; done\n')


def program_dir(folder, busybox, wanted, fakes, extras=()):
    """A PATH folder with exactly the programs `wanted` names (plus `extras`): each a stand-in from `fakes`
    or BusyBox's applet."""
    os.makedirs(folder)
    unused = set(fakes) - set(wanted) - set(extras)
    if unused:
        raise AssertionError(f"stand-ins for programs the script does not name in NEEDS: {sorted(unused)}")
    for name in list(wanted) + list(extras):
        if name in fakes:
            write(os.path.join(folder, name), fakes[name].replace("{bb}", busybox), 0o755)
        else:
            os.symlink(busybox, os.path.join(folder, name))


def layout_text(layout):
    return "\n".join(f"{n} {name} {start} {'-' if size is None else size}" for n, name, start, size in layout)


def blob(seed, size):
    out, block = bytearray(), seed.encode()
    while len(out) < size:
        block = hashlib.sha256(block).digest()
        out += block
    return bytes(out[:size])


class FakeEmmc:
    """The eMMC controller as the kernel and udev answer setup-backup.sh. Writing the controller's name to
    /sys/bus/platform/drivers_probe binds it: the block devices appear in /sys/block and /dev (with udev's
    /dev/disk links). Each disk's ro file, and its partitions', follow the disk's force_ro file (they are
    links to it), as the MMC driver's do: writing 1 there marks the whole disk read-only. boot0 and boot1
    start with force_ro = 1, as the kernel makes them; the user area only if the window's udev rule was
    there first (udev writes its force_ro). Writing the name to the driver's unbind file takes them all
    away again. Options make it misbehave: `writable`, a disk that stays writable whatever force_ro says;
    `ignore_rule`, udev that does not apply the rule."""

    def __init__(self, root, layout=TEST_LAYOUT, ignore_unbind=False, no_disk=False, extra_disk=False,
                 writable=False, ignore_rule=False):
        self.root, self.layout = root, layout
        self.ignore_unbind, self.no_disk, self.extra_disk, self.writable = ignore_unbind, no_disk, extra_disk, writable
        self.ignore_rule = ignore_rule
        self.force_ro_at_unbind = {}
        self.bound, self.unbound, self.rule = [], [], None
        self.images = {"mmcblk0": blob("user area", DISK_SECTORS * 512),
                       "mmcblk0boot0": blob("boot0", BOOT_SECTORS * 512),
                       "mmcblk0boot1": blob("boot1", BOOT_SECTORS * 512)}
        self.drv = os.path.join(root, "sys/bus/platform/drivers/dwmmc_rockchip")
        os.makedirs(self.drv)
        for name in (EMMC, SD, "ff0d0000.dwmmc"):
            os.makedirs(os.path.join(root, "sys/bus/platform/devices", name))
        # The SDIO controller (Wi-Fi) stays bound; the eMMC and SD card's were switched off at startup.
        self.link_driver("ff0d0000.dwmmc")
        os.makedirs(os.path.join(root, "sys/block"))
        os.makedirs(os.path.join(root, "dev/disk/by-partlabel"))
        os.symlink("../../sda1", os.path.join(root, "dev/disk/by-partlabel/mpfree-test-root"))
        self.probe = os.path.join(root, "sys/bus/platform/drivers_probe")
        os.mkfifo(self.probe)
        os.mkfifo(os.path.join(self.drv, "unbind"))
        self.stop = False
        self.threads = [threading.Thread(target=self.serve, args=(self.probe, self.bind), daemon=True),
                        threading.Thread(target=self.serve, args=(os.path.join(self.drv, "unbind"), self.unbind),
                                         daemon=True)]
        for t in self.threads:
            t.start()

    def link_driver(self, name):
        dev = os.path.join(self.root, "sys/bus/platform/devices", name)
        os.symlink(os.path.relpath(self.drv, dev), os.path.join(dev, "driver"))

    def serve(self, fifo, action):
        with os.fdopen(os.open(fifo, os.O_RDWR), "r") as fh:
            for line in fh:
                if self.stop:
                    return
                for name in line.split():
                    action(name)

    def disk(self, name, files, parts=()):
        """A /sys/block entry, made whole in a side folder and renamed into place, as the script may look
        at any moment. A value ("link", target) is a symbolic link."""
        stage = os.path.join(self.root, "sys/block", "." + name)

        def put(path, value):
            if isinstance(value, tuple):
                os.makedirs(os.path.dirname(path), exist_ok=True)
                os.symlink(value[1], path)
            else:
                write(path, value)
        for rel, value in files.items():
            put(os.path.join(stage, rel), value)
        for pname, pfiles in parts:
            for rel, value in pfiles.items():
                put(os.path.join(stage, pname, rel), value)
        os.rename(stage, os.path.join(self.root, "sys/block", name))

    def bind(self, name):
        self.bound.append(name)
        if name != EMMC:
            return
        if self.no_disk:   # the controller binds, but no card answers
            self.link_driver(EMMC)
            return
        rule = os.path.join(self.root, "run/udev/rules.d/00-mpfree-setup-ro.rules")
        self.rule = read_text(rule) or None
        user_area = "1\n" if self.rule and not self.ignore_rule else "0\n"
        disk_ro = "0\n" if self.writable else ("link", "force_ro")
        part_ro = "0\n" if self.writable else ("link", "../force_ro")
        for disk, data in self.images.items():
            write(os.path.join(self.root, "dev", disk), data)
        for n, pname, start, size in self.layout:
            write(os.path.join(self.root, "dev", f"mmcblk0p{n}"), "")
            os.symlink(f"../../mmcblk0p{n}", os.path.join(self.root, "dev/disk/by-partlabel", pname))
        stat = STAT.format(w=0, ws=0, d=0)
        parts = []
        for n, pname, start, size in self.layout:
            size = DISK_SECTORS - start if size is None else size
            parts.append((f"mmcblk0p{n}", {"uevent": f"MAJOR=179\nMINOR={n}\nDEVNAME=mmcblk0p{n}\nDEVTYPE=partition\n"
                                                     f"PARTN={n}\nPARTNAME={pname}\n",
                                           "start": f"{start}\n", "size": f"{size}\n", "ro": part_ro,
                                           "stat": stat}))
        for boot in ("mmcblk0boot0", "mmcblk0boot1"):
            self.disk(boot, {"ro": disk_ro, "force_ro": "1\n", "size": f"{BOOT_SECTORS}\n", "stat": stat})
        if self.extra_disk:
            self.disk("mmcblk1", {"ro": ("link", "force_ro"), "force_ro": "0\n", "size": "1024\n", "stat": stat})
        self.disk("mmcblk0", {"ro": disk_ro, "force_ro": user_area, "size": f"{DISK_SECTORS}\n", "stat": stat,
                              "device/name": "DG4008\n"}, parts)
        self.link_driver(EMMC)

    def unbind(self, name):
        self.unbound.append(name)
        if self.ignore_unbind:
            return
        os.unlink(os.path.join(self.root, "sys/bus/platform/devices", name, "driver"))
        if name != EMMC:
            return
        for disk in os.listdir(os.path.join(self.root, "sys/block")):
            if disk.startswith("mmcblk"):
                self.force_ro_at_unbind[disk] = read_text(os.path.join(self.root, "sys/block", disk, "force_ro"))
                shutil.rmtree(os.path.join(self.root, "sys/block", disk))
        for dev in os.listdir(os.path.join(self.root, "dev")):
            if dev.startswith("mmcblk"):
                os.unlink(os.path.join(self.root, "dev", dev))
        # udev takes its /dev/disk links away; the script removes any it left (here: none).

    def bump_writes(self):
        write(os.path.join(self.root, "sys/block/mmcblk0/mmcblk0p8/stat"), STAT.format(w=1, ws=8, d=0))

    def close(self):
        self.stop = True
        for fifo in (self.probe, os.path.join(self.drv, "unbind")):
            try:   # no reader (a thread that died) must not hang the test
                fd = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
            except OSError:
                continue
            os.write(fd, b"\n")
            os.close(fd)
        for t in self.threads:
            t.join(5)


RO_RULE = 'SUBSYSTEM=="block", KERNEL=="mmcblk*", ENV{DEVTYPE}=="disk", ACTION=="add", ATTR{force_ro}="1"\n'


class BackupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox, cls.dash = need_busybox()

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-setup-backup-test-")
        self.root = os.path.join(self.tmp, "root")
        self.bin = os.path.join(self.tmp, "bin")
        self.log = os.path.join(self.tmp, "log")
        bb = self.busybox
        fakes = {
            "sleep": f'#!/bin/sh\necho "sleep: $*" >> "$FAKE_LOG"\nexec {bb} usleep 50000\n',
            "sync": '#!/bin/sh\necho "sync" >> "$FAKE_LOG"\n',
            "df": '#!/bin/sh\necho "df: $*" >> "$FAKE_LOG"\necho "Filesystem 1024-blocks Used Available Capacity '
                  'Mounted on"\necho "/dev/sda4 30000000 1000 $FAKE_FREE_KB 1% $2"\n',
            "udevadm": '#!/bin/sh\necho "udevadm: $*" >> "$FAKE_LOG"\n',
            "systemctl": '#!/bin/sh\necho "systemctl: $*" >> "$FAKE_LOG"\n',
            "poweroff": '#!/bin/sh\necho "poweroff: $*" >> "$FAKE_LOG"\n',
            # Owners are not the test's to set; the calls are checked instead.
            "chown": '#!/bin/sh\necho "chown: $*" >> "$FAKE_LOG"\n',
            # dd as the MPC's BusyBox's, every call logged, with vm.dirty_bytes as the copy sees it (and the
            # kernel's answer to it: dirty_ratio reads 0 once dirty_bytes is set); with FAKE_BUMP, something
            # writes to the eMMC during the copy; with FAKE_SLOW it takes 2 s.
            "dd": f'#!/bin/sh\necho "dd: $*" >> "$FAKE_LOG"\n{DD_REFUSES}'
                  f'echo "dirty_bytes: $({bb} cat "$FAKE_ROOT/proc/sys/vm/dirty_bytes")" >> "$FAKE_LOG"\n'
                  f'echo 0 > "$FAKE_ROOT/proc/sys/vm/dirty_ratio"\n'
                  f'[ -n "${{FAKE_BUMP:-}}" ] && echo "{STAT.format(w=1, ws=8, d=0).strip()}" > '
                  f'"$FAKE_ROOT/sys/block/mmcblk0/mmcblk0p8/stat"\n'
                  f'[ -n "${{FAKE_SLOW:-}}" ] && {bb} sleep 2\nexec {bb} dd "$@"\n',
        }
        program_dir(self.bin, bb, needs(BACKUP), fakes, extras=("poweroff",))
        r = self.root
        write(os.path.join(r, "proc/mounts"), "/dev/sda1 / ext4 ro,relatime,noload 0 0\n"
                                              "/dev/sda4 /run/mpfree-data ext4 rw,nosuid,nodev,noexec,relatime 0 0\n")
        write(os.path.join(r, "proc/uptime"), "41.20 80.00\n")
        write(os.path.join(r, "proc/sys/kernel/osrelease"), RELEASE + "\n")
        write(os.path.join(r, "proc/sys/kernel/random/boot_id"), "0123abcd-0000-4000-8000-00000000beef\n")
        write(os.path.join(r, "proc/sys/vm/drop_caches"), "")
        write(os.path.join(r, "proc/sys/vm/dirty_bytes"), "0\n")
        write(os.path.join(r, "proc/sys/vm/dirty_ratio"), "20\n")
        write(os.path.join(r, "dev/kmsg"), "")
        write(os.path.join(r, "run/mpfree-internal-off.txt"),
              "mpfree-internal-off: switched off ff0f0000.dwmmc\nmpfree-internal-off: switched off ff0c0000.dwmmc\n"
              "mpfree-internal-off: done: no eMMC or SD card block device is left\n")
        os.makedirs(os.path.join(r, "run/mpfree-data"))
        self.kernel = None

    def tearDown(self):
        if self.kernel:
            self.kernel.close()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def script(self):
        with open(BACKUP) as fh:
            text = fh.read()
        self.assertTrue(text.startswith("R=''\n"))
        text, n = re.subn(r"LAYOUT='[^']*'", "LAYOUT='" + layout_text(TEST_LAYOUT) + "'", text, count=1)
        self.assertEqual(n, 1)
        text, n = re.subn(r"^NEED_KB=\d+$", "NEED_KB=1000", text, count=1, flags=re.M)
        self.assertEqual(n, 1)
        return text.replace("R=''", f"R='{self.root}'", 1)

    def run_backup(self, free_kb=5000, **env):
        return subprocess.run([self.dash, "-c", self.script(), "setup-backup.sh"],
                              env={"PATH": self.bin, "FAKE_LOG": self.log, "FAKE_ROOT": self.root,
                                   "FAKE_FREE_KB": str(free_kb), **env},
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=60)

    def calls(self, prefix):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as fh:
            return [line[len(prefix):] for line in fh.read().splitlines() if line.startswith(prefix)]

    def status(self):
        with open(os.path.join(self.root, "run/mpfree-setup/status")) as fh:
            return dict(line.split("=", 1) for line in fh.read().splitlines())

    def notes(self):
        with open(os.path.join(self.root, "run/mpfree-setup/backup-log.txt")) as fh:
            return fh.read()

    def backups(self):
        folder = os.path.join(self.root, "run/mpfree-data/backup")
        return sorted(os.listdir(folder)) if os.path.isdir(folder) else []

    def assert_closed(self):
        """The window is shut: the controller unbound, no eMMC block device, no rule, nothing else bound."""
        self.assertEqual(self.kernel.bound, [EMMC], "only the eMMC controller is ever bound, the SD card's never")
        self.assertEqual(self.kernel.unbound, [EMMC])
        self.assertFalse(os.path.lexists(os.path.join(self.root, "sys/bus/platform/devices", EMMC, "driver")))
        self.assertEqual([d for d in os.listdir(os.path.join(self.root, "sys/block")) if d.startswith("mmcblk")], [])
        self.assertFalse(os.path.exists(os.path.join(self.root, "run/udev/rules.d/00-mpfree-setup-ro.rules")))
        links = sorted(os.listdir(os.path.join(self.root, "dev/disk/by-partlabel")))
        self.assertEqual(links, ["mpfree-test-root"], "stale eMMC links are removed, the stick's stays")
        self.assertEqual(self.calls("systemctl: ") + self.calls("poweroff: "), [])

    def assert_only_reads(self):
        """dd only ever reads an eMMC disk, into its image in the backup folder on the stick."""
        backup = re.escape(os.path.join(self.root, "run/mpfree-data/backup/"))
        for call in self.calls("dd: "):
            self.assertRegex(call, rf"^if={re.escape(self.root)}/dev/(mmcblk0(boot[01])?) "
                                   rf"of={backup}mpc-one-emmc-\d{{8}}T\d{{6}}Z\.part/\1\.img bs=1048576$")

    def assert_dirty_bytes_back(self):
        """vm.dirty_bytes was 32 MiB while dd copied, and is as it was again (dirty_ratio written back)."""
        self.assertEqual(set(self.calls("dirty_bytes: ")), {"33554432"})
        with open(os.path.join(self.root, "proc/sys/vm/dirty_ratio")) as fh:
            self.assertEqual(fh.read(), "20\n")

    def request(self):
        path = os.path.join(self.root, "run/mpfree-app/setup-backup.request")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w"):
            pass
        return path

    def test_copies_the_emmc_through_a_read_only_window(self):
        self.kernel = FakeEmmc(self.root)
        request = self.request()
        res = self.run_backup()
        self.assertFalse(os.path.exists(request), "the request is taken away, so the path unit stays quiet")
        self.assertEqual(res.returncode, 0, res.stdout + self.notes())
        self.assert_closed()
        self.assert_only_reads()
        self.assertEqual(len(self.calls("dd: ")), 3)
        self.assertEqual(self.kernel.rule, RO_RULE, "the rule is there before the bind")
        self.assertEqual(self.kernel.force_ro_at_unbind, {d: "1\n" for d in self.kernel.images})
        self.assertEqual(self.calls("udevadm: "), ["control --reload", "control --reload"])
        (name,) = self.backups()
        self.assertRegex(name, r"^mpc-one-emmc-\d{8}T\d{6}Z$")
        folder = os.path.join(self.root, "run/mpfree-data/backup", name)
        self.assertEqual(sorted(os.listdir(folder)), ["MANIFEST.txt", "SHA256SUMS", "mmcblk0.img", "mmcblk0boot0.img",
                                                      "mmcblk0boot1.img"])
        sums = []
        for disk, data in self.kernel.images.items():
            with open(os.path.join(folder, disk + ".img"), "rb") as fh:
                self.assertEqual(fh.read(), data, disk)
            sums.append(f"{hashlib.sha256(data).hexdigest()}  {disk}.img\n")
        with open(os.path.join(folder, "SHA256SUMS")) as fh:
            self.assertEqual(fh.read(), "".join(sums))
        with open(os.path.join(folder, "MANIFEST.txt")) as fh:
            manifest = fh.read()
        self.assertIn(f"eMMC: DG4008, {DISK_SECTORS} sectors\n", manifest)
        self.assertIn(f"kernel: {RELEASE}\n", manifest)
        self.assertIn("  p7 content 1024 512\n", manifest)
        self.assertIn("  mmcblk0p8 0 0 0 0 0 0\n", manifest, "the counters it checked are recorded")
        backup = os.path.join(self.root, "run/mpfree-data/backup")
        self.assertEqual(self.calls("chown: "), [f"0:960 {backup} {folder}.part",
                                                 f"0:960 {folder}.part/MANIFEST.txt {folder}.part/SHA256SUMS "
                                                 f"{folder}.part/mmcblk0.img {folder}.part/mmcblk0boot0.img "
                                                 f"{folder}.part/mmcblk0boot1.img"])
        self.assertEqual(oct(os.stat(folder).st_mode & 0o777), "0o750")
        self.assertEqual(oct(os.stat(os.path.join(folder, "mmcblk0.img")).st_mode & 0o777), "0o640")
        status = self.status()
        total = str((DISK_SECTORS + 2 * BOOT_SECTORS) * 512)
        self.assertEqual(status, {"state": "done", "step": "done", "done": total, "total": total,
                                  "message": "Backup done", "folder": "/run/mpfree-data/backup/" + name})
        notes = self.notes()
        self.assertIn("window open: eMMC DG4008, 2048 sectors, all read-only; layout checked", notes)
        self.assertIn("window closed: no eMMC block device is left", notes)
        with open(os.path.join(self.root, "proc/sys/vm/drop_caches")) as fh:
            self.assertEqual(fh.read(), "3\n", "the read-back comes from the stick, not the page cache")
        with open(os.path.join(self.root, "run/mpfree-setup/verify.txt")) as fh:
            self.assertEqual(fh.read(), "".join(sums), "every image read back from the stick and hashed again")
        self.assert_dirty_bytes_back()
        self.assertRegex(notes, r"copy took \d+ s \(\d+ KB/s\)\n")
        self.assertRegex(notes, r"read back and checked in \d+ s \(\d+ KB/s\)\n")
        # Once per boot: a second request does nothing, and is taken away too.
        request = self.request()
        res = self.run_backup()
        self.assertFalse(os.path.exists(request))
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.kernel.bound, [EMMC])
        self.assertIn("already run this boot", self.notes())
        self.assertEqual(self.backups(), [name])

    def test_a_slow_copy_says_how_it_goes(self):
        """Every 30 status updates the kernel log gets the copy's pace and the block layer's counters."""
        self.kernel = FakeEmmc(self.root)
        write(os.path.join(self.root, "proc/meminfo"), "MemTotal: 2000000 kB\nDirty: 102400 kB\nWriteback: 2048 kB\n")
        write(os.path.join(self.root, "sys/class/block/sda4/stat"),
              "     120        0     4000      30 99 0 409600 0        0       40       30 0 0 0 0 0\n")
        res = self.run_backup(FAKE_SLOW="1")
        self.assertEqual(res.returncode, 0, res.stdout + self.notes())
        self.assertRegex(self.notes(), r"copying mmcblk0: 0 MiB, 0 KB/s; eMMC read 1 MiB, stick written 200 MiB, "
                                       r"waiting in memory 102 MiB\n")

    def test_marks_the_disks_read_only_itself_too(self):
        """Should udev not apply the rule, the script's own force_ro writes make every disk read-only."""
        self.kernel = FakeEmmc(self.root, ignore_rule=True)
        res = self.run_backup()
        self.assertEqual(res.returncode, 0, res.stdout + self.notes())
        self.assert_closed()
        self.assertEqual(self.kernel.force_ro_at_unbind, {d: "1\n" for d in self.kernel.images})
        self.assertEqual(len(self.backups()), 1)

    def test_stops_first_when_a_program_is_missing(self):
        self.kernel = FakeEmmc(self.root)
        for name in ("stat", "sha256sum"):
            os.unlink(os.path.join(self.bin, name))
        res = self.run_backup()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual(self.kernel.bound, [], "nothing is bound")
        self.assertEqual(self.status()["message"], "this system lacks programs the backup needs: sha256sum stat")
        self.assertEqual(self.calls("dd: ") + self.calls("df: "), [])

    def test_checks_before_the_window_opens(self):
        cases = {
            "partition 4 not mounted read-write": (
                lambda: write(os.path.join(self.root, "proc/mounts"), "/dev/sda4 /run/mpfree-data ext4 ro 0 0\n"),
                "the stick's partition 4 is not mounted read-write at /run/mpfree-data"),
            "an eMMC block device already there": (
                lambda: os.makedirs(os.path.join(self.root, "sys/block/mmcblk0")),
                "an eMMC or SD card block device is already there (mmcblk0 )"),
            "the startup switch-off did not finish": (
                lambda: os.unlink(os.path.join(self.root, "run/mpfree-internal-off.txt")),
                "the startup switch-off of the internal storage did not finish"),
            "the controller already bound": (
                lambda: self.kernel.link_driver(EMMC),
                f"the eMMC controller {EMMC} is switched on already"),
            "no room": (None, "partition 4 has 0 MiB free; the backup needs 0 MiB (use a stick of 16 GB or more)"),
        }
        for label, (change, message) in cases.items():
            with self.subTest(label):
                self.tearDown()
                self.setUp()
                self.kernel = FakeEmmc(self.root)
                if change:
                    change()
                res = self.run_backup(free_kb=10 if change is None else 5000)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertEqual(self.kernel.bound, [], "nothing is bound")
                self.assertEqual(self.kernel.unbound, [])
                self.assertEqual(self.status()["state"], "failed")
                self.assertEqual(self.status()["message"], message)
                self.assertEqual(self.backups(), [])
                self.assertEqual(self.calls("dd: "), [])
                self.assertFalse(os.path.exists(os.path.join(self.root, "run/udev/rules.d/00-mpfree-setup-ro.rules")))

    def test_stops_and_closes_the_window_on_any_surprise(self):
        wrong = tuple((n, name, start, 300 if n == 6 else size) for n, name, start, size in TEST_LAYOUT)
        cases = {
            "not read-only": ({"writable": True}, "mmcblk0 is not read-only"),
            "no force_ro": ({"writable": True, "ignore_rule": True}, "mmcblk0 is not read-only"),
            "another layout": ({"layout": wrong}, "this is not the MPC One's eMMC layout: partition 6 (rootfs) has "
                                                  "300 sectors, not 256"),
            "another block device": ({"extra_disk": True}, "an unexpected block device appeared: mmcblk1"),
            "the eMMC does not come up": ({"no_disk": True}, "the eMMC did not appear within 10 s"),
        }
        for label, (opts, message) in cases.items():
            with self.subTest(label):
                self.tearDown()
                self.setUp()
                self.kernel = FakeEmmc(self.root, **opts)
                res = self.run_backup()
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assert_closed()
                status = self.status()
                self.assertEqual((status["state"], status["step"], status["message"]), ("failed", "checking", message))
                self.assertEqual(self.calls("dd: "), [], "nothing is copied")
                self.assertEqual(self.backups(), [])

    def test_writes_during_the_copy_fail_it(self):
        self.kernel = FakeEmmc(self.root)
        res = self.run_backup(FAKE_BUMP="1")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assert_closed()
        self.assert_only_reads()
        status = self.status()
        self.assertEqual((status["state"], status["step"]), ("failed", "copying"))
        self.assertIn("the eMMC's write or discard counters moved during the copy", status["message"])
        self.assertEqual(self.backups(), [], "the failed copy is deleted")

    def test_a_copy_that_does_not_read_back_fails(self):
        """A stick that hands back other bytes than it was given: the backup is deleted, the status says so."""
        self.kernel = FakeEmmc(self.root)
        os.unlink(os.path.join(self.bin, "sha256sum"))
        # Hashing an image on the stick (the read-back, run in the backup folder) gives another answer.
        write(os.path.join(self.bin, "sha256sum"),
              f'#!/bin/sh\ncase "$1" in *.img) echo "{"0" * 64}  $1"; exit 0 ;; esac\n'
              f'exec {self.busybox} sha256sum "$@"\n', 0o755)
        res = self.run_backup()
        self.assertEqual(res.returncode, 1, res.stdout)
        status = self.status()
        self.assertEqual((status["state"], status["step"]), ("failed", "verifying"))
        self.assertEqual(status["message"], "the backup on the stick does not read back as copied: mmcblk0.img "
                                            "mmcblk0boot0.img mmcblk0boot1.img")
        self.assertEqual(self.backups(), [], "the failed copy is deleted")
        self.assert_dirty_bytes_back()

    def test_a_stop_during_the_copy_closes_the_window(self):
        """systemd's TERM (a timeout or a shutdown) mid-copy: the copy and hash stop, the window closes,
        vm.dirty_bytes goes back and the half copy is deleted."""
        self.kernel = FakeEmmc(self.root)
        proc = subprocess.Popen([self.dash, "-c", self.script(), "setup-backup.sh"],
                                env={"PATH": self.bin, "FAKE_LOG": self.log, "FAKE_ROOT": self.root,
                                     "FAKE_FREE_KB": "5000", "FAKE_SLOW": "1"},
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        try:
            for _ in range(200):
                if os.path.exists(os.path.join(self.root, "run/mpfree-setup/status")) and \
                        self.status().get("step") == "copying" and self.calls("dd: "):
                    break
                threading.Event().wait(0.05)
            else:
                self.fail("the copy did not start")
            proc.terminate()
            out, _ = proc.communicate(timeout=30)
        finally:
            if proc.poll() is None:
                proc.kill()
        self.assertEqual(proc.returncode, 1, out)
        self.assert_closed()
        status = self.status()
        self.assertEqual((status["state"], status["step"], status["message"]),
                         ("failed", "copying", "stopped by the system before the backup finished"))
        self.assertEqual(self.backups(), [], "the half copy is deleted")
        with open(os.path.join(self.root, "proc/sys/vm/dirty_ratio")) as fh:
            self.assertEqual(fh.read(), "20\n", "vm.dirty_bytes is as it was")

    def test_powers_off_if_the_controller_does_not_let_go(self):
        self.kernel = FakeEmmc(self.root, ignore_unbind=True)
        res = self.run_backup()
        self.assertEqual(res.returncode, 2, res.stdout)
        self.assertEqual(self.calls("systemctl: "), ["poweroff"])
        self.assertEqual(self.calls("sleep: ")[-10:], ["1"] * 10)
        status = self.status()
        self.assertEqual((status["state"], status["step"]), ("failed", "closing"))
        self.assertIn("could not be switched off again, so the MPC turns off", status["message"])
        self.assertIn("STOP: the eMMC controller did not let go; powering off", self.notes())


class StaticTests(unittest.TestCase):
    def read(self, path):
        with open(path) as fh:
            return fh.read()

    def test_scripts_parse(self):
        dash = shutil.which("dash")
        if not dash:
            self.skipTest("dash not installed")
        for path in (BACKUP, FORMAT, AUTORUN):
            res = subprocess.run([dash, "-n", path], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            self.assertEqual(res.returncode, 0, f"{path}: {res.stdout}")

    def test_the_layout_is_the_mpc_ones(self):
        text = self.read(BACKUP)
        layout = re.search(r"LAYOUT='([^']*)'", text).group(1).splitlines()
        want = []
        for line in self.read(BASELINE).splitlines():
            f = line.split()
            if f and f[0] == "part":
                n = int(f[1].rsplit("p", 1)[1])
                want.append(f"{n} {f[2]} {f[3]} {'-' if f[2] == 'data' else f[4]}")
        self.assertEqual(layout, want)
        self.assertIn("disk  mmcblk0      15273600", self.read(BASELINE))
        # The user area and both boot areas, with room to spare, fit in NEED_KB.
        need = int(re.search(r"^NEED_KB=(\d+)$", text, re.M).group(1))
        self.assertGreater(need * 1024, (15273600 + 2 * 8192) * 512)

    def test_the_backup_window_never_writes_the_emmc(self):
        text = self.read(BACKUP)
        code = "\n".join(line for line in text.splitlines()
                         if not line.lstrip().startswith("#") and not line.startswith("NEEDS="))
        # dd's one output is its image in the backup folder on the stick's partition 4.
        self.assertEqual(re.findall(r"\bof=\S+", code), ['of="$out"'])
        self.assertEqual(re.findall(r"\bout=\S+", code), ['out="$dir.part/$b.img"'])
        self.assertEqual(re.findall(r"\bdir=\S+", code), ["dir=''", 'dir="$R$DATA/backup/mpc-one-emmc-$stamp"'])
        self.assertIn("DATA=/run/mpfree-data\n", code)
        self.assertNotRegex(code, r"(^|[\s;|&(])tee(\s|$)", "Akai's tee moves one byte per system call")
        self.assertNotIn("sha256sum -c", code)
        for word in ("mount", "umount", "fsck", "mkfs", "mke2fs", "blkdiscard", "hdparm", "mmc ", "sfdisk", "sgdisk",
                     "parted", "wipefs", "tune2fs", "resize2fs", "e2fsck", "debugfs", "truncate", "fallocate"):
            self.assertNotRegex(code, rf"(^|[\s;|&(]){re.escape(word.strip())}(\s|$)", word)
        self.assertNotIn("blockdev", code, "Akai's system has none")
        self.assertEqual(re.findall(r'echo (\S+) > "\$R/sys/block/\$b/force_ro"', code), ["1"])
        self.assertEqual(re.findall(r"ATTR\{[^}]*\}=\S*", code), ['ATTR{force_ro}="1"\''])
        self.assertEqual(re.findall(r"\bdd\b[^\n|]*", code), ['dd if="$R/dev/$b" of="$out" bs=1048576 2> "$S/copy-$b.err" &'])
        self.assertEqual(sorted(set(re.findall(r'>\s*"?\$R/proc/[^\s;"]+', code))),
                         ['> "$R/proc/sys/vm/dirty_bytes', '> "$R/proc/sys/vm/dirty_ratio', '> "$R/proc/sys/vm/drop_caches'])
        self.assertEqual(re.findall(r'>\s*"?\$R/dev/[^\s;]+', code), ['> "$R/dev/kmsg"'])
        self.assertEqual(re.findall(r'>\s*"?\$R/sys/[^\s;]+', code), ['> "$R/sys/block/$b/force_ro"',
                                                                     '> "$R/sys/bus/platform/drivers_probe"'])

    def test_units(self):
        service = self.read(os.path.join(STICK, "mpfree-setup-backup.service"))
        self.assertIn("ExecStart=/bin/sh /etc/mpfree-test/setup-backup.sh\n", service)
        self.assertIn("Requires=mpfree-test-internal-off.service\n", service)
        self.assertIn("Type=oneshot\n", service)
        path = self.read(os.path.join(STICK, "mpfree-setup-backup.path"))
        self.assertIn("PathExists=/run/mpfree-app/setup-backup.request\n", path)
        self.assertIn("Unit=mpfree-setup-backup.service\n", path)
        self.assertIn("WantedBy=sysinit.target\n", path)
        fmt = self.read(os.path.join(STICK, "mpfree-setup-format.service"))
        self.assertIn("ExecStart=/bin/sh /etc/mpfree-test/setup-format.sh\n", fmt)
        self.assertIn("Before=mpfree-test-storage.service mpfree-test-app.service\n", fmt)
        self.assertEqual(self.read(os.path.join(STICK, "mode.txt")), "install\n")


class FormatTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox, cls.dash = need_busybox()

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-setup-format-test-")
        self.root = os.path.join(self.tmp, "root")
        self.bin = os.path.join(self.tmp, "bin")
        self.log = os.path.join(self.tmp, "log")
        bb = self.busybox
        fakes = {
            "sleep": '#!/bin/sh\necho "sleep: $*" >> "$FAKE_LOG"\n',
            "sync": '#!/bin/sh\necho "sync" >> "$FAKE_LOG"\n',
            "od": OD,
            "dd": f'#!/bin/sh\n{DD_REFUSES}exec {{bb}} dd "$@"\n',
            # mke2fs: logged, and it leaves an ext2/3/4 magic number where the superblock has it. It "writes"
            # 1 MiB (the partition's block layer counter), then logs the step's report as the startup screen
            # would read it while it runs.
            "mke2fs": f'#!/bin/sh\necho "mke2fs: $*" >> "$FAKE_LOG"\n[ -n "${{FAKE_MKE2FS_FAIL:-}}" ] && exit 1\n'
                      f'stat="$FAKE_ROOT/sys/class/block/sda4/stat"\n'
                      f'[ -e "$stat" ] && echo "{STAT.format(w=2, ws=2048 + 64, d=0).strip()}" > "$stat" && '
                      f'{bb} sleep 1 && {bb} tr "\\n" " " < "$FAKE_ROOT/run/mpfree-setup/format-status" >> "$FAKE_LOG" '
                      f'&& echo >> "$FAKE_LOG"\n'
                      f'for dev; do :; done\n{bb} printf "\\123\\357" | {bb} dd of="$dev" bs=1 seek=1080 '
                      f'conv=notrunc 2>/dev/null\n',
        }
        program_dir(self.bin, bb, needs(FORMAT), fakes)
        r = self.root
        part = os.path.join(r, "sys/devices/platform/ff540000.usb/usb2/2-1/2-1:1.0/host0/target0:0:0/0:0:0:0/"
                               "block/sda/sda4")
        os.makedirs(part)
        os.makedirs(os.path.join(r, "sys/class/block"))
        os.symlink(os.path.relpath(part, os.path.join(r, "sys/class/block")), os.path.join(r, "sys/class/block/sda4"))
        os.makedirs(os.path.join(r, "dev/disk/by-partuuid"))
        os.symlink("../../sda4", os.path.join(r, "dev/disk/by-partuuid", DATA_UUID))
        write(os.path.join(r, "etc/mpfree-test/format-token"), TOKEN + "\n")
        write(os.path.join(r, "proc/mounts"), "/dev/sda1 / ext4 ro 0 0\n")
        write(os.path.join(r, "dev/kmsg"), "")
        os.makedirs(os.path.join(r, "run"))
        self.dev = os.path.join(r, "dev/sda4")
        self.disk(b"MPFREE-FORMAT-ME " + TOKEN.encode())

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def disk(self, start, magic=False):
        data = bytearray(blob("old stick", 64 << 10))
        data[:len(start)] = start
        if magic:
            data[1080:1082] = b"\x53\xef"
        with open(self.dev, "wb") as fh:
            fh.write(data)
        self.before = bytes(data)

    def run_format(self, **env):
        with open(FORMAT) as fh:
            text = fh.read()
        self.assertEqual(text.count("@DATA_UUID@"), 1)
        text = text.replace("R=''", f"R='{self.root}'", 1).replace("@DATA_UUID@", DATA_UUID)
        return subprocess.run([self.dash, "-c", text, "setup-format.sh"],
                              env={"PATH": self.bin, "FAKE_LOG": self.log, "FAKE_ROOT": self.root, **env},
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=30)

    def calls(self, prefix):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as fh:
            return [line[len(prefix):] for line in fh.read().splitlines() if line.startswith(prefix)]

    def notes(self):
        return read_text(os.path.join(self.root, "run/mpfree-setup-format.txt"))

    def after(self):
        with open(self.dev, "rb") as fh:
            return fh.read()

    def report(self):
        """The step's report, as the startup screen reads it."""
        with open(os.path.join(self.root, "run/mpfree-setup/format-status")) as fh:
            return dict(line.split("=", 1) for line in fh.read().splitlines())

    def test_makes_the_filesystem_once_and_clears_the_mark(self):
        res = self.run_format()
        self.assertEqual(res.returncode, 0, res.stdout + self.notes())
        self.assertEqual(self.calls("mke2fs: "),
                         [f"-q -F -t ext4 -m 0 -J size=64 -L MPFREE-DATA -E root_owner=0:0 {self.dev}"])
        self.assertEqual(self.report(), {"state": "done", "done": str(64 << 20), "total": str(64 << 20)})
        data = self.after()
        self.assertEqual(data[:512], bytes(512), "the mark is gone")
        self.assertEqual(data[1080:1082], b"\x53\xef")
        self.assertIn("partition 4 is ready", self.notes())
        # The next boot finds no mark and does nothing.
        res = self.run_format()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(len(self.calls("mke2fs: ")), 1)
        self.assertEqual(self.after(), data)
        self.assertEqual(self.report()["state"], "none", "the startup screen ends at once")

    def test_reports_the_bytes_written_while_it_works(self):
        part = os.path.realpath(os.path.join(self.root, "sys/class/block/sda4"))
        write(os.path.join(part, "stat"), STAT.format(w=2, ws=64, d=0))
        write(os.path.join(part, "size"), "8192000\n")   # a 4 GB partition
        res = self.run_format()
        self.assertEqual(res.returncode, 0, res.stdout + self.notes())
        total = (64 << 20) + 8192000 * 512 // 8192
        (running,) = [line for line in read_text(self.log).splitlines() if line.startswith("state=")]
        self.assertEqual(running, f"state=running done=1048576 total={total} ",
                         "1 MiB written since mke2fs started, of the journal and 0.01 % of the partition")
        self.assertEqual(self.report(), {"state": "done", "done": str(total), "total": str(total)})

    def test_a_filesystem_already_there_is_kept(self):
        self.disk(b"MPFREE-FORMAT-ME " + TOKEN.encode(), magic=True)
        res = self.run_format()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.calls("mke2fs: "), [])
        self.assertEqual(self.after(), bytes(512) + self.before[512:], "only the mark is cleared")
        self.assertIn("holds a filesystem already; only its format mark is cleared", self.notes())

    def test_leaves_partition_4_alone_unless_everything_matches(self):
        def no_mark():
            self.disk(b"\0" * 64)

        def other_token():
            self.disk(b"MPFREE-FORMAT-ME " + b"f" * 32)

        def mounted():
            write(os.path.join(self.root, "proc/mounts"), f"{self.dev} /run/mpfree-data ext4 rw 0 0\n")

        def not_usb():
            link = os.path.join(self.root, "sys/class/block/sda4")
            os.unlink(link)
            os.makedirs(os.path.join(self.root, "sys/devices/platform/fe320000.mmc/block/sda/sda4"))
            os.symlink("../../devices/platform/fe320000.mmc/block/sda/sda4", link)

        def emmc():
            link = os.path.join(self.root, "dev/disk/by-partuuid", DATA_UUID)
            os.unlink(link)
            os.symlink("../../mmcblk0p8", link)
            shutil.copy(self.dev, os.path.join(self.root, "dev/mmcblk0p8"))

        def no_token():
            os.unlink(os.path.join(self.root, "etc/mpfree-test/format-token"))

        def no_partition():
            os.unlink(os.path.join(self.root, "dev/disk/by-partuuid", DATA_UUID))

        cases = {no_mark: "", other_token: "", mounted: "is mounted; not touched", not_usb: "is not on USB",
                 emmc: "partition 4 is mmcblk0p8, not partition 4 of a USB disk", no_token: "no format token",
                 no_partition: "partition 4 not found"}
        for change, note in cases.items():
            with self.subTest(change.__name__):
                self.tearDown()
                self.setUp()
                change()
                before = self.after()
                res = self.run_format()
                self.assertEqual(res.returncode, 0, res.stdout)
                self.assertEqual(self.calls("mke2fs: "), [])
                self.assertEqual(self.after(), before, "partition 4 is untouched")
                self.assertIn(note, self.notes())
                self.assertEqual(self.report()["state"], "none")
                if change is no_partition:
                    self.assertEqual(self.calls("sleep: "), ["1"] * 30)

    def test_a_failed_mke2fs_keeps_the_mark(self):
        res = self.run_format(FAKE_MKE2FS_FAIL="1")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual(self.after()[:49], b"MPFREE-FORMAT-ME " + TOKEN.encode(), "the next boot tries again")
        self.assertIn("mke2fs failed", self.notes())
        self.assertEqual(self.report()["state"], "failed")

    def test_touches_nothing_when_a_program_is_missing(self):
        os.unlink(os.path.join(self.bin, "mke2fs"))
        res = self.run_format()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertEqual(self.after(), self.before, "partition 4 is untouched")
        self.assertEqual(self.notes(), "this system lacks programs this needs: mke2fs; partition 4 not touched\n")
        self.assertEqual(self.report()["state"], "failed")


AUTORUN_SYSTEMCTL = FAKE_SYSTEMCTL.replace('  *) echo "systemctl stand-in', '  reboot) ;;\n  *) echo "systemctl stand-in')


class AutorunTests(HookFixture):
    """autorun.sh as Akai's script runner runs it, from the stick's MPFREE partition, on the fake MPC of
    test_boot_hook.py, with the real hook-install.sh and hook-remove.sh next to it."""

    def setUp(self):
        super().setUp()
        write(os.path.join(self.remote, "systemctl"), AUTORUN_SYSTEMCTL, 0o755)
        write(os.path.join(self.remote, "id"), '#!/bin/sh\n[ "$1" = -u ] && echo "${FAKE_UID:-0}"\n', 0o755)
        self.drive = os.path.join(self.tmp, "media/MPFREE")
        with open(AUTORUN) as fh:
            text = fh.read()
        self.assertEqual(text.count("@RELEASE@"), 1)
        self.assertEqual(text.count("\nR=''\n"), 1)
        write(os.path.join(self.drive, "autorun.sh"),
              text.replace("\nR=''\n", f"\nR='{self.root}'\n", 1).replace("@RELEASE@", RELEASE))
        for name in ("hook-install.sh", "hook-remove.sh"):
            shutil.copy(os.path.join(TOOLS, name), os.path.join(self.drive, name))
        shutil.copy(os.path.join(STICK, "mode.txt"), os.path.join(self.drive, "mode.txt"))

    def autorun(self, rc, **env):
        res = subprocess.run([self.dash, os.path.join(self.drive, "autorun.sh")],
                             env={"PATH": self.remote, "FAKE_LOG": self.log, "FAKE_ROOT": self.root,
                                  "FAKE_STICK": self.stick, **env},
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=120)
        out = self.setup_log()
        self.assertEqual(res.returncode, rc, res.stdout + out)
        self.assertEqual(res.stdout, "", "everything goes to setup-log.txt")
        return out

    def setup_log(self):
        return read_text(os.path.join(self.drive, "setup-log.txt"))

    def test_installs_the_hook_and_restarts_then_removes_it(self):
        self.plug_in()
        out = self.autorun(0)
        self.assertIn("board: MPC One (inmusic,acva)", out)
        self.assertIn("CHECK-OK: at power-on with this stick in, the MPC would soft-reboot into it", out)
        self.assertIn("RESULT: MPFree's startup hook is installed. Restarting the MPC in 5 s; leave the stick in.", out)
        self.assertTrue(self.read(SCRIPT).startswith("R=''\n"))
        self.assertEqual(self.calls("systemctl: reboot"), ["systemctl: reboot"])
        self.assertEqual(self.calls("sleep: 5"), ["sleep: 5"])
        # The undo: mode.txt says remove.
        write(os.path.join(self.drive, "mode.txt"), "remove\n")
        os.unlink(os.path.join(self.drive, "setup-log.txt"))
        out = self.autorun(0)
        self.assertIn("RESULT: the hook is gone", out)
        self.assertIn("RESULT: MPFree's startup hook is removed. Restarting the MPC in 5 s.", out)
        self.assertEqual(self.tree(), self.before)
        self.assertEqual(len(self.calls("systemctl: reboot")), 2)
        # After the restart the MPC may ask again: a second Yes does nothing and does not restart.
        self.assertEqual(read_text(os.path.join(self.drive, "mode.txt")), "removed\n")
        os.unlink(os.path.join(self.drive, "setup-log.txt"))
        out = self.autorun(0)
        self.assertIn("DONE: nothing to do. MPFree's startup hook was already removed with this stick", out)
        self.assertEqual(self.tree(), self.before)
        self.assertEqual(len(self.calls("systemctl: reboot")), 2)

    def test_stops_without_changing_anything(self):
        def other_board():
            write(os.path.join(self.root, "proc/device-tree/compatible"), "inmusic,acvb\0inmusic,az01\0rockchip,rk3288\0")

        def other_release():
            write(os.path.join(self.root, "proc/sys/kernel/osrelease"), "6.18.44-az01-2026-09-30-rt6\n")

        cases = {
            "another MPC": (other_board, {}, "STOP: this is not an MPC One (inmusic,acva)."),
            "another release": (other_release, {}, f"STOP: this stick was made for kernel {RELEASE} (Akai 3.10.1); "
                                                   f"the MPC runs 6.18.44-az01-2026-09-30-rt6."),
            "no stick": (None, {}, "STOP: MPFree's stick is not in (no partition named mpfree-test-root)."),
            "a firmware update running": (self.plug_in, {"FAKE_PIDOF_HIT": "az01-update"},
                                          "STOP: az01-update is running (a firmware update?); try again later."),
            "not root": (self.plug_in, {"FAKE_UID": "1000"}, "STOP: this must run as root"),
            "no mode": (lambda: write(os.path.join(self.drive, "mode.txt"), "maybe?\n"), {},
                        "STOP: mode.txt must say install or remove."),
        }
        for label, (change, env, message) in cases.items():
            with self.subTest(label):
                self.tearDown()
                self.setUp()
                if change:
                    change()
                out = self.autorun(1, **env)
                self.assertIn(message, out)
                self.assertEqual(self.tree(), self.before, "nothing on the MPC changed")
                self.assertEqual(self.calls("systemctl: reboot"), [])


if __name__ == "__main__":
    unittest.main()
