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
from test_offline_dtb import board_tree, dtb, s as dt_strings  # noqa: E402
from test_offline_kernel_config import zimage  # noqa: E402

SCRIPT = os.path.join(os.path.dirname(HERE), "offline-rootfs-survey.sh")
UNITS = "usr/lib/systemd/system"
SME0_BLOB = bytes(range(256)) * 8
SIG_MARKER = b"\0FAKE-SIG verify /dev/mmcblk0p3 with key FAKE-KEY\0harmless-unmatched-text\0"
ILITEK = b"\x00" * 3000
LONG = "x" * 70 + "-update.service"  # too long for a fast symlink
ZIMAGE = zimage("gzip")


def sha_file(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def acva2_tree():
    tree = board_tree(model="Akai Professional MPC Live II")
    tree.root.set_prop("compatible", dt_strings("inmusic,acva2", "rockchip,rk3288"))
    return tree


def build_image(base, name="rootfs.img"):
    root = os.path.join(base, "root-" + name)
    files = {
        "etc/os-release": "NAME=az0x\nVERSION_ID=FAKE-OS-9.9\n",
        "etc/fstab": "/dev/mmcblk0p8 /data ext4 defaults 0 2\n",
        f"{UNITS}/content.mount": "[Mount]\nWhat=/dev/mmcblk0p7\nWhere=/content\n",
        f"{UNITS}/xmos-update.service": "[Service]\nExecStart=/usr/bin/xmosdfu --write FAKE-XMOS\n",
        f"{UNITS}/touch-fw-update.service": "[Service]\nExecStart=/usr/bin/touch-fw-upd FAKE-TOUCH\n",
        f"{UNITS}/acvs.service": "[Service]\nExecStart=/usr/bin/MPC FAKE-APP-UNIT\n",
        f"{UNITS}/getty-other.service": "[Service]\nExecStart=/bin/true FAKE-OTHER-UNIT\n",
        f"{UNITS}/{LONG}": "[Service]\nExecStart=/bin/true FAKE-LONG\n",
        "usr/bin/touch-fw-upd": "#!/bin/sh\n",
        "usr/lib/firmware/top.bin": b"\x01\x02",
        "usr/lib/firmware/ilitek/fw.bin": ILITEK,
        "boot/zImage-9.9": ZIMAGE,
        "boot/rk3288-az01-acva.dtb": dtb.build(board_tree()),
        "boot/rk3288-az01-acva-c.dtb": dtb.build(board_tree(model="Akai Professional MPC One (c)")),
        "boot/rk3288-az05-acva2.dtb": dtb.build(acva2_tree()),
        "usr/lib/modules/9.9/modules.builtin": "kernel/drivers/usb/storage/usb-storage.ko\n"
                                               "kernel/fs/ext4/ext4.ko\nkernel/drivers/gpu/fake.ko\n",
        "usr/lib/modules/9.9/modules.dep": "kernel/drivers/usb/storage/uas.ko: kernel/drivers/scsi/sd_mod.ko\n"
                                           "kernel/drivers/scsi/sd_mod.ko:\nkernel/sound/fake-snd.ko:\n",
        "opt/note.sh": "dd of=/dev/mmcblk0 erase-data-partition\n",
        "usr/lib/modules/9.9-fake/modules.dep": "",
        # systemd escapes "-" in unit names as \x2d; debugfs can't take such a name as a path.
        f"{UNITS}/media-acvs\\x2dcontent.mount": "[Mount]\nWhat=PARTUUID=FAKE-CONTENT\nWhere=/media/acvs-content\n",
        "usr/bin/az01-launch-MPC": "#!/bin/sh\n# FAKE-LAUNCHER\ndfu-util -D /usr/share/Akai/SME0/Firmware/fw.bin\nexec MPC\n",
        "usr/lib/systemd/system-preset/98-az01-touch-fw-updater.preset": "enable touch-fw-update.service\n",
        "usr/share/Akai/SME0/Firmware/update.sh": "#!/bin/sh\ndfu-util -a 0 -D fw.bin FAKE-SME0\n",
        "usr/share/Akai/SME0/Firmware/fw.bin": SME0_BLOB,
        "usr/share/Akai/SME0/Docs/readme.txt": "not a firmware folder\n",
    }
    for path, data in files.items():
        write(os.path.join(root, path), data)
    with open(shutil.which("true"), "rb") as fh:
        elf = fh.read()
    write(os.path.join(root, "usr/bin/xmosdfu"), elf + b"\0fastboot flash uboot-spl\0", 0o755)
    write(os.path.join(root, "usr/bin/az01-bootloader-sig"), elf + SIG_MARKER, 0o755)
    os.symlink("xmos-update.service", os.path.join(root, UNITS, "alias-update.service"))
    os.symlink("../system/" + LONG, os.path.join(root, UNITS, "slow-update.service"))
    os.symlink("zImage-9.9", os.path.join(root, "boot/zImage"))
    for merged in ("lib", "bin"):  # merged /usr, as on the MPC
        os.symlink("usr/" + merged, os.path.join(root, merged))
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
        self.assertRegex(out, r"--- kernel module folders in /usr/lib/modules\n(dir 9\.9(-fake)?\n){2}")
        self.assertIn("--- /etc/fstab\n/dev/mmcblk0p8 /data ext4", out)
        self.assertIn("--- /usr/lib/systemd/system/content.mount\n[Mount]\nWhat=/dev/mmcblk0p7", out)
        self.assertIn("link alias-update.service", out)
        self.assertIn("ExecStart=/usr/bin/xmosdfu --write FAKE-XMOS", out)
        self.assertIn("ExecStart=/usr/bin/touch-fw-upd FAKE-TOUCH", out)
        self.assertIn("--- /usr/lib/systemd/system/acvs.service\n[Service]\nExecStart=/usr/bin/MPC FAKE-APP-UNIT", out)
        self.assertIn("file getty-other.service", out)
        self.assertNotIn("FAKE-OTHER-UNIT", out, "units that don't match the pattern are listed, not printed")
        self.assertIn("--- /usr/lib/systemd/system/alias-update.service -> xmos-update.service", out)
        self.assertIn(f"--- /usr/lib/systemd/system/slow-update.service -> ../system/{LONG}", out)
        self.assertIn("/etc/systemd/system/multi-user.target.wants: xmos-update.service", out)
        self.assertIn("/usr/bin/xmosdfu (file)", out)
        self.assertIn("/usr/bin/touch-fw-upd (file)", out)
        self.assertIn(f"--- /usr/lib/firmware\ndir ilitek\n    file fw.bin sha256 {hashlib.sha256(ILITEK).hexdigest()}\n"
                      f"file top.bin sha256 {hashlib.sha256(bytes([1, 2])).hexdigest()}", out)
        self.assertIn(f"{hashlib.sha256(ZIMAGE).hexdigest()} zImage-9.9", out)
        self.assertIn("--- /usr/lib/systemd/system/media-acvs\\x2dcontent.mount\n[Mount]\nWhat=PARTUUID=FAKE-CONTENT", out)
        self.assertNotIn("unusual name", out)
        self.assertNotIn("all entries in /lib/systemd/system", out, "/lib is a link to /usr/lib: list it once")
        self.assertEqual(out.count("--- /usr/bin/az01-launch-MPC\n"), 1)
        self.assertNotIn("--- /bin/az01-launch-MPC", out)
        self.assertIn("=== 7. Akai's own programs", out)
        self.assertIn("--- /usr/bin/az01-launch-MPC\n#!/bin/sh\n# FAKE-LAUNCHER", out)
        self.assertIn("--- /usr/bin/az01-bootloader-sig: ELF", out)
        self.assertIn("    FAKE-SIG verify /dev/mmcblk0p3 with key FAKE-KEY", out)
        self.assertNotIn("harmless-unmatched-text", out)
        self.assertIn("--- /usr/lib/systemd/system-preset/98-az01-touch-fw-updater.preset\nenable touch-fw-update", out)
        self.assertIn("dfu-util -a 0 -D fw.bin FAKE-SME0", out)
        self.assertRegex(out, r"--- /usr/share/Akai/SME0/Firmware/fw.bin: .*\(2048 bytes\), sha256 "
                              + hashlib.sha256(SME0_BLOB).hexdigest())
        self.assertNotIn("not a firmware folder", out)
        self.assertIn("erase-data-partition: /opt/note.sh", out)
        self.assertIn("fastboot: /usr/bin/xmosdfu", out)
        self.assertIn("uboot-spl: /usr/bin/xmosdfu", out)
        self.assertIn("/dev/mmcblk0: /etc/fstab", out)
        self.assertIn("/dev/mmcblk0: /usr/lib/systemd/system/content.mount", out)
        self.assertIn("xmosdfu: /usr/bin (a file name in this folder)", out)
        self.assertIn("ramfs-update: no match", out)
        self.check_kernel_section(out)
        self.assertIn("DONE. Report:", out)
        [folder] = self.folders()
        with open(os.path.join(self.work, folder, "report.txt")) as fh:
            self.assertIn("fastboot: /usr/bin/xmosdfu", fh.read())
        self.assertEqual(os.listdir(os.path.join(self.work, folder)), ["report.txt"], "no temporary files left")
        self.assertEqual((sha_file(self.image), os.stat(self.image).st_mtime_ns), before, "image must not change")

    def check_kernel_section(self, out):
        section = out[out.index("=== 8. Kernel and device trees"):]
        copy = os.path.join(self.work, "boot-files", "9.9")
        self.assertIn("kernel release: 9.9\n", section)
        self.assertIn(f"boot files copied to {os.path.realpath(copy)}", section)
        self.assertIn("    kernel/drivers/usb/storage/usb-storage.ko\n    kernel/fs/ext4/ext4.ko\n"
                      "    (3 entries in all)", section)
        self.assertIn("    kernel/drivers/usb/storage/uas.ko\n    kernel/drivers/scsi/sd_mod.ko\n"
                      "    (3 entries in all)", section)
        self.assertNotIn("fake.ko", section)
        self.assertIn("(config found: gzip payload", section)
        self.assertIn("CONFIG_USB_STORAGE=y", section)
        self.assertIn("rk3288-az05-acva2.dtb: Akai Professional MPC Live II; inmusic,acva2, rockchip,rk3288", section)
        self.assertIn("--- device trees for this board (inmusic,acva): 2", section)
        self.assertIn("storage controller /dwmmc@ff0f0000: rockchip,rk3288-dw-mshc; status okay; aliases: mmc0",
                      section)
        self.assertIn('property /model: differs (A 26 bytes, B 30 bytes): A "Akai Professional MPC One" | '
                      'B "Akai Professional MPC One (c)"', section)
        self.assertEqual(sorted(os.listdir(copy)), ["SHA256SUMS", "rk3288-az01-acva-c.dtb", "rk3288-az01-acva.dtb",
                                                    "rk3288-az05-acva2.dtb", "zImage-9.9"])
        with open(os.path.join(copy, "zImage-9.9"), "rb") as fh:
            self.assertEqual(fh.read(), ZIMAGE)

    def test_boot_files_are_never_overwritten(self):
        res = self.survey("--image", self.image)
        self.assertEqual(res.returncode, 0, res.stdout)
        copy = os.path.join(self.work, "boot-files", "9.9")
        res = self.survey("--image", self.image)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"boot files: {os.path.realpath(copy)} (already there, same hashes)", res.stdout)
        tampered = os.path.join(copy, "zImage-9.9")
        with open(tampered, "ab") as fh:
            fh.write(b"changed")
        with open(os.path.join(copy, "SHA256SUMS"), "a") as fh:
            fh.write("changed\n")
        before = sha_file(tampered)
        res = self.survey("--image", self.image)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("exists with different files; left unchanged", res.stdout)
        self.assertIn("(config found: gzip payload", res.stdout, "the checks use the image's own copy")
        self.assertEqual(sha_file(tampered), before)
        for folder in self.folders():
            self.assertEqual(os.listdir(os.path.join(self.work, folder)), ["report.txt"])

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
        for folder in self.folders(work):
            self.assertEqual(os.listdir(os.path.join(work, folder)), ["report.txt"], "the p6 copy must be removed")

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
        for forbidden in ("debugfs -w", "of=/dev", "curl", "wget"):
            self.assertNotIn(forbidden, code)
        # A command word followed by a space or the end of the line; names inside the grep patterns don't count.
        commands = re.compile(r"(^|[;&|(]|\$\()\s*(sudo\s+)?(ssh|scp|u?mount|losetup|kexec|fsck|e2fsck|tune2fs)(?=\s|$)",
                              re.M)
        self.assertIsNone(commands.search(code))
        subprocess.run(["bash", "-n", SCRIPT], check=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
