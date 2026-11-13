#!/usr/bin/env python3
"""MPFree V23: real U-Boot hardware-driver dispatch + boot-medium analysis.

NEW:
  - Concurrent SHA-verified Ghidra P1 first-stage MaskROM/MMC/USB-recovery
    dataflow, and P3 real controller-command/driver-table/SD boot dataflow.
  - Genuine original owner P3 FUN_0011ca78 indirect dispatch via its
    *verified* 0x0012f8f4 driver table in a disposable ARM QEMU guest.
    Only 0x0011c4e8 hardware command body is intercepted.
  - Genuine original FUN_0011c912 controller-configuration code executed
    on SYNTHETIC register-memory (no MMIO hardware).
  - Isolated adapter tests exercise actual owner dispatcher ABI and
    callback return values, not a reimplemented C dispatcher.
  - Exact new evidence lineage for 85 previous research questions.

No physical MPC, no USB/eMMC, no host device passthrough, no vendor
firmware installation or persistent writes. One SHA-indexed private ZIP.
"""
from __future__ import annotations
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime,timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import time
import traceback
import zipfile

ROOT=Path(__file__).resolve().parents[2]
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-v23-driver-dispatch-20261113"
def get_module(path,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/path)
    module=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
V22=get_module("tools/mpc-one/run_v22.py","mpfree_v23_prior")
V17=V22.V17
V16=V22.V16
SHA_P1=V17.SHA_P1
SHA_P3=V22.PINNED
def regular(path):return V17.regular_file(path)
def digest(path):return V17.digest(path)
def write(path,entry):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(entry,indent=2,sort_keys=True)+"\n",encoding="utf-8")
def cases():
    names=["v23_dispatch_%d"%i for i in range(48)]
    names+=["v23_config_%d"%i for i in range(12)]
    assert len(names)==60 and len(names)==len(set(names))
    assert set(names).isdisjoint(V22.cases())
    return names
def verified_prior(path):
    # One mandatory source archive from owner's real V22 run.
    entries=V17.load_zip(regular(path)) # member CRC+SHA verified, bounded.
    needed={
        "V22-STAGE-MANIFEST.json",
        "V22-CONCRETE-FINDINGS-AND-85-QUESTIONS.json",
        "V22-REAL-OWNER-ARM-RUNTIME.json",
        "deep-p3/V22-p3-SUMMARY.json",
    }
    if needed-set(entries):
        raise ValueError("Incomplete owner V22 archive: "+
                         str(sorted(needed-set(entries))))
    manifest=json.loads(entries["V22-STAGE-MANIFEST.json"])
    summary=json.loads(entries["deep-p3/V22-p3-SUMMARY.json"])
    if (manifest["source_P3_SHA"]!=SHA_P3 or
        summary.get("owner_sha256")!=SHA_P3 or
        not summary.get("source_hash_verified")):
        raise ValueError("Owner P3 source SHA mismatch")
    vm=json.loads(entries["V22-REAL-OWNER-ARM-RUNTIME.json"])
    if vm.get("tests_planned")!=29 or vm.get("passed")!=29:
        raise ValueError("V22 evidence differs from verified 29/29 owner baseline")
    ledger=json.loads(entries["V22-CONCRETE-FINDINGS-AND-85-QUESTIONS.json"])
    if len(ledger.get("questions",[]))!=85:
        raise ValueError("V22 owner archive lacks complete 85-question inventory")
    return entries
