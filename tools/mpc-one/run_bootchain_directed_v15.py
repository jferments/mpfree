#!/usr/bin/env python3
"""MPFree V15: directed OWNER-OFFLINE boot research, private single ZIP.

Concurrent SHA-verified P1/P3 Ghidra P-code analysis, comprehensive reuse of
V14 graph+V11 updater reports, then a novel actual-U-Boot *function slice*
execution against mock RAM inside generic ARM QEMU virt. NO MPC/USB/eMMC contact,
no actual vendor updater or full bootloader, no persistent firmware writes.
"""
from __future__ import annotations
import argparse
from collections import defaultdict, deque
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
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

ROOT=Path(__file__).resolve().parents[2]
WORK=Path.home()/"mpfree-work"
BRANCH="research/bootchain-directed-v15-20261112"
P1_SHA="6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"
P3_SHA="b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
def load(name,alias):
    spec=importlib.util.spec_from_file_location(alias,ROOT/name)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module
v14=load("tools/mpc-one/run_bootchain_deep_v14.py","mpfree_v15_prior")
v12=v14.v12
def write(path,data):
    path.write_text(json.dumps(data,indent=2,sort_keys=True)+"\n",encoding="utf8")
def digest(path):
    h=hashlib.sha256()
    with path.open("rb") as f:
        for b in iter(lambda:f.read(1024*1024),b""):h.update(b)
    return h.hexdigest()
def timed(name,call,*args):
    before=time.monotonic()
    try:r=call(*args)
    except Exception as e:r={"success":False,"error":repr(e),
                             "traceback":traceback.format_exc()[-2800:]}
    r.update({"stage":name,"elapsed_seconds":round(time.monotonic()-before,2)})
    return r

def ghidra(role,dst,private_page):
    report=dst/("v15-"+role+"-directed.json")
    console=dst/("v15-"+role+"-ghidra-console.log")
    result={"success":False,"verified":False,"vendor_code_executed":False}
    try:
        run=v12.ghidra_launcher()
        if not run:raise RuntimeError("Ghidra launcher unavailable")
        project=v12.project(role)
        input_name=("p1-rk3288-sram-stage.bin" if role=="p1"
                    else "p3-uimage-payload.bin")
        expected=P1_SHA if role=="p1" else P3_SHA
        cmd=run+[str(v12.PROJECTS),project,"-process",input_name,
                 "-readOnly","-noanalysis","-scriptPath",str(ROOT/"tools/ghidra"),
                 "-postScript","MPFreeDirectedBootV15.py",role,str(report),
                 str(private_page) if role=="p3" else "none",
                 "-log",str(dst/("v15-"+role+"-ghidra-engine.log"))]
        with console.open("w") as f:
            p=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT,
                             timeout=2200,check=False)
        if p.returncode:raise RuntimeError("Ghidra exit "+str(p.returncode))
        j=json.loads(report.read_text())
        if (j.get("schema")!="mpfree-v15-directed-ghidra-v1" or
            j.get("role")!=role or j.get("source_sha256")!=expected or
            not j.get("image_hash_verified")):
            raise RuntimeError("Pinned owner SHA or V15 report schema missing")
        if role=="p3":
            if not private_page.is_file() or private_page.stat().st_size!=4096:
                raise RuntimeError("Missing temporary 4KB P3 page")
            if digest(private_page)!=j["isolated_code_page"]["sha256"]:
                raise RuntimeError("Temporary code page SHA differs from verified Ghidra")
        result.update({"success":True,"verified":True,"functions":j["statistics"]["reported_functions"],
                       "pcode_ops":j["statistics"]["pcode_operations"],
                       "indirect_sites":j["statistics"]["unresolved_indirect_sites"],
                       "report":report.name,"source_sha256":expected})
    except Exception as exc:
        result["error"]=str(exc)
        with console.open("a") as f:f.write("\nV15 Ghidra stage failed:\n"+traceback.format_exc()[-2100:])
    return result

def graph_path(graph,source,target,max_steps=8):
    if source==target:return [source]
    queue=deque([(source,[source])]);seen={source}
    while queue:
        node,path=queue.popleft()
        if len(path)>max_steps:continue
        for nxt in graph.get(node,()):
            if nxt==target:return path+[nxt]
            if nxt not in seen:
                seen.add(nxt);queue.append((nxt,path+[nxt]))
    return None

