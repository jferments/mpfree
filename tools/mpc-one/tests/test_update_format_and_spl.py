"""Synthetic tests of 3.10 image format, verify-state, XZ and SPL findings."""
import importlib.util
import json
from pathlib import Path
import unittest

SOURCE=Path(__file__).resolve().parents[1]/"synthesize_update_format_and_spl.py"
SPEC=importlib.util.spec_from_file_location("mpfree_update_format_spl_tests", SOURCE)
M=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)

def fixture():
    libsha="a"*64
    splsha="b"*64
    def func(name,pseudocode):
        return {"name":name,"pseudocode":pseudocode}
    formats=(
        "*pcVar3 == 'A'; pcVar3[1] == 'Z'; pcVar3[2] == '0';"
        "pcVar3[3] == 'x'; pcVar3[1] != 'Z'; pcVar3[2] != '0';"
        "pcVar3[3] != '1'; *pcVar3 != -0x30;"
        "pcVar3[1] != '\\r'; pcVar3[2] != -2; pcVar3[3] != -0x13;"
        "pcVar2 = (code *)*puVar4; (*pcVar2)(__ptr);"
    )
    verification=(
        "param_1[7] == 1; param_1[7] != 0; param_2 >> 1;"
        "if (iVar1 == 0) {param_1[7] = 1;}"
        "if (iVar1 == -0xd) {param_1[7] = 2;}"
    )
    writer=(
        "az01_write_bootloader(param_1,param_2[5],param_2[4],param_3);"
        "az01_write_partition(param_1,param_2[1],param_2[5],param_2[4]);"
        "*__s1 == 'x'; __s1[1] == 'z'; __s1[2] == '\\0';"
        "if (0xa00000 < local_3c) return; malloc(local_3c);"
        "lzma_stream_buffer_decode(); lzma_stream_decoder();"
        "pthread_create(); az01_stream_partition(); pthread_join();"
        "ppiVar4 = &local_e0; ppiVar4 = ppiVar4 + 1;"
        "malloc(0x100000); while (ppiVar4 != local_dc + 3);"
    )
    functions={
        "0x0001738c":func("az01_open_image",formats),
        "0x00017554":func("az01_verify_image",verification),
        "0x0001798c":func("az01_write_partition_image",writer),
        "0x00014d00":func("az01_stream_partition",
            "if (UNRECOVERED_JUMPTABLE == (code *)0x0)"
            " return 0xfffffff5;"),
    }
    backends=[
        {"descriptor":name,"present_operations":ops}
        for name,ops in (
            ("ff/42/03",["stream_partition","write_partition"]),
            ("ff/61/7a",["stream_partition","write_partition"]),
            ("ff/06/05",["write_partition"]),
        )
    ]
    return {
        "lib":{
            "role":"libaz01-prog",
            "source_sha256_prechecked_by_launcher":libsha,
            "analyzed_functions":functions},
        "spl":{
            "role":"spl",
            "source_sha256_prechecked_by_launcher":splsha,
            "analyzed_functions":{
                "0xff704120":{"pseudocode":"uVar2 = 1; if (iVar1 == 0x35) uVar2 = 8;"},
                "0xff705350":{"pseudocode":"FUN_ff705342(DAT_ff7053fc);"}}},
        "usb":{"original_library_sha256":libsha,
               "all_source_only_backends_matched":True,
               "backends":backends},
    }

def build(data):
    inverse={value:key for key,value in M.FILES.items()}
    return M.build(lambda file:json.dumps(data[inverse[file]]).encode())

class UpdateFormatAndSPLTests(unittest.TestCase):
    def test_synthetic_original_code_signatures(self):
        report=build(fixture())
        self.assertTrue(report["all_checkable_static_claims_passed"])
        self.assertEqual(report["xz_partition_buffer_count_observed"],4)
        self.assertEqual(report["xz_bootloader_check_max_bytes_on_this_path"],
                         10*1024*1024)
        self.assertFalse(report["FDT_image_must_be_valid_FIT_proven"])
        self.assertFalse(report["SPL_code_8_physical_media_identified"])

    def test_verification_state_transitions_and_option_flags(self):
        f=M.verification_state_step
        self.assertEqual(f(0,0,-13),
            {"callback_called":True,"result":-13,"new_state":2})
        self.assertEqual(f(2,2,-13),
            {"callback_called":False,"result":0,"new_state":2})
        self.assertEqual(f(2,0,-13),
            {"callback_called":True,"result":-13,"new_state":2})
        self.assertEqual(f(0,2,0),
            {"callback_called":True,"result":0,"new_state":1})
        self.assertEqual(f(1,0,-9),
            {"callback_called":False,"result":0,"new_state":1})
        self.assertEqual(f(0,0,-9),
            {"callback_called":True,"result":-9,"new_state":0})

    def test_outside_verified_flag_domain_is_rejected(self):
        for state,option in ((3,2),(-1,0),(0,4)):
            with self.assertRaises(ValueError):
                M.verification_state_step(state,option,0)

    def test_missing_fdt_magic_rejects_format_claim(self):
        data=fixture()
        data["lib"]["analyzed_functions"]["0x0001738c"]["pseudocode"]="none"
        report=build(data)
        self.assertFalse(report["checks"]["recognizes_FDT_magic_d00dfeed"])
        self.assertFalse(report["all_checkable_static_claims_passed"])

    def test_missing_signature_cache_path_rejects_claim(self):
        data=fixture()
        data["lib"]["analyzed_functions"]["0x00017554"][
            "pseudocode"]="no state"
        self.assertFalse(build(data)["checks"]["verification_marks_minus13_as_state2"])

    def test_missing_xz_stream_buffer_fails_closed(self):
        data=fixture()
        key="0x0001798c"
        p=data["lib"]["analyzed_functions"][key]["pseudocode"]
        data["lib"]["analyzed_functions"][key][
            "pseudocode"]=p.replace("malloc(0x100000)","malloc(0x80000)")
        self.assertFalse(build(data)["checks"]["XZ_partition_4_x_1MiB_chunks"])

    def test_wrong_original_usb_provenance_is_fatal(self):
        data=fixture()
        data["usb"]["original_library_sha256"]="f"*64
        with self.assertRaisesRegex(ValueError,"USB backend source"):
            build(data)

    def test_null_rockusb_stream_slot_contributes_to_evidence(self):
        data=fixture()
        for backend in data["usb"]["backends"]:
            if backend["descriptor"]=="ff/06/05":
                backend["present_operations"].append("stream_partition")
        self.assertFalse(build(data)["checks"][
            "RockUSB_like_backend_lacks_stream_slot"])

if __name__=="__main__":
    unittest.main()
