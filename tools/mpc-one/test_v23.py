#!/usr/bin/env python3
"""MPFree V23 host preflight: provenance, 60 NEW ARM cases, P1/P3
function-table source contracts, no hardware pass-through, SHA-indexed ZIP.
These do not claim to have executed the real U-Boot guest or Ghidra.
"""
from __future__ import annotations
import ast
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location(
    "mpfree_v23_host_contract",ROOT/"tools/mpc-one/run_v23.py")
v23=importlib.util.module_from_spec(spec)
spec.loader.exec_module(v23)
def fake_previous(path,bad=False):
    entries={
       "V22-STAGE-MANIFEST.json":{"source_P3_SHA":
                                 "0"*64 if bad else v23.SHA_P3},
       "V22-CONCRETE-FINDINGS-AND-85-QUESTIONS.json":{
         "questions":[{"id":"CASE-%02d"%i,"question":"Unanswered"}
                      for i in range(85)]},
       "V22-REAL-OWNER-ARM-RUNTIME.json":{
         "tests_planned":29,"passed":29},
       "deep-p3/V22-p3-SUMMARY.json":{
         "source_hash_verified":True,"owner_sha256":v23.SHA_P3}
    }
    index=[]
    with zipfile.ZipFile(path,"w",compression=zipfile.ZIP_DEFLATED) as z:
        for name,item in entries.items():
            body=json.dumps(item).encode()
            z.writestr(name,body)
            index.append({"name":name,"bytes":len(body),
                          "sha256":hashlib.sha256(body).hexdigest()})
        z.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v22","files":index}))
