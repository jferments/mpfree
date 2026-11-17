#!/usr/bin/env python3
"""Tests for collect-over-ssh.sh --drivers and --stick-drivers against a fake MPC. No device, no network.

ip, ping and ssh are stand-ins, and the network identity is a test-only address
on a made-up interface. The fake ssh records its options and runs the real
read-only-drivers.sh under BusyBox applets (as on the MPC) against a fake /proc
and /sys tree, with the kernel log read from a file in that tree. It refuses
every remote command other than `sh -s`.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import glob
import gzip
import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_kexec_preflight import FAKE_IP, TEST_HOST, TEST_IFACE, TEST_IP, TEST_MAC, write  # noqa: E402

TOOLS = os.path.dirname(HERE)
WRAPPER = os.path.join(TOOLS, "collect-over-ssh.sh")
REMOTE = os.path.join(TOOLS, "read-only-drivers.sh")
SURVEY = os.path.join(TOOLS, "offline-own-kernel-survey.py")
SECRET_MAC = "02:11:22:33:44:55"
SECRET_SERIAL = "SN7Q4X9Z"
SECRET_UUID = "0a1b2c3d-4e5f-4a6b-8c7d-0e1f2a3b4c5d"
SECRET_MBR_ID = "931ad49d-02"
APPLETS = ("sh", "cat", "grep", "sed", "head", "cut", "basename", "readlink", "tr", "gzip", "sha256sum")

FAKE_SSH = r'''#!/usr/bin/env python3
import os, subprocess, sys
args = sys.argv[1:]
command = args[-1]
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write("ssh: " + " ".join(args) + "\n")
if command == "sh -s":
    env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
    sys.exit(subprocess.run([os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh"), "-s"], env=env).returncode)
print("fake ssh: command not allowed: " + command, file=sys.stderr)
sys.exit(255)
'''

CONFIG = """#
# Automatically generated file; DO NOT EDIT.
# Linux/arm 6.18.44 Kernel Configuration
#
CONFIG_CC_VERSION_TEXT="arm-fake-gcc (GCC) 13.3.0"
CONFIG_LOCALVERSION="-az01-fake-rt6"
CONFIG_PREEMPT_RT=y
CONFIG_HZ_1000=y
CONFIG_MODULES=y
{sig}
CONFIG_MMC=y
CONFIG_MMC_DW_ROCKCHIP=y
CONFIG_SND_SOC_ROCKCHIP_I2S=y
CONFIG_SND_SOC_JP07_FAKE=y
CONFIG_TOUCHSCREEN_ILI210X=m
# CONFIG_DRM_PANEL_SIMPLE is not set
CONFIG_NOT_SELECTED_FAKE=y
"""
SIG = {"none": "# CONFIG_MODULE_SIG is not set",
       "checked": "CONFIG_MODULE_SIG=y\n# CONFIG_MODULE_SIG_FORCE is not set",
       "forced": "CONFIG_MODULE_SIG=y\nCONFIG_MODULE_SIG_FORCE=y"}

KERNEL_LOG = f"""[    0.000000] Booting Linux on physical CPU 0x500
[    0.350000] rk_gmac-dwmac ff290000.ethernet eth0: device MAC address {SECRET_MAC}
[    0.360000] rk_gmac-dwmac ff290000.ethernet eth0: no firmware for the PHY at {SECRET_MAC}
[    1.200000] usb 2-1: SerialNumber: {SECRET_SERIAL}, firmware request from a fake device
[    3.970000] LoadPin: sda1 (8:1): writable
[    4.100000] ili210x_i2c 0-0026: Direct firmware load for ilitek/ili2117.bin failed with error -2
[    5.000000] platform ff960000.dsi: deferred probe pending
[    7.670000] cfg80211: Loading compiled-in X.509 certificates for regulatory database
[    8.000000] random: crng init done
"""

INPUT_DEVICES = f"""I: Bus=0018 Vendor=0000 Product=0000 Version=0000
N: Name="ili210x_i2c"
P: Phys=0-0026/input0
S: Sysfs=/devices/platform/ff160000.i2c/i2c-0/0-0026/input/input0
U: Uniq={SECRET_SERIAL}
H: Handlers=mouse0 event0
B: PROP=2

