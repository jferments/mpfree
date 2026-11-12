#!/usr/bin/env python3
"""MPFree V17: REAL concurrent deep owner-private Ghidra + ARM QEMU research.

Runs P1/P3 exhaustive recursive fixed-point High P-code passes, fresh updater
ELF Ghidra imports, complete owner 3.10 boot/DTB comparison, and hundreds of
diverse actual vendor ARM function-path experiments in a network-isolated
generic ARM QEMU virt guest. No MPC, Fastboot hardware or physical eMMC used.

Everything that actually ran is measured. Failed stages remain failures.
Only derived textual reports enter one SHA-indexed PRIVATE ZIP.
"""
from __future__ import annotations
import argparse,hashlib,importlib.util,json,os,re,shutil,stat,subprocess
import tempfile,time,traceback,zipfile
from collections import Counter
from concurrent.futures import ThreadPoolExecutor,as_completed
from datetime import datetime,timezone
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
HERE=Path(__file__).resolve().parent
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-deep-runtime-v17-20261112"
SHA_P1="6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"
SHA_P3="b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
SHA_ELF={
    "az01-update":"57511e1d503f3ceaae3cb9bc40174e073b7ae410e65c986f1991731c77ae655b",
    "libaz01-prog":"af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623",
}
def module(path,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/path)
    val=importlib.util.module_from_spec(spec);spec.loader.exec_module(val)
    return val
V16=module("tools/mpc-one/run_v16.py","mpfree_v17_vm")
V10=module("tools/mpc-one/run_boot_updater_v10.py","mpfree_v17_verified_updater")
OFF=module("tools/mpc-one/offline_boot_assets_v17.py","mpfree_v17_offline")
def digest(p):
    h=hashlib.sha256()
    with Path(p).open("rb") as f:
        for block in iter(lambda:f.read(1024*1024),b""):h.update(block)
    return h.hexdigest()
def report(path,obj):
    path.parent.mkdir(exist_ok=True,parents=True)
    path.write_text(json.dumps(obj,indent=2,sort_keys=True)+"\n",encoding="utf8")
def regular_file(path):
    if path is None:raise ValueError("Input file not present")
    p=Path(path).expanduser()
    if (p.is_symlink() or not p.is_file() or
        not stat.S_ISREG(p.lstat().st_mode) or
        str(p).startswith(("/dev/","/proc/","/sys/"))):
        raise ValueError("Only immutable normal owner input files allowed: "+str(p))
    return p
def load_zip(path):
    p=regular_file(path);data={}
    with zipfile.ZipFile(p) as z:
        bad=z.testzip()
        if bad:raise ValueError("Owner archive CRC failed: "+bad)
        index=json.loads(z.read("ZIP_INDEX.json"))
        rows=index["files"]
        if len(rows)<5 or len(rows)>500:
            raise ValueError("Unexpected prior report member count")
        total=0;names=set()
        for item in rows:
            name=item["name"]
            if (name in names or name.startswith("/") or ".." in Path(name).parts
                or Path(name).suffix not in (".json",".txt",".log",".md",".jsonl")):
                raise ValueError("Unsafe prior report name: "+str(name))
            names.add(name);info=z.getinfo(name)
            if info.file_size!=item["bytes"] or info.file_size>64*1024*1024:
                raise ValueError("Untrusted prior report size: "+name)
            total+=info.file_size
            if total>256*1024*1024:raise ValueError("Oversized prior report archive")
            content=z.read(name)
            if hashlib.sha256(content).hexdigest()!=item["sha256"]:
                raise ValueError("Previous report SHA differs: "+name)
            data[name]=content
    return data
def tested_cases():
    # Distinct, reproducible boundary partitions and cross-products;
    # cases are NOT counted as distinct research questions.
    items=["v17_oem_shell_locked","v17_oem_shell_unlocked",
           "v17_oem_env_locked","v17_oem_env_unlocked",
           "v17_boot_completion"]
    critical=[0,1,2,3,7,15,16,31,32,63,64,255,256,257,4095,4096,4097,
              65535,65536,65537,0x7fffff,0x1000000,0x10000000,
              0x6fffffff,0x70000000,0x70000001,0x7000ffff,
              0x7fffffff,0x80000000,0xfffffffe,0xffffffff]
    # 128 independent deterministic values cross the threshold and wrap region.
    v=0x6f2b1947
    for _ in range(128):
        v=(1664525*v+1013904223)&0xffffffff
        critical.append(v)
    items.extend("v17_download_%08x"%i for i in sorted(set(critical)))
    packet=[0,1,2,3,7,15,16,31,32,63,64,127,128,255,256,
            511,512,1023,1024,2047,2048,4095,4096,4097,8191,8192]
    pairs={(a,b) for a in packet for b in packet}
    # Additional interior/near-boundary packets (400 deterministic pairs).
    for i in range(400):
        a=(i*6571+31)%8193
        b=(i*4093+64)%8193
        pairs.add((a,b))
    items.extend("v17_receive_%d_%d"%(a,b) for a,b in sorted(pairs))
    return items
