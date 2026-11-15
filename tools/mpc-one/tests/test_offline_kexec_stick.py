#!/usr/bin/env python3
"""Tests for offline-kexec-stick.py against a fake rootfs, fake boot files and a fake running tree. No device.

Needs e2fsprogs (debugfs, e2fsck, mke2fs), sfdisk and ssh-keygen. Set
MPFREE_REQUIRE_OFFLINE_TOOLS=1 (CI does) to fail instead of skip without them.
"""
import hashlib
import importlib.util
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_offline_dtb import board_tree, cells, dtb, node, s as dt_strings  # noqa: E402

TOOLS = os.path.dirname(HERE)
TOOL = os.path.join(TOOLS, "offline-kexec-stick.py")
NEW, OLD = "9.9-fake-new", "8.8-fake-old"
SERIAL = "FAKE-SERIAL-4455"
MAC = bytes.fromhex("020000ddeeff")
ZIMAGE = b"FAKE-ZIMAGE-NEW" * 1000
SBIN_PATH = "/usr/sbin:/usr/bin:/sbin:/bin"
UNITS = "usr/lib/systemd/system"


def stick_module():
    spec = importlib.util.spec_from_file_location("offline_kexec_stick", TOOL)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


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


def fake_elf(machine=40, flags=0x05000400, etype=2, interp=False):
    """A stand-in for the screen log program: only its ELF header and program headers are real."""
    phnum = 2 if interp else 1
    head = b"\x7fELF\x01\x01\x01" + b"\x00" * 9 + struct.pack(
        "<HHIIIIIHHHHHH", etype, machine, 1, 0x10000, 52, 0, flags, 52, 32, phnum, 40, 0, 0)
    ph = struct.pack("<8I", 1, 0, 0x10000, 0x10000, 4096, 4096, 5, 0x1000)
    if interp:
        ph += struct.pack("<8I", 3, 0, 0, 0, 20, 20, 4, 1)
    return head + ph + b"FAKE-SCREEN-LOG" * 100


def new_board(variant=False, phy=True):
    tree = board_tree()
    new_parts(tree, phy)
    if variant:
        tree.root.set_prop("compatible", dt_strings("inmusic,acva", "inmusic,az01", "rockchip,rk3288-c",
                                                    "rockchip,rk3288"))
    return dtb.build(tree)


def new_parts(tree, phy=True):
    """What the new release's board file has that the old one lacks."""
    if phy:
        gmac = tree.find("/ethernet@ff290000")
        gmac.set_prop("phy-handle", cells(0x4B))
        gmac.children.append(node("mdio", [], [node("ethernet-phy@1", [
            ("compatible", dt_strings("ethernet-phy-ieee802.3-c22")), ("reg", cells(1)),
            ("reset-gpios", cells(0x18, 8, 1)), ("phandle", cells(0x4B))])]))
        tree.root.children.append(node("mipi@ff960000", [], [node("panel@0", [("rotation", cells(270))])]))
    tree.find("/chosen").set_prop("inmusic,acvs-emmc-content", b"")
    tree.find("/dwmmc@ff0f0000").set_prop("new-in-9.9", dt_strings("yes"))


def old_board(variant=False):
    tree = board_tree()
    tree.find("/chosen").set_prop("inmusic,acvs-emmc-content", b"")
    if variant:
        tree.root.set_prop("compatible", dt_strings("inmusic,acva", "inmusic,az01", "rockchip,rk3288-c",
                                                    "rockchip,rk3288"))
    return dtb.build(tree)


def live_tree(new=False):
    """The tree U-Boot hands the running kernel: the old release's board file, or the new one's (new=True)."""
    tree = board_tree(serial=SERIAL, mac=MAC)
    if new:
        new_parts(tree)
    tree.find("/chosen").set_prop("inmusic,acvs-emmc-content", b"")
    tree.find("/chosen").set_prop("bootargs", dt_strings("root=PARTUUID=FAKE-EMMC-UUID rootwait ro"))
    return dtb.build(tree)


def live_tree_new(emmc="okay"):
    """The tree U-Boot gives an installed NEW release: its board file plus what U-Boot fills in."""
    tree = dtb.parse(new_board())
    tree.root.set_prop("serial-number", dt_strings(SERIAL))
    tree.find("/ethernet@ff290000").set_prop("local-mac-address", MAC)
    tree.find("/memory@0").set_prop("reg", cells(0, 0x80000000))
    tree.find("/chosen").set_prop("bootargs", dt_strings("root=PARTUUID=FAKE-EMMC-UUID rootwait ro"))
    tree.find("/dwmmc@ff0f0000").set_prop("status", dt_strings(emmc))
    return dtb.build(tree)


