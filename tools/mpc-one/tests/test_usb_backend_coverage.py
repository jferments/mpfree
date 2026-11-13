"""Synthetic cross-layer USB backend slot and ABI relocation tests."""
import importlib.util
import json
from pathlib import Path
import unittest

FILE=Path(__file__).resolve().parents[1]/"synthesize_usb_backend_coverage.py"
SPEC=importlib.util.spec_from_file_location("mpfree_usb_backend_coverage_test",FILE)
M=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


def make():
    sha="a"*64
    callbacks={
        "ff/42/03":dict(zip(M.SLOTS,(
            "0x000054b8","0x00005bec","0x00006c14",
            "0x00006dbc","0x00006028","0x00005da8"))),
        "ff/61/7a":{
            "open":"0x0000a170","write_bootloader":"0x0000b35c",
            "write_partition":"0x0000b27c","stream_partition":"0x0000b150",
            "reset":"0x0000a8d0"},
        "ff/06/05":{
            "open":"0x0000c73c","write_bootloader":"0x0000c49c",
            "write_partition":"0x0000c3fc","reset":"0x0000c8c8"},
    }
    funcs={}
    tables=[]
    usbs=[]
    for i,(desc,cbs) in enumerate(callbacks.items()):
        address="0x%08x"%(0x12a74+i*0x100)
        tables.append({"base_vaddr":address,"callbacks":cbs})
        evidence=[]
        for name,slot in M.SLOTS.items():
            raw=cbs.get(name)
            actual=("0x%08x"%(int(raw,16)+0x10000)) if raw else None
            if actual:funcs[actual]={"name":"synthetic"}
            evidence.append({"offset":slot,"resolved_function":actual,
                             "ghidra_function_recognized":bool(actual)})
        usbs.append({
            # Match the actual 3.10 ZIP descriptor representation.
            "usb_descriptor":dict(zip(("class","subclass","protocol"),
                                      ("0x"+part for part in desc.split("/")))),
            "candidate_table_vaddr":address,
            "initializer_vaddr":"0x0001aaaa",
            "initializer_references_table":True,
            "backend_callback_evidence":evidence,
            "upstream_protocol_family":"synthetic-"+desc,
        })
    return {
        "tables":{"source_sha256":sha,"tables":tables},
        "usb":{"verified_source_library_sha256":sha,
               "ghidra_image_base":"0x00010000",
               "verified_backend_interfaces":usbs},
        "lib":{"role":"libaz01-prog",
               "source_sha256_prechecked_by_launcher":sha,
               "ghidra_image_base":"0x00010000",
               "analyzed_functions":funcs},
    }


def run(data):
    inverse={v:k for k,v in M.FILES.items()}
    return M.build(lambda path:json.dumps(data[inverse[path]]).encode())


class BackendCoverageTests(unittest.TestCase):
    def test_three_distinct_callback_capability_tables(self):
        result=run(make())
        self.assertTrue(result["all_source_only_backends_matched"])
        by={b["descriptor"]:b for b in result["backends"]}
        self.assertNotIn("commit_bootloader",by["ff/61/7a"]["present_operations"])
        self.assertNotIn("stream_partition",by["ff/06/05"]["present_operations"])
        self.assertIn("commit_bootloader",by["ff/42/03"]["present_operations"])
        self.assertFalse(result["whole_emmc_recovery_proven"])

    def test_compact_and_0x_prefixed_usb_descriptor_forms_match(self):
        data=make()
        self.assertTrue(run(data)["all_source_only_backends_matched"])
        # Normalize exact same descriptor bytes regardless of JSON formatting.
        for backend in data["usb"]["verified_backend_interfaces"]:
            descriptor=backend["usb_descriptor"]
            for field in ("class","subclass","protocol"):
                descriptor[field]=descriptor[field][2:]
        self.assertTrue(run(data)["all_source_only_backends_matched"])

    def test_malformed_usb_descriptor_fails_closed(self):
        data=make()
        data["usb"]["verified_backend_interfaces"][0][
            "usb_descriptor"]["class"]="not-hex"
        with self.assertRaisesRegex(ValueError,"Invalid USB descriptor field"):
            run(data)

    def test_wrong_relocated_callback_fails_closed(self):
        d=make()
        d["usb"]["verified_backend_interfaces"][0][
            "backend_callback_evidence"][0]["resolved_function"]="0x000154f8"
        self.assertFalse(run(d)["all_source_only_backends_matched"])

    def test_missing_initializer_reference_rejected(self):
        d=make()
        d["usb"]["verified_backend_interfaces"][1][
            "initializer_references_table"]=False
        with self.assertRaisesRegex(ValueError,"source-linked"):
            run(d)

    def test_wrong_original_hash_is_fatal(self):
        d=make()
        d["tables"]["source_sha256"]="f"*64
        with self.assertRaisesRegex(ValueError,"provenance"):
            run(d)

if __name__=="__main__":
    unittest.main()
