#!/usr/bin/env python3
"""MPFree V21: backend linked-list registration and callback discovery.

Concurrent SHA-pinned owner-P3 Ghidra full value-flow scan and isolated
ARM-virt execution of REAL U-Boot backend registry and storage wrapper.
Further exercises the two V20 SIGILL paths with guest PC/LR/R5/R8 reports.
No real MMC, USB, MPC, eMMC, Fastboot, firmware installation or writes.

ONE SHA-indexed private ZIP. Prior V20 archival lineage verified fully.
"""
from __future__ import annotations
from collections import Counter
from concurrent.futures import ThreadPoolExecutor,as_completed
from datetime import datetime,timezone
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile
import time
import traceback
import zipfile

ROOT=Path(__file__).resolve().parents[2]
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-v21-backend-registry-20261112"
def load(filename,key):
    spec=importlib.util.spec_from_file_location(key,ROOT/filename)
    obj=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj
V20=load("tools/mpc-one/run_v20.py","mpfree_v21_v20")
V19=V20.V19
V17=V20.V17
V16=V20.V16
SHA=V20.PINNED
def write(path,body):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(body,indent=2,sort_keys=True)+"\n",encoding="utf8")
def digest(path):return V17.digest(path)
def timed(fn,*args):
    start=time.monotonic()
    try: r=fn(*args)
    except Exception as exc:
        r={"success":False,"error":str(exc),
           "traceback":traceback.format_exc()[-5000:]}
    r["actual_wall_seconds"]=round(time.monotonic()-start,2)
    return r
def cases():
    cases=["v21_registry_%u"%i for i in range(5)]
    cases.extend("v21_transfer_%u"%i for i in range(6))
    assert len(cases)==11 and len(cases)==len(set(cases))
    assert not (set(cases)&set(V20.cases()))
    assert not (set(cases)&set(V19.selected_cases()))
    return cases
def verified_prior(path):
    # Owner research V20 has nine SHA-indexed files, each within the
    # established private ZIP member size cap. V17's reader verifies all.
    files=V17.load_zip(V17.regular_file(path))
    required={
        "V20-STAGE-MANIFEST.json",
        "V20-EXACT-FINDINGS-AND-85-QUESTIONS.json",
        "V20-ACTUAL-VENDOR-STORAGE-COMMAND-ARM.json",
        "deep-p3/V20-P3-BACKEND-COMMAND-SUMMARY.json",
    }
    missing=required-set(files)
    if missing:raise ValueError("Missing V20 source evidence: "+str(sorted(missing)))
    manifest=json.loads(files["V20-STAGE-MANIFEST.json"])
    if manifest.get("owner_p3_sha256")!=SHA:
        raise ValueError("Wrong owner firmware SHA in V20 manifest")
    p3=json.loads(files["deep-p3/V20-P3-BACKEND-COMMAND-SUMMARY.json"])
    if not p3.get("owner_input_verified") or p3.get("owner_sha256")!=SHA:
        raise ValueError("Wrong owner firmware Ghidra P3 evidence")
    ledger=json.loads(files["V20-EXACT-FINDINGS-AND-85-QUESTIONS.json"])
    if len(ledger.get("questions",[]))!=85:
        raise ValueError("V20 did not preserve 85 original research questions")
    vm=json.loads(files["V20-ACTUAL-VENDOR-STORAGE-COMMAND-ARM.json"])
    if vm.get("tests_planned")!=22 or vm.get("passed")!=20:
        raise ValueError("Unexpected V20 evidence baseline; investigate before proceeding")
    return files