"""


def link(target, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    os.symlink(target, path)


def fake_root(root, sig="none", config=True, debugfs=True):
    def put(rel, data):
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        write(path, data)

    def node(path, *compatible):
        put(f"sys/firmware/devicetree/base/{path}/compatible", ("\0".join(compatible) + "\0").encode())

    def device(bus, name, driver=None, of=None, module=None):
        d = f"sys/bus/{bus}/devices/{name}"
        os.makedirs(os.path.join(root, d), exist_ok=True)
        if driver:
            os.makedirs(os.path.join(root, f"sys/bus/{bus}/drivers/{driver}"), exist_ok=True)
            link(f"../../../../bus/{bus}/drivers/{driver}", os.path.join(root, d, "driver"))
            if module:
                os.makedirs(os.path.join(root, "sys/module", module), exist_ok=True)
                mod_link = os.path.join(root, f"sys/bus/{bus}/drivers/{driver}/module")
                if not os.path.islink(mod_link):
                    link(f"../../../../module/{module}", mod_link)
        if of:
            link(f"../../../../firmware/devicetree/base/{of}", os.path.join(root, d, "of_node"))
        return d

    put("kernel-log", KERNEL_LOG)
    put("proc/sys/kernel/osrelease", "6.18.44-az01-fake-rt6\n")
    put("proc/sys/kernel/version", "#1 SMP PREEMPT_RT Sun Aug 10 12:00:00 UTC 2026\n")
    put("proc/sys/kernel/loadpin/enforce", "1\n")
    put("proc/sys/kernel/kexec_load_disabled", "1\n")
    put("proc/uptime", "61.20 200.00\n")
    put("proc/cmdline", f"root=PARTUUID={SECRET_MBR_ID} rootwait ro androidboot.serialno={SECRET_SERIAL} "
                        f"mpfree.stick=PARTUUID={SECRET_UUID} isolcpus=2-3\n")
    put("sys/firmware/devicetree/base/model", b"Akai Professional MPC One\0")
    put("sys/firmware/devicetree/base/compatible", b"inmusic,acva\0inmusic,az01\0rockchip,rk3288\0")
    if config:
        put("proc/config.gz", gzip.compress(CONFIG.format(sig=SIG[sig]).encode()))
    node("i2s@ff890000", "rockchip,rk3288-i2s", "rockchip,rk3066-i2s")
    node("sound", "inmusic,acva-audio")
    node("dsi@ff960000/panel@0", "inmusic,fake-panel")
    node("backlight", "inmusic,mipi-backlight")
    device("platform", "ff890000.i2s", "rockchip-i2s", "i2s@ff890000")
    sound = device("platform", "sound", "jp07-audio", "sound", module="snd_soc_jp07")
    device("platform", "ff0f0000.mmc", None, None)
    device("platform", "backlight", "mipi-backlight", "backlight")
    device("mipi-dsi", "ff960000.dsi.0", "panel-fake", "dsi@ff960000/panel@0")
    usb = device("usb", "1-1", "usb")
    put(usb + "/idVendor", "09e8\n")
    put(usb + "/idProduct", "0046\n")
    put(usb + "/product", "MPC One\n")
    put(usb + "/serial", SECRET_SERIAL + "\n")
    os.makedirs(os.path.join(root, "sys/bus/spi/devices"))
    put("proc/modules", "snd_soc_jp07 16384 1 - Live 0xbf000000\ncfg80211 524288 0 - Live 0xbf100000\n")
    put("proc/asound/cards", " 0 [ACVA           ]: jp07-audio - ACVA\n                      ACVA\n")
    put("proc/asound/pcm", "00-00: ff890000.i2s-acva-codec acva-codec-0 : : playback 1 : capture 1\n")
    link("../../../../" + sound, os.path.join(root, "sys/class/sound/card0/device"))
    for name, text in (("status", "connected"), ("enabled", "enabled"), ("modes", "800x1280")):
        put(f"sys/class/drm/card0-DSI-1/{name}", text + "\n")
    os.makedirs(os.path.join(root, "sys/class/drm/card0"))
    for name, text in (("name", "rockchipdrmfb"), ("virtual_size", "800,1280"), ("bits_per_pixel", "32"),
                       ("rotate", "0")):
        put(f"sys/class/graphics/fb0/{name}", text + "\n")
    for name, text in (("type", "raw"), ("max_brightness", "255"), ("brightness", "200")):
        put(f"sys/class/backlight/backlight/{name}", text + "\n")
    link("../../../bus/platform/devices/backlight", os.path.join(root, "sys/class/backlight/backlight/device"))
    put("proc/bus/input/devices", INPUT_DEVICES)
    for name, text in (("name", "vcc_codec"), ("state", "enabled"), ("microvolts", "3300000"), ("num_users", "1")):
        put(f"sys/class/regulator/regulator.7/{name}", text + "\n")
    put("proc/interrupts", " 61:   9  0  0  0  GIC-0  59 Level  eth0\n 70:  1 0 0 0 GIC-0 85 Level  ff890000.i2s\n")
    mounts = "proc /proc proc rw 0 0\n"
    if debugfs:
        mounts += "debugfs /sys/kernel/debug debugfs rw,nosuid,nodev,noexec,relatime 0 0\n"
    put("proc/mounts", mounts)
    put("sys/kernel/debug/gpio", " gpio-180 (                    |acva-mute           ) out lo\n")
    put("sys/kernel/debug/devices_deferred", "ff960000.dsi\n")


class Base(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-drivers-test-")
        self.reports = os.path.join(self.tmp, "research")
        self.log = os.path.join(self.tmp, "commands.log")
        self.root = os.path.join(self.tmp, "root")
        self.work = os.path.join(self.tmp, "work")
        fakebin, self.remote = os.path.join(self.tmp, "fakebin"), os.path.join(self.tmp, "remote-bin")
        for d in (fakebin, self.remote, self.work):
            os.makedirs(d)
        write(os.path.join(fakebin, "ssh"), FAKE_SSH, 0o755)
        write(os.path.join(fakebin, "ip"), FAKE_IP, 0o755)
        write(os.path.join(fakebin, "ping"), "#!/bin/sh\nexit 0\n", 0o755)
        for applet in APPLETS:
            os.symlink(self.busybox, os.path.join(self.remote, applet))
        self.key = os.path.join(self.tmp, "key")
        write(self.key, "not a real key\n", 0o600)
        self.env = {
            "HOME": self.tmp, "LC_ALL": "C", "PATH": fakebin + os.pathsep + "/usr/bin:/bin",
            "MPC_KEY": self.key, "MPC_REPORT_DIR": self.reports, "MPFREE_WORK_DIR": self.work,
            "MPC_IP": TEST_IP, "MPC_IFACE": TEST_IFACE, "MPC_HOST_IP_CIDR": TEST_HOST, "MPC_EXPECT_MAC": TEST_MAC,
            "FAKE_LOG": self.log, "FAKE_REMOTE_PATH": self.remote, "MPFREE_FAKE_ROOT": self.root,
        }

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_wrapper(self, mode="--drivers", kind="mpc-one-drivers"):
        result = subprocess.run(["bash", WRAPPER, mode], env=self.env, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, timeout=120)
        reports = glob.glob(os.path.join(self.reports, f"{kind}-*.txt"))
        report = ""
        if reports:
            self.assertEqual(len(reports), 1, result.stdout)
            with open(reports[0]) as fh:
                report = fh.read()
        return result, report

    def ssh_calls(self):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as fh:
            return [line for line in fh.read().splitlines() if line.startswith("ssh:")]


class DriversTests(Base):
    def test_report(self):
        fake_root(self.root)
        result, report = self.run_wrapper()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("Collecting passive/read-only report", result.stdout)
        for text in ("MPFREE DRIVER SURVEY - READ ONLY", "release: 6.18.44-az01-fake-rt6",
                     "version: #1 SMP PREEMPT_RT", "uptime: 61.20 s",
                     "command line: root=PARTUUID=<uuid> rootwait ro androidboot.serialno=<serial> "
                     "mpfree.stick=PARTUUID=<uuid> isolcpus=2-3",
                     "model: Akai Professional MPC One", "compatible: inmusic,acva inmusic,az01 rockchip,rk3288",
                     "loadpin enforce: 1", "kexec_load_disabled: 1",
                     "signed modules: not checked (no CONFIG_MODULE_SIG)",
                     "lines: 17, built in (=y): 8, modules (=m): 1",
                     'CONFIG_CC_VERSION_TEXT="arm-fake-gcc (GCC) 13.3.0"', "CONFIG_PREEMPT_RT=y",
                     "CONFIG_MMC_DW_ROCKCHIP=y", "# CONFIG_DRM_PANEL_SIMPLE is not set",
                     "--- platform ---",
                     "ff890000.i2s  driver=rockchip-i2s  node=/i2s@ff890000  "
                     "compatible=rockchip,rk3288-i2s rockchip,rk3066-i2s",
                     "sound  driver=jp07-audio  module=snd_soc_jp07  node=/sound  compatible=inmusic,acva-audio",
                     "ff0f0000.mmc  driver=none\n",
                     "--- mipi-dsi ---\nff960000.dsi.0  driver=panel-fake  node=/dsi@ff960000/panel@0  "
                     "compatible=inmusic,fake-panel",
                     "1-1  driver=usb  id=09e8:0046  product=MPC One", "--- spi ---\n",
                     "snd_soc_jp07 16384 1 -", "cfg80211 524288 0 -",
                     "jp07-audio - ACVA", "playback 1 : capture 1",
                     "card0: device driver=jp07-audio node=/sound compatible=inmusic,acva-audio",
                     "card0-DSI-1  status=connected  enabled=enabled  mode=800x1280",
                     "fb0  name=rockchipdrmfb  virtual_size=800,1280  bits_per_pixel=32",
                     "backlight  type=raw  max_brightness=255  brightness=200  device driver=mipi-backlight  "
                     "node=/backlight  compatible=inmusic,mipi-backlight",
                     'N: Name="ili210x_i2c"', "H: Handlers=mouse0 event0",
                     "regulator.7  name=vcc_codec  state=enabled  microvolts=3300000",
                     "GIC-0 85 Level  ff890000.i2s", "|acva-mute           ) out lo",
                     "--- devices_deferred ---\nff960000.dsi",
                     "Direct firmware load for ilitek/ili2117.bin failed", "LoadPin: sda1 (8:1): writable",
                     "deferred probe pending", "no firmware for the PHY at <MAC>", "SerialNumber: <serial>",
                     "CONFIG_NOT_SELECTED_FAKE=y", "END OF DRIVER SURVEY"):
            self.assertIn(text, report)
        for secret in (SECRET_MAC, SECRET_SERIAL, SECRET_UUID, SECRET_MBR_ID):
            self.assertNotIn(secret, report)
        self.assertNotIn("crng init done", report)
        self.assertNotIn("Sysfs=", report)
        self.assertNotIn("Uniq=", report)
        # The selection comes before the whole config, which alone has the unselected symbol.
        self.assertEqual(report.count("CONFIG_NOT_SELECTED_FAKE=y"), 1)
        self.assertGreater(report.index("CONFIG_NOT_SELECTED_FAKE=y"), report.index("Build settings, whole"))
        calls = self.ssh_calls()
        self.assertEqual(len(calls), 1)
        self.assertTrue(calls[0].endswith(" sh -s"))
        self.assertIn(" -p 22 ", calls[0])

    def test_signed_modules(self):
        for sig, line in (("checked", "signed modules: checked but not required (CONFIG_MODULE_SIG=y, "
                                      "no CONFIG_MODULE_SIG_FORCE)"),
                          ("forced", "signed modules: REQUIRED (CONFIG_MODULE_SIG_FORCE=y)")):
            with self.subTest(sig=sig):
                shutil.rmtree(self.root, ignore_errors=True)
                shutil.rmtree(self.reports, ignore_errors=True)
                fake_root(self.root, sig=sig)
                result, report = self.run_wrapper()
                self.assertEqual(result.returncode, 0, result.stdout)
                self.assertIn(line, report)

    def test_without_config_or_debugfs(self):
        fake_root(self.root, config=False, debugfs=False)
        result, report = self.run_wrapper()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(report.count("(/proc/config.gz is not readable here)"), 2)
        self.assertIn("debugfs is not mounted; this script does not mount it", report)
        self.assertNotIn("acva-mute", report)
        self.assertIn("END OF DRIVER SURVEY", report)


class StickDriversTests(Base):
    def build(self, name, needs):
        folder = os.path.join(self.work, name)
        os.makedirs(folder)
        write(os.path.join(folder, needs), "x")
        write(os.path.join(folder, "known_hosts"), "mpfree-kexec-test ssh-ed25519 AAAAfake\n")
        return folder

    def test_uses_the_newest_builds_host_key_on_the_test_port(self):
        fake_root(self.root)
        self.build("kexec-stick-20261115T201107Z-abcdef", "stick.img")
        newest = self.build("creator-stick-20261116T120000Z", "report.txt")
        result, report = self.run_wrapper("--stick-drivers", "mpc-one-stick-drivers")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("On MPFree's system on the stick (port 2222)", result.stdout)
        self.assertIn("release: 6.18.44-az01-fake-rt6", report)
        calls = self.ssh_calls()
        self.assertEqual(len(calls), 1)
        for option in (" -p 2222 ", f" -o UserKnownHostsFile={newest}/known_hosts ",
                       " -o GlobalKnownHostsFile=/dev/null ", " -o HostKeyAlias=mpfree-kexec-test ",
                       " -o StrictHostKeyChecking=yes "):
            self.assertIn(option, calls[0])

    def test_stops_without_a_build(self):
        fake_root(self.root)
        result, report = self.run_wrapper("--stick-drivers", "mpc-one-stick-drivers")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no stick build under", result.stdout)
        self.assertEqual(self.ssh_calls(), [])
        self.assertEqual(report, "")


class StaticTests(unittest.TestCase):
    def test_script_only_reads(self):
        with open(REMOTE) as fh:
            lines = [line for line in fh.read().splitlines() if not line.lstrip().startswith("#")]
        # Text in single quotes is printed or is a grep pattern, never a command.
        code = "\n".join(re.sub(r"'[^']*'", "''", line) for line in lines)
        for line in lines:
            stripped = re.sub(r"'[^']*'", "''", line).replace("2>/dev/null", "").replace("2>&1", "") \
                .replace(">/dev/null", "")
            self.assertNotIn(">", stripped, f"the script redirects output: {line}")
        for forbidden in (r"\bmount\b", r"\bumount\b", r"\bip\s", r"\bsystemctl\b", r"\becho\b", r"\btee\b",
                          r"\bdd\b", r"\bdevmem\b", r"\brm\b", r"\bmv\b", r"\bcp\b", r"\bmkdir\b", r"\btouch\b",
                          r"\bmodprobe\b", r"\binsmod\b", r"\brmmod\b", r"\bsysctl\b", r"/bind\b", r"/unbind\b",
                          r"dmesg\s+-", r"/dev/mem", r"\bethtool\b", r"\bkexec\b"):
            self.assertIsNone(re.search(forbidden, code), forbidden)

    def test_same_selection_as_the_offline_survey(self):
        with open(REMOTE) as fh:
            shell = re.search(r"^SELECT='(.*)'$", fh.read(), re.M).group(1)
        spec = importlib.util.spec_from_file_location("offline_own_kernel_survey", SURVEY)
        survey = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(survey)
        self.assertEqual(shell, survey.SELECT.pattern)


if __name__ == "__main__":
    unittest.main(verbosity=2)
