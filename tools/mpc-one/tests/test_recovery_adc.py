"""Independent synthetic checks for stock MPC recovery-button ADC evidence."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest

MODULE = Path(__file__).resolve().parents[1] / "synthesize_recovery_adc.py"
SPEC = importlib.util.spec_from_file_location("mpfree_recovery_adc_synthetic", MODULE)
LAB = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = LAB
SPEC.loader.exec_module(LAB)


def source_fixture(pointer=True):
    read = LAB.ARM_SEQUENCE
    arm = [{"address":at[2:],"text":instruction}
           for at,instruction in read.items()]
    string_rows = [
        {"text":message,"xrefs":[{"from":at[2:],"type":"PARAM"}]}
        for at,message in LAB.ERROR_MESSAGES.items()
    ]
    slots = {
        "mode_register_pointer":{"value":"0xff730094"},
        "mode_prefix_constant":{"value":"0x5242c300"},
        "recovery_flag_pointer":{"value":"0x001462fc"},
    }
    if pointer:
        slots["board_setup_recovery_flag_pointer"] = {
            "value":"0x001462fc"}
    return {
        "uboot":{
            "role":"uboot", "source_sha256_prechecked_by_launcher":"a"*64,
            "direct_boot_probe":{"reboot_mode_original_literal_slots": slots},
            "analyzed_functions":{
                "0x00100fe4":{
                    "machine_instruction_evidence":arm,
                    "pseudocode":(
                        "FUN_001147f0(10,0,&local_20);"
                        "if (local_1c[0] < 100) { *DAT_00101154 = 1; }"
                    ),
                },
                "0x001011e0":{"pseudocode":(
                    "if (*PTR_DAT_001013ac != 0) "
                    "FUN_00108560(PTR_s_dorecovery_001013b4,PTR_DAT_001013b0);"
                )},
                "0x00100ef0":{"pseudocode":(
                    "FUN_001147f0(10,0,&local_24);"
                    "FUN_00115ef4(local_24,0);"
                    "FUN_00115f30(local_24,0,&local_20);"
                )},
            },
        },
        "strings":{"strings": string_rows},
        "fdt":{"uboot_stage_fdt":[{"selected_dts_lines":[
            'adc0 = "/saradc@ff100000";',
            'saradc@ff100000 {',
            'compatible = "rockchip,saradc";',
            'reg = <0xff100000 0x100>;',
            'status = "okay";',
            '};',
        ]}]},
        "env":{"p2":{"boot_related_variables":{
            "az01_bootcmd":"if test x$" + "{dorecovery} = x1; run az01_recovercmd",
            "az01_recovercmd":"fastboot 0",
        }}},
    }


def build(data):
    inverse = {value:key for key,value in LAB.FILES.items()}
    return LAB.build(lambda filename: json.dumps(data[inverse[filename]]).encode())


class RecoveryADCTests(unittest.TestCase):
    def test_original_independent_evidence_proves_adc_logic(self):
        report = build(source_fixture())
        self.assertTrue(report["all_existing_cross_checks_passed"])
        self.assertTrue(report["all_pointer_checks_passed"])
        self.assertEqual(report["adc_recovery_threshold_unsigned_max_inclusive"], 99)
        self.assertEqual(report["adc_trigger_channel_argument"], 1)
        self.assertFalse(report["physical_device_access"])
        self.assertFalse(report["maskrom_restore_proven"])

    def test_missing_board_pointer_must_remain_unknown(self):
        report = build(source_fixture(pointer=False))
        self.assertTrue(report["all_existing_cross_checks_passed"])
        self.assertFalse(report["all_pointer_checks_passed"])
        self.assertIsNone(report["checks"][
            "recovery_adc_and_board_export_same_runtime_flag_pointer"])

    def test_wrong_board_pointer_is_not_accepted(self):
        data=source_fixture()
        data["uboot"]["direct_boot_probe"]["reboot_mode_original_literal_slots"][
            "board_setup_recovery_flag_pointer"]["value"]="0x001462f8"
        report=build(data)
        self.assertIs(report["checks"][
            "recovery_adc_and_board_export_same_runtime_flag_pointer"],False)

    def test_wrong_arm_condition_prevents_adc_claim(self):
        data=source_fixture()
        for entry in data["uboot"]["analyzed_functions"]["0x00100fe4"][
                "machine_instruction_evidence"]:
            if entry["address"]=="0010110e":
                entry["text"]="ittt gt"
        report=build(data)
        self.assertFalse(report["all_existing_cross_checks_passed"])

    def test_disabled_saradc_fdt_fails_closed(self):
        data = source_fixture()
        self.assertTrue(build(data)["checks"]["original_uboot_fdt_saradc_enabled"])
        dts = data["fdt"]["uboot_stage_fdt"][0]["selected_dts_lines"]
        dts[dts.index('status = "okay";')] = 'status = "disabled";'
        self.assertFalse(build(data)["checks"]["original_uboot_fdt_saradc_enabled"])

    def test_missing_diagnostic_breaks_cross_validation(self):
        data=source_fixture()
        data["strings"]["strings"].pop(0)
        report=build(data)
        self.assertFalse(report["checks"][
            "three_diagnostic_xrefs_prove_recovery_adc"])

    def test_wrong_environment_prevents_fastboot_chain_claim(self):
        data=source_fixture()
        data["env"]["p2"]["boot_related_variables"]["az01_recovercmd"]="echo No"
        report=build(data)
        self.assertFalse(report["checks"]["saved_dorecovery_runs_stock_fastboot"])

if __name__ == "__main__":
    unittest.main()
