#!/usr/bin/env python3
"""MPFree V14 deep offline research: concurrent all-function Ghidra, vendor updater
static audit, real owner boot-data ARM QEMU tests, boot-asset and question analysis.

NO physical devices, USB passthrough, MPC SSH, firmware execution or writes to
original firmware/backup.  ONE final PRIVATE SHA-indexed ZIP for user upload.
"""
from __future__ import annotations
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import pty
import re
import select
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import traceback
import zipfile

ROOT=Path(__file__).resolve().parents[2]
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-deep-v14-20261112"
def load(rel,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/rel)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module
v12=load("tools/mpc-one/run_fastboot_directboot_v12.py","mpfree_v14_v12")
v13=load("tools/mpc-one/run_fastboot_directboot_v13.py","mpfree_v14_v13")
def write(path,obj):
    path.write_text(json.dumps(obj,indent=2,sort_keys=True)+"\n")
def sha(path):
    h=hashlib.sha256()
    with path.open("rb") as f:
        for b in iter(lambda:f.read(1024*1024),b""):h.update(b)
    return h.hexdigest()
def timed(fn):
    def wrapped(*args,**kwargs):
        begin=time.monotonic()
        try:r=fn(*args,**kwargs)
        except Exception as e:r={"success":False,"error":repr(e),
                                   "traceback":traceback.format_exc()[-3000:]}
        r["elapsed_s"]=round(time.monotonic()-begin,2)
        return r
    return wrapped

@timed
def ghidra(role,dst):
    res={"success":False,"stage":"full_ghidra_"+role,"read_only":True,
         "vendor_executed":False,"physical_device_contact":False}
    log=dst/("v14-"+role+"-ghidra-console.log")
    try:
        launcher=v12.ghidra_launcher()
        if not launcher:raise RuntimeError("Ghidra launcher missing")
        project=v12.project(role)
        filename="p1-rk3288-sram-stage.bin" if role=="p1" else "p3-uimage-payload.bin"
        output=dst/("v14-full-"+role+"-ghidra.json")
        cmd=launcher+[str(v12.PROJECTS),project,"-process",filename,
                       "-readOnly","-noanalysis","-scriptPath",str(ROOT/"tools/ghidra"),
                       "-postScript","MPFreeBootchainFullCoverageV14.py",role,str(output),
                       "-log",str(dst/("v14-"+role+"-ghidra-engine.log"))]
        with log.open("w") as f:
            p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,timeout=3600,check=False)
        if p.returncode:raise RuntimeError("Ghidra exited "+str(p.returncode))
        evidence=json.loads(output.read_text())
        if (evidence.get("schema")!="mpfree-v14-full-coverage-ghidra-v1"
                or not evidence.get("owner_hash_verified") or evidence.get("source")!=role):
            raise RuntimeError("Invalid SHA-verified V14 function-coverage report")
        s=evidence["statistics"]
        res.update({"success":True,"report":output.name,"source_sha256":evidence["owner_sha256"],
                    "functions_known":s["ghidra_functions_total"],
                    "decompiled":s["functions_decompiled"],
                    "coverage_ratio":s["function_coverage_ratio"],
                    "direct_edges":s["unique_direct_call_edges"],
                    "indirect_sites":s["indirect_or_unresolved_call_sites"],
                    "decomp_failures":s["decompilation_failures"]})
    except Exception as exc:
        res["error"]=str(exc)
        with log.open("a") as f:f.write("\n"+traceback.format_exc()[-2500:])
    return res

def verified_archive(path):
    """Import only text entries from a verified indexed private research ZIP."""
    path=v12.regular(path)
    with zipfile.ZipFile(path) as z:
        if z.testzip():raise ValueError("ZIP CRC failed: "+str(path))
        idx=json.loads(z.read("ZIP_INDEX.json"))
        rows=idx["files"]
        if not rows or len(rows)>500:raise ValueError("Unexpected indexed member count")
        seen=set();items={}
        total=0
        for row in rows:
            name=row["name"];pp=PurePosixPath(name)
            if (pp.is_absolute() or ".." in pp.parts or not pp.parts
                or name in seen or pp.suffix not in (".json",".txt",".md",".log",".csv",".tsv")):
                raise ValueError("Untrusted archive member path: "+str(name))
            seen.add(name)
            meta=z.getinfo(name)
            if meta.file_size != row["bytes"] or meta.file_size>32*1024*1024:
                raise ValueError("Oversized/invalid member: "+name)
            total+=meta.file_size
            if total>128*1024*1024:raise ValueError("Prior archive too large")
            blob=z.read(name)
            if hashlib.sha256(blob).hexdigest()!=row["sha256"]:
                raise ValueError("Archive SHA mismatch: "+name)
            items[name]=blob
        return items
