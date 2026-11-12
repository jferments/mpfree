#!/usr/bin/env python3
"""V19 deep owner-private MPC boot research.

New evidence, NOT a synthetic Fastboot model:
- Concurrent SHA-pinned read-only Ghidra P1 and P3 typed interprocedural
  High P-code dependency witnesses, extending beyond V18's constant-only scan.
- Actual U-Boot recovery mode and ADC policy executed on ARM QEMU virt,
  with guest-only mocked PMU/ADC and hardware callbacks.
- Isolated real vendor getvar environment branch with safe null/empty/value
  lookup stubs, investigating the two V18 guest crashes.
- Carry 85 question IDs with exact new evidence classification, not aliases.
- One SHA-256 indexed owner-private ZIP. No physical MPC/USB/eMMC access.
"""
from __future__ import annotations
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor,as_completed
from datetime import datetime,timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import time
import traceback
import zipfile

ROOT=Path(__file__).resolve().parents[2]
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-v19-recovery-getvar-20261112"
def module(path,alias):
    spec=importlib.util.spec_from_file_location(alias,ROOT/path)
    instance=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(instance)
    return instance
V18=module("tools/mpc-one/run_v18.py","mpfree_v19_v18")
V17=V18.V17
V16=V18.V16
SHA_P1=V17.SHA_P1
SHA_P3=V17.SHA_P3
def write(path,value):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(value,indent=2,sort_keys=True)+"\n",encoding="utf8")
def regular(path):
    return V17.regular_file(path)
def timed(fn,*args):
    start=time.monotonic()
    try:result=fn(*args)
    except Exception as exc:
        result={"success":False,"error":repr(exc),
                "traceback":traceback.format_exc()[-3500:]}
    result["wall_seconds"]=round(time.monotonic()-start,2)
    return result
def selected_cases():
    # 12 owner code calls to recovery mode/ADC, including exact threshold
    # boundary 99/100, and 12 new environment-dependent getvar cases.
    recovery=[
       "v19_recovery_mode_"+str(v) for v in (0,1,3,9,255)
    ]+["v19_recovery_badprefix"]+[
       "v19_recovery_adc_"+str(v) for v in (0,1,99,100,101,255)]
    env=[
       "v19_getvar_%d_%d_%d"%(variant,auth,mode)
       for variant in (4,5) for auth in (0,1) for mode in (0,1,2)]
    cases=recovery+env
    assert len(cases)==24 and len(cases)==len(set(cases))
    assert not (set(cases)&set(V17.tested_cases()))
    return cases
def verified_prior(path):
    items=V17.load_zip(regular(path))
    for name in ("V18-STAGE-MANIFEST.json",
                 "deep-p1/V17-p1-SUMMARY.json",
                 "deep-p3/V17-p3-SUMMARY.json",
                 "V18-REPAIRED-DATAFLOW-AND-85-QUESTIONS.json",
                 "V18-ACTUAL-VENDOR-ARM-CASES.json"):
        if name not in items:
            raise ValueError("Incomplete prior owner-private V18 archive: "+name)
    for role,sha in (("p1",SHA_P1),("p3",SHA_P3)):
        report=json.loads(items["deep-"+role+"/V17-"+role+"-SUMMARY.json"])
        if not report.get("verified_owner") or report.get("source_sha256")!=sha:
            raise ValueError("Owner firmware SHA mismatch in V18 lineage")
    ledger=json.loads(items["V18-REPAIRED-DATAFLOW-AND-85-QUESTIONS.json"])
    if len(ledger.get("question_evidence",[]))!=85:
        raise ValueError("V18 prior archive does not carry exactly 85 questions")
    return items
