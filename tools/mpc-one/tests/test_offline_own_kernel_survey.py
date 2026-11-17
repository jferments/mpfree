#!/usr/bin/env python3
"""Tests for offline-own-kernel-survey.py against a fake rootfs image. No device, no network.

The image is a small ext4 file made with mke2fs -d. It holds a fake kernel whose
gzip payload carries an IKCONFIG block, a fake board file, and module lists with
device-tree aliases. Needs e2fsprogs (debugfs, mke2fs); set
MPFREE_REQUIRE_OFFLINE_TOOLS=1 (CI does) to fail instead of skip without them.
"""
import gzip
import hashlib
import importlib.util
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
TOOL = os.path.join(TOOLS, "offline-own-kernel-survey.py")
SPEC = importlib.util.spec_from_file_location("offline_dtb", os.path.join(TOOLS, "offline-dtb.py"))
dtb = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(dtb)
SBIN_PATH = os.pathsep.join([os.environ.get("PATH", ""), "/usr/sbin", "/sbin"])

RELEASE = "6.18.44-az01-fake-rt6"
CONFIG = (b"#\n# Automatically generated file; DO NOT EDIT.\n"
          b'CONFIG_CC_VERSION_TEXT="arm-fake-gcc (GCC) 13.3.0"\n'
          b'CONFIG_LOCALVERSION="-az01-fake-rt6"\n'
          b"CONFIG_PREEMPT_RT=y\n"
          b"CONFIG_MODULES=y\n"
          b"# CONFIG_MODULE_SIG is not set\n"
          b"CONFIG_MMC=y\n"
          b"CONFIG_SND_SOC_ROCKCHIP_I2S=y\n"
          b"CONFIG_SND_SOC_INMUSIC_ACVA=y\n"
          b"CONFIG_TOUCHSCREEN_ILI210X=m\n"
          b"CONFIG_NOT_SELECTED_FAKE=y\n")
BUILTIN_MODINFO = b"".join(e + b"\0" for e in (
    b"snd_soc_jp07.alias=of:N*T*Cinmusic,acva-audio",
    b"snd_soc_jp07.alias=of:N*T*Cinmusic,acva-audioC*",
    b"snd_soc_jp07.description=fake",
    b"snd_soc_rockchip_i2s.alias=of:N*T*Crockchip,rk3066-i2s",
    b"dw_mmc_rockchip.alias=of:N*T*Crockchip,rk3288-dw-mshc",
))
MODULES_ALIAS = (b"# Aliases extracted from modules themselves.\n"
                 b"alias of:N*T*Cilitek,ili2117 ili210x\n"
                 b"alias of:N*T*Cilitek,ili2117C* ili210x\n"
                 b"alias usb:v0BDAp8179d*dc*dsc*dp*ic*isc*ip*in* rtl8xxxu\n")
MODULES_BUILTIN = (b"kernel/sound/soc/inmusic/snd-soc-jp07.ko\n"
                   b"kernel/sound/soc/rockchip/snd-soc-rockchip-i2s.ko\n"
                   b"kernel/drivers/mmc/host/dw_mmc-rockchip.ko\n")
MODULES_ORDER = b"kernel/drivers/input/touchscreen/ili210x.ko\nkernel/net/wireless/cfg80211.ko\n"


def s(*texts):
    return b"".join(t.encode() + b"\x00" for t in texts)


def cells(*values):
    return struct.pack(f">{len(values)}I", *values)


def node(name, props=(), children=()):
    n = dtb.Node(name)
    n.props = [[k, v] for k, v in props]
    n.children = list(children)
    return n


def board():
    root = node("", [("model", s("Akai Professional MPC One")),
                     ("compatible", s("inmusic,acva", "inmusic,az01", "rockchip,rk3288")),
                     ("#address-cells", cells(1)), ("#size-cells", cells(1))], [
        node("i2s@ff890000", [("compatible", s("rockchip,rk3288-i2s", "rockchip,rk3066-i2s")),
                              ("reg", cells(0xFF890000, 0x10000)), ("phandle", cells(5))]),
        node("sound", [("compatible", s("inmusic,acva-audio")), ("inmusic,cpu", cells(5)),
                       ("mute-gpios", cells(7, 20, 0))]),
        node("dwmmc@ff0f0000", [("compatible", s("rockchip,rk3288-dw-mshc")), ("status", s("okay"))]),
        node("dwmmc@ff0c0000", [("compatible", s("rockchip,rk3288-dw-mshc")), ("status", s("disabled"))]),
        node("i2c@ff160000", [("compatible", s("rockchip,rk3288-i2c"))], [
            node("touchscreen@26", [("compatible", s("ilitek,ili2117")), ("reg", cells(0x26))]),
        ]),
        node("odd-thing", [("compatible", s("vendor,nothing-claims-this"))]),
    ])
    return dtb.build(dtb.Tree(root, [], 0))


