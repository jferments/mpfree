#!/usr/bin/env python3
"""MPFree V22: full directed Ghidra + actual vendor block-write ARM experiments.

Actual owner 3.10.0 P3 source image SHA is pinned. The guest executes the
unchanged ARM-mode divide routine, original U-Boot storage wrapper, and
concrete registered FUN_0011de50 block-write handler in disposable QEMU.
All low-level device transport and write callbacks are intercepted BEFORE
hardware-side operations. No physical device or persistent firmware writes.

The new Ghidra investigation follows registration 0x0011d8e8 to literal
0x0011d95c -> FUN_0011de50, then indirect 0x0011ca78 transport dispatch,
as well as erase callback and removable boot selection. One private ZIP.
"""
from __future__ import annotations
from collections import Counter
from concurrent.futures import ThreadPoolExecutor,as_completed
from datetime import datetime,timezone
import argparse,hashlib,importlib.util,json,os,stat,subprocess,tempfile,time,traceback,zipfile
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-v22-real-storage-write-20261112"
def load(path,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/path)
    module=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
V21=load("tools/mpc-one/run_v21.py","mpfree_v22_prior")
V17=V21.V17
V16=V21.V16
PINNED=V21.SHA
def regular(path):return V17.regular_file(path)
def digest(path):return V17.digest(path)
def write(path,data):
    path.parent.mkdir(exist_ok=True,parents=True)
    path.write_text(json.dumps(data,indent=2,sort_keys=True)+"\n",encoding="utf8")
def cases():
    names=["v22_div_%d"%i for i in range(12)]
    names+=["v22_wrapper_%d"%i for i in range(8)]
    names+=["v22_write_%d"%i for i in range(9)]
    assert len(names)==29 and len(names)==len(set(names))
    assert not set(names)&set(V21.cases())
    return names
def verified_prior(path):
    # V21 private ZIP is already SHA-indexed, 11 text report files.
    # Reuse the established verified archive reader, not an arbitrary ZIP.
    archive=V17.load_zip(regular(path))
    required={
       "V21-STAGE-MANIFEST.json",
       "V21-REGISTRY-DISCOVERY-AND-85-QUESTIONS.json",
       "deep-p3/V21-P3-BACKEND-REGISTRY-SUMMARY.json",
       "V21-ACTUAL-VENDOR-BACKEND-ARM.json",
       "V21-RUNTIME-FAULT-AND-CALLBACK-TRACES.json",
    }
    if required-set(archive):
        raise ValueError("V21 SHA-indexed archive missing required reports: "+
                         str(sorted(required-set(archive))))
    meta=json.loads(archive["V21-STAGE-MANIFEST.json"])
    p3=json.loads(archive["deep-p3/V21-P3-BACKEND-REGISTRY-SUMMARY.json"])
    if (meta["owner_P3_sha256"]!=PINNED or
        p3.get("owner_input_sha256")!=PINNED or
        not p3.get("owner_source_verified")):
        raise ValueError("Owner firmware SHA lineage differs from pinned P3")
    ledger=json.loads(archive["V21-REGISTRY-DISCOVERY-AND-85-QUESTIONS.json"])
    if len(ledger.get("questions",[]))!=85:
        raise ValueError("V21 research ledger does not contain 85 questions")
    return archive
def timed(fn,*args):
    began=time.monotonic()
    try: result=fn(*args)
    except Exception as e:
        result={"success":False,"error":str(e),
                "traceback":traceback.format_exc()[-4000:]}
    result["wall_seconds"]=round(time.monotonic()-began,2)
    return result
