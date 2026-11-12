#!/usr/bin/env python3
"""MPFree V20: full owner-P3 backend/command Ghidra census and real ARM tests.

Runs concurrent read-only SHA-pinned Ghidra plus isolated actual-U-Boot
ARM-virt tests of the backend's indirect persistence callback and command
interpreter. All real storage/command callbacks are stubbed in guest RAM.
Outputs ONE private ZIP. No MPC/USB/eMMC contact or firmware writes.
"""
from concurrent.futures import ThreadPoolExecutor,as_completed
from datetime import datetime,timezone
import argparse,hashlib,importlib.util,json,os,stat,subprocess,tempfile,time,traceback,zipfile
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-v20-storage-command-20261112"
def module(rel,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/rel)
    obj=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj
V19=module("tools/mpc-one/run_v19.py","mpfree_v20_prior")
V17=V19.V17
V16=V19.V16
PINNED=V19.SHA_P3
def write(path,obj):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(obj,indent=2,sort_keys=True)+"\n",encoding="utf8")
def cases():
    out=["v20_storage_%u"%n for n in range(6)]
    out.extend("v20_command_%u_%u"%(n,mode)
               for n in range(8) for mode in (0,1))
    assert len(out)==22 and len(set(out))==22
    assert not set(out)&set(V19.selected_cases())
    assert not set(out)&set(V17.tested_cases())
    return out
def prior(path):
    data=V17.load_zip(V17.regular_file(path))
    required={"V19-STAGE-MANIFEST.json","V19-85-QUESTION-EVIDENCE.json",
              "deep-p3/V19-p3-SUMMARY.json","V19-ACTUAL-P3-ARM-RUNTIME.json"}
    if not required.issubset(data):raise ValueError("Missing V19 source lineage")
    previous=json.loads(data["deep-p3/V19-p3-SUMMARY.json"])
    if not previous.get("source_hash_verified") or previous.get("owner_sha256")!=PINNED:
        raise ValueError("V19 owner's firmware image checksum mismatch")
    questions=json.loads(data["V19-85-QUESTION-EVIDENCE.json"])
    if len(questions.get("questions",[]))!=85:
        raise ValueError("V19 prior research ledger missing 85 questions")
    return data
def timed(fn,*args):
    before=time.monotonic()
    try:result=fn(*args)
    except Exception as e:
        result={"success":False,"error":str(e),
                "traceback":traceback.format_exc()[-3500:]}
    result["wall_seconds"]=round(time.monotonic()-before,2)
    return result
def ghidra(out,tmp):
    res={"success":False,"source_verified":False}
    path=out/"deep-p3";path.mkdir()
    log=out/"v20-ghidra-p3-console.log"
    try:
        launcher=V16.get_ghidra()
        if not launcher:raise RuntimeError("Missing owner saved Ghidra launcher")
        name=V16.gproject("p3")
        raw=tmp/"owner-P3.bin"
        marker=tmp/"owner-P3.ready"
        command=launcher+[str(WORK/"ghidra/projects"),name,
            "-process","p3-uimage-payload.bin","-readOnly","-noanalysis",
            "-scriptPath",str(ROOT/"tools/ghidra"),
            "-postScript","MPFreeV20BackendCensus.py",
            str(path),str(raw),str(marker),
            "-log",str(path/"ghidra-engine.log")]
        with log.open("w") as handle:
            proc=subprocess.run(command,stdout=handle,stderr=subprocess.STDOUT,
                                timeout=3600,check=False)
        if proc.returncode:raise RuntimeError("Ghidra exit: "+str(proc.returncode))
        report=json.loads((path/"V20-P3-BACKEND-COMMAND-SUMMARY.json").read_text())
        if (not report.get("owner_input_verified") or
            report.get("owner_sha256")!=PINNED or
            report.get("schema")!="mpfree-v20-full-p3-field-and-command-census"):
            raise RuntimeError("Incorrect Ghidra P3 SHA or schema")
        res.update({"source_verified":True,"recognized":report["functions_attempted"],
                    "decompiled":report["functions_decompiled"],
                    "dataflow_indirect":report["indirect_sites"],
                    "callback_field_stores":report["store_relevant"],
                    "decompilation_failures":len(report["failures"])})
        res["success"]=bool(res["decompiled"]/max(1,res["recognized"])>=.96 and
                            res["dataflow_indirect"]>0 and res["callback_field_stores"]>0)
        if not res["success"]:res["error"]="Ghidra failed required coverage/effect checks"
    except Exception as exc:
        res["error"]=str(exc)
        with log.open("a") as handle:
            handle.write("\n"+traceback.format_exc()[-3000:])
    return res