def noise(n, seed):
    rnd = random.Random(seed)
    return bytes(rnd.choice(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789") for _ in range(n))


def zimage(config=CONFIG):
    # mtime=0: gzip stamps the current second into its header, so two calls a second apart
    # (the image, then the hash the test expects) would make different files.
    inner = noise(50000, 1) + b"IKCFG_ST" + gzip.compress(config, mtime=0) + b"IKCFG_ED" + noise(20000, 2)
    return noise(4000, 3) + gzip.compress(inner, mtime=0) + noise(64, 4)


def sha_file(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def tools_present():
    return all(shutil.which(t, path=SBIN_PATH) for t in ("debugfs", "mke2fs"))


def make_image(base, files):
    root = os.path.join(base, "root")
    for rel, data in files.items():
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as fh:
            fh.write(data)
    os.makedirs(os.path.join(root, "usr", "lib"), exist_ok=True)
    os.symlink("usr/lib", os.path.join(root, "lib"))
    image = os.path.join(base, "rootfs.img")
    with open(image, "wb") as fh:
        fh.truncate(16 << 20)
    res = subprocess.run(["mke2fs", "-q", "-F", "-t", "ext4", "-O", "^has_journal", "-E", "root_owner=0:0",
                          "-d", root, image], capture_output=True, text=True,
                         env=dict(os.environ, PATH=SBIN_PATH))
    assert res.returncode == 0, res.stderr
    shutil.rmtree(root)
    return image


def standard_files(kernel=None, modules=True):
    files = {f"boot/zImage-{RELEASE}": kernel or zimage(), "boot/rk3288-az01-acva.dtb": board(),
             "boot/rk3288-az01-acva-c.dtb": board()}
    if modules:
        folder = f"usr/lib/modules/{RELEASE}/"
        files.update({folder + "modules.builtin.modinfo": BUILTIN_MODINFO, folder + "modules.alias": MODULES_ALIAS,
                      folder + "modules.builtin": MODULES_BUILTIN, folder + "modules.order": MODULES_ORDER})
    return files


@unittest.skipUnless(tools_present() or os.environ.get("MPFREE_REQUIRE_OFFLINE_TOOLS") == "1",
                     "needs e2fsprogs (debugfs, mke2fs)")
class SurveyTests(unittest.TestCase):
    def setUp(self):
        if not tools_present():
            self.fail("debugfs or mke2fs is missing")
        self.tmp = tempfile.mkdtemp(prefix="mpfree-own-kernel-test-")
        self.env = dict(os.environ, PATH=SBIN_PATH, MPFREE_WORK_DIR=os.path.join(self.tmp, "work"), LC_ALL="C")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_tool(self, *args):
        return subprocess.run([sys.executable, "-I", TOOL, *args], env=self.env, capture_output=True, text=True,
                              timeout=120)

    def test_survey(self):
        image = make_image(self.tmp, standard_files())
        before = sha_file(image)
        res = self.run_tool("--image", image)
        self.assertEqual(res.returncode, 0, res.stderr)
        out = res.stdout
        kernel_sum = hashlib.sha256(zimage()).hexdigest()
        for text in (f"kernel: /boot/zImage-{RELEASE}, {len(zimage())} bytes, sha256 {kernel_sum}",
                     "found: gzip payload at offset 4000; lines: 12, built in (=y): 6, modules (=m): 1",
                     f"sha256 of the whole config: {hashlib.sha256(CONFIG).hexdigest()}",
                     "signed modules: not checked (no CONFIG_MODULE_SIG)",
                     'CONFIG_CC_VERSION_TEXT="arm-fake-gcc (GCC) 13.3.0"', "CONFIG_PREEMPT_RT=y", "CONFIG_MMC=y",
                     "--- symbols with a vendor name in them (no mainline driver)\nCONFIG_SND_SOC_INMUSIC_ACVA=y\n",
                     "model: Akai Professional MPC One; compatible: inmusic,acva inmusic,az01 rockchip,rk3288",
                     "/i2s@ff890000  okay  rockchip,rk3288-i2s rockchip,rk3066-i2s  ->  "
                     "snd_soc_rockchip_i2s (built in, for rockchip,rk3066-i2s)",
                     "/sound  okay  inmusic,acva-audio  ->  snd_soc_jp07 (built in, for inmusic,acva-audio)  [vendor]",
                     "/dwmmc@ff0c0000  disabled  rockchip,rk3288-dw-mshc  ->  dw_mmc_rockchip (built in",
                     "/i2c@ff160000/touchscreen@26  okay  ilitek,ili2117  ->  ili210x (module, for ilitek,ili2117)",
                     "/odd-thing  okay  vendor,nothing-claims-this  ->  (none)",
                     "--- enabled nodes no driver in this kernel claims by compatible",
                     "/sound {", "mute-gpios = ", "inmusic,cpu = <0x5>;",
                     "touchscreen@26 {",
                     f"=== 4. Modules (/usr/lib/modules/{RELEASE}) ===", "built in: 3; loadable: 2",
                     "--- with a vendor name\nsnd_soc_jp07 (built in)\n",
                     "--- loadable modules\nili210x\ncfg80211\n",
                     "=== 5. Build settings, whole ===", "CONFIG_NOT_SELECTED_FAKE=y", "END OF OWN KERNEL SURVEY"):
            self.assertIn(text, out)
        unclaimed = out.split("--- enabled nodes no driver in this kernel claims by compatible")[1].split("===")[0]
        self.assertIn("/odd-thing", unclaimed)
        self.assertIn("/i2c@ff160000\n", unclaimed)
        self.assertNotIn("/sound", unclaimed)
        self.assertNotIn("/dwmmc@ff0c0000", unclaimed)  # disabled
        self.assertEqual(out.count("CONFIG_NOT_SELECTED_FAKE=y"), 1)
        self.assertEqual(sha_file(image), before, "the image changed")
        self.assertEqual(res.stderr, "")

    def test_same_hash_as_proc_config_gz(self):
        # The running kernel's /proc/config.gz holds the same gzip block; read-only-drivers.sh hashes
        # its decompressed bytes, so the two hashes match for the same kernel build.
        image = make_image(self.tmp, standard_files())
        res = self.run_tool("--image", image)
        proc_config_gz = gzip.compress(CONFIG)
        self.assertIn(f"sha256 of the whole config: {hashlib.sha256(gzip.decompress(proc_config_gz)).hexdigest()}",
                      res.stdout)

    def test_signed_modules_and_no_module_lists(self):
        config = CONFIG.replace(b"# CONFIG_MODULE_SIG is not set\n",
                                b"CONFIG_MODULE_SIG=y\nCONFIG_MODULE_SIG_FORCE=y\n")
        image = make_image(self.tmp, standard_files(kernel=zimage(config), modules=False))
        res = self.run_tool("--image", image, "--board", "rk3288-az01-acva-c.dtb")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("signed modules: REQUIRED (CONFIG_MODULE_SIG_FORCE=y)", res.stdout)
        self.assertIn(f"(no device-tree aliases in the module lists: no /lib/modules/{RELEASE}; "
                      f"the driver column is empty)", res.stdout)
        self.assertIn("=== 4. Modules (no module folder in the image) ===", res.stdout)
        self.assertNotIn("--- enabled nodes no driver", res.stdout)

    def test_default_image_is_the_builders(self):
        folder = os.path.join(self.tmp, "work", "akai-unpacked-20261110T030000Z-abc")
        os.makedirs(folder)
        image = make_image(self.tmp, standard_files())
        shutil.move(image, os.path.join(folder, "MPC-3.10-rootfs.img"))
        res = self.run_tool()
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn(os.path.join(folder, "MPC-3.10-rootfs.img") + " (opened read-only by debugfs)", res.stdout)

    def test_stops(self):
        res = self.run_tool()
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: no unpacked update under", res.stderr)
        image = make_image(self.tmp, standard_files())
        res = self.run_tool("--image", image, "--board", "rk3288-other.dtb")
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: the image has no /boot/rk3288-other.dtb; its board files: "
                      "rk3288-az01-acva-c.dtb, rk3288-az01-acva.dtb", res.stderr)
        res = self.run_tool("--image", image, "--board", "../x.dtb")
        self.assertIn("STOP: --board must be a plain .dtb file name", res.stderr)

    def test_kernel_without_config(self):
        image = make_image(self.tmp, standard_files(kernel=noise(9000, 7)))
        res = self.run_tool("--image", image)
        self.assertEqual(res.returncode, 1)
        self.assertIn("STOP: could not read the kernel's build settings: no embedded config found", res.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