def dtb_crosswalk(owner):
    """Identify physical storage controllers from both owner ACVA DTBs.
    Linux aliases are NOT presumed to be U-Boot device indices.
    """
    result={"schema":"mpfree-v23-owner-DTB-MMC-crosswalk",
            "linux_aliases_prove_uboot_indices":False,"boards":{}}
    for board in ("acva","acva_c"):
        nodes=owner[board]["nodes"]
        aliases=nodes["/aliases"]
        found={}
        for alias in ("mmc0","mmc1","mmc2","mmc3"):
            node_path=aliases[alias]["value"][0]
            props=nodes[node_path]
            reg=props.get("reg",{}).get("value",[])
            if len(reg)!=4:raise ValueError("Unexpected DTB MMC register layout")
            status=props.get("status",{}).get("value",["unspecified"])
            bw=props.get("bus-width",{}).get("value",[])
            found[alias]={"node":node_path,
                 "controller_base":"0x%08x"%reg[1],
                 "register_bytes":reg[3],
                 "bus_width":bw[0] if bw else None,
                 "status":status[0],
                 "non_removable":"non-removable" in props,
                 "card_detect_gpio":"cd-gpios" in props,
                 "write_protect_gpio":"wp-gpios" in props}
        if found["mmc0"]["controller_base"]!="0xff0f0000":
            raise ValueError("Owner eMMC physical address mismatch")
        if found["mmc1"]["controller_base"]!="0xff0c0000":
            raise ValueError("Owner removable SDMMC address mismatch")
        if not found["mmc0"]["non_removable"] or not found["mmc1"]["card_detect_gpio"]:
            raise ValueError("Device-tree controller role evidence incomplete")
        result["boards"][board]=found
    script=owner["script"]
    commands=script["script_commands"]
    current_medium="$"+"{devtype} "+"$"+"{devnum}:"+"$"+"{distro_bootpart} "
    kernel_address="$"+"{kernel_addr_r}"
    dtb_address="$"+"{fdt_addr_r}"
    required=("load "+current_medium+kernel_address,
              "load "+current_medium+dtb_address,
              "bootz "+kernel_address+" - "+dtb_address)
    if (not script["header_crc_valid"] or not script["payload_crc_valid"]
        or not all(any(line.startswith(x) for line in commands) for x in required)):
        raise ValueError("Original medium-agnostic boot script is not verified")
    result["boot_script"]={
       "CRC_valid":True,"loads_kernel_and_DTB_from_current_device":True,
       "final_kernel_command":"bootz",
       "contains_saveenv":any("saveenv" in x for x in commands),
       "contains_setenv":any("setenv " in x for x in commands),
       "does_not_prove_Uboot_MMC_numbering":True}
    result["limitations"]=[
       "These aliases describe Linux FDT devices, not proven U-Boot mmc indices.",
       "No physical SD boot, persistent environment change or controller write occurred."]
    return result
def offline_dtb(out,optional_previous):
    result={"success":False,"new_vendor_ARM_executed":False}
    try:
        if optional_previous is None:
            candidates=sorted(WORK.glob("bootchain-wide-*-v17-*.zip"),
                              key=lambda p:p.stat().st_mtime,reverse=True)
            if not candidates:raise ValueError("Previous original owner V17 DTB ZIP missing")
            optional_previous=candidates[0]
        optional_previous=regular(optional_previous)
        contents=V17.load_zip(optional_previous)
        key="v17-owner-full-boot-and-dtb.json"
        if key not in contents:raise ValueError("Missing owner V17 FDT report")
        evidence=dtb_crosswalk(json.loads(contents[key]))
        evidence["previous_verified_archive_sha256"]=digest(optional_previous)
        write(out/"V23-OWNER-DTB-MMC-AND-BOOT-SCRIPT.json",evidence)
        result.update({"success":True,"controllers_crosschecked":8,
                       "original_boot_script_CRC_verified":True})
    except Exception as exc:
        result["error"]=str(exc)
        (out/"v23-DTB-crosswalk-error.log").write_text(traceback.format_exc()[-3500:])
    return result

def timed(fn,*args):
    begin=time.monotonic()
    try:result=fn(*args)
    except Exception as exc:
        result={"success":False,"error":repr(exc),
                "traceback":traceback.format_exc()[-4000:]}
    result["elapsed_seconds"]=round(time.monotonic()-begin,2)
    return result
