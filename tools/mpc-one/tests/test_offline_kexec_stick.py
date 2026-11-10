#!/usr/bin/env python3
"""Tests for offline-kexec-stick.py against a fake rootfs, fake boot files and a fake running tree. No device.

Needs e2fsprogs (debugfs, e2fsck, mke2fs), sfdisk and ssh-keygen. Set
MPFREE_REQUIRE_OFFLINE_TOOLS=1 (CI does) to fail instead of skip without them.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_offline_dtb import board_tree, dtb, s as dt_strings  # noqa: E402

TOOLS = os.path.dirname(HERE)
TOOL = os.path.join(TOOLS, "offline-kexec-stick.py")
NEW, OLD = "9.9-fake-new", "8.8-fake-old"
SERIAL = "FAKE-SERIAL-4455"
MAC = bytes.fromhex("020000ddeeff")
ZIMAGE = b"FAKE-ZIMAGE-NEW" * 1000
SBIN_PATH = "/usr/sbin:/usr/bin:/sbin:/bin"
UNITS = "usr/lib/systemd/system"


def tools_present():
    return all(shutil.which(t, path=SBIN_PATH) for t in ("debugfs", "e2fsck", "mke2fs", "sfdisk", "ssh-keygen"))


def write(path, data, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as fh:
        fh.write(data)
    os.chmod(path, mode)


def sha(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def run(cmd, **kw):
    return subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                          env={**os.environ, "PATH": SBIN_PATH}, **kw)


def new_board(variant=False):
    tree = board_tree()
    tree.find("/chosen").set_prop("inmusic,acvs-emmc-content", b"")
    tree.find("/dwmmc@ff0f0000").set_prop("new-in-9.9", dt_strings("yes"))
    if variant:
        tree.root.set_prop("compatible", dt_strings("inmusic,acva", "inmusic,az01", "rockchip,rk3288-c",
                                                    "rockchip,rk3288"))
    return dtb.build(tree)


def old_board(variant=False):
    tree = board_tree()
    tree.find("/chosen").set_prop("inmusic,acvs-emmc-content", b"")
    if variant:
        tree.root.set_prop("compatible", dt_strings("inmusic,acva", "inmusic,az01", "rockchip,rk3288-c",
                                                    "rockchip,rk3288"))
    return dtb.build(tree)


def live_tree():
    tree = board_tree(serial=SERIAL, mac=MAC)
    tree.find("/chosen").set_prop("inmusic,acvs-emmc-content", b"")
    tree.find("/chosen").set_prop("bootargs", dt_strings("root=PARTUUID=FAKE-EMMC-UUID rootwait ro"))
    return dtb.build(tree)


def build_rootfs(base, shadow_root="!", extra=None, merged=True):
    root = os.path.join(base, "root")
    shutil.rmtree(root, ignore_errors=True)
    files = {
        "boot/zImage-" + NEW: ZIMAGE,
        "boot/rk3288-az01-acva.dtb": new_board(),
        "boot/rk3288-az01-acva-c.dtb": new_board(variant=True),
        f"{UNITS}/data.mount": "[Mount]\nWhat=PARTUUID=FAKE-DATA\nWhere=/data\n",
        f"{UNITS}/acvs.service": "[Service]\nExecStart=/usr/bin/az01-launch-MPC\n",
        "etc/passwd": "root:x:0:0:root:/home/root:/bin/sh\n",
        "etc/shadow": f"root:{shadow_root}:19000:0:99999:7:::\ndaemon:*:19000:0:99999:7:::\n",
        "usr/sbin/sshd": "#!/bin/sh\n# FAKE-SSHD\n",
        "usr/bin/sh": "#!/fake/shell\n",
        "usr/sbin/az01-update": "#!/bin/sh\n# FAKE-UPDATER\n",
        "usr/bin/dfu-util": "#!/bin/sh\n# FAKE-DFU\n",
        "usr/bin/fw_printenv": "#!/bin/sh\n# FAKE-FW-PRINTENV\n",
        "usr/bin/az01-launch-MPC": "#!/bin/sh\n# FAKE-LAUNCHER\n",
    }
    files.update(extra or {})
    for path, data in files.items():
        write(os.path.join(root, path), data, 0o755 if path.startswith("usr/") and "bin/" in path else 0o644)
    os.chmod(os.path.join(root, "etc/shadow"), 0o400)
    os.symlink("fw_printenv", os.path.join(root, "usr/bin/fw_setenv"))
    os.makedirs(os.path.join(root, "etc/systemd/system/multi-user.target.wants"))
    if merged:
        for top in ("bin", "sbin", "lib"):
            os.makedirs(os.path.join(root, "usr", top), exist_ok=True)
            os.symlink("usr/" + top, os.path.join(root, top))
    else:
        os.makedirs(os.path.join(root, "sbin"))
    image = os.path.join(base, "rootfs.img")
    with open(image, "wb") as fh:
        fh.truncate(24 << 20)
    res = run(["mke2fs", "-q", "-F", "-t", "ext4", "-O", "^has_journal", "-E", "root_owner=0:0", "-d", root, image])
    assert res.returncode == 0, res.stdout
    return image


@unittest.skipUnless(tools_present() or os.environ.get("MPFREE_REQUIRE_OFFLINE_TOOLS") == "1",
                     "needs e2fsprogs, sfdisk and ssh-keygen")
class KexecStickTests(unittest.TestCase):
    def setUp(self):
        if not tools_present():
            self.fail("debugfs, e2fsck, mke2fs, sfdisk or ssh-keygen is missing")
        self.tmp = tempfile.mkdtemp(prefix="mpfree-kexec-stick-test-")
        self.home = os.path.join(self.tmp, "home")
        self.work = os.path.join(self.home, "mpfree-work")
        unpacked = os.path.join(self.work, "akai-unpacked-20260101T000000Z-abc")
        os.makedirs(unpacked)
        self.image = build_rootfs(unpacked)
        self.image_sha = sha(self.image)
        for release, files in ((NEW, {"zImage-" + NEW: ZIMAGE, "rk3288-az01-acva.dtb": new_board(),
                                      "rk3288-az01-acva-c.dtb": new_board(variant=True)}),
                               (OLD, {"zImage-" + OLD: b"OLD", "rk3288-az01-acva.dtb": old_board(),
                                      "rk3288-az01-acva-c.dtb": old_board(variant=True)})):
            folder = os.path.join(self.work, "boot-files", release)
            for name, data in files.items():
                write(os.path.join(folder, name), data)
            with open(os.path.join(folder, "SHA256SUMS"), "w") as fh:
                for name in sorted(files):
                    fh.write(f"{sha(os.path.join(folder, name))}  {name}\n")
        self.pre = os.path.join(self.work, "kexec-preflight-20260101T000000Z-xyz")
        write(os.path.join(self.pre, "live.dtb"), live_tree())
        write(os.path.join(self.pre, "release.txt"), OLD + "\n")
        self.key = os.path.join(self.tmp, "mpc_key")
        made = run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "fake-user-key", "-f", self.key])
        self.assertEqual(made.returncode, 0, made.stdout)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def build(self, **extra):
        env = {"HOME": self.home, "LC_ALL": "C", "PATH": SBIN_PATH, "MPC_KEY": self.key, **extra}
        return subprocess.run([sys.executable, "-I", TOOL], env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=300)

    def outputs(self):
        return sorted(d for d in os.listdir(self.work) if d.startswith("kexec-stick-"))

    def partition(self, stick, number, dest):
        table = json.loads(run(["sfdisk", "--json", stick]).stdout)["partitiontable"]["partitions"]
        part = table[number - 1]
        with open(stick, "rb") as fin, open(dest, "wb") as fout:
            fin.seek(part["start"] * 512)
            fout.write(fin.read(part["size"] * 512))
        return part

    def dfs(self, image, command):
        out = run(["debugfs", "-R", command, image]).stdout
        return "".join(line for line in out.splitlines(True) if not line.startswith("debugfs "))

    def test_builds_the_stick_and_leaves_the_source_alone(self):
        res = self.build()
        self.assertEqual(res.returncode, 0, res.stdout)
        [folder] = self.outputs()
        out = os.path.join(self.work, folder)
        self.assertEqual(sorted(os.listdir(out)), ["SHA256SUMS", "cmdline.txt", "kexec.dtb", "known_hosts",
                                                   "report.txt", "stick.img", "zImage"])
        self.assertEqual(os.stat(out).st_mode & 0o777, 0o700)
        self.assertEqual(sha(self.image), self.image_sha, "the source rootfs must never change")
        with open(os.path.join(out, "SHA256SUMS")) as fh:
            for line in fh:
                digest, name = line.split()
                self.assertEqual(sha(os.path.join(out, name)), digest, name)
        with open(os.path.join(out, "zImage"), "rb") as fh:
            self.assertEqual(fh.read(), ZIMAGE)
        self.assertIn(f"running release: {OLD} (recorded by the preflight)", res.stdout)
        self.assertIn("boot-files/" + NEW + "/rk3288-az01-acva.dtb", res.stdout)
        self.assertNotIn("acva-c.dtb", res.stdout, "the -c board file must not be picked")
        self.assertNotIn(SERIAL, res.stdout)
        self.assertNotIn("FAKE-EMMC-UUID", res.stdout)

        tree = dtb.load(os.path.join(out, "kexec.dtb"))
        self.assertEqual(tree.find("/dwmmc@ff0f0000").prop("status"), dt_strings("disabled"))
        self.assertEqual(tree.find("/dwmmc@ff0c0000").prop("status"), dt_strings("disabled"))
        self.assertEqual(tree.find("/dwmmc@ff0f0000").prop("new-in-9.9"), dt_strings("yes"))
        self.assertEqual(tree.root.prop("serial-number"), dt_strings(SERIAL))
        self.assertEqual(tree.find("/ethernet@ff290000").prop("local-mac-address"), MAC)
        self.assertIsNone(tree.find("/chosen").prop("inmusic,acvs-emmc-content"))
        self.assertIsNone(tree.find("/chosen").prop("bootargs"))

        stick = os.path.join(out, "stick.img")
        root_img, log_img = os.path.join(self.tmp, "p1.img"), os.path.join(self.tmp, "p2.img")
        p1, p2 = self.partition(stick, 1, root_img), self.partition(stick, 2, log_img)
        self.assertEqual((p1["start"], p1["name"], p2["name"]), (2048, "mpfree-test-root", "mpfree-test-log"))
        with open(os.path.join(out, "cmdline.txt")) as fh:
            cmdline = fh.read()
        self.assertEqual(cmdline, f"root=PARTUUID={p1['uuid'].lower()} rootwait=60 ro panic=10 "
                                  "rfkill.default_state=0 sysctl.kernel.kexec_load_disabled=1\n")
        self.assertEqual(run(["e2fsck", "-fn", root_img]).returncode, 0)
        self.assertEqual(run(["e2fsck", "-fn", log_img]).returncode, 0)
        self.assertIn(p2["uuid"].lower(), self.dfs(root_img, "cat /etc/mpfree-test/save-logs.sh"))

        for unit in ("acvs.service", "touch-fw-update.service", "az0x-data-mkfs.service", "iptables.service"):
            self.assertIn('Fast link dest: "/dev/null"', self.dfs(root_img, f"stat /etc/systemd/system/{unit}"))
        self.assertIn('Fast link dest: "/dev/null"',
                      self.dfs(root_img, "stat /etc/systemd/system-generators/acva-emmc-content-generator"))
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-sshd.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/multi-user.target.wants/mpfree-test-sshd.service"))
        self.assertIn("What=tmpfs", self.dfs(root_img, "cat /etc/systemd/system/data.mount"))
        self.assertIn("kernel.kexec_load_disabled = 1", self.dfs(root_img, "cat /etc/sysctl.d/90-mpfree-test.conf"))
        for prog in ("/usr/sbin/az01-update", "/usr/bin/dfu-util", "/usr/bin/fw_setenv", "/usr/bin/az01-launch-MPC"):
            self.assertIn("mpfree-test: blocked", self.dfs(root_img, f"cat {prog}"), prog)
            self.assertRegex(self.dfs(root_img, f"stat {prog}"), r"Type: regular\s+Mode:\s+0755")
        self.assertIn("FAKE-FW-PRINTENV", self.dfs(root_img, "cat /usr/bin/fw_printenv"), "read-only twin stays")
        self.assertIn("FAKE-SSHD", self.dfs(root_img, "cat /usr/sbin/sshd"))
        self.assertIn("absent   /usr/sbin/az01config (nothing to stub)", res.stdout)
        shadow = self.dfs(root_img, "cat /etc/shadow")
        self.assertIn("root:*:19000:", shadow)
        self.assertIn("daemon:*:19000:", shadow)
        self.assertRegex(self.dfs(root_img, "stat /etc/shadow"), r"Mode:\s+0400")
        self.assertIn('root account was marked locked ("!")', res.stdout)
        with open(self.key + ".pub") as fh:
            self.assertEqual(self.dfs(root_img, "cat /etc/mpfree-test/authorized_keys"), fh.read())
        stat = self.dfs(root_img, "stat /etc/mpfree-test/host_ed25519")
        self.assertRegex(stat, r"Mode:\s+0600")
        self.assertRegex(stat, r"User:\s+0\s+Group:\s+0")
        host_key = os.path.join(self.tmp, "host_key")
        run(["debugfs", "-R", f"dump /etc/mpfree-test/host_ed25519 {host_key}", root_img])
        os.chmod(host_key, 0o600)
        derived = run(["ssh-keygen", "-y", "-f", host_key]).stdout.split()[:2]
        with open(os.path.join(out, "known_hosts")) as fh:
            self.assertEqual(fh.read().split(), ["mpfree-kexec-test"] + derived)
        with open(os.path.join(out, "report.txt")) as fh:
            self.assertNotIn(SERIAL, fh.read())

    def test_unlocked_root_is_left_alone(self):
        os.unlink(self.image)
        build_rootfs(os.path.dirname(self.image), shadow_root="$6$FAKEHASH")
        res = self.build()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("root account is not marked locked; /etc/shadow unchanged", res.stdout)
        self.assertNotIn("FAKEHASH", res.stdout)

    def expect_stop(self, message, **env):
        before = self.outputs()
        res = self.build(**env)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn(message, res.stdout)
        for folder in set(self.outputs()) - set(before):
            self.assertEqual(os.listdir(os.path.join(self.work, folder)), ["report.txt"], "only the report stays")
        self.assertEqual(sha(self.image), self.image_sha)
        return res

    def test_stops(self):
        with self.subTest("an existing file where a mask goes"):
            os.unlink(self.image)
            build_rootfs(os.path.dirname(self.image), extra={"etc/systemd/system/acvs.service": "[Service]\n"})
            self.image_sha = sha(self.image)
            self.expect_stop("/etc/systemd/system/acvs.service already exists in the image")
        with self.subTest("/sbin is a real folder"):
            os.unlink(self.image)
            build_rootfs(os.path.dirname(self.image), merged=False)
            self.image_sha = sha(self.image)
            self.expect_stop("/sbin is not a link into /usr (directory)")
        os.unlink(self.image)
        build_rootfs(os.path.dirname(self.image))
        self.image_sha = sha(self.image)
        with self.subTest("boot files from another build"):
            folder = os.path.join(self.work, "boot-files", NEW)
            write(os.path.join(folder, "zImage-" + NEW), b"OTHER BUILD")
            with open(os.path.join(folder, "SHA256SUMS")) as fh:
                sums = fh.read()
            with open(os.path.join(folder, "SHA256SUMS"), "w") as fh:
                new_sum = sha(os.path.join(folder, "zImage-" + NEW))
                fh.write(re.sub(r"^\w+(  zImage-)", new_sum + r"\1", sums, flags=re.M))
            self.expect_stop("boot-files belong to another build")
            write(os.path.join(folder, "zImage-" + NEW), ZIMAGE)
            self.expect_stop("does not match")
        with self.subTest("no preflight"):
            shutil.rmtree(self.pre)
            self.expect_stop("run collect-over-ssh.sh --kexec-preflight first")
        with self.subTest("work folder inside the clone"):
            res = self.build(MPFREE_WORK_DIR=os.path.join(os.path.dirname(TOOLS), "not-ignored-work"))
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn("inside the git clone and not git-ignored", res.stdout)
            self.assertFalse(os.path.exists(os.path.join(os.path.dirname(TOOLS), "not-ignored-work")))

    def test_running_release_without_release_txt(self):
        os.unlink(os.path.join(self.pre, "release.txt"))
        res = self.build()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"running release: {OLD} (the only other boot-files release", res.stdout)
        os.makedirs(os.path.join(self.work, "boot-files", "7.7-third"))
        self.expect_stop("cannot tell which release the MPC runs")


class StaticTests(unittest.TestCase):
    def test_never_contacts_the_mpc_or_writes_devices(self):
        with open(TOOL) as fh:
            code = "\n".join(line for line in fh.read().splitlines() if not line.lstrip().startswith("#"))
        for forbidden in ('"ssh"', '"scp"', "'ssh'", "/dev/sd", "/dev/mmc", "of=/dev", "kexec -l", "sudo"):
            self.assertNotIn(forbidden, code)
        self.assertIn('"debugfs", "-w", "-f", script, img.path', code, "-w only on the copy")
        self.assertEqual(code.count('"-w"'), 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
