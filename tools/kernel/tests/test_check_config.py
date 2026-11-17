#!/usr/bin/env python3
"""Tests for tools/kernel/check-config.py and MPFree's kernel settings (tools/kernel/mpfree.config)."""
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
KERNEL = os.path.dirname(HERE)
SCRIPT = os.path.join(KERNEL, "check-config.py")
FRAGMENT = os.path.join(KERNEL, "mpfree.config")

spec = importlib.util.spec_from_file_location("check_config", SCRIPT)
cc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cc)

GOOD = """\
CONFIG_LOCALVERSION="-mpfree"
# CONFIG_KEXEC is not set
CONFIG_MMC=m
CONFIG_MMC_DW=m
CONFIG_IKCONFIG_PROC=y
CONFIG_PREEMPT_RT=y
"""

FRAG = """\
# a comment
# --- a heading
CONFIG_LOCALVERSION="-mpfree"
# CONFIG_KEXEC is not set
CONFIG_MMC=m
CONFIG_PREEMPT_RT=y
"""


def run(fragment, config):
    with tempfile.TemporaryDirectory() as tmp:
        f = os.path.join(tmp, "f")
        c = os.path.join(tmp, "c")
        with open(f, "w") as fh:
            fh.write(fragment)
        with open(c, "w") as fh:
            fh.write(config)
        return subprocess.run([sys.executable, "-I", SCRIPT, f, c], capture_output=True, text=True)


