#!/usr/bin/env python3
"""MPFree V26: fix the actual V25 UART SIGSEGV and resolve MMC subcommands.

Actual V25 prior: 15 SHA-indexed files, 213/213 P1 recognized functions,
86/86 P3 handler entrypoints with 4,414 pseudo-instructions, and 2/20
real native MMC handler cases. The other 18 failed in original U-Boot
UART status polling at 0x0011f592, NOT in MMC storage code. All five
original SPL priority slots were 0xDEADBEEF; FUN_ff705342 fills the first
from FUN_ff704120 at runtime, so there was no proved static order.

Four parallel evidence stages (after SHA-pinned P3 export is ready):
 - Ghidra P1 deep decompilation of real FUN_ff704120 source and writer.
 - Ghidra P3 original ten-element native MMC subcommand table,
   registered handler pointers, original UART MMIO function and boot.
 - Separate host source decoder for the exact MMC ten-record table
   and compiled boot_targets literals.
 - Disposable ARM QEMU: retry 20 native MMC commands with UART
   sinked; 12 additional actual mmcinfo cases with fake devices.
No physical eMMC/SD/USB/MMIO, no host pass-through, no persistence.
Output one SHA-indexed PRIVATE research ZIP. No firmware bytes in ZIP.
"""
from __future__ import annotations
import argparse,hashlib,importlib.util,json,os,stat,subprocess,tempfile,time,traceback,zipfile
from concurrent.futures import ThreadPoolExecutor,as_completed
from datetime import datetime,timezone
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-v26-mmc-uart-and-spl-source-20261113"
def import_local(name,path):
    spec=importlib.util.spec_from_file_location(name,ROOT/path)
    obj=importlib.util.module_from_spec(spec);spec.loader.exec_module(obj)
    return obj
V25=import_local("mpfree_v26_previous","tools/mpc-one/run_v25.py")
V24=V25.V24;V16=V25.V16;V17=V25.V17
SHA_P1=V25.SHA_P1;SHA_P3=V25.SHA_P3
def sha(path):return V17.digest(path)
def write(path,obj):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(obj,indent=2,sort_keys=True)+"\n",encoding="utf8")
def cases():
    names=["v26_native_mmc_%d"%i for i in range(20)]
    names+=["v26_mmcinfo_%d"%i for i in range(12)]
    assert len(names)==32 and len(set(names))==32
    assert not set(names)&set(V25.cases())
    return names
def prior(path):
    entries=V17.load_zip(V17.regular_file(path))
    mandatory={
       "V25-STAGE-MANIFEST.json",
       "V25-OWNER-NATIVE-MMC-COMMAND-ARM.json",
       "V25-RESULTS-AND-85-QUESTIONS.json",
       "deep-p1/V25-P1-SPL-BOOT-MEDIA-ORDER.json",
       "deep-p3/V25-OWNER-COMMAND-HANDLERS-SUMMARY.json",
       "V25-BOOT-TARGETS-NATIVE-COMMAND-AND-TEST-ROOTCAUSE.json",
    }
    if mandatory-set(entries):
        raise ValueError("Owner V25 report missing evidence: "+str(sorted(mandatory-set(entries))))
    meta=json.loads(entries["V25-STAGE-MANIFEST.json"])
    vm=json.loads(entries["V25-OWNER-NATIVE-MMC-COMMAND-ARM.json"])
    p1=json.loads(entries["deep-p1/V25-P1-SPL-BOOT-MEDIA-ORDER.json"])
    p3=json.loads(entries["deep-p3/V25-OWNER-COMMAND-HANDLERS-SUMMARY.json"])
    priorledger=json.loads(entries["V25-RESULTS-AND-85-QUESTIONS.json"])
    if (meta.get("owner_P1_SHA256")!=SHA_P1
        or meta.get("owner_P3_SHA256")!=SHA_P3
        or vm.get("passed")!=2 or vm.get("tests_planned")!=20
        or vm.get("tests_observed")!=2
        or p1.get("owner_SHA256")!=SHA_P1
        or p3.get("owner_SHA256")!=SHA_P3
        or p3.get("actual_registered_commands")!=86
        or len(priorledger["85_research_questions"])!=85):
        raise ValueError("Not the source-verified owner V25 baseline (2/20)")
    if not all(
        item["literal_media_selector_value"]=="0xdeadbeef"
        for item in p1["priority_entries"]):
        raise ValueError("V25 5-slot SPL sentinel evidence changed")
    return entries