def ghidra(out,temp):
    result={"success":False,"source_verified":False,
            "actual_vendor_code_executed":False}
    folder=out/"deep-p3";folder.mkdir(parents=True,exist_ok=True)
    log=out/"v22-ghidra-p3-console.log"
    try:
        launcher=V16.get_ghidra()
        if not launcher:raise RuntimeError("No owner saved Ghidra installation")
        project=V16.gproject("p3")
        raw=temp/"verified-P3-owner.raw"
        ready=temp/"verified-P3-owner.ready"
        command=launcher+[
          str(WORK/"ghidra/projects"),project,
          "-process","p3-uimage-payload.bin",
          "-readOnly","-noanalysis","-scriptPath",str(ROOT/"tools/ghidra"),
          "-postScript","MPFreeV22BlockTransport.py","p3",
          str(folder),str(raw),str(ready),
          "-log",str(folder/"ghidra-engine.log")]
        with log.open("w") as f:
            process=subprocess.run(command,stdout=f,stderr=subprocess.STDOUT,
                                   timeout=3600,check=False)
        if process.returncode:
            raise RuntimeError("Ghidra P3 stage exited "+str(process.returncode))
        summary=json.loads((folder/"V22-p3-SUMMARY.json").read_text())
        if (summary.get("schema")!="mpfree-v22-directed-interprocedural-pcode-v1" or
            summary.get("owner_sha256")!=PINNED or
            not summary.get("source_hash_verified")):
            raise RuntimeError("New P3 Ghidra report failed source/schema provenance")
        proof=summary.get("literal_evidence",{}).get("registered_backend_callbacks_verified",{})
        if (proof.get("write_pointer")!="0x0011de51" or
            proof.get("erase_pointer")!="0x0011dd4d"):
            raise RuntimeError("Registered owner write/erase pointer source not verified")
        rfile=folder/"V22-p3-PATH-WITNESSES.jsonl"
        if not rfile.is_file() or rfile.stat().st_size==0:
            raise RuntimeError("New Ghidra transport code witnesses missing")
        actual=summary["stats"]
        total=summary["traced_function_count"]
        decoded=actual.get("functions_decompiled",0)
        result.update({
            "source_verified":True,"recognized":summary["known_functions"],
            "targeted_functions":total,"decompiled":decoded,
            "actual_store_dependencies":actual.get("deep_STORE",0),
            "actual_indirect_call_dependencies":actual.get("deep_CALLIND",0),
            "reported_failures":len(summary["decompilation_failures"]),
            "confirmed_write_callback":"0x0011de50",
            "confirmed_erase_callback":"0x0011dd4c"})
        result["success"]=bool(total>=40 and decoded/max(1,total)>.95 and
                               result["actual_store_dependencies"]>=100 and
                               result["actual_indirect_call_dependencies"]>=10)
        if not result["success"]:
            result["error"]="Directed Ghidra path coverage or High P-code evidence inadequate"
    except Exception as exc:
        result["error"]=str(exc)
        with log.open("a") as f:f.write("\n"+traceback.format_exc()[-4000:])
    return result
def guest(out,temp,ghidra_future,args):
    result={"success":False,"board":"generic ARM virt; not RK3288",
            "new_cases":len(cases()),"hardware_contact":False,
            "low_level_MMC_transport":"all replaced with guest-only stubs"}
    raw=temp/"verified-P3-owner.raw";marker=temp/"verified-P3-owner.ready"
    deadline=time.monotonic()+args.export_timeout
    while time.monotonic()<deadline:
        if raw.is_file() and marker.is_file():break
        if ghidra_future.done() and not ghidra_future.result().get("success"):
            return {**result,"error":"P3 verification stage failed before export"}
        time.sleep(.2)
    else:return {**result,"error":"Timed out waiting for verified Ghidra P3 image"}
    try:
        if (raw.stat().st_size!=363649 or digest(raw)!=PINNED or
            marker.read_text().strip()!=PINNED):
            raise RuntimeError("Owner private P3 image SHA not verified")
        V16.C_SOURCE=ROOT/"tools/mpc-one/vendor_probe_v22.c"
        V16.CASES=cases()
        observed=V16.qemu_real_code(out,raw,temp,{"success":True},args)
        report=out/"v16-actual-vendor-QEMU-results.json"
        log=out/"v16-qemu-vendor-function-console.log"
        if report.is_file():report.rename(out/"V22-REAL-OWNER-ARM-RUNTIME.json")
        if log.is_file():log.rename(out/"v22-arm-vendor-console.log")
        result.update(observed)
        result["new_cases"]=len(cases())
        result["old_v21_cases_repeated"]=0
    except Exception as exc:
        result["error"]=str(exc)
        (out/"V22-ARM-ERROR.log").write_text(traceback.format_exc()[-5000:])
    return result
