#!/usr/bin/env python3
"""Tests for collect-over-ssh.sh --ethernet against a fake MPC. No device, no network.

ip, ping and ssh are stand-ins, and the network identity is a test-only address
on a made-up interface. The fake ssh runs the real read-only-ethernet.sh under
BusyBox applets (as on the MPC) against a fake /proc, /sys and debugfs tree,
with the kernel log read from a file in that tree and a stand-in for ethtool
that logs how it was called. It refuses
every remote command other than `sh -s`.

The same script is copied onto the kexec test build's stick, so these tests
cover both uses.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import glob
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
REMOTE = os.path.join(TOOLS, "read-only-ethernet.sh")
SECRET_MAC = "02:11:22:33:44:55"
APPLETS = ("sh", "cat", "grep", "sed", "head", "cut", "basename", "readlink")

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

KERNEL_LOG = """[    0.224777] rk_gmac-dwmac ff290000.ethernet: clock input from PHY
[    0.230000] rk_gmac-dwmac ff290000.ethernet end0: device MAC address 02:11:22:33:44:55
[    3.100000] rk_gmac-dwmac ff290000.ethernet end0: PHY [stmmac-0:01] driver [RTL8211E Gigabit Ethernet] (irq=54)
[    3.200000] usb 1-1: new high-speed USB device
"""

FAKE_ETHTOOL = r'''#!/bin/sh
echo "ethtool: $*" >> "$FAKE_LOG"
case "$*" in
  "-i end0") echo "driver: st_gmac" ;;
  "end0") echo "Settings for end0:"; echo "	Speed: 1000Mb/s"; echo "	Permanent address: 02:11:22:33:44:55" ;;
  *) echo "ethtool stand-in: refusing $*" >&2; exit 99 ;;
esac
'''

CLK_SUMMARY = """                                 enable  prepare  protect                                duty  hardware
   clock                          count    count    count        rate   accuracy phase  cycle    enable   consumer
---------------------------------------------------------------------------------------------
 ext_gmac                            2        2        0   125000000          0     0  50000      Y   deviceless
    mac_clk                          2        2        0   125000000          0     0  50000      Y   ff290000.ethernet
       sclk_mac_tx                   1        1        0   125000000          0     0  50000      Y   ff290000.ethernet
 xin24m                              5        5        0    24000000          0     0  50000      Y   deviceless
    aclk_peri                        3        3        0   148500000          0     0  50000      Y   deviceless
"""

PINMUX = """Pinmux settings per pin
pin 0 (gpio0-0): (MUX UNCLAIMED) (GPIO UNCLAIMED)
pin 111 (gpio3-15): ff290000.ethernet (GPIO UNCLAIMED) function gmac group phy-int
pin 128 (gpio4-0): ff290000.ethernet (GPIO UNCLAIMED) function gmac group rgmii-pins
pin 136 (gpio4-8): ff290000.ethernet gpio4:136 function gmac group phy-rst
pin 137 (gpio4-9): ff290000.ethernet (GPIO UNCLAIMED) function gmac group rgmii-pins
"""

PINCONF = """Pin config settings per pin
pin 111 (gpio3-15): input bias pull up (1 ohms)
pin 136 (gpio4-8): output drive strength (2 mA), output enabled, pin output (1 level)
pin 140 (gpio4-12): input bias pull down (1 ohms)
"""

GPIO = """gpiochip4: GPIOs 128-159, parent: platform/ff7b0000.gpio, gpio4:
 gpio-136 (                    |PHY reset           ) out hi ACTIVE LOW