def ghidra(out,temp):
    r={"success":False,"mode":"readOnly,noanalysis",
       "actual_vendor_executed":False,"hardware_contact":False}
    report=out/"deep-p3";report.mkdir(parents=True,exist_ok=True)
    log=out/"v21-owner-p3-ghidra-console.log"
    try:
        launcher=V16.get_ghidra()
        if not launcher:raise RuntimeError("Local owner Ghidra headless launcher missing")
        name=V16.gproject("p3")
        raw=temp/"owner-p3-verified.bin"
        marker=temp/"owner-p3-verified.ready"
        command=launcher+[
            str(WORK/"ghidra/projects"),name,"-process","p3-uimage-payload.bin",
            "-readOnly","-noanalysis",
            "-scriptPath",str(ROOT/"tools/ghidra"),
            "-postScript","MPFreeV21BackendRegistry.py",
            str(report),str(raw),str(marker),
            "-log",str(report/"ghidra-engine.log")]
        with log.open("w") as fd:
            process=subprocess.run(command,stdout=fd,stderr=subprocess.STDOUT,
                                   check=False,timeout=3600)
        if process.returncode:
            raise RuntimeError("Ghidra P3 exited "+str(process.returncode))
        metadata=json.loads((report/"V21-P3-BACKEND-REGISTRY-SUMMARY.json").read_text())
        if (metadata.get("schema")!="mpfree-v21-verified-backend-registry-and-boot"
            or not metadata.get("owner_source_verified")
            or metadata.get("owner_input_sha256")!=SHA):
            raise ValueError("Saved owner firmware report SHA/schema mismatch")
        if raw.stat().st_size!=363649 or digest(raw)!=SHA:
            raise ValueError("Private P3 export was not SHA verified")
        if marker.read_text().strip()!=SHA:
            raise ValueError("Private Ghidra output marker mismatch")
        stats=metadata["statistics"]
        seen=metadata["recognized_functions"]
        done=metadata["decompiled_functions"]
        for artifact in ("V21-P3-BACKEND-VALUE-WRITES.jsonl",
                         "V21-P3-BACKEND-AND-BOOT-FUNCTIONS.jsonl"):
            p=report/artifact
            if not p.is_file() or p.stat().st_size==0:
                raise ValueError("Ghidra emitted no new "+artifact)
        r.update({
            "owner_image_verified":True,
            "recognized":seen,"decompiled":done,
            "unresolved_decompilation_failures":len(metadata["decompile_failures"]),
            "all_store_sites_examined":stats.get("all_store_sites",0),
            "new_relevant_store_value_witnesses":stats.get("relevant_value_stores",0),
            "function_pointer_value_store_candidates":stats.get("function_pointer_value_stores",0),
            "indirect_sites_examined":stats.get("all_indirect_sites",0),
            "boot_strings_located":len(metadata["boot_command_string_pointer_inventory"])
        })
        r["success"]=bool(done/max(1,seen)>=0.96
                          and r["all_store_sites_examined"]>2000
                          and r["new_relevant_store_value_witnesses"]>75
                          and r["indirect_sites_examined"]>150)
        if not r["success"]:
            r["error"]="Insufficient real Ghidra value-flow evidence; see counts and logs"
    except Exception as exc:
        r["error"]=str(exc)
        with log.open("a") as fd:fd.write("\n"+traceback.format_exc()[-4000:])
    return r
def guest(out,temp,ghidra_future,args):
    r={"success":False,"scope":"actual vendor backend registration + callback paths",
       "board":"generic ARM virt, not physical RK3288",
       "physical_USB_or_eMMC_contact":False,
       "new_cases_requested":len(cases())}
    raw=temp/"owner-p3-verified.bin"
    marker=temp/"owner-p3-verified.ready"
    deadline=time.monotonic()+args.export_timeout
    while time.monotonic()<deadline:
        if raw.is_file() and marker.is_file():break
        if ghidra_future.done() and not ghidra_future.result().get("success"):
            return {**r,"error":"Ghidra failed before exporting private verified owner bytes"}
        time.sleep(.15)
    else:return {**r,"error":"Timed out awaiting SHA-verified owner P3 export"}
    try:
        if (raw.stat().st_size!=363649 or digest(raw)!=SHA
            or marker.read_text().strip()!=SHA):
            raise ValueError("Owner private P3 source hash mismatch")
        V16.C_SOURCE=ROOT/"tools/mpc-one/vendor_probe_v21.c"
        V16.CASES=cases()
        run=V16.qemu_real_code(out,raw,temp,{"success":True},args)
        report=out/"v16-actual-vendor-QEMU-results.json"
        log=out/"v16-qemu-vendor-function-console.log"
        if report.exists():report.rename(out/"V21-ACTUAL-VENDOR-BACKEND-ARM.json")
        if log.exists():log.rename(out/"v21-owner-ARM-guest-console.log")
        r.update(run)
        r["new_cases_requested"]=len(cases())
        r["old_V20_cases_repeated"]=0
    except Exception as exc:
        r["error"]=str(exc)
        (out/"v21-guest-error.log").write_text(traceback.format_exc()[-4000:])
    return r