def analyze(out,prior,stages):
    report=out/"V22-REAL-OWNER-ARM-RUNTIME.json"
    dynamic=json.loads(report.read_text()) if report.is_file() else {}
    p=out/"deep-p3/V22-p3-SUMMARY.json"
    static=json.loads(p.read_text()) if p.is_file() else {}
    observations={r.get("test"):r for r in dynamic.get("cases",[])}
    section_counts={}
    for name,prefix,total in [
        ("real_arm_unsigned_division","v22_div_",12),
        ("real_storage_wrapper_with_unpatched_arm_division","v22_wrapper_",8),
        ("real_registered_block_write_with_mocked_IO","v22_write_",9)
    ]:
        rows=[observations.get(prefix+str(i)) for i in range(total)]
        section_counts[name]={
            "planned":total,"observed":sum(bool(x and "observation" in x) for x in rows),
            "passed":sum(bool(x and x.get("pass")) for x in rows),
            "all_passed":all(x and x.get("pass") for x in rows),
            "failures":[prefix+str(i) for i,x in enumerate(rows)
                        if not x or not x.get("pass")]
        }
    v21=json.loads(prior["V21-REGISTRY-DISCOVERY-AND-85-QUESTIONS.json"])
    rows=[]
    static_scope={
      "FASTBOOT_RUNTIME-02":"Reachable block-write code and authorization/storage relationship; no persistent side-effect proof.",
      "REMOVABLE_DIRECTBOOT-01":"Boot-source control flow and concrete storage-write backend, not a working SD boot.",
      "REMOVABLE_DIRECTBOOT-03":"P3 MMC-related function paths only; SPL boot-source priority not retested.",
      "REMOVABLE_DIRECTBOOT-05":"P3 device-driver table and storage IDs, NOT physical MMC numbering.",
      "REMOVABLE_DIRECTBOOT-08":"P3 command dispatcher/boot source roots, NOT proof of a safe transient command.",
      "BOOT_SELECTION-04":"Owner bootz/bootm and load-source control-flow candidates.",
      "RECOVERY_SECURITY-10":"Concrete block-write handler code, NOT proof of updater P6 LBA boundaries.",
    }
    dynamic_scope={
      "FASTBOOT_RUNTIME-02":"Genuine registered write handler under fake transport, showing a potential write mechanism, not a live eMMC side effect.",
      "REMOVABLE_DIRECTBOOT-01":"Original U-Boot write-wrapper executes with nonwriting mock media; does not establish external boot.",
      "REMOVABLE_DIRECTBOOT-08":"Nonwriting wrapper branch supports transient-command safety investigation, but does not execute a real command.",
      "RECOVERY_SECURITY-10":"Actual block-count/bounds behavior under fake media, not official P6 partition-write proof.",
    }
    runtime_has_support=(
      section_counts["real_registered_block_write_with_mocked_IO"]["passed"]>0 or
      section_counts["real_storage_wrapper_with_unpatched_arm_division"]["passed"]>0)
    for old in v21["questions"]:
        row={"id":old["id"],"question":old["question"],
             "v21_research_status":old.get("answered_in_v21"),
             "v22_new_evidence":[],"answered_by_v22":False}
        if old["id"] in static_scope and stages.get("new_p3_ghidra",{}).get("success"):
            row["v22_new_evidence"].append({
                "file":"deep-p3/V22-p3-SUMMARY.json",
                "scope":static_scope[old["id"]],"kind":"NEW_STATIC_SOURCE_ANALYSIS"})
        if old["id"] in dynamic_scope and runtime_has_support:
            row["v22_new_evidence"].append({
                "file":"V22-REAL-OWNER-ARM-RUNTIME.json",
                "scope":dynamic_scope[old["id"]],
                "kind":"ISOLATED_ACTUAL_OWNER_ARM_NO_PHYSICAL_IO"})
        rows.append(row)
    findings={
       "schema":"mpfree-v22-actual-block-transport-results",
       "p3_SHA":PINNED,
       "runtime_sections":section_counts,
       "total_new_vendor_cases":len(cases()),
       "total_new_vendor_cases_passed":dynamic.get("passed",0),
       "registered_backend_callback_source":{
          "registration_function":"FUN_0011D8E8",
          "registered_vendor_block_write_function":"FUN_0011DE50",
          "erase_function":"FUN_0011DD4C",
          "transport_dispatch_function":"FUN_0011CA78",
          "device_driver_through_function_table":True,
          "runtime_selected_physical_device":"UNVERIFIED",
          "eMMC_write_ever_executed":False,
          "Ghidra_pointer_literals_verified":bool(
             static.get("literal_evidence",{}).get(
                "registered_backend_callbacks_verified"))
       },
       "questions":rows,
       "questions_with_new_evidence":sum(bool(x["v22_new_evidence"]) for x in rows),
       "unresolved":[
         "Concrete live MMC host driver function pointer and device numbering",
         "Proven read-only removable boot selection and tested Linux handoff",
         "Physical RK3288 MaskROM recovery and USB transport independence",
         "Exact runtime memory reservations and safe boot image sizing",
         "Real hardware eMMC/SD behavior (not tested by generic ARM virt)"
       ]}
    write(out/"V22-CONCRETE-FINDINGS-AND-85-QUESTIONS.json",findings)
    return findings
