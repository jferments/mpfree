#!/usr/bin/env python3
"""Tests for how tools/kernel/build.sh picks, fetches and checks the kernel, the RT patch and
the compiler.

They run `build.sh --inside --sources-only` against a fake kernel.org served on localhost, with
files signed by throwaway test keys. The kernel's test key is not Greg Kroah-Hartman's or Linus
Torvalds's, so an unpinned run must refuse the kernel after checking its signature. The
compiler's is a test key with Arnd Bergmann's address (the real check is that the key comes from
kernel.org's pgpkeys or keys.openpgp.org, which publishes only addresses their owners confirmed);
signed by any other key, it is refused. A pinned run must take exactly the pinned sha256 and
nothing else. The compile itself runs only in CI (.github/workflows/kernel.yml). Needs bash, gpg,
xz, curl and tar; skips without them.
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
GCC = "14.2.0"
GCC_TAR = f"x86_64-gcc-{GCC}-nolibc-arm-linux-gnueabi.tar.xz"


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
        self.crosstool = os.path.join(self.web, "crosstool")
        for d in (os.path.join(self.korg, "v6.x"), os.path.join(self.korg, "projects/rt/6.18"),
                  os.path.join(self.web, "pgpkeys/keys"), os.path.join(self.repo, "tools/kernel"),
                  self.signer_home):
            os.makedirs(d)
        os.chmod(self.signer_home, 0o700)
        self.fpr = self.new_key("MPFree Test Signer <test@example.invalid>")
        self.arnd = self.new_key("Arnd Bergmann (test key) <arnd@arndb.de>")

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

        # The compiler: gcc-VERSION-nolibc/arm-linux-gnueabi/bin/..., beside other gcc versions.
        for v in ("13.3.0", "14.1.0", GCC, "15.1.0"):
            os.makedirs(os.path.join(self.crosstool, v))
        tc = os.path.join(t, "tc", f"gcc-{GCC}-nolibc/arm-linux-gnueabi/bin")
        os.makedirs(tc)
        with open(os.path.join(tc, "arm-linux-gnueabi-gcc"), "w") as fh:
            fh.write("#!/bin/sh\necho fake gcc\n")
        plain = os.path.join(self.crosstool, GCC, GCC_TAR[:-3])
        with tarfile.open(plain, "w") as tf:
            tf.add(os.path.join(t, "tc", f"gcc-{GCC}-nolibc"), arcname=f"gcc-{GCC}-nolibc")
        self.sign(plain, plain[:-len(".tar")] + ".tar.sign", self.arnd)
        subprocess.run(["xz", "-0", plain], check=True)

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

    def new_key(self, uid):
        self.gpg("--quick-gen-key", uid, "ed25519", "sign", "never")
        out = self.gpg("--with-colons", "--list-keys", uid).stdout
        fpr = [line.split(":")[9] for line in out.splitlines() if line.startswith("fpr:")][0]
        with open(os.path.join(self.web, "pgpkeys/keys", fpr[-16:] + ".asc"), "w") as fh:
            fh.write(self.gpg("--armor", "--export", fpr).stdout)
        return fpr

    def sign(self, path, sig, key=None):
        if os.path.exists(sig):
            os.remove(sig)
        self.gpg("--armor", "--detach-sign", "--local-user", key or self.fpr, "-o", sig, path)

    def pin(self, version="", patch="", tar_sha="", patch_sha="", gcc="", gcc_sha=""):
        with open(os.path.join(self.repo, "tools/kernel/versions.env"), "w") as fh:
            fh.write(f"LINUX_SERIES=6.18\nLINUX_VERSION={version}\nRT_PATCH={patch}\n"
                     f"LINUX_TAR_SHA256={tar_sha}\nRT_PATCH_SHA256={patch_sha}\n"
                     f"GCC_SERIES=14\nGCC_VERSION={gcc}\nGCC_TAR_SHA256={gcc_sha}\n")

    def pin_kernel(self, **gcc):
        self.pin(VERSION, PATCH, sha(self.served(f"v6.x/linux-{VERSION}.tar.xz")),
                 sha(self.served(f"projects/rt/6.18/{PATCH}")), **gcc)

    def gcc_served(self):
        return os.path.join(self.crosstool, GCC, GCC_TAR)

    def run_sources(self):
        env = {k: v for k, v in os.environ.items() if "proxy" not in k.lower()}
        env.update(HOME=self.home, NO_PROXY="*", no_proxy="*",
                   MPFREE_KERNEL_KORG=self.base + "/korg",
                   MPFREE_KERNEL_CROSSTOOL=self.base + "/crosstool",
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
        self.pin_kernel(gcc=GCC, gcc_sha=sha(self.gcc_served()))
        r = self.run_sources()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn(f"linux-{VERSION}.tar.xz: sha256 pinned in versions.env", r.stdout)
        self.assertIn(f"{GCC_TAR}: sha256 pinned in versions.env", r.stdout)
        self.assertIn(f"SOURCES-OK linux-{VERSION} {PATCH} gcc-{GCC} pinned=yes", r.stdout)
        # A second run takes them from the cache, and each run's own folder is gone.
        shutil.rmtree(os.path.join(self.korg, "v6.x"))
        shutil.rmtree(self.crosstool)
        r = self.run_sources()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual([n for n in os.listdir(self.cache) if n.startswith("run.") or ".part." in n], [])

    def test_unpinned_compiler_is_the_newest_gcc_14_signed_by_arnd_bergmann(self):
        self.pin_kernel()
        r = self.run_sources()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("taking the newest gcc 14 crosstool", r.stdout)
        self.assertIn(f"{GCC_TAR}: PGP: {self.arnd} Arnd Bergmann (test key) <arnd@arndb.de> (key from kernel.org pgpkeys",
                      r.stdout)
        self.assertIn(f"SOURCES-OK linux-{VERSION} {PATCH} gcc-{GCC} pinned=no", r.stdout)

    def test_a_compiler_signed_by_anyone_else_is_refused(self):
        plain = os.path.join(self.tmp, "gcc.tar")
        subprocess.run(["sh", "-c", f'xz -cd "{self.gcc_served()}" > "{plain}"'], check=True)
        self.sign(plain, os.path.join(self.crosstool, GCC, GCC_TAR[:-3] + ".sign"), self.fpr)
        self.pin_kernel()
        r = self.run_sources()
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn(f"{GCC_TAR} is signed by {self.fpr} (MPFree Test Signer <test@example.invalid>), "
                      "not by Arnd Bergmann <arnd@arndb.de>", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.cache, GCC_TAR)))

    def test_a_changed_compiler_fails_its_signature(self):
        tar = self.gcc_served()
        subprocess.run(["xz", "-d", tar], check=True)
        with open(tar[:-3], "ab") as fh:
            fh.write(b"\0" * 512)
        subprocess.run(["xz", "-0", tar[:-3]], check=True)
        self.pin_kernel()
        r = self.run_sources()
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn(f"{GCC_TAR}: kernel.org's PGP signature does not verify", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.cache, GCC_TAR)))

    def test_pinned_compiler_refuses_a_different_file(self):
        self.pin_kernel(gcc=GCC, gcc_sha="0" * 64)
        r = self.run_sources()
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn(f"{GCC_TAR}: sha256 {sha(self.gcc_served())}, pinned {'0' * 64} (removed from the cache)",
                      r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.cache, GCC_TAR)))

    def test_a_compiler_outside_gcc_14_stops(self):
        self.pin_kernel(gcc="15.1.0", gcc_sha="0" * 64)
        r = self.run_sources()
        self.assertEqual(r.returncode, 1)
        self.assertIn("GCC_VERSION 15.1.0 is not gcc 14", r.stderr)

    def test_half_a_pin_stops(self):
        self.pin(VERSION, PATCH)
        r = self.run_sources()
        self.assertEqual(r.returncode, 1)
        self.assertIn("versions.env pins LINUX_VERSION but not all of", r.stderr)
        self.pin_kernel(gcc=GCC)
        r = self.run_sources()
        self.assertEqual(r.returncode, 1)
        self.assertIn("versions.env pins GCC_VERSION but not GCC_TAR_SHA256", r.stderr)

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
                 sha(os.path.join(rt, "older", PATCH)), GCC, sha(self.gcc_served()))
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
        self.assertIn("CROSSTOOL_SIGNER=arnd@arndb.de\n", text)
        self.assertNotIn("MPFREE_KERNEL_GREG", text)
        self.assertNotIn("MPFREE_KERNEL_SIGNER", text)
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
