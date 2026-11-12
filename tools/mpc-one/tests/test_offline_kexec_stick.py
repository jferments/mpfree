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
    if phy:
        gmac = tree.find("/ethernet@ff290000")
        gmac.set_prop("phy-handle", cells(0x4B))
        gmac.children.append(node("mdio", [], [node("ethernet-phy@1", [
            ("compatible", dt_strings("ethernet-phy-ieee802.3-c22")), ("reg", cells(1)),
            ("reset-gpios", cells(0x18, 8, 1)), ("phandle", cells(0x4B))])]))
        tree.root.children.append(node("mipi@ff960000", [], [node("panel@0", [("rotation", cells(270))])]))
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

        for unit in ("acvs.service", "touch-fw-update.service", "az0x-data-mkfs.service", "iptables.service"):
            self.assertIn('Fast link dest: "/dev/null"', self.dfs(root_img, f"stat /etc/systemd/system/{unit}"))
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
        self.assertEqual(table[3]["start"], table[2]["start"] + table[2]["size"])
        self.assertEqual(table[3]["start"] % 2048, 0)
        self.assertEqual(table[3]["size"], (64 << 20) // 512)
        self.assertEqual(os.path.getsize(stick), (table[3]["start"] + table[3]["size"] + 2048) * 512)

        # Partition 4: an empty journaled ext4, labelled for write-test-stick.py to recognise.
        data_img = os.path.join(self.tmp, "p4.img")
        self.partition(stick, 4, data_img)
        self.assertEqual(run(["e2fsck", "-fn", data_img]).returncode, 0)
        stats = self.dfs(data_img, "stats")
        self.assertIn("has_journal", re.search(r"^Filesystem features:(.*)$", stats, re.M).group(1))
        self.assertRegex(stats, r"Filesystem volume name:\s+MPFREE-DATA\n")
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
                     "OnFailure=mpfree-test-screen.service", "ExecStart=/bin/sh /etc/mpfree-test/start-app.sh"):
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
                                                  data_uuid="00000000-0000-0000-0000-000000000001", keep="")):
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
        for line in lines:
            for target in re.findall(r">\s*(\S+)", line):
                self.assertEqual(target, "/dev/kmsg", line)
        for forbidden in ("/dev/mmc", "/dev/sd", "remount"):
            self.assertNotIn(forbidden, tool.APP_SCRIPT)

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


if __name__ == "__main__":
    unittest.main(verbosity=2)