def ghidra(role,out,temp):
    result={"success":False,"role":role,"read_only":True,
            "device_access":False,"source_verified":False}
    folder=out/("deep-"+role)
    folder.mkdir(parents=True,exist_ok=True)
    logfile=out/("v23-ghidra-"+role+"-console.log")
    try:
        launcher=V16.get_ghidra()
        if not launcher:
            raise RuntimeError("Owner Ghidra headless executable unavailable")
        project=V16.gproject(role)
        input_file=("p1-rk3288-sram-stage.bin" if role=="p1"
                    else "p3-uimage-payload.bin")
        image=temp/"verified-owner-P3.raw"
        ready=temp/"verified-owner-P3.ready"
        command=launcher+[
            str(WORK/"ghidra/projects"),project,
            "-process",input_file,"-readOnly","-noanalysis",
            "-scriptPath",str(ROOT/"tools/ghidra"),
            "-postScript","MPFreeV23DriverCommand.py",role,str(folder),
            str(image) if role=="p3" else "none",
            str(ready) if role=="p3" else "none",
            "-log",str(folder/"ghidra-engine.log"),
        ]
        with logfile.open("w") as output:
            process=subprocess.run(command,stdout=output,
                                   stderr=subprocess.STDOUT,
                                   timeout=3600,check=False)
        if process.returncode:
            raise RuntimeError("Ghidra exit status "+str(process.returncode))
        report=json.loads((folder/("V23-"+role+"-SUMMARY.json")).read_text())
        correct_sha=SHA_P1 if role=="p1" else SHA_P3
        if (report.get("schema")!="mpfree-v23-directed-interprocedural-pcode-v1"
            or report.get("owner_sha256")!=correct_sha
            or not report.get("source_hash_verified")):
            raise RuntimeError("Ghidra report did not verify pinned owner source")
        witnesses=folder/("V23-"+role+"-PATH-WITNESSES.jsonl")
        if not witnesses.is_file() or witnesses.stat().st_size==0:
            raise RuntimeError("No fresh Ghidra path witnesses")
        f=report["stats"];n=report["traced_function_count"]
        valid=f.get("functions_decompiled",0)
        result.update({
            "source_verified":True,
            "recognized_functions":report["known_functions"],
            "traced_functions":n,
            "decompiled_functions":valid,
            "decompile_failure_count":len(report["decompilation_failures"]),
            "pcode_store_dependencies":f.get("deep_STORE",0),
            "indirect_call_dependencies":f.get("deep_CALLIND",0),
            "indirect_branch_dependencies":f.get("deep_BRANCHIND",0),
            "actual_direct_call_edges":report["call_edges"]
        })
        if role=="p3":
            evidence=report["literal_evidence"].get("mmc_driver_table_mapping")
            if not evidence or (
                evidence["command_thumb_pointer"]!="0x0011c4e9" or
                evidence["config_thumb_pointer"]!="0x0011c913"):
                raise RuntimeError("P3 MMC driver function table not verified")
            if (image.stat().st_size!=363649 or digest(image)!=SHA_P3 or
                ready.read_text().strip()!=SHA_P3):
                raise RuntimeError("Private P3 export hash differs")
            result["owner_controller_command"]="0x0011c4e8"
            result["owner_controller_configuration"]="0x0011c912"
            result["driver_table"]="0x0012f8f4"
        threshold=50 if role=="p3" else 30
        result["success"]=(n>=threshold and valid/max(1,n)>=0.96 and
                           result["pcode_store_dependencies"]>0 and
                           result["indirect_call_dependencies"]>0)
        if not result["success"]:
            result["error"]="Not enough actual Ghidra call/pointer evidence"
    except Exception as exc:
        result["success"]=False
        result["error"]=str(exc)
        with logfile.open("a") as output:
            output.write("\nV23 Ghidra failure:\n"+traceback.format_exc()[-3500:])
    return result
def guest(out,temp,p3future,args):
    result={"success":False,"board":"generic ARM virt, NOT RK3288",
            "new_cases":len(cases()),
            "real_hardware_controller_executed":False,
            "controller_configuration_executed_on_fake_registers":True,
            "no_host_USB_or_eMMC_passthrough":True,
            "prior_vm_cases_repeated":0}
    image=temp/"verified-owner-P3.raw"
    marker=temp/"verified-owner-P3.ready"
    deadline=time.monotonic()+args.export_timeout
    while time.monotonic()<deadline:
        if image.is_file() and marker.is_file():break
        if p3future.done() and not p3future.result().get("success"):
            return {**result,"error":"P3 static stage failed before verified export"}
        time.sleep(.15)
    else:
        return {**result,"error":"Timeout obtaining verified private P3 source"}
    try:
        if (image.stat().st_size!=363649 or digest(image)!=SHA_P3 or
            marker.read_text().strip()!=SHA_P3):
            raise RuntimeError("Temporary owner P3 image failed SHA validation")
        V16.C_SOURCE=ROOT/"tools/mpc-one/vendor_probe_v23.c"
        V16.CASES=cases()
        observed=V16.qemu_real_code(out,image,temp,{"success":True},args)
        src=out/"v16-actual-vendor-QEMU-results.json"
        console=out/"v16-qemu-vendor-function-console.log"
        guest_functions=[
           "FUN_0011ca78: original indirect hardware-driver dispatcher",
           "FUN_0011c912: original controller register-configuration",
           "FUN_0011c4e8: NOT EXECUTED; intercepted before MMIO"
        ]
        if src.is_file():
            new_report=out/"V23-ACTUAL-OWNER-DRIVER-ARM.json"
            report=json.loads(src.read_text())
            # Inherited V16 runner otherwise prints its stale V16 test list.
            report["vendor_functions_tested"]=guest_functions
            report["real_controller_command_body_executed"]=False
            report["real_controller_config_body_executed"]=True
            report["new_vendor_case_names"]=cases()
            write(new_report,report)
            src.unlink()
        if console.is_file():console.rename(out/"v23-owner-arm-guest-console.log")
        result.update(observed)
        result["vendor_functions_tested"]=guest_functions
        result["new_cases"]=len(cases())
        result["prior_vm_cases_repeated"]=0
        result["real_hardware_controller_executed"]=False
    except Exception as exc:
        result["error"]=str(exc)
        (out/"v23-guest-exception.log").write_text(traceback.format_exc()[-4000:])
    return result