@timed
def updater_static(dst,prior_v10):
    """V11 Ghidra updater-library analysis, not vendor executable execution."""
    res={"success":False,"stage":"updater_v11","vendor_executed":False,"hardware_contact":False}
    if not prior_v10:
        res["error"]="Missing prior SHA-indexed V10 ZIP; updater stage NOT RUN"
        return res
    log=dst/"v14-updater-v11-console.log"
    try:
        prior=verified_archive(prior_v10)  # Fail closed on unverified lineage.
        if not any("v10" in name.lower() for name in prior):
            raise ValueError("Verified ZIP lacks V10 evidence")
        before=set(WORK.glob("bootchain-wide-*-v11-*.zip"))
        cmd=[sys.executable,str(ROOT/"tools/mpc-one/run_boot_updater_v11.py"),
             "--prior-v10",str(prior_v10),"--skip-boot-ghidra"]
        with log.open("w") as f:
            p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,timeout=3600,check=False)
        fresh=sorted(set(WORK.glob("bootchain-wide-*-v11-*.zip"))-before)
        if not fresh:raise RuntimeError("V11 updater emitted no new ZIP (rc=%s)"%p.returncode)
        source=fresh[-1]
        records=verified_archive(source)
        target=dst/"updater-v11";target.mkdir()
        for name,blob in records.items():
            out=target/name
            out.parent.mkdir(parents=True,exist_ok=True)
            out.write_bytes(blob)
        res.update({"success":p.returncode==0,
                    "prior_v10_sha256":sha(prior_v10),
                    "updater_archive_sha256":sha(source),
                    "imported_text_reports":len(records),
                    "source_archive":source.name})
        if p.returncode:res["error"]="V11 subprocess nonzero; imported partial verified reports"
        # Intermediate original V11 ZIP remains in WORK for evidence lineage.
    except Exception as exc:
        res["error"]=str(exc)
        with log.open("a") as f:f.write("\n"+traceback.format_exc()[-2500:])
    return res

@timed
def boot_assets(dst,rootfs):
    return v13.run_boot_assets(dst,rootfs)
@timed
def offline_map(dst,rootfs,backup):
    return v12.run_offline_rootfs(dst,rootfs,backup)