def guest(out,tmp,ghidra_future,args):
    result={"success":False,"board":"ARM virt, not RK3288","new_cases":len(cases()),
            "no_host_usb_or_storage":True}
    raw=tmp/"owner-P3.bin";marker=tmp/"owner-P3.ready"
    limit=time.monotonic()+args.export_timeout
    while time.monotonic()<limit:
        if raw.is_file() and marker.is_file():break
        if ghidra_future.done() and not ghidra_future.result().get("success"):
            return {**result,"error":"Ghidra P3 failed before verified export"}
        time.sleep(.2)
    else:return {**result,"error":"Verified owner P3 export timed out"}
    try:
        if (raw.stat().st_size!=363649 or V17.digest(raw)!=PINNED or
            marker.read_text().strip()!=PINNED):
            raise RuntimeError("Temporary P3 source failed SHA verification")
        V16.C_SOURCE=ROOT/"tools/mpc-one/vendor_probe_v20.c"
        V16.CASES=cases()
        run=V16.qemu_real_code(out,raw,tmp,{"success":True},args)
        source=out/"v16-actual-vendor-QEMU-results.json"
        serial=out/"v16-qemu-vendor-function-console.log"
        if source.is_file():source.rename(out/"V20-ACTUAL-VENDOR-STORAGE-COMMAND-ARM.json")
        if serial.is_file():serial.rename(out/"v20-guest-console.log")
        result.update(run)
        result["previous_case_repeats"]=0
    except Exception as e:
        result["error"]=str(e)
        (out/"v20-guest-error.log").write_text(traceback.format_exc()[-3500:])
    return result
def correlate(out,previous,stages):
    old=json.loads(previous["V19-85-QUESTION-EVIDENCE.json"])
    p=out/"deep-p3/V20-P3-BACKEND-COMMAND-SUMMARY.json"
    static=json.loads(p.read_text()) if p.exists() else {}
    vm=out/"V20-ACTUAL-VENDOR-STORAGE-COMMAND-ARM.json"
    dynamic=json.loads(vm.read_text()) if vm.exists() else {}
    result_by_case={c.get("test"):c for c in dynamic.get("cases",[])}
    storage=all(result_by_case.get("v20_storage_%u"%n,{}).get("pass") for n in range(6))
    parser=all(result_by_case.get("v20_command_%u_%u"%(n,m),{}).get("pass")
               for n in range(8) for m in (0,1))
    decomp=stages.get("ghidra_p3",{}).get("success",False)
    question_ids={
       "FASTBOOT_RUNTIME-02","REMOVABLE_DIRECTBOOT-01",
       "REMOVABLE_DIRECTBOOT-03","REMOVABLE_DIRECTBOOT-05",
       "REMOVABLE_DIRECTBOOT-06","BOOT_SELECTION-04",
       "BOOT_SELECTION-06","RECOVERY_SECURITY-12"}
    rows=[]
    for item in old["questions"]:
        r={"id":item["id"],"question":item["question"],
           "prior_status":item.get("v19_status"),
           "new_probes":[],"answered_by_v20":False}
        if item["id"] in question_ids:
            if decomp:r["new_probes"].append("deep-p3/V20-P3-BACKEND-COMMAND-SUMMARY.json")
            if storage or parser:
                r["new_probes"].append("V20-ACTUAL-VENDOR-STORAGE-COMMAND-ARM.json")
        rows.append(r)
    findings={
        "schema":"mpfree-v20-provenance-and-questions",
        "owner_P3_SHA256":PINNED,"new_static_functions":static.get("functions_decompiled",0),
        "new_vendor_cases_planned":len(cases()),
        "new_vendor_cases_observed":dynamic.get("tests_observed",0),
        "new_vendor_cases_passed":dynamic.get("passed",0),
        "storage_backend_callback_mock_test_complete":storage,
        "real_Uboot_parser_to_mock_dispatch_complete":parser,
        "provenance_propositions":{
          "backend_callback_method_offset":{
             "value":"0x130","status":"STATIC_SOURCE_MODEL" if decomp else "UNRESOLVED",
             "actual_backend_identity":"UNKNOWN",
             "proof":"deep-p3/V20-P3-BACKEND-COMMAND-SUMMARY.json"},
          "vendor_backend_branching":{
             "status":"ISOLATED_ARM_CALLBACK_REPLACED" if storage else "UNRESOLVED",
             "physical_storage_writes_proven":False},
          "Uboot_command_interpreter_entry":{
             "status":"ISOLATED_ARM_FINAL_COMMAND_DISPATCH_REPLACED" if parser else "UNRESOLVED",
             "working_physical_SD_boot_proven":False}},
        "questions":rows,
        "questions_with_new_probes":sum(bool(r["new_probes"]) for r in rows),
        "risks":["No real backend callback implementation executed",
                 "No physical SD controller or U-Boot command execution",
                 "No bootm/Linux FDT handoff or BootROM proof"]}
    write(out/"V20-EXACT-FINDINGS-AND-85-QUESTIONS.json",findings)
    return findings