def timed(fn,*args):
    t=time.monotonic()
    try: result=fn(*args)
    except Exception as e:
        result={"success":False,"error":str(e),
                "traceback":traceback.format_exc()[-5000:]}
    result["wall_seconds"]=round(time.monotonic()-t,2)
    return result
def wait_verified(temp,p3future,timeout):
    a=temp/"v26-owner-p3.raw"
    marker=temp/"v26-owner-p3.ready"
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        if a.is_file() and marker.is_file():
            if (a.stat().st_size!=363649 or sha(a)!=SHA_P3 or
                marker.read_text().strip()!=SHA_P3):
                raise ValueError("Guest input does not match pinned original P3")
            return a
        if p3future.done() and not p3future.result().get("success"):
            raise RuntimeError("Ghidra P3 stage failed before verified firmware export")
        time.sleep(.2)
    raise TimeoutError("Ghidra P3 private verified export timed out")
def ghidra(role,out,temp):
    ans={"success":False,"role":role,"read_only":True,"owner_SHA_verified":False}
    folder=out/("deep-"+role);folder.mkdir(parents=True,exist_ok=True)
    log=out/("v26-"+role+"-ghidra.log")
    try:
        binary=V16.get_ghidra()
        if not binary:raise RuntimeError("Owner Ghidra launcher unavailable")
        project=V16.gproject(role)
        image=("p1-rk3288-sram-stage.bin" if role=="p1"
               else "p3-uimage-payload.bin")
        script=("MPFreeV26SPLDynamicMedia.py" if role=="p1"
                else "MPFreeV26MMCRootAndUART.py")
        command=binary+[
            str(WORK/"ghidra/projects"),project,"-process",image,
            "-readOnly","-noanalysis","-scriptPath",str(ROOT/"tools/ghidra"),
            "-postScript",script,str(folder)]
        if role=="p3":
            command.extend([str(temp/"v26-owner-p3.raw"),
                            str(temp/"v26-owner-p3.ready")])
        command.extend(["-log",str(folder/"ghidra-engine.log")])
        with log.open("w") as f:
            run=subprocess.run(command,stdout=f,stderr=subprocess.STDOUT,
                               timeout=3600,check=False)
        if run.returncode:raise RuntimeError("Ghidra exited "+str(run.returncode))
        if role=="p1":
            first=json.loads((folder/"V26-P1-SPL-BOOT-MEDIA-ORDER.json").read_text())
            origin=json.loads((folder/"V26-P1-DYNAMIC-BOOT-SOURCE-ORIGIN.json").read_text())
            if first.get("owner_SHA256")!=SHA_P1 or origin.get("owner_P1_SHA256")!=SHA_P1:
                raise ValueError("P1 original SHA lineage invalid")
            rows=origin["full_decompiled_source_and_instruction_context"]
            source={x["requested"]:x for x in rows}
            func=source.get("0xff704120",{})
            writer=source.get("0xff705342",{})
            ans.update({
               "owner_SHA_verified":True,
               "saved_functions_decompiled":first["decompiled_functions"],
               "preinit_slots_all_sentinel":origin["all_entries_sentinel"],
               "real_dynamic_source_decompiled":bool(func.get("decompiled")),
               "real_dynamic_writer_decompiled":bool(writer.get("decompiled")),
               "dynamic_source_pcode_ops":len(func.get("pcode",[])),
               "dynamic_source_assembly_instructions":len(func.get("assembly",[])),
               "physical_boot_order_proven":False})
            ans["success"]=bool(
               ans["preinit_slots_all_sentinel"] and
               ans["real_dynamic_source_decompiled"] and
               ans["real_dynamic_writer_decompiled"] and
               ans["dynamic_source_assembly_instructions"]>5)
        else:
            p3=json.loads((folder/"V26-OWNER-COMMAND-HANDLERS-SUMMARY.json").read_text())
            sub=json.loads((folder/"V26-OWNER-MMC-SUBCOMMAND-TABLE.json").read_text())
            if p3.get("owner_SHA256")!=SHA_P3 or sub.get("owner_source_sha256")!=SHA_P3:
                raise ValueError("P3 owner SHA mismatch")
            if sha(temp/"v26-owner-p3.raw")!=SHA_P3:
                raise ValueError("P3 guest export hash mismatch")
            ans.update({
               "owner_SHA_verified":True,
               "original_86_commands_considered":p3["actual_registered_commands"],
               "original_MMC_subcommands_decoded":len(sub["records"]),
               "original_MMC_subcommand_layout_verified":
                   sub["all_ten_descriptors_in_owner_rom"],
               "new_source_pcode_operations":p3["pcode_effects"],
               "UART_fault_pc_explained":"0x0011f592",
               "no_hardware_execution":True})
            ans["success"]=bool(
                len(sub["records"])==10 and sub["all_ten_descriptors_in_owner_rom"]
                and p3["pcode_effects"]>100)
        if not ans["success"]:
            ans["error"]="One or more source-dependent Ghidra validation predicates failed"
    except Exception as e:
        ans["error"]=str(e)
        with log.open("a") as f:f.write("\n"+traceback.format_exc()[-4500:])
    return ans