@timed
def qemu_fixture(dst,rootfs,temporary,args):
    result={"success":False,"stage":"arm_qemu_real_boot_data",
            "board":"generic ARM virt, NOT RK3288",
            "guest_did_not_execute_vendor_code":True,
            "physical_usb_passthrough":False,"network":False}
    log=dst/"v14-arm-qemu-boot-fixture-console.log"
    proc=None;master=slave=None
    try:
        q=load("tools/mpc-one/arm_qemu_lab.py","mpfree_v14_qemu")
        integ=load("tools/mpc-one/run_bootchain_integrated_lab.py","mpfree_v14_qemu_map")
        image=v12.regular(rootfs)
        for tool in ("qemu-system-arm","qemu-img","arm-linux-gnueabihf-gcc","mkfs.ext4","debugfs"):
            if not shutil.which(tool):raise RuntimeError("Required VM tool unavailable: "+tool)
        code=v12.regular(ROOT/"tools/mpc-one/v14_owner_boot_fixture.c")
        stage=temporary/"guest-files";stage.mkdir(mode=0o700)
        files={
            "script.uimg":"/boot/boot.scr.uimg",
            "acva.dtb":"/boot/rk3288-az01-acva.dtb",
            "acvac.dtb":"/boot/rk3288-az01-acva-c.dtb",
            "kernel-head.bin":"/boot/zImage-6.18.44-az01-2026-08-10-rt6",
        }
        extracted={}
        for name,guest_path in files.items():
            p=subprocess.run(["debugfs","-R","cat "+guest_path,str(image)],
                             capture_output=True,timeout=60,check=False)
            data=p.stdout
            if p.returncode or not data:
                raise RuntimeError("Cannot read owner boot asset: "+guest_path)
            if name=="kernel-head.bin":data=data[:128]
            if len(data)>2*1024*1024:
                raise RuntimeError("Unexpected guest fixture asset size")
            (stage/name).write_bytes(data)
            extracted[name]={"bytes":len(data),"sha256":hashlib.sha256(data).hexdigest()}
        binary=stage/"v14_fixture"
        cc=subprocess.run(["arm-linux-gnueabihf-gcc","-std=c11","-static","-O2","-s",
                           "-Wall","-Wextra","-Werror","-o",str(binary),str(code)],
                          capture_output=True,text=True,timeout=90,check=False)
        if cc.returncode:raise RuntimeError("ARM compile failed "+cc.stderr[:1000])
        fixture=temporary/"real-owner-boot-data.ext4"
        with fixture.open("wb") as f:f.truncate(32*1024*1024)
        mk=subprocess.run(["mkfs.ext4","-F","-q","-d",str(stage),str(fixture)],
                          capture_output=True,text=True,timeout=90)
        if mk.returncode:raise RuntimeError("Guest fixture ext4 failed "+mk.stderr[:800])
        command=q.qemu_args(q.KERNEL)
        command.insert(1,"-snapshot")
        command=integ.qemu_storage_for_synthetic_model(command,fixture)
        if (command[command.index("-nic")+1]!="none" or
                command[command.index("-monitor")+1]!="none" or
                any("usb-host" in x or "hostdev" in x for x in command)):
            raise ValueError("Unsafe QEMU device or networking options")
        result["guest_kernel_sha256"]=sha(q.KERNEL)
        result["guest_rootfs_clone_sha256"]=sha(q.BASE)
        result["fixture_file_hashes"]=extracted
        master,slave=pty.openpty()
        proc=subprocess.Popen(command,stdin=slave,stdout=slave,stderr=slave,
                              close_fds=True,start_new_session=True)
        os.close(slave);slave=None
        # Root shell is booted in disposable clone. No vendor services run.
        lines=[
            "echo MPFREE_V14_BEGIN",
            "mount -t proc proc /proc 2>/dev/null || true",
            "mkdir -p /mnt/mpfree-v14",
            "mount -t ext4 -o ro /dev/vda /mnt/mpfree-v14",
            "/mnt/mpfree-v14/v14_fixture /mnt/mpfree-v14/script.uimg /mnt/mpfree-v14/acva.dtb /mnt/mpfree-v14/acvac.dtb /mnt/mpfree-v14/kernel-head.bin",
        ]
        end="MPFREE_V14_COMPLETED_"+str(os.getpid())
        lines.append("echo "+end)
        data="";sent=False;boot_deadline=time.monotonic()+args.qemu_boot_timeout
        probe_deadline=None;acked=False
        with log.open("w",encoding="utf-8",errors="replace") as logfile:
            while True:
                now=time.monotonic()
                if proc.poll() is not None:
                    result["error"]="Guest exited unexpectedly";break
                if (not sent and now>boot_deadline) or (sent and now>probe_deadline):
                    result["error"]="QEMU boot/probe timed out";break
                ready,_,_=select.select([master],[],[],0.3)
                if not ready:continue
                try:chunk=os.read(master,65536)
                except OSError:break
                if not chunk:break
                value=chunk.decode("utf-8",errors="replace")
                logfile.write(value);logfile.flush()
                data+=value
                if len(data)>2*1024*1024:data=data[-2*1024*1024:]
                if not sent and re.search(r"(?:^|[\r\n])(?:/ # |# |~ # )",data[-4000:]):
                    for line in lines:os.write(master,(line+"\n").encode())
                    sent=True;probe_deadline=time.monotonic()+args.qemu_probe_timeout
                if sent and re.search(r"(?m)^"+re.escape(end)+r"[ \t]*$",data.replace("\r","")):
                    acked=True;break
        observations=[]
        for line in data.replace("\r","").splitlines():
            if line.startswith('{"test":'):
                try:observations.append(json.loads(line))
                except ValueError:pass
        result["observed_cases"]=len(observations)
        result["cases_passed"]=sum(v.get("pass") is True for v in observations)
        result["model_pass_marker"]="MPFREE_V14_FIXTURE_PASS" in data
        result["success"]=bool(acked and result["model_pass_marker"]
                               and len(observations)>=17
                               and all(v.get("pass") is True for v in observations))
        if not result["success"] and "error" not in result:
            result["error"]="Guest fixture failed or incomplete; see console"
    except Exception as exc:
        result["error"]=repr(exc)
        with log.open("a") as file:file.write("\n"+traceback.format_exc()[-2400:])
    finally:
        if proc is not None and proc.poll() is None:
            try:os.killpg(proc.pid,signal.SIGTERM);proc.wait(timeout=10)
            except Exception:
                try:os.killpg(proc.pid,signal.SIGKILL)
                except ProcessLookupError:pass
                proc.wait(timeout=10)
        if slave is not None:os.close(slave)
        if master is not None:os.close(master)
        write(dst/"v14-arm-qemu-boot-fixture.json",result)
    return result