def timed(fn,*args):
    t=time.monotonic()
    try:res=fn(*args)
    except Exception as e:res={"success":False,"error":str(e),
                               "trace":traceback.format_exc()[-2600:]}
    res["seconds"]=round(time.monotonic()-t,2)
    return res

def ghidra_saved(role,out,export=None):
    result={"success":False,"role":role,"project_readonly":True}
    log=out/("v17-ghidra-"+role+"-console.log")
    try:
        launcher=V16.get_ghidra()
        if not launcher:raise RuntimeError("PyGhidra headless launcher unavailable")
        project=V16.gproject(role)
        source="p1-rk3288-sram-stage.bin" if role=="p1" else "p3-uimage-payload.bin"
        folder=out/("deep-"+role)
        folder.mkdir(exist_ok=True)
        cmd=launcher+[str(WORK/"ghidra/projects"),project,"-process",source,
                      "-readOnly","-noanalysis",
                      "-scriptPath",str(ROOT/"tools/ghidra"),
                      "-postScript","MPFreeV17FixedPointDive.py",role,str(folder)]
        if export is not None:cmd.append(str(export))
        cmd+=["-log",str(folder/"engine.log")]
        with log.open("w") as f:
            process=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,
                                   timeout=3600,check=False)
        if process.returncode:
            raise RuntimeError("Ghidra "+role+" exited "+str(process.returncode))
        dest=folder/("V17-"+role+"-SUMMARY.json")
        j=json.loads(dest.read_text())
        expected=SHA_P1 if role=="p1" else SHA_P3
        if (j.get("schema")!="mpfree-v17-full-fixedpoint-high-pcode-v1" or
            j.get("source_sha256")!=expected or not j.get("verified_owner")):
            raise RuntimeError("Ghidra output missing verified owner SHA/source schema")
        result.update({"success":True,"functions_attempted":j["functions_attempted"],
                       "stats":j["statistics"],"edges":j["direct_unique_edges"],
                       "unknown_calls":len(j["unresolved_instruction_calls"]),
                       "callback_tables":len(j["candidate_function_pointer_tables"]),
                       "report":str(dest.relative_to(out))})
    except Exception as e:
        result["error"]=str(e)
        with log.open("a") as f:f.write("\n"+traceback.format_exc()[-2600:])
    return result

def updater_sources(rootfs,backup,temp):
    blobs=V10.V9.verify_source(regular_file(rootfs),regular_file(backup))
    files={}
    for name,content in blobs.items():
        if name not in SHA_ELF:continue
        if hashlib.sha256(content).hexdigest()!=SHA_ELF[name]:
            raise ValueError("Verified owner updater ELF SHA changed: "+name)
        filepath=temp/(name+".elf")
        with filepath.open("xb") as f:f.write(content)
        os.chmod(filepath,0o400)
        files[name]=filepath
    if set(files)!=set(SHA_ELF):
        raise RuntimeError("Owner uploader ELF source missing from verified filesystem")
    return files

