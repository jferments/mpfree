"""Synthetic SHA-pinned evidence tests for the 3.10 security utilities."""
import importlib.util
from pathlib import Path
import unittest

SOURCE = Path(__file__).resolve().parents[1] / "synthesize_310_security_utilities.py"
SPEC = importlib.util.spec_from_file_location("mpfree_310_security_tests", SOURCE)
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


def fixture():
    def e(path, lines):
        return {"firmware310":{
            "present":True, "sha256":M.SHA[path],
            "small_elf_complete_text":{
                "disassembler_returncode":0,
                "lines":["00000600 <fake>:"] + [" 600:\tnop"]*104 + lines,
            },
        }}
    boot = "/usr/bin/az01-bootloader-sig"
    load = "/usr/sbin/az0x-loadpin-trustpoints"
    return {"elfs":{
        boot:e(boot,[
            "  d20:\tbl\t694 <SHA256_Init@plt>",
            "  d9c:\tbl\t718 <SHA256_Update@plt>",
            "  db8:\tbl\t760 <SHA256_Final@plt>",
            "  b70:\tbl\t6a0 <RC4_set_key@plt>",
            "  b80:\tmov\tr1, #512",
            "  b84:\tbl\t700 <RC4@plt>",
            "  7dc:\tmov\tr2, #32",
            "  7e8:\tbl\t730 <memcmp@plt>",
        ]),
        load:e(load,[
            "  734:\tmov\tr1, #19456",
            "  738:\tmovt\tr1, #16388",
            "  740:\tbl\t6a0 <__ioctl_time64@plt>",
        ]),
    }}


class SecurityUtilitiesTests(unittest.TestCase):
    def test_rc4_sha256_and_loadpin_ioctl(self):
        doc = M.inspect_310(fixture())
        self.assertFalse(doc["vendor_binary_executed"])
        self.assertTrue(doc["bootloader_sig"]["rc4_observed_length_512"])
        self.assertTrue(doc["bootloader_sig"]["memcmp_observed_length_32"])
        self.assertEqual(doc["loadpin_trustpoints"]["ioctl_request"], "0x40044c00")
        self.assertIn("LOADPIN_IOC_SET_TRUSTED_VERITY_DIGESTS",
                      M.markdown(doc))

    def test_wrong_source_sha_is_fatal(self):
        d = fixture()
        d["elfs"]["/usr/bin/az01-bootloader-sig"][
            "firmware310"]["sha256"] = "0"*64
        with self.assertRaises(ValueError):
            M.inspect_310(d)

    def test_missing_ioctl_does_not_claim_verity(self):
        d = fixture()
        lines = d["elfs"]["/usr/sbin/az0x-loadpin-trustpoints"][
            "firmware310"]["small_elf_complete_text"]["lines"]
        lines[-1] = "  740:\tnop"
        result = M.inspect_310(d)
        self.assertIsNone(result["loadpin_trustpoints"]["ioctl_request"])

    def test_missing_disassembly_is_fatal(self):
        d = fixture()
        d["elfs"]["/usr/sbin/az0x-loadpin-trustpoints"][
            "firmware310"]["small_elf_complete_text"]["lines"] = []
        with self.assertRaises(ValueError):
            M.inspect_310(d)

if __name__ == "__main__":
    unittest.main()