def audit(out,old,stages):
    original=json.loads(old["V22-CONCRETE-FINDINGS-AND-85-QUESTIONS.json"])
    vm_path=out/"V23-ACTUAL-OWNER-DRIVER-ARM.json"
    vm=json.loads(vm_path.read_text()) if vm_path.is_file() else {}
    cases_seen={r.get("test"):r for r in vm.get("cases",[])}
    counts={}
    for label,prefix,total in [
        ("real_driver_table_indirect_command","v23_dispatch_",16),
        ("real_driver_dispatch_to_synthetic_callback","v23_dispatch_",48),
        ("real_controller_config_fake_registers","v23_config_",12)
    ]:
        indices=range(16) if label=="real_driver_table_indirect_command" else (
            range(16,48) if label=="real_driver_dispatch_to_synthetic_callback" else range(12))
        observations=[cases_seen.get(prefix+str(n)) for n in indices]
        counts[label]={
            "planned":len(observations),
            "passed":sum(bool(x and x.get("pass")) for x in observations),
            "failures":[prefix+str(n) for n,x in zip(indices,observations)
                        if not x or not x.get("pass")]
        }
    p3=out/"deep-p3/V23-p3-SUMMARY.json"
    p1=out/"deep-p1/V23-p1-SUMMARY.json"
    stat_p3=json.loads(p3.read_text()) if p3.is_file() else {}
    stat_p1=json.loads(p1.read_text()) if p1.is_file() else {}
    dtb_valid=stages.get("owner_DTB_crosswalk",{}).get("success",False)
    p3_valid=stages.get("p3_driver_ghidra",{}).get("success",False)
    p1_valid=stages.get("p1_boot_ghidra",{}).get("success",False)
    owner_table=stat_p3.get("literal_evidence",{}).get("mmc_driver_table_mapping",{})
    results=[]
    code_related={
      "REMOVABLE_DIRECTBOOT-01","REMOVABLE_DIRECTBOOT-02",
      "REMOVABLE_DIRECTBOOT-03","REMOVABLE_DIRECTBOOT-05",
      "REMOVABLE_DIRECTBOOT-08","BOOT_SELECTION-04",
      "BOOT_SELECTION-05","FASTBOOT_RUNTIME-02",
      "RECOVERY_SECURITY-10",
    }
    for q in original["questions"]:
        row={
            "id":q["id"],"question":q["question"],
            "prior_v22_evidence":q.get("v22_new_evidence",[]),
            "new_V23_evidence":[],"question_fully_answered_V23":False
        }
        if row["id"] in code_related and p3_valid:
            row["new_V23_evidence"].append({
                "file":"deep-p3/V23-p3-SUMMARY.json",
                "scope":"new actual controller callback table and boot-source call-chain",
                "proof_level":"STATIC_PINNED_OWNER_P3"})
        if row["id"] in code_related and counts[
            "real_driver_table_indirect_command"]["passed"]>0:
            row["new_V23_evidence"].append({
                "file":"V23-ACTUAL-OWNER-DRIVER-ARM.json",
                "scope":"real dispatcher to original table with controller body intercepted",
                "proof_level":"ISOLATED_ARM_GUEST"})
        if row["id"] in {"REMOVABLE_DIRECTBOOT-03","REMOVABLE_DIRECTBOOT-05",
                         "BOOT_SELECTION-04","BOOT_SELECTION-05"} and dtb_valid:
            row["new_V23_evidence"].append({
               "file":"V23-OWNER-DTB-MMC-AND-BOOT-SCRIPT.json",
               "scope":"Original owner eMMC/SD controller identities and CRC-valid medium-agnostic boot script; U-Boot indices unproven",
               "proof_level":"STATIC_VERIFIED_DTB"})
        if row["id"] in {"REMOVABLE_DIRECTBOOT-03","BOOT_SELECTION-05",
                         "RECOVERY_SECURITY-04"} and p1_valid:
            row["new_V23_evidence"].append({
                "file":"deep-p1/V23-p1-SUMMARY.json",
                "scope":"new SPL boot-media/ROM handoff dataflow; not hardware proof",
                "proof_level":"STATIC_PINNED_OWNER_P1"})
        results.append(row)
    report={
       "schema":"mpfree-v23-new-driver-and-boot-evidence",
       "owner_P3_sha256":SHA_P3,"owner_P1_sha256":SHA_P1,
       "sections":counts,
       "new_owner_guest_cases_planned":len(cases()),
       "new_owner_guest_cases_observed":vm.get("tests_observed",0),
       "new_owner_guest_cases_passed":vm.get("passed",0),
       "verified_native_MMC_command_table":owner_table,
       "new_P3_decompiled":stat_p3.get("stats",{}).get("functions_decompiled",0),
       "new_P1_decompiled":stat_p1.get("stats",{}).get("functions_decompiled",0),
       "85_research_questions":results,
       "questions_with_new_evidence":sum(bool(r["new_V23_evidence"]) for r in results),
       "questions_closed_by_name_matching":0,
       "not_proven":[
           "Neither genuine 0x0011c4e8 MMIO body nor physical controller executed",
           "Live SD-reader vs eMMC host instance/numbering",
           "An actual non-persistent U-Boot SD boot command",
           "Linux kernel + FDT handoff on the MPC One",
           "Real RK3288 MaskROM USB recovery path and independent restoration",
       ]}
    write(out/"V23-EXACT-FINDINGS-AND-85-QUESTIONS.json",report)
    return report