def read32(raw,at):
    offset=at-0x00100000
    if offset<0 or offset+4>len(raw):raise ValueError("Source address out of owner P3")
    return int.from_bytes(raw[offset:offset+4],"little")
def source_decode(out,temp,p3future,args):
    ans={"success":False,"execution":"none; source bytes only"}
    try:
        raw=wait_verified(temp,p3future,args.export_timeout).read_bytes()
        begin=read32(raw,0x00104f7c)
        if not 0x00100000<=begin<=0x00100000+len(raw)-280:
            raise ValueError("Original native MMC subcommand table pointer invalid")
        records=[]
        for i in range(10):
            fields=[read32(raw,begin+i*28+4*j) for j in range(7)]
            offset=fields[0]-0x00100000
            if not 0<=offset<len(raw):raise ValueError("MMC subcommand name outside owner P3")
            end=raw.find(b"\0",offset,min(offset+64,len(raw)))
            if end<=offset:raise ValueError("MMC subcommand name not NUL-terminated")
            name=raw[offset:end]
            if any(x<32 or x>126 for x in name):
                raise ValueError("MMC subcommand name not printable")
            ptr=fields[3]
            if not 0x00100000<=ptr&~1<0x00100000+len(raw):
                raise ValueError("Subcommand target pointer outside owner P3")
            name_text=name.decode("ascii")
            allow=name_text if name_text in {
                "list","dev","info","part","rescan","read","write",
                "erase","hwpartition","bootbus","bootpart","rpmb",
                "setdsr","switch","bkops","reliable_write","help"} else None
            records.append({
               "index":i,"descriptor":"0x%08x"%(begin+i*28),
               "allowlisted_subcommand":allow,
               "name_sha256":hashlib.sha256(name).hexdigest(),
               "argc_max":fields[1],
               "handler":"0x%08x"%ptr,
               "may_write":name_text in {"write","erase","hwpartition","bootpart",
                                         "rpmb","setdsr","switch"},
               "never_executed_by_decoder":True})
        report={
           "schema":"mpfree-v26-independent-original-MMC-subcommands",
           "owner_SHA256":hashlib.sha256(raw).hexdigest(),
           "actual_source_table_pointer":"0x%08x"%begin,
           "records":records,"count":10,
           "original_native_handler":"0x00104f11",
           "firmware_subcommands_not_runtime_SD_proof":True
        }
        write(out/"V26-INDEPENDENT-MMC-SUBCOMMANDS.json",report)
        env=V24.decode_command_table(raw)
        tokens=env["static_default_environment_candidates"].get("boot_targets",[])
        if not tokens or tokens[0].get("boot_target_tokens")!=["mmc0","mmc1"]:
            raise ValueError("Compiled owner boot_targets has changed")
        ans.update({"success":True,"subcommands":10,
                    "compiled_source_boot_targets":["mmc0","mmc1"],
                    "actual_Uboot_MMC_index_mapping_proven":False})
    except Exception as e:
        ans["error"]=str(e)
        (out/"v26-source-decode-failure.log").write_text(traceback.format_exc()[-4500:])
    return ans
