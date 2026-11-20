"""Fail-closed synthetic checks for original SPL legacy-image handoff."""
from __future__ import annotations
import importlib.util
import json
from pathlib import Path
import unittest

SOURCE=Path(__file__).resolve().parents[1]/"synthesize_spl_image_handoff.py"
SPEC=importlib.util.spec_from_file_location("mpfree_spl_image_handoff_test",SOURCE)
M=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


def fixture():
    sha_spl="a"*64
    sha_uboot="b"*64
    def pseudocode(body):
        return {"pseudocode":body}
    header=(
        "if (*param_1 == DAT_ff705308) {"
        "uVar3 = param_1[4]; uVar3 = uVar3 << 0x18 | uVar3 >> 0x18;"
        "if (DAT_ff70530c[5] << 0x1f < 0) {"
        "piVar2[3] = uVar3 << 0x18; param_1[5];"
        "} else {DAT_ff70530c[3] = uVar3;} "
        "param_1[3];}")
    return {
        "spl":{
            "role":"spl","source_sha256_prechecked_by_launcher":sha_spl,
            "analyzed_functions":{
                "0xff7052c0":pseudocode(header),
                "0xff70529c":pseudocode(
                    "puVar1[3] = 0x100000; DAT_ff7052b8;"),
                "0xff7054a4":pseudocode("FUN_ff7052c0(DAT_ff705598);"),
                "0xff705310":pseudocode("(**(code **)(param_1 + 0xc))();"),
                "0xff705350":pseudocode("FUN_ff705310(DAT_ff70540c);"),
            },
            "direct_boot_probe":{"extended_stage_evidence":{
                "indirect_handoff_context_address":"0xff70a244",
                "indirect_handoff_function_pointer_slot":"0xff70a250",
                "indirect_handoff_original_word":"0x00000000",
                "second_handoff_pointer_slot_static_xrefs":[
                    {"from":site,"type":"WRITE"}
                    for site in M.EXPECTED_WRITE_SITES
                ],
                "legacy_image_header_original_words":{
                    "image_header_magic_literal_word":"0x56190527",
                    "image_header_context_ptr":"0xff70a244",
                    "image_header_default_context_ptr":"0xff70a244",
                },
            }},
        },
        "p1":{"sha256":sha_spl,
              "literal_hits":{"legacy_uimage_magic":[
                  {"address":"0xff705308"}]}},
        "factory_p3":{"p3":{
            "valid":True,"header_crc32_valid":True,
            "payload_crc32_valid":True,"sha256":sha_uboot,
            "load_address":"0x00100000","entry_address":"0x00000000",
            "legacy_type_id":5,"arch_id":2}},
        "uboot":{"role":"uboot",
                 "source_sha256_prechecked_by_launcher":sha_uboot},
    }


def run(d):
    inv={p:role for role,p in M.INPUTS.items()}
    return M.build(lambda path:json.dumps(d[inv[path]]).encode())


class SPLImageHandoffTests(unittest.TestCase):
    def test_header_to_callback_original_source_dataflow(self):
        out=run(fixture())
        self.assertTrue(out["all_prior_static_evidence_passed"])
        self.assertTrue(out["all_new_literal_checks_passed"])
        self.assertEqual(out["header_offsets_bytes"]["entry"],0x14)
        self.assertEqual(out["header_offsets_bytes"]["load"],0x10)
        self.assertFalse(out["physical_proofs"]["rom_execution_or_maskrom_accepted"])
        self.assertFalse(out["SPL_entry_address_runtime_value_known"])

    def test_legacy_factory_image_entry_zero_but_load_nonzero(self):
        report=run(fixture())
        self.assertEqual(report["factory_P3_verified_legacy_header"][
            "entry_address"],"0x00000000")
        self.assertEqual(report["factory_P3_verified_legacy_header"][
            "load_address"],"0x00100000")

    def test_missing_new_literal_remains_unproven(self):
        source=fixture()
        source["spl"]["direct_boot_probe"]["extended_stage_evidence"][
            "legacy_image_header_original_words"].pop(
                "image_header_context_ptr")
        checks=run(source)["new_literal_pointer_checks"]
        self.assertIsNone(checks[
            "literal_header_context_is_same_as_handoff_context"])

    def test_wrong_header_context_pointer_fails(self):
        source=fixture()
        source["spl"]["direct_boot_probe"]["extended_stage_evidence"][
            "legacy_image_header_original_words"][
                "image_header_context_ptr"]="0xff70a240"
        self.assertFalse(run(source)["all_new_literal_checks_passed"])

    def test_bad_Uboot_crc_or_sha_fails_closed(self):
        source=fixture()
        source["factory_p3"]["p3"]["payload_crc32_valid"]=False
        with self.assertRaisesRegex(ValueError,"Factory P3"):
            run(source)
        source=fixture()
        source["uboot"]["source_sha256_prechecked_by_launcher"]="f"*64
        with self.assertRaisesRegex(ValueError,"Factory P3"):
            run(source)

    def test_changed_parser_header_offsets_fails(self):
        source=fixture()
        body=source["spl"]["analyzed_functions"][
            "0xff7052c0"]["pseudocode"]
        source["spl"]["analyzed_functions"]["0xff7052c0"][
            "pseudocode"]=body.replace("param_1[5]","param_1[8]")
        self.assertFalse(run(source)["checks"][
            "header_entry_word_offset_0x14"])

    def test_handoff_write_xrefs_required(self):
        source=fixture()
        refs=source["spl"]["direct_boot_probe"][
            "extended_stage_evidence"][
                "second_handoff_pointer_slot_static_xrefs"]
        refs.pop()
        self.assertFalse(run(source)["checks"][
            "three_original_instruction_writes_to_handoff_slot"])

    def test_missing_factory_load_header_fails_evidence(self):
        source=fixture()
        source["factory_p3"]["p3"]["load_address"]="0x00000000"
        self.assertFalse(run(source)["checks"][
            "original_factory_p3_image_loads_at_0x00100000"])


if __name__=="__main__":
    unittest.main()