def build_graph(report):
    edges=defaultdict(set)
    for addr,item in report["functions"].items():
        for op in item.get("calls",[]):
            for target in op.get("target_entries",[]):
                if target in report["functions"]:
                    edges[addr].add(target)
    return edges

def focused_updater(lib):
    data=lib.get("analyzed_functions",{})
    graph=defaultdict(set)
    for address,f in data.items():
        for dst in f.get("graph",{}).get("callees",[]):
            graph[address].add(dst)
    terms=("backend","write","verify","signature","prepare","partition","bootloader",
           "stream","commit","rockusb","loader","erase","check","flash","update","device")
    candidates=[]
    for address,f in data.items():
        name=f.get("name","")
        if any(t in name.lower() for t in terms):
            candidates.append({"address":address,"name":name,
              "direct_callees":f.get("graph",{}).get("callees",[]),
              "computed_calls":f.get("graph",{}).get("computed",[]),
              "snippet":f.get("pseudocode","")[:3800]})
    candidates.sort(key=lambda x:(not any(y in x["name"].lower() for y in
                                    ("signature","backend","bootloader","verify")),-len(x["computed_calls"])))
    sources=[addr for addr,fn in data.items() if "write_image" in fn.get("name","")]
    sinks=[addr for addr,fn in data.items()
           if any(t in fn.get("name","").lower() for t in ("signature","verify","bootloader","write_partition"))]
    routes=[]
    for a in sources[:14]:
        for b in sinks[:22]:
            path=graph_path(graph,a,b,7)
            if path and len(path)>1:routes.append({"from":a,"to":b,"path":path})
    indirect=[{"function":addr,"name":f.get("name"),
               "computed":f.get("graph",{}).get("computed",[])}
              for addr,f in data.items() if f.get("graph",{}).get("computed")]
    return {"analyzed_functions":len(data),"candidates":candidates[:125],
            "computed_call_sites":indirect,"recovered_direct_paths":routes[:130],
            "limits":["An unobserved path in a partial Ghidra graph is NOT proof of unreachable behavior.",
                      "ELF vtable assignments may be relocated and need further proof."]}

def term_hits(q,item):
    text=(item.get("name","")+" "+item.get("pseudocode","")).lower()
    words=set(re.findall(r"[a-z][a-z0-9_]{3,}",q.lower()))
    common={"does","have","what","which","where","this","that","with","from",
            "when","into","only","without","could","would","actual","current",
            "exact","possible","before","after","firmware","device","using"}
    words-=common
    return len([word for word in words if word in text])