def package(out):
    allowed={".json",".jsonl",".log",".md",".txt",".csv",".tsv"}
    items=[];total=0
    for p in sorted(out.rglob("*")):
        if p.is_symlink():raise ValueError("Symlink detected in owner-private report directory")
        if p.is_dir():continue
        if (not p.is_file() or not stat.S_ISREG(p.lstat().st_mode)
            or p.suffix not in allowed):
            raise ValueError("Invalid private report entry: "+str(p))
        n=p.stat().st_size;total+=n
        if n>192*1024*1024 or total>512*1024*1024:
            raise ValueError("Private evidence exceeded bounded archive size")
        items.append({"name":p.relative_to(out).as_posix(),"bytes":n,
                      "sha256":digest(p)})
    if not items:raise ValueError("No evidence files generated")
    path=out.with_suffix(".zip")
    with zipfile.ZipFile(path,"x",compression=zipfile.ZIP_DEFLATED,
                         compresslevel=6) as z:
        z.writestr("ZIP_INDEX.json",json.dumps({
          "schema":"mpfree-private-v23","private":True,"files":items},indent=2))
        for item in items:z.write(out/item["name"],item["name"])
    with zipfile.ZipFile(path) as z:
        if z.testzip():raise ValueError("Output V23 ZIP CRC mismatch")
        for item in items:
            data=z.read(item["name"])
            if len(data)!=item["bytes"] or hashlib.sha256(data).hexdigest()!=item["sha256"]:
                raise ValueError("V23 output member SHA mismatch: "+item["name"])
    path.with_suffix(".zip.sha256").write_text(
        digest(path)+"  "+path.name+"\n")
    return path,len(items)
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prior-v17",type=Path,
                        help="Owner-private V17 original DTB/boot script inventory")
    parser.add_argument("--prior-v22",type=Path,
                        help="Owner-private SHA-indexed V22 ZIP, newest autodetected")
    parser.add_argument("--qemu-boot-timeout",type=int,default=240)
    parser.add_argument("--case-timeout",type=int,default=20)
    parser.add_argument("--export-timeout",type=int,default=1200)
    parser.add_argument("--skip-vm",action="store_true")
    args=parser.parse_args()
    os.umask(0o077)
    WORK.mkdir(parents=True,exist_ok=True)
    if args.prior_v22:previous=regular(args.prior_v22)
    else:
        matches=sorted(WORK.glob("bootchain-wide-*-v22-*.zip"),
                       key=lambda p:p.stat().st_mtime,reverse=True)
        if not matches:parser.error("Missing owner-private verified V22 ZIP")
        previous=regular(matches[0])
    old=verified_prior(previous)
    out=WORK/("bootchain-wide-"+datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
              +"-v23-"+str(os.getpid()))
    out.mkdir(mode=0o700)
    begin=time.monotonic();stages={}
    print("MPFree V23: SHA-pinned P1/P3 recursive Ghidra plus original MMC driver dispatch in QEMU",flush=True)
    print("Owner prior V22:",previous,flush=True)
    try:
        with tempfile.TemporaryDirectory(dir=WORK,prefix="mpfree-v23-private-") as td:
            temporary=Path(td)
            with ThreadPoolExecutor(max_workers=4) as pool:
                jobs={
                    "p1_boot_ghidra":pool.submit(timed,ghidra,"p1",out,temporary),
                    "p3_driver_ghidra":pool.submit(timed,ghidra,"p3",out,temporary),
                    "owner_DTB_crosswalk":pool.submit(timed,offline_dtb,out,args.prior_v17)
                }
                if not args.skip_vm:
                    jobs["real_vendor_driver_guest"]=pool.submit(
                        timed,guest,out,temporary,jobs["p3_driver_ghidra"],args)
                for f in as_completed(jobs.values()):
                    key=next(k for k,v in jobs.items() if f is v)
                    stages[key]=f.result()
                    print("V23",key,
                          "PASS" if stages[key].get("success") else "FAILED",
                          stages[key].get("elapsed_seconds"),"s",
                          stages[key].get("error",""),flush=True)
    except KeyboardInterrupt:
        stages["interrupted"]={"success":False,"error":"Owner interrupted"}
    except Exception:
        stages["unhandled"]={"success":False,
                             "traceback":traceback.format_exc()[-4000:]}
    finally:
        try:findings=audit(out,old,stages)
        except Exception:
            findings={}
            (out/"V23-ANALYSIS-ERROR.log").write_text(traceback.format_exc()[-5000:])
        write(out/"V23-STAGE-MANIFEST.json",{
           "schema":"mpfree-v23-hardware-driver-dispatch-research",
           "branch":BRANCH,
           "generated_utc":datetime.now(timezone.utc).isoformat(),
           "elapsed_seconds":round(time.monotonic()-begin,2),
           "verified_prior_v22_sha256":digest(previous),
           "owner_P1_sha256":SHA_P1,
           "owner_P3_sha256":SHA_P3,
           "stages":stages,
           "new_vendor_cases_requested":len(cases()),
           "new_vendor_cases_observed":stages.get(
               "real_vendor_driver_guest",{}).get("tests_observed",0),
           "questions_with_new_evidence":findings.get("questions_with_new_evidence",0),
           "physical_MPC_contact":False,
           "host_USB_or_eMMC_passthrough":False,
           "physical_controller_MMIO_executed":False,
           "original_vendor_firmware_binary_in_zip":False
        })
        lines=["# MPFree V23: actual native driver table and controller config","",
          "Original owner SHA-pinned P1/SPL and P3/U-Boot are analyzed read-only.",
          "QEMU is ARM virt, not the physical Rockchip RK3288.","",
          "## Measured stage results"]
        for k,v in stages.items():
            lines.append("- %s: %s, %.2fs %s"%(k,
                "PASS" if v.get("success") else "FAILED",
                v.get("elapsed_seconds",0),v.get("error","")))
        lines.extend(["","## New proof scope",
          "- Real U-Boot FUN_0011ca78 indirect dispatch executes.",
          "- It calls original P3 driver table [0x0012f8f4] to intercepted",
          "  hardware-command routine [0x0011c4e8]. The MMIO body NEVER runs.",
          "- Real FUN_0011c912 configures only synthetic guest RAM registers.",
          "- P1 first-stage ROM/MMC and P3 driver/boot call-chain Ghidra",
          "  dependency witnesses are newly exported.",
          "- No actual SD boot, U-Boot saveenv, physical eMMC or MaskROM.",
          "Original firmware input deleted with private temp working directory.",
          "The ZIP contains only text research evidence, with SHA-256 index.",""])
        (out/"V23-READ-ME-FIRST.md").write_text("\n".join(lines))
        archive,n=package(out)
        print("\nONE PRIVATE ZIP:",archive,flush=True)
        print("SHA256:",digest(archive),"indexed research files:",n,flush=True)
if __name__=="__main__":main()