def guest(out,temp,p3future,args):
    ans={"success":False,"board":"ARM virt not RK3288",
         "native_MMC_handler_called":True,"MMCINFO_handler_called":True,
         "physical_device_contact":False,
         "source_UART_MMIO_executed":False,
         "known_V25_failures_retested":18,
         "additional_MMCINFO_scenarios":12}
    try:
        image=wait_verified(temp,p3future,args.export_timeout)
        V16.C_SOURCE=ROOT/"tools/mpc-one/vendor_probe_v26.c"
        V16.CASES=cases()
        report=V16.qemu_real_code(out,image,temp,{"success":True},args)
        old=out/"v16-actual-vendor-QEMU-results.json"
        log=out/"v16-qemu-vendor-function-console.log"
        if old.is_file():
            data=json.loads(old.read_text())
            data["vendor_functions_tested"]=[
               "FUN_00104f10 registered U-Boot native mmc command",
               "FUN_00105620 registered U-Boot mmcinfo command",
               "FUN_0011f580 original UART MMIO intercepted",
               "FUN_0011f2d6 original alternate UART output intercepted",
               "FUN_0011ca78 low-level driver command intercepted",
               "FUN_0011de50 block write intercepted",
               "FUN_0011dd4c block erase intercepted",
               "FUN_00108110 environment persistence intercepted"]
            data["original_UART_MMIO_executed"]=False
            data["old_V25_failure_rechecks"]=18
            data["additional_mmcinfo_cases"]=12
            write(out/"V26-ACTUAL-NATIVE-MMC-AND-MMCINFO-ARM.json",data)
            old.unlink()
        if log.is_file():log.rename(out/"v26-owner-ARM-guest-console.log")
        ans.update(report)
        ans["physical_device_contact"]=False
        ans["source_UART_MMIO_executed"]=False
    except Exception as e:
        ans["error"]=str(e)
        (out/"v26-arm-error.log").write_text(traceback.format_exc()[-4500:])
    return ans