UDEV_RULES = ('# ENV{SYSTEMD_WANTS}+="commented-out.service"\n'
              'SUBSYSTEM=="block", ENV{ID_FS_TYPE}=="?*", ENV{SYSTEMD_WANTS}+="no-reserved-blocks@%k.service"\n'
              'SUBSYSTEM=="net", KERNEL=="eth0", ENV{SYSTEMD_WANTS}+="fake-net.service"\n'
              'SUBSYSTEM=="block", PROGRAM="/usr/bin/fake-namer", ENV{SYSTEMD_WANTS}+="%c"\n'
              'KERNEL=="sd*", ENV{SYSTEMD_WANTS}="systemd-fsck@dev-sda1.service acvs.service"\n')


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
        f"{UNITS}/iptables.service": "[Service]\nType=oneshot\nExecStart=/usr/sbin/iptables-restore "
                                     "/etc/iptables/iptables.rules\n",
        "etc/iptables/iptables.rules": "*filter\n:INPUT DROP [0:0]\n-A INPUT -p tcp --dport 2222 -j ACCEPT\nCOMMIT\n",
        # As on 3.10.1: tune2fs on each partition as it appears (started for the SD card after the soft-reboot).
        f"{UNITS}/no-reserved-blocks@.service": "[Unit]\nDefaultDependencies=no\n[Service]\nType=oneshot\n"
                                                "ExecStart=/usr/sbin/set-no-ext4-reserved-blocks %I\n",
        "usr/lib/udev/rules.d/60-fake-block.rules": UDEV_RULES,
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
        self.screen_log = os.path.join(self.tmp, "mpfree-screen-log")
        write(self.screen_log, fake_elf(), 0o755)
        self.key = os.path.join(self.tmp, "mpc_key")
        made = run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "fake-user-key", "-f", self.key])
        self.assertEqual(made.returncode, 0, made.stdout)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def build(self, *args, **extra):
        env = {"HOME": self.home, "LC_ALL": "C", "PATH": SBIN_PATH, "MPC_KEY": self.key,
               "MPFREE_SCREEN_LOG": self.screen_log, **extra}
        return subprocess.run([sys.executable, "-I", TOOL, *args], env=env, stdout=subprocess.PIPE,
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
        self.assertIn("  route: soft-reboot only (no --kexec-file)", res.stdout)

        tree = dtb.load(os.path.join(out, "kexec.dtb"))
        self.assertEqual(tree.find("/dwmmc@ff0f0000").prop("status"), dt_strings("disabled"))
        self.assertEqual(tree.find("/dwmmc@ff0c0000").prop("status"), dt_strings("disabled"))
        self.assertEqual(tree.find("/dwmmc@ff0f0000").prop("new-in-9.9"), dt_strings("yes"))
        self.assertEqual(tree.root.prop("serial-number"), dt_strings(SERIAL))
        self.assertEqual(tree.find("/ethernet@ff290000").prop("local-mac-address"), MAC)
        self.assertIsNone(tree.find("/chosen").prop("inmusic,acvs-emmc-content"))
        self.assertIsNone(tree.find("/chosen").prop("bootargs"))
        phy = "/ethernet@ff290000/mdio/ethernet-phy@1"
        self.assertEqual(tree.find(phy).prop("compatible"),
                         dt_strings("ethernet-phy-id001c.c915", "ethernet-phy-ieee802.3-c22"))
        self.assertIn(f"Ethernet PHY {phy}: the kernel takes its ID (ethernet-phy-id001c.c915) from the tree",
                      res.stdout)
        self.assertIn("panel rotation (from /mipi@ff960000/panel@0): 270 degrees", res.stdout)
        self.assertIn(f"screen log program: {self.screen_log} ({len(fake_elf())} bytes, sha256 ", res.stdout)

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
        self.check_hook_files(root_img, out)

        for unit in ("acvs.service", "touch-fw-update.service", "az0x-data-mkfs.service", "iptables.service",
                     "no-reserved-blocks@.service"):
            self.assertIn('Fast link dest: "/dev/null"', self.dfs(root_img, f"stat /etc/systemd/system/{unit}"))
        rules = "/usr/lib/udev/rules.d/60-fake-block.rules"
        self.assertIn(
            "\nunits Akai's udev rules start when a device appears (after the soft-reboot these can start before the "
            "eMMC and SD card are off, so any that can touch a disk must be masked):\n"
            f'  {rules}: SUBSYSTEM=="block", ENV{{ID_FS_TYPE}}=="?*", '
            'ENV{SYSTEMD_WANTS}+="no-reserved-blocks@%k.service"\n'
            "    no-reserved-blocks@%k.service: masked on the stick\n"
            f'  {rules}: SUBSYSTEM=="net", KERNEL=="eth0", ENV{{SYSTEMD_WANTS}}+="fake-net.service"\n'
            "    fake-net.service: NOT masked\n"
            f'  {rules}: SUBSYSTEM=="block", PROGRAM="/usr/bin/fake-namer", ENV{{SYSTEMD_WANTS}}+="%c"\n'
            "    %c: named only when the device appears; this tool cannot tell which unit\n"
            f'  {rules}: KERNEL=="sd*", ENV{{SYSTEMD_WANTS}}="systemd-fsck@dev-sda1.service acvs.service"\n'
            "    systemd-fsck@dev-sda1.service: NOT masked\n"
            "    acvs.service: masked on the stick\n", res.stdout)
        self.assertNotIn("commented-out", res.stdout)
        self.assertIn('Fast link dest: "/dev/null"',
                      self.dfs(root_img, "stat /etc/systemd/system-generators/acva-emmc-content-generator"))
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-sshd.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/multi-user.target.wants/mpfree-test-sshd.service"))
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-logs.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-logs.service"))
        self.assertIn("File not found",
                      self.dfs(root_img, "stat /etc/systemd/system/multi-user.target.wants/mpfree-test-logs.service"))
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-screen.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-screen.service"))
        screen_unit = self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-screen.service")
        for line in ("DefaultDependencies=no", "IgnoreOnIsolate=yes", "WantedBy=sysinit.target",
                     "ExecStart=/bin/sh -c 'journalctl -b -f -n 100 --no-pager --no-hostname -o short-monotonic "
                     "2>&1 | /etc/mpfree-test/mpfree-screen-log --rotation 270'"):
            self.assertIn(line + "\n", screen_unit)
        program = os.path.join(self.tmp, "screen-log-copy")
        run(["debugfs", "-R", f"dump /etc/mpfree-test/mpfree-screen-log {program}", root_img])
        self.assertEqual(sha(program), sha(self.screen_log))
        self.assertRegex(self.dfs(root_img, "stat /etc/mpfree-test/mpfree-screen-log"), r"Type: regular\s+Mode:\s+0755")
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-ethernet.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-ethernet.service"))
        eth_unit = self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-ethernet.service")
        for line in ("DefaultDependencies=no", "IgnoreOnIsolate=yes", "WantedBy=sysinit.target",
                     "After=systemd-journald.service sys-kernel-debug.mount",
                     "ExecStart=/bin/sh /etc/mpfree-test/ethernet-retry.sh"):
            self.assertIn(line + "\n", eth_unit)
        details = os.path.join(self.tmp, "ethernet-details-copy")
        run(["debugfs", "-R", f"dump /etc/mpfree-test/ethernet-details.sh {details}", root_img])
        self.assertEqual(sha(details), sha(os.path.join(TOOLS, "read-only-ethernet.sh")))
        retry = self.dfs(root_img, "cat /etc/mpfree-test/ethernet-retry.sh")
        self.assertIn("sh /etc/mpfree-test/ethernet-details.sh > \"$out/ethernet-$1.txt\"", retry)
        self.assertIn("for at in 20 45 90; do", retry)
        self.assertIn('cp /run/mpfree-test-eth/*.txt "$d/" 2>/dev/null', self.dfs(root_img, "cat /etc/mpfree-test/save-logs.sh"))
        for path in ("/etc/mpfree-test/ethernet-details.sh", "/etc/mpfree-test/ethernet-retry.sh"):
            self.assertRegex(self.dfs(root_img, f"stat {path}"), r"Type: regular\s+Mode:\s+0755")
        log_unit = self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-logs.service")
        for line in ("DefaultDependencies=no", "IgnoreOnIsolate=yes", "WantedBy=sysinit.target"):
            self.assertIn(line + "\n", log_unit)
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

    def check_hook_files(self, root_img, out):
        """/boot/mpfree-kexec on partition 1: the boot hook's copy of the build folder's kexec files."""
        self.assertIn('Fast link dest: "../zImage-' + NEW + '"', self.dfs(root_img, "stat /boot/mpfree-kexec/zImage"))
        for name in ("kexec.dtb", "cmdline.txt"):
            copy = os.path.join(self.tmp, "hook-" + name)
            run(["debugfs", "-R", f"dump /boot/mpfree-kexec/{name} {copy}", root_img])
            self.assertEqual(sha(copy), sha(os.path.join(out, name)), name)
            self.assertRegex(self.dfs(root_img, f"stat /boot/mpfree-kexec/{name}"), r"Type: regular\s+Mode:\s+0644")
        sums = self.dfs(root_img, "cat /boot/mpfree-kexec/SHA256SUMS")
        self.assertEqual(sums, "".join(f"{sha(os.path.join(out, n))}  {n}\n" for n in ("zImage", "kexec.dtb", "cmdline.txt")))
        self.check_softreboot_files(root_img, NEW)

    def check_softreboot_files(self, root_img, release, modules=(), kexec_file=None, chain=False):
        """/boot/mpfree-softreboot and the service that switches the eMMC and SD card off, as the hook checks them.
        With `kexec_file` (the command line), the three files of --kexec-file too; without, none of them. With
        `chain`, the chain's service and the stick's own kernel files are listed too."""
        mod = stick_module()
        self.assertEqual(self.dfs(root_img, "cat /boot/mpfree-softreboot/PROTOCOL"), "mpfree-softreboot-1\n")
        self.assertEqual(self.dfs(root_img, "cat /boot/mpfree-softreboot/release"), release + "\n")
        listed = ["/etc/mpfree-test/internal-off.sh", "/etc/systemd/system/mpfree-test-internal-off.service",
                  "/etc/udev/rules.d/00-mpfree-internal-off.rules", "/boot/mpfree-softreboot/PROTOCOL",
                  "/boot/mpfree-softreboot/release"]
        if modules:
            self.assertEqual(self.dfs(root_img, "cat /boot/mpfree-softreboot/preload-modules.txt"),
                             "".join(m + "\n" for m in modules))
            listed.append("/boot/mpfree-softreboot/preload-modules.txt")
        else:
            self.assertIn("File not found", self.dfs(root_img, "stat /boot/mpfree-softreboot/preload-modules.txt"))
        if kexec_file:
            self.assertEqual(self.dfs(root_img, "cat /boot/mpfree-softreboot/jump"), "kexec-file\n")
            self.assertEqual(self.dfs(root_img, "cat /boot/mpfree-softreboot/cmdline"), kexec_file + "\n")
            self.assertEqual(self.dfs(root_img, "cat /boot/mpfree-softreboot/kernel.sha256"),
                             f"{hashlib.sha256(ZIMAGE).hexdigest()}  /boot/zImage-{release}\n")
            listed += [f"/boot/mpfree-softreboot/{n}" for n in ("jump", "cmdline", "kernel.sha256")]
        else:
            for name in ("jump", "cmdline", "kernel.sha256"):
                self.assertIn("File not found", self.dfs(root_img, f"stat /boot/mpfree-softreboot/{name}"))
        if chain:
            listed += ["/etc/mpfree-test/chain.sh", "/etc/systemd/system/mpfree-test-chain.service",
                       "/boot/mpfree-kexec/kexec.dtb", "/boot/mpfree-kexec/cmdline.txt", "/boot/mpfree-kexec/SHA256SUMS"]
        else:
            for path in ("/etc/mpfree-test/chain.sh", "/etc/systemd/system/mpfree-test-chain.service",
                         "/etc/systemd/system/sysinit.target.wants/mpfree-test-chain.service"):
                self.assertIn("File not found", self.dfs(root_img, f"stat {path}"))
        want = ""
        for path in listed:
            copy = os.path.join(self.tmp, "softreboot-copy")
            run(["debugfs", "-R", f"dump {path} {copy}", root_img])
            want += f"{sha(copy)}  {path}\n"
        self.assertEqual(self.dfs(root_img, "cat /boot/mpfree-softreboot/SHA256SUMS"), want)
        self.assertEqual(self.dfs(root_img, "cat /etc/mpfree-test/internal-off.sh"), mod.INTERNAL_OFF_SCRIPT)
        self.assertRegex(self.dfs(root_img, "stat /etc/mpfree-test/internal-off.sh"), r"Type: regular\s+Mode:\s+0755")
        unit = self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-internal-off.service")
        for line in ("DefaultDependencies=no", "FailureAction=poweroff", "Type=oneshot", "WantedBy=sysinit.target",
                     "ExecStart=/bin/sh /etc/mpfree-test/internal-off.sh"):
            self.assertIn(line + "\n", unit)
        before = " ".join(l[len("Before="):] for l in unit.splitlines() if l.startswith("Before=")).split()
        for u in ("sysinit.target", "local-fs-pre.target", "systemd-udevd.service", "systemd-udev-trigger.service",
                  "data.mount", "mpfree-test-app.service"):
            self.assertIn(u, before)
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-internal-off.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-internal-off.service"))
        rule = self.dfs(root_img, "cat /etc/udev/rules.d/00-mpfree-internal-off.rules")
        self.assertIn('KERNEL=="mmcblk*", ENV{SYSTEMD_READY}="0", ENV{UDISKS_IGNORE}="1", ENV{UDISKS_AUTO}="0"', rule)
        if shutil.which("udevadm"):
            path = os.path.join(self.tmp, "00-mpfree-internal-off.rules")
            write(path, rule)
            check = subprocess.run(["udevadm", "verify", path], stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True)
            self.assertEqual(check.returncode, 0, check.stdout)

    def test_preload_modules_for_the_soft_reboot(self):
        write(os.path.join(self.pre, "release.txt"), NEW + "\n")
        write(os.path.join(self.pre, "live.dtb"), live_tree_new())
        res = self.build("--preload-module", "snd_usb_audio", "--preload-module", "hid-multitouch")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("modules the hook loads from Akai's system before the soft-reboot: snd_usb_audio hid-multitouch",
                      res.stdout)
        [folder] = self.outputs()
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(os.path.join(self.work, folder, "stick.img"), 1, root_img)
        self.check_softreboot_files(root_img, NEW, ("snd_usb_audio", "hid-multitouch"))
        shutil.rmtree(os.path.join(self.work, folder))
        for bad in (["--preload-module", "../evil"], ["--preload-module", "a b"],
                    ["--preload-module", "x", "--preload-module", "x"]):
            res = self.build(*bad)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn("STOP: --preload-module", res.stdout)
            self.assertEqual(self.outputs(), [])

    def test_kexec_file_route_is_opt_in(self):
        write(os.path.join(self.pre, "release.txt"), NEW + "\n")
        write(os.path.join(self.pre, "live.dtb"), live_tree_new())
        res = self.build("--kexec-file", "--loadpin-off")
        self.assertEqual(res.returncode, 0, res.stdout)
        [folder] = self.outputs()
        out = os.path.join(self.work, folder)
        with open(os.path.join(out, "cmdline.txt")) as fh:
            cmdline = fh.read().strip()
        self.assertNotIn("loadpin", cmdline, "the kexec test boot's own command line is unchanged")
        self.assertIn(f"  its command line: {cmdline} loadpin.enforce=0", res.stdout)
        self.assertIn("--kexec-file: /boot/mpfree-softreboot/jump asks the hook to restart into Akai's own kernel",
                      res.stdout)
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(os.path.join(out, "stick.img"), 1, root_img)
        self.check_softreboot_files(root_img, NEW, kexec_file=cmdline + " loadpin.enforce=0")
        shutil.rmtree(out)
        res = self.build("--kexec-file")
        self.assertEqual(res.returncode, 0, res.stdout)
        [folder] = self.outputs()
        with open(os.path.join(self.work, folder, "cmdline.txt")) as fh:
            cmdline = fh.read().strip()   # a new stick: a new partition UUID
        self.assertIn(f"  its command line: {cmdline}\n", res.stdout)
        self.partition(os.path.join(self.work, folder, "stick.img"), 1, root_img)
        self.check_softreboot_files(root_img, NEW, kexec_file=cmdline)
        shutil.rmtree(os.path.join(self.work, folder))
        res = self.build("--loadpin-off")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: --loadpin-off goes with --kexec-file", res.stdout)
        self.assertEqual(self.outputs(), [])

    def chain_image(self, shutdown=b"\x00/usr/sbin/kexec\x00-e\x00/sys/kernel/kexec_loaded\x00", kexec=True):
        unpacked = os.path.join(self.work, "akai-unpacked-20260101T000000Z-abc")
        extra = {"usr/sbin/kexec": "#!/bin/sh\n# AKAI-KEXEC\n"} if kexec else {}
        if shutdown is not None:
            extra["usr/lib/systemd/systemd-shutdown"] = b"\x7fELF-FAKE" + shutdown
        build_rootfs(unpacked, extra=extra)

    def test_chain_is_opt_in(self):
        """--chain: the kexec-file kernel keeps kexec on and asks for the chain; the stick restarts into its own
        kernel and device tree with chain.sh; Akai's kexec stays. Without it, the stick is as before."""
        write(os.path.join(self.pre, "release.txt"), NEW + "\n")
        write(os.path.join(self.pre, "live.dtb"), live_tree_new())
        self.chain_image()
        mod = stick_module()
        res = self.build("--kexec-file", "--loadpin-off", "--chain")
        self.assertEqual(res.returncode, 0, res.stdout)
        [folder] = self.outputs()
        out = os.path.join(self.work, folder)
        with open(os.path.join(out, "cmdline.txt")) as fh:
            cmdline = fh.read().strip()
        self.assertIn("sysctl.kernel.kexec_load_disabled=1", cmdline, "the second kernel keeps kexec off")
        self.assertNotIn("chain", cmdline)
        first = cmdline.replace(" sysctl.kernel.kexec_load_disabled=1", "") + " loadpin.enforce=0 mpfree.chain=1"
        self.assertIn(f"  its command line: {first}\n", res.stdout)
        self.assertIn("  --chain: on that kernel (mpfree.chain=1, kexec left on), mpfree-test-chain.service loads "
                      "/boot/mpfree-kexec's kernel and device tree", res.stdout)
        self.assertIn("  --chain: /usr/lib/systemd/systemd-shutdown runs /usr/sbin/kexec for kexec.target; "
                      "/usr/sbin/kexec stays Akai's", res.stdout)
        self.assertNotIn("written  /usr/sbin/kexec", res.stdout)
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(os.path.join(out, "stick.img"), 1, root_img)
        self.check_softreboot_files(root_img, NEW, kexec_file=first, chain=True)
        self.assertEqual(self.dfs(root_img, "cat /usr/sbin/kexec"), "#!/bin/sh\n# AKAI-KEXEC\n")
        self.assertEqual(self.dfs(root_img, "cat /etc/mpfree-test/chain.sh"), mod.CHAIN_SCRIPT)
        self.assertRegex(self.dfs(root_img, "stat /etc/mpfree-test/chain.sh"), r"Type: regular\s+Mode:\s+0755")
        self.assertEqual(self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-chain.service"), mod.CHAIN_UNIT)
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-chain.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-chain.service"))
        self.assertEqual(self.dfs(root_img, "cat /boot/mpfree-kexec/cmdline.txt"), cmdline + "\n")
        self.assertIn("kernel.kexec_load_disabled = 1", self.dfs(root_img, "cat /etc/sysctl.d/90-mpfree-test.conf"))
        shutil.rmtree(out)
        # The same image without --chain: Akai's kexec is a stub again, and no chain files.
        res = self.build("--kexec-file", "--loadpin-off")
        self.assertEqual(res.returncode, 0, res.stdout)
        [folder] = self.outputs()
        self.assertIn("written  /usr/sbin/kexec  (stub)", res.stdout)
        self.assertNotIn("--chain", res.stdout)
        with open(os.path.join(self.work, folder, "cmdline.txt")) as fh:
            cmdline = fh.read().strip()
        self.partition(os.path.join(self.work, folder, "stick.img"), 1, root_img)
        self.check_softreboot_files(root_img, NEW, kexec_file=cmdline + " loadpin.enforce=0")
        self.assertIn("is disabled on this test boot", self.dfs(root_img, "cat /usr/sbin/kexec"))
        shutil.rmtree(os.path.join(self.work, folder))

    def test_chain_stops(self):
        write(os.path.join(self.pre, "release.txt"), NEW + "\n")
        write(os.path.join(self.pre, "live.dtb"), live_tree_new())
        for args, stop in ((("--chain",), "--chain goes with --kexec-file and --loadpin-off"),
                           (("--kexec-file", "--chain"), "--chain goes with --kexec-file and --loadpin-off"),
                           (("--loadpin-off", "--chain"), "--loadpin-off goes with --kexec-file")):
            with self.subTest(args):
                res = self.build(*args)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("STOP: " + stop, res.stdout)
                self.assertEqual(self.outputs(), [])
        for name, image, stop in (
                ("no kexec in the image", dict(kexec=False),
                 "--chain: the image has no regular /usr/sbin/kexec"),
                ("no systemd-shutdown", dict(shutdown=None),
                 "--chain: the image has no /usr/lib/systemd/systemd-shutdown"),
                ("systemd-shutdown runs another kexec", dict(shutdown=b"\x00/usr/bin/kexec\x00"),
                 "--chain: /usr/lib/systemd/systemd-shutdown names /usr/bin/kexec for kexec")):
            with self.subTest(name):
                self.chain_image(**image)
                res = self.build("--kexec-file", "--loadpin-off", "--chain")
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("STOP: " + stop, res.stdout)
                self.assertEqual([d for d in self.outputs() if os.path.exists(os.path.join(self.work, d, "stick.img"))],
                                 [])
        with self.subTest("systemd-shutdown starts the kernel itself"):
            self.chain_image(shutdown=b"\x00reboot\x00/sys/kernel/kexec_loaded\x00")
            res = self.build("--kexec-file", "--loadpin-off", "--chain")
            self.assertEqual(res.returncode, 0, res.stdout)
            self.assertIn("runs no kexec program (it starts the loaded kernel itself)", res.stdout)

    def test_same_release_installed(self):
        """An installed 3.10.1 starting the 3.10.1 stick: the running tree is U-Boot's for the same release."""
        write(os.path.join(self.pre, "release.txt"), NEW + "\n")
        write(os.path.join(self.pre, "live.dtb"), live_tree_new())
        res = self.build()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"running release: {NEW} (recorded by the preflight; the same kernel as the test build's)\n"
                      "  the stick's own release, installed",
                      res.stdout)
        self.assertIn("board file of the running release: " + os.path.join(self.work, "boot-files", NEW,
                                                                             "rk3288-az01-acva.dtb"), res.stdout)
        [folder] = self.outputs()
        out = os.path.join(self.work, folder)
        tree = dtb.load(os.path.join(out, "kexec.dtb"))
        self.assertEqual(tree.find("/dwmmc@ff0f0000").prop("status"), dt_strings("disabled"))
        self.assertEqual(tree.find("/dwmmc@ff0f0000").prop("new-in-9.9"), dt_strings("yes"))
        self.assertEqual(tree.root.prop("serial-number"), dt_strings(SERIAL))
        self.assertEqual(tree.find("/ethernet@ff290000").prop("local-mac-address"), MAC)
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(os.path.join(out, "stick.img"), 1, root_img)
        self.check_hook_files(root_img, out)

        # A tree read on a test build that the stick test started (its eMMC hidden) is refused.
        shutil.rmtree(out)
        write(os.path.join(self.pre, "live.dtb"), live_tree_new(emmc="disabled"))
        self.expect_stop("the running tree has no enabled eMMC controller")

    def test_startup_screen_before_the_log(self):
        splash = os.path.join(self.tmp, "mpfree-splash")
        write(splash, fake_elf(), 0o755)
        font = os.path.join(self.tmp, "Cynatar.otf")
        write(font, b"OTTO" + os.urandom(2000))
        res = self.build("--splash", splash, "--splash-font", font)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("shown for 6 s before the log", res.stdout)
        self.assertIn(f"startup screen font: {os.path.realpath(font)}", res.stdout)
        [folder] = self.outputs()
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(os.path.join(self.work, folder, "stick.img"), 1, root_img)
        self.assertEqual(run(["e2fsck", "-fn", root_img]).returncode, 0)
        unit = self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-splash.service")
        for line in ("DefaultDependencies=no", "Type=oneshot", "Before=shutdown.target mpfree-test-screen.service",
                     "ExecStart=/etc/mpfree-test/mpfree-splash --framebuffer --rotation 270 --seconds 6 "
                     "--font /etc/mpfree-test/splash-font.otf --tracking 0.15",
                     "TimeoutStartSec=20", "WantedBy=sysinit.target"):
            self.assertIn(line + "\n", unit)
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-splash.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-splash.service"))
        self.assertIn("After=systemd-journald.service mpfree-test-splash.service\n",
                      self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-screen.service"))
        for name, source, mode in (("mpfree-splash", splash, "0755"), ("splash-font.otf", font, "0644")):
            copy = os.path.join(self.tmp, name + "-copy")
            run(["debugfs", "-R", f"dump /etc/mpfree-test/{name} {copy}", root_img])
            self.assertEqual(sha(copy), sha(source))
            self.assertRegex(self.dfs(root_img, f"stat /etc/mpfree-test/{name}"), rf"Type: regular\s+Mode:\s+{mode}")

    def test_startup_screen_is_optional_and_checked(self):
        res = self.build()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("startup screen: none (no --splash)", res.stdout)
        [folder] = self.outputs()
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(os.path.join(self.work, folder, "stick.img"), 1, root_img)
        self.assertIn("File not found", self.dfs(root_img, "stat /etc/systemd/system/mpfree-test-splash.service"))
        self.assertIn("After=systemd-journald.service\n",
                      self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-screen.service"))
        splash = os.path.join(self.tmp, "mpfree-splash")
        write(splash, fake_elf(), 0o755)
        not_font = os.path.join(self.tmp, "notes.otf")
        write(not_font, b"hello, not a font")
        for args, problem in ((("--splash-font", not_font), "--splash-font needs --splash"),
                              (("--splash", splash, "--splash-font", not_font), "is not a single TrueType or OpenType"),
                              (("--splash", self.key + ".pub"), "is not a 32-bit little-endian ELF")):
            res = self.build(*args)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(problem, res.stdout)

    def kits_file(self, folder, count=3, size=3 << 20):
        path = os.path.join(self.work, folder, "kits.bin")
        write(path, b"MPFKITS1" + struct.pack("<I", count) + os.urandom(size))
        return path

    def test_pad_player_at_power_on(self):
        splash = os.path.join(self.tmp, "mpfree-splash")
        write(splash, fake_elf(), 0o755)
        app = os.path.join(self.tmp, "mpfree")
        write(app, fake_elf() + os.urandom(5000), 0o755)
        self.kits_file("factory-kit-20260101T000000Z-old", count=1, size=1000)
        kits = self.kits_file("factory-kit-20260102T000000Z-new")
        res = self.build("--splash", splash, "--app", app)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"drum kits: {os.path.realpath(kits)} (3 kits, {os.path.getsize(kits)} bytes", res.stdout)
        self.assertIn("started at power-on in place of the log display, over the startup screen", res.stdout)
        [folder] = self.outputs()
        out = os.path.join(self.work, folder)
        with open(os.path.join(out, "report.txt")) as fh:
            report = fh.read()
        # read-stick-logs.py finds the log partition by this line; it must stay as it was.
        self.assertRegex(report, r"partition 2 logs PARTUUID [0-9a-f-]{36}\n")
        kits_uuid = re.search(r"partition 3 kits PARTUUID ([0-9a-f-]{36}) ", report).group(1)
        data_uuid = re.search(r"partition 4 data PARTUUID ([0-9a-f-]{36}) \(mpfree-test-data, ", report).group(1)
        self.assertIn("holds Akai's sounds: keep it private", report)
        stick = os.path.join(out, "stick.img")
        table = json.loads(run(["sfdisk", "--json", stick]).stdout)["partitiontable"]["partitions"]
        self.assertEqual([p.get("name") for p in table],
                         ["mpfree-test-root", "mpfree-test-log", "mpfree-test-kits", "mpfree-test-data"])
        self.assertEqual(table[2]["uuid"].lower(), kits_uuid)
        self.assertEqual(table[2]["start"], table[1]["start"] + table[1]["size"])
        self.assertEqual(table[2]["start"] % 2048, 0)
        self.assertEqual(table[3]["uuid"].lower(), data_uuid)
        # Partition 4 starts 1 GiB in on every build, so write-test-stick.py can leave it in place.
        self.assertEqual(table[3]["start"], (1 << 30) // 512)
        self.assertLess(table[2]["start"] + table[2]["size"], table[3]["start"])
        self.assertEqual(table[3]["size"], (64 << 20) // 512)
        self.assertEqual(os.path.getsize(stick), (table[3]["start"] + table[3]["size"] + 2048) * 512)

        # Partition 4: an empty journaled ext4, labelled for write-test-stick.py to recognise.
        data_img = os.path.join(self.tmp, "p4.img")
        self.partition(stick, 4, data_img)
        self.assertEqual(run(["e2fsck", "-fn", data_img]).returncode, 0)
        stats = self.dfs(data_img, "stats")
        self.assertIn("has_journal", re.search(r"^Filesystem features:(.*)$", stats, re.M).group(1))
        self.assertRegex(stats, r"Filesystem volume name:\s+MPFREE-DATA\n")
        # Room to grow online to 256 GiB: (group descriptor blocks + reserved ones) * 64 descriptors of
        # 64 bytes per 4 KiB block * 128 MiB per group.
        self.assertRegex(stats, r"Block size:\s+4096\n")
        self.assertIn("resize_inode", re.search(r"^Filesystem features:(.*)$", stats, re.M).group(1))
        reserved = int(re.search(r"Reserved GDT blocks:\s+(\d+)", stats).group(1))
        self.assertGreaterEqual((1 + reserved) * 64 * (128 << 20), 256 << 30)
        self.assertIn("partition 4: this version has no /usr/sbin/resize2fs", res.stdout)
        self.assertEqual(sorted(n for n in self.dfs(data_img, "ls -p /").split("/") if n.strip() and
                                not n.strip().isdigit() and n.strip() not in (".", "..")), ["lost+found"])
        # write-test-stick.py finds it the same way when it carries the old stick's over.
        spec = importlib.util.spec_from_file_location("write_test_stick", os.path.join(os.path.dirname(TOOL),
                                                                                      "write-test-stick.py"))
        writer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(writer)
        self.assertEqual(writer.image_data_partition(stick), (table[3]["start"], table[3]["size"]))
        with open(data_img, "rb") as fh:
            self.assertTrue(writer.is_data_fs(fh.read(), 64 << 20))

        kits_img = os.path.join(self.tmp, "p3.img")
        self.partition(stick, 3, kits_img)
        self.assertEqual(run(["e2fsck", "-fn", kits_img]).returncode, 0)
        stats = self.dfs(kits_img, "stats")
        self.assertNotIn("has_journal", re.search(r"^Filesystem features:(.*)$", stats, re.M).group(1))
        self.assertIn("MPFREE-KITS", stats)
        self.assertEqual(sorted(n for n in self.dfs(kits_img, "ls -p /").split("/") if n.strip() and
                                not n.strip().isdigit() and n.strip() not in (".", "..", "lost+found")),
                         ["kits.bin"])
        copy = os.path.join(self.tmp, "kits-copy")
        run(["debugfs", "-R", f"dump /kits.bin {copy}", kits_img])
        self.assertEqual(sha(copy), sha(kits))
        self.assertRegex(self.dfs(kits_img, "stat /kits.bin"), r"Type: regular\s+Mode:\s+0644")
        self.assertRegex(self.dfs(kits_img, "stat /kits.bin"), r"User:\s+0\s+Group:\s+0")

        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(stick, 1, root_img)
        self.assertEqual(run(["e2fsck", "-fn", root_img]).returncode, 0)
        unit = self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-app.service")
        for line in ("DefaultDependencies=no", "IgnoreOnIsolate=yes", "WantedBy=sysinit.target",
                     "After=systemd-journald.service systemd-udevd.service mpfree-test-splash.service",
                     "OnFailure=mpfree-test-screen.service", "ExecStart=/bin/sh /etc/mpfree-test/start-app.sh",
                     # A crash restarts the app (a safe start), at most four times a minute.
                     "Restart=on-failure", "RestartSec=2", "StartLimitIntervalSec=60", "StartLimitBurst=4"):
            self.assertIn(line + "\n", unit)
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-app.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-app.service"))
        # The log display is still there for OnFailure, but does not start at power-on.
        self.assertIn("File not found",
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-screen.service"))
        self.assertIn("WantedBy=sysinit.target", self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-screen.service"))
        script = self.dfs(root_img, "cat /etc/mpfree-test/start-app.sh")
        self.assertIn(f"dev=/dev/disk/by-partuuid/{kits_uuid}\n", script)
        self.assertIn(f"data=/dev/disk/by-partuuid/{data_uuid}\n", script)
        self.assertIn("exec /etc/mpfree-test/mpfree app --kits /run/mpfree-kits/kits.bin --keep-screen "
                      "--data /run/mpfree-data\n", script)
        self.assertIn('mount -t ext4 -o ro,nosuid,nodev,noexec "$dev" /run/mpfree-kits', script)
        self.assertIn("exec /etc/mpfree-test/mpfree app --kits /run/mpfree-kits/kits.bin --keep-screen\n", script)
        copy = os.path.join(self.tmp, "app-copy")
        run(["debugfs", "-R", f"dump /etc/mpfree-test/mpfree {copy}", root_img])
        self.assertEqual(sha(copy), sha(app))
        self.assertRegex(self.dfs(root_img, "stat /etc/mpfree-test/mpfree"), r"Type: regular\s+Mode:\s+0755")
        self.assertIn("cp /run/mpfree-app/report.txt", self.dfs(root_img, "cat /etc/mpfree-test/save-logs.sh"))

    def test_pad_player_without_splash_and_its_checks(self):
        app = os.path.join(self.tmp, "mpfree")
        write(app, fake_elf(), 0o755)
        kits = self.kits_file("elsewhere", count=2)
        res = self.build("--app", app, "--kits", kits)
        self.assertEqual(res.returncode, 0, res.stdout)
        [folder] = self.outputs()
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(os.path.join(self.work, folder, "stick.img"), 1, root_img)
        self.assertIn("exec /etc/mpfree-test/mpfree app --kits /run/mpfree-kits/kits.bin\n",
                      self.dfs(root_img, "cat /etc/mpfree-test/start-app.sh"))
        self.assertIn("After=systemd-journald.service systemd-udevd.service\n",
                      self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-app.service"))
        not_kits = os.path.join(self.tmp, "not-kits.bin")
        write(not_kits, b"MPFKIT01" + os.urandom(100))
        too_big = os.path.join(self.tmp, "too-big.bin")
        write(too_big, b"MPFKITS1" + b"\0" * (48 << 20))
        for args, problem in ((("--kits", kits), "--kits needs --app"),
                              (("--app", app), "no drum kits under"),
                              (("--app", app, "--kits", not_kits), "is not a kits file"),
                              (("--app", app, "--kits", too_big), "is not a kits file"),
                              (("--app", self.key + ".pub", "--kits", kits), "is not a 32-bit little-endian ELF"),
                              (("--app", os.path.join(self.tmp, "missing"), "--kits", kits), "build the pad player")):
            res = self.build(*args)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(problem, res.stdout)

    def test_the_pad_player_runs_as_root_without_app_user(self):
        """Without --app-user the stick is as before: no mpfree user, no storage service, the app unit as it was."""
        app = os.path.join(self.tmp, "mpfree")
        write(app, fake_elf(), 0o755)
        kits = self.kits_file("elsewhere", count=2)
        res = self.build("--app", app, "--kits", kits)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("pad player user: root (no --app-user)", res.stdout)
        self.assertNotIn("mpfree-test-storage", res.stdout)
        [folder] = self.outputs()
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(os.path.join(self.work, folder, "stick.img"), 1, root_img)
        spec = importlib.util.spec_from_file_location("kexec_stick", TOOL)
        tool = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(tool)
        self.assertEqual(self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-app.service"),
                         tool.APP_UNIT.format(after=""))
        for path in ("/etc/systemd/system/mpfree-test-storage.service", "/etc/mpfree-test/storage.sh"):
            self.assertIn("File not found", self.dfs(root_img, f"stat {path}"))
        self.assertEqual(self.dfs(root_img, "cat /etc/passwd"), "root:x:0:0:root:/home/root:/bin/sh\n")
        self.assertIn("File not found", self.dfs(root_img, "stat /etc/group"))

    def app_user_image(self, group=None, programs=("find", "chown", "chgrp", "chmod"), passwd=None):
        unpacked = os.path.join(self.work, "akai-unpacked-20260101T000000Z-abc")
        extra = {f"usr/bin/{p}": "#!/bin/sh\n" for p in programs}
        extra["etc/group"] = group if group is not None else (
            "root:x:0:\naudio:x:29:\nvideo:x:44:pulse\ninput:x:101:\nsystemd-journal:x:190:\n")
        if passwd is not None:
            extra["etc/passwd"] = passwd
        build_rootfs(unpacked, extra=extra)

    def test_app_user_runs_the_pad_player_as_mpfree(self):
        """--app-user: the mpfree user and its groups, a root storage step first, and an app unit without root."""
        self.app_user_image()
        splash = os.path.join(self.tmp, "mpfree-splash")
        write(splash, fake_elf(), 0o755)
        app = os.path.join(self.tmp, "mpfree")
        write(app, fake_elf(), 0o755)
        kits = self.kits_file("elsewhere", count=2)
        res = self.build("--splash", splash, "--app", app, "--kits", kits, "--app-user")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("pad player user: mpfree (--app-user)", res.stdout)
        self.assertIn("  mpfree is in the groups: audio video input systemd-journal\n", res.stdout)
        [folder] = self.outputs()
        out = os.path.join(self.work, folder)
        with open(os.path.join(out, "report.txt")) as fh:
            report = fh.read()
        kits_uuid = re.search(r"partition 3 kits PARTUUID ([0-9a-f-]{36}) ", report).group(1)
        data_uuid = re.search(r"partition 4 data PARTUUID ([0-9a-f-]{36}) ", report).group(1)
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(os.path.join(out, "stick.img"), 1, root_img)
        self.assertEqual(run(["e2fsck", "-fn", root_img]).returncode, 0)
        self.assertEqual(self.dfs(root_img, "cat /etc/passwd"),
                         "root:x:0:0:root:/home/root:/bin/sh\n"
                         "mpfree:x:960:960:MPFree pad player:/run/mpfree-app:/sbin/nologin\n")
        self.assertEqual(self.dfs(root_img, "cat /etc/group"),
                         "root:x:0:\naudio:x:29:mpfree\nvideo:x:44:pulse,mpfree\ninput:x:101:mpfree\n"
                         "systemd-journal:x:190:mpfree\nmpfree:x:960:\n")
        # No password for mpfree: no shadow line, so no login works.
        self.assertNotIn("mpfree", self.dfs(root_img, "cat /etc/shadow"))
        unit = self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-app.service")
        for line in ("User=mpfree", "Group=mpfree", "NoNewPrivileges=yes", "CapabilityBoundingSet=",
                     "AmbientCapabilities=", "LimitRTPRIO=70", "LimitMEMLOCK=infinity",
                     "RuntimeDirectory=mpfree-app", "RuntimeDirectoryPreserve=yes",
                     "Wants=mpfree-test-storage.service",
                     "After=systemd-journald.service systemd-udevd.service mpfree-test-storage.service "
                     "mpfree-test-splash.service",
                     # As without --app-user: crash restarts, then the log display.
                     "DefaultDependencies=no", "IgnoreOnIsolate=yes", "WantedBy=sysinit.target",
                     "OnFailure=mpfree-test-screen.service", "ExecStart=/bin/sh /etc/mpfree-test/start-app.sh",
                     "Restart=on-failure", "RestartSec=2", "StartLimitIntervalSec=60", "StartLimitBurst=4"):
            self.assertIn(line + "\n", unit)
        self.assertNotRegex(unit, r"(?m)^(User|Group)=root")
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-app.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-app.service"))
        storage = self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-storage.service")
        for line in ("Type=oneshot", "RemainAfterExit=yes", "ExecStart=/bin/sh /etc/mpfree-test/storage.sh",
                     "After=systemd-journald.service systemd-udevd.service mpfree-test-internal-off.service",
                     "DefaultDependencies=no"):
            self.assertIn(line + "\n", storage)
        self.assertNotIn("User=", storage)
        script = self.dfs(root_img, "cat /etc/mpfree-test/storage.sh")
        self.assertIn(f"dev=/dev/disk/by-partuuid/{kits_uuid}\n", script)
        self.assertIn(f"data=/dev/disk/by-partuuid/{data_uuid}\n", script)
        self.assertIn("find /run/mpfree-data -xdev -path /run/mpfree-data/backup -prune -o ! -user 960 -exec chown -h 960:960 {} \\; ||", script)
        start = self.dfs(root_img, "cat /etc/mpfree-test/start-app.sh")
        self.assertIn("exec /etc/mpfree-test/mpfree app --kits /run/mpfree-kits/kits.bin --keep-screen "
                      "--data /run/mpfree-data\n", start)
        self.assertNotRegex(start, r"(?m)^\s*(mount|mkdir|chown|chgrp|chmod)\b")
        for path in ("/etc/mpfree-test/storage.sh", "/etc/mpfree-test/start-app.sh", "/etc/passwd", "/etc/group",
                     "/etc/systemd/system/mpfree-test-storage.service"):
            self.assertRegex(self.dfs(root_img, f"stat {path}"), r"User:\s+0\s+Group:\s+0", path)
        # The soft-reboot hook's files are the same as without --app-user: the hook checks them.
        sums = self.dfs(root_img, "cat /boot/mpfree-softreboot/SHA256SUMS")
        self.assertNotIn("storage", sums)
        self.assertNotIn("passwd", sums)

    def test_app_user_and_kexec_file_together(self):
        """The round of step 6 builds both opt-ins into one stick; each does what it does alone."""
        self.app_user_image()
        app = os.path.join(self.tmp, "mpfree")
        write(app, fake_elf(), 0o755)
        kits = self.kits_file("elsewhere", count=2)
        res = self.build("--app", app, "--kits", kits, "--app-user", "--kexec-file", "--loadpin-off")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("pad player user: mpfree (--app-user)", res.stdout)
        [folder] = self.outputs()
        out = os.path.join(self.work, folder)
        with open(os.path.join(out, "cmdline.txt")) as fh:
            cmdline = fh.read().strip()
        self.assertIn(f"  its command line: {cmdline} loadpin.enforce=0\n", res.stdout)
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(os.path.join(out, "stick.img"), 1, root_img)
        self.assertEqual(run(["e2fsck", "-fn", root_img]).returncode, 0)
        self.check_softreboot_files(root_img, NEW, kexec_file=cmdline + " loadpin.enforce=0")
        self.assertIn("mpfree:x:960:960:", self.dfs(root_img, "cat /etc/passwd"))
        self.assertIn("User=mpfree\n", self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-app.service"))
        sums = self.dfs(root_img, "cat /boot/mpfree-softreboot/SHA256SUMS")
        self.assertNotIn("storage", sums)
        self.assertNotIn("passwd", sums)

    def test_app_user_without_the_journal_group_and_its_stops(self):
        self.app_user_image(group="root:x:0:\naudio:x:29:\nvideo:x:44:\ninput:x:101:\n")
        app = os.path.join(self.tmp, "mpfree")
        write(app, fake_elf(), 0o755)
        kits = self.kits_file("elsewhere", count=2)
        res = self.build("--app", app, "--kits", kits, "--app-user")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("mpfree is in the groups: audio video input (the image has no systemd-journal group", res.stdout)
        for args, image, problem in (
                (("--app-user",), {}, "--app-user needs --app"),
                (("--akai-app", "--app-user"), {}, "--app-user needs --app"),
                (("--app", app, "--kits", kits, "--app-user"), {"group": "root:x:0:\naudio:x:29:\nvideo:x:44:\n"},
                 "has no input group"),
                (("--app", app, "--kits", kits, "--app-user"), {"group": "root:x:0:\naudio:x:29:\nvideo:x:44:\n"
                                                                         "input:x:101:\nmpfree:x:500:\n"},
                 "already has the name mpfree or the id 960"),
                (("--app", app, "--kits", kits, "--app-user"), {"passwd": "root:x:0:0:root:/root:/bin/sh\n"
                                                                          "web:x:960:960::/:/bin/false\n"},
                 "already has the name mpfree or the id 960"),
                (("--app", app, "--kits", kits, "--app-user"), {"programs": ("chown", "chgrp", "chmod")},
                 "the image has no find"),
                (("--app", app, "--kits", kits, "--app-user"), {"group": "root:x:0\n"},
                 "/etc/group is not lines of colon-separated fields")):
            for d in self.outputs():
                shutil.rmtree(os.path.join(self.work, d))
            self.app_user_image(**image)
            res = self.build(*args)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(problem, res.stdout)
            self.assertFalse(any(os.path.exists(os.path.join(self.work, d, "stick.img")) for d in self.outputs()))

    def test_akai_app_for_developer_mode(self):
        kits = self.kits_file("factory-kit-20260102T000000Z-new")
        res = self.build("--akai-app")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("\nakai app: on (--akai-app)\n", res.stdout, "host_kexec_boot.py reads this line")
        self.assertIn(f"drum kits (only to keep the layout; nothing reads them): {os.path.realpath(kits)}", res.stdout)
        self.assertIn("startup screen: none (no --splash)", res.stdout)
        self.assertIn("Akai's firewall: masked on the stick", res.stdout)
        self.assertIn("  /usr/lib/systemd/system/iptables.service: ExecStart=/usr/sbin/iptables-restore "
                      "/etc/iptables/iptables.rules\n  --- /etc/iptables/iptables.rules\n    *filter\n"
                      "    :INPUT DROP [0:0]\n    -A INPUT -p tcp --dport 2222 -j ACCEPT\n    COMMIT\n", res.stdout)
        self.assertIn("  /usr/lib/systemd/system/ip6tables.service: not in this version", res.stdout)
        [folder] = self.outputs()
        out = os.path.join(self.work, folder)
        with open(os.path.join(out, "report.txt")) as fh:
            self.assertIn("\nakai app: on (--akai-app)\n", fh.read())
        stick = os.path.join(out, "stick.img")
        table = json.loads(run(["sfdisk", "--json", stick]).stdout)["partitiontable"]["partitions"]
        self.assertEqual([p.get("name") for p in table],
                         ["mpfree-test-root", "mpfree-test-log", "mpfree-test-kits", "mpfree-test-data"])
        self.assertEqual(table[3]["start"], (1 << 30) // 512, "partition 4 stays where write-test-stick.py keeps it")
        root_img = os.path.join(self.tmp, "p1.img")
        self.partition(stick, 1, root_img)
        self.assertEqual(run(["e2fsck", "-fn", root_img]).returncode, 0)
        # Akai's app: not masked, its launcher not stubbed.
        self.assertIn("File not found", self.dfs(root_img, "stat /etc/systemd/system/acvs.service"))
        self.assertEqual(self.dfs(root_img, "cat /usr/bin/az01-launch-MPC"), "#!/bin/sh\n# FAKE-LAUNCHER\n")
        self.assertIn("written  /usr/bin/dfu-util  (stub)", res.stdout)
        self.assertNotIn("az01-launch-MPC  (stub)", res.stdout)
        self.assertIn("\n    acvs.service: NOT masked\n", res.stdout)
        # Every other mask and stub stays.
        for unit in ("touch-fw-update.service", "xmos-update.service", "az01-script-runner.service",
                     "az0x-webserver.service", "az0x-data-mkfs.service", "acvs-content-resize.service",
                     "no-reserved-blocks@.service"):
            self.assertIn('Fast link dest: "/dev/null"', self.dfs(root_img, f"stat /etc/systemd/system/{unit}"), unit)
        self.assertIn("mpfree-test: blocked", self.dfs(root_img, "cat /usr/bin/dfu-util"))
        self.assertIn("mpfree-test: blocked", self.dfs(root_img, "cat /usr/sbin/az01-update"))
        self.assertIn("kexec_load_disabled = 1", self.dfs(root_img, "cat /etc/sysctl.d/90-mpfree-test.conf"))
        self.assertIn("What=tmpfs", self.dfs(root_img, "cat /etc/systemd/system/data.mount"))
        # No pad player, and the log display is installed but does not start (Akai's app draws the screen).
        self.assertIn("File not found", self.dfs(root_img, "stat /etc/systemd/system/mpfree-test-app.service"))
        self.assertIn("File not found", self.dfs(root_img, "stat /etc/mpfree-test/mpfree"))
        self.assertIn("File not found",
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-screen.service"))
        self.assertIn("mpfree-screen-log", self.dfs(root_img, "cat /etc/systemd/system/mpfree-test-screen.service"))
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-sshd.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/multi-user.target.wants/mpfree-test-sshd.service"))
        self.assertIn('Fast link dest: "/etc/systemd/system/mpfree-test-logs.service"',
                      self.dfs(root_img, "stat /etc/systemd/system/sysinit.target.wants/mpfree-test-logs.service"))
        # 3.10.1 ships its rules files empty (0 bytes): the report says so rather than showing nothing.
        build_rootfs(os.path.dirname(self.image), extra={
            f"{UNITS}/ip6tables.service": "[Service]\nExecStart=/usr/sbin/ip6tables-restore "
                                          "/etc/iptables/ip6tables.rules\n",
            "etc/iptables/ip6tables.rules": ""})
        self.image_sha = sha(self.image)
        res = self.build("--akai-app")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("  /usr/lib/systemd/system/ip6tables.service: ExecStart=/usr/sbin/ip6tables-restore "
                      "/etc/iptables/ip6tables.rules\n  --- /etc/iptables/ip6tables.rules\n"
                      "    (empty: 0 bytes, so it loads no rules)\n", res.stdout)

    def test_akai_app_refuses_our_own_programs(self):
        self.kits_file("factory-kit-20260102T000000Z-new")
        app = os.path.join(self.tmp, "mpfree")
        write(app, fake_elf(), 0o755)
        font = os.path.join(self.tmp, "f.ttf")
        write(font, b"\x00\x01\x00\x00" + b"\0" * 100)
        for args, problem in ((("--akai-app", "--app", app), "--akai-app and --app cannot go together"),
                              (("--akai-app", "--splash", app), "--akai-app and --splash cannot go together"),
                              (("--akai-app", "--splash", app, "--splash-font", font),
                               "--akai-app and --splash cannot go together"),
                              (("--akai-app", "--splash-font", font), "--splash-font needs --splash")):
            res = self.build(*args)
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn(problem, res.stdout)
        self.assertEqual(self.outputs(), [], "nothing is built")
        shutil.rmtree(os.path.join(self.work, "factory-kit-20260102T000000Z-new"))
        res = self.build("--akai-app")
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("no drum kits under", res.stdout)

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
        with self.subTest("screen log program"):
            for data, message in ((fake_elf(interp=True), "is dynamically linked"),
                                  (fake_elf(machine=62), "is not an ARM executable"),
                                  (fake_elf(flags=0x05000200), "is not EABI5 hard-float"),
                                  (b"#!/bin/sh\n", "is not a 32-bit little-endian ELF file")):
                write(self.screen_log, data, 0o755)
                self.expect_stop(message)
            os.unlink(self.screen_log)
            self.expect_stop("build the screen log program first")
            write(self.screen_log, fake_elf(), 0o755)
        with self.subTest("a board file without the PHY layout"):
            folder = os.path.join(self.work, "boot-files", NEW)
            unpacked = os.path.dirname(self.image)
            os.unlink(self.image)
            build_rootfs(unpacked, extra={"boot/rk3288-az01-acva.dtb": new_board(phy=False)})
            self.image_sha = sha(self.image)
            write(os.path.join(folder, "rk3288-az01-acva.dtb"), new_board(phy=False))
            with open(os.path.join(folder, "SHA256SUMS"), "w") as fh:
                for name in sorted(os.listdir(folder)):
                    if name != "SHA256SUMS":
                        fh.write(f"{sha(os.path.join(folder, name))}  {name}\n")
            self.expect_stop("has no phy-handle to a PHY node under it")
        with self.subTest("no preflight"):
            shutil.rmtree(self.pre)
            self.expect_stop("run collect-over-ssh.sh --kexec-preflight first")
        with self.subTest("work folder inside the clone"):
            res = self.build(MPFREE_WORK_DIR=os.path.join(os.path.dirname(TOOLS), "not-ignored-work"))
            self.assertEqual(res.returncode, 1, res.stdout)
            self.assertIn("inside the git clone and not git-ignored", res.stdout)
            self.assertFalse(os.path.exists(os.path.join(os.path.dirname(TOOLS), "not-ignored-work")))

    def test_the_mpc_runs_the_test_builds_own_kernel(self):
        # An installed 3.10.1 runs the kernel the test build boots: one boot-files folder serves both.
        write(os.path.join(self.pre, "release.txt"), NEW + "\n")
        write(os.path.join(self.pre, "live.dtb"), live_tree(new=True))
        shutil.rmtree(os.path.join(self.work, "boot-files", OLD))
        res = self.build()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"running release: {NEW} (recorded by the preflight; the same kernel as the test build's)",
                      res.stdout)
        self.assertIn(f"board file of the running release: {self.work}/boot-files/{NEW}/rk3288-az01-acva.dtb",
                      res.stdout)
        [folder] = self.outputs()
        tree = dtb.load(os.path.join(self.work, folder, "kexec.dtb"))
        self.assertEqual(tree.find("/dwmmc@ff0f0000").prop("status"), dt_strings("disabled"))
        self.assertEqual(tree.find("/dwmmc@ff0c0000").prop("status"), dt_strings("disabled"))
        self.assertEqual(tree.root.prop("serial-number"), dt_strings(SERIAL))
        self.assertEqual(tree.find("/ethernet@ff290000").prop("local-mac-address"), MAC)
        self.assertIsNone(tree.find("/chosen").prop("inmusic,acvs-emmc-content"))
        self.assertIsNone(tree.find("/chosen").prop("bootargs"))

    def test_running_release_without_release_txt(self):
        os.unlink(os.path.join(self.pre, "release.txt"))
        res = self.build()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn(f"running release: {OLD} (the only other boot-files release", res.stdout)
        os.makedirs(os.path.join(self.work, "boot-files", "7.7-third"))
        self.expect_stop("cannot tell which release the MPC runs")


class StaticTests(unittest.TestCase):
    def test_ethernet_script_changes_only_ram(self):
        spec = importlib.util.spec_from_file_location("kexec_stick", TOOL)
        tool = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(tool)
        busybox = shutil.which("busybox")
        if busybox:
            for script in (tool.ETHERNET_SCRIPT, tool.LOG_SCRIPT.format(log_uuid="00000000-0000-0000-0000-000000000000"),
                           tool.APP_SCRIPT.format(kits_uuid="00000000-0000-0000-0000-000000000000",
                                                  data_uuid="00000000-0000-0000-0000-000000000001", keep="",
                                                  grow=tool.GROW_PROGRAM)):
                res = subprocess.run([busybox, "sh", "-n"], input=script, text=True, capture_output=True)
                self.assertEqual(res.returncode, 0, res.stderr)
        lines = [line.strip() for line in tool.ETHERNET_SCRIPT.splitlines() if not line.lstrip().startswith("#")]
        # The only commands that change anything: mounting debugfs, starting eth0, and its own files in /run.
        changing = [line for line in lines if re.search(r"\b(mount|ip|mkdir)\b", line)]
        self.assertEqual(changing, [
            'mkdir -p "$out"',
            "grep -q ' /sys/kernel/debug debugfs ' /proc/mounts || mount -t debugfs debugfs /sys/kernel/debug",
            "ip link set eth0 up 2>&1 | sed 's/^/mpfree-test-ethernet: ip: /'",
        ])
        for line in lines:
            for target in re.findall(r">\s*(\S+)", line.replace("2>&1", "").replace("2>/dev/null", "")):
                self.assertEqual(target, '"$out/ethernet-$1.txt"', line)
        for forbidden in ("/dev/mmc", "/dev/sd", "systemctl", "devmem", "/bind", "/unbind", "rm ", "dd "):
            self.assertNotIn(forbidden, tool.ETHERNET_SCRIPT)

    def test_app_start_mounts_the_kits_read_only_the_data_read_write_and_changes_nothing_else(self):
        spec = importlib.util.spec_from_file_location("kexec_stick", TOOL)
        tool = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(tool)
        lines = [line.strip() for line in tool.APP_SCRIPT.splitlines() if not line.lstrip().startswith("#")]
        changing = [line for line in lines if re.search(r"\b(mount|mkdir|rm|mv|cp|dd|systemctl|kill)\b", line)]
        self.assertEqual(changing, [
            "mkdir -p /run/mpfree-kits",
            'mount -t ext4 -o ro,nosuid,nodev,noexec "$dev" /run/mpfree-kits ||',
            'echo "mpfree-test-app: the kit partition did not mount" > /dev/kmsg',
            "mkdir -p /run/mpfree-data",
            # Partition 4, by its own PARTUUID: the only thing mounted read-write (storage-spec.md).
            'mount -t ext4 -o rw,nosuid,nodev,noexec,noatime "$data" /run/mpfree-data; then',
            'echo "mpfree-test-app: the data partition did not mount, so nothing is saved" > /dev/kmsg',
        ])
        self.assertEqual(tool.APP_SCRIPT.count("rw"), 1)
        self.assertIn("data=/dev/disk/by-partuuid/{data_uuid}\n", tool.APP_SCRIPT)
        # Growing partition 4's filesystem to fill the partition, online, after it is mounted: the only
        # other change, and it touches only "$data".
        grow = [line for line in lines if "{grow}" in line]
        self.assertEqual(grow, ["if [ -x {grow} ]; then",
                                '( {grow} "$data" > /run/mpfree-data-grow.txt 2>&1 ||'])
        self.assertEqual(tool.GROW_PROGRAM, "/usr/sbin/resize2fs")
        self.assertLess(tool.APP_SCRIPT.index("/run/mpfree-data; then"), tool.APP_SCRIPT.index("{grow}"))
        for line in lines:
            for target in re.findall(r">\s*(\S+)", line.replace("2>&1", "")):
                self.assertIn(target, ("/dev/kmsg", "/run/mpfree-data-grow.txt"), line)
        for forbidden in ("/dev/mmc", "/dev/sd", "remount"):
            self.assertNotIn(forbidden, tool.APP_SCRIPT)

    def test_app_user_scripts_and_unit(self):
        """--app-user: storage.sh changes only what start-app.sh did, plus partition 4's owners and the pan
        file's group; the new start-app.sh changes nothing; the app unit gives no privileges."""
        spec = importlib.util.spec_from_file_location("kexec_stick", TOOL)
        tool = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(tool)
        storage = tool.STORAGE_SCRIPT.format(user="mpfree", uid=960, kits_uuid="00000000-0000-0000-0000-000000000000",
                                             data_uuid="00000000-0000-0000-0000-000000000001", grow=tool.GROW_PROGRAM)
        start = tool.APP_USER_SCRIPT.format(user="mpfree", keep="")
        busybox = shutil.which("busybox")
        for script in (storage, start):
            for shell in ([busybox, "sh"] if busybox else [], ["sh"]):
                if shell:
                    res = subprocess.run([*shell, "-n"], input=script, text=True, capture_output=True)
                    self.assertEqual(res.returncode, 0, res.stderr)
        lines = [line.strip() for line in storage.splitlines() if not line.lstrip().startswith("#")]
        changing = [line for line in lines
                    if re.search(r"\b(mount|mkdir|rm|mv|cp|dd|systemctl|kill|chown|chgrp|chmod)\b", line)]
        self.assertEqual(changing, [
            "mkdir -p /run/mpfree-kits",
            'mount -t ext4 -o ro,nosuid,nodev,noexec "$dev" /run/mpfree-kits ||',
            'echo "mpfree-test-storage: the kit partition did not mount" > /dev/kmsg',
            "mkdir -p /run/mpfree-data",
            'mount -t ext4 -o rw,nosuid,nodev,noexec,noatime "$data" /run/mpfree-data; then',
            # Only on partition 4, only files not yet mpfree's; -h never follows a link.
            "find /run/mpfree-data -xdev -path /run/mpfree-data/backup -prune -o ! -user 960 -exec chown -h 960:960 {} \\; ||",
            'echo "mpfree-test-storage: the data partition did not mount, so nothing is saved" > /dev/kmsg',
            'chgrp 960 "$p" && chmod g+w "$p" ||',
        ])
        self.assertIn("for p in /sys/class/graphics/fb*/pan; do", storage)
        self.assertEqual(storage.count("rw"), 1)
        self.assertIn([line for line in lines if "{grow}" in line or tool.GROW_PROGRAM in line],
                      [["if [ -x /usr/sbin/resize2fs ]; then",
                        '( /usr/sbin/resize2fs "$data" > /run/mpfree-data-grow.txt 2>&1 ||']])
        self.assertLess(storage.index("/run/mpfree-data; then"), storage.index("find /run/mpfree-data"))
        for line in lines:
            for target in re.findall(r">\s*(\S+)", line.replace("2>&1", "")):
                self.assertIn(target, ("/dev/kmsg", "/run/mpfree-data-grow.txt"), line)
        for forbidden in ("/dev/mmc", "/dev/sd", "remount", "/bind", "/unbind", "chown -R", "-L "):
            self.assertNotIn(forbidden, storage)
        self.assertTrue(storage.rstrip().endswith("exit 0"), "storage.sh never fails the boot")
        # The new start-app.sh only starts the program.
        start_lines = [line.strip() for line in start.splitlines() if line.strip() and not line.lstrip().startswith("#")]
        self.assertIsNone(re.search(r"\b(mount|mkdir|rm|mv|cp|dd|systemctl|kill|chown|chgrp|chmod)\b",
                                    "\n".join(start_lines)))
        self.assertNotIn(">", "\n".join(start_lines))
        unit = tool.APP_USER_UNIT.format(user="mpfree", after="")
        self.assertRegex(unit, r"(?m)^CapabilityBoundingSet=$")
        self.assertRegex(unit, r"(?m)^AmbientCapabilities=$")
        for word in ("PermissionsStartOnly", "+/", "!/", "SupplementaryGroups=root", "User=root"):
            self.assertNotIn(word, unit)
        self.assertEqual(re.findall(r"(?m)^ExecStart\S*=.*$", unit), ["ExecStart=/bin/sh /etc/mpfree-test/start-app.sh"])
        self.assertEqual(tool.APP_UID, 960)

    def test_busybox_find_takes_the_storage_scripts_expression(self):
        """The MPC's shell tools may be BusyBox: its find must understand -xdev, -path ... -prune, ! -user and
        -exec ... (semicolon). The eMMC backups in backup/ are left alone."""
        busybox = shutil.which("busybox")
        if not busybox:
            self.skipTest("no busybox")
        d = tempfile.mkdtemp(prefix="mpfree-find-test-")
        try:
            os.makedirs(os.path.join(d, "settings"))
            write(os.path.join(d, "settings", "prefs"), "x")
            os.symlink("/etc/passwd", os.path.join(d, "link"))
            write(os.path.join(d, "backup", "mpc-one-emmc-20261115T120000Z", "SHA256SUMS"), "x")
            res = subprocess.run([busybox, "find", d, "-xdev", "-path", f"{d}/backup", "-prune", "-o", "!", "-user",
                                  "960", "-exec", "echo", "X", "{}", ";"], capture_output=True, text=True)
            self.assertEqual(res.returncode, 0, res.stderr)
            self.assertEqual(sorted(res.stdout.splitlines()),
                             sorted(f"X {p}" for p in (d, f"{d}/settings", f"{d}/settings/prefs", f"{d}/link")))
        finally:
            shutil.rmtree(d)

    def test_never_contacts_the_mpc_or_writes_devices(self):
        with open(TOOL) as fh:
            code = "\n".join(line for line in fh.read().splitlines() if not line.lstrip().startswith("#"))
        for forbidden in ('"ssh"', '"scp"', "'ssh'", "/dev/sd", "/dev/mmc", "of=/dev", "kexec -l", "sudo"):
            self.assertNotIn(forbidden, code)
        self.assertIn('"debugfs", "-w", "-f", script, img.path', code, "-w only on the copy")
        # ... and on the kits filesystem it has just made in its own scratch folder.
        self.assertIn('path = os.path.join(scratch, "kits.img")', code)
        self.assertIn('"debugfs", "-w", "-f", script, path', code)
        self.assertEqual(code.count('"-w"'), 2)


class FakeKernel:
    """A fake sysfs and /dev for internal-off.sh, with the eMMC and SD controllers bound to their driver. Each
    unbind file is a FIFO: writing a device name to it makes this "kernel" do what Linux does, remove the
    driver link and the controller's block devices (unless told to ignore it)."""

    def __init__(self, root, ignore=()):
        self.root, self.ignore, self.unbound = root, set(ignore), []
        drv = os.path.join(root, "sys/bus/platform/drivers/dwmmc_rockchip")
        os.makedirs(drv)
        for name, disks in (("ff0f0000.dwmmc", ("mmcblk0", "mmcblk0boot0", "mmcblk0boot1")),
                            ("ff0c0000.dwmmc", ("mmcblk1",)), ("ff0d0000.dwmmc", ())):
            dev = os.path.join(root, "sys/bus/platform/devices", name)
            os.makedirs(dev)
            os.symlink(os.path.relpath(drv, dev), os.path.join(dev, "driver"))
            for disk in disks:
                os.makedirs(os.path.join(root, "sys/block", disk))
                write(os.path.join(root, "sys/block", disk, "owner"), name)
        os.mkfifo(os.path.join(drv, "unbind"))
        os.makedirs(os.path.join(root, "dev/disk/by-partlabel"))
        os.symlink("../../mmcblk0p8", os.path.join(root, "dev/disk/by-partlabel/data"))
        os.symlink("../../sda1", os.path.join(root, "dev/disk/by-partlabel/mpfree-test-root"))
        for rel, text in (("proc/uptime", "7.40 2.00\n"), ("dev/kmsg", "")):
            write(os.path.join(root, rel), text)
        os.makedirs(os.path.join(root, "run"), exist_ok=True)
        import threading
        self.stop = False
        self.thread = threading.Thread(target=self.serve, args=(os.path.join(drv, "unbind"),), daemon=True)
        self.thread.start()

    def serve(self, fifo):
        # Held open for writing too, so the FIFO never reports an end and no write between two reads is lost.
        with os.fdopen(os.open(fifo, os.O_RDWR), "r") as fh:
            for line in fh:
                if self.stop:
                    return
                for name in line.split():
                    self.unbound.append(name)
                    if name in self.ignore:
                        continue
                    os.unlink(os.path.join(self.root, "sys/bus/platform/devices", name, "driver"))
                    for disk in os.listdir(os.path.join(self.root, "sys/block")):
                        with open(os.path.join(self.root, "sys/block", disk, "owner")) as o:
                            if o.read() == name:
                                shutil.rmtree(os.path.join(self.root, "sys/block", disk))

    def close(self):
        self.stop = True
        with open(os.path.join(self.root, "sys/bus/platform/drivers/dwmmc_rockchip/unbind"), "w") as fh:
            fh.write("\n")
        self.thread.join(5)


class InternalOffTests(unittest.TestCase):
    """internal-off.sh, the first thing the stick's system runs after the soft-reboot, against a fake kernel."""

    def setUp(self):
        self.dash = shutil.which("dash")
        if not self.dash:
            self.skipTest("dash not installed")
        self.tmp = tempfile.mkdtemp(prefix="mpfree-internal-off-test-")
        self.root = os.path.join(self.tmp, "root")
        self.bin = os.path.join(self.tmp, "bin")
        os.makedirs(self.bin)
        write(os.path.join(self.bin, "sleep"), "#!/bin/sh\necho sleep >> \"$FAKE_LOG\"\nexec /bin/sleep 0.1\n", 0o755)
        write(os.path.join(self.bin, "sync"), "#!/bin/sh\necho sync >> \"$FAKE_LOG\"\n", 0o755)
        for tool in ("cat", "cut", "readlink", "rm"):
            os.symlink(shutil.which(tool), os.path.join(self.bin, tool))
        self.log = os.path.join(self.tmp, "log")
        self.kernel = None

    def tearDown(self):
        if self.kernel:
            self.kernel.close()
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_script(self):
        script = stick_module().INTERNAL_OFF_SCRIPT.replace("R=''", f"R='{self.root}'", 1)
        return subprocess.run([self.dash, "-c", script], env={"PATH": self.bin, "FAKE_LOG": self.log},
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=30)

    def notes(self):
        with open(os.path.join(self.root, "run/mpfree-internal-off.txt")) as fh:
            return fh.read()

    def test_switches_off_the_emmc_and_sd_card_after_a_soft_reboot(self):
        self.kernel = FakeKernel(self.root)
        write(os.path.join(self.root, "run/mpfree-hook/soft-reboot"), "6.10\n")
        # Akai's read-only dm-verity root, still known to the kernel: not waited for.
        os.makedirs(os.path.join(self.root, "sys/fs/ext4/dm-0"))
        os.makedirs(os.path.join(self.root, "sys/block/dm-0/slaves/mmcblk0p5"))
        write(os.path.join(self.root, "sys/block/dm-0/ro"), "1\n")
        write(os.path.join(self.root, "sys/block/dm-0/owner"), "dm")
        res = self.run_script()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.kernel.unbound, ["ff0f0000.dwmmc", "ff0c0000.dwmmc"], "the SDIO controller is left")
        notes = self.notes()
        self.assertIn("mpfree-internal-off: started at 7.40 s (after soft-reboot)\n", notes)
        self.assertIn("mpfree-internal-off: switched off ff0f0000.dwmmc\n", notes)
        self.assertIn("mpfree-internal-off: switched off ff0c0000.dwmmc\n", notes)
        self.assertIn("no eMMC or SD card block device is left", notes)
        self.assertFalse(os.path.lexists(os.path.join(self.root, "dev/disk/by-partlabel/data")))
        self.assertTrue(os.path.lexists(os.path.join(self.root, "dev/disk/by-partlabel/mpfree-test-root")))
        self.assertTrue(os.path.lexists(os.path.join(self.root, "sys/bus/platform/devices/ff0d0000.dwmmc/driver")))
        if os.path.exists(self.log):
            with open(self.log) as fh:
                self.assertNotIn("sync", fh.read(), "nothing to wait for")

    def test_waits_for_akai_data_to_be_let_go_then_powers_off_if_it_is_not(self):
        self.kernel = FakeKernel(self.root)
        os.makedirs(os.path.join(self.root, "sys/fs/ext4/mmcblk0p8"))
        res = self.run_script()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: mmcblk0p8 is still mounted after 10 s; powering off", self.notes())
        with open(self.log) as fh:
            self.assertEqual(fh.read().split(), ["sleep"] * 10 + ["sync"])
        self.assertEqual(self.kernel.unbound, [], "nothing is unbound under a live filesystem")

    def test_a_stop_keeps_its_notes_on_the_sticks_log_partition(self):
        self.kernel = FakeKernel(self.root, ignore=("ff0f0000.dwmmc",))
        part = os.path.join(self.root, "sys/devices/platform/ff540000.usb/usb2/2-1/host0/block/sda/sda2")
        write(os.path.join(part, "uevent"), "MAJOR=8\nMINOR=2\nDEVNAME=sda2\nDEVTYPE=partition\nPARTN=2\n"
                                            "PARTNAME=mpfree-test-log\n")
        os.makedirs(os.path.join(self.root, "sys/class/block"))
        os.symlink(os.path.relpath(part, os.path.join(self.root, "sys/class/block")),
                   os.path.join(self.root, "sys/class/block/sda2"))
        write(os.path.join(self.root, "proc/sys/kernel/random/boot_id"), "0123abcd-0000-4000-8000-00000000beef\n")
        dev = os.path.join(self.root, "dev/sda2")
        self.disk = os.path.join(self.tmp, "log-partition")
        os.makedirs(self.disk)
        if os.geteuid() == 0:
            subprocess.run(["mknod", dev, "b", "7", "250"], check=True)
        else:
            self.skipTest("needs root to make a block device node")
        write(os.path.join(self.bin, "mount"), '#!/bin/sh\necho "mount: $*" >> "$FAKE_LOG"\n'
                                               'for d; do :; done\nrmdir "$d" && ln -s "$FAKE_DISK" "$d"\n', 0o755)
        write(os.path.join(self.bin, "umount"), '#!/bin/sh\necho "umount: $*" >> "$FAKE_LOG"\n', 0o755)
        write(os.path.join(self.bin, "dmesg"), '#!/bin/sh\necho "[ 7.5] fake kernel log"\n', 0o755)
        for tool in ("grep", "mkdir", "rmdir", "ln"):
            os.symlink(shutil.which(tool), os.path.join(self.bin, tool))
        script = stick_module().INTERNAL_OFF_SCRIPT.replace("R=''", f"R='{self.root}'", 1)
        res = subprocess.run([self.dash, "-c", script], env={"PATH": self.bin, "FAKE_LOG": self.log,
                                                             "FAKE_DISK": self.disk},
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=30)
        self.assertEqual(res.returncode, 1, res.stdout)
        with open(self.log) as fh:
            calls = [l for l in fh.read().splitlines() if l.startswith(("mount", "umount"))]
        mnt = os.path.join(self.root, "run/mpfree-internal-off-log")
        self.assertEqual(calls, [f"mount: -t ext4 -o rw,nosuid,nodev,noexec,sync {dev} {mnt}", f"umount: {mnt}"])
        with open(os.path.join(self.disk, "boot-0123abcd-0000-4000-8000-00000000beef/internal-off.txt")) as fh:
            saved = fh.read()
        self.assertIn("STOP: ff0f0000.dwmmc still has its driver; powering off", saved)
        self.assertIn("fake kernel log", saved)

    def test_powers_off_if_a_controller_stays(self):
        self.kernel = FakeKernel(self.root, ignore=("ff0c0000.dwmmc",))
        res = self.run_script()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("STOP: ff0c0000.dwmmc still has its driver; powering off", self.notes())
        self.assertIn("(after kexec)", self.notes())

    def test_nothing_to_do_after_a_kexec_jump(self):
        os.makedirs(os.path.join(self.root, "sys/block/sda"))
        for rel, text in (("proc/uptime", "3.00 1.00\n"), ("dev/kmsg", "")):
            write(os.path.join(self.root, rel), text)
        os.makedirs(os.path.join(self.root, "run"))
        res = self.run_script()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("(after kexec)", self.notes())
        self.assertNotIn("switched off", self.notes())

    def test_script_writes_only_ram_sysfs_unbind_and_dev_links(self):
        code = "\n".join(l for l in stick_module().INTERNAL_OFF_SCRIPT.splitlines() if not l.lstrip().startswith("#"))
        for forbidden in ("mkfs", "dd ", "fw_setenv", "/data", "> /dev/mmc", "wipefs", "mmcblk*p", "blockdev --setrw"):
            self.assertNotIn(forbidden, code)
        # One mount only: the stick's own log partition, found by its GPT name on USB, to keep the notes of a stop.
        self.assertEqual(code.count("mount -t"), 1)
        self.assertIn('mount -t ext4 -o rw,nosuid,nodev,noexec,sync "$R/dev/$log" "$m"', code)
        self.assertIn("grep -qx 'PARTNAME=mpfree-test-log'", code)
        self.assertIn("*/usb*) log=", code)
        writes = [l.strip() for l in code.splitlines() if " > " in l or ">>" in l]
        for line in writes:
            self.assertRegex(line, r'(> /dev/null|> "\$R/dev/kmsg"|>> "\$R/run/mpfree-internal-off.txt"|'
                                   r'> "\$dev/driver/unbind"|> "\$d/internal-off.txt")', line)



CHAIN_UUID = "1a2b3c4d-5e6f-4071-8293-a4b5c6d7e8f9"
CHAIN_NOW = (f"root=PARTUUID={CHAIN_UUID} rootwait=60 ro panic=10 rfkill.default_state=0 loadpin.enforce=0 "
             "mpfree.chain=1 isolcpus=2-3")
CHAIN_NEXT = f"root=PARTUUID={CHAIN_UUID} rootwait=60 ro panic=10 rfkill.default_state=0 sysctl.kernel.kexec_load_disabled=1"
# Akai's kexec on the kernel the hook started: LoadPin is not enforcing, so the ordinary load is let through.
CHAIN_KEXEC = r"""#!/bin/sh
echo "kexec: $*" >> "$FAKE_LOG"
case "$1" in
  --load)
    [ -n "${FAKE_REFUSE:-}" ] && { echo "kexec_load failed: Operation not permitted" >&2; exit 1; }
    echo 1 > "$FAKE_ROOT/sys/kernel/kexec_loaded"
    [ -n "${FAKE_FAIL_LOADED:-}" ] && { echo "kexec: late error" >&2; exit 1; }
    exit 0 ;;
  --unload)
    [ -n "${FAKE_UNLOAD_FAIL:-}" ] && exit 1
    echo 0 > "$FAKE_ROOT/sys/kernel/kexec_loaded"; exit 0 ;;
esac
exit 2
"""


class ChainTests(unittest.TestCase):
    """chain.sh (--chain), on the kernel the hook started with mpfree.chain=1, against a fake root, under dash
    with the host's tools (ChainBusyBoxTests: the same under BusyBox's shell and tools)."""
    busybox = False

    def tool(self, name):
        return shutil.which(name)

    def setUp(self):
        if self.busybox:
            self.bb = shutil.which("busybox")
            if not self.bb:
                if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                    self.fail("busybox is missing")
                self.skipTest("busybox not installed")
            self.dash = None
            self.shell = [self.bb, "sh"]
        else:
            self.dash = shutil.which("dash")
            if not self.dash:
                self.skipTest("dash not installed")
            self.shell = [self.dash]
        self.tmp = tempfile.mkdtemp(prefix="mpfree-chain-test-")
        self.root = os.path.join(self.tmp, "root")
        self.bin = os.path.join(self.tmp, "bin")
        self.log = os.path.join(self.tmp, "log")
        os.makedirs(self.bin)
        write(os.path.join(self.bin, "systemctl"),
              '#!/bin/sh\necho "systemctl: $*" >> "$FAKE_LOG"\nexit "${FAKE_START_RC:-0}"\n', 0o755)
        write(os.path.join(self.bin, "dmesg"), '#!/bin/sh\necho "[ 7.5] fake kernel log"\n', 0o755)
        for tool in ("cat", "cut", "sed", "sha256sum", "wc", "tr", "grep", "readlink", "mkdir"):
            self.link(tool)
        r = self.root
        for rel, text in (("proc/cmdline", CHAIN_NOW + "\n"), ("proc/uptime", "7.40 2.00\n"), ("dev/kmsg", ""),
                          ("sys/kernel/kexec_loaded", "0\n"), ("proc/sys/kernel/kexec_load_disabled", "0\n"),
                          ("boot/zImage-" + NEW, ZIMAGE), ("boot/mpfree-kexec/kexec.dtb", b"FAKE-STICK-DTB"),
                          ("boot/mpfree-kexec/cmdline.txt", CHAIN_NEXT + "\n")):
            write(os.path.join(r, rel), text)
        os.symlink("../zImage-" + NEW, os.path.join(r, "boot/mpfree-kexec/zImage"))
        write(os.path.join(r, "usr/sbin/kexec"), CHAIN_KEXEC, 0o755)
        os.makedirs(os.path.join(r, "run"))
        self.sums()

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def link(self, tool):
        if self.busybox:
            write(os.path.join(self.bin, tool), f'#!/bin/sh\nexec {self.bb} {tool} "$@"\n', 0o755)
        else:
            os.symlink(shutil.which(tool), os.path.join(self.bin, tool))

    def sums(self):
        d = os.path.join(self.root, "boot/mpfree-kexec")
        write(os.path.join(d, "SHA256SUMS"), "".join(f"{sha(os.path.join(d, n))}  {n}\n"
                                                     for n in ("zImage", "kexec.dtb", "cmdline.txt")))

    def run_script(self, **env):
        script = stick_module().CHAIN_SCRIPT.replace("R=''", f"R='{self.root}'", 1)
        return subprocess.run([*self.shell, "-c", script],
                              env={"PATH": self.bin, "FAKE_LOG": self.log, "FAKE_ROOT": self.root, **env},
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=30)

    def calls(self, prefix):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as fh:
            return [l for l in fh.read().splitlines() if l.startswith(prefix)]

    def notes(self):
        with open(os.path.join(self.root, "run/mpfree-chain.txt")) as fh:
            return fh.read()

    def load_call(self):
        d = os.path.join(self.root, "boot/mpfree-kexec")
        return f"kexec: --load --kexec-syscall --dtb={d}/kexec.dtb --command-line={CHAIN_NEXT} {d}/zImage"

    def loaded(self):
        with open(os.path.join(self.root, "sys/kernel/kexec_loaded")) as fh:
            return fh.read()

    def test_loads_the_sticks_kernel_and_tree_and_restarts_into_them(self):
        res = self.run_script()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertEqual(self.calls("kexec: "), [self.load_call()])
        self.assertEqual(self.calls("systemctl: "), ["systemctl: --no-block --job-mode=replace-irreversibly start "
                                                     "kexec.target"])
        self.assertEqual(self.loaded(), "1\n")
        notes = self.notes()
        self.assertIn("mpfree-chain: started at 7.40 s\n", notes)
        self.assertIn("mpfree-chain: the stick's kernel and device tree are loaded at 7.40 s; restart into them "
                      "(kexec.target)\n", notes)
        self.assertNotIn("instead", notes)

    def test_nothing_without_the_chain_word(self):
        for cmdline in (CHAIN_NEXT, CHAIN_NOW.replace("mpfree.chain=1", "mpfree.chain=10")):
            with self.subTest(cmdline):
                write(os.path.join(self.root, "proc/cmdline"), cmdline + "\n")
                res = self.run_script()
                self.assertEqual(res.returncode, 0, res.stdout)
                self.assertEqual(self.calls("kexec: ") + self.calls("systemctl: "), [])
                self.assertFalse(os.path.exists(os.path.join(self.root, "run/mpfree-chain.txt")))

    def test_any_problem_leaves_mpfree_on_the_first_kernel(self):
        d = os.path.join(self.root, "boot/mpfree-kexec")
        tail = "; MPFree starts on this kernel, with Akai's device tree\n"

        def check(note, loads=0, unloads=0, **env):
            for f in (self.log, os.path.join(self.root, "run/mpfree-chain.txt")):
                if os.path.exists(f):
                    os.unlink(f)
            res = self.run_script(**env)
            self.assertEqual(res.returncode, 0, res.stdout)
            self.assertIn("mpfree-chain: " + note + tail, self.notes())
            self.assertEqual(len(self.calls("kexec: --load")), loads)
            self.assertEqual(self.calls("kexec: --unload"), ["kexec: --unload --kexec-syscall"] * unloads)
            self.assertEqual(self.calls("systemctl: ")[:1] if not env.get("FAKE_START_RC") else [], [])
            self.assertEqual(self.loaded(), "0\n", "nothing left loaded")

        with self.subTest("the kernel refuses the load"):
            check("the kernel refused the load (kexec_load failed: Operation not permitted)", 1, FAKE_REFUSE="1")
        with self.subTest("a late failure leaves it loaded: unloaded again"):
            check("the kernel refused the load (kexec: late error)", 1, 1, FAKE_FAIL_LOADED="1")
        with self.subTest("systemd will not start kexec.target"):
            check("systemctl could not start kexec.target", 1, 1, FAKE_START_RC="1")
        with self.subTest("kexec is off on this kernel"):
            write(os.path.join(self.root, "proc/sys/kernel/kexec_load_disabled"), "1\n")
            check("kexec is turned off on this kernel")
            write(os.path.join(self.root, "proc/sys/kernel/kexec_load_disabled"), "0\n")
        with self.subTest("a kernel is loaded already"):
            write(os.path.join(self.root, "sys/kernel/kexec_loaded"), "1\n")
            os.unlink(self.log) if os.path.exists(self.log) else None
            res = self.run_script()
            self.assertIn("a kexec kernel is loaded already", self.notes())
            self.assertEqual(self.calls("kexec: "), [], "someone else's kernel is left alone")
            write(os.path.join(self.root, "sys/kernel/kexec_loaded"), "0\n")
        with self.subTest("the device tree does not match its SHA256SUMS"):
            write(os.path.join(d, "kexec.dtb"), b"DAMAGED")
            check("/boot/mpfree-kexec/kexec.dtb does not match its SHA256SUMS")
            write(os.path.join(d, "kexec.dtb"), b"FAKE-STICK-DTB")
        with self.subTest("the kernel is not listed"):
            with open(os.path.join(d, "SHA256SUMS")) as fh:
                lines = fh.read().splitlines(True)
            write(os.path.join(d, "SHA256SUMS"), "".join(l for l in lines if not l.endswith("  zImage\n")))
            check("/boot/mpfree-kexec/zImage does not match its SHA256SUMS")
            self.sums()
        for name, cmdline, note in (
                ("another partition", CHAIN_NEXT.replace(CHAIN_UUID, "99999999-2222-3333-4444-555555555555"),
                 "cmdline.txt does not start the system on this partition"),
                ("a second root=", CHAIN_NEXT + " root=/dev/mmcblk0p6",
                 "cmdline.txt does not start the system on this partition"),
                ("kexec left on", CHAIN_NEXT.replace(" sysctl.kernel.kexec_load_disabled=1", ""),
                 "cmdline.txt does not turn kexec off"),
                ("the chain again", CHAIN_NEXT + " mpfree.chain=1", "cmdline.txt asks for the chain again")):
            with self.subTest(name):
                write(os.path.join(d, "cmdline.txt"), cmdline + "\n")
                self.sums()
                check(note)
        write(os.path.join(d, "cmdline.txt"), CHAIN_NEXT + "\n")
        self.sums()
        with self.subTest("no kexec program"):
            os.unlink(os.path.join(self.root, "usr/sbin/kexec"))
            check("this system has no /usr/sbin/kexec")

    def test_a_kernel_that_will_not_unload_is_noted(self):
        res = self.run_script(FAKE_START_RC="1", FAKE_UNLOAD_FAIL="1")
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("could not unload the kernel loaded here (it stays in RAM until the MPC is switched off)",
                      self.notes())

    def test_keeps_its_notes_on_the_sticks_log_partition(self):
        part = os.path.join(self.root, "sys/devices/platform/ff540000.usb/usb2/2-1/host0/block/sda/sda2")
        write(os.path.join(part, "uevent"), "MAJOR=8\nMINOR=2\nDEVNAME=sda2\nDEVTYPE=partition\nPARTN=2\n"
                                            "PARTNAME=mpfree-test-log\n")
        os.makedirs(os.path.join(self.root, "sys/class/block"))
        os.symlink(os.path.relpath(part, os.path.join(self.root, "sys/class/block")),
                   os.path.join(self.root, "sys/class/block/sda2"))
        write(os.path.join(self.root, "proc/sys/kernel/random/boot_id"), "0123abcd-0000-4000-8000-00000000c4a1\n")
        dev = os.path.join(self.root, "dev/sda2")
        disk = os.path.join(self.tmp, "log-partition")
        os.makedirs(disk)
        if os.geteuid() == 0:
            subprocess.run(["mknod", dev, "b", "7", "250"], check=True)
        else:
            self.skipTest("needs root to make a block device node")
        write(os.path.join(self.bin, "mount"), '#!/bin/sh\necho "mount: $*" >> "$FAKE_LOG"\n'
                                               'for d; do :; done\nrmdir "$d" && ln -s "$FAKE_DISK" "$d"\n', 0o755)
        write(os.path.join(self.bin, "umount"), '#!/bin/sh\necho "umount: $*" >> "$FAKE_LOG"\n', 0o755)
        for tool in ("rmdir", "ln"):
            self.link(tool)
        res = self.run_script(FAKE_DISK=disk)
        self.assertEqual(res.returncode, 0, res.stdout)
        mnt = os.path.join(self.root, "run/mpfree-chain-log")
        self.assertEqual(self.calls("mount") + self.calls("umount"),
                         [f"mount: -t ext4 -o rw,nosuid,nodev,noexec,sync {dev} {mnt}", f"umount: {mnt}"])
        with open(self.log) as fh:
            order = [l.split(":")[0] for l in fh.read().splitlines()]
        self.assertLess(order.index("umount"), order.index("systemctl"), "the notes are kept before the restart")
        with open(os.path.join(disk, "boot-0123abcd-0000-4000-8000-00000000c4a1/chain.txt")) as fh:
            saved = fh.read()
        self.assertIn("restart into them (kexec.target)", saved)
        self.assertIn("fake kernel log", saved)

    def test_script_and_unit(self):
        mod = stick_module()
        code = "\n".join(l for l in mod.CHAIN_SCRIPT.splitlines() if not l.lstrip().startswith("#"))
        for forbidden in ("mkfs", "dd ", "fw_setenv", "/data", "mmcblk", "--kexec-file-syscall", "kexec -e",
                          "systemctl kexec", "--exec"):
            self.assertNotIn(forbidden, code)
        self.assertEqual(code.count("mount -t"), 1, "only the stick's own log partition")
        self.assertEqual(code.count('"$K" --load'), 1)
        unit = mod.CHAIN_UNIT
        for line in ("DefaultDependencies=no", "ConditionKernelCommandLine=mpfree.chain=1",
                     "Requires=mpfree-test-internal-off.service", "After=mpfree-test-internal-off.service",
                     "Type=oneshot", "ExecStart=/bin/sh /etc/mpfree-test/chain.sh", "WantedBy=sysinit.target"):
            self.assertIn(line + "\n", unit)
        self.assertNotIn("FailureAction", unit, "a failed chain leaves MPFree on the first kernel")
        before = " ".join(l[len("Before="):] for l in unit.splitlines() if l.startswith("Before=")).split()
        # Before systemd-sysctl: the stick's sysctl file turns kexec off. Before the log service: both mount
        # the log partition.
        for u in ("systemd-sysctl.service", "mpfree-test-logs.service", "mpfree-test-app.service",
                  "mpfree-test-storage.service", "data.mount", "sysinit.target"):
            self.assertIn(u, before)


class ChainBusyBoxTests(ChainTests):
    busybox = True

if __name__ == "__main__":
    unittest.main(verbosity=2)
