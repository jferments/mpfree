#!/usr/bin/env python3
"""Tests for how tools/kernel/build.sh picks, fetches and checks the kernel and RT patch.

They run `build.sh --inside --sources-only` against a fake kernel.org served on localhost, with
files signed by a throwaway test key. That key is not Greg Kroah-Hartman's or Linus Torvalds's,
so an unpinned run must refuse the kernel after checking its signature; a pinned run must take
exactly the pinned sha256 and nothing else. The compile itself runs only in CI
(.github/workflows/kernel.yml). Needs bash, gpg, xz, curl and tar; skips without them.
"""
import functools
import hashlib
import http.server
import os
import shutil
import subprocess
import tarfile
import tempfile
import threading
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
BUILD = os.path.join(os.path.dirname(HERE), "build.sh")
TOOLS = ("bash", "gpg", "xz", "curl", "tar")

VERSION = "6.18.12"
PATCH = f"patch-{VERSION}-rt7.patch.xz"


def sha(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


@unittest.skipUnless(all(shutil.which(t) for t in TOOLS), "needs " + ", ".join(TOOLS))
class SourcesTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mpfree-kernel-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        t = self.tmp
        self.web = os.path.join(t, "web")
        self.korg = os.path.join(self.web, "korg")
        self.repo = os.path.join(t, "repo")
        self.cache = os.path.join(t, "cache")
        self.home = os.path.join(t, "home")
        self.signer_home = os.path.join(t, "signer")
        for d in (os.path.join(self.korg, "v6.x"), os.path.join(self.korg, "projects/rt/6.18"),
                  os.path.join(self.web, "pgpkeys/keys"), os.path.join(self.repo, "tools/kernel"),
                  self.signer_home):
            os.makedirs(d)
        os.chmod(self.signer_home, 0o700)
        self.gpg("--quick-gen-key", "MPFree Test Signer <test@example.invalid>", "ed25519", "sign", "never")
        out = self.gpg("--with-colons", "--list-keys").stdout
        self.fpr = [line.split(":")[9] for line in out.splitlines() if line.startswith("fpr:")][0]
        with open(os.path.join(self.web, "pgpkeys/keys", self.fpr[-16:] + ".asc"), "w") as fh:
            fh.write(self.gpg("--armor", "--export", self.fpr).stdout)

        # The kernel: a tar of linux-VERSION/, signed uncompressed, served compressed.
        rt = os.path.join(self.korg, "projects/rt/6.18")
        tree = os.path.join(t, f"linux-{VERSION}")
        os.makedirs(tree)
        with open(os.path.join(tree, "Makefile"), "w") as fh:
            fh.write("# fake\n")
        plain = os.path.join(self.korg, "v6.x", f"linux-{VERSION}.tar")
        with tarfile.open(plain, "w") as tf:
            tf.add(tree, arcname=f"linux-{VERSION}")
        self.sign(plain, plain + ".sign")
        subprocess.run(["xz", "-0", plain], check=True)
        # The RT patch, an older one beside it, and a tarball of patches that must not be taken.
        patch = os.path.join(rt, PATCH[:-3])
        with open(patch, "w") as fh:
            fh.write("--- a/Makefile\n+++ b/Makefile\n")
        self.sign(patch, patch[:-len(".patch")] + ".patch.sign")
        subprocess.run(["xz", "-0", patch], check=True)
        for name in ("patch-6.18.5-rt3.patch.xz", "patches-6.18.99-rt99.tar.xz"):
            open(os.path.join(rt, name), "w").close()

        handler = functools.partial(Quiet, directory=self.web)
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.pin()

    def gpg(self, *args):
        return subprocess.run(["gpg", "--batch", "--quiet", "--homedir", self.signer_home, "--passphrase", "",
                               "--pinentry-mode", "loopback", *args],
                              check=True, capture_output=True, text=True)

    def sign(self, path, sig):
        self.gpg("--armor", "--detach-sign", "-o", sig, path)

    def pin(self, version="", patch="", tar_sha="", patch_sha=""):
        with open(os.path.join(self.repo, "tools/kernel/versions.env"), "w") as fh:
            fh.write(f"LINUX_SERIES=6.18\nLINUX_VERSION={version}\nRT_PATCH={patch}\n"
                     f"LINUX_TAR_SHA256={tar_sha}\nRT_PATCH_SHA256={patch_sha}\n")

    def run_sources(self):
        env = {k: v for k, v in os.environ.items() if "proxy" not in k.lower()}
        env.update(HOME=self.home, NO_PROXY="*", no_proxy="*",
                   MPFREE_KERNEL_KORG=self.base + "/korg",
                   MPFREE_KERNEL_PGPKEYS=self.base + "/pgpkeys/keys",
                   MPFREE_KERNEL_OPENPGP=self.base + "/no-such-server",
                   MPFREE_INSIDE_REPO=self.repo, MPFREE_INSIDE_CACHE=self.cache)
        return subprocess.run(["bash", BUILD, "--inside", "--sources-only"], env=env,
                              capture_output=True, text=True, timeout=120)

    def served(self, rel):
        return os.path.join(self.korg, rel)

    def test_unpinned_takes_the_newest_patch_and_refuses_a_key_that_is_not_the_kernels(self):
        r = self.run_sources()
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn(f"kernel {VERSION}, RT patch {PATCH}", r.stdout)
        self.assertIn(f"linux-{VERSION}.tar.xz is signed by {self.fpr}, not by Greg Kroah-Hartman or Linus Torvalds",
                      r.stderr)
        self.assertNotIn("SOURCES-OK", r.stdout)

    def test_a_changed_kernel_fails_its_signature(self):
        tar = self.served(f"v6.x/linux-{VERSION}.tar.xz")
        subprocess.run(["xz", "-d", tar], check=True)
        with open(tar[:-3], "ab") as fh:
            fh.write(b"\0" * 512)  # one more tar block: the same files, different bytes
        subprocess.run(["xz", "-0", tar[:-3]], check=True)
        r = self.run_sources()
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn(f"linux-{VERSION}.tar.xz: kernel.org's PGP signature does not verify", r.stderr)

    def test_no_key_for_the_signer_stops(self):
        shutil.rmtree(os.path.join(self.web, "pgpkeys"))
        r = self.run_sources()
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("could not fetch the signer's key", r.stderr)

    def test_pinned_takes_the_pinned_files(self):
        self.pin(VERSION, PATCH, sha(self.served(f"v6.x/linux-{VERSION}.tar.xz")),
                 sha(self.served(f"projects/rt/6.18/{PATCH}")))
        r = self.run_sources()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn(f"linux-{VERSION}.tar.xz: sha256 pinned in versions.env", r.stdout)
        self.assertIn(f"SOURCES-OK linux-{VERSION} {PATCH} pinned=yes", r.stdout)
        # A second run takes them from the cache.
        shutil.rmtree(os.path.join(self.korg, "v6.x"))
        r = self.run_sources()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_pinned_refuses_a_different_file_and_drops_it_from_the_cache(self):
        self.pin(VERSION, PATCH, "0" * 64, sha(self.served(f"projects/rt/6.18/{PATCH}")))
        r = self.run_sources()
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn(f"linux-{VERSION}.tar.xz: sha256 ", r.stderr)
        self.assertIn("(removed from the cache)", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.cache, f"linux-{VERSION}.tar.xz")))

    def test_pinned_patch_from_the_older_folder(self):
        rt = os.path.join(self.korg, "projects/rt/6.18")
        os.makedirs(os.path.join(rt, "older"))
        shutil.move(os.path.join(rt, PATCH), os.path.join(rt, "older", PATCH))
        self.pin(VERSION, PATCH, sha(self.served(f"v6.x/linux-{VERSION}.tar.xz")),
                 sha(os.path.join(rt, "older", PATCH)))
        r = self.run_sources()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_a_version_outside_the_series_stops(self):
        self.pin("6.12.1", "patch-6.12.1-rt1.patch.xz", "0" * 64, "0" * 64)
        r = self.run_sources()
        self.assertEqual(r.returncode, 1)
        self.assertIn("LINUX_VERSION 6.12.1 is not in 6.18", r.stderr)


class StaticTests(unittest.TestCase):
    def test_the_trust_anchors_are_fixed(self):
        with open(BUILD, encoding="utf-8") as fh:
            text = fh.read()
        self.assertIn("GREG_KH=647F28654894E3BD457199BE38DBBDC86092693E\n", text)
        self.assertIn("LINUS=ABAF11C65A2970B130ABE3C479BE3E4300411886\n", text)
        self.assertNotIn("MPFREE_KERNEL_GREG", text)
        self.assertEqual(subprocess.run(["bash", "-n", BUILD]).returncode, 0)

    def test_dockerfile_is_pinned(self):
        with open(os.path.join(os.path.dirname(BUILD), "Dockerfile"), encoding="utf-8") as fh:
            text = fh.read()
        frm = [line for line in text.splitlines() if line.startswith("FROM ")]
        self.assertEqual(len(frm), 1)
        self.assertRegex(frm[0], r"@sha256:[0-9a-f]{64}$")
        self.assertIn("snapshot.debian.org/archive/debian/${SNAPSHOT}", text)


if __name__ == "__main__":
    unittest.main()