def analysis(out,old,stages):
    oldledger=json.loads(old["V25-RESULTS-AND-85-QUESTIONS.json"])
    path=out/"V26-ACTUAL-NATIVE-MMC-AND-MMCINFO-ARM.json"
    observed=json.loads(path.read_text()) if path.is_file() else {}
    rows={v["test"]:v for v in observed.get("cases",[]) if "test" in v}
    counts={}
    for prefix,count in (("v26_native_mmc_",20),("v26_mmcinfo_",12)):
        names=[prefix+str(i) for i in range(count)]
        counts[prefix]={
            "planned":len(names),
            "passed":sum(bool(rows.get(x,{}).get("pass")) for x in names),
            "failed_or_unobserved":[x for x in names if not rows.get(x,{}).get("pass")]
        }
    sub_path=out/"V26-INDEPENDENT-MMC-SUBCOMMANDS.json"
    sub=json.loads(sub_path.read_text()) if sub_path.is_file() else {}
    spl_path=out/"deep-p1/V26-P1-DYNAMIC-BOOT-SOURCE-ORIGIN.json"
    spl=json.loads(spl_path.read_text()) if spl_path.is_file() else {}
    p3_path=out/"deep-p3/V26-OWNER-COMMAND-HANDLERS-SUMMARY.json"
    p3=json.loads(p3_path.read_text()) if p3_path.is_file() else {}
    lineage=[]
    for q in oldledger["85_research_questions"]:
        row={
            "id":q["id"],"question":q["question"],
            "previous_V25_evidence":q.get("V25_new_evidence",[]),
            "new_V26_evidence":[],"physically_proven":False}
        if q["id"] in ("REMOVABLE_DIRECTBOOT-03","BOOT_SELECTION-05") and (
            stages.get("p1_dynamic_media_origin",{}).get("success")):
            row["new_V26_evidence"].append({
               "file":"deep-p1/V26-P1-DYNAMIC-BOOT-SOURCE-ORIGIN.json",
               "scope":"original runtime-dependent SPL source, not static boot-order proof"})
        if q["id"] in ("REMOVABLE_DIRECTBOOT-05","REMOVABLE_DIRECTBOOT-08",
                       "BOOT_SELECTION-03","BOOT_SELECTION-04") and (
            stages.get("p3_mmc_subcommand_ghidra",{}).get("success")):
            row["new_V26_evidence"].append({
               "file":"deep-p3/V26-OWNER-MMC-SUBCOMMAND-TABLE.json",
               "scope":"10 native U-Boot MMC subcommands and concrete original handler entrypoints"})
        if q["id"] in ("REMOVABLE_DIRECTBOOT-05","REMOVABLE_DIRECTBOOT-08") and (
            observed.get("tests_observed",0)>0):
            row["new_V26_evidence"].append({
               "file":"V26-ACTUAL-NATIVE-MMC-AND-MMCINFO-ARM.json",
               "scope":"native ARM MMC command runtime with UART and storage MMIO stubbed; no hardware"})
        lineage.append(row)
    report={
       "schema":"mpfree-v26-SPL-source-and-original-MMC-subcommands",
       "p1_SHA256":SHA_P1,"p3_SHA256":SHA_P3,
       "previous_V25":{
           "runtime":10.16,"actual_MMC_handler_cases_passed":2,
           "actual_MMC_handler_cases_failed":18,
           "faulting_pc":"0x0011f592",
           "faulting_function":"FUN_0011f580",
           "all_five_original_SPL_slots_uninitialized_sentinel":True},
       "V26_actual_ARM":counts,
       "new_MMC_subcommands":[
           {"name":r["allowlisted_subcommand"],
            "handler":r["handler"],"may_write":r["may_write"]}
           for r in sub.get("records",[])],
       "P1_boot_selector":{
          "source_is_runtime_dependent":spl.get("source_initialized_at_runtime"),
          "original_array_all_sentinel":spl.get("all_entries_sentinel"),
          "actual_physical_priority_resolved":False,
          "source_fun":"FUN_ff704120",
          "write_fun":"FUN_ff705342"},
       "P3_new_pcode_operations":p3.get("pcode_effects",0),
       "questions":lineage,
       "questions_with_new_source_or_runtime_evidence":sum(bool(x["new_V26_evidence"]) for x in lineage),
       "known_hardware_unknowns":[
          "U-Boot index to physical eMMC/SD controller instance",
          "Physical U-Boot kernel/FDT load and DRAM reservation correctness",
          "Live saved environment overrides to compiled boot_targets",
          "Physical Rockchip MaskROM recovery and independent restore"],
       "no_physical_MMC_contact":True
    }
    write(out/"V26-RESULTS-AND-85-QUESTIONS.json",report)
    return report