def ghidra(role,dest,temp):
    result={"success":False,"stage":"ghidra_"+role,
            "source_SHA_verified":False,"guest_firmware_executed":False}
    output=dest/("deep-"+role)
    output.mkdir(parents=True,exist_ok=True)
    log=dest/("v19-ghidra-"+role+"-console.log")
    try:
        launch=V16.get_ghidra()
        if not launch:raise RuntimeError("No local PyGhidra launcher")
        project=V16.gproject(role)
        file=("p1-rk3288-sram-stage.bin" if role=="p1"
              else "p3-uimage-payload.bin")
        private=temp/"owner-verified-p3.raw"
        ready=temp/"owner-verified-p3.ready"
        args=[role,str(output),
              str(private) if role=="p3" else "none",
              str(ready) if role=="p3" else "none"]
        cmd=launch+[str(WORK/"ghidra/projects"),project,
                    "-process",file,"-readOnly","-noanalysis",
                    "-scriptPath",str(ROOT/"tools/ghidra"),
                    "-postScript","MPFreeV19PathWitness.py"]+args+[
                    "-log",str(output/"ghidra-engine.log")]
        with log.open("w") as handle:
            proc=subprocess.run(cmd,stdout=handle,stderr=subprocess.STDOUT,
                                check=False,timeout=3600)
        if proc.returncode:
            raise RuntimeError("Ghidra "+role+" exited "+str(proc.returncode))
        path=output/("V19-"+role+"-SUMMARY.json")
        data=json.loads(path.read_text())
        sha=SHA_P1 if role=="p1" else SHA_P3
        if (data.get("schema")!="mpfree-v19-directed-interprocedural-pcode-v1" or
            not data.get("source_hash_verified") or data.get("owner_sha256")!=sha):
            raise RuntimeError("P%d owner SHA or source report mismatch"%(1 if role=="p1" else 3))
        stats=data["stats"];size=data["traced_function_count"]
        compiled=stats.get("functions_decompiled",0)
        actual_writes=stats.get("deep_STORE",0)
        actual_indirect=stats.get("deep_CALLIND",0)
        result.update({
            "source_SHA_verified":True,
            "functions_traced":size,"functions_decompiled":compiled,
            "store_dependency_witnesses":actual_writes,
            "indirect_call_dependency_witnesses":actual_indirect,
            "known_function_count":data["known_functions"],
            "known_direct_call_edges":data["call_edges"],
            "failure_count":len(data["decompilation_failures"]),
            "report":path.relative_to(dest).as_posix(),
        })
        result["success"]=bool(size>=20 and compiled/size>=.9 and
                               actual_writes>0 and actual_indirect>0)
        if not result["success"]:
            result["error"]="Insufficient typed P-code/decompilation evidence; see full logs"
        if role=="p3" and result["success"]:
            if not private.is_file() or private.stat().st_size!=363649:
                raise RuntimeError("Missing temporary private P3 ROM export")
            if V17.digest(private)!=SHA_P3 or ready.read_text().strip()!=SHA_P3:
                raise RuntimeError("Private P3 image/marker SHA mismatch")
    except Exception as exc:
        result["success"]=False;result["error"]=str(exc)
        with log.open("a") as handle:
            handle.write("\n"+traceback.format_exc()[-3200:])
    return result
def qemu(dest,temp,p3future,args):
    result={"success":False,"stage":"actual_vendor_runtime",
            "physical_MPC_contact":False,"MMC_passthrough":False,
            "new_cases":len(selected_cases()),
            "old_V18_cases_repeated":0,
            "scope":"actual P3 recovery/getvar ARM instructions; PMU/ADC/env mock only"}
    raw=temp/"owner-verified-p3.raw"
    ready=temp/"owner-verified-p3.ready"
    deadline=time.monotonic()+args.export_timeout
    while time.monotonic()<deadline:
        if ready.is_file() and raw.is_file():break
        if p3future.done():
            gh=p3future.result()
            if not gh.get("success"):
                return {**result,"error":"P3 Ghidra failed before source verification"}
        time.sleep(.15)
    else:return {**result,"error":"SHA-verified private P3 export timed out"}
    try:
        if raw.stat().st_size!=363649 or V17.digest(raw)!=SHA_P3:
            raise RuntimeError("P3 owner private image SHA mismatch")
        if ready.read_text().strip()!=SHA_P3:
            raise RuntimeError("P3 private image ready marker mismatch")
        # Shared V16 VM executor launches a fresh isolated ARM process for
        # each case and records SIGSEGV/no-result as FAILURE, then continues.
        V16.C_SOURCE=ROOT/"tools/mpc-one/vendor_probe_v19.c"
        V16.CASES=selected_cases()
        previous=V16.qemu_real_code(dest,raw,temp,{"success":True},args)
        oldjson=dest/"v16-actual-vendor-QEMU-results.json"
        oldlog=dest/"v16-qemu-vendor-function-console.log"
        if oldjson.is_file():
            oldjson.rename(dest/"V19-ACTUAL-P3-ARM-RUNTIME.json")
        if oldlog.is_file():
            oldlog.rename(dest/"v19-actual-vendor-arm-console.log")
        result.update(previous)
        result["new_cases"]=len(selected_cases())
        result["old_V18_cases_repeated"]=0
    except Exception as exc:
        result["error"]=str(exc)
        (dest/"v19-qemu-exception.log").write_text(traceback.format_exc()[-4500:])
    return result
