"""Synthetic regression cases for SPL boot selection and updater linkage."""
import importlib.util
import json
from pathlib import Path
import unittest

P=Path(__file__).resolve().parents[1] / "synthesize_stage_indirection.py"
S=importlib.util.spec_from_file_location("mpfree_stage_indirection_test",P)
M=importlib.util.module_from_spec(S)
S.loader.exec_module(M)

def fixture():
    spl_probe={
        "role":"spl",
        "original_image_pointers":{
            "candidate_stage_entry_handoff":{"original_word":"0xff705351"},
            "stage_handoff_param":{"original_word":"0xff70a244"},
        },
        "extended_stage_evidence":{
            "first_indirect_call_matches_boot_selector":True,
            "indirect_handoff_function_pointer_slot":"0xff70a250",
            "indirect_handoff_original_word":None,
            "opaque_media_table_words":{
                "device_order":{"table_start":"0xff709c70",
                                "entries":[{"word":"0x%08x"%i} for i in range(5)]},
                "device_mapping":{"table_start":"0xff709c50",
                                  "entries":[{"word":"0x%08x"%i} for i in range(8)]},
            },
        },
    }
    slots={
        "getvar_adc_key_string":{"printable_string":"power-source"},
        "getvar_power_source_threshold_1":{"value":"0x00000010"},
        "getvar_power_source_label_1_string":{
            "value":"0x00002000","printable_string":"demo"}
    }
    return {
        "spl":{
            "role":"spl","source_sha256_prechecked_by_launcher":"1"*64,
            "direct_boot_probe":spl_probe,
            "analyzed_functions":{
                "0xff705208":{"pseudocode":"(*DAT_ff705274)(iVar2,param2);"},
                "0xff705350":{"pseudocode":(
                    "FUN_ff7054a4(); FUN_ff705310(DAT_ff70540c);")},
                "0xff705310":{"pseudocode":"(**(code **)(param_1 + 0xc))();"},
            },
        },
        "uboot":{
            "role":"uboot","source_sha256_prechecked_by_launcher":"2"*64,
            "direct_boot_probe":{"reboot_mode_original_literal_slots":slots},
            "analyzed_functions":{
                "0x00100ef0":{"pseudocode":(
                    "FUN_001147f0(10,0,&local_24);"
                    "FUN_00115ef4(local_24,0);"
                    "FUN_00115f30(local_24,0,&local_20);"
                    "FUN_00115f90(local_24,aiStack_1c);"
                    "local_20 * aiStack_1c[0] * DAT_00100fc4 >> 0x26;")},
                "0x00122d38":{"pseudocode":(
                    "FUN_00100ef0(local_70,auStack_4c,0x40 - iVar1);")},
            },
        },
        "updater":{
            "role":"az01-update","source_sha256_prechecked_by_launcher":"3"*64,
            "direct_boot_probe":{
                "role":"az01-update",
                "original_image_pointers":{
                    k:{"original_word":value,"possible_code_target":value}
                    for k,value in (
                        ("usb_update_mode_trampoline","0x00014028"),
                        ("apply_image_trampoline","0x00014040"),
                        ("download_image_trampoline","0x00014070"),
                    )}},
            "analyzed_functions":{value:{"pseudocode":"halt_baddata();"}
                 for value in ("0x00014028","0x00014040","0x00014070")},
        },
    }

def run(doc):
    inverse={value:key for key,value in M.INPUTS.items()}
    return M.build(lambda path:json.dumps(doc[inverse[path]]).encode())

class StageIndirectionTests(unittest.TestCase):
    def test_first_stage_indirection_reaches_selector(self):
        data=run(fixture())
        self.assertTrue(data["spl"]["first_indirect_boot_selector_proven"])
        self.assertTrue(data["spl"]["checks"]["selector_calls_device_loader"])
        self.assertTrue(data["spl"]["checks"]["second_handoff_indirect_dereference_observed"])
        self.assertTrue(data["spl"]["checks"]["second_handoff_slot_is_context_plus_0x0c"])
        self.assertFalse(data["spl"]["next_handoff_runtime_target_proven"])
        self.assertTrue(data["spl"]["checks"][
            "mapping_table_ends_at_device_order_start"])

    def test_wrong_thumb_pointer_does_not_prove_selector(self):
        data=fixture()
        data["spl"]["direct_boot_probe"]["original_image_pointers"][
            "candidate_stage_entry_handoff"]["original_word"]="0xff705371"
        self.assertFalse(run(data)["spl"]["first_indirect_boot_selector_proven"])

    def test_power_source_adc_identified_without_claiming_voltage_measurement(self):
        data=run(fixture())
        p=data["fastboot_getvar_power_source"]
        self.assertTrue(p["power_source_key_verified"])
        self.assertTrue(p["fastboot_getvar_dispatches_to_adc_probe"])
        self.assertTrue(p["saradc_channel_0_used"])
        self.assertFalse(p["runtime_voltages_measured"])
        self.assertEqual(p["power_source_threshold_original_words"][
            "getvar_power_source_threshold_1"]["value"],"0x00000010")

    def test_linkage_veneer_does_not_equal_updater_implementation(self):
        data=run(fixture())
        for value in data["updater_import_linkage"].values():
            self.assertTrue(value["stub_decompilation_bad_or_missing"])
            self.assertFalse(value["dynamic_library_implementation_resolved"])

    def test_corrupt_source_role_is_fatal(self):
        data=fixture()
        data["updater"]["role"]="uboot"
        with self.assertRaises(ValueError):
            run(data)

if __name__=="__main__":
    unittest.main()