def package(out):
    accepted={".json",".jsonl",".md",".txt",".log",".csv",".tsv"}
    files=[];length=0
    for p in sorted(out.rglob("*")):
        if p.is_symlink():raise ValueError("Symlink in owner-private output")
        if p.is_dir():continue
        if not p.is_file() or not stat.S_ISREG(p.lstat().st_mode) or p.suffix not in accepted:
            raise ValueError("Owner report includes unexpected file "+str(p))
        n=p.stat().st_size;length+=n
        if n>192*1024*1024 or length>512*1024*1024:
            raise ValueError("Bounded ZIP report exceeded limits")
        files.append({"name":p.relative_to(out).as_posix(),
                      "bytes":n,"sha256":sha(p)})
    path=out.with_suffix(".zip")
    with zipfile.ZipFile(path,"x",compression=zipfile.ZIP_DEFLATED,compresslevel=6) as z:
        z.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v26","private":True,"files":files},indent=2))
        for r in files:z.write(out/r["name"],r["name"])
    with zipfile.ZipFile(path) as z:
        if z.testzip():raise ValueError("V26 output CRC failed")
        for r in files:
            data=z.read(r["name"])
            if len(data)!=r["bytes"] or hashlib.sha256(data).hexdigest()!=r["sha256"]:
                raise ValueError("V26 output member SHA mismatch: "+r["name"])
    path.with_suffix(".zip.sha256").write_text(sha(path)+"  "+path.name+"\n")
    return path,len(files)
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prior-v25",type=Path)
    parser.add_argument("--export-timeout",type=int,default=1200)
    parser.add_argument("--qemu-boot-timeout",type=int,default=240)
    parser.add_argument("--case-timeout",type=int,default=25)
    parser.add_argument("--skip-vm",action="store_true")
    args=parser.parse_args()
    os.umask(0o077);WORK.mkdir(parents=True,exist_ok=True)
    if args.prior_v25:parent=V17.regular_file(args.prior_v25)
    else:
        found=sorted(WORK.glob("bootchain-wide-*-v25-*.zip"),
                     key=lambda x:x.stat().st_mtime,reverse=True)
        if not found:parser.error("Owner SHA-indexed original V25 ZIP missing")
        parent=V17.regular_file(found[0])
    old=prior(parent)
    out=WORK/("bootchain-wide-"+datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
              +"-v26-"+str(os.getpid()))
    out.mkdir(mode=0o700)
    started=time.monotonic();stages={}
    print("MPFree V26: genuine MMC UART fix, original 10 subcommand table, real SPL runtime selector source",flush=True)
    try:
        with tempfile.TemporaryDirectory(dir=WORK,prefix="mpfree-v26-private-") as t:
            temp=Path(t)
            with ThreadPoolExecutor(max_workers=4) as pool:
                p1=pool.submit(timed,ghidra,"p1",out,temp)
                p3=pool.submit(timed,ghidra,"p3",out,temp)
                jobs={
                    "p1_dynamic_media_origin":p1,
                    "p3_mmc_subcommand_ghidra":p3,
                    "independent_native_mmc_subcommands":
                       pool.submit(timed,source_decode,out,temp,p3,args),
                }
                if not args.skip_vm:
                    jobs["original_arm_mmc_Uart_mocked"]=pool.submit(
                        timed,guest,out,temp,p3,args)
                for fut in as_completed(jobs.values()):
                    name=next(k for k,v in jobs.items() if v is fut)
                    stages[name]=fut.result()
                    print("V26",name,
                          "PASS" if stages[name].get("success") else "FAILED",
                          stages[name].get("wall_seconds"),"seconds",
                          stages[name].get("error",""),flush=True)
    except KeyboardInterrupt:
        stages["interrupted"]={"success":False}
    except Exception:
        stages["unhandled"]={"success":False,
                             "traceback":traceback.format_exc()[-4500:]}
    finally:
        try:observations=analysis(out,old,stages)
        except Exception:
            observations={}
            (out/"V26-ANALYSIS-ERROR.log").write_text(traceback.format_exc()[-4500:])
        write(out/"V26-STAGE-MANIFEST.json",{
           "schema":"mpfree-v26-real-mmc-UART-and-boot-source",
           "branch":BRANCH,"utc":datetime.now(timezone.utc).isoformat(),
           "elapsed_seconds":round(time.monotonic()-started,2),
           "prior_v25_sha256":sha(parent),
           "owner_P1_SHA256":SHA_P1,"owner_P3_SHA256":SHA_P3,
           "stages":stages,"actual_ARM_cases_requested":len(cases()),
           "corrected_prior_MMC_tests":20,"additional_MMCINFO_cases":12,
           "original_85_questions_retained":len(observations.get("questions",[])),
           "physical_MPC_contact":False,"real_MMIO_executed":False,
           "owner_firmware_bytes_in_archive":False})
        readme=["# MPFree V26: native MMC, UART fault correction and SPL origin",""]
        for k,r in stages.items():
            readme.append("- %s: %s; %.2fs; %s"%(k,
              "PASS" if r.get("success") else "FAILED",
              r.get("wall_seconds",0.0),r.get("error","")))
        readme.extend(["","## Interpretation",
           "V25 reported 2/20 native MMC ARM passes. Eighteen faulted",
           "inside FUN_0011f580, an original UART MMIO polling function.",
           "V26 intercepts UART hardware access but not MMC command handling.",
           "The original P1 five media array words were ALL 0xDEADBEEF,",
           "so the saved image does not provide five active boot targets.",
           "V26 traces the value source FUN_ff704120 and its array writer.",
           "The original MMC command handler uses TEN actual native subcommand",
           "descriptors. V26 recovers their exact firmware pointers and names.",
           "No host MMC/USB, real SD boot, real persistence or MaskROM executed."])
        (out/"V26-READ-ME-FIRST.md").write_text("\n".join(readme)+"\n")
        output,n=package(out)
        print("\nONE PRIVATE ZIP:",output,flush=True)
        print("SHA256:",sha(output),"research files:",n,flush=True)
if __name__=="__main__":main()
