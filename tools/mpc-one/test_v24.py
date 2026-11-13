#!/usr/bin/env python3
"""V24 non-owner host regressions. End-to-end pinned Ghidra/QEMU still
requires the user's existing local firmware and saved Ghidra projects.
"""
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
    "mpfree_v24_contracts",ROOT/"tools/mpc-one/run_v24.py")
v24=importlib.util.module_from_spec(spec)
spec.loader.exec_module(v24)
def sample_raw():
    raw=bytearray(v24.P3SIZE)
    def word(addr,value):
        at=addr-v24.P3BASE
        raw[at:at+4]=value.to_bytes(4,"little")
    first=v24.P3BASE+0x20000
    names=("mmc","bootz","source","saveenv","ext4load",
           "vendor-secret-code-not-safe-to-display")
    last=first+28*len(names)
    word(0x00110d3c,first)
    word(0x00110d40,last)
    word(0x00110d44,0x24924925)
    for i,name in enumerate(names):
        text=v24.P3BASE+0x30000+64*i
        raw[text-v24.P3BASE:text-v24.P3BASE+len(name)+1]=name.encode()+b"\0"
        location=first+i*28
        words=(text,10,1,v24.P3BASE+0x40001+32*i,0,0,0)
        for w,value in enumerate(words):word(location+4*w,value)
    environment=b"boot_targets=mmc0 mmc1 usb0\x00"
    raw[0x50000:0x50000+len(environment)]=environment
    return bytes(raw)
def fake_v23_zip(path,bad=False):
    files={
      "V23-STAGE-MANIFEST.json":{
        "owner_P1_sha256":v24.SHA_P1,
        "owner_P3_sha256":v24.SHA_P3 if not bad else "0"*64},
      "V23-EXACT-FINDINGS-AND-85-QUESTIONS.json":{
        "85_research_questions":[{"id":"Q-%02d"%i,"question":"Unknown"}
                                 for i in range(85)]},
      "V23-ACTUAL-OWNER-DRIVER-ARM.json":{
        "success":True,"passed":60,"tests_observed":60},
      "deep-p1/V23-p1-SUMMARY.json":{
        "source_hash_verified":True,"owner_sha256":v24.SHA_P1},
      "deep-p3/V23-p3-SUMMARY.json":{
        "source_hash_verified":True,"owner_sha256":v24.SHA_P3},
      "V23-OWNER-DTB-MMC-AND-BOOT-SCRIPT.json":{
        "boards":{},"linux_aliases_prove_uboot_indices":False}}
    indexed=[]
    with zipfile.ZipFile(path,"w",compression=zipfile.ZIP_DEFLATED) as z:
        for name,body in files.items():
            raw=json.dumps(body).encode()
            z.writestr(name,raw)
            indexed.append({"name":name,"bytes":len(raw),
                            "sha256":hashlib.sha256(raw).hexdigest()})
        z.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v23","private":True,"files":indexed}))
