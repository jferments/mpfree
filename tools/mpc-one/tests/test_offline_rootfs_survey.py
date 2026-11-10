#!/usr/bin/env python3
"""Tests for offline-rootfs-survey.sh against small fake rootfs images. No device.

Set MPFREE_REQUIRE_OFFLINE_TOOLS=1 (CI does) to fail instead of skip when
e2fsprogs, squashfs-tools, file or binutils are missing.
"""
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_offline_akai_updater import (  # noqa: E402
    REPO, SBIN_PATH, Fixture, build_rootfs, run, tools_present, write)

SCRIPT = os.path.join(os.path.dirname(HERE), "offline-rootfs-survey.sh")
UNITS = "usr/lib/systemd/system"
LONG = "x" * 70 + "-update.service"  # too long for a fast symlink


def sha_file(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def build_image(base, name="rootfs.img"):
    root = os.path.join(base, "root-" + name)
    files = {
        "etc/os-release": "NAME=az0x\nVERSION_ID=FAKE-OS-9.9\n",
        "etc/fstab": "/dev/mmcblk0p8 /data ext4 defaults 0 2\n",
        f"{UNITS}/content.mount": "[Mount]\nWhat=/dev/mmcblk0p7\nWhere=/content\n",
        f"{UNITS}/xmos-update.service": "[Service]\nExecStart=/usr/bin/xmosdfu --write FAKE-XMOS\n",
        f"{UNITS}/touch-fw-update.service": "[Service]\nExecStart=/usr/bin/touch-fw-upd FAKE-TOUCH\n",
        f"{UNITS}/acvs.service": "[Service]\nExecStart=/usr/bin/MPC FAKE-APP-UNIT\n",
        f"{UNITS}/{LONG}": "[Service]\nExecStart=/bin/true FAKE-LONG\n",
        "usr/bin/touch-fw-upd": "#!/bin/sh\n",
        "usr/lib/firmware/top.bin": b"\x01\x02",
        "usr/lib/firmware/ilitek/fw.bin": b"\x00" * 3000,
        "boot/zImage-9.9": b"\x00" * 1000,
        "opt/note.sh": "dd of=/dev/mmcblk0 erase-data-partition\n",
        "usr/lib/modules/9.9-fake/modules.dep": "",
    }
    for path, data in files.items():
        write(os.path.join(root, path), data)
    with open(shutil.which("true"), "rb") as fh:
        write(os.path.join(root, "usr/bin/xmosdfu"), fh.read() + b"\0fastboot flash uboot-spl\0", 0o755)
    os.symlink("xmos-update.service", os.path.join(root, UNITS, "alias-update.service"))
    os.symlink("../system/" + LONG, os.path.join(root, UNITS, "slow-update.service"))
    os.symlink("zImage-9.9", os.path.join(root, "boot/zImage"))
    wants = os.path.join(root, "etc/systemd/system/multi-user.target.wants")
    os.makedirs(wants)
    os.symlink("/" + UNITS + "/xmos-update.service", os.path.join(wants, "xmos-update.service"))
    image = os.path.join(base, name)
    with open(image, "wb") as fh:
        fh.truncate(16 << 20)
    run(["mkfs.ext4", "-q", "-F", "-b", "1024", "-O", "^has_journal", "-d", root, image])
    return image


@unittest.skipUnless(tools_present(), "needs e2fsprogs, squashfs-tools, file and binutils")
class OfflineRootfsSurveyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix="mpfree-survey-test-")
        os.chmod(cls.tmp, 0o755)
        cls.image = build_image(cls.tmp)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def setUp(self):
        self.base = tempfile.mkdtemp(dir=self.tmp)
        self.home = os.path.join(self.base, "home")
        self.work = os.path.join(self.home, "mpfree-work")
        os.makedirs(self.home)

    def survey(self, *args, work=None, home=None):
        env = {k: v for k, v in os.environ.items() if k not in ("MPC_BACKUP_DIR", "MPFREE_WORK_DIR")}
        env.update(HOME=home or self.home, LC_ALL="C", PATH=SBIN_PATH)
        if work:
            env["MPFREE_WORK_DIR"] = work
        return subprocess.run(["bash", SCRIPT, *args], env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=300)

    def folders(self, root=None):
        root = root or self.work
        return sorted(d for d in os.listdir(root) if d.startswith("rootfs-survey-")) if os.path.isdir(root) else []

    def test_report_on_an_image(self):
        before = (sha_file(self.image), os.stat(self.image).st_mtime_ns)
        res = self.survey("--image", self.image)
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertIn("VERSION_ID=FAKE-OS-9.9", out)
        self.assertIn("(/etc/issue: not in the image)", out)
        self.assertIn("zImage-9.9", out)
        self.assertIn("--- kernel module folders in /usr/lib/modules\ndir 9.9-fake", out)
        self.assertIn("--- /etc/fstab\n/dev/mmcblk0p8 /data ext4", out)
        self.assertIn("--- /usr/lib/systemd/system/content.mount\n[Mount]\nWhat=/dev/mmcblk0p7", out)
        self.assertIn("link alias-update.service", out)
        self.assertIn("ExecStart=/usr/bin/xmosdfu --write FAKE-XMOS", out)
        self.assertIn("ExecStart=/usr/bin/touch-fw-upd FAKE-TOUCH", out)
        self.assertNotIn("FAKE-APP-UNIT", out, "units that don't match the pattern are listed, not printed")
        self.assertIn("--- /usr/lib/systemd/system/alias-update.service -> xmos-update.service", out)
        self.assertIn(f"--- /usr/lib/systemd/system/slow-update.service -> ../system/{LONG}", out)
        self.assertIn("/etc/systemd/system/multi-user.target.wants: xmos-update.service", out)
        self.assertIn("/usr/bin/xmosdfu (file)", out)
        self.assertIn("/usr/bin/touch-fw-upd (file)", out)
        self.assertIn("--- /usr/lib/firmware\ndir ilitek\n    file fw.bin\nfile top.bin", out)
        self.assertIn("erase-data-partition: /opt/note.sh", out)
        self.assertIn("fastboot: /usr/bin/xmosdfu", out)
        self.assertIn("uboot-spl: /usr/bin/xmosdfu", out)
        self.assertIn("/dev/mmcblk0: /etc/fstab", out)
        self.assertIn("/dev/mmcblk0: /usr/lib/systemd/system/content.mount", out)
        self.assertIn("xmosdfu: /usr/bin (a file name in this folder)", out)
        self.assertIn("ramfs-update: no match", out)
        self.assertIn("DONE. Report:", out)
        [folder] = self.folders()
        with open(os.path.join(self.work, folder, "report.txt")) as fh:
            self.assertIn("fastboot: /usr/bin/xmosdfu", fh.read())
        self.assertEqual(os.listdir(os.path.join(self.work, folder)), ["report.txt"], "no temporary files left")
        self.assertEqual((sha_file(self.image), os.stat(self.image).st_mtime_ns), before, "image must not change")

    def test_default_is_the_newest_unpacked_update(self):
        older = os.path.join(self.work, "akai-unpacked-20261101T000000Z-aaaa")
        newer = os.path.join(self.work, "akai-unpacked-20261110T000000Z-bbbb")
        # Folders that sort after the newest unpack must not be picked: the owner's download
        # folder, and a hand-made one without the extractor's timestamp.
        others = [os.path.join(self.work, "akai-update-files"), os.path.join(self.work, "akai-unpacked-notes")]
        for folder in (older, newer, *others):
            os.makedirs(folder)
        for folder in others:
            shutil.copy(self.image, os.path.join(folder, "rootfs.img"))
        shutil.copy(self.image, os.path.join(newer, "rootfs.img"))
        res = self.survey()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"source: {os.path.realpath(newer)}/rootfs.img", res.stdout)
        shutil.copy(self.image, os.path.join(newer, "images_rootfs.img"))
        res = self.survey()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("expected one *rootfs*.img", res.stdout)
        for folder in (newer, older, *others):
            shutil.rmtree(folder)
        res = self.survey()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("no unpacked update under", res.stdout)

    def test_from_backup_compares_with_the_stock_rootfs(self):
        p6, squash_sha = build_rootfs(os.path.join(self.base, "build"))
        fx = Fixture(os.path.join(self.base, "fx"), p6, squash_sha)
        before = fx.snapshot()
        res = self.survey("--from-backup", "--baseline", fx.baseline, home=fx.home)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"source: p6 of {fx.backup}", res.stdout)
        self.assertIn("p6 matches the boot map", res.stdout)
        self.assertIn("ramfs-update: /usr/libexec/az01/start-update.sh", res.stdout)
        self.assertIn("ExecStart=/usr/bin/touch-fw-upd FAKE-UNIT-TOUCH", res.stdout)
        work = os.path.join(fx.home, "mpfree-work")
        [folder] = self.folders(work)
        self.assertEqual(os.listdir(os.path.join(work, folder)), ["report.txt"], "the p6 copy must be removed")
        self.assertEqual(fx.snapshot(), before, "the backup must not change")
        with open(fx.baseline) as fh:
            text = fh.read()
        with open(fx.baseline, "w") as fh:
            fh.write(text.replace(hashlib.sha256(p6).hexdigest(), "0" * 64))
        res = self.survey("--from-backup", "--baseline", fx.baseline, home=fx.home)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("does not match the boot-map hash; stopping", res.stdout)

    def test_guards(self):
        inside = os.path.join(REPO, "tools", "mpc-one", "survey-test-not-ignored")
        res = self.survey("--image", self.image, work=inside)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("inside the git clone and not git-ignored", res.stdout)
        self.assertFalse(os.path.exists(inside))
        spaced = os.path.join(self.base, "my image.img")
        shutil.copy(self.image, spaced)
        res = self.survey("--image", spaced)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("must not contain spaces or quotes", res.stdout)
        res = self.survey("--image", os.path.join(self.base, "missing.img"))
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("cannot read", res.stdout)
        junk = os.path.join(self.base, "junk.img")
        write(junk, b"\x00" * 65536)
        res = self.survey("--image", junk)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("debugfs could not read", res.stdout)
        res = self.survey("--bogus")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("unknown argument: --bogus", res.stdout)


class StaticTests(unittest.TestCase):
    def test_script_never_contacts_the_mpc_or_writes_images(self):
        with open(SCRIPT) as fh:
            code = fh.read()
        for forbidden in ("ssh", "scp", "debugfs -w", "of=/dev", "curl", "wget"):
            self.assertNotIn(forbidden, code)
        commands = re.compile(r"(^|[;&|(]|\$\()\s*(sudo\s+)?(u?mount|losetup|kexec|fsck|e2fsck|tune2fs)\b", re.M)
        self.assertIsNone(commands.search(code))
        subprocess.run(["bash", "-n", SCRIPT], check=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