def prior_analysis(dst,prior):
    result={"success":False,"vendor_code_executed":False,"source":"verified V14 private ZIP"}
    try:
        rows=v14.verified_archive(prior)
        p1=json.loads(rows["v14-full-p1-ghidra.json"])
        p3=json.loads(rows["v14-full-p3-ghidra.json"])
        if p1.get("owner_sha256")!=P1_SHA or p3.get("owner_sha256")!=P3_SHA:
            raise RuntimeError("Unrecognized owner boot image lineage")
        if not p1.get("owner_hash_verified") or not p3.get("owner_hash_verified"):
            raise RuntimeError("V14 Ghidra image verification missing")
        ledger=json.loads(rows["V14-85-QUESTION-EVIDENCE-LEDGER.json"])
        questions=ledger.get("questions",[])
        if len(questions)!=85:raise RuntimeError("Expected 85 tracked questions")
        allgraph=build_graph(p3)
        fns=p3["functions"]
        origins={
            "oem_unlock":"0x00122854",
            "download":"0x00122980",
            "usb_receive":"0x00122a10",
            "boot_completion":"0x001226b4",
            "board_boot_selection":"0x00100fe4",
            "fastboot_getvar":"0x00122d38",
        }
        targets={
            "permission_gate":"0x00122834",
            "command_interpreter":"0x00110484",
            "bootm":"0x00101744",
            "cpu_reset":"0x0011622a",
            "image_classifier":"0x0010e390",
            "environment_candidate":"0x00108560",
        }
        routes=[]
        for aname,a in origins.items():
            for bname,b in targets.items():
                p=graph_path(allgraph,a,b,9)
                if p:
                    routes.append({"from":aname,"to":bname,"path":p,
                                   "function_names":[fns[x]["name"] for x in p]})
        # Find potentially storage-mutating paths without interpreting absence as proof.
        write_names=("saveenv","env_save","mmc_write","write_partition","write_bootloader",
                     "flash","erase","setenv","run_command")
        mutation_candidates=[]
        for addr,item in fns.items():
            title=item.get("name","").lower()
            if any(x in title for x in write_names):
                mutation_candidates.append({"address":addr,"name":item["name"]})
        mutation_paths=[]
        for a in origins.values():
            for item in mutation_candidates:
                p=graph_path(allgraph,a,item["address"],8)
                if p:
                    mutation_paths.append({"source":a,"target":item["address"],"path":p,
                                           "target_name":item["name"]})
        # Correlate every single question against the same verified corpus.
        topic_evidence=[]
        for question in questions:
            text=question["question"]
            kind=question.get("id","")
            selected=[]
            for addr,entry in fns.items():
                score=term_hits(text,entry)
                if score>=2:selected.append((score,addr,entry))
            selected.sort(key=lambda t:(-t[0],t[1]))
            topic_evidence.append({
                "id":kind,"question":text,
                "prior_classification":question.get("classification"),
                "fresh_analysis":"INDEXED_PRIOR_CORPUS_ONLY",
                "best_function_candidates":[{"address":a,"name":x["name"],"keyword_hits":s,
                    "decompile_status":x.get("decompile_status")} for s,a,x in selected[:6]],
                "no_new_runtime_proof":True})
        updater=None
        if "updater-v11/v11-ghidra-libaz01-prog.json" in rows:
            lib=json.loads(rows["updater-v11/v11-ghidra-libaz01-prog.json"])
            if lib.get("source_sha256_prechecked_by_launcher")!=(
                 "af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623"):
                raise RuntimeError("Updater library checksum lineage mismatch")
            updater=focused_updater(lib)
        out={
            "schema":"mpfree-v15-prior-static-directed-graph",
            "owner_v14_zip_sha256":digest(prior),
            "call_edges":sum(len(v) for v in allgraph.values()),
            "routes":routes,
            "mutation_candidates":mutation_candidates,
            "mutation_paths":mutation_paths[:180],
            "updater":updater,
            "individual_questions":topic_evidence,
            "question_count":len(topic_evidence),
            "research_warnings":[
                "All graph results here are derived from V14 and are NOT newly decompiled functions.",
                "Matching function names or topics does not close a question.",
                "A missing direct call does not prove absence through indirect calls.",
                "No hardware interactions, vendor updater execution or live Fastboot.",
            ]}
        write(dst/"V15-PRIOR-GRAPH-AND-85-QUESTIONS.json",out)
        result.update({"success":True,"questions_indexed":len(topic_evidence),
                       "direct_graph_edges":out["call_edges"],"paths":len(routes),
                       "possible_mutation_paths":len(mutation_paths),
                       "updater_functions_reused":updater["analyzed_functions"] if updater else 0})
    except Exception as exc:
        result["error"]=str(exc)
        (dst/"v15-prior-analysis-error.log").write_text(traceback.format_exc()[-3000:])
    return result