"""


def link(target, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    os.symlink(target, path)


def fake_root(root, debugfs=True):
    def put(rel, text):
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        write(path, text)
    put("kernel-log", KERNEL_LOG)
    put("proc/sys/kernel/osrelease", "6.6.95-fake-rt55\n")
    put("proc/uptime", "123.45 400.00\n")
    put("proc/interrupts", " 54:   0  0  0  0  rockchip_gpio_irq 15 Level  stmmac-0:01\n"
                           " 61:   9  0  0  0  GIC-0  59 Level  end0\n 70:  1 0 0 0 GIC-0 70 Level ff540000.usb\n")
    mounts = "proc /proc proc rw 0 0\n"
    if debugfs:
        mounts += "debugfs /sys/kernel/debug debugfs rw,nosuid,nodev,noexec,relatime 0 0\n"
    put("proc/mounts", mounts)
    put("sys/class/net/lo/operstate", "unknown\n")
    put("sys/class/net/end0/operstate", "up\n")
    put("sys/class/net/end0/flags", "0x1003\n")
    put("sys/class/net/end0/carrier", "1\n")
    put("sys/class/net/end0/mtu", "1500\n")
    put("sys/class/net/end0/address", SECRET_MAC + "\n")
    put("sys/bus/platform/drivers/rk_gmac-dwmac/bind", "")
    link("../../../bus/platform/drivers/rk_gmac-dwmac", os.path.join(root, "sys/class/net/end0/device/driver"))
    phy = "sys/bus/mdio_bus/devices/stmmac-0:01"
    put(phy + "/phy_id", "0x001cc915\n")
    put(phy + "/phy_interface", "rgmii\n")
    put(phy + "/phy_has_fixups", "0\n")
    put("sys/bus/mdio_bus/drivers/RTL8211E Gigabit Ethernet/bind", "")
    link("../../drivers/RTL8211E Gigabit Ethernet", os.path.join(root, phy, "driver"))
    link("../../../../class/net/end0", os.path.join(root, phy, "attached_dev"))
    link("../../../bus/mdio_bus/devices/stmmac-0:01", os.path.join(root, "sys/class/net/end0/phydev"))
    for name, number in (("transfers", 812), ("errors", 0), ("reads", 700), ("writes", 112),
                         ("transfers_1", 812), ("errors_1", 0), ("reads_1", 700), ("writes_1", 112)):
        put(f"sys/class/mdio_bus/stmmac-0/statistics/{name}", f"{number}\n")
    for reg, name in (("regulator.12", "VCCIO_PMU"), ("regulator.3", "vcc_sys")):
        put(f"sys/class/regulator/{reg}/name", name + "\n")
        put(f"sys/class/regulator/{reg}/state", "enabled\n")
        put(f"sys/class/regulator/{reg}/microvolts", "3300000\n")
        put(f"sys/class/regulator/{reg}/num_users", "1\n")
    put("sys/kernel/debug/gpio", GPIO)
    put("sys/kernel/debug/clk/clk_summary", CLK_SUMMARY)
    put("sys/kernel/debug/pinctrl/pinctrl-rockchip-pinctrl/pinmux-pins", PINMUX)
    put("sys/kernel/debug/pinctrl/pinctrl-rockchip-pinctrl/pinconf-pins", PINCONF)
    put("sys/kernel/debug/devices_deferred", "")


class EthernetDetailsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.busybox = shutil.which("busybox")
        if not cls.busybox:
            if os.environ.get("MPFREE_REQUIRE_BUSYBOX") == "1":
                raise RuntimeError("BusyBox is required for these tests")
            raise unittest.SkipTest("BusyBox not installed")

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-ethernet-test-")
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
        write(os.path.join(self.remote, "ethtool"), FAKE_ETHTOOL, 0o755)
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
        result = subprocess.run(["bash", WRAPPER, "--ethernet"], env=self.env, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, timeout=120)
        reports = glob.glob(os.path.join(self.reports, "mpc-one-ethernet-*.txt"))
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
        for text in ("release: 6.6.95-fake-rt55", "uptime: 123.45 s", "end0\n  operstate: up", "flags: 0x1003",
                     "driver: rk_gmac-dwmac", "phy: stmmac-0:01", "phy_id: 0x001cc915",
                     "phy driver: RTL8211E Gigabit Ethernet", "stmmac-0\n  transfers: 812", "errors_1: 0",
                     "attached to: end0", "rockchip_gpio_irq 15 Level  stmmac-0:01", "GIC-0  59 Level  end0", "VCCIO_PMU (regulator.12)",
                     "mounted at /sys/kernel/debug", "gpio-136 (                    |PHY reset           ) out hi",
                     " ext_gmac ", "sclk_mac_tx", "aclk_peri", "pin 136 (gpio4-8): ff290000.ethernet gpio4:136",
                     "pin 111 (gpio3-15): input bias pull up", "clock input from PHY",
                     "driver [RTL8211E Gigabit Ethernet]", "device MAC address <MAC>", "--- ethtool end0 ---",
                     "Speed: 1000Mb/s", "Permanent address: <MAC>", "END OF ETHERNET DETAILS"):
            self.assertIn(text, report)
        self.assertNotIn(SECRET_MAC, report)
        self.assertNotIn("xin24m", report)
        self.assertNotIn("vcc_sys", report)
        self.assertNotIn("gpio0-0", report)
        self.assertNotIn("gpio4-12", report)
        self.assertNotIn("new high-speed USB", report)
        self.assertNotIn("ff540000.usb", report)
        self.assertNotIn("lo\n  operstate", report)
        self.assertEqual([c for c in self.calls() if c.startswith("ssh:")], ["ssh: sh -s"])
        self.assertEqual(sorted(c for c in self.calls() if not c.startswith("ssh:")),
                         ["ethtool: -i end0", "ethtool: end0"])

    def test_without_debugfs_or_ethtool(self):
        fake_root(self.root, debugfs=False)
        os.remove(os.path.join(self.remote, "ethtool"))
        result, report = self.run_wrapper()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("debugfs is not mounted; this script does not mount it", report)
        self.assertIn("ethtool is not installed", report)
        self.assertNotIn("PHY reset", report)
        self.assertNotIn("ext_gmac", report)

    def test_interface_without_a_phy(self):
        fake_root(self.root)
        os.remove(os.path.join(self.root, "sys/class/net/end0/phydev"))
        result, report = self.run_wrapper()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("phy: none attached", report)


class StaticTests(unittest.TestCase):
    def test_script_only_reads(self):
        with open(REMOTE) as fh:
            lines = [line for line in fh.read().splitlines() if not line.lstrip().startswith("#")]
        # Text in single quotes is printed or is a grep pattern, never a command.
        code = "\n".join(re.sub(r"'[^']*'", "''", line) for line in lines)
        for line in lines:
            stripped = re.sub(r"'[^']*'", "''", line).replace("2>/dev/null", "").replace("2>&1", "").replace(">/dev/null", "")
            self.assertNotIn(">", stripped, f"the script redirects output: {line}")
        for forbidden in (r"\bmount\b", r"\bumount\b", r"\bip\s", r"\bifconfig\b", r"\bsystemctl\b", r"\becho\b",
                          r"\btee\b", r"\bdd\b", r"\bdevmem\b", r"\brm\b", r"\bmv\b", r"\bcp\b", r"\bmkdir\b",
                          r"\bmodprobe\b", r"\binsmod\b", r"\brmmod\b", r"\bsysctl\b", r"/bind\b", r"/unbind\b",
                          r"ethtool\s+-[^i]", r"dmesg\s+-", r"/dev/mem"):
            self.assertIsNone(re.search(forbidden, code), forbidden)


if __name__ == "__main__":
    unittest.main(verbosity=2)