class V24Contracts(unittest.TestCase):
    def test_48_new_command_and_MMC_tests(self):
        cases=v24.cases()
        self.assertEqual(len(cases),48)
        self.assertEqual(len(set(cases)),48)
        self.assertEqual(sum(n.startswith("v24_cmd_") for n in cases),32)
        self.assertEqual(sum(n.startswith("v24_mmc_") for n in cases),16)
        self.assertTrue(set(cases).isdisjoint(v24.V23.cases()))

    def test_independent_real_seven_word_descriptor_decoder(self):
        report=v24.decode_command_table(sample_raw())
        self.assertEqual(report["descriptor_count"],6)
        self.assertEqual(report["malformed_descriptor_count"],0)
        self.assertEqual(report["safe_named_commands"]["mmc"]["argc"],10)
        self.assertIn("source",report["safe_named_commands"])
        self.assertIn("saveenv",report["safe_named_commands"])
        self.assertNotIn("vendor-secret-code-not-safe-to-display",
                         json.dumps(report))
        self.assertEqual(report["valid_descriptor_ratio"],1.0)
        self.assertFalse(report["hardware_MMC_index_assigned"])
        defaults=report["static_default_environment_candidates"]["boot_targets"]
        self.assertEqual(len(defaults),1)
        self.assertEqual(defaults[0]["boot_target_tokens"],["mmc0","mmc1","usb0"])
        self.assertTrue(defaults[0]["value_redacted"])
        self.assertNotIn("mmc0 mmc1 usb0",json.dumps(report))

    def test_invalid_table_is_rejected_not_invented(self):
        broken=bytearray(sample_raw())
        at=0x00110d40-v24.P3BASE
        broken[at:at+4]=(0xffffffff).to_bytes(4,"little")
        with self.assertRaises(ValueError):
            v24.decode_command_table(bytes(broken))

    def test_native_ARM_does_not_execute_command_handlers(self):
        s=(ROOT/"tools/mpc-one/vendor_probe_v24.c").read_text()
        for token in (
            "V24_CMD_DIRECT ((uintptr_t)0x00110ccdu)",
            "V24_CMD_WRAPPER ((uintptr_t)0x00110d2du)",
            "V24_MMC_DEVICE ((uintptr_t)0x00104f89u)",
            "v24_exact_descriptor",
            "original-U-Boot-command-descriptor-lookup-never-dispatched",
            "real-owner-MMC-ID-selector-with-synthetic-list-and-no-media",
            "v24_setup_mmc_registry",
        ):self.assertIn(token,s)
        self.assertNotIn("system(",s)
        self.assertNotIn("intercept(0x00110ccc",s)
        self.assertNotIn("intercept(0x001110d2c",s)
        self.assertNotIn("intercept(0x00104f88",s)

    def test_Ghidra_decompiles_real_command_handlers_and_DFU_not_boot(self):
        s=(ROOT/"tools/ghidra/MPFreeV24CommandRegistry.py").read_text()
        tree=ast.parse(s)
        funcs={node.name:node.lineno for node in tree.body
               if isinstance(node,ast.FunctionDef)}
        for name in ("v24_u32","v24_ascii","trace","fixed_reach"):
            self.assertIn(name,funcs)
            calls=[node.lineno for node in ast.walk(tree)
                   if isinstance(node,ast.Call)
                   and isinstance(node.func,ast.Name)
                   and node.func.id==name]
            self.assertTrue(calls)
            self.assertFalse([n for n in calls if n<funcs[name]])
        for term in ("0x00110ccc","0x00110d2c","0x001111e0",
                     "dfu_mmc_is_USB_updater_not_direct_boot",
                     "registered_SD_and_boot_command_handlers",
                     "V24-COMMAND-REGISTRY.json",
                     "source_hash_verified"):
            self.assertIn(term,s)

    def test_prior_V23_sha_and_85_questions(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/"V23.zip";fake_v23_zip(p)
            self.assertEqual(len(v24.prior(p)),6)

    def test_wrong_owner_p3_is_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/"V23-bad.zip";fake_v23_zip(p,bad=True)
            with self.assertRaises(ValueError):v24.prior(p)

    def test_no_auto_closed_questions_without_new_evidence(self):
        with tempfile.TemporaryDirectory() as td:
            old={"V23-EXACT-FINDINGS-AND-85-QUESTIONS.json":json.dumps({
              "85_research_questions":[{"id":"Q-%02d"%i,
                                       "question":"Unknown"} for i in range(85)]
              }).encode()}
            result=v24.reconcile(Path(td),old,{})
            self.assertEqual(result["questions_with_new_evidence"],0)
            self.assertEqual(len(result["original_questions"]),85)
            self.assertTrue(all(not q["fully_resolved"]
                                for q in result["original_questions"]))

    def test_one_private_zip_SHA_check(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/"private";p.mkdir()
            v24.record(p/"V24-STAGE-MANIFEST.json",{"test":True})
            archive,count=v24.package(p)
            self.assertEqual(count,1)
            with zipfile.ZipFile(archive) as z:
                self.assertIsNone(z.testzip())
                data=json.loads(z.read("ZIP_INDEX.json"))
                self.assertEqual(data["schema"],"mpfree-private-v24")
                entry=data["files"][0]
                self.assertEqual(
                    hashlib.sha256(z.read(entry["name"])).hexdigest(),
                    entry["sha256"])
if __name__=="__main__":
    unittest.main(verbosity=2)
