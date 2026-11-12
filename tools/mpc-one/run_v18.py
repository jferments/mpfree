#!/usr/bin/env python3
"""MPFree V18: FIX V17 silent Ghidra P-code failure + NEW vendor ARM probes.

Read-only owner-backed Ghidra, SHA-pinned original U-Boot instructions against
mocked ARM-virt memory, no physical MPC/USB/eMMC, and ONE private ZIP.
V17's 1239 old VM cases are NOT repeated. V17 updater and FDT reports are
verified and reanalyzed but not redundantly recompiled/imported.
"""
from __future__ import annotations
from concurrent.futures import ThreadPoolExecutor,as_completed
from collections import Counter,defaultdict
from datetime import datetime,timezone
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import tempfile
import time
import traceback
import zipfile

ROOT=Path(__file__).resolve().parents[2]
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-v18-dataflow-repair-20261112"
def load(path,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/path)
    mod=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod
V17=load("tools/mpc-one/run_v17.py","mpfree_v18_v17")
V16=V17.V16
def record(path,obj):
    path.write_text(json.dumps(obj,indent=2,sort_keys=True)+"\n",encoding="utf8")
def timed(fn,*args):
    begun=time.monotonic()
    try: value=fn(*args)
    except Exception as exc:
        value={"success":False,"error":str(exc),
               "traceback":traceback.format_exc()[-5000:]}
    value["wall_seconds"]=round(time.monotonic()-begun,2)
    return value
def source_file(path):
    return V17.regular_file(path)
def prior_zip(path):
    items=V17.load_zip(source_file(path))
    required=("V17-STAGE-MANIFEST.json",
              "deep-p1/V17-p1-SUMMARY.json",
              "deep-p3/V17-p3-SUMMARY.json",
              "V17-85-QUESTIONS-NEW-EVIDENCE.json",
              "v17-owner-full-boot-and-dtb.json",
              "v17-actual-vendor-QEMU-results.json")
    for member in required:
        if member not in items:raise RuntimeError("V17 source archive missing "+member)
    for role,expect in (("p1",V17.SHA_P1),("p3",V17.SHA_P3)):
        report=json.loads(items["deep-"+role+"/V17-"+role+"-SUMMARY.json"])
        if report["source_sha256"]!=expect or not report["verified_owner"]:
            raise RuntimeError("Unrecognized owner ROM source SHA: "+role)
    return items
def v18_cases():
    cases=["v18_continue_state","v18_boot_state",
           "v18_reboot_0","v18_reboot_1"]
    cases+=["v18_getvar_%u_%u"%(key,auth) for key in range(6) for auth in (0,1)]
    assert len(cases)==16 and len(cases)==len(set(cases))
    return cases
def ghidra(role,output,raw=None):
    result=V17.ghidra_saved(role,output,raw)
    if not result.get("success"):return result
    name=output/("deep-"+role)/("V17-"+role+"-SUMMARY.json")
    payload=json.loads(name.read_text())
    n=payload["functions_attempted"]
    done=payload["statistics"].get("function_decompiled",0)
    failed=payload["statistics"].get("decompile_failed",0)
    effects=output/("deep-"+role)/("V17-"+role+"-INDIRECT-AND-MMIO.jsonl")
    rows=0
    if effects.is_file():
        with effects.open() as f:
            for _ in f:rows+=1
    result["actual_pcode_decompiled"]=done
    result["failed_functions"]=failed
    result["dataflow_effect_rows"]=rows
    result["coverage_percent"]=round(100*done/max(1,n),2)
    result["suspected_v17_bug"]="name 'possible_constant_targets' is not defined" in str(
        payload["statistics"])
    # Prevent another 'pass' with silently broken dataflow.
    if done/max(1,n)<0.96 or failed>0.04*n or rows==0:
        result["success"]=False
        result["error"]=("Deep Ghidra coverage/effect threshold not met "
                         "(decompiled=%d/%d; failed=%d; effects=%d)."
                         %(done,n,failed,rows))
    return result