def correlate(dest,old,stages):
    prior=json.loads(old["V18-REPAIRED-DATAFLOW-AND-85-QUESTIONS.json"])
    vm={}
    path=dest/"V19-ACTUAL-P3-ARM-RUNTIME.json"
    if path.is_file():
        vm=json.loads(path.read_text())
    records=vm.get("cases",[])
    by_name={x.get("test"):x for x in records}
    categories=Counter()
    annotated=[]
    for old_row in prior["question_evidence"]:
        id_=old_row["id"];item={
            "id":id_,"question":old_row["question"],
            "v18_evidence":old_row.get("v18_evidence_files",[]),
            "v19_probe_executed":False,
            "v19_new_evidence":[],"v19_status":"UNRESOLVED"
        }
        if id_ in ("RECOVERY-01","RECOVERY_SECURITY-04",
                   "RECOVERY_SECURITY-13","BOOT_SELECTION-01",
                   "REMOVABLE_DIRECTBOOT-02"):
            if stages.get("ghidra_p3",{}).get("success"):
                item["v19_probe_executed"]=True
                item["v19_new_evidence"].append({
                    "file":"deep-p3/V19-p3-PATH-WITNESSES.jsonl",
                    "scope":"typed static source expression and call-chain evidence"})
        if id_ in ("FASTBOOT-05","FASTBOOT_RUNTIME-03","FASTBOOT-09"):
            if stages.get("ghidra_p3",{}).get("success"):
                item["v19_probe_executed"]=True
                item["v19_new_evidence"].append({
                    "file":"deep-p3/V19-p3-PATH-WITNESSES.jsonl",
                    "scope":"environment/storage authorization paths; static only"})
        if id_ in ("REMOVABLE_DIRECTBOOT-03","BOOT_SELECTION-03",
                   "RECOVERY_SECURITY-03"):
            if stages.get("ghidra_p1",{}).get("success"):
                item["v19_probe_executed"]=True
                item["v19_new_evidence"].append({
                    "file":"deep-p1/V19-p1-PATH-WITNESSES.jsonl",
                    "scope":"SPL recursively connected loader functions; static only"})
        if id_ in ("RECOVERY-01","RECOVERY_SECURITY-04",
                   "RECOVERY_SECURITY-13"):
            observed=[by_name.get("v19_recovery_mode_"+str(x)) for x in (0,1,3,9,255)]
            if all(x is not None for x in observed):
                item["v19_probe_executed"]=True
                item["v19_new_evidence"].append({
                    "file":"V19-ACTUAL-P3-ARM-RUNTIME.json",
                    "scope":"actual vendor recovery mode selection with mocked PMU"})
                item["v19_status"]="PARTIAL_ISOLATED_ARM_RUNTIME"
        if id_ in ("FASTBOOT-09","FASTBOOT_RUNTIME-03"):
            observed=[by_name.get("v19_getvar_4_"+str(auth)+"_"+str(mode))
                      for auth in (0,1) for mode in (0,1,2)]
            if all(x is not None for x in observed):
                item["v19_probe_executed"]=True
                item["v19_new_evidence"].append({
                    "file":"V19-ACTUAL-P3-ARM-RUNTIME.json",
                    "scope":"actual vendor getvar environment-backed path with mocked getter"})
                item["v19_status"]="PARTIAL_ISOLATED_ARM_RUNTIME"
        if item["v19_status"]=="UNRESOLVED" and item["v19_probe_executed"]:
            item["v19_status"]="PROBED_UNRESOLVED"
        categories[item["v19_status"]]+=1
        annotated.append(item)
    real_pass=[x for x in records if x.get("pass") is True]
    output={
        "schema":"mpfree-v19-85-question-evidence-audit",
        "prior_v18_verified":True,
        "questions":annotated,"classifications":dict(categories),
        "questions_with_new_probes":sum(x["v19_probe_executed"] for x in annotated),
        "new_runtime_cases_passed":len(real_pass),
        "new_runtime_cases_requested":len(selected_cases()),
        "limits":[
            "Source xrefs and Ghidra P-code are static, not hardware verification.",
            "Generic ARM virt does NOT emulate the RK3288 BootROM, DRAM or fuses.",
            "Only actual vendor functions under synthetic PMU/ADC/environment run.",
            "An unreported guest case is a failure, not a claimed passed test.",
            "Question classification remains partial unless all subclaims are proven."
        ]
    }
    write(dest/"V19-85-QUESTION-EVIDENCE.json",output)
    return output