def parse_faults(out):
    log=out/"v21-owner-ARM-guest-console.log"
    faults=[];checkpoints=[];traces=[]
    if log.is_file():
        with log.open(errors="replace") as fd:
            for raw in fd:
                row=raw.strip()
                if not row.startswith("{"):continue
                try:obj=json.loads(row)
                except ValueError:continue
                if obj.get("v21_guest_fault")=="signal":
                    faults.append(obj)
                if "v21_checkpoint" in obj:
                    checkpoints.append(obj)
                if obj.get("trace","").startswith("v21-"):
                    traces.append(obj)
    result={
        "schema":"mpfree-v21-guest-control-flow-and-fault-witnesses",
        "traps":faults,"checkpoints":checkpoints,
        "runtime_traces":traces,
        "traps_by_location":[
           {**x,"pc_region":
            ("owner_P3" if 0x00100000<=int(x.get("pc","0"),16)<0x00159000 else
             "guest_counter_trampoline" if 0x00900000<=int(x.get("pc","0"),16)<0x00901000 else
             "other_or_unknown")}
           for x in faults
        ],
        "limitations":[
            "Guest signals identify emulation-side fault addresses only.",
            "A trap in proprietary owner instructions is not proof of a physical-board crash.",
            "A guest callback success confirms only fake-media control flow."
        ]
    }
    write(out/"V21-RUNTIME-FAULT-AND-CALLBACK-TRACES.json",result)
    return result
def analyze(out,old,stages):
    prior=json.loads(old["V20-EXACT-FINDINGS-AND-85-QUESTIONS.json"])
    path=out/"deep-p3/V21-P3-BACKEND-REGISTRY-SUMMARY.json"
    info=json.loads(path.read_text()) if path.exists() else {}
    vm_path=out/"V21-ACTUAL-VENDOR-BACKEND-ARM.json"
    vm=json.loads(vm_path.read_text()) if vm_path.exists() else {}
    seen={i.get("test"):i for i in vm.get("cases",[])}
    faults=parse_faults(out)
    # Ranking by actual newly recorded SOURCE- and DESTINATION-side P-code.
    writes=out/"deep-p3/V21-P3-BACKEND-VALUE-WRITES.jsonl"
    ranked=[]
    reported=set()
    if writes.is_file():
        with writes.open() as f:
            for line in f:
                row=json.loads(line)
                offsets=set(row.get("offset_constants",[]))
                flags=set(row.get("known_function_pointer_literals",[]))
                score=(10 if "0x00000130" in offsets else 0)+(
                    6 if "0x000000cc" in offsets else 0)+(
                    5 if "0x000000c8" in offsets else 0)+(
                    3 if "0x00000068" in offsets else 0)+(
                    2 if "0x0000006c" in offsets else 0)+(
                    7 if flags else 0)
                if score<3:continue
                key=(row.get("fn"),row.get("at"))
                if key in reported:continue
                reported.add(key)
                ranked.append({"score":score,"function":row.get("fn"),
                    "instruction":row.get("at"),
                    "interesting_constants":sorted(offsets),
                    "candidate_function_pointer_values":sorted(flags),
                    "evidence_file":"deep-p3/V21-P3-BACKEND-VALUE-WRITES.jsonl",
                    "rank_is_hypothesis_not_registration_proof":True})
    ranked.sort(key=lambda x:(-x["score"],x["function"],x["instruction"]))
    # No question is closed on the basis of this ranking.
    result_rows=[]
    for oldrow in prior["questions"]:
        key=oldrow["id"]
        row={
            "id":key,"question":oldrow["question"],
            "old_evidence":oldrow.get("new_probes",[]),
            "new_verified_evidence":[],"answered_in_v21":False
        }
        relevant=key in {
            "FASTBOOT_RUNTIME-02","REMOVABLE_DIRECTBOOT-01",
            "REMOVABLE_DIRECTBOOT-03","REMOVABLE_DIRECTBOOT-05",
            "REMOVABLE_DIRECTBOOT-06","BOOT_SELECTION-04",
            "BOOT_SELECTION-06","RECOVERY_SECURITY-12",
        }
        if relevant and stages.get("ghidra_p3",{}).get("success"):
            row["new_verified_evidence"].append({
                "file":"deep-p3/V21-P3-BACKEND-VALUE-WRITES.jsonl",
                "scope":"new register/indirect callback candidate dependencies, static only"})
        if relevant and vm.get("tests_observed",0)>0:
            row["new_verified_evidence"].append({
                "file":"V21-ACTUAL-VENDOR-BACKEND-ARM.json",
                "scope":"actual owner backend lookup/projection and storage-path diagnostics; fake RAM only"})
        result_rows.append(row)
    findings={
        "schema":"mpfree-v21-owner-evidence-no-inflated-answers",
        "new_arm_cases_planned":len(cases()),
        "new_arm_cases_observed":vm.get("tests_observed",0),
        "new_arm_cases_passed":vm.get("passed",0),
        "new_static_store_value_witnesses":info.get("statistics",{}).get("relevant_value_stores",0),
        "new_static_function_pointer_registration_candidates":
            info.get("statistics",{}).get("function_pointer_value_stores",0),
        "backend_linked_list_head_literal":info.get("storage_list_registry",{}),
        "ranked_register_writers":ranked[:250],
        "new_runtime_faults":faults["traps"],
        "runtime_fault_count":len(faults["traps"]),
        "storage_success_path_experiment_status":
            ("PASSED_GUEST_MOCKED_CALLBACK" if
              all(seen.get("v21_transfer_%u"%i,{}).get("pass") for i in (0,1,3,4,5)) else
             "NOT_FULLY_VERIFIED"),
        "backend_registration_runtime_status":
            ("PASSED_GUEST_LINKED_LIST" if
             all(seen.get("v21_registry_%u"%i,{}).get("pass") for i in range(5)) else
             "NOT_FULLY_VERIFIED"),
        "questions":result_rows,
        "questions_with_new_evidence":sum(bool(r["new_verified_evidence"]) for r in result_rows),
        "questions_closed_by_name_matching":0,
        "still_unknown":[
            "Actual registered device backend object and concrete method identity",
            "Physical eMMC write destination and storage side effects",
            "A confirmed non-persistent SD U-Boot command sequence",
            "Physical RK3288 ROM and independent eMMC recovery",
            "Real DRAM relocation, FDT and Linux handoff safety"],
    }
    write(out/"V21-REGISTRY-DISCOVERY-AND-85-QUESTIONS.json",findings)
    return findings