def package(out):
    allowed={".json",".jsonl",".log",".md",".txt",".csv",".tsv"}
    files=[];total=0
    for p in sorted(out.rglob("*")):
        if p.is_symlink():raise ValueError("Symlink in private research output")
        if p.is_dir():continue
        if (not p.is_file() or not stat.S_ISREG(p.lstat().st_mode) or
            p.suffix not in allowed):
            raise ValueError("Nontext or nonregular output "+str(p))
        n=p.stat().st_size;total+=n
        if n>192*1024*1024 or total>512*1024*1024:
            raise ValueError("Research report output exceeds cap")
        files.append({"name":p.relative_to(out).as_posix(),
                      "bytes":n,"sha256":digest(p)})
    if not files:raise ValueError("No new research evidence collected")
    destination=out.with_suffix(".zip")
    with zipfile.ZipFile(destination,"x",compression=zipfile.ZIP_DEFLATED,
                         compresslevel=6) as z:
        z.writestr("ZIP_INDEX.json",json.dumps({
           "schema":"mpfree-private-v22","private":True,"files":files},indent=2))
        for item in files:z.write(out/item["name"],item["name"])
    with zipfile.ZipFile(destination) as z:
        if z.testzip():raise ValueError("V22 ZIP CRC integrity failed")
        for item in files:
            b=z.read(item["name"])
            if len(b)!=item["bytes"] or hashlib.sha256(b).hexdigest()!=item["sha256"]:
                raise ValueError("V22 member checksum failed: "+item["name"])
    destination.with_suffix(".zip.sha256").write_text(
        digest(destination)+"  "+destination.name+"\n")
    return destination,len(files)
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prior-v21",type=Path)
    parser.add_argument("--qemu-boot-timeout",type=int,default=240)
    parser.add_argument("--case-timeout",type=int,default=24)
    parser.add_argument("--export-timeout",type=int,default=1200)
    parser.add_argument("--skip-vm",action="store_true")
    args=parser.parse_args()
    os.umask(0o077);WORK.mkdir(parents=True,exist_ok=True)
    if args.prior_v21:source=regular(args.prior_v21)
    else:
        files=sorted(WORK.glob("bootchain-wide-*-v21-*.zip"),
                     key=lambda p:p.stat().st_mtime,reverse=True)
        if not files:parser.error("Owner-private V21 archive missing under ~/mpfree-work")
        source=regular(files[0])
    old=verified_prior(source)
    output=WORK/("bootchain-wide-"+datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
                 +"-v22-"+str(os.getpid()))
    output.mkdir(mode=0o700);begin=time.monotonic();stages={}
    print("MPFree V22: actual registered block WRITE callback, real ARM division and low-level driver trace",flush=True)
    print("Verified private V21 input:",source,flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="mpfree-v22-private-",dir=WORK) as d:
            temp=Path(d)
            with ThreadPoolExecutor(max_workers=2) as pool:
                jobs={"new_p3_ghidra":pool.submit(timed,ghidra,output,temp)}
                if not args.skip_vm:
                    jobs["actual_arm_guest"]=pool.submit(
                        timed,guest,output,temp,jobs["new_p3_ghidra"],args)
                for f in as_completed(jobs.values()):
                    k=next(k for k,x in jobs.items() if x is f)
                    stages[k]=f.result()
                    print("V22",k,"PASS" if stages[k].get("success") else "INCOMPLETE",
                          stages[k].get("wall_seconds"),"seconds",
                          stages[k].get("error",""),flush=True)
    except KeyboardInterrupt:
        stages["interrupted"]={"success":False,"error":"Owner interrupted"}
    except Exception:
        stages["uncaught"]={"success":False,
                            "traceback":traceback.format_exc()[-4500:]}
    finally:
        try: findings=analyze(output,old,stages)
        except Exception:
            findings={}
            (output/"V22-ANALYSIS-ERROR.log").write_text(traceback.format_exc()[-4500:])
        write(output/"V22-STAGE-MANIFEST.json",{
            "schema":"mpfree-v22-actual-block-driver-runtime",
            "branch":BRANCH,"time_utc":datetime.now(timezone.utc).isoformat(),
            "duration_seconds":round(time.monotonic()-begin,2),
            "prior_v21_sha256":digest(source),"source_P3_SHA":PINNED,
            "stages":stages,"new_arm_cases_requested":len(cases()),
            "new_arm_cases_observed":stages.get("actual_arm_guest",{}).get("tests_observed",0),
            "physical_MPC_access":False,"host_MMC_USB_passthrough":False,
            "persistent_media_write_performed":False,"proprietary_firmware_bin_in_zip":False,
            "questions_with_new_evidence":findings.get("questions_with_new_evidence",0)})
        summary=["# MPFree V22: original U-Boot block-write and low-level transport","",
           "The previous V21 SIGILL at 0x0012ee08 was caused by patching an",
           "original ARM-mode unsigned division function with Thumb instructions.",
           "V22 does NOT patch the division routine.","",
           "## Measured concurrent stages"]
        for name,r in stages.items():
            summary.append("- %s: %s, %.2fs; %s"%(name,
                "PASS" if r.get("success") else "INCOMPLETE",
                r.get("wall_seconds",0),r.get("error","")))
        summary+=["","## New evidence and proof limitations",
             "- Original write handler FUN_0011DE50 is taken from owner-verified",
             "  block-device registration literal at 0x0011D95C.",
             "- Real wrapper calls real ARM division, then nonwriting C callback.",
             "- Real registered write function executes chunking/boundary logic,",
             "  but ALL low-level MMC/USB data transport remains guest-stubbed.",
             "- New Ghidra follows driver-function-table indirection and",
             "  registration values with per-instruction typed dependencies.",
             "- The 85-question lineage is preserved without counting",
             "  a candidate pointer as a proven physical SD/eMMC operation.",
             "- No firmware image or raw owner binary is packaged.",""]
        (output/"V22-READ-ME-FIRST.md").write_text("\n".join(summary))
        archive,n=package(output)
        print("\nONE PRIVATE ZIP:",archive,flush=True)
        print("SHA256:",digest(archive),"reports:",n,flush=True)
if __name__=="__main__":
    main()
