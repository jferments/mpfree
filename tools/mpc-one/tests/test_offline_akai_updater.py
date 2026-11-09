#!/usr/bin/env python3
"""Tests for offline-akai-updater.sh against a fake eMMC backup. No device.

Builds a small ext4 rootfs (no journal, like p6 on the MPC) holding a fake
/opt/ramfs-update.squashfs, /usr/bin/az01-update, a U-Boot boot script and
some systemd units, places it at p6's real start sector in a fake
mmcblk0.img, and runs the real script on it. Set
MPFREE_REQUIRE_OFFLINE_TOOLS=1 (CI does) to fail instead of skip when
e2fsprogs or squashfs-tools are missing.
"""
import hashlib
import os
import random
import shutil
import stat
import struct
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(TOOLS))
SCRIPT = os.path.join(TOOLS, "offline-akai-updater.sh")
REAL_BASELINE = os.path.join(TOOLS, "baselines", "mpc-one-3.6.0-reference.txt")
NEEDED = ("mkfs.ext4", "debugfs", "mksquashfs", "unsquashfs", "file", "strings")
SBIN_PATH = os.environ.get("PATH", "") + ":/usr/sbin:/sbin"

SECTOR = 512
P6_START = 17408  # the real start sector, so the copy needs a non-MiB-aligned skip
P6_SECTORS = 16384  # 8 MiB instead of 500 MiB
TAIL_SECTORS = 2048

INIT_SCRIPT = "#!/bin/sh\n# FAKE-UPDATER-INIT\ndd if=/tmp/rootfs.img of=/dev/mmcblk0p6 bs=1M\n"
AZ01_UPDATE = "#!/bin/sh\n# FAKE-AZ01-UPDATE uses /opt/ramfs-update.squashfs\n"
BOOT_SCRIPT = "setenv bootargs FAKE-BOOT-SCRIPT\nload mmc 0:6 ${kernel_addr_r} /boot/zImage\n"
UNITS = {
    "az01-update.service": "[Service]\nExecStart=/usr/bin/az01-update FAKE-UNIT-AZ01\n",
    "touch-fw-update.service": "[Service]\nExecStart=/usr/bin/touch-fw-upd FAKE-UNIT-TOUCH\n",
    "acvs.service": "[Service]\nExecStart=/usr/bin/MPC FAKE-UNIT-ACVS\n",
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


def tools_present():
    missing = [t for t in NEEDED if shutil.which(t, path=SBIN_PATH) is None]
    if missing and os.environ.get("MPFREE_REQUIRE_OFFLINE_TOOLS") == "1":
        raise RuntimeError("missing tools: " + " ".join(missing))
    return not missing


def run(cmd, **kw):
    env = dict(os.environ, PATH=SBIN_PATH)
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, env=env, **kw)


def uboot_script_image(text):
    body = text.encode()
    payload = struct.pack(">II", len(body), 0) + body
    header = struct.pack(">IIIIIIIBBBB32s", 0x27051956, 0, 0, len(payload), 0, 0, 0,
                         5, 2, 6, 0, b"boot script")
    return header + payload