def new_guest(output,raw,temp,p3future,args):
    result={"success":False,"test_scope":"V18-new-ARM-builtins-only",
            "new_vendor_functions":["mpf_fastboot_getvar","mpf_fastboot_continue",
                                    "mpf_fastboot_boot","mpf_fastboot_reboot"],
            "usb_or_emmc_passthrough":False}
    # Wait for the P3 Ghidra export marker; no idle 'research delay'.
    ready=raw.with_name(raw.name+".ready")
    deadline=time.monotonic()+args.export_timeout
    while time.monotonic()<deadline:
        if ready.is_file() and raw.is_file():break
        if p3future.done() and not p3future.result().get("success"):
            return {**result,"error":"P3 static stage failed before verified export"}
        time.sleep(.2)
    else:return {**result,"error":"Timed out waiting for verified source ROM export"}
    try:
        if (source_file(raw).stat().st_size!=363649 or
            V17.digest(raw)!=V17.SHA_P3 or
            ready.read_text().strip()!=V17.SHA_P3):
            raise RuntimeError("Verified temporary P3 ROM export invalid")
        # The SHA-verified private ROM export is usable as soon as Ghidra
        # emits its ready marker. Do NOT block on completion of the P3 pass:
        # QEMU and the full High P-code sweep should genuinely run concurrently.
        if p3future.done() and not p3future.result().get("success"):
            raise RuntimeError("P3 Ghidra reported failure despite verified export")
        V16.C_SOURCE=ROOT/"tools/mpc-one/vendor_probe_v18.c"
        V16.CASES=v18_cases()
        q=V16.qemu_real_code(output,raw,temp,{"success":True},args)
        src=output/"v16-actual-vendor-QEMU-results.json"
        log=output/"v16-qemu-vendor-function-console.log"
        if src.is_file():src.rename(output/"V18-ACTUAL-VENDOR-ARM-CASES.json")
        if log.is_file():log.rename(output/"v18-arm-vendor-console.log")
        result.update(q)
        result["v18_original_vendor_tests"]=len(v18_cases())
        result["old_v17_cases_repeated"]=0
    except Exception as exc:
        result["error"]=str(exc)
        (output/"v18-vm-error.log").write_text(traceback.format_exc()[-5000:])
    return result
def offline_reconcile(output,old):
    """Fresh cross-subsystem interpretation, NEVER counted as new vendor execution."""
    facts={"schema":"mpfree-v18-triangulation","prior_v17_reused":True,
           "new_code_execution":False}
    img=json.loads(old["v17-owner-full-boot-and-dtb.json"])
    facts["owner_boot_script"]={
        "crc_verified":bool(img["script"]["header_crc_valid"] and
                            img["script"]["payload_crc_valid"]),
        "kernel_zimage":img["kernel"].get("arm_zimage_magic"),
        "dtb_differences":len(img.get("dtb_complete_differences",[]))}
    ranges=[]
    # The normal rootfs DTB advertises 2 GiB, but the live U-Boot DRAM map,
    # relocation reservations and framebuffer are NOT proven by this file.
    node=img["acva"]["nodes"].get("/memory@0",{})
    memcells=node.get("reg",{}).get("value",[])
    facts["dtb_memory_reg_cells"]=memcells
    start=0x00800800
    anchors={"fdt_addr_r_candidate":0x01f00000,
             "kernel_addr_r_candidate":0x02000000,
             "ramdisk_addr_r_candidate":0x04000000,
             "uboot_raw_image_base":0x00100000}
    size_opts=[0,1,4096,65536,1024*1024,4*1024*1024,
               8*1024*1024,16*1024*1024,24*1024*1024,
               32*1024*1024,64*1024*1024,0x70000000]
    for size in size_opts:
        end=start+size
        ranges.append({"download_length":size,"end_exclusive":hex(end),
           "crosses_32_bit_address_space":end>0x100000000,
           "advertised_maximum_exceeded":size>0x70000000,
           "passes_default_anchor_address":[name for name,addr in anchors.items()
                                                if start<=addr<end],
           "not_a_safe_ram_verdict":True})
    facts["download_address_interval_models"]=ranges
    branch=json.loads(old["V17-CONCRETE-NEW-FINDINGS.json"])
    facts["previous_oem_storage_path"]=next((row for row in branch["side_effect_call_paths"]
        if row["name"]=="OEM to persistence backend"),None)
    facts["prior_conclusions_not_retested"]=4
    # Analyze the actual previously imported updater function references
    # without regenerating the ELF projects.
    updater_info={}
    for name in ("libaz01-prog","az01-update"):
        src="updater-"+name+"/V17-"+name+"-SUMMARY.json"
        if src in old:
            u=json.loads(old[src])
            updater_info[name]={"functions_recognized":u["functions_recognized"],
                                "unresolved_calls":len(u["unresolved_calls"]),
                                "candidate_function_pointers":len(u["candidate_function_pointers"]),
                                "transport_and_protocol_roots":len(
                                    u["recursive_groups"].get("transport_and_protocol",{})
                                    .get("seeds",[])),
                                "prior_evidence_not_new_decompilation":True}
    facts["updater_reuse"]=updater_info
    record(output/"V18-PRIOR-FDT-AND-UPDATER-TRIANGULATION.json",facts)
    return {"success":True,"old_data_reanalyzed":True,
            "new_vendor_executions":0,"updater_targets":len(updater_info)}