def ghidra_updater(name,path,temp,out):
    result={"success":False,"role":name,"ELF_never_executed":True}
    logfile=out/("v17-updater-"+name+"-console.log")
    try:
        if digest(path)!=SHA_ELF[name]:
            raise RuntimeError("Updater source changed after verification")
        launcher=V16.get_ghidra()
        if not launcher:raise RuntimeError("No Ghidra launcher")
        dirname=out/("updater-"+name)
        dirname.mkdir(exist_ok=True)
        project=temp/("ghidra-import-"+name);project.mkdir(exist_ok=True)
        cmd=launcher+[str(project),"MPFreeV17_"+name,"-import",str(path),
                      "-scriptPath",str(ROOT/"tools/ghidra"),
                      "-postScript","MPFreeV17UpdaterDive.py",name,
                      str(dirname),SHA_ELF[name],
                      "-log",str(dirname/"engine.log")]
        with logfile.open("w") as f:
            p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,
                             timeout=3600,check=False)
        if p.returncode:raise RuntimeError("Ghidra import returned "+str(p.returncode))
        target=dirname/("V17-"+name+"-SUMMARY.json")
        summary=json.loads(target.read_text())
        if (summary.get("schema")!="mpfree-v17-updater-elf-recursive-v1" or
            summary.get("source_sha256_preverified_by_owner_host")!=SHA_ELF[name]):
            raise RuntimeError("Wrong updater Ghidra report provenance")
        result.update({"success":True,"functions":summary["functions_recognized"],
                       "pcode":summary["statistics"]["pcode_ops_total"],
                       "decompiled":summary["statistics"]["functions_decompiled"],
                       "unknown_calls":len(summary["unresolved_calls"]),
                       "callback_tables":len(summary["candidate_callback_tables"]),
                       "report":str(target.relative_to(out))})
    except Exception as e:
        result["error"]=str(e)
        with logfile.open("a") as f:f.write("\n"+traceback.format_exc()[-2600:])
    return result

def offline_boot(rootfs,out):
    result={"success":False}
    try:
        evidence=OFF.audit(rootfs)
        dst=out/"v17-owner-full-boot-and-dtb.json"
        report(dst,evidence)
        result.update({"success":True,"acva_nodes":evidence["acva"]["node_count"],
                       "acva_c_nodes":evidence["acva_c"]["node_count"],
                       "complete_dtb_differences":len(evidence["dtb_complete_differences"]),
                       "boot_script_commands":len(evidence["script"]["script_commands"]),
                       "header_crc_valid":evidence["script"]["header_crc_valid"],
                       "payload_crc_valid":evidence["script"]["payload_crc_valid"]})
    except Exception as e:
        result["error"]=str(e)
        (out/"v17-offline-boot-error.log").write_text(traceback.format_exc()[-2600:])
    return result

def real_vendor_guest(out,raw,temp,ghidra_future,args):
    result={"success":False,"actual_vendor_arm":True,"physical_devices":False}
    deadline=time.monotonic()+args.export_timeout
    ready=raw.with_name(raw.name+".ready")
    while time.monotonic()<deadline:
        if ready.is_file() and raw.is_file():break
        if ghidra_future.done():
            gh=ghidra_future.result()
            if not gh.get("success") and not raw.is_file():
                return {"success":False,"error":"P3 Ghidra failed before exporting verified image"}
        time.sleep(0.25) # Wait for evidence readiness; no artificial research delay.
    else:
        return {"success":False,"error":"Timed out waiting for verified P3 export"}
    try:
        if raw.stat().st_size!=363649 or digest(raw)!=SHA_P3:
            raise RuntimeError("Private source image failed SHA-256 before guest")
        if ready.read_text().strip()!=SHA_P3:
            raise RuntimeError("Ghidra SHA-ready marker invalid")
        V16.C_SOURCE=HERE/"vendor_probe_v17.c"
        V16.CASES=tested_cases()
        outcome=V16.qemu_real_code(out,raw,temp,{"success":True},args)
        # V16 helper names outputs as V16; migrate to V17 private stage records.
        source=out/"v16-actual-vendor-QEMU-results.json"
        if source.is_file():
            target=out/"v17-actual-vendor-QEMU-results.json"
            source.rename(target)
        console=out/"v16-qemu-vendor-function-console.log"
        if console.is_file():
            console.rename(out/"v17-qemu-vendor-function-console.log")
        outcome["cases_requested"]=len(tested_cases())
        outcome["actual_new_v17_cases"]=True
        result.update(outcome)
    except Exception as e:
        result.update({"success":False,"error":repr(e),"trace":traceback.format_exc()[-2200:]})
    return result

