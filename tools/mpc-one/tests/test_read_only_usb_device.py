#!/usr/bin/env python3
"""Tests for collect-over-ssh.sh --usb-device against a fake MPC. No device, no network.

ip, ping and ssh are stand-ins, and the network identity is a test-only address
on a made-up interface. The fake ssh runs the real read-only-usb-device.sh under
BusyBox applets (as on the MPC) against a fake /proc, /sys, configfs, /lib/modules
and /etc tree, with the kernel log read from a file in that tree. It refuses
every remote command other than `sh -s`.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import glob
import gzip
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
REMOTE = os.path.join(TOOLS, "read-only-usb-device.sh")
SECRET_MAC = "02:11:22:33:44:55"
SECRET_SERIAL = "SN7Q4X9Z"
APPLETS = ("sh", "cat", "grep", "sed", "head", "tail", "cut", "basename", "readlink", "gzip")

FAKE_SSH = r'''#!/usr/bin/env python3
import os, subprocess, sys
args = sys.argv[1:]
command = args[-1]
with open(os.environ["FAKE_LOG"], "a") as fh:
    fh.write("ssh: " + command + "\n")
if command == "sh -s":
    env = dict(os.environ, PATH=os.environ["FAKE_REMOTE_PATH"])
    sys.exit(subprocess.run([os.path.join(os.environ["FAKE_REMOTE_PATH"], "sh"), "-s"], env=env).returncode)
print("fake ssh: command not allowed: " + command, file=sys.stderr)
sys.exit(255)
'''

CONFIG = """#
# Linux/arm 6.18.44 Kernel Configuration
#
CONFIG_CONFIGFS_FS=y
CONFIG_USB_DWC2=y
CONFIG_USB_DWC2_DUAL_ROLE=y
CONFIG_USB_GADGET=y
CONFIG_USB_LIBCOMPOSITE=m
CONFIG_USB_F_MIDI=m
CONFIG_USB_CONFIGFS=m
CONFIG_USB_CONFIGFS_F_MIDI=y
# CONFIG_USB_CONFIGFS_F_UAC2 is not set
CONFIG_SND_RAWMIDI=y
CONFIG_NOT_SELECTED_FAKE=y
"""

KERNEL_LOG = f"""[    0.000000] Booting Linux on physical CPU 0x500
[    1.100000] dwc2 ff580000.usb: DWC OTG Controller
[    1.200000] configfs-gadget.g1 gadget.0: SerialNumber: {SECRET_SERIAL}
[    1.300000] dwc2 ff580000.usb: bound driver configfs-gadget.g1
[    1.400000] usb_f_ecm gadget.0: HOST MAC {SECRET_MAC}
[    2.000000] rk_gmac-dwmac ff290000.ethernet end0: Link is Up
"""

STARTUP = f"""#!/bin/sh
# Sets up the computer port (a made-up stand-in for Akai's script).
G=/sys/kernel/config/usb_gadget/g1
mkdir -p $G/functions/midi.usb0
echo 0x09e8 > $G/idVendor
echo {SECRET_SERIAL} > $G/strings/0x409/serialnumber
ln -s $G/functions/midi.usb0 $G/configs/c.1/
echo ff580000.usb > $G/UDC
"""


def link(target, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    os.symlink(target, path)


def fake_root(root, gadgets=True):
    def put(rel, text):
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        write(path, text)
    put("kernel-log", KERNEL_LOG)
    put("proc/sys/kernel/osrelease", "6.18.44-fake\n")
    put("proc/sys/kernel/version", "#1 SMP PREEMPT_RT fake\n")
    put("proc/uptime", "321.00 900.00\n")
    udc = "sys/class/udc/ff580000.usb"
    for name, text in (("state", "configured"), ("current_speed", "high-speed"),
                       ("maximum_speed", "high-speed"), ("is_otg", "0"), ("function", "g1")):
        put(f"{udc}/{name}", text + "\n")
    put("sys/bus/platform/drivers/dwc2/.keep", "")
    link("../../../bus/platform/drivers/dwc2", os.path.join(root, udc, "device/driver"))
    put("sys/class/usb_role/ff580000.usb-role-switch/role", "device\n")
    with gzip.open(os.path.join(root, "proc/config.gz"), "wt") as fh:
        fh.write(CONFIG)
    put("proc/modules", "usb_f_midi 20480 1 - Live 0x00000000\n"
                        "libcomposite 61440 2 usb_f_midi, Live 0x00000000\n"
                        "snd_usb_audio 270336 0 - Live 0x00000000\n")
    put("lib/modules/6.18.44-fake/kernel/drivers/usb/gadget/libcomposite.ko", "")
    put("lib/modules/6.18.44-fake/kernel/drivers/usb/gadget/function/usb_f_midi.ko", "")
    put("proc/filesystems", "nodev\tsysfs\nnodev\tconfigfs\nnodev\tfunctionfs\n")
    put("proc/mounts", "sysfs /sys sysfs rw 0 0\nconfigfs /sys/kernel/config configfs rw 0 0\n")
    put("proc/asound/cards", " 0 [MPCOne        ]: USB-Audio - MPC One\n"
                             " 1 [Gadget        ]: f_midi - f_midi\n                      f_midi\n")
    put("proc/123/comm", "MPC\n")
    link("/dev/snd/midiC1D0", os.path.join(root, "proc/123/fd/5"))
    link("/dev/null", os.path.join(root, "proc/123/fd/6"))
    put("proc/456/comm", "adbd\n")
    link("/dev/usb-ffs/adb/ep0", os.path.join(root, "proc/456/fd/3"))
    put("proc/789/comm", "sshd\n")
    link("socket:[12345]", os.path.join(root, "proc/789/fd/3"))
    put("etc/init.d/S50computer-port", STARTUP)
    put("etc/unrelated.conf", "nothing about USB here\n")
    os.makedirs(os.path.join(root, "usr/bin"), exist_ok=True)
    write(os.path.join(root, "usr/bin/MPC"), b"\x7fELF\x00\x00/sys/kernel/config/usb_gadget\x00")
    if not gadgets:
        return
    g = "sys/kernel/config/usb_gadget/g1"
    for name, text in (("UDC", "ff580000.usb"), ("idVendor", "0x09e8"), ("idProduct", "0x00ff"),
                       ("bcdUSB", "0x0200"), ("bDeviceClass", "0x00")):
        put(f"{g}/{name}", text + "\n")
    put(f"{g}/strings/0x409/manufacturer", "Akai Fake\n")
    put(f"{g}/strings/0x409/product", "MPC Fake\n")
    put(f"{g}/strings/0x409/serialnumber", SECRET_SERIAL + "\n")
    put(f"{g}/configs/c.1/MaxPower", "500\n")
    put(f"{g}/configs/c.1/strings/0x409/configuration", "MIDI\n")
    for name, text in (("id", "Gadget"), ("index", "-1"), ("in_ports", "2"), ("out_ports", "2"),
                       ("buflen", "512"), ("qlen", "32")):
        put(f"{g}/functions/midi.usb0/{name}", text + "\n")
    put(f"{g}/functions/ecm.usb0/dev_addr", SECRET_MAC + "\n")
    put(f"{g}/functions/ecm.usb0/host_addr", SECRET_MAC + "\n")
    link("../../../../usb_gadget/g1/functions/midi.usb0", os.path.join(root, g, "configs/c.1/midi.usb0"))
    put("sys/kernel/config/usb_gadget/spare/UDC", "\n")


class UsbDeviceSurveyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-usb-device-test-")
        self.reports = os.path.join(self.tmp, "research")
        self.log = os.path.join(self.tmp, "commands.log")
        self.root = os.path.join(self.tmp, "root")
        fakebin, self.remote = os.path.join(self.tmp, "fakebin"), os.path.join(self.tmp, "remote-bin")
        for d in (fakebin, self.remote):
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
            "MPC_KEY": self.key, "MPC_REPORT_DIR": self.reports,
            "MPC_IP": TEST_IP, "MPC_IFACE": TEST_IFACE, "MPC_HOST_IP_CIDR": TEST_HOST, "MPC_EXPECT_MAC": TEST_MAC,
            "FAKE_LOG": self.log, "FAKE_REMOTE_PATH": self.remote, "MPFREE_FAKE_ROOT": self.root,
        }

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_wrapper(self):
        result = subprocess.run(["bash", WRAPPER, "--usb-device"], env=self.env, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, timeout=120)
        reports = glob.glob(os.path.join(self.reports, "mpc-one-usb-device-*.txt"))
        self.assertEqual(len(reports), 1, result.stdout)
        with open(reports[0]) as fh:
            return result, fh.read()

    def calls(self):
        with open(self.log) as fh:
            return fh.read().splitlines()

    def test_report(self):
        fake_root(self.root)
        result, report = self.run_wrapper()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("Collecting passive/read-only report", result.stdout)
        for text in ("release: 6.18.44-fake", "ff580000.usb\n  state: configured", "current_speed: high-speed",
                     "  function: g1", "  driver: dwc2", "ff580000.usb-role-switch\n  role: device",
                     "gadget: g1\n  bound to: ff580000.usb", "idVendor: 0x09e8", "idProduct: 0x00ff",
                     "strings 0x409 manufacturer: Akai Fake", "strings 0x409 product: MPC Fake",
                     "  config: c.1", "MaxPower: 500", "configuration: MIDI", "    uses function: midi.usb0",
                     "  function: midi.usb0", "in_ports: 2", "out_ports: 2", "  function: ecm.usb0",
                     "gadget: spare\n  bound to: (nothing", "CONFIG_USB_CONFIGFS_F_MIDI=y",
                     "# CONFIG_USB_CONFIGFS_F_UAC2 is not set", "CONFIG_USB_DWC2_DUAL_ROLE=y",
                     "usb_f_midi 1 -", "libcomposite 2 usb_f_midi,",
                     "on disk: /lib/modules/6.18.44-fake/kernel/drivers/usb/gadget/function/usb_f_midi.ko",
                     "nodev\tconfigfs", "configfs /sys/kernel/config configfs", "f_midi - f_midi",
                     "pid 123 (MPC): /dev/snd/midiC1D0", "pid 456 (adbd): /dev/usb-ffs/adb/ep0",
                     "--- /etc/init.d/S50computer-port ---", "echo ff580000.usb > $G/UDC",
                     "--- /usr/bin/MPC (a program; not printed) ---",
                     "bound driver configfs-gadget.g1", "HOST MAC <MAC>", "SerialNumber: <serial>",
                     "END OF USB-B SURVEY"):
            self.assertIn(text, report)
        self.assertNotIn(SECRET_SERIAL, report)
        self.assertNotIn(SECRET_MAC, report)
        self.assertNotIn("CONFIG_NOT_SELECTED_FAKE", report)
        self.assertNotIn("snd_usb_audio", report)
        self.assertNotIn("unrelated.conf", report)
        self.assertNotIn("/dev/null", report)
        self.assertNotIn("sshd", report)
        self.assertNotIn("Link is Up", report)
        self.assertNotIn("dev_addr", report)
        self.assertEqual([c for c in self.calls() if c.startswith("ssh:")], ["ssh: sh -s"])

    def test_without_gadgets_or_configfs(self):
        fake_root(self.root, gadgets=False)
        result, report = self.run_wrapper()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("(no usb_gadget directory", report)
        self.assertIn("END OF USB-B SURVEY", report)

    def test_an_empty_system(self):
        os.makedirs(os.path.join(self.root, "proc"))
        result, report = self.run_wrapper()
        self.assertEqual(result.returncode, 0, result.stdout)
        for text in ("(no USB device controller registered)", "(no usb_gadget directory",
                     "(/proc/config.gz is not readable here)", "(no /proc/asound/cards)", "END OF USB-B SURVEY"):
            self.assertIn(text, report)


class StaticTests(unittest.TestCase):
    def test_script_only_reads(self):
        with open(REMOTE) as fh:
            lines = [line for line in fh.read().splitlines() if not line.lstrip().startswith("#")]
        # Text in single quotes is printed or is a grep pattern, never a command.
        code = "\n".join(re.sub(r"'[^']*'", "''", line) for line in lines)
        for line in lines:
            stripped = re.sub(r"'[^']*'", "''", line).replace("2>/dev/null", "").replace("2>&1", "").replace(">/dev/null", "")
            self.assertNotIn(">", stripped, f"the script redirects output: {line}")
        for forbidden in (r"\bmount\b", r"\bumount\b", r"\bip\s", r"\bsystemctl\b", r"\becho\b", r"\btee\b",
                          r"\bdd\b", r"\bdevmem\b", r"\brm\b", r"\bmv\b", r"\bcp\b", r"\bln\b", r"\bmkdir\b",
                          r"\brmdir\b", r"\btouch\b", r"\bmodprobe\b", r"\binsmod\b", r"\brmmod\b", r"\bsysctl\b",
                          r"/bind\b", r"/unbind\b", r"dmesg\s+-", r"/dev/mem", r"\bamidi\b", r"\baplay\b",
                          r"\barecord\b", r"serialnumber", r"dev_addr", r"host_addr"):
            self.assertIsNone(re.search(forbidden, code), forbidden)

    def test_the_hardware_page_names_the_mode(self):
        # The design page lists the mode, so the round's owner finds it.
        doc = os.path.join(TOOLS, "..", "..", "docs", "dev", "hardware", "usb-b-midi.md")
        with open(doc) as fh:
            self.assertIn("--usb-device", fh.read())


if __name__ == "__main__":
    unittest.main(verbosity=2)