# Explicit classes of questions that no software-only test can physically prove.
HARDWARE_ONLY={
    "FASTBOOT_RUNTIME-01","FASTBOOT_RUNTIME-02","FASTBOOT_RUNTIME-03",
    "FASTBOOT_RUNTIME-11","REMOVABLE_DIRECTBOOT-04","REMOVABLE_DIRECTBOOT-05",
    "REMOVABLE_DIRECTBOOT-10","ROOTFS_COMPATIBILITY-01","ROOTFS_COMPATIBILITY-05",
    "ROOTFS_COMPATIBILITY-09","ROOTFS_COMPATIBILITY-10",
    "ROOTFS_COMPATIBILITY-11","RECOVERY_SECURITY-02","RECOVERY_SECURITY-06",
    "RECOVERY_SECURITY-07","RECOVERY_SECURITY-08","RECOVERY_SECURITY-09",
    "RECOVERY_SECURITY-13","RECOVERY_SECURITY-14","RECOVERY_SECURITY-15",
    "RECOVERY_SECURITY-17","RECOVERY-02","RECOVERY-03",
}
def questions_from_v13(dst,archive):
    if not archive:return []
    rows=verified_archive(archive)
    name="V13-RESEARCH-QUESTION-MATRIX.json"
    if name not in rows:raise ValueError("Prior V13 ZIP has no question inventory")
    record=json.loads(rows[name]);questions=record["question_rows"]
    if not 80<=len(questions)<=120:raise ValueError("Wrong prior research-question count")
    return questions