def build_rootfs(base, with_updater=True):
    """Return (p6 bytes, squashfs sha256) for a fake rootfs."""
    sq_src = os.path.join(base, "squash-src")
    write(os.path.join(sq_src, "init"), INIT_SCRIPT, 0o755)
    with open(shutil.which("true"), "rb") as fh:
        elf = fh.read() + b"\0FAKE-ELF-MARKER /dev/mmcblk0p3\0"
    write(os.path.join(sq_src, "bin", "flasher"), elf, 0o755)
    squash = os.path.join(base, "ramfs-update.squashfs")
    run(["mksquashfs", sq_src, squash, "-noappend", "-no-progress", "-quiet",
         "-p", "dev d 755 0 0", "-p", "dev/console c 600 0 0 5 1"])
    root = os.path.join(base, "rootfs-src")
    if with_updater:
        os.makedirs(os.path.join(root, "opt"))
        shutil.copy(squash, os.path.join(root, "opt", "ramfs-update.squashfs"))
    # An absolute symlink, as many distributions install their updaters.
    write(os.path.join(root, "usr", "libexec", "az01", "updater"), AZ01_UPDATE, 0o755)
    os.makedirs(os.path.join(root, "usr", "bin"))
    os.symlink("/usr/libexec/az01/updater", os.path.join(root, "usr", "bin", "az01-update"))
    write(os.path.join(root, "boot", "boot.scr.uimg"), uboot_script_image(BOOT_SCRIPT))
    units = os.path.join(root, "usr", "lib", "systemd", "system")
    for name, text in UNITS.items():
        write(os.path.join(units, name), text)
    os.symlink("az01-update.service", os.path.join(units, "fw-alias.service"))
    long_target = "../system/" + "x" * 70 + "-update.service"  # too long for a fast symlink
    os.symlink(long_target, os.path.join(units, "slow-link-update.service"))
    write(os.path.join(units, "x" * 70 + "-update.service"), "[Service]\nExecStart=/bin/true FAKE-SLOW-TARGET\n")
    write(os.path.join(units, "empty-fw.service"), "")
    os.symlink("loop-update.service", os.path.join(units, "loop-update.service"))
    image = os.path.join(base, "p6.img")
    with open(image, "wb") as fh:
        fh.truncate(P6_SECTORS * SECTOR)
    run(["mkfs.ext4", "-q", "-F", "-O", "^has_journal", "-d", root, image])
    with open(image, "rb") as fh:
        p6 = fh.read()
    with open(squash, "rb") as fh:
        squash_sha = sha(fh.read())
    return p6, squash_sha


class Fixture:
    """A fake backup folder, baseline and HOME inside one temporary folder."""

    def __init__(self, base, p6, squash_sha, name="mpc-one-emmc-20261109T233604Z-abcd1234"):
        self.base = base
        self.home = os.path.join(base, "home")
        self.backups = os.path.join(self.home, "mpfree-backups")
        self.backup = os.path.join(self.backups, name)
        os.makedirs(self.backup)
        rng = random.Random(23)
        self.disk = (rng.randbytes(P6_START * SECTOR) + p6 + rng.randbytes(TAIL_SECTORS * SECTOR))
        write(os.path.join(self.backup, "mmcblk0.img"), self.disk, 0o400)
        write(os.path.join(self.backup, "SHA256SUMS"), f"{sha(self.disk)}  mmcblk0.img\n", 0o400)
        self.squash_sha = squash_sha
        self.baseline = os.path.join(base, "baseline.txt")
        with open(REAL_BASELINE) as fh:
            lines = fh.read().splitlines()
        out = []
        for line in lines:
            if line.split()[:2] == ["part", "mmcblk0p6"]:
                line = f"part  mmcblk0p6    rootfs         {P6_START}    {P6_SECTORS}  {sha(p6)}"
            out.append(line)
        write(self.baseline, "\n".join(out) + "\n")

    def run(self, *extra, env_extra=None, baseline=True, updater_sha=True):
        args = ["bash", SCRIPT]
        if baseline:
            args += ["--baseline", self.baseline]
        if updater_sha:
            args += ["--updater-sha256", self.squash_sha]
        args += list(extra)
        env = {k: v for k, v in os.environ.items() if k not in ("MPC_BACKUP_DIR", "MPFREE_WORK_DIR")}
        env.update(HOME=self.home, LC_ALL="C")
        env.update(env_extra or {})
        return subprocess.run(args, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                              text=True, timeout=300)

    def snapshot(self, folder=None):
        folder = folder or self.backups
        state = {}
        for dirpath, _, files in os.walk(folder):
            for name in files:
                path = os.path.join(dirpath, name)
                st = os.lstat(path)
                with open(path, "rb") as fh:
                    state[path] = (sha(fh.read()), stat.S_IMODE(st.st_mode), st.st_mtime_ns)
        return state

    def work_folders(self, root=None):
        root = root or os.path.join(self.home, "mpfree-work")
        if not os.path.isdir(root):
            return []
        return sorted(os.path.join(root, d) for d in os.listdir(root))