def package(out):
    types={".json",".jsonl",".log",".txt",".md",".csv",".tsv"}
    members=[];total=0
    for path in sorted(out.rglob("*")):
        if path.is_symlink():raise ValueError("Symlink under owner-private outputs")
        if path.is_dir():continue
        if (not path.is_file() or not stat.S_ISREG(path.lstat().st_mode)
            or path.suffix not in types):
            raise ValueError("Unexpected owner-private output file: "+str(path))
        n=path.stat().st_size
        total+=n
        if n>192*1024*1024 or total>512*1024*1024:
            raise ValueError("Owner research output exceeds private ZIP safety limits")
        members.append({"name":path.relative_to(out).as_posix(),
                        "bytes":n,"sha256":digest(path)})
    output=out.with_suffix(".zip")
    with zipfile.ZipFile(output,"x",compression=zipfile.ZIP_DEFLATED,
                         compresslevel=6) as archive:
        archive.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v21","private":True,
            "files":members},indent=2))
        for row in members:
            archive.write(out/row["name"],row["name"])
    with zipfile.ZipFile(output) as archive:
        if archive.testzip():raise RuntimeError("V21 output ZIP CRC failed")
        for row in members:
            buf=archive.read(row["name"])
            if len(buf)!=row["bytes"] or hashlib.sha256(buf).hexdigest()!=row["sha256"]:
                raise RuntimeError("V21 output member SHA mismatch: "+row["name"])
    output.with_suffix(".zip.sha256").write_text(digest(output)+"  "+output.name+"\n")
    return output,len(members)
