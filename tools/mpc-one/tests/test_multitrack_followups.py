"""Offline synthetic SPL/MMC, power ADC, updater verification/source tests."""
import importlib.util
import json
from pathlib import Path
import unittest

P=Path(__file__).resolve().parents[1]/"synthesize_multitrack_followups.py"
SPEC=importlib.util.spec_from_file_location("mpfree_multitrack_fixtures",P)
M=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)

def fixture():
    splsha="1"*64
    libsha="2"*64
    strings=[{"at":"0xff709a8f","text":"MMC1"},
             {"at":"0xff709a94","text":"MMC2"},
             {"at":"0xff709a99","text":"MMC2_2"}]
    mapping=[{"word":word} for word in (
        "0x00000001","0xff709a8f","0x00000002","0xff709a94",
        "0x00000003","0xff709a99","0xdeadbeef","0xff709aa0")]
    order=[{"word":"0xdeadbeef"} for _ in range(5)]
    values={
        "getvar_power_source_scale":0x10624dd3,
        "getvar_power_source_threshold_1":624999,
        "getvar_power_source_threshold_2":1089999,
        "getvar_power_source_threshold_3":1249999,
    }
    labels=("invalid","battery","ac","invalid-ac")
    slots={key:{"value":"0x%08x" % value} for key,value in values.items()}
    slots.update({"getvar_power_source_label_"+str(i)+"_string":{
        "printable_string":name} for i,name in enumerate(labels,1)})
    getter=("local_20 * aiStack_1c[0];"
            " >> 0x26;"
            "if (DAT_00100fc8 < uVar3) {"
            " if (DAT_00100fd0 < (int)uVar3) {"
            " if (DAT_00100fd8 < (int)uVar3) {} }}")
    libf={}
    def fn(address,name,pseudo):
        libf[address]={"name":name,"pseudocode":pseudo}
    fn("0x0001b9bc","az01_download_image",
       "az01_image_set_onboard_key_lookup(local_1064);"
       " az01_verify_image(local_1064,0); az01_apply_image(iVar2,1);")
    fn("0x00017554","az01_verify_image",
       "param_1[7] == 1; param_2 >> 1; param_1[7] = 1; param_1[7] = 2;")
    fn("0x0001f094","az01_verify_signature",
       "EVP_sha256(); EVP_DigestVerifyInit(ctx); "
       "EVP_DigestVerify(ctx,param_4,0x100,param_2,param_3);")
    fn("0x0001e194","az01_write_image",
       "az01_verify_image(param_1,2);"
       "if ((param_3 & 2) == 0) az01_image_is_compatible(); "
       "az01_prepare_to_write();")
    fn("0x0001798c","az01_write_partition_image",
       "az01_write_bootloader(); az01_write_partition();"
       " az01_stream_partition();")
    for address,name,offset in (
        ("0x00014ac8","az01_commit_bootloader","+ 0x2c"),
        ("0x00014b98","az01_write_bootloader","+ 0x18"),
        ("0x00014c50","az01_write_partition","+ 0x1c"),
    ):
        fn(address,name,"UNRECOVERED_JUMPTABLE; *(param_1 + 0x18) "
           +offset+";")
    return {
        "spl":{
            "role":"spl","source_sha256_prechecked_by_launcher":splsha,
            "analyzed_functions":{
                "0xff705350":{"pseudocode":"FUN_ff705342(DAT_ff7053fc);"},
                "0xff705342":{"pseudocode":"uVar1 = FUN_ff704120(); *param_1 = uVar1;"}},
            "direct_boot_probe":{"extended_stage_evidence":{
                "indirect_handoff_original_word":"0x00000000",
                "opaque_media_table_words":{
                    "device_order":{"entries":order},
                    "device_mapping":{"entries":mapping},
                }},
            },
        },
        "p1":{"sha256":splsha,"strings":strings},
        "uboot":{
            "role":"uboot","source_sha256_prechecked_by_launcher":"3"*64,
            "direct_boot_probe":{"reboot_mode_original_literal_slots":slots},
            "analyzed_functions":{"0x00100ef0":{"pseudocode":getter}},
        },
        "lib":{
            "role":"libaz01-prog",
            "source_sha256_prechecked_by_launcher":libsha,
            "analyzed_functions":libf,
        },
        "elf":{"source_sha256_verified":libsha},
        "links":{
            "owner_sources_sha256":{"libaz01-prog":libsha},
            "target_update_functions":[
                {"imported_by_updater":True,"defined_by_libaz01_prog":True}
                for _ in range(3)],
        },
    }

def run(fixture_data):
    reverse={path:role for role,path in M.PATHS.items()}
    return M.build(lambda path: json.dumps(fixture_data[reverse[path]]).encode())

class MultiTrackTests(unittest.TestCase):
    def test_fully_supported_synthetic_checks(self):
        result=run(fixture())
        self.assertTrue(result["all_static_preconditions_passed"])
        self.assertTrue(result["spl"]["first_order_entry_written_before_iteration"])
        self.assertFalse(result["spl"]["second_callback_runtime_target_proven"])
        self.assertFalse(result["updater"]["actual_backend_write_invoked"])
        self.assertFalse(result["usb_maskrom"]["physical_maskrom_entry_proven"])

    def test_power_source_boundaries_and_uint32_behavior(self):
        for value,expected in (
            (0,"invalid"),(624999,"invalid"),(625000,"battery"),
            (1089999,"battery"),(1090000,"ac"),
            (1249999,"ac"),(1250000,"invalid-ac")):
            self.assertEqual(M.classify_uint32(value),expected)
        self.assertEqual(M.classify_uint32(0xffffffff),"battery")

    def test_fixed_point_multiplier_approximately_divides_by_1000(self):
        self.assertAlmostEqual(M.SCALE/(2**38),.001,places=10)
        self.assertEqual(M.adc_fixed_point(1000,1000),
                         (1000000*M.SCALE)>>38)
        with self.assertRaises(ValueError):
            M.adc_fixed_point(-1,1)

    def test_wrong_source_sha_fails(self):
        data=fixture()
        data["elf"]["source_sha256_verified"]="0"*64
        with self.assertRaisesRegex(ValueError,"source"):
            run(data)

    def test_invalid_media_mapping_label_fails_closed(self):
        data=fixture()
        data["p1"]["strings"][1]["text"]="unverified"
        self.assertFalse(run(data)["spl"]["checks"][
            "three_id_to_mmc_label_pairs"])

    def test_unknown_boot_order_not_misreported_as_priority(self):
        data=fixture()
        data["spl"]["direct_boot_probe"]["extended_stage_evidence"][
            "opaque_media_table_words"]["device_order"]["entries"][0]["word"]="0x1"
        res=run(data)
        self.assertFalse(res["spl"]["checks"][
            "boot_order_five_original_words_are_deadbeef"])
        self.assertFalse(res["spl"]["real_boot_media_priority_proven"])

    def test_missing_image_verification_does_not_count_as_safe_write(self):
        data=fixture()
        data["lib"]["analyzed_functions"]["0x0001e194"][
            "pseudocode"]="az01_prepare_to_write();"
        self.assertFalse(run(data)["updater"]["checks"][
            "normal_image_write_verifies_before_storage_prepare"])

    def test_missing_crypto_verification_prevents_signature_claim(self):
        data=fixture()
        data["lib"]["analyzed_functions"]["0x0001f094"][
            "pseudocode"]="EVP_sha256();"
        self.assertFalse(run(data)["updater"]["checks"][
            "openssl_sha256_256byte_signature_verify"])

if __name__=="__main__":
    unittest.main()
