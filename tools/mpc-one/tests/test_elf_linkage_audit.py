"""Synthetic readelf import, export and relocation parsing regression tests."""
import hashlib
import importlib.util
from pathlib import Path
import unittest

P=Path(__file__).resolve().parents[1]/"offline_elf_linkage_audit.py"
S=importlib.util.spec_from_file_location("mpfree_elf_linkage_tests",P)
M=importlib.util.module_from_spec(S)
S.loader.exec_module(M)

FAKE_READ_ELF="""
Dynamic section at offset 0x100 contains 2 entries:
 0x00000001 (NEEDED) Shared library: [libaz01-prog.so]
Relocation section '.rel.plt':
00012f98  000c0016 R_ARM_JUMP_SLOT   00000000   az01_usb_update_mode
00012fac  000d0016 R_ARM_JUMP_SLOT   00000000   az01_apply_image
00012fd0  000e0016 R_ARM_JUMP_SLOT   00000000   az01_download_image
Symbol table '.dynsym' contains:
    12: 00000000     0 FUNC    GLOBAL DEFAULT  UND az01_usb_update_mode
    13: 00000000     0 FUNC    GLOBAL DEFAULT  UND az01_apply_image
    14: 00000000     0 FUNC    GLOBAL DEFAULT  UND az01_download_image
Symbol table '.symtab' contains:
   122: 00001100    44 FUNC    GLOBAL DEFAULT   12 az01_unrelated_defined
"""

class ELFStaticMetadataTests(unittest.TestCase):
    def test_arm_import_relocations_and_soname(self):
        data=M.parse_readelf(FAKE_READ_ELF)
        self.assertIn("libaz01-prog.so",data["needed_sonames"])
        self.assertEqual(len(data["az01_import_symbols"]),3)
        self.assertIn("az01_unrelated_defined",data["az01_export_symbols"])
        self.assertEqual(len(data["az01_relocations"]),3)
        self.assertEqual(data["target_symbol_evidence"][
            "az01_apply_image"]["relocation_offsets"],["0x00012fac"])

    def test_defined_symbol_is_not_misidentified_as_undefined_import(self):
        data=M.parse_readelf(
            "1: 00014040 16 FUNC GLOBAL DEFAULT 6 az01_apply_image\n")
        self.assertNotIn("az01_apply_image",data["az01_import_symbols"])
        self.assertIn("az01_apply_image",data["az01_export_symbols"])

    def test_relocation_without_export_does_not_prove_implementation(self):
        data=M.parse_readelf(FAKE_READ_ELF)
        self.assertTrue(data["target_symbol_evidence"][
            "az01_download_image"]["undefined_import"])
        self.assertFalse(data["target_symbol_evidence"][
            "az01_download_image"]["defined_export"])

    def test_wrong_source_sha_rejected_before_parsing(self):
        data=(b"\x7fELF"+b"\x01\x01"+b"\x00"*12
              +b"\x28\x00"+b"\x00"*60)
        self.assertGreaterEqual(len(data),64)
        with self.assertRaises(ValueError):
            M.validate_elf(data,"0"*64)

    def test_non_arm_rejected_even_with_matching_digest(self):
        data=(b"\x7fELF"+b"\x01\x01"+b"\x00"*12
              +b"\x3e\x00"+b"\x00"*60)
        with self.assertRaises(ValueError):
            M.validate_elf(data,hashlib.sha256(data).hexdigest())

if __name__=="__main__":
    unittest.main()
