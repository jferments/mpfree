#!/usr/bin/env python3
"""The USB creator's recipe (rust/mpfree-stick/src/recipe.json) is what offline-kexec-stick.py's build_rootfs
makes today, and its device-tree example (rust/mpfree-stick/tests/data/kexec-dtb-example.json) what
build_kexec_dtb makes. A change to the builder fails here until both are written again:
  python3 tools/mpc-one/creator-recipe.py --write
"""
import importlib.util
import json
import os
import shutil
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.dirname(HERE)
SBIN_PATH = os.pathsep.join(["/usr/sbin", "/sbin", "/usr/bin", "/bin", os.environ.get("PATH", "")])


def load():
    spec = importlib.util.spec_from_file_location("creator_recipe", os.path.join(TOOLS, "creator-recipe.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def tools_present():
    return all(shutil.which(t, path=SBIN_PATH) for t in ("debugfs", "e2fsck", "mke2fs"))


@unittest.skipUnless(tools_present() or os.environ.get("MPFREE_REQUIRE_OFFLINE_TOOLS") == "1",
                     "needs e2fsprogs (mke2fs, debugfs, e2fsck)")
class CreatorRecipeTests(unittest.TestCase):
    def test_the_recipe_in_the_repository_is_current(self):
        cr = load()
        with open(cr.OUT) as fh:
            have = fh.read()
        want = cr.text(cr.recipe())
        self.assertEqual(have, want, "the builder changed: run python3 tools/mpc-one/creator-recipe.py --write")

    def test_the_recipe_has_what_the_creator_needs(self):
        cr = load()
        with open(cr.OUT) as fh:
            r = json.load(fh)
        b = cr.builder()
        # The soft-reboot pieces the hook checks, and its protocol.
        self.assertEqual(r["softreboot_protocol"], b.SOFTREBOOT_PROTOCOL)
        for path in r["softreboot_listed"] + [r["softreboot_sums"]]:
            self.assertIn(path, r["files"])
        # Every placeholder the creator fills in appears somewhere, and nothing else looks like one.
        everything = json.dumps([r["files"], r["routes"]])
        for key, value in r["placeholders"].items():
            self.assertIn(value, everything, key)
        # Developer SSH pieces are all optional; the pad player and the hook are not.
        for path in r["only_with_ssh"]:
            self.assertTrue("ssh" in path or "ethernet" in path or "authorized_keys" in path
                            or "host_ed25519" in path, path)
        self.assertNotIn("/etc/mpfree-test/mpfree", r["only_with_ssh"])
        self.assertNotIn("/etc/mpfree-test/internal-off.sh", r["only_with_ssh"])
        # Akai's own updater and launcher are stubbed, and the factory app is masked.
        self.assertIn("/usr/sbin/az01-update", r["stub_programs"])
        self.assertEqual(r["links"]["/etc/systemd/system/acvs.service"], "/dev/null")
        self.assertEqual(r["data_start_sectors"], 2 * 1024 * 1024)


    def test_the_kexec_dtb_example_is_current(self):
        cr = load()
        with open(cr.EXAMPLE_OUT) as fh:
            have = fh.read()
        self.assertEqual(have, cr.text(cr.example()),
                         "build_kexec_dtb changed: run python3 tools/mpc-one/creator-recipe.py --write")
        example = json.loads(have)
        for value in ("EXAMPLE-SERIAL-0", "020000000001"):
            self.assertNotIn(value, json.dumps(example["notes"]), "the notes name properties, never values")

    def test_the_developer_routes(self):
        cr = load()
        with open(cr.OUT) as fh:
            r = json.load(fh)
        b = cr.builder()
        routes = r["routes"]
        self.assertEqual(set(routes), {"kexec-file", "kexec-file-loadpin-off", "chain"})
        hook = b.SOFTREBOOT_DIR
        for name, route in routes.items():
            with self.subTest(name):
                sums = route["kexec_sums"]
                for path in route["softreboot_listed"]:
                    self.assertTrue(path in r["files"] or path in route["files"] or (sums and path == sums["path"]),
                                    path)
                self.assertEqual(route["softreboot_listed"][:len(r["softreboot_listed"])], r["softreboot_listed"])
                for f in b.KEXEC_FILE_FILES:
                    self.assertIn(f"{hook}/{f}", route["files"])
                self.assertEqual(route["files"][f"{hook}/jump"]["text"], b.KEXEC_FILE_JUMP + "\n")
                self.assertTrue(route["files"][f"{hook}/kernel.sha256"]["text"].startswith("@KERNEL_SHA256@  "))
                # Akai's own LoadPin unit runs there: only the soft-reboot stick masks it.
                self.assertEqual(route["unlinked"], ["/etc/systemd/system/" + u for u in b.SOFTREBOOT_MASK_UNITS])
                self.assertFalse(set(route["files"]) & set(r["files"]))
        words = {name: route["files"][f"{hook}/cmdline"]["text"].split() for name, route in routes.items()}
        self.assertIn(b.KEXEC_OFF_WORD, words["kexec-file"])
        self.assertNotIn(b.LOADPIN_OFF, words["kexec-file"])
        self.assertEqual(words["kexec-file-loadpin-off"], words["kexec-file"] + [b.LOADPIN_OFF])
        self.assertIn(b.CHAIN_WORD, words["chain"])
        self.assertNotIn(b.KEXEC_OFF_WORD, words["chain"])
        self.assertEqual(words["kexec-file"][0], "root=PARTUUID=@ROOT_UUID@")
        chain = routes["chain"]
        self.assertEqual(chain["unstubbed"], [b.KEXEC_PROGRAM])
        self.assertEqual(chain["files"][b.KEXEC_DIR + "/kexec.dtb"], {"mode": 0o644, "placeholder": "@KEXEC_DTB@"})
        self.assertEqual(chain["links"][b.KEXEC_DIR + "/zImage"], "../zImage-@RELEASE@")
        self.assertEqual(chain["kexec_sums"]["names"], ["zImage", "kexec.dtb", "cmdline.txt"])
        self.assertEqual(chain["mkdirs"], [b.KEXEC_DIR])
        for path in b.CHAIN_FILES:
            self.assertIn(path, chain["files"])
        for name in ("kexec-file", "kexec-file-loadpin-off"):
            self.assertEqual((routes[name]["unstubbed"], routes[name]["links"], routes[name]["kexec_sums"]),
                             ([], {}, None))


if __name__ == "__main__":
    unittest.main()
