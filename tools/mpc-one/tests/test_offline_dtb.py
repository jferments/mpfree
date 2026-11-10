#!/usr/bin/env python3
"""Tests for offline-dtb.py: parse, rebuild, summarise and compare device trees. No device.

If dtc is installed, trees compiled by dtc are also checked to survive a
parse/rebuild round trip unchanged. CI installs it (MPFREE_REQUIRE_DTC=1).
"""
import importlib.util
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "offline-dtb.py")
SPEC = importlib.util.spec_from_file_location("offline_dtb", TOOL)
dtb = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(dtb)

SERIAL = "FAKE-SERIAL-0123"
MAC = bytes.fromhex("0005950a0b0c")


def s(*texts):
    return b"".join(t.encode() + b"\x00" for t in texts)


def cells(*values):
    return struct.pack(f">{len(values)}I", *values)


def node(name, props=(), children=()):
    n = dtb.Node(name)
    n.props = [[k, v] for k, v in props]
    n.children = list(children)
    return n


def board_tree(serial=None, mac=None, emmc_status="okay", model="Akai Professional MPC One"):
    root_props = [("model", s(model)), ("compatible", s("inmusic,acva", "inmusic,az01", "rockchip,rk3288")),
                  ("#address-cells", cells(1)), ("#size-cells", cells(1))]
    if serial:
        root_props.append(("serial-number", s(serial)))
    gmac_props = [("compatible", s("rockchip,rk3288-gmac")), ("reg", cells(0xFF290000, 0x10000))]
    if mac:
        gmac_props.append(("local-mac-address", mac))
    root = node("", root_props, [
        node("aliases", [("mmc0", s("/dwmmc@ff0f0000")), ("mmc1", s("/dwmmc@ff0c0000"))]),
        node("chosen", [("stdout-path", s("serial2:115200n8"))]),
        node("memory@0", [("device_type", s("memory")), ("reg", cells(0, 0x80000000 if serial else 0))]),
        node("dwmmc@ff0f0000", [("compatible", s("rockchip,rk3288-dw-mshc")), ("reg", cells(0xFF0F0000, 0x4000)),
                                ("status", s(emmc_status)), ("non-removable", b"")]),
        node("dwmmc@ff0c0000", [("compatible", s("rockchip,rk3288-dw-mshc")), ("reg", cells(0xFF0C0000, 0x4000))]),
        node("ethernet@ff290000", gmac_props),
        node("sound", [("compatible", s("simple-audio-card"))]),
    ])
    return dtb.Tree(root, [(0x10000000, 0x100000)], 0)


def flatten(tree):
    return [(path, [(k, v) for k, v in n.props]) for path, n in tree.walk()]


class DtbTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-dtb-test-")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def save(self, name, tree):
        path = os.path.join(self.tmp, name)
        with open(path, "wb") as fh:
            fh.write(dtb.build(tree))
        return path

    def cli(self, *args):
        return subprocess.run([sys.executable, "-I", TOOL, *args], stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, timeout=60)

    def test_round_trip(self):
        tree = board_tree(serial=SERIAL, mac=MAC)
        blob = dtb.build(tree)
        again = dtb.parse(blob)
        self.assertEqual(flatten(again), flatten(tree))
        self.assertEqual(again.reserved, [(0x10000000, 0x100000)])
        self.assertEqual(dtb.build(again), blob, "rebuilding a parsed tree gives the same bytes")
        self.assertEqual(again.find("/dwmmc@ff0f0000").prop("status"), s("okay"))
        self.assertIsNone(again.find("/nope"))

    @unittest.skipUnless(shutil.which("dtc") or os.environ.get("MPFREE_REQUIRE_DTC"), "dtc not installed")
    def test_dtc_trees_survive_a_round_trip(self):
        source = os.path.join(self.tmp, "board.dts")
        with open(source, "w") as fh:
            fh.write("""/dts-v1/;
/memreserve/ 0x10000000 0x100000;
/ {
    model = "Akai Professional MPC One";
    compatible = "inmusic,acva", "inmusic,az01", "rockchip,rk3288";
    #address-cells = <1>; #size-cells = <1>;
    aliases { mmc0 = "/dwmmc@ff0f0000"; };
    chosen { stdout-path = "serial2:115200n8"; };
    memory@0 { device_type = "memory"; reg = <0 0>; };
    emmc: dwmmc@ff0f0000 { compatible = "rockchip,rk3288-dw-mshc"; reg = <0xff0f0000 0x4000>; bus-width = <8>; };
    user { link = <&emmc>; bytes = [01 02 03]; flag; };
};
""")
        compiled, rebuilt = os.path.join(self.tmp, "a.dtb"), os.path.join(self.tmp, "b.dtb")
        subprocess.run(["dtc", "-q", "-I", "dts", "-O", "dtb", "-o", compiled, source], check=True)
        with open(compiled, "rb") as fh:
            tree = dtb.parse(fh.read())
        with open(rebuilt, "wb") as fh:
            fh.write(dtb.build(tree))
        a = subprocess.run(["dtc", "-q", "-I", "dtb", "-O", "dts", compiled], check=True, capture_output=True).stdout
        b = subprocess.run(["dtc", "-q", "-I", "dtb", "-O", "dts", rebuilt], check=True, capture_output=True).stdout
        self.assertEqual(a, b)

    def test_summary(self):
        path = self.save("acva.dtb", board_tree())
        res = self.cli("summary", path)
        self.assertEqual(res.returncode, 0, res.stdout)
        out = res.stdout
        self.assertIn("model: Akai Professional MPC One", out)
        self.assertIn("compatible: inmusic,acva, inmusic,az01, rockchip,rk3288", out)
        self.assertIn("storage controller /dwmmc@ff0f0000: rockchip,rk3288-dw-mshc; status okay; aliases: mmc0", out)
        self.assertIn("storage controller /dwmmc@ff0c0000: rockchip,rk3288-dw-mshc; "
                      "status okay (no status property); aliases: mmc1", out)
        self.assertIn("nodes at the eMMC address ff0f0000: 1", out)
        self.assertIn("memory node /memory@0: reg <0x0 0x0>", out)
        self.assertIn("/chosen properties: stdout-path (17 bytes)", out)
        disabled = self.save("off.dtb", board_tree(emmc_status="disabled"))
        self.assertIn("/dwmmc@ff0f0000: rockchip,rk3288-dw-mshc; status disabled", self.cli("summary", disabled).stdout)

    def test_diff_hides_values_unless_asked(self):
        stock = self.save("stock.dtb", board_tree())
        live = self.save("live.dtb", board_tree(serial=SERIAL, mac=MAC))
        res = self.cli("diff", stock, live)
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("property /serial-number: only in B (17 bytes)", res.stdout)
        self.assertIn("property /ethernet@ff290000/local-mac-address: only in B (6 bytes)", res.stdout)
        self.assertIn("property /memory@0/reg: differs (A 8 bytes, B 8 bytes)", res.stdout)
        self.assertIn("3 difference(s)", res.stdout)
        self.assertNotIn(SERIAL, res.stdout)
        self.assertNotIn(MAC.hex(), res.stdout)
        shown = self.cli("diff", stock, live, "--values")
        self.assertIn(f'B "{SERIAL}"', shown.stdout)
        self.assertIn("B <0x0 0x80000000>", shown.stdout)
        self.assertIn("0 difference(s)", self.cli("diff", stock, stock).stdout)

    def test_boards_and_match(self):
        acva = self.save("acva.dtb", board_tree())
        other = board_tree(model="Other")
        other.root.set_prop("compatible", s("inmusic,acva2", "rockchip,rk3288"))
        acva2 = self.save("acva2.dtb", other)
        junk = os.path.join(self.tmp, "junk.dtb")
        with open(junk, "wb") as fh:
            fh.write(b"\x00" * 64)
        res = self.cli("boards", acva, acva2, junk)
        self.assertIn(f"{acva}: Akai Professional MPC One; inmusic,acva, inmusic,az01, rockchip,rk3288", res.stdout)
        self.assertIn(f"{acva2}: Other; inmusic,acva2, rockchip,rk3288", res.stdout)
        self.assertIn(f"{junk}: ERROR not a device tree", res.stdout)
        res = self.cli("match", "inmusic,acva", acva, acva2)
        self.assertEqual(res.stdout.split(), [acva], "acva2 must not match acva")

    def test_damaged_trees_are_refused(self):
        good = dtb.build(board_tree())
        cases = {
            "magic": b"\x00" * 4 + good[4:],
            "short": good[:20],
            "truncated": good[:len(good) // 2],
            "string offset": None,
        }
        hdr = list(dtb.HEADER.unpack_from(good, 0))
        bad = bytearray(good)
        # Point the first property's name offset far outside the strings block.
        first_prop = good.find(struct.pack(">I", dtb.FDT_PROP), hdr[2])
        struct.pack_into(">I", bad, first_prop + 8, 0xFFFF)
        cases["string offset"] = bytes(bad)
        for label, blob in cases.items():
            with self.subTest(label):
                path = os.path.join(self.tmp, label.replace(" ", "-") + ".dtb")
                with open(path, "wb") as fh:
                    fh.write(blob)
                res = self.cli("summary", path)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("ERROR:", res.stdout)
                with self.assertRaises(dtb.DtbError):
                    dtb.parse(blob)

    def kexec_inputs(self, target=None):
        live = board_tree(serial=SERIAL, mac=MAC)
        chosen = live.find("/chosen")
        chosen.set_prop("bootargs", s("root=PARTUUID=FAKE-UUID rootwait ro"))
        chosen.set_prop("inmusic,internal-sd-fitted", b"")
        live.find("/").children.append(node("reserved-memory", [("ranges", b"")], [node("ramoops@8000000")]))
        live.reserved.append((0x8F000000, 0xD000))  # as U-Boot reserves the tree's own location
        if target is None:
            target = board_tree(model="Akai Professional MPC One")
            target.find("/dwmmc@ff0f0000").set_prop("new-binding", cells(7))
        return (self.save("new.dtb", target), self.save("old.dtb", board_tree()), self.save("live.dtb", live))

    def kexec(self, new, old, live, out, *extra):
        return self.cli("kexec-dtb", "--target", new, "--reference", old, "--live", live, "--out", out, *extra)

    def test_kexec_dtb(self):
        new, old, live = self.kexec_inputs()
        out = os.path.join(self.tmp, "kexec.dtb")
        res = self.kexec(new, old, live, out, "--drop", "/chosen/inmusic,internal-sd-fitted")
        self.assertEqual(res.returncode, 0, res.stdout)
        for line in ("added from the running tree (17 bytes): /serial-number",
                     "added from the running tree (6 bytes): /ethernet@ff290000/local-mac-address",
                     "replaced from the running tree (8 bytes): /memory@0/reg",
                     "not copied (kexec sets it): /chosen/bootargs",
                     "copied node added by U-Boot: /reserved-memory",
                     "dropped as asked: /chosen/inmusic,internal-sd-fitted",
                     'eMMC controller disabled: /dwmmc@ff0f0000 status = "disabled"',
                     'SD card controller disabled: /dwmmc@ff0c0000 status = "disabled"',
                     "not copied: memory reservations (running tree 2, result keeps the new file's 1)",
                     f"written: {out} ",
                     "its eMMC and SD card controllers are disabled"):
            self.assertIn(line, res.stdout)
        self.assertNotIn(SERIAL, res.stdout)
        self.assertNotIn("FAKE-UUID", res.stdout)
        tree = dtb.load(out)
        self.assertEqual(tree.root.prop("serial-number"), s(SERIAL))
        self.assertEqual(tree.find("/ethernet@ff290000").prop("local-mac-address"), MAC)
        self.assertEqual(tree.find("/memory@0").prop("reg"), cells(0, 0x80000000))
        self.assertIsNone(tree.find("/chosen").prop("bootargs"))
        self.assertIsNone(tree.find("/chosen").prop("inmusic,internal-sd-fitted"))
        self.assertIsNotNone(tree.find("/reserved-memory/ramoops@8000000"))
        emmc = tree.find("/dwmmc@ff0f0000")
        self.assertEqual(emmc.prop("status"), s("disabled"))
        self.assertEqual(emmc.prop("new-binding"), cells(7), "the new version's own properties stay")
        self.assertEqual(tree.find("/dwmmc@ff0c0000").prop("status"), s("disabled"))
        self.assertEqual(tree.reserved, [(0x10000000, 0x100000)], "U-Boot's reservations are not copied")
        res = self.kexec(new, old, live, out)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("already exists; not overwriting it", res.stdout)

    def test_kexec_dtb_stops(self):
        out = os.path.join(self.tmp, "kexec.dtb")
        no_ethernet = board_tree()
        no_ethernet.root.children = [c for c in no_ethernet.root.children if not c.name.startswith("ethernet")]
        no_emmc = board_tree()
        no_emmc.root.children = [c for c in no_emmc.root.children if c.name != "dwmmc@ff0f0000"]
        no_sd = board_tree()
        no_sd.root.children = [c for c in no_sd.root.children if c.name != "dwmmc@ff0c0000"]
        cases = {
            "the new tree has no /ethernet@ff290000": no_ethernet,
            "expected one node at the eMMC address ff0f0000 in the new tree, found 0": no_emmc,
            "expected one node at the SD card address ff0c0000 in the new tree, found 0": no_sd,
        }
        for message, target in cases.items():
            with self.subTest(message):
                new, old, live = self.kexec_inputs(target)
                res = self.kexec(new, old, live, out)
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("STOP: ", res.stdout)
                self.assertIn(message, res.stdout)
                self.assertIn("No file written.", res.stdout)
                self.assertFalse(os.path.exists(out))
        new, old, live = self.kexec_inputs()
        res = self.kexec(new, old, live, out, "--drop", "/chosen/nope")
        self.assertIn("STOP: --drop /chosen/nope: no such property in the result", res.stdout)
        removed = board_tree(serial=SERIAL)
        removed.find("/chosen").del_prop("stdout-path")
        res = self.kexec(new, old, self.save("live2.dtb", removed), out)
        self.assertIn("STOP: U-Boot removed /chosen/stdout-path; decide by hand", res.stdout)
        unsized = self.save("live3.dtb", board_tree())
        res = self.kexec(new, old, unsized, out)
        self.assertIn("STOP: /memory@0 has no memory size", res.stdout)
        other = board_tree()
        other.root.set_prop("compatible", s("inmusic,acva2", "rockchip,rk3288"))
        res = self.kexec(self.save("other.dtb", other), old, live, out)
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn("--target is not a inmusic,acva tree", res.stdout)
        self.assertFalse(os.path.exists(out))
        self.assertEqual(self.cli("kexec-dtb", "--target", new).returncode, 2)

    def test_usage(self):
        self.assertEqual(self.cli().returncode, 2)
        self.assertEqual(self.cli("diff", "a").returncode, 2)
        self.assertEqual(self.cli("diff", "a", "b", "--bogus").returncode, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