class CheckTests(unittest.TestCase):
    def test_all_taken(self):
        r = run(FRAG, GOOD)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(r.stdout.strip(), "CONFIG-OK")

    def test_a_dropped_setting_fails(self):
        r = run(FRAG, GOOD.replace("CONFIG_PREEMPT_RT=y\n", "# CONFIG_PREEMPT_RT is not set\n"))
        self.assertEqual(r.returncode, 1)
        self.assertIn("CONFIG_PREEMPT_RT: wanted CONFIG_PREEMPT_RT=y, the build has unset", r.stdout)
        self.assertIn("CONFIG-FAILED: 1 problems", r.stdout)

    def test_a_changed_value_fails(self):
        r = run(FRAG, GOOD.replace('"-mpfree"', '"-other"'))
        self.assertEqual(r.returncode, 1)
        self.assertIn('wanted CONFIG_LOCALVERSION="-mpfree", the build has CONFIG_LOCALVERSION="-other"', r.stdout)

    def test_an_unset_that_came_back_fails(self):
        r = run(FRAG, GOOD.replace("# CONFIG_KEXEC is not set\n", "CONFIG_KEXEC=y\n"))
        self.assertEqual(r.returncode, 1)
        self.assertIn("CONFIG_KEXEC: wanted unset, the build has CONFIG_KEXEC=y", r.stdout)

    def test_mmc_built_in_fails_even_if_the_fragment_asks(self):
        frag = FRAG.replace("CONFIG_MMC=m", "CONFIG_MMC=y")
        r = run(frag, GOOD.replace("CONFIG_MMC=m", "CONFIG_MMC=y"))
        self.assertEqual(r.returncode, 1)
        self.assertIn("CONFIG_MMC: the MMC core must be a module or absent", r.stdout)
        self.assertNotIn("wanted CONFIG_MMC", r.stdout)

    def test_mmc_absent_passes_the_rule(self):
        frag = FRAG.replace("CONFIG_MMC=m\n", "")
        r = run(frag, GOOD.replace("CONFIG_MMC=m\nCONFIG_MMC_DW=m\n", "# CONFIG_MMC is not set\n"))
        self.assertEqual(r.returncode, 0, r.stdout)

    def test_signed_modules_required_fails(self):
        r = run(FRAG, GOOD + "CONFIG_MODULE_SIG=y\nCONFIG_MODULE_SIG_FORCE=y\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("CONFIG_MODULE_SIG_FORCE: modules must not be required to be signed", r.stdout)

    def test_no_config_in_proc_fails(self):
        r = run(FRAG, GOOD.replace("CONFIG_IKCONFIG_PROC=y\n", ""))
        self.assertEqual(r.returncode, 1)
        self.assertIn("CONFIG_IKCONFIG_PROC: the kernel must carry its config", r.stdout)

    def test_empty_config_fails(self):
        r = run(FRAG, "# nothing\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("the .config has no CONFIG_ lines", r.stdout)

    def test_usage_and_missing_file(self):
        r = subprocess.run([sys.executable, "-I", SCRIPT], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)
        r = subprocess.run([sys.executable, "-I", SCRIPT, "/nonexistent/f", "/nonexistent/c"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)
        self.assertIn("STOP:", r.stderr)


class FragmentTests(unittest.TestCase):
    """MPFree's own settings file keeps the rules the plans promise."""

    def setUp(self):
        with open(FRAGMENT, encoding="utf-8") as fh:
            self.text = fh.read()
        self.frag = cc.parse(self.text)

    def test_every_line_is_a_setting_a_comment_or_blank(self):
        for n, line in enumerate(self.text.splitlines(), 1):
            s = line.strip()
            ok = (not s or cc.SET.match(s) or cc.UNSET.match(s)
                  or (s.startswith("#") and not s.startswith("# CONFIG_")))
            self.assertTrue(ok, f"line {n}: {line!r}")

    def test_no_symbol_twice(self):
        seen = []
        for line in self.text.splitlines():
            m = cc.SET.match(line.strip()) or cc.UNSET.match(line.strip())
            if m:
                self.assertNotIn(m.group(1), seen)
                seen.append(m.group(1))

    def test_storage_drivers_are_modules(self):
        for sym in ("CONFIG_MMC", "CONFIG_MMC_BLOCK", "CONFIG_MMC_DW", "CONFIG_MMC_DW_ROCKCHIP"):
            self.assertEqual(self.frag.get(sym), "m", sym)

    def test_the_chain_rules(self):
        self.assertEqual(self.frag.get("CONFIG_PREEMPT_RT"), "y")
        self.assertEqual(self.frag.get("CONFIG_SECURITY_LOADPIN"), "y")
        self.assertEqual(self.frag.get("CONFIG_SECURITY_LOADPIN_ENFORCE"), "y")
        self.assertIn("CONFIG_KEXEC", self.frag)
        self.assertIsNone(self.frag["CONFIG_KEXEC"])
        self.assertIsNone(self.frag["CONFIG_MODULE_SIG"])
        self.assertEqual(self.frag.get("CONFIG_LOCALVERSION"), '"-mpfree"')

    def test_root_on_the_stick_is_built_in(self):
        for sym in ("CONFIG_USB_DWC2", "CONFIG_USB_STORAGE", "CONFIG_BLK_DEV_SD", "CONFIG_EXT4_FS",
                    "CONFIG_DEVTMPFS"):
            self.assertEqual(self.frag.get(sym), "y", sym)

    def test_the_board_as_akais_kernel_drives_it(self):
        # Blocks K1.1 and K1.2 (2026-11-17): the drivers Akai's kernel binds on the MPC One.
        for sym in ("CONFIG_REGULATOR_ACT8865", "CONFIG_REGULATOR_FAN53555", "CONFIG_RTC_DRV_HYM8563",
                    "CONFIG_PHY_ROCKCHIP_USB", "CONFIG_ROCKCHIP_PM_DOMAINS", "CONFIG_ROCKCHIP_IOMMU",
                    "CONFIG_POWER_RESET_GPIO_RESTART", "CONFIG_SYSCON_REBOOT_MODE", "CONFIG_PL330_DMA",
                    "CONFIG_PWM_ROCKCHIP", "CONFIG_CMDLINE_EXTEND", "CONFIG_PANIC_ON_OOPS"):
            self.assertEqual(self.frag.get(sym), "y", sym)
        # Not the MPC One's power chip (an ACT8846 with two SYR827/828).
        self.assertNotIn("CONFIG_REGULATOR_RK808", self.frag)
        # MPFree's audio keeps cores 2 and 3, as on Akai's kernel.
        self.assertIn("isolcpus=2-3", self.frag["CONFIG_CMDLINE"].strip('"').split())
        self.assertIn("loadpin", self.frag["CONFIG_LSM"].strip('"').split(","))

    def test_what_the_built_config_lacked_against_akais(self):
        # The built config compared with Akai's, symbol by symbol (2026-11-17).
        for sym in ("CONFIG_ARM_ERRATA_818325_852422", "CONFIG_ARM_ERRATA_821420",
                    "CONFIG_ARM_ERRATA_825619", "CONFIG_ARM_ERRATA_852421", "CONFIG_ARM_ERRATA_852423",
                    "CONFIG_ARM_ERRATA_857271", "CONFIG_ARM_ERRATA_857272", "CONFIG_IP_MULTICAST",
                    "CONFIG_USB_CONFIGFS", "CONFIG_CGROUP_SCHED", "CONFIG_GPIO_SYSFS",
                    "CONFIG_DM_VERITY", "CONFIG_SECURITY_LOADPIN_VERITY"):
            self.assertEqual(self.frag.get(sym), "y", sym)
        # Akai's kernel leaves the SoC's crypto engine alone.
        self.assertIn("CONFIG_CRYPTO_DEV_ROCKCHIP", self.frag)
        self.assertIsNone(self.frag["CONFIG_CRYPTO_DEV_ROCKCHIP"])
        # Devices a musician plugs in are modules, never built in.
        for sym in ("CONFIG_HFSPLUS_FS", "CONFIG_USB_ACM", "CONFIG_USB_SERIAL", "CONFIG_HID_APPLE"):
            self.assertEqual(self.frag.get(sym), "m", sym)

    def test_the_fragment_passes_against_itself(self):
        self.assertEqual(cc.check(self.text, self.text), [])


if __name__ == "__main__":
    unittest.main()
