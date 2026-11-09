"""Synthetic host-side tests of conservative ABI compatibility assessment."""
import importlib.util
from pathlib import Path
import unittest

SOURCE = Path(__file__).resolve().parents[1] / "tools/mpc-one/check-arm-abi.py"
spec = importlib.util.spec_from_file_location("mpfree_arm_abi", SOURCE)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

BASE_REPORT = """MPFREE ARMV7 USERSPACE ABI SURVEY - READ ONLY
=== Runtime GNU libc version ===
getconf not found (libc version remains unverified)
=== GNU libc embedded release banner (read-only strings) ===
GNU libc release banner: unavailable (do not infer version)
=== Dynamic ELF interpreter and known native library paths ===
/lib/ld-linux-armhf.so.3 -> (regular file)
/lib/libc.so.6 -> (regular file)
/lib/libm.so.6 -> (regular file)
=== Available GLIBC symbol tag maximum, if strings exists ===
GLIBC_2.37
GLIBC_2.38
GLIBC_2.39
END MPFREE ARMV7 ABI SURVEY
"""
ELF_HEADER = "Class: ELF32\nMachine: ARM\nFlags: 0x5000400, Version5 EABI, hard-float ABI"
PROGRAM_HEADERS = "[Requesting program interpreter: /lib/ld-linux-armhf.so.3]"
VERSIONS = "Version needs section: Name: GLIBC_2.34\nName: GLIBC_2.17"
DYNAMIC = "Shared library: [libc.so.6]\nShared library: [libm.so.6]\n"


def assess(report=BASE_REPORT, h=ELF_HEADER, prog=PROGRAM_HEADERS,
           v=VERSIONS, dyn=DYNAMIC):
    return checker.assess(report, h, prog, v, dyn)


class ArmAbiTests(unittest.TestCase):
    def test_explicit_getconf_version_and_banner(self):
        explicit = BASE_REPORT.replace(
            "getconf not found (libc version remains unverified)",
            "getconf GNU_LIBC_VERSION: glibc 2.39"
        )
        self.assertEqual(checker.extract_target_glibc(explicit), (2, 39, 0))
        banner = BASE_REPORT.replace(
            "GNU libc release banner: unavailable (do not infer version)",
            "GNU libc release banner: GNU C Library (GNU libc) stable release version 2.39."
        )
        self.assertEqual(checker.extract_target_glibc(banner), (2, 39, 0))
        self.assertIsNone(checker.extract_target_glibc(BASE_REPORT))

    def test_absent_getconf_tag_ceiling_is_indeterminate_not_pass(self):
        code, lines = assess()
        self.assertEqual(code, 4)
        self.assertTrue(any("Highest observed libc GLIBC_* string: 2.39" in s for s in lines))
        self.assertTrue(any("NOT QUALIFIED" in s for s in lines))

    def test_explicit_version_and_installed_libraries_pass_necessary_checks(self):
        report = BASE_REPORT.replace(
            "getconf not found (libc version remains unverified)",
            "getconf GNU_LIBC_VERSION: glibc 2.39"
        )
        code, lines = assess(report)
        self.assertEqual(code, 0)
        self.assertTrue(any("NECESSARY CHECKS PASS" in s for s in lines))
        self.assertTrue(any("NOT runtime-qualified" in s for s in lines))

    def test_older_explicit_glibc_is_known_incompatible(self):
        report = BASE_REPORT.replace(
            "getconf not found (libc version remains unverified)",
            "getconf GNU_LIBC_VERSION: glibc 2.31"
        )
        code, lines = assess(report)
        self.assertEqual(code, 2)
        self.assertTrue(any("newer glibc required" in s for s in lines))

    def test_missing_shared_library_is_not_qualified(self):
        report = BASE_REPORT.replace("/lib/libm.so.6 -> (regular file)\n", "")
        report = report.replace(
            "getconf not found (libc version remains unverified)",
            "getconf GNU_LIBC_VERSION: glibc 2.39"
        )
        code, lines = assess(report)
        self.assertEqual(code, 4)
        self.assertTrue(any("libm.so.6" in s for s in lines))

    def test_wrong_abi_or_interpreter_rejected(self):
        code, _ = assess(h=ELF_HEADER.replace("hard-float ABI", "soft-float ABI"))
        self.assertEqual(code, 2)
        code, _ = assess(prog="[Requesting program interpreter: /lib/ld-linux.so.3]")
        self.assertEqual(code, 2)

    def test_highest_glibc_required_and_library_parsing(self):
        self.assertEqual(checker.highest_glibc_required(VERSIONS), (2, 34, 0))
        self.assertEqual(checker.needed_libraries(DYNAMIC), ["libc.so.6", "libm.so.6"])
        self.assertEqual(checker.highest_reported_glibc_symbol(BASE_REPORT), (2, 39, 0))
        self.assertEqual(
            checker.interpreter(PROGRAM_HEADERS), "/lib/ld-linux-armhf.so.3"
        )

    def test_unrecognized_report_rejected(self):
        with self.assertRaises(ValueError):
            assess(report="uninformative text")


if __name__ == "__main__":
    unittest.main()