def qemu_vendor_permission(dst,private_page,temporary,args,ghidra_result):
    result={"success":False,"kind":"ACTUAL_PINNED_VENDOR_FUNCTION_IN_MOCK_RAM",
            "guest_board":"generic ARM virt, NOT RK3288",
            "hardware_contact":False,"real_usb_stack_executed":False,
            "vendor_updater_executed":False,
            "known_modified_code":"only USB-response function replaced by Thumb BX LR"}
    logfile=dst/"v15-qemu-vendor-permission-console.log"
    proc=None;master=slave=None
    try:
        if not ghidra_result.get("success"):
            raise RuntimeError("Skipped dynamic test: Ghidra P3 verification failed")
        q=load("tools/mpc-one/arm_qemu_lab.py","mpfree_v15_qemu")
        integ=load("tools/mpc-one/run_bootchain_integrated_lab.py","mpfree_v15_vm_mapping")
        for tool in ("qemu-system-arm","qemu-img","arm-linux-gnueabihf-gcc","mkfs.ext4"):
            if not shutil.which(tool):raise RuntimeError("VM prerequisite missing: "+tool)
        private_page=v12.regular(private_page)
        if private_page.stat().st_size!=4096:raise RuntimeError("Unexpected private code page")
        source=v12.regular(ROOT/"tools/mpc-one/v15_vendor_permission_guest.c")
        folder=temporary/"guest-files";folder.mkdir(mode=0o700)
        binary=folder/"v15_permission_lab"
        p=subprocess.run(["arm-linux-gnueabihf-gcc","-std=c11","-O2","-static",
                          "-Wall","-Wextra","-Werror","-o",str(binary),str(source)],
                         capture_output=True,text=True,timeout=120)
        if p.returncode:raise RuntimeError("ARM compile failed: "+p.stderr[:1500])
        shutil.copyfile(private_page,folder/"p3-fastboot-page.bin")
        disk=temporary/"v15-private-guest-fixture.ext4"
        with disk.open("wb") as f:f.truncate(32*1024*1024)
        p=subprocess.run(["mkfs.ext4","-F","-q","-d",str(folder),str(disk)],
                         capture_output=True,text=True,timeout=90)
        if p.returncode:raise RuntimeError("QEMU fixture creation failed "+p.stderr[:1000])
        cmd=q.qemu_args(q.KERNEL)
        cmd.insert(1,"-snapshot")
        cmd=integ.qemu_storage_for_synthetic_model(cmd,disk)
        if (cmd[cmd.index("-nic")+1]!="none" or
            cmd[cmd.index("-monitor")+1]!="none" or
            any("usb-host" in arg or "hostdev" in arg for arg in cmd)):
            raise RuntimeError("Unsafe QEMU device/network options")
        result["verified_p3_page_sha256"]=digest(private_page)
        result["guest_kernel_sha256"]=digest(q.KERNEL)
        result["guest_clone_sha256"]=digest(q.BASE)
        master,slave=pty.openpty()
        proc=subprocess.Popen(cmd,stdin=slave,stdout=slave,stderr=slave,
                              close_fds=True,start_new_session=True)
        os.close(slave);slave=None
        token="MPFREE_V15_COMPLETED_"+str(os.getpid())
        commands=["echo MPFREE_V15_GUEST_BOOTED",
                  "mount -t proc proc /proc 2>/dev/null || true",
                  "mkdir -p /mnt/mpfree-v15",
                  "mount -t ext4 -o ro /dev/vda /mnt/mpfree-v15",
                  "/mnt/mpfree-v15/v15_permission_lab /mnt/mpfree-v15/p3-fastboot-page.bin",
                  "echo "+token]
        sent=False;data="";deadline=time.monotonic()+args.qemu_boot_timeout
        with logfile.open("w",encoding="utf8",errors="replace") as f:
            while True:
                if proc.poll() is not None:
                    result["error"]="Guest process exited prematurely";break
                if time.monotonic()>deadline:
                    result["error"]="Guest boot/probe deadline exceeded";break
                ready,_,_=select.select([master],[],[],0.25)
                if not ready:continue
                try:chunk=os.read(master,65536)
                except OSError:break
                if not chunk:break
                piece=chunk.decode("utf8","replace");f.write(piece);f.flush()
                data+=piece
                if len(data)>2*1024*1024:data=data[-2*1024*1024:]
                if not sent and re.search(r"(?:^|[\r\n])(?:/ # |# |~ # )",data[-4000:]):
                    for command in commands:
                        os.write(master,(command+"\n").encode())
                    sent=True;deadline=time.monotonic()+args.qemu_probe_timeout
                if sent and re.search(r"(?m)^"+re.escape(token)+r"[ \t]*$",data.replace("\r","")):
                    break
        tests=[]
        for line in data.replace("\r","").splitlines():
            if line.startswith('{"test":'):
                try:tests.append(json.loads(line))
                except ValueError:pass
        result["actual_function_tests"]=len(tests)
        result["actual_function_passed"]=sum(r.get("pass") is True for r in tests)
        result["success"]=bool("MPFREE_V15_VENDOR_FUNCTION_PASS" in data and
              len(tests)>=7 and all(r.get("pass") is True for r in tests)
              and re.search(r"(?m)^"+re.escape(token)+r"[ \t]*$",data.replace("\r","")))
        if not result["success"] and "error" not in result:
            result["error"]="Real ARM permission function did not complete expected mock-RAM cases"
    except Exception as exc:
        result["error"]=str(exc)
        with logfile.open("a") as f:f.write("\n"+traceback.format_exc()[-3000:])
    finally:
        if proc and proc.poll() is None:
            try:os.killpg(proc.pid,signal.SIGTERM);proc.wait(timeout=10)
            except Exception:
                try:os.killpg(proc.pid,signal.SIGKILL)
                except ProcessLookupError:pass
                proc.wait(timeout=10)
        if master is not None:os.close(master)
        if slave is not None:os.close(slave)
        write(dst/"V15-ACTUAL-VENDOR-FUNCTION-VM.json",result)
    return result