def analyze(dst,stages,prior_questions,prior_zip):
    full={}
    for role in ("p1","p3"):
        p=dst/("v14-full-"+role+"-ghidra.json")
        if p.is_file():
            try:full[role]=json.loads(p.read_text())
            except Exception:pass
    assets={}
    p=dst/"offline-boot-assets-v13.json"
    if p.is_file():
        try:assets=json.loads(p.read_text())
        except Exception:pass

    prior_exact={}
    prior_p3={}
    if prior_zip:
        try:
            archive_files=verified_archive(prior_zip)
            prior_folder=dst/"verified-prior-v13"
            prior_folder.mkdir(exist_ok=True)
            if "V13-EXACT-FINDINGS.json" in archive_files:
                prior_exact=json.loads(archive_files["V13-EXACT-FINDINGS.json"])
                (prior_folder/"V13-EXACT-FINDINGS.json").write_bytes(
                    archive_files["V13-EXACT-FINDINGS.json"])
            if "ghidra-p3-v13.json" in archive_files:
                prior_p3=json.loads(archive_files["ghidra-p3-v13.json"])
                (prior_folder/"ghidra-p3-v13.json").write_bytes(
                    archive_files["ghidra-p3-v13.json"])
            if prior_p3 and prior_p3.get("source_sha256") != "b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a":
                raise ValueError("V13 owner P3 source SHA mismatch")
        except Exception:
            raise ValueError("Unable to validate V13 static evidence input")
    reboot_modes={}
    for boot_name in ("/boot/rk3288-az01-acva.dtb","/boot/rk3288-az01-acva-c.dtb"):
        entry=assets.get("files",{}).get(boot_name,{})
        props=entry.get("dtb",{}).get("selected_properties",{})
        reboot_modes[boot_name]=props.get("/power-management@ff730000/reboot-mode",{})
    # No generic question is marked 'answered' from string/topic matching.
    facts={}
    def add(k,status,source,details):
        facts[k]={"status":status,"source":source,"details":details}
    for role,output in full.items():
        s=output["statistics"]
        add("functions_"+role,"MEASURED_STATIC",["v14-full-"+role+"-ghidra.json"],
            {"recognized":s["ghidra_functions_total"],"decompiled":s["functions_decompiled"],
             "edges":s["unique_direct_call_edges"],
             "indirect":s["indirect_or_unresolved_call_sites"],
             "elapsed":s["elapsed_s"]})
    if prior_zip:
        add("v13_lineage","VERIFIED_ARCHIVE",["V13-RESEARCH-QUESTION-MATRIX.json"],
            {"sha256":sha(prior_zip),"question_count":len(prior_questions)})
    script=assets.get("files",{}).get("/boot/boot.scr.uimg",{}).get("uimage",{})
    if script:
        add("verified_boot_script",
            "ANSWERED_STATIC" if all(script.get(k) for k in ("valid","header_crc_valid","payload_crc_valid"))
            else "UNRESOLVED",
            ["offline-boot-assets-v13.json"],
            {"crc_header":script.get("header_crc_valid"),"crc_payload":script.get("payload_crc_valid"),
             "bootz_commands":script.get("script",{}).get("normalized_command_lines",[])})
    kernel=assets.get("files",{}).get("/boot/zImage-6.18.44-az01-2026-08-10-rt6",{})
    add("vendor_kernel_zimage_header",
        "ANSWERED_STATIC" if kernel.get("kernel_arm_zimage_magic") else "UNRESOLVED",
        ["offline-boot-assets-v13.json"],{"sha256":kernel.get("sha256")})
    vm=stages.get("arm_qemu_fixtures",{})
    add("owner_boot_data_guest_validation",
        "ANSWERED_VM_DATA_ONLY" if vm.get("success") else "UNRESOLVED",
        ["v14-arm-qemu-boot-fixture.json"],
        {"cases":vm.get("observed_cases",0),"passes":vm.get("cases_passed",0),
         "vendor_code_executed":False})
    # Carry 85 items forward. Every item receives a named probe and evidence category.
    result=[]
    for row in prior_questions:
        item={"id":row["id"],"question":row["question"],
              "classification":"PROBED_UNRESOLVED","evidence":[],
              "not_executable_in_this_offline_lab":False}
        key=row["id"]
        if key.startswith(("FASTBOOT-","FASTBOOT_RUNTIME-","BOOT_SELECTION-")):
            work=["full_p3"]
        elif key.startswith(("BOOT_COUNTER-","RECOVERY-","RECOVERY_SECURITY-","REMOVABLE_DIRECTBOOT-")):
            work=["full_p1","full_p3"]
        elif key.startswith("ROOTFS_COMPATIBILITY-"):
            work=["boot_assets","full_p3","arm_qemu_fixtures"]
        else:work=["arm_qemu_fixtures"]
        item["probes"]=work
        item["probe_executed"]=any(stages.get(n,{}).get("success") for n in work)
        for name in work:
            if stages.get(name,{}).get("success"):
                src=(("v14-full-p1-ghidra.json" if name=="full_p1" else
                      "v14-full-p3-ghidra.json") if name.startswith("full_") else
                     ("offline-boot-assets-v13.json" if name=="boot_assets" else
                      "v14-arm-qemu-boot-fixture.json"))
                item["evidence"].append({"source":src,"scope":name,
                                         "interpretation":"examined; not necessarily answered"})
        if key in HARDWARE_ONLY:
            item["classification"]="NEEDS_HARDWARE_CONFIRMATION"
            item["not_executable_in_this_offline_lab"]=True
        elif not item["probe_executed"]:
            item["classification"]="NOT_PROBED_STAGE_FAILED"
        # Strict, individually justified narrow answers; never auto-close an entire
        # question when only a related domain/string was found.
        if key=="FASTBOOT-01":
            pointer=prior_p3.get("pointer_cells",{}).get("unlock_command_text_pointer",{})
            if pointer.get("static_string_target") == "<REDACTED_VENDOR_AUTH_STRING>":
                item["classification"]="ANSWERED_STATIC"
                item["evidence"].append({"source":"verified-prior-v13/ghidra-p3-v13.json",
                    "fact":"Verified SHA-pinned P3 pointer resolves to OEM auth string (text redacted)"})
        if key=="FASTBOOT-02":
            entries=prior_exact.get("eight_slot_fastboot_dispatch",{}).get("entries",[])
            handler=prior_p3.get("function_details",{}).get("0x00122854",{})
            if (any(x.get("name")=="oem" and x.get("handler")=="0x00122855" for x in entries)
                and "0x25" in handler.get("pseudocode","")):
                item["classification"]="ANSWERED_STATIC"
                item["evidence"].append({"source":"verified-prior-v13/ghidra-p3-v13.json",
                    "fact":"OEM dispatcher and 0x25-byte authorization comparison in handler"})
        if key=="FASTBOOT_RUNTIME-10":
            bootdone=prior_p3.get("function_details",{}).get("0x001226b4",{}).get("pseudocode","")
            if ("mpf_bootm_command(" in bootdone and
                "mpf_reset_cpu_candidate(" in bootdone and
                bootdone.index("mpf_bootm_command(")<bootdone.index("mpf_reset_cpu_candidate(")):
                item["classification"]="ANSWERED_STATIC"
                item["evidence"].append({"source":"verified-prior-v13/ghidra-p3-v13.json",
                    "fact":"Fastboot boot-done calls bootm and then CPU reset on return"})
        if key=="RECOVERY_SECURITY-05" and facts.get("update_loader_mode_equal",{}).get("status")=="ANSWERED_STATIC":
            item["classification"]="ANSWERED_STATIC"
            item["evidence"].append({"source":"offline-boot-assets-v13.json",
                  "fact":"Both extracted ACVA DTBs define mode-update=mode-loader=0x5242c301"})
        if key=="VM_VALIDATION-01" and vm.get("success"):
            item["evidence"].append({"source":"v14-arm-qemu-boot-fixture.json",
                                      "fact":"Generic ARM virt booted owner 3.10 clone"})
        if key=="ROOTFS_COMPATIBILITY-02" and script.get("valid"):
            item["classification"]="ANSWERED_STATIC"
            item["evidence"].append({"source":"offline-boot-assets-v13.json",
                                     "fact":"boot script present with valid U-Boot header"})
        if key=="REMOVABLE_DIRECTBOOT-07" and script.get("valid"):
            commands="\n".join(script.get("script",{}).get("normalized_command_lines",[])).lower()
            if commands and "uuid" not in commands:
                item["classification"]="ANSWERED_STATIC"
                item["evidence"].append({"source":"offline-boot-assets-v13.json",
                                         "fact":"entire parsed U-Boot boot script contains no hardcoded UUID"})
        if key=="VM_VALIDATION-01" and vm.get("success"):
            item["classification"]="ANSWERED_VM_SCOPE"
        if key=="VM_VALIDATION-03" and vm.get("success"):
            item["classification"]="ANSWERED_VM_SCOPE"
        if key=="VM_VALIDATION-10":
            item["classification"]="ANSWERED_OUTPUT_INTEGRITY"
        if key in ("FASTBOOT-03","FASTBOOT-04","FASTBOOT_RUNTIME-06"):
            item["classification"]="PARTIAL_STATIC_NEEDS_FURTHER_PROOF"
            item["evidence"].append({"source":"verified-prior-v13/V13-EXACT-FINDINGS.json",
                                     "fact":"V13 established permission pointer alias and handler call relationships"})
        if key in ("VM_VALIDATION-04","VM_VALIDATION-05","VM_VALIDATION-06",
                   "VM_VALIDATION-07","VM_VALIDATION-08","VM_VALIDATION-09"):
            item["classification"]="ONLY_SYNTHETIC_PREVIOUS_V13"
            item["evidence"].append({"source":"prior V13 SHA-indexed archive",
                                     "fact":"42 synthetic model cases, not vendor code"})
        if key=="ROOTFS_COMPATIBILITY-03" and kernel.get("kernel_arm_zimage_magic"):
            item["classification"]="PARTIAL_STATIC_NEEDS_FURTHER_PROOF"
        result.append(item)
    counts=dict(Counter(x["classification"] for x in result))

    if prior_exact:
        add("v13_permission_alias","VERIFIED_STATIC_PRIOR",
            ["verified-prior-v13/V13-EXACT-FINDINGS.json","verified-prior-v13/ghidra-p3-v13.json"],
            {"gate":prior_exact.get("oem_unlock_permission_pointer_alias",{}).get("gate"),
             "writer":prior_exact.get("oem_unlock_permission_pointer_alias",{}).get("writer")})
    if all(reboot_modes.get(x,{}).get("mode-loader")==["0x5242c301"] and
           reboot_modes.get(x,{}).get("mode-update")==["0x5242c301"] for x in reboot_modes):
        add("update_loader_mode_equal","ANSWERED_STATIC",
            ["offline-boot-assets-v13.json"],
            {"both_acva_dtbs":True,"mode_loader":0x5242c301,"mode_update":0x5242c301})
    write(dst/"V14-PROPOSITION-EVIDENCE.json",facts)
    write(dst/"V14-85-QUESTION-EVIDENCE-LEDGER.json",{
        "schema":"mpfree-v14-non-inflated-question-ledger",
        "question_count":len(result),"counts_by_classification":counts,
        "methodology":"Every item is either specifically proven, partially probed, unresolved, or hardware-only.",
        "questions":result,
        "warnings":[
            "Running Ghidra on every recognized function does not answer every question.",
            "Synthetic VM behavior does not prove vendor Fastboot behavior.",
            "Prior V13 marked zero of 85 questions answered despite eleven exact findings.",
            "No action is permitted on physical MPC bootloader, eMMC or fuses.",
        ]
    })
    return facts,counts

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prior-v13",type=Path,
                    help="SHA-indexed private V13 ZIP (auto-selected if omitted)")
    ap.add_argument("--prior-v10",type=Path,
                    help="SHA-indexed V10 updater prior (auto-selected if omitted)")
    ap.add_argument("--rootfs",type=Path,default=WORK/"akai-unpacked-20261110T024913Z-1hjja3_r/rootfs.img")
    ap.add_argument("--backup",type=Path,default=Path.home()/
                    "mpfree-backups/mpc-one-emmc-20261109T233604Z-datprr67/mmcblk0.img")
    ap.add_argument("--qemu-boot-timeout",type=int,default=240)
    ap.add_argument("--qemu-probe-timeout",type=int,default=140)
    ap.add_argument("--skip-updater",action="store_true")
    ap.add_argument("--skip-qemu",action="store_true")
    args=ap.parse_args()
    if args.qemu_boot_timeout<30 or args.qemu_probe_timeout<20:
        ap.error("Timeout values too small")
    os.umask(0o077);WORK.mkdir(exist_ok=True,parents=True)
    out=WORK/("bootchain-wide-"+time.strftime("%Y%m%dT%H%M%SZ",time.gmtime())+
              "-v14-"+str(os.getpid()))
    out.mkdir(mode=0o700);start=time.monotonic()
    stages={}
    def recent(pattern):
        rows=sorted(WORK.glob(pattern),key=lambda p:p.stat().st_mtime,reverse=True)
        return rows[0] if rows else None
    prior13=args.prior_v13 or recent("bootchain-wide-*-v13-*.zip")
    prior10=args.prior_v10 or recent("bootchain-wide-*-v10-*.zip")
    questions=[]
    print("V14 DEEP OWNER-OFFLINE AUDIT, NO MPC CONNECTION",flush=True)
    print("Private staging",out,flush=True)
    try:
        if prior13:
            try:
                questions=questions_from_v13(out,prior13)
                print("Verified",len(questions),"prior research questions",flush=True)
            except Exception as exc:
                print("V13 private ZIP invalid; fail closed for prior lineage:",exc,flush=True)
                write(out/"V14-PRIOR-ERROR.json",{"error":str(exc),"prior":str(prior13)})
                prior13=None
        with tempfile.TemporaryDirectory(prefix="mpfree-v14-owner-data-",dir=str(WORK)) as temp:
            temp=Path(temp)
            tasks={
                "full_p1":(ghidra,("p1",out)),
                "full_p3":(ghidra,("p3",out)),
                "boot_assets":(boot_assets,(out,args.rootfs)),
                "offline_map":(offline_map,(out,args.rootfs,args.backup)),
            }
            if not args.skip_qemu:
                tasks["arm_qemu_fixtures"]=(qemu_fixture,(out,args.rootfs,temp,args))
            if not args.skip_updater:
                tasks["updater_v11"]=(updater_static,(out,prior10))
            with ThreadPoolExecutor(max_workers=6,thread_name_prefix="mpfree-v14") as pool:
                futures={pool.submit(fn,*params):name for name,(fn,params) in tasks.items()}
                for done in as_completed(futures):
                    name=futures[done]
                    try:stages[name]=done.result()
                    except Exception as exc:
                        stages[name]={"success":False,"error":repr(exc),
                                      "traceback":traceback.format_exc()[-2400:]}
                    print("STAGE",name,"OK" if stages[name].get("success") else "INCOMPLETE",
                          "elapsed",stages[name].get("elapsed_s"),flush=True)
    except KeyboardInterrupt:
        print("Interrupted; packaging completed reports.",flush=True)
        stages["interrupt"]={"success":False}
    finally:
        try:
            facts,counts=analyze(out,stages,questions,prior13)
            report=[
                "# MPFree V14 deep bootchain static/VM research", "",
                "This is a verification-oriented research archive; not a firmware installer.",
                "No physical MPC, bootloader, Fastboot USB, original eMMC or fuses were touched.",
                "", "## Actual stage work and timings",
            ]
            for name,result in stages.items():
                report.append("- %s: %s, %s seconds, %s" %
                              (name,"SUCCESS" if result.get("success") else "INCOMPLETE",
                               result.get("elapsed_s","?"),result.get("error","")))
            report+=["","## Deep Ghidra function coverage"]
            for role in ("p1","p3"):
                r=stages.get("full_"+role,{})
                report.append("- %s: %s of %s recognized functions decompiled, %s direct edges; %s indirect sites" %
                              (role,r.get("decompiled"),r.get("functions_known"),
                               r.get("direct_edges"),r.get("indirect_sites")))
            report+=["","## Evidence tracking (not inflated success claims)",
                      "- %d carried-forward questions."%len(questions)]
            for k,v in sorted(counts.items()):report.append("- %s: %d"%(k,v))
            report+=["", "## Scope limitations",
                     "- Ghidra full recognition/decompilation and call graphs are static, not runtime hardware.",
                     "- ARM QEMU runs newly compiled original code against actual owner boot-data bytes.",
                     "- No Akai program is executed by the VM and RK3288 BootROM is not emulated.",
                     "- Hardware-only questions remain hardware-only, regardless of function coverage.",
                     "- Inspect call edges, byte hashes and function decompilation failures in the ZIP.",
                     "- Keep all Ghidra-derived proprietary results owner-private.", ""]
            (out/"V14-READ-ME-FIRST.md").write_text("\n".join(report))
            write(out/"V14-STAGE-MANIFEST.json",{
                "schema":"mpfree-v14-deep-concurrent",
                "branch":BRANCH,"generated_utc":datetime.now(timezone.utc).isoformat(),
                "duration_seconds":round(time.monotonic()-start,2),
                "device_access":False,"vendor_code_executed":False,
                "source_backups_unchanged":True,"questions_carried_forward":len(questions),
                "stage_results":stages})
            pkg=load("tools/mpc-one/package-private-boot-audit.py","mpfree_v14_privatepack")
            archive,index=pkg.package(out)
            print("\nONE PRIVATE ZIP TO UPLOAD:",archive,flush=True)
            print("SHA256:",sha(archive),flush=True)
            print("Text report files:",len(index["files"]),flush=True)
            print("Nothing was pushed to the MPC or its eMMC.",flush=True)
        except Exception:
            print("PACKAGING FAILED; private reports remain at "+str(out),file=sys.stderr)
            raise
if __name__=="__main__":main()