def evidence_ledger(out,prior,stages):
    """Per-question evidence/remaining blockers. No status based on keyword hits."""
    priorzip=load_zip(prior)
    key="V16-85-QUESTION-EVIDENCE.json"
    if key not in priorzip:raise RuntimeError("Prior verified V16 question baseline absent")
    origin=json.loads(priorzip[key])
    questions=origin["questions"]
    if len(questions)!=85:raise RuntimeError("Expected exactly 85 prior questions")
    guestpath=out/"v17-actual-vendor-QEMU-results.json"
    observations=json.loads(guestpath.read_text()) if guestpath.is_file() else {}
    cases=observations.get("cases",[])
    good=[x for x in cases if x.get("pass") and x.get("real_vendor_arm")]
    tested={x["test"] for x in good}
    def group(prefix):
        rows=[x for x in cases if x.get("test","").startswith(prefix)]
        return {"attempted":len(rows),"passed":sum(x.get("pass") is True for x in rows),
                "failed":sum(x.get("pass") is not True for x in rows)}
    tested_groups={
        "oem_authorized_storage_command":group("v17_oem_"),
        "bootm_callback_reset_sequence":group("v17_boot_completion"),
        "download_size_and_hex_wrap":group("v17_download_"),
        "usb_receive_and_canary_matrix":group("v17_receive_"),
    }
    domain_to_reports={
        "FASTBOOT":["deep-p3/V17-p3-SUMMARY.json","v17-actual-vendor-QEMU-results.json"],
        "FASTBOOT_RUNTIME":["deep-p3/V17-p3-SUMMARY.json","v17-actual-vendor-QEMU-results.json"],
        "BOOT_SELECTION":["deep-p1/V17-p1-SUMMARY.json","deep-p3/V17-p3-SUMMARY.json"],
        "REMOVABLE_DIRECTBOOT":["deep-p1/V17-p1-SUMMARY.json","v17-owner-full-boot-and-dtb.json"],
        "RECOVERY":["deep-p1/V17-p1-SUMMARY.json","deep-p3/V17-p3-SUMMARY.json"],
        "RECOVERY_SECURITY":["deep-p1/V17-p1-SUMMARY.json","updater-libaz01-prog/V17-libaz01-prog-SUMMARY.json"],
        "ROOTFS_COMPATIBILITY":["v17-owner-full-boot-and-dtb.json","deep-p3/V17-p3-SUMMARY.json"],
        "VM_VALIDATION":["v17-actual-vendor-QEMU-results.json"],
    }
    records=[]
    for item in questions:
        key=item["id"];prefix=key.rsplit("-",1)[0]
        sources=domain_to_reports.get(prefix,["deep-p3/V17-p3-SUMMARY.json"])
        valid=[s for s in sources if (out/s).is_file()]
        is_hw=bool("HARDWARE" in item.get("status","") or
                   "HARDWARE" in item.get("classification",""))
        row={
            "id":key,"question":item["question"],
            "previous":item.get("status",item.get("classification","UNRESOLVED")),
            "new_probe_reports":valid,
            "new_probe_executed":bool(valid),
            "new_scope":"STATIC_GHIDRA_AND_ISOLATED_QEMU_NOT_HARDWARE",
            "hardware_only_unresolved":is_hw,
            "closed_by_V17":False,
            "new_observation_group":None,
            "new_factual_status":"PROBED_NOT_YET_ADJUDICATED" if valid else "NOT_PROBED",
        }
        text=item["question"].lower()
        if "oem" in text or "unlock" in text:
            row["new_observation_group"]="oem_authorized_storage_command"
        elif "download" in text or "transfer" in text:
            row["new_observation_group"]="download_size_and_hex_wrap"
        elif "bootm" in text or "ram boot" in text:
            row["new_observation_group"]="bootm_callback_reset_sequence"
        elif "receive" in text or "usb" in text:
            row["new_observation_group"]="usb_receive_and_canary_matrix"
        if row["new_observation_group"] and tested_groups[row["new_observation_group"]]["passed"]:
            row["new_factual_status"]="NEW_FUNCTION_PATH_VM_EVIDENCE_REQUIRES_REVIEW"
        records.append(row)
    # Numeric accounting only of genuinely observed experiments and decompilation.
    summary={
        "questions":len(records),
        "questions_with_new_report":sum(bool(x["new_probe_reports"]) for x in records),
        "questions_closed_automatically":0,
        "dynamic_cases_attempted":len(cases),
        "dynamic_cases_passed":len(good),
        "dynamic_cases_failed":len(cases)-len(good),
        "distinct_dynamic_scenario_families":len(tested_groups),
        "dynamic_family_outcomes":tested_groups,
        "static_decompilation_by_stage":{
            key:stage.get("stats",{}).get("function_decompiled",stage.get("decompiled",0))
            for key,stage in stages.items() if key.startswith(("deep_","updater_"))},
        "high_pcode_operations_extracted":{
            key:stage.get("stats",{}).get("pcode_ops_total",stage.get("pcode",0))
            for key,stage in stages.items() if key.startswith(("deep_","updater_"))},
    }
    report(out/"V17-85-QUESTIONS-NEW-EVIDENCE.json",{
        "schema":"mpfree-v17-honest-new-data-85-ledger","summary":summary,
        "questions":records,
        "limitations":[
            "A question is not closed merely by its report being present.",
            "Generic ARM guest experiments cannot establish silicon MaskROM or physical MMC/USB.",
            "Every dynamic input is a new case, not a new research question.",
            "Human review is required for Ghidra High P-code/dataflow conclusions."
        ]})
    return summary

