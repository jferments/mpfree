#!/usr/bin/env python3
"""Incremental offline V5 audit from a prior owner-private MPFree ZIP.

Copy ONLY hash-verified structured reports from the previous archive into
a fresh private bootchain-wide-* directory. Run NEW V5 cross-function
synthesis and optional 3.10 utility Ghidra imports, generate the evidence
ledger, and deliver ONE new verified ZIP. Avoid repeating V1-V4 Ghidra
decompilation of all 1,263 already recognized functions.

No device access, no U-Boot/Flash/ROM calls and no proprietary sources
are committed or redistributed. All original reports stay private.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import time
import zipfile

REQUIRED=(
 "p1-evidence-graph-v3.json",
 "p3-evidence-graph-v3.json",
 "p1-coverage-v4.json",
 "p3-coverage-v4.json",
 "firmware-3.10-bootchain-v4-compare.json",
)
OPTIONAL=(
 "storage-env-boot-dtb.json",
 "p1-p3-embedded-fdt-crossstage.json",
 "firmware-3.10-boot-compare.json",
 "firmware-3.10-rootfs-locator.json",
 "manifest.json",
 "p1-embedded-spl-dtb.json",
 "firmware-3.10-container.txt",
)
SPL_SHA="6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f"
UBOOT_SHA="b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
def require_file_regular(path):
    if path.is_symlink() or not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError("Require a private regular-file ZIP, not a symlink or live device")
def run_and_log(argv,dest,timeout=1200):
    result=subprocess.run(argv,capture_output=True,timeout=timeout,check=False)
    dest.write_bytes(result.stdout+b"\n---stderr---\n"+result.stderr)
    return result.returncode
def verify_zip(archive):
    with zipfile.ZipFile(archive) as z:
        if z.testzip():raise ValueError("ZIP has corrupt content")
        expected=json.loads(z.read("ZIP_INDEX.json"))
        rows=expected["files"]
        if len(rows)>100 or len(set(x["name"] for x in rows))!=len(rows):
            raise ValueError("Unexpected or duplicate index entries")
        known={}
        for row in rows:
            name=row["name"]
            if not name or "/" in name or "\\" in name or name.startswith("."):
                raise ValueError("Unexpected report path in ZIP index")
            info=z.getinfo(name)
            if (info.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError("Symlink in source ZIP")
            if info.file_size!=row["bytes"] or info.file_size>32*1024*1024:
                raise ValueError("Source report exceeds size bound")
            data=z.read(name)
            if hashlib.sha256(data).hexdigest()!=row["sha256"]:
                raise ValueError("Source report hash mismatch")
            known[name]=data
        for name in REQUIRED:
            if name not in known:raise ValueError("Missing required V3/V4 source: "+name)
        for kind,sha in (("p1",SPL_SHA),("p3",UBOOT_SHA)):
            v3=json.loads(known[kind+"-evidence-graph-v3.json"])
            v4=json.loads(known[kind+"-coverage-v4.json"])
            if v3.get("sha256")!=sha or v4.get("source_payload_sha256")!=sha:
                raise ValueError("Wrong expected P1/P3 owner image SHA in prior reports")
        return known
def write_private(path,content):
    path.write_bytes(content)
    path.chmod(0o600)
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("prior_zip",type=Path,help="prior bootchain-wide-*.zip on workstation")
    parser.add_argument("--no-elf-ghidra",action="store_true",
                        help="skip temporary Linux ELF decompilation, keep graph synthesis")
    parser.add_argument("--rootfs-310",type=Path,default=None,
                        help="explicit existing extracted 3.10 regular-file ext4")
    args=parser.parse_args()
    prior=args.prior_zip.expanduser()
    require_file_regular(prior)
    known=verify_zip(prior)
    repo=Path(__file__).resolve().parents[2]
    work=Path.home()/"mpfree-work"
    if not work.is_dir():parser.error("MPFree work directory missing")
    stamp=time.strftime("%Y%m%dT%H%M%SZ",time.gmtime())
    report=work/("bootchain-wide-"+stamp+"-v5-"+str(os.getpid()))
    report.mkdir(mode=0o700)
    required_sources=REQUIRED+OPTIONAL
    for name in required_sources:
        if name in known:write_private(report/name,known[name])
    lineage={
      "schema":"mpfree-v5-incremental-lineage-v1",
      "source_zip_sha256":hashlib.file_digest(prior.open("rb"),"sha256").hexdigest(),
      "source_zip_name":prior.name,
      "created_utc":datetime.now(timezone.utc).isoformat(),
      "source_reports_reused":list(name for name in required_sources if name in known),
      "method":"reuses validated SHA-indexed offline reports; no previous Ghidra passes rerun",
      "hardware":"never contacted",
    }
    write_private(report/"lineage.json",(json.dumps(lineage,indent=2)+"\n").encode())
    code=run_and_log(
        [sys.executable,str(repo/"tools/mpc-one/offline-bootchain-v5-synthesis.py"),str(report)],
        report/"bootchain-v5-synthesis-console.txt",timeout=300)
    if code!=0:print("WARNING: V5 synthesis failed; see ZIP console",file=sys.stderr)
    path=args.rootfs_310.expanduser() if args.rootfs_310 else None
    if path is None and "firmware-3.10-rootfs-locator.json" in known:
        locator=json.loads(known["firmware-3.10-rootfs-locator.json"])
        if locator.get("selected"):path=Path(locator["selected"])
    if not args.no_elf_ghidra and path and path.is_file() and not path.is_symlink():
        code=run_and_log([
            sys.executable,str(repo/"tools/mpc-one/offline-bootchain-v5-elf-import.py"),
            "--rootfs-310",str(path),"--repo",str(repo),
            "--report-dir",str(report)],
            report/"v5-elf-import-console.txt",timeout=2400)
        if code!=0:print("WARNING: Linux ELF import failed; see ZIP console",file=sys.stderr)
    else:
        write_private(report/"v5-elf-import-skipped.txt",
          ("No verified extracted 3.10 rootfs selected, or --no-elf-ghidra set.\n").encode())
    code=run_and_log([sys.executable,
                       str(repo/"tools/mpc-one/bootchain-evidence-ledger.py"),
                       str(report)],
                      report/"evidence-ledger-console.txt",timeout=120)
    if code!=0:print("WARNING: evidence ledger failed; see ZIP console",file=sys.stderr)
    spec=importlib.util.spec_from_file_location(
       "mpfree_pack_private_boot_audit",repo/"tools/mpc-one/package-private-boot-audit.py")
    mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
    archive,index=mod.package(report)
    print("ONE PRIVATE MPFREE V5 ZIP:",archive)
    print("Validated indexed files:",len(index["files"]))
    print("Keep private; upload this one ZIP. Source eMMC, boot media and runtime MPC are unchanged.")
if __name__=="__main__":main()