def package(out):
    members=[];total=0;types={".json",".jsonl",".log",".md",".txt",".csv",".tsv"}
    for path in sorted(out.rglob("*")):
        if path.is_symlink():raise ValueError("Symlink in private evidence")
        if path.is_dir():continue
        if (not path.is_file() or not stat.S_ISREG(path.lstat().st_mode) or
            path.suffix not in types):
            raise ValueError("Unrecognized private evidence file "+str(path))
        size=path.stat().st_size;total+=size
        if size>192*1024*1024 or total>512*1024*1024:
            raise ValueError("Research evidence exceeds bounded archive limits")
        members.append({"name":path.relative_to(out).as_posix(),
                        "bytes":size,"sha256":V17.digest(path)})
    target=out.with_suffix(".zip")
    with zipfile.ZipFile(target,"x",compression=zipfile.ZIP_DEFLATED,
                         compresslevel=6) as z:
        z.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v20","private":True,"files":members},indent=2))
        for row in members:z.write(out/row["name"],row["name"])
    with zipfile.ZipFile(target) as z:
        if z.testzip():raise ValueError("V20 ZIP CRC failure")
        for row in members:
            b=z.read(row["name"])
            if len(b)!=row["bytes"] or hashlib.sha256(b).hexdigest()!=row["sha256"]:
                raise ValueError("V20 archive SHA mismatch "+row["name"])
    target.with_suffix(".zip.sha256").write_text(
        V17.digest(target)+"  "+target.name+"\n")
    return target,len(members)
def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--prior-v19",type=Path)
    p.add_argument("--qemu-boot-timeout",type=int,default=240)
    p.add_argument("--case-timeout",type=int,default=20)
    p.add_argument("--export-timeout",type=int,default=1200)
    p.add_argument("--skip-vm",action="store_true")
    args=p.parse_args()
    os.umask(0o077);WORK.mkdir(exist_ok=True,parents=True)
    if args.prior_v19: source=V17.regular_file(args.prior_v19)
    else:
        found=sorted(WORK.glob("bootchain-wide-*-v19-*.zip"),
                     key=lambda x:x.stat().st_mtime,reverse=True)
        if not found:p.error("Private V19 ZIP required in ~/mpfree-work")
        source=V17.regular_file(found[0])
    previous=prior(source)
    out=WORK/("bootchain-wide-"+datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
              +"-v20-"+str(os.getpid()))
    out.mkdir(mode=0o700);start=time.monotonic();stages={}
    print("V20 concurrent actual U-Boot backend + command parser and P3 callback census",flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="mpfree-v20-private-",dir=WORK) as td:
            temp=Path(td)
            with ThreadPoolExecutor(max_workers=2) as pool:
                jobs={"ghidra_p3":pool.submit(timed,ghidra,out,temp)}
                if not args.skip_vm:
                    jobs["new_actual_arm_vm"]=pool.submit(
                        timed,guest,out,temp,jobs["ghidra_p3"],args)
                for future in as_completed(jobs.values()):
                    name=next(k for k,v in jobs.items() if v is future)
                    stages[name]=future.result()
                    print("V20",name,"PASS" if stages[name].get("success") else "INCOMPLETE",
                          stages[name].get("wall_seconds"),"seconds",
                          stages[name].get("error",""),flush=True)
    except KeyboardInterrupt:
        stages["interrupted"]={"success":False}
    except Exception:
        stages["uncaught"]={"success":False,
                            "traceback":traceback.format_exc()[-4000:]}
    finally:
        try: findings=correlate(out,previous,stages)
        except Exception:
            findings={}
            (out/"V20-FINDINGS-ERROR.log").write_text(traceback.format_exc()[-4500:])
        write(out/"V20-STAGE-MANIFEST.json",{
            "branch":BRANCH,"timestamp":datetime.now(timezone.utc).isoformat(),
            "elapsed_seconds":round(time.monotonic()-start,2),
            "stages":stages,"prior_v19_sha256":V17.digest(source),
            "owner_p3_sha256":PINNED,"new_arm_case_count":len(cases()),
            "questions_with_actual_new_probes":findings.get("questions_with_new_probes",0),
            "physical_device_contact":False,"vendor_binary_archived":False,
            "USB_MMC_passthrough":False,"persisted_storage_actions":False})
        text=["# MPFree V20 storage backend and U-Boot parser","",
              "Real P3 instructions in isolated ARM virt; no physical device.","",
              "## Execution results"]
        for key,val in stages.items():
            text.append("- %s: %s; %.2f sec; %s"%(key,
                "PASS" if val.get("success") else "INCOMPLETE",
                val.get("wall_seconds",0),val.get("error","")))
        text+=["","## Scope",
            "- Full recognized P3 Ghidra dataflow census for callback/dispatch fields.",
            "- Actual FUN_0010b74c through mock backend+0x130 callback.",
            "- Actual U-Boot command wrapper/parser; final dispatch stubbed.",
            "- No actual eMMC write, saveenv, SD boot, bootm or RK3288 MaskROM proof."]
        (out/"V20-READ-ME-FIRST.md").write_text("\n".join(text)+"\n")
        archive,n=package(out)
        print("\nONE PRIVATE ZIP:",archive,flush=True)
        print("SHA256:",V17.digest(archive),"report files",n,flush=True)
if __name__=="__main__":
    main()
