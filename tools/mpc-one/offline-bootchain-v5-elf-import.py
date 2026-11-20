#!/usr/bin/env python3
"""Deep OFFLINE Ghidra import of three 3.10 boot/security helper ELFs.

Reads extracted 3.10 regular-file ext4 using debugfs and verifies kernel
version. Imports selected ELF bytes into *ephemeral* Ghidra projects under a
private temporary directory, invokes read-only-to-source decompilation and
removes the projects. Never runs the ELF, contacts the MPC, edits existing
Ghidra projects, writes eMMC, or flashes anything.

Results and logs remain in the existing private report ZIP directory.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import tempfile
from datetime import datetime, timezone

TARGETS={
 "bootloader_signature":"/usr/bin/az01-bootloader-sig",
 "loadpin_trustpoints":"/usr/sbin/az0x-loadpin-trustpoints",
 "board_az01config":"/usr/sbin/az01config",
}
KNOWN_ELF_SHA={
 "bootloader_signature":"b7a82030c85ea90e31533773538abfc537d46982bdeaead6963ec9e5ea9a5861",
 "loadpin_trustpoints":"fd59c2946b2f8e4bc1c143d5270881aa2168ad2851c4726748642d2f4e040239",
 "board_az01config":"2a7b6b7d24a2c0f52b66edb9b59447b513f3ba8cea071cbdba0013b12096390f",
}
def sh(argv,timeout=60):
    return subprocess.run(argv,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                          timeout=timeout,check=False)
def get_file(ext4,path):
    p=sh(["debugfs","-R","cat "+path,str(ext4)])
    if p.returncode or b"File not found" in p.stderr:
        return None
    return p.stdout
def analyze(rootfs,repo,report_dir):
    if rootfs.is_symlink() or not stat.S_ISREG(rootfs.lstat().st_mode):
        raise ValueError("Rootfs must be a regular existing ext4 file, not symlink or live block device")
    if not report_dir.name.startswith("bootchain-wide-") or report_dir.is_symlink():
        raise ValueError("Refuse output outside bootchain-wide-* report tree")
    with rootfs.open("rb") as fh:
        fh.seek(1080)
        if fh.read(2)!=b"\x53\xef":raise ValueError("Rootfs is not ext4")
    boot=sh(["debugfs","-R","ls -p /boot",str(rootfs)]).stdout
    if b"6.18.44" not in boot:raise ValueError("Expected 3.10 Gen1 Linux 6.18.44 rootfs missing")
    script=repo/"tools"/"ghidra"/"MPFreeImportedELFAuditV5.py"
    if not script.is_file():raise ValueError("Missing Ghidra V5 ELF audit script")
    if os.environ.get("GHIDRA_HOME"):
        tool=Path(os.environ["GHIDRA_HOME"])/"support"/"pyghidraRun"
        if not tool.is_file():raise ValueError("Invalid GHIDRA_HOME")
        runner=[str(tool),"-H"]
    elif shutil.which("snap") and sh(["snap","list","ghidra"]).returncode==0:
        runner=["snap","run","ghidra.ghidra","-H"]
    else:
        raise RuntimeError("No supported offline Ghidra headless launcher")
    results={
       "schema":"mpfree-imported-linux-elfs-v5",
       "utc":datetime.now(timezone.utc).isoformat(),
       "rootfs_verified":"ext4 Linux 6.18.44, factory MPC 3.10 Gen1",
       "status":"private workstation static analysis; imported ELF never executed",
       "targets":{},
    }
    work=Path.home()/"mpfree-work"
    if not work.is_dir():raise ValueError("MPFree work directory missing")
    with tempfile.TemporaryDirectory(prefix="v5-elf-ephemeral-",dir=str(work)) as scratch:
        tmp=Path(scratch);prj=tmp/"projects";prj.mkdir()
        for index,(name,elfpath) in enumerate(TARGETS.items()):
            record={"elf_path":elfpath,"status":"not started"}
            results["targets"][name]=record
            content=get_file(rootfs,elfpath)
            if not content or content[:4]!=b"\x7fELF" or len(content)>2*1024*1024:
                record["status"]="missing or invalid ELF";continue
            sha=hashlib.sha256(content).hexdigest()
            record["size"]=len(content)
            record["sha256"]=sha
            if sha!=KNOWN_ELF_SHA[name]:
                record["status"]="SHA mismatch against verified 3.10 owner report; skipped"
                continue
            name_on_disk="mpfree-v5-"+name+".elf"
            source=tmp/name_on_disk
            source.write_bytes(content)
            source.chmod(0o400)
            output=report_dir/("v5-ghidra-"+name+".json")
            log=report_dir/("v5-ghidra-"+name+".log")
            console=report_dir/("v5-ghidra-"+name+"-console.txt")
            cmd=runner+[str(prj),"V5_"+name,
                         "-import",str(source),
                         "-scriptPath",str(script.parent),
                         "-postScript",script.name,str(output),name,sha,
                         "-log",str(log)]
            try:
                p=sh(cmd,timeout=600)
                console.write_bytes(p.stdout+b"\n---stderr---\n"+p.stderr)
                record["exit_code"]=p.returncode
                if p.returncode==0 and output.exists():
                    parsed=json.loads(output.read_text(encoding="utf-8"))
                    if parsed.get("source_sha256_verified_by_launcher")!=sha:
                        raise ValueError("Unexpected imported ELF SHA in report")
                    record["status"]="ok"
                    record["recognized_functions"]=parsed.get("recognized_functions")
                    record["decompiled"]=len(parsed.get("functions",{}))
                    record["strings"]=len(parsed.get("selected_strings",[]))
                else:record["status"]="Ghidra import/decompilation failed; see console"
            except (subprocess.TimeoutExpired,OSError,ValueError) as e:
                record["status"]="analysis error: "+type(e).__name__
                console.write_text(str(e)+"\n")
    return results
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rootfs-310",type=Path,required=True)
    parser.add_argument("--repo",type=Path,required=True)
    parser.add_argument("--report-dir",type=Path,required=True)
    args=parser.parse_args()
    root=args.report_dir.expanduser()
    if not root.is_absolute():parser.error("Absolute report directory required")
    root.mkdir(parents=True,exist_ok=True)
    old=os.umask(0o077)
    try:
        results=analyze(args.rootfs_310.expanduser(),args.repo.expanduser(),root)
        (root/"v5-imported-elf-summary.json").write_text(json.dumps(results,indent=2)+"\n")
    finally:os.umask(old)
    print("V5 ELF helper audits:",[(k,v["status"]) for k,v in results["targets"].items()])
    print("No binary material is included in the reports or ZIP.")
if __name__=="__main__":main()