class V23(unittest.TestCase):
    def test_60_unique_owner_cases_no_repetition(self):
        names=v23.cases()
        self.assertEqual(len(names),60)
        self.assertEqual(len(set(names)),60)
        self.assertEqual(sum(n.startswith("v23_dispatch_") for n in names),48)
        self.assertEqual(sum(n.startswith("v23_config_") for n in names),12)
        self.assertFalse(set(names)&set(v23.V22.cases()))

    def test_real_owner_driver_table_and_intercept(self):
        source=(ROOT/"tools/mpc-one/vendor_probe_v23.c").read_text()
        for token in (
          "V23_DRIVER_TABLE ((uintptr_t)0x0012f8f4u)",
          "V23_OWNER_COMMAND ((uintptr_t)0x0011c4e9u)",
          "V23_OWNER_CONFIG ((uintptr_t)0x0011c913u)",
          "V23_DISPATCH ((uintptr_t)0x0011ca79u)",
          "V23_CFG ((uintptr_t)0x0011c913u)",
          "intercept(0x0011c4e8u,ret,&trap)",
          "v23_nonwriting_command",
          "command_register_unchanged",
          "v23_check_table"
        ):
            self.assertIn(token,source)
        self.assertNotIn("intercept(0x0011ca78u",source)
        self.assertNotIn("intercept(0x0011c912u",source)
        self.assertNotIn("0xffffffedu)",source)
        self.assertNotIn("system(",source)
        self.assertIn("real-controller-configuration-routine",source)

    def test_original_DTB_crosswalk_distinguishes_SD_and_eMMC(self):
        controllers={}
        for name,address,bus,removable,okay in (
            ("mmc0","ff0f0000",8,False,True),
            ("mmc1","ff0c0000",4,True,True),
            ("mmc2","ff0d0000",4,False,False),
            ("mmc3","ff0e0000",None,False,False),
        ):
            controllers[name]={
                "path":"/dwmmc@"+address,
                "reg":{"value":[0,int(address,16),0,16384]},
                "status":{"value":["okay" if okay else "disabled"]},
                **({"bus-width":{"value":[bus]}} if bus else {}),
                **({"cd-gpios":{"value":[1,2,3]}} if removable else {}),
                **({"non-removable":{"value":[]}} if name=="mmc0" else {}),
            }
        data={}
        for board in ("acva","acva_c"):
            data[board]={"nodes":{
                "/aliases":{name:{"value":[v["path"]]}
                            for name,v in controllers.items()},
                **{v["path"]:{k:x for k,x in v.items() if k!="path"}
                   for v in controllers.values()}
            }}
        medium="$"+"{devtype} "+"$"+"{devnum}:"+"$"+"{distro_bootpart} "
        kernel="$"+"{kernel_addr_r}";fdt="$"+"{fdt_addr_r}"
        data["script"]={
            "header_crc_valid":True,"payload_crc_valid":True,
            "script_commands":[
                "load "+medium+kernel+" "+ "$"+"{prefix}zImage || exit",
                "load "+medium+fdt+" "+ "$"+"{prefix}"+"$"+"{fdtfile} || exit",
                "setenv bootargs hello","bootz "+kernel+" - "+fdt,
            ]
        }
        found=v23.dtb_crosswalk(data)
        self.assertEqual(
            found["boards"]["acva"]["mmc0"]["controller_base"],
            "0xff0f0000")
        self.assertEqual(
            found["boards"]["acva_c"]["mmc1"]["controller_base"],
            "0xff0c0000")
        self.assertFalse(found["linux_aliases_prove_uboot_indices"])
        self.assertFalse(found["boot_script"]["contains_saveenv"])
        self.assertTrue(found["boot_script"]["loads_kernel_and_DTB_from_current_device"])

    def test_existing_source_uses_compatible_main_wrappers(self):
        ancestor=(ROOT/"tools/mpc-one/vendor_probe_v22.c").read_text()
        self.assertIn("#ifdef MPFREE_V23_WRAPPING",ancestor)
        self.assertIn("#define main mpfree_v22_legacy_main",ancestor)

    def test_new_Ghidra_trace_has_actual_controller_roots(self):
        source=(ROOT/"tools/ghidra/MPFreeV23DriverCommand.py").read_text()
        tree=ast.parse(source)
        names={x.name:x.lineno for x in tree.body
               if isinstance(x,ast.FunctionDef)}
        for fn in ("trace","fixed_reach","image_u32"):
            self.assertIn(fn,names)
            calls=[node.lineno for node in ast.walk(tree)
                   if isinstance(node,ast.Call)
                   and isinstance(node.func,ast.Name)
                   and node.func.id==fn]
            self.assertTrue(calls)
            self.assertFalse([n for n in calls if n<names[fn]])
        for word in ("0x0012f8f4","0x0011c4e9","0x0011c913",
                     "mmc_driver_table_mapping","source_hash_verified",
                     "MaskROM_and_first_stage_entry",
                     "first_stage_MMC_media_selector"):
            self.assertIn(word,source)

    def test_previous_archive_sha_and_85_question_lineage(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/"V22.zip"
            fake_previous(path)
            prior=v23.verified_prior(path)
            self.assertEqual(len(prior),4)

    def test_previous_archive_wrong_owner_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/"V22.zip"
            fake_previous(path,bad=True)
            with self.assertRaises(ValueError):
                v23.verified_prior(path)

    def test_no_automatically_answered_questions(self):
        with tempfile.TemporaryDirectory() as td:
            directory=Path(td)
            earlier={"V22-CONCRETE-FINDINGS-AND-85-QUESTIONS.json":
                json.dumps({"questions":[{"id":"Q-%02d"%i,
                "question":"Unknown"} for i in range(85)]}).encode()}
            findings=v23.audit(directory,earlier,{})
            self.assertEqual(findings["questions_with_new_evidence"],0)
            self.assertEqual(len(findings["85_research_questions"]),85)
            self.assertTrue(all(not q["question_fully_answered_V23"]
                                for q in findings["85_research_questions"]))

    def test_private_output_sha_archive(self):
        with tempfile.TemporaryDirectory() as td:
            out=Path(td)/"private";out.mkdir()
            v23.write(out/"V23-STAGE-MANIFEST.json",{"preflight":True})
            result,num=v23.package(out)
            self.assertEqual(num,1)
            with zipfile.ZipFile(result) as z:
                self.assertIsNone(z.testzip())
                idx=json.loads(z.read("ZIP_INDEX.json"))
                self.assertEqual(idx["schema"],"mpfree-private-v23")
                item=idx["files"][0]
                self.assertEqual(hashlib.sha256(z.read(item["name"])).hexdigest(),
                                 item["sha256"])

if __name__=="__main__":
    unittest.main(verbosity=2)