def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prior-v20",type=Path,
                    help="Verified owner-private V20 ZIP; newest autodetected")
    ap.add_argument("--qemu-boot-timeout",type=int,default=240)
    ap.add_argument("--case-timeout",type=int,default=20)
    ap.add_argument("--export-timeout",type=int,default=1200)
    ap.add_argument("--skip-vm",action="store_true")
    args=ap.parse_args()
    os.umask(0o077)
    WORK.mkdir(parents=True,exist_ok=True)
    if args.prior_v20:
        source=V17.regular_file(args.prior_v20)
    else:
        priorzips=sorted(WORK.glob("bootchain-wide-*-v20-*.zip"),
                         key=lambda x:x.stat().st_mtime,reverse=True)
        if not priorzips:ap.error("Owner V20 ZIP required in ~/mpfree-work")
        source=V17.regular_file(priorzips[0])
    old=verified_prior(source)
    target=WORK/("bootchain-wide-"+datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
                 +"-v21-"+str(os.getpid()))
    target.mkdir(mode=0o700)
    started=time.monotonic();stages={}
    print("MPFree V21: new registered backend value-flow and genuine indirect ARM callback tracing",flush=True)
    print("SHA-verified V20 source:",source,flush=True)
    try:
        with tempfile.TemporaryDirectory(dir=WORK,prefix="mpfree-v21-owner-p3-") as temp:
            temporary=Path(temp)
            with ThreadPoolExecutor(max_workers=2) as pool:
                jobs={"ghidra_p3":pool.submit(timed,ghidra,target,temporary)}
                if not args.skip_vm:
                    jobs["new_actual_arm_guest"]=pool.submit(
                        timed,guest,target,temporary,jobs["ghidra_p3"],args)
                for done in as_completed(jobs.values()):
                    key=next(k for k,v in jobs.items() if v is done)
                    stages[key]=done.result()
                    print("V21",key,
                          "PASS" if stages[key].get("success") else "INCOMPLETE",
                          stages[key].get("actual_wall_seconds"),"s",
                          stages[key].get("error",""),flush=True)
    except KeyboardInterrupt:
        stages["interrupted"]={"success":False}
    except Exception:
        stages["unhandled"]={"success":False,
                             "traceback":traceback.format_exc()[-4500:]}
    finally:
        try: findings=analyze(target,old,stages)
        except Exception:
            findings={}
            (target/"V21-ANALYSIS-ERROR.log").write_text(traceback.format_exc()[-5000:])
        write(target/"V21-STAGE-MANIFEST.json",{
            "schema":"mpfree-v21-actual-backend-registry-dataflow",
            "branch":BRANCH,"time_utc":datetime.now(timezone.utc).isoformat(),
            "duration_seconds":round(time.monotonic()-started,2),
            "prior_v20_sha256":digest(source),
            "owner_P3_sha256":SHA,
            "stages":stages,"new_owner_arm_cases_requested":len(cases()),
            "new_owner_arm_cases_observed":stages.get("new_actual_arm_guest",{}).get("tests_observed",0),
            "new_Ghidra_value_witnesses":findings.get("new_static_store_value_witnesses",0),
            "hardware_contact":False,"USB_or_eMMC_pass_through":False,
            "original_vendor_bin_archived":False,"host_storage_written":False})
        text=["# MPFree V21 — backend registry and indirect call provenance","",
              "## Actual concurrent stage results"]
        for k,row in stages.items():
            text.append("- %s: %s; %.2fs; %s"%(k,
                "PASS" if row.get("success") else "INCOMPLETE",
                row.get("actual_wall_seconds",0),row.get("error","")))
        text+=["","## NEW research output",
            "- Full owner-P3 Ghidra High P-code value-side STORE dependencies,",
            "  backend registration candidates, executable pointer literals,",
            "  linked-list sentinel references and MMC/boot command string xrefs.",
            "- Real owner backend linked-list search and +0xc8 projection",
            "  in private ARM QEMU with synthetic list entries.",
            "- Actual owner storage wrapper success path, mock callbacks and",
            "  guest PC/LR/R5/R8 crash diagnostics if the old SIGILL recurs.",
            "- All 85 original questions retained with exact evidence pointers.",
            "",
            "## Proof limits",
            "- Runtime test uses fake-media backend and synthetic list.",
            "- Ghidra pointer assignment candidate is not the real registered backend.",
            "- No physical eMMC write, SD boot, RK3288 BootROM or restore proven.",
            "- Guest proprietary firmware bytes never enter this ZIP.",""]
        (target/"V21-READ-ME-FIRST.md").write_text("\n".join(text))
        archive,n=package(target)
        print("\nONE PRIVATE ZIP:",archive,flush=True)
        print("SHA256:",digest(archive),"text research files",n,flush=True)
if __name__=="__main__":main()