@unittest.skipUnless(tools_present(), "needs e2fsprogs, squashfs-tools, file and binutils")
class OfflineAkaiUpdaterTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="mpfree-offline-test-")
        os.chmod(cls.tmp, 0o755)
        cls.p6, cls.squash_sha = build_rootfs(os.path.join(cls.tmp, "build"))

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def fixture(self, p6=None, squash_sha=None):
        base = tempfile.mkdtemp(dir=self.tmp)
        os.chmod(base, 0o755)
        return Fixture(base, p6 or self.p6, squash_sha or self.squash_sha)

    def test_report_from_default_backup(self):
        fx = self.fixture()
        before = fx.snapshot()
        res = fx.run()
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertEqual(out.count("matches the boot map"), 2, out)
        self.assertIn(f"backup: {fx.backup}", out)
        self.assertIn(INIT_SCRIPT, out)
        self.assertIn("FAKE-ELF-MARKER /dev/mmcblk0p3", out)
        self.assertIn("squashfs-root/dev/console", out)
        self.assertIn(AZ01_UPDATE, out)
        self.assertIn("FAKE-BOOT-SCRIPT", out)
        self.assertIn("FAKE-UNIT-AZ01", out)
        self.assertIn("FAKE-UNIT-TOUCH", out)
        self.assertIn("/usr/bin/az01-update -> /usr/libexec/az01/updater\ncopied /usr/libexec/az01/updater", out)
        self.assertIn("/usr/lib/systemd/system/fw-alias.service -> az01-update.service\n"
                      "copied /usr/lib/systemd/system/az01-update.service", out)
        self.assertIn("slow-link-update.service -> ../system/xxx", out)
        self.assertIn("FAKE-SLOW-TARGET", out)
        self.assertIn("copied /usr/lib/systemd/system/empty-fw.service", out)
        self.assertIn("rootfs:empty-fw.service: empty", out)
        self.assertIn("/usr/lib/systemd/system/loop-update.service: symlink, not copied", out)
        self.assertNotIn("FAKE-UNIT-ACVS", out)
        self.assertIn("never commit or share them", out)
        self.assertEqual(fx.snapshot(), before, "the backup must not change")
        [work] = fx.work_folders()
        self.assertIn(f"work folder: {work}", out)
        with open(os.path.join(work, "report.txt")) as fh:
            self.assertIn("DONE. Report:", fh.read())
        self.assertFalse(os.path.exists(os.path.join(work, "p6-rootfs.img")))
        self.assertTrue(os.path.isfile(os.path.join(work, "ramfs-update", "init")))
        self.assertEqual(stat.S_IMODE(os.stat(work).st_mode), 0o700)

    def test_picks_newest_verified_backup(self):
        fx = self.fixture()
        older = os.path.join(fx.backups, "mpc-one-emmc-20261108T120000Z-zzzz9999")
        shutil.copytree(fx.backup, older)
        failed = os.path.join(fx.backups, "mpc-one-emmc-20261110T000000Z-aaaa0000")
        write(os.path.join(failed, "MANIFEST-FAILED.txt"), "failed\n")
        res = fx.run()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"backup: {fx.backup}\n", res.stdout)

    def test_explicit_backup_and_work_dir(self):
        fx = self.fixture()
        work_root = os.path.join(fx.base, "elsewhere")
        res = fx.run("--backup", fx.backup, env_extra={"MPFREE_WORK_DIR": work_root})
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(len(fx.work_folders(work_root)), 1)
        self.assertEqual(fx.work_folders(), [])

    def test_rootfs_hash_mismatch_stops(self):
        fx = self.fixture()
        os.chmod(os.path.join(fx.backup, "mmcblk0.img"), 0o600)
        with open(os.path.join(fx.backup, "mmcblk0.img"), "r+b") as fh:
            fh.seek(P6_START * SECTOR + 4096 + 7)
            byte = fh.read(1)
            fh.seek(-1, os.SEEK_CUR)
            fh.write(bytes([byte[0] ^ 0xFF]))
        res = fx.run()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("does not match the boot-map hash", res.stdout)
        [work] = fx.work_folders()
        self.assertFalse(os.path.exists(os.path.join(work, "ramfs-update.squashfs")))

    def test_updater_hash_mismatch_stops(self):
        fx = self.fixture()
        res = fx.run("--updater-sha256", "0" * 64)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("ramfs-update.squashfs does not match the boot-map hash", res.stdout)
        [work] = fx.work_folders()
        self.assertFalse(os.path.exists(os.path.join(work, "ramfs-update")))

    def test_default_updater_hash_is_the_boot_map_one(self):
        fx = self.fixture()
        res = fx.run(updater_sha=False)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("(39db4a08bc988d8cb8ce24fb40e29ed6289c1484753e07c52c842906ad24fadb)", res.stdout)

    def test_missing_updater_stops(self):
        p6, _ = build_rootfs(tempfile.mkdtemp(dir=self.tmp), with_updater=False)
        fx = self.fixture(p6=p6)
        res = fx.run()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("/opt/ramfs-update.squashfs not found in the rootfs", res.stdout)

    def test_unreadable_rootfs_stops(self):
        junk = random.Random(5).randbytes(P6_SECTORS * SECTOR)
        fx = self.fixture(p6=junk)
        res = fx.run()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("debugfs could not read the rootfs copy", res.stdout)

    def test_real_baseline_refuses_the_fake_disk(self):
        fx = self.fixture()
        res = fx.run(baseline=False)
        self.assertNotEqual(res.returncode, 0, res.stdout)
        self.assertNotIn("matches the boot map", res.stdout)

    def test_refuses_work_dir_inside_backup(self):
        fx = self.fixture()
        before = fx.snapshot()
        res = fx.run(env_extra={"MPFREE_WORK_DIR": os.path.join(fx.backup, "work")})
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("must not be inside the backup", res.stdout)
        self.assertFalse(os.path.exists(os.path.join(fx.backup, "work")))
        self.assertEqual(fx.snapshot(), before)

    def test_refuses_work_dir_inside_clone(self):
        fx = self.fixture()
        target = os.path.join(REPO, "tools", "mpc-one", "offline-test-not-ignored")
        res = fx.run(env_extra={"MPFREE_WORK_DIR": target})
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("inside the git clone and not git-ignored", res.stdout)
        self.assertFalse(os.path.exists(target))

    def test_refuses_work_dir_with_spaces(self):
        fx = self.fixture()
        target = os.path.join(fx.base, "work dir")
        res = fx.run(env_extra={"MPFREE_WORK_DIR": target})
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("must not contain spaces", res.stdout)
        self.assertFalse(os.path.exists(target))

    def test_no_verified_backup(self):
        fx = self.fixture()
        res = fx.run(env_extra={"MPC_BACKUP_DIR": os.path.join(fx.base, "empty")})
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("no verified backup", res.stdout)


class StaticTests(unittest.TestCase):
    def test_script_never_contacts_the_mpc_or_writes_images(self):
        with open(SCRIPT) as fh:
            lines = [l for l in fh.read().splitlines() if not l.lstrip().startswith("#")]
        # The strings filter for binaries is a list of words to look for, not commands.
        lines = [l for l in lines if "grep -E -i" not in l]
        code = "\n".join(lines)
        for forbidden in ("ssh", "scp", "curl", "wget", "debugfs -w", "mount ", "losetup",
                          "sudo", "mkfs", "fsck", "tune2fs"):
            self.assertNotIn(forbidden, code)
        # The only dd writes the work-folder copy of p6.
        dd_lines = [l for l in lines if l.lstrip().startswith("dd ")]
        self.assertEqual(len(dd_lines), 1)
        self.assertIn('of="$ROOTFS"', dd_lines[0])


if __name__ == "__main__":
    unittest.main(verbosity=2)