def package(folder):
    allowed={".json",".jsonl",".log",".txt",".md",".csv",".tsv"}
    members=[];total=0
    for p in sorted(folder.rglob("*")):
        if p.is_symlink():raise ValueError("Symlink in private report output")
        if not p.is_file():continue
        if (not stat.S_ISREG(p.lstat().st_mode) or p.suffix not in allowed
            or p.stat().st_size>192*1024*1024):
            raise ValueError("Refuse non-text or huge report: "+str(p))
        total+=p.stat().st_size
        if total>512*1024*1024:
            raise ValueError("Private reports exceed 512 MB safety cap")
        members.append({"name":p.relative_to(folder).as_posix(),
                        "bytes":p.stat().st_size,"sha256":digest(p)})
    if not members:raise ValueError("No generated research reports")
    dest=folder.with_suffix(".zip")
    with zipfile.ZipFile(dest,"x",compression=zipfile.ZIP_DEFLATED,compresslevel=6) as z:
        z.writestr("ZIP_INDEX.json",json.dumps({
            "schema":"mpfree-private-v17","private":True,"files":members},indent=2)+"\n")
        for row in members:z.write(folder/row["name"],row["name"])
    with zipfile.ZipFile(dest) as z:
        bad=z.testzip()
        if bad:raise ValueError("ZIP member failed CRC: "+bad)
        for row in members:
            if hashlib.sha256(z.read(row["name"])).hexdigest()!=row["sha256"]:
                raise ValueError("Written ZIP file SHA verification failed")
    dest.with_suffix(".zip.sha256").write_text(digest(dest)+"  "+dest.name+"\n")
    return dest,len(members)

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prior-v16",type=Path)
    ap.add_argument("--rootfs",type=Path,default=WORK/"akai-unpacked-20261110T024913Z-1hjja3_r/rootfs.img")
    ap.add_argument("--backup",type=Path,default=Path.home()/
                    "mpfree-backups/mpc-one-emmc-20261109T233604Z-datprr67/mmcblk0.img")
    ap.add_argument("--qemu-boot-timeout",type=int,default=300)
    ap.add_argument("--case-timeout",type=int,default=18)
    ap.add_argument("--export-timeout",type=int,default=1200)
    ap.add_argument("--skip-updater",action="store_true")
    ap.add_argument("--skip-qemu",action="store_true")
    ap.add_argument("--skip-offline",action="store_true")
    args=ap.parse_args()
    os.umask(0o077);WORK.mkdir(exist_ok=True,parents=True)
    paths=sorted(WORK.glob("bootchain-wide-*-v16-*.zip"),
                 key=lambda x:x.stat().st_mtime,reverse=True)
    prior=regular_file(args.prior_v16 or (paths[0] if paths else None))
    priorbytes=load_zip(prior)
    meta=json.loads(priorbytes["V16-STAGE-MANIFEST.json"])
    if not meta.get("stages",{}).get("ghidra_p3",{}).get("verified_owner_source"):
        ap.error("Previous V16 owner P3 provenance unverified")
    outfile=WORK/("bootchain-wide-"+datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
                  +"-v17-"+str(os.getpid()))
    outfile.mkdir(mode=0o700)
    stages={};started=time.monotonic()
    print("V17: exhaustive recursive Ghidra + actual vendor ARM-QEMU + new updater analysis",
          flush=True)
    print("Verified V16 reference:",prior,flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="mpfree-v17-private-",dir=str(WORK)) as td:
            temp=Path(td);raw=temp/"verified-owner-p3.raw"
            with ThreadPoolExecutor(max_workers=7) as pool:
                jobs={
                  "deep_p1":pool.submit(timed,ghidra_saved,"p1",outfile,None),
                  "deep_p3":pool.submit(timed,ghidra_saved,"p3",outfile,raw),
                }
                if not args.skip_offline:
                    jobs["boot_full_dtb"]=pool.submit(timed,offline_boot,args.rootfs,outfile)
                if not args.skip_qemu:
                    jobs["real_vendor_vm"]=pool.submit(
                        timed,real_vendor_guest,outfile,raw,temp,jobs["deep_p3"],args)
                if not args.skip_updater:
                    try:
                        elfs=updater_sources(args.rootfs,args.backup,temp)
                        for name,path in elfs.items():
                            jobs["updater_"+name]=pool.submit(
                                timed,ghidra_updater,name,path,temp,outfile)
                    except Exception as e:
                        stages["updater_source"]={"success":False,"error":str(e),
                                                  "trace":traceback.format_exc()[-2500:]}
                for job in as_completed(list(jobs.values())):
                    key=next(k for k,v in jobs.items() if v is job)
                    stages[key]=job.result()
                    print("V17",key,
                          "PASS" if stages[key].get("success") else "INCOMPLETE",
                          stages[key].get("seconds"),"seconds",
                          stages[key].get("error",""),flush=True)
    except KeyboardInterrupt:
        stages["interrupt"]={"success":False,"error":"Keyboard interrupt"}
    except Exception:
        stages["uncaught"]={"success":False,"trace":traceback.format_exc()[-4000:]}
    finally:
        try:
            summary=evidence_ledger(outfile,prior,stages)
        except Exception:
            summary={}
            (outfile/"V17-LEDGER-ERROR.log").write_text(traceback.format_exc())
        report(outfile/"V17-STAGE-MANIFEST.json",{
            "schema":"mpfree-v17-full-research-stages",
            "branch":BRANCH,"generated_utc":datetime.now(timezone.utc).isoformat(),
            "duration_seconds":round(time.monotonic()-started,2),
            "prior_v16_sha256":digest(prior),
            "stages":stages,"new_data":summary,
            "physical_MPC_contact":False,"USB_passthrough":False,
            "vendor_elf_executed":False,"vendor_raw_bytes_in_zip":False})
        text=[
          "# MPFree V17 — full recursive Ghidra and actual ARM guest experimentation","",
          "No artificially timed delays; these counts reflect actual Ghidra and VM work.",
          "This is not full RK3288/MaskROM emulation, and no owner device is contacted.","",
          "## New research stages",
        ]
        for key,r in stages.items():
            text.append("- %s: %s; %.2f s; %s"%(key,
                        "PASS" if r.get("success") else "INCOMPLETE",
                        r.get("seconds",0),r.get("error","")))
        text+=["","## Genuine evidence counts",
          "- New actual vendor ARM cases attempted: %s"%summary.get("dynamic_cases_attempted",0),
          "- New actual vendor ARM cases passed: %s"%summary.get("dynamic_cases_passed",0),
          "- Questions marked closed automatically: 0 (requires evidence adjudication)",
          "- Ghidra recursive closure and all-function High P-code: see deep-p1/deep-p3.",
          "- Updater ELF import/deep call graph: see updater-*.",
          "- Complete ACVA/ACVA-C DTB changes: see v17-owner-full-boot-and-dtb.json.",
          "- Signed owner proprietary binaries are NEVER included in this ZIP.",
          "",
          "## New dynamic scope",
          "OEM console and persistence branches, real boot-done / bootm/reset",
          "handoff, download numeric boundaries, USB receive clipping and canaries.",
          "Storage, command execution and bootm/reset operations are guest-instrumented",
          "return stubs. Their real implementations are NOT executed in these cases.",
          "No physical boot-media selection, BootROM, eFuse or RK3288 DRAM proof.",
          ""]
        (outfile/"V17-READ-ME-FIRST.md").write_text("\n".join(text)+"\n")
        archive,num=package(outfile)
        print("\nONE PRIVATE ZIP TO UPLOAD:",archive,flush=True)
        print("SHA256:",digest(archive),"Reports:",num,flush=True)
        print("No physical MPC, Fastboot, USB or eMMC device accessed.",flush=True)
if __name__=="__main__":main()