def synthesize(dst,stages,prior):
    results={"schema":"mpfree-v15-verified-evidence","prior_v14_sha256":digest(prior),
             "conclusions":{},"all_85_questions_closed":False}
    p3file=dst/"v15-p3-directed.json"
    if p3file.is_file():
        p3=json.loads(p3file.read_text())
        alias=p3.get("permission_alias",{})
        results["conclusions"]["auth_pointer_alias"]={
            "status":"ANSWERED_STATIC" if alias.get("same_nonzero_address") else "UNRESOLVED",
            "facts":alias,"evidence":"v15-p3-directed.json:permission_alias"}
        cmd=p3.get("command_dispatch",[])
        results["conclusions"]["fastboot_command_table"]={
            "status":"ANSWERED_STATIC" if len(cmd)==8 else "UNRESOLVED",
            "entries":cmd,"evidence":"v15-p3-directed.json:command_dispatch"}
    vm=stages.get("real_vendor_function_vm",{})
    results["conclusions"]["isolated_permission_execution"]={
        "status":"ANSWERED_ISOLATED_ARM_FUNCTION_ONLY" if vm.get("success")
                 else "UNRESOLVED",
        "test_cases":vm.get("actual_function_tests",0),
        "passed":vm.get("actual_function_passed",0),
        "evidence":"V15-ACTUAL-VENDOR-FUNCTION-VM.json",
        "limits":"No physical MPC, no OEM command handler, no USB hardware; denied branch uses BX LR stub."}
    graph=dst/"V15-PRIOR-GRAPH-AND-85-QUESTIONS.json"
    if graph.is_file():
        g=json.loads(graph.read_text())
        results["questions_prior_evidence_reviewed"]=g["question_count"]
        results["direct_call_routes_reconstructed"]=len(g["routes"])
        results["updater_analysis_reused_not_rerun"]=True
    write(dst/"V15-EXACT-FINDINGS.json",results)
    summary=[
      "# MPFree V15 directed Fastboot/direct-boot research","",
      "**Only V15 Ghidra and permission-function ARM guest tests are new vendor-code investigations.**",
      "All V14 graph and V11 updater reuse is labelled accordingly.","",
      "## Actual stages"]
    for key,v in stages.items():
        summary.append("- %s: %s, %.2f s%s"%(key,
                "PASS" if v.get("success") else "INCOMPLETE",
                v.get("elapsed_seconds",0.0),
                " — "+v["error"] if v.get("error") else ""))
    summary+=["","## Core evidence"]
    for key,v in results["conclusions"].items():
        summary.append("- %s: %s"%(key,v["status"]))
    summary+=["",
      "The ARM QEMU experiment, if passed, executes ONLY the verified P3 permission function",
      "at 0x00122834 in emulated user RAM. The USB-response subroutine is replaced",
      "in guest memory by a harmless return. It does not run the OEM unlock path,",
      "Fastboot USB, bootm, actual ROM, or update/flash operations.",
      "The ephemeral proprietary 4KB code slice is NOT in this ZIP.",
      "Detailed P-code and references in v15-p3-directed.json are STATIC evidence.",
      "The previous 85 questions remain open unless an exact narrower proposition is established.",
      "",
      "## Best next experiments",
      "1. Mock and execute the full OEM compare path only after verifying all external calls.",
      "2. Establish actual U-Boot RAM relocation/stack/DRAM reservation before bootm emulation.",
      "3. Trace temporary SD boot selection from OEM command handler to boot_targets.",
      "4. Resolve updater backend virtual dispatch and signature gating with focused P-code.",
      "5. Hardware/MaskROM evidence still requires separately authorized observation.",
      ""]
    (dst/"V15-READ-ME-FIRST.md").write_text("\n".join(summary),encoding="utf8")
    return results

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prior-v14",type=Path,help="SHA-indexed private V14 ZIP; autodetect if omitted")
    ap.add_argument("--skip-vm",action="store_true")
    ap.add_argument("--qemu-boot-timeout",type=int,default=240)
    ap.add_argument("--qemu-probe-timeout",type=int,default=100)
    args=ap.parse_args()
    if args.qemu_boot_timeout<30 or args.qemu_probe_timeout<20:
        ap.error("Guest timeouts too small")
    os.umask(0o077);WORK.mkdir(exist_ok=True,parents=True)
    if args.prior_v14:
        prior=v12.regular(args.prior_v14)
    else:
        matches=sorted(WORK.glob("bootchain-wide-*-v14-*.zip"),
                       key=lambda x:x.stat().st_mtime,reverse=True)
        if not matches:ap.error("No SHA-indexed V14 ZIP found in ~/mpfree-work")
        prior=v12.regular(matches[0])
    # Fail closed before running expensive analyses.
    rows=v14.verified_archive(prior)
    for rel,want in (("v14-full-p1-ghidra.json",P1_SHA),
                     ("v14-full-p3-ghidra.json",P3_SHA)):
        j=json.loads(rows[rel])
        if not j.get("owner_hash_verified") or j.get("owner_sha256")!=want:
            ap.error("V14 prior source image SHA mismatch: "+rel)
    stamp=time.strftime("%Y%m%dT%H%M%SZ",time.gmtime())
    dst=WORK/("bootchain-wide-"+stamp+"-v15-"+str(os.getpid()))
    dst.mkdir(mode=0o700)
    stages={};started=time.monotonic()
    print("MPFree V15 owner-private directed read-only analysis",flush=True)
    print("Prior SHA-verified V14 archive:",prior,flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="mpfree-v15-private-fn-",dir=str(WORK)) as temp:
            private=Path(temp)
            page=private/"verified-p3-fastboot-page.bin"
            with ThreadPoolExecutor(max_workers=4,thread_name_prefix="mpfree-v15") as pool:
                jobs={
                    "ghidra_p1":pool.submit(timed,"ghidra_p1",ghidra,"p1",dst,page),
                    "ghidra_p3":pool.submit(timed,"ghidra_p3",ghidra,"p3",dst,page),
                    "prior_graph":pool.submit(timed,"prior_graph",prior_analysis,dst,prior),
                }
                # Begin QEMU as soon as the pinned private P3 page has been exported.
                p3=jobs["ghidra_p3"].result()
                stages["ghidra_p3"]=p3
                if not args.skip_vm:
                    jobs["real_vendor_function_vm"]=pool.submit(
                        timed,"real_vendor_function_vm",
                        qemu_vendor_permission,dst,page,private,args,p3)
                for name,fut in jobs.items():
                    if name=="ghidra_p3":continue
                    stages[name]=fut.result()
                    print("STAGE",name,"PASS" if stages[name].get("success") else "INCOMPLETE",
                          stages[name].get("elapsed_seconds"),flush=True)
                print("STAGE ghidra_p3","PASS" if p3.get("success") else "INCOMPLETE",
                      p3.get("elapsed_seconds"),flush=True)
    except KeyboardInterrupt:
        stages["interrupted"]={"success":False,"error":"Owner interrupted"}
        print("Interrupted; packaging available results.",flush=True)
    finally:
        try:
            evidence=synthesize(dst,stages,prior)
            write(dst/"V15-STAGE-MANIFEST.json",{
                "schema":"mpfree-v15-directed-offline",
                "timestamp_utc":datetime.now(timezone.utc).isoformat(),
                "branch":BRANCH,"elapsed_seconds":round(time.monotonic()-started,2),
                "stages":stages,"original_files_unchanged":True,
                "hardware_contact":False,"physical_usb_pass_through":False,
                "vendor_updater_executed":False,
                "temporary_vendor_function_bytes_included_in_archive":False,
                "prior_archive_sha256":digest(prior)})
            pkg=load("tools/mpc-one/package-private-boot-audit.py",
                     "mpfree_v15_zip")
            archive,index=pkg.package(dst)
            print("\nONE PRIVATE ZIP TO UPLOAD:",archive,flush=True)
            print("SHA256:",digest(archive),flush=True)
            print("Indexed text reports:",len(index["files"]),flush=True)
            print("No physical MPC, USB or eMMC access.",flush=True)
        except Exception:
            print("Packaging failed; reports remain at "+str(dst),file=sys.stderr)
            raise
if __name__=="__main__":
    main()
