#!/usr/bin/env python3
"""MPFree V24 -- real U-Boot command registry + MMC device-number investigation.

Unlike V17-V23, investigates REAL 7-word U-Boot command descriptors,
actual handler pointers, the genuine command-lookup function, MMC ID lookup
and the source-level distinction between normal SD boot and DFU updater.
Ghidra P1/P3, independent host raw-table decoder and isolated ARM QEMU
execute concurrently after SHA-verified private P3 source export.

No registered command handler is executed by the VM. No host USB/eMMC/SD,
saveenv, flash, erase, MMC driver MMIO or physical MPC access.
ONE SHA-indexed private research ZIP, never owner firmware bytes.
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
import re
import stat
import subprocess
import tempfile
import time
import traceback
import zipfile

ROOT=Path(__file__).resolve().parents[2]
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-v24-uboot-command-registry-20261113"
def load(name,file):
    spec=importlib.util.spec_from_file_location(name,ROOT/file)
    mod=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod
V23=load("mpfree_v24_prior_v23","tools/mpc-one/run_v23.py")
V17=V23.V17
V16=V23.V16
SHA_P1=V23.SHA_P1
SHA_P3=V23.SHA_P3
P3BASE=0x00100000
P3SIZE=363649
SAFE_NAMES={
    "mmc","mmcinfo","load","ls","ext4load","fatload","ext4ls","fatls",
    "bootz","bootm","booti","boot","source","sysboot","run",
    "env","printenv","setenv","saveenv","help","echo","test","fdt",
    "part","usb","dfu","fastboot","reset","reboot","version",
    "crc32","iminfo","imxtract","setexpr","sleep","md","mw",
    "cp","erase","sf","nand","blk","fstype","pci","gpio",
    "dhcp","tftpboot","ping"}
KEY_TERMS=(
    b"boot_targets\0",b"bootcmd\0",b"bootdelay\0",b"bootargs\0",
    b"mmcdev\0",b"mmcpart\0",b"kernel_addr_r\0",b"fdt_addr_r\0",
    b"scriptaddr\0",b"distro_bootcmd\0",b"boot.scr.uimg\0",
    b"bootz\0",b"source\0",b"ext4load\0",b"fatload\0",
)
def record(path,data):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(data,indent=2,sort_keys=True)+"\n",encoding="utf8")
def digest(path):return V17.digest(path)
def regular(path):return V17.regular_file(path)
def cases():
    names=["v24_cmd_%u"%i for i in range(32)]
    names+=["v24_mmc_%u"%i for i in range(16)]
    assert len(names)==48 and len(names)==len(set(names))
    assert set(names).isdisjoint(V23.cases())
    return names
def prior(path):
    files=V17.load_zip(regular(path))
    required={
        "V23-STAGE-MANIFEST.json",
        "V23-EXACT-FINDINGS-AND-85-QUESTIONS.json",
        "V23-ACTUAL-OWNER-DRIVER-ARM.json",
        "deep-p1/V23-p1-SUMMARY.json",
        "deep-p3/V23-p3-SUMMARY.json",
        "V23-OWNER-DTB-MMC-AND-BOOT-SCRIPT.json"}
    if required-set(files):
        raise ValueError("V23 owner research missing "+str(sorted(required-set(files))))
    manifest=json.loads(files["V23-STAGE-MANIFEST.json"])
    actual=json.loads(files["V23-ACTUAL-OWNER-DRIVER-ARM.json"])
    if (manifest.get("owner_P1_sha256")!=SHA_P1
        or manifest.get("owner_P3_sha256")!=SHA_P3
        or actual.get("passed")!=60 or actual.get("tests_observed")!=60
        or not actual.get("success")):
        raise ValueError("Owner V23 provenance or 60/60 baseline mismatch")
    for role,sha in (("p1",SHA_P1),("p3",SHA_P3)):
        summary=json.loads(files["deep-"+role+"/V23-"+role+"-SUMMARY.json"])
        if summary.get("owner_sha256")!=sha or not summary.get("source_hash_verified"):
            raise ValueError("Source owner-"+role+" SHA mismatch")
    q=json.loads(files["V23-EXACT-FINDINGS-AND-85-QUESTIONS.json"])
    if len(q.get("85_research_questions",[]))!=85:
        raise ValueError("Original 85 questions not preserved")
    return files
def timed(fn,*args):
    t=time.monotonic()
    try:result=fn(*args)
    except Exception as e:
        result={"success":False,"error":repr(e),
                "traceback":traceback.format_exc()[-4500:]}
    result["elapsed_seconds"]=round(time.monotonic()-t,2)
    return result
def checked_raw(temp):
    image=temp/"owner-P3-verbatim.raw"
    marker=temp/"owner-P3.ready"
    if not image.is_file() or not marker.is_file():
        raise RuntimeError("Verified owner P3 temporary export unavailable")
    if image.stat().st_size!=P3SIZE or digest(image)!=SHA_P3:
        raise RuntimeError("Raw U-Boot P3 export failed SHA-256")
    if marker.read_text().strip()!=SHA_P3:
        raise RuntimeError("Ghidra verified source marker invalid")
    return image
def find_word(raw,absolute):
    offset=absolute-P3BASE
    if offset<0 or offset+4>len(raw):return None
    return int.from_bytes(raw[offset:offset+4],"little")
def owner_string(raw,absolute,max_bytes=64):
    offset=absolute-P3BASE
    if offset<0 or offset>=len(raw):return None
    end=raw.find(b"\0",offset,min(offset+max_bytes+1,len(raw)))
    if end<=offset or end-offset>max_bytes:return None
    text=raw[offset:end]
    return text if all(32<=b<=126 for b in text) else None
def decode_command_table(raw):
    """Independent source-byte decoder for REAL U-Boot cmd_tbl_t-like rows.

    Hard-reject bad pointers, field alignment, duplicate command names,
    inconsistent handler pointers; NEVER export unallowlisted name text.
    This does not execute any owner function and is not device evidence.
    """
    if len(raw)!=P3SIZE:raise ValueError("Unexpected U-Boot P3 raw length")
    begin=find_word(raw,0x00110d3c)
    end=find_word(raw,0x00110d40)
    multiplier=find_word(raw,0x00110d44)
    report={
      "schema":"mpfree-v24-owner-command-table-independent-decoder",
      "owner_SHA256":hashlib.sha256(raw).hexdigest(),
      "start":"0x%08x"%begin,"end":"0x%08x"%end,
      "count_multiplier":"0x%08x"%multiplier,
      "kind":"original 7x32-bit U-Boot command descriptors",
      "no_command_handler_executed":True,
      "unallowlisted_command_names_sha256_only":True}
    if (begin>=end or begin<P3BASE or end>P3BASE+P3SIZE or
        (end-begin)%28!=0 or not(5<=(end-begin)//28<=512)):
        raise ValueError("Native U-Boot command-table pointer range/stride invalid")
    count=(end-begin)//28
    records=[];safe_commands={};bad=[];lookup={}
    for n in range(count):
        location=begin+n*28
        fields=[find_word(raw,location+4*i) for i in range(7)]
        name=owner_string(raw,fields[0])
        raw_handler=fields[3]
        sane_code=P3BASE<= (raw_handler&~1) <P3BASE+P3SIZE
        if not name or not sane_code or fields[1]>255:
            bad.append({"descriptor":"0x%08x"%location,
                        "has_ASCII_name":name is not None,
                        "handler_in_original_image":sane_code})
        safe=name.decode("ascii") if name and name.decode("ascii") in SAFE_NAMES else None
        hashed=hashlib.sha256(name).hexdigest() if name is not None else None
        if safe is not None:
            if safe in safe_commands:raise ValueError("Duplicate allowlisted U-Boot command "+safe)
            safe_commands[safe]={"descriptor":"0x%08x"%location,
                                 "handler":"0x%08x"%raw_handler,
                                 "argc":fields[1],
                                 "repeatable_flags":fields[2]}
        key=hashed
        if key in lookup and key is not None:
            raise ValueError("Duplicate SHA of command name in U-Boot registry")
        lookup[key]=location
        records.append({
           "address":"0x%08x"%location,
           "safe_command_name":safe,
           "name_SHA256":hashed,
           "name_length":len(name) if name is not None else None,
           "handler_pointer":"0x%08x"%raw_handler,
           "argument_limit":fields[1],
           "repeatable_flags":fields[2],
           "usage_text_present":owner_string(raw,fields[4]) is not None,
           "help_text_present":owner_string(raw,fields[5]) is not None
        })
    report["descriptor_count"]=count
    report["safe_named_commands"]=safe_commands
    report["rows"]=records
    report["malformed_descriptor_count"]=len(bad)
    report["malformed_descriptor_samples"]=bad[:30]
    report["valid_descriptor_ratio"]=(count-len(bad))/count
    # Source file may contain env defaults as static C strings.
    # Report only whitelisted token locations; never emit values.
    report["boot_environment_literal_sites"]={
       key.decode("ascii").rstrip("\0"): [
          "0x%08x"%(P3BASE+m.start()) for m in
          __import__("re").finditer(__import__("re").escape(key),raw)
       ][:100] for key in KEY_TERMS}
    report["physical_controller_address_literal_sites"]={
       "internal_eMMC_0xff0f0000":[
           "0x%08x"%(P3BASE+m.start()) for m in
           __import__("re").finditer(b"\x00\x00\x0f\xff",raw)][:80],
       "removable_SD_0xff0c0000":[
           "0x%08x"%(P3BASE+m.start()) for m in
           __import__("re").finditer(b"\x00\x00\x0c\xff",raw)][:80],
    }
    report["hardware_MMC_index_assigned"]=False
    report["scope_limitations"]=[
        "Linux DT alias numbering does not prove U-Boot MMC device numbering.",
        "Command registration and source handlers do not prove a working boot.",
        "Even if source or mmc commands exist, actual I/O remains untested.",
        "No unallowlisted command names or vendor credential strings emitted."]
    return report
def ghidra(role,out,temp):
    response={"success":False,"role":role,"SHA_verified":False}
    folder=out/("deep-"+role);folder.mkdir(parents=True,exist_ok=True)
    log=out/("v24-ghidra-"+role+".log")
    try:
        launcher=V16.get_ghidra()
        if not launcher:raise RuntimeError("Original owner Ghidra installation not found")
        project=V16.gproject(role)
        image="p1-rk3288-sram-stage.bin" if role=="p1" else "p3-uimage-payload.bin"
        priv=temp/"owner-P3-verbatim.raw"
        ready=temp/"owner-P3.ready"
        command=launcher+[str(WORK/"ghidra/projects"),project,
            "-process",image,"-readOnly","-noanalysis",
            "-scriptPath",str(ROOT/"tools/ghidra"),
            "-postScript","MPFreeV24CommandRegistry.py",role,str(folder),
            str(priv) if role=="p3" else "none",
            str(ready) if role=="p3" else "none",
            "-log",str(folder/"ghidra-engine.log")]
        with log.open("w") as stream:
            process=subprocess.run(command,stdout=stream,stderr=subprocess.STDOUT,
                                   timeout=3600,check=False)
        if process.returncode:raise RuntimeError("Owner Ghidra exited "+str(process.returncode))
        report=json.loads((folder/("V24-"+role+"-SUMMARY.json")).read_text())
        exact=SHA_P1 if role=="p1" else SHA_P3
        if (report.get("schema")!="mpfree-v24-native-command-registry-and-media-v1"
            or not report.get("source_hash_verified") or
            report.get("owner_sha256")!=exact):
            raise RuntimeError("Ghidra source SHA/schema missing")
        stats=report["stats"]
        total=report["traced_function_count"]
        ok=stats.get("functions_decompiled",0)
        output=folder/("V24-"+role+"-PATH-WITNESSES.jsonl")
        if not output.is_file() or output.stat().st_size==0:
            raise RuntimeError("No newly traced Ghidra High P-code")
        response.update({
          "SHA_verified":True,"recognized_functions":report["known_functions"],
          "targeted_functions":total,"functions_decompiled":ok,
          "memory_store_dependencies":stats.get("deep_STORE",0),
          "indirect_call_dependencies":stats.get("deep_CALLIND",0),
          "decompilation_failures":len(report["decompilation_failures"])})
        response["success"]=bool(total>=40 and ok/max(1,total)>=.95 and
                                 response["memory_store_dependencies"]>0 and
                                 response["indirect_call_dependencies"]>0)
        if role=="p3":
            command_registry=report["original_command_registry"]
            response["command_table_layout_verified"]=command_registry["layout_verified"]
            response["safe_registered_command_handlers"]=len(command_registry.get("candidate_handler_roots",[]))
            checked_raw(temp)
            if not command_registry["layout_verified"]:
                response["success"]=False
                response["error"]="Owner command table could not be validated as 7-word registry"
        if not response["success"] and "error" not in response:
            response["error"]="Insufficient real Ghidra coverage or effect evidence"
    except Exception as exc:
        response["success"]=False
        response["error"]=str(exc)
        with log.open("a") as stream:stream.write(traceback.format_exc()[-4000:])
    return response
def wait_image(temp,p3future,deadline):
    image=temp/"owner-P3-verbatim.raw"
    marker=temp/"owner-P3.ready"
    while time.monotonic()<deadline:
        if image.is_file() and marker.is_file():
            checked_raw(temp)
            return image
        if p3future.done() and not p3future.result().get("success"):
            raise RuntimeError("P3 Ghidra failed before verified source export")
        time.sleep(.15)
    raise TimeoutError("Timed out waiting for SHA-verified Ghidra export")
def independent_decoder(out,temp,p3future,args):
    response={"success":False,"mode":"original source-byte independent decode; not guest execution"}
    try:
        image=wait_image(temp,p3future,time.monotonic()+args.export_timeout)
        data=decode_command_table(image.read_bytes())
        if data["owner_SHA256"]!=SHA_P3:
            raise RuntimeError("Host decoder owner SHA mismatch")
        output=out/"V24-ORIGINAL-UBOOT-COMMAND-DESCRIPTORS.json"
        record(output,data)
        response.update({"success":data["valid_descriptor_ratio"]>=.90,
                         "descriptor_count":data["descriptor_count"],
                         "valid_ratio":data["valid_descriptor_ratio"],
                         "safe_command_handlers_found":len(data["safe_named_commands"]),
                         "physical_UBOOT_MMC_index_proven":False})
        if not response["success"]:
            response["error"]="Over 10% of decoded command records failed pointer checks"
    except Exception as exc:
        response["error"]=str(exc)
        (out/"v24-command-decoder-error.log").write_text(traceback.format_exc()[-4500:])
    return response
def guest(out,temp,p3future,args):
    response={"success":False,"generic_ARM_virt_not_RK3288":True,
              "command_handlers_executed":False,"physical_MMC_IO":False,
              "new_cases":len(cases())}
    try:
        image=wait_image(temp,p3future,time.monotonic()+args.export_timeout)
        V16.C_SOURCE=ROOT/"tools/mpc-one/vendor_probe_v24.c"
        V16.CASES=cases()
        experiment=V16.qemu_real_code(out,image,temp,{"success":True},args)
        old=out/"v16-actual-vendor-QEMU-results.json"
        logs=out/"v16-qemu-vendor-function-console.log"
        new=out/"V24-REAL-OWNER-COMMAND-AND-MMC-ARM.json"
        if old.is_file():
            evidence=json.loads(old.read_text())
            evidence["vendor_functions_tested"]=[
                "FUN_00110ccc: read-only native U-Boot command lookup",
                "FUN_00110d2c: native command registry wrapper",
                "FUN_00104f88: real MMC number to registered backend",
                "FUN_0011c3ac: real backend list ID selector",
                "NONE OF THE RETURNED COMMAND HANDLERS WERE EXECUTED"]
            evidence["registered_command_handlers_executed"]=False
            evidence["physical_Uboot_MMC_index_proven"]=False
            record(new,evidence)
            old.unlink()
        if logs.is_file():logs.rename(out/"v24-real-owner-command-console.log")
        response.update(experiment)
        response["new_cases"]=len(cases())
        response["prior_VM_cases_repeated"]=0
        response["command_handlers_executed"]=False
    except Exception as exc:
        response["error"]=str(exc)
        (out/"v24-guest-error.log").write_text(traceback.format_exc()[-4500:])
    return response
def reconcile(out,old,stages):
    previous=json.loads(old["V23-EXACT-FINDINGS-AND-85-QUESTIONS.json"])
    path=out/"V24-ORIGINAL-UBOOT-COMMAND-DESCRIPTORS.json"
    cmd=json.loads(path.read_text()) if path.is_file() else {}
    vm_path=out/"V24-REAL-OWNER-COMMAND-AND-MMC-ARM.json"
    vm=json.loads(vm_path.read_text()) if vm_path.is_file() else {}
    p3_path=out/"deep-p3/V24-p3-SUMMARY.json"
    p3=json.loads(p3_path.read_text()) if p3_path.is_file() else {}
    observed={c["test"]:c for c in vm.get("cases",[]) if "test" in c}
    sections={}
    for family,start,end in (("command_lookup",0,32),("fake_MMC_index",0,16)):
        prefix="v24_cmd_" if family=="command_lookup" else "v24_mmc_"
        rows=[observed.get(prefix+str(i)) for i in range(start,end)]
        sections[family]={
          "requested":len(rows),
          "passed":sum(bool(x and x.get("pass") is True) for x in rows),
          "missing_or_failed":[prefix+str(i) for i,x in enumerate(rows)
                               if not x or x.get("pass") is not True]}
    related={
      "BOOT_SELECTION-01":("Source-level registered MMC and boot commands, no environment writes executed",),
      "BOOT_SELECTION-03":("U-Boot MMC index lookup and original physical DTB aliases are distinct",),
      "BOOT_SELECTION-04":("Actual registered bootz/source/load handlers, if present, not executed",),
      "REMOVABLE_DIRECTBOOT-01":("Original U-Boot boot command registry and handler implementations",),
      "REMOVABLE_DIRECTBOOT-03":("Distinct SPL selection versus U-Boot command registry",),
      "REMOVABLE_DIRECTBOOT-05":("Actual MMC index lookup with synthetic backends, NOT SD/eMMC physical mapping",),
      "REMOVABLE_DIRECTBOOT-06":("Original U-Boot registered USB commands separated from DFU updater code",),
      "REMOVABLE_DIRECTBOOT-08":("Original source/run/setenv/saveenv registrations; no command execution",),
      "REMOVABLE_DIRECTBOOT-09":("Kernel/FDT boot command roots only; precise RAM reservations still open",),
      "ROOTFS_COMPATIBILITY-05":("Registered bootz handler proves reachability, not valid kernel boot",),
    }
    result=[]
    for oldrow in previous["85_research_questions"]:
        item={
           "id":oldrow["id"],"question":oldrow["question"],
           "prior_v23_evidence":oldrow.get("new_V23_evidence",[]),
           "v24_new_evidence":[],"fully_resolved":False}
        if item["id"] in related:
            if stages.get("independent_command_decoder",{}).get("success"):
                item["v24_new_evidence"].append({
                    "file":"V24-ORIGINAL-UBOOT-COMMAND-DESCRIPTORS.json",
                    "scope":related[item["id"]][0],
                    "proof":"SHA_VERIFIED_RAW_P3_STATIC_ONLY"})
            if stages.get("new_command_p3_ghidra",{}).get("success"):
                item["v24_new_evidence"].append({
                    "file":"deep-p3/V24-p3-PATH-WITNESSES.jsonl",
                    "scope":related[item["id"]][0],
                    "proof":"DIRECTED_GHIDRA_SOURCE_ONLY"})
            if sections["fake_MMC_index"]["passed"]>0 and item["id"] in (
                "REMOVABLE_DIRECTBOOT-05","BOOT_SELECTION-03"):
                item["v24_new_evidence"].append({
                    "file":"V24-REAL-OWNER-COMMAND-AND-MMC-ARM.json",
                    "scope":"Actual vendor MMC selector on synthetic list; no physical device numbering",
                    "proof":"ISOLATED_ARM_MOCKED_STORAGE"})
        result.append(item)
    outreport={
       "schema":"mpfree-v24-uninflated-native-commands-and-SD-selection",
       "owner_P1_SHA256":SHA_P1,"owner_P3_SHA256":SHA_P3,
       "native_Uboot_commands":cmd.get("safe_named_commands",{}),
       "source_registry_valid":stages.get("independent_command_decoder",{}).get("success",False),
       "ghidra_registry_valid":p3.get("original_command_registry",{}).get("layout_verified",False),
       "guest_test_families":sections,
       "original_questions":result,
       "questions_with_new_evidence":sum(bool(row["v24_new_evidence"]) for row in result),
       "questions_closed_by_keyword_matching":0,
       "known_distinction":{
          "DFU_mmc_updater_NOT_bootloader_SD_autoboot":True,
          "Linux_mmc0_eMMC_mmc1_SD_aliases_proven_from_V23_DTB":True,
          "Uboot_MMC_numeric_indices_equal_Linux_aliases":False},
       "physical_MMC_access":False,
       "bootloader_env_saved":False,
       "executable_registered_commands_actually_dispatched":False,
       "open_blockers":[
          "Identify real bootloader default boot_targets and actual MMC numbering from board init",
          "Demonstrate safe temporary SD kernel/FDT load without persistent writes",
          "Determine real RAM/relocation reservations before kernel handoff",
          "Verify real RK3288 MaskROM recovery path and restore independence"],
    }
    record(out/"V24-EXACT-FINDINGS-AND-85-QUESTIONS.json",outreport)
    return outreport
def package(out):
    suffixes={".json",".jsonl",".log",".md",".txt",".csv",".tsv"}
    records=[];total=0
    for p in sorted(out.rglob("*")):
        if p.is_symlink():raise RuntimeError("Output symlink prohibited")
        if p.is_dir():continue
        if (not p.is_file() or not stat.S_ISREG(p.lstat().st_mode)
            or p.suffix not in suffixes):
            raise RuntimeError("Unexpected output "+str(p))
        n=p.stat().st_size;total+=n
        if n>192*1024*1024 or total>512*1024*1024:
            raise ValueError("Private research exceeds archive limits")
        records.append({"name":p.relative_to(out).as_posix(),
                        "bytes":n,"sha256":digest(p)})
    if not records:raise RuntimeError("V24 produced no research")
    file=out.with_suffix(".zip")
    with zipfile.ZipFile(file,"x",compression=zipfile.ZIP_DEFLATED,
                         compresslevel=6) as z:
        z.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v24","private":True,"files":records},indent=2))
        for entry in records:z.write(out/entry["name"],entry["name"])
    with zipfile.ZipFile(file) as z:
        if z.testzip():raise ValueError("Final V24 ZIP CRC failure")
        for entry in records:
            content=z.read(entry["name"])
            if (len(content)!=entry["bytes"] or
                hashlib.sha256(content).hexdigest()!=entry["sha256"]):
                raise ValueError("Final V24 ZIP SHA failure: "+entry["name"])
    file.with_suffix(".zip.sha256").write_text(digest(file)+"  "+file.name+"\n")
    return file,len(records)
def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prior-v23",type=Path,
                    help="Owner-private verified V23 archive, newest autodetected")
    ap.add_argument("--qemu-boot-timeout",type=int,default=240)
    ap.add_argument("--case-timeout",type=int,default=20)
    ap.add_argument("--export-timeout",type=int,default=1200)
    ap.add_argument("--skip-vm",action="store_true")
    args=ap.parse_args()
    os.umask(0o077);WORK.mkdir(parents=True,exist_ok=True)
    if args.prior_v23:priorfile=regular(args.prior_v23)
    else:
        found=sorted(WORK.glob("bootchain-wide-*-v23-*.zip"),
                     key=lambda p:p.stat().st_mtime,reverse=True)
        if not found:ap.error("SHA-indexed private V23 ZIP is required")
        priorfile=regular(found[0])
    old=prior(priorfile)
    output=WORK/("bootchain-wide-"+datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
                 +"-v24-"+str(os.getpid()))
    output.mkdir(mode=0o700)
    start=time.monotonic();stages={}
    print("MPFree V24: new native command registry, genuine read-only command lookup and fake MMC index",flush=True)
    print("Original SHA-verified V23 ZIP:",priorfile,flush=True)
    try:
        with tempfile.TemporaryDirectory(dir=WORK,prefix="mpfree-v24-p3-private-") as private:
            temp=Path(private)
            with ThreadPoolExecutor(max_workers=4) as pool:
                jobs={
                  "new_SLP_media_p1_ghidra":pool.submit(timed,ghidra,"p1",output,temp),
                  "new_command_p3_ghidra":pool.submit(timed,ghidra,"p3",output,temp)
                }
                jobs["independent_command_decoder"]=pool.submit(
                    timed,independent_decoder,output,temp,jobs["new_command_p3_ghidra"],args)
                if not args.skip_vm:
                    jobs["real_vendor_command_guest"]=pool.submit(
                        timed,guest,output,temp,jobs["new_command_p3_ghidra"],args)
                for future in as_completed(jobs.values()):
                    name=next(k for k,v in jobs.items() if v is future)
                    stages[name]=future.result()
                    print("V24",name,"PASS" if stages[name].get("success") else "FAILED",
                          stages[name].get("elapsed_seconds"),"sec",
                          stages[name].get("error",""),flush=True)
    except KeyboardInterrupt:
        stages["interrupted"]={"success":False,"error":"Owner interrupted"}
    except Exception:
        stages["unhandled"]={"success":False,
                             "traceback":traceback.format_exc()[-4500:]}
    finally:
        try:findings=reconcile(output,old,stages)
        except Exception:
            findings={}
            (output/"V24-ANALYSIS-ERROR.log").write_text(traceback.format_exc()[-5000:])
        record(output/"V24-STAGE-MANIFEST.json",{
            "schema":"mpfree-v24-command-lookup-and-mmc-ID-owner-research",
            "branch":BRANCH,
            "timestamp":datetime.now(timezone.utc).isoformat(),
            "total_wall_seconds":round(time.monotonic()-start,2),
            "prior_V23_SHA256":digest(priorfile),
            "P1_SHA256":SHA_P1,"P3_SHA256":SHA_P3,
            "stages":stages,"new_ARM_cases_planned":len(cases()),
            "new_ARM_cases_observed":stages.get("real_vendor_command_guest",{}).get("tests_observed",0),
            "question_evidence_added":findings.get("questions_with_new_evidence",0),
            "real_command_handler_executed":False,
            "physical_MPC_USB_MMC_access":False,
            "proprietary_owner_binary_in_report_zip":False})
        readme=[
            "# MPFree V24: native U-Boot command registration and MMC device IDs","",
            "## Actual stage statuses"]
        for name,st in stages.items():
            readme.append("- %s: %s; %.2f seconds; %s"%(name,
                "PASS" if st.get("success") else "FAILED",
                st.get("elapsed_seconds",0.0),st.get("error","")))
        readme.extend(["",
            "## NEW not recycled evidence",
            "- SHA-pinned original 7-word command registry, exact allowlisted",
            "  names and handler function pointers; other names SHA-only.",
            "- Independent pure-Python source-byte decoder compared with",
            "  Ghidra's original ARM command-table interpretation.",
            "- Actual owner command lookup and wrapper, but HANDLERS NOT EXECUTED.",
            "- Actual MMC device ID lookup with fake registered device list",
            "  and intercepted initialization. NO physical eMMC/SD touched.",
            "- New P1 SPL boot priority and P3 native command/boot roots",
            "  recursively decompiled with High P-code path witnesses.",
            "- Previous DFU MMC functions explicitly separated from bootcmd.",
            "- Original 85 questions retained; no physical boot claim.",""])
        (output/"V24-READ-ME-FIRST.md").write_text("\n".join(readme))
        archive,n=package(output)
        print("\nONE PRIVATE ZIP TO UPLOAD:",archive,flush=True)
        print("SHA256:",digest(archive),"reports:",n,flush=True)
if __name__=="__main__":
    main()