def package(dest):
    allowed={".json",".jsonl",".log",".txt",".md",".csv",".tsv"}
    members=[];size_total=0
    for file in sorted(dest.rglob("*")):
        if file.is_symlink():
            raise ValueError("Symlink in owner-private research output")
        if file.is_dir():continue
        if not file.is_file() or not stat.S_ISREG(file.lstat().st_mode):
            raise ValueError("Nonregular owner report file: "+str(file))
        if file.suffix not in allowed:
            raise ValueError("Unexpected nontext owner-private artifact")
        n=file.stat().st_size;size_total+=n
        if n>192*1024*1024 or size_total>512*1024*1024:
            raise ValueError("Owner-private report exceeds bounded size")
        members.append({"name":file.relative_to(dest).as_posix(),
                        "bytes":n,"sha256":V17.digest(file)})
    if not members:raise RuntimeError("No V19 evidence files")
    output=dest.with_suffix(".zip")
    with zipfile.ZipFile(output,"x",compression=zipfile.ZIP_DEFLATED,
                         compresslevel=6) as archive:
        archive.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v19","files":members,"private":True},indent=2))
        for item in members:
            archive.write(dest/item["name"],item["name"])
    with zipfile.ZipFile(output) as archive:
        if archive.testzip():raise RuntimeError("Output V19 ZIP CRC failure")
        for item in members:
            data=archive.read(item["name"])
            if len(data)!=item["bytes"] or hashlib.sha256(data).hexdigest()!=item["sha256"]:
                raise RuntimeError("Output member checksum mismatch: "+item["name"])
    output.with_suffix(".zip.sha256").write_text(V17.digest(output)+"  "+output.name+"\n")
    return output,len(members)
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prior-v18",type=Path,
                        help="SHA-indexed owner V18 private ZIP; newest autodetected")
    parser.add_argument("--qemu-boot-timeout",type=int,default=240)
    parser.add_argument("--case-timeout",type=int,default=20)
    parser.add_argument("--export-timeout",type=int,default=1200)
    parser.add_argument("--skip-vm",action="store_true")
    args=parser.parse_args()
    os.umask(0o077)
    WORK.mkdir(parents=True,exist_ok=True)
    if args.prior_v18:
        earlier=regular(args.prior_v18)
    else:
        previous=sorted(WORK.glob("bootchain-wide-*-v18-*.zip"),
                        key=lambda p:p.stat().st_mtime,reverse=True)
        if not previous:parser.error("No owner-private V18 ZIP found in ~/mpfree-work")
        earlier=regular(previous[0])
    old=verified_prior(earlier)
    dest=WORK/("bootchain-wide-"+datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
               +"-v19-"+str(os.getpid()))
    dest.mkdir(mode=0o700)
    began=time.monotonic();stages={}
    print("MPFree V19: NEW typed High P-code + real recovery/getvar ARM runtime",flush=True)
    print("Verified private V18 prior:",earlier,flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="mpfree-v19-owner-p3-",dir=WORK) as td:
            temp=Path(td)
            with ThreadPoolExecutor(max_workers=4) as pool:
                jobs={
                    "ghidra_p1":pool.submit(timed,ghidra,"p1",dest,temp),
                    "ghidra_p3":pool.submit(timed,ghidra,"p3",dest,temp),
                }
                if not args.skip_vm:
                    jobs["actual_arm_vm"]=pool.submit(timed,qemu,dest,temp,jobs["ghidra_p3"],args)
                for future in as_completed(list(jobs.values())):
                    key=next(k for k,v in jobs.items() if v is future)
                    stages[key]=future.result()
                    print("V19",key,"PASS" if stages[key].get("success") else "INCOMPLETE",
                          stages[key].get("wall_seconds"),"seconds",
                          stages[key].get("error",""),flush=True)
    except KeyboardInterrupt:
        stages["interrupted"]={"success":False,"error":"Owner interrupted"}
    except Exception:
        stages["unhandled"]={"success":False,
                            "traceback":traceback.format_exc()[-3500:]}
    finally:
        try:ledger=correlate(dest,old,stages)
        except Exception:
            ledger={}
            (dest/"V19-LEDGER-ERROR.log").write_text(traceback.format_exc()[-4000:])
        write(dest/"V19-STAGE-MANIFEST.json",{
            "schema":"mpfree-v19-directed-recovery-getvar-runtime",
            "branch":BRANCH,"generated_utc":datetime.now(timezone.utc).isoformat(),
            "duration_seconds":round(time.monotonic()-began,2),
            "stages":stages,"prior_v18_sha256":V17.digest(earlier),
            "owner_P3_source_sha256":SHA_P3,
            "physical_MPC_access":False,"host_USB_or_MMC_passthrough":False,
            "Ghidra_source_mode":"readOnly,noanalysis",
            "original_vendor_firmware_bytes_in_archive":False,
            "new_vendor_vm_cases_requested":len(selected_cases()),
            "new_vendor_vm_cases_observed":stages.get("actual_arm_vm",{}).get("tests_observed",0),
            "new_questions_with_probes":ledger.get("questions_with_new_probes",0)})
        info=[
            "# MPFree V19: directed owner-private recovery, boot and getvar research","",
            "## Concurrent stages and actual measured elapsed times"]
        for name,result in stages.items():
            info.append("- %s: %s, %.2fs; %s"%(name,
                "PASS" if result.get("success") else "INCOMPLETE",
                result.get("wall_seconds",0.0),result.get("error","")))
        info+=["","## New research, not repeats",
          "- Genuine owner U-Boot recovery selector is executed in generic ARM QEMU",
          "  with mocked reboot-mode register and ADC threshold data.",
          "- V18's two environment-backed GETVAR SIGSEGV paths are retested",
          "  with instrumented fake environment lookup returning null, empty",
          "  or known synthetic strings, across authorized/unauthorized modes.",
          "- Typed Ghidra High P-code witnesses preserve LOAD/STORE dependencies,",
          "  unknown runtime values and function callbacks with source addresses.",
          "- Fixed-point caller/callee reachability retained, with relevant",
          "  per-function expression tracing (bounded against PHI explosions).",
          "",
          "## Proof limits",
          "- No real MPC I/O or true RK3288 BootROM is emulated.",
          "- Fake PMU/ADC/environment responses do not prove actual hardware behavior.",
          "- Existing VM cases are NOT rerun or counted as new work.",
          "- Each reported question remains open unless its exact subclaims are evidenced.",
          "- All proprietary guest source bytes remain in a deleted private temp directory.",
          ""]
        (dest/"V19-READ-ME-FIRST.md").write_text("\n".join(info),encoding="utf8")
        output,count=package(dest)
        print("\nONE PRIVATE ZIP TO UPLOAD:",output,flush=True)
        print("SHA256:",V17.digest(output),"reports",count,flush=True)
        print("No physical MPC, USB, eMMC or OTP was contacted.",flush=True)
if __name__=="__main__":main()
