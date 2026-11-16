#!/usr/bin/env python3
"""The USB creator's recipe (rust/mpfree-stick/src/recipe.json) is what offline-kexec-stick.py's build_rootfs
makes today. A change to the builder fails here until the recipe is written again:
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
        everything = json.dumps(r["files"])
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


if __name__ == "__main__":
    unittest.main()