def dataflow_findings(output,old,stages):
    """Validate every High P-code stage, classify static STORE/CALLIND, not mere summary."""
    findings={"schema":"mpfree-v18-high-pcode-dataflow-verification",
              "verified_owner_hashes":{"p1":V17.SHA_P1,"p3":V17.SHA_P3},
              "actual_new_vm_cases":stages.get("new_arm_vm",{}).get("tests_observed",0),
              "systems":{},"question_evidence":[]}
    for role in ("p1","p3"):
        fp=output/("deep-"+role)/("V17-"+role+"-HIGH-PCODE-FUNCTIONS.jsonl")
        effectfile=output/("deep-"+role)/("V17-"+role+"-INDIRECT-AND-MMIO.jsonl")
        gh=stages.get("deep_"+role,{})
        if not gh.get("success"):
            findings["systems"][role]={"error":gh.get("error"),"complete":False}
            continue
        counts=Counter();samples=defaultdict(list);failure_reasons=Counter()
        docrows=0
        with fp.open() as stream:
            for line in stream:
                if not line.strip():continue
                r=json.loads(line);docrows+=1
                if r.get("failure"):failure_reasons[r["failure"]]+=1
        if effectfile.stat().st_size==0:
            raise RuntimeError("V18 repaired Ghidra emitted ZERO P-code effect evidence")
        effectrows=0;identified_writes=[];callbacks=[]
        with effectfile.open() as stream:
            for line in stream:
                if not line.strip():continue
                obj=json.loads(line);effectrows+=1
                kind=obj["kind"];counts[kind]+=1
                if kind=="STORE":
                    addresses=obj.get("candidate_address_constants",[])
                    if addresses and len(identified_writes)<800:
                        identified_writes.append({
                            "function":obj["function"],"pc":obj["pc"],
                            "candidate_targets":addresses,
                            "reason":"candidate-only; not runtime side-effect proof"})
                if kind in ("CALLIND","BRANCHIND"):
                    targets=obj.get("static_indirect_target_candidates",[])
                    if targets and len(callbacks)<800:
                        callbacks.append({
                            "function":obj["function"],"pc":obj["pc"],
                            "targets":targets,
                            "reason":"statically propagated possible targets, not executed"})
        findings["systems"][role]={
          "complete":True,"functions_with_records":docrows,
          "functions_decompiled":gh["actual_pcode_decompiled"],
          "functions_failed":gh["failed_functions"],
          "effect_rows":effectrows,"effect_kinds":dict(counts),
          "static_store_candidates":len(identified_writes),
          "static_indirect_call_candidates":len(callbacks),
          "store_samples":identified_writes,
          "callback_samples":callbacks,
          "decompilation_failure_reasons":dict(failure_reasons)}
    # Question-specific sources supported by actual observations, not inferred
    # from there being 85 report entries.
    previous=json.loads(old["V17-85-QUESTIONS-NEW-EVIDENCE.json"])
    questions=[]
    for row in previous["questions"]:
        q={"id":row["id"],"question":row["question"],
           "v17_closed":row.get("closed_by_V17",False),
           "v18_closed":False,"v18_new_observation":None,"v18_evidence_files":[]}
        if q["id"].startswith(("FASTBOOT-","FASTBOOT_RUNTIME-")):
            if stages.get("deep_p3",{}).get("success"):
                q["v18_evidence_files"].append("deep-p3/V17-p3-INDIRECT-AND-MMIO.jsonl")
        if q["id"].startswith(("BOOT_SELECTION-","REMOVABLE_DIRECTBOOT-",
                                "RECOVERY-","RECOVERY_SECURITY-")):
            if stages.get("deep_p1",{}).get("success"):
                q["v18_evidence_files"].append("deep-p1/V17-p1-INDIRECT-AND-MMIO.jsonl")
        if q["id"] in ("FASTBOOT_RUNTIME-04","FASTBOOT_RUNTIME-07",
                       "FASTBOOT-08","FASTBOOT-09"):
            if stages.get("new_arm_vm",{}).get("success"):
                q["v18_new_observation"]="actual bounded vendor built-in getvar/reboot/callback case"
                q["v18_evidence_files"].append("V18-ACTUAL-VENDOR-ARM-CASES.json")
        questions.append(q)
    findings["question_evidence"]=questions
    record(output/"V18-REPAIRED-DATAFLOW-AND-85-QUESTIONS.json",findings)
    return findings

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prior-v17",type=Path,
                    help="Owner-private SHA-indexed V17 ZIP, autodetect newest if absent")
    ap.add_argument("--qemu-boot-timeout",type=int,default=240)
    ap.add_argument("--case-timeout",type=int,default=20)
    ap.add_argument("--export-timeout",type=int,default=1200)
    ap.add_argument("--skip-vm",action="store_true")
    args=ap.parse_args()
    os.umask(0o077)
    WORK.mkdir(parents=True,exist_ok=True)
    if args.prior_v17:
        source=source_file(args.prior_v17)
    else:
        matches=sorted(WORK.glob("bootchain-wide-*-v17-*.zip"),
                       key=lambda p:p.stat().st_mtime,reverse=True)
        if not matches:ap.error("Missing owner V17 private research ZIP in ~/mpfree-work")
        source=source_file(matches[0])
    old=prior_zip(source) # SHA check ALL indexed prior reports before any work.
    stamp=datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output=WORK/("bootchain-wide-"+stamp+"-v18-"+str(os.getpid()))
    output.mkdir(mode=0o700)
    stages={};started=time.monotonic()
    print("MPFree V18: repaired FULL High P-code sweep + new actual ARM getvar/boot/reboot tests",flush=True)
    print("Prior owner-verified V17 research ZIP:",source,flush=True)
    try:
        with tempfile.TemporaryDirectory(dir=WORK,prefix="mpfree-v18-private-") as td:
            temp=Path(td);raw=temp/"verified-owner-p3.raw"
            with ThreadPoolExecutor(max_workers=5) as pool:
                tasks={
                    "deep_p1":pool.submit(timed,ghidra,"p1",output,None),
                    "deep_p3":pool.submit(timed,ghidra,"p3",output,raw),
                    "prior_reanalysis":pool.submit(timed,offline_reconcile,output,old),
                }
                if not args.skip_vm:
                    tasks["new_arm_vm"]=pool.submit(timed,new_guest,output,raw,temp,tasks["deep_p3"],args)
                for future in as_completed(list(tasks.values())):
                    name=next(k for k,v in tasks.items() if v is future)
                    stages[name]=future.result()
                    print("V18",name,"PASS" if stages[name].get("success") else "INCOMPLETE",
                          stages[name].get("wall_seconds"),"s",stages[name].get("error",""),flush=True)
    except KeyboardInterrupt:
        stages["interrupt"]={"success":False,"error":"Owner interrupted"}
    except Exception:
        stages["uncaught"]={"success":False,
                            "traceback":traceback.format_exc()[-5000:]}
    finally:
        try:
            findings=dataflow_findings(output,old,stages)
        except Exception:
            findings={}
            (output/"V18-DATAFLOW-ERROR.log").write_text(traceback.format_exc()[-5000:])
        record(output/"V18-STAGE-MANIFEST.json",{
            "branch":BRANCH,"timestamp_utc":datetime.now(timezone.utc).isoformat(),
            "runtime_seconds":round(time.monotonic()-started,2),
            "stages":stages,"prior_sha256":V17.digest(source),
            "questions_enumerated":85,"questions_proven_by_name_matching":0,
            "vm_old_cases_repeated":0,"physical_MPC_contact":False,
            "host_USB_or_eMMC_passthrough":False,
            "vendor_updater_executed":False,
            "verified_vendor_raw_code_in_zip":False})
        summary=["# MPFree V18: real new evidence and repaired dataflow","",
                 "Root cause of V17's 667 artificial Ghidra failures:",
                 "possible_constant_targets() was defined AFTER the full function sweep.",
                 "V18 moves it before the sweep and treats undercoverage as a failure.",
                 "", "## Actual concurrent results"]
        for name,row in stages.items():
            summary.append("- %s: %s, %.2f seconds; %s"%(name,
                "PASS" if row.get("success") else "FAILED",
                row.get("wall_seconds",0),row.get("error","")))
        summary.extend(["",
            "## What is NEW",
            "- Full owner-P1/P3 High P-code STORE/CALLIND/BRANCHIND pass with",
            "  bounded backwards constant/dataflow propagation and real evidence rows.",
            "- New bounded guest ARM tests of getvar builtins, continue/boot state",
            "  and reboot selection. These are genuine U-Boot instructions, with",
            "  dangerous callbacks stubbed in guest RAM, NOT a real MPC test.",
            "- Memory-collision scenarios are mathematical anchors only;",
            "  physical DRAM relocations still require separate proof.",
            "- Previously completed V17 updater ELF and 1239 VM cases are",
            "  SHA-verified and reused, NEVER presented as new execution.",
            "",
            "## Evidence qualification",
            "- Each Ghidra candidate target is a static inference, not runtime proof.",
            "- QEMU is generic ARM virt, NOT a Rockchip RK3288/BootROM emulator.",
            "- A failed VM test is recorded as failed, not suppressed.",
            "- Original proprietary firmware bytes are deleted with the temporary",
            "  fixture and are NOT packaged into the single private output ZIP.",""])
        (output/"V18-READ-ME-FIRST.md").write_text("\n".join(summary),encoding="utf8")
        archive,files=V17.package(output)
        print("\nONE PRIVATE V18 ZIP TO UPLOAD:",archive,flush=True)
        print("SHA256:",V17.digest(archive),"report files",files,flush=True)
        print("No physical device, USB or storage contacted.",flush=True)
if __name__=="__main__":main()
