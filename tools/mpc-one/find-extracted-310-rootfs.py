#!/usr/bin/env python3
"""Locate the previously extracted MPC 3.10 rootfs.ext4 safely.

This is a bounded OFFLINE metadata search inside MPFree work directories.
No device mount, firmware execution, network, extraction, or writes to sources.
The output is a private JSON report and a selected exact regular-file path
when kernel-version evidence supports a unique match.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
from datetime import datetime, timezone

def is_regular_ext4(path):
    try:
        if path.is_symlink() or not stat.S_ISREG(path.lstat().st_mode):return False
        size=path.stat().st_size
        if not 100*1024*1024<=size<=700*1024*1024:return False
        with path.open("rb") as f:
            f.seek(1080)
            return f.read(2)==b"\x53\xef"
    except (OSError,PermissionError):return False

def find(root, explicit=None):
    # Avoid recursively scanning backups, downloaded model directories or /dev.
    folders=[]
    if explicit:folders.append(Path(explicit).expanduser())
    folders.extend([
        root/"firmware-analysis",
        root/"akai-update-files",
        root/"boot-files",
    ])
    folders.extend(sorted(root.glob("akai-unpacked-*"))[:35])
    folders.extend(sorted(root.glob("MPC-3.10*"))[:15])
    candidates={}
    for folder in folders:
        if folder.is_file():
            paths=[folder]
        elif folder.is_dir() and not folder.is_symlink():
            # Bounded depth/filenames; avoid unexpectedly traversing large trees.
            paths=[]
            for current,dirs,files in os.walk(folder,followlinks=False):
                relative=Path(current).relative_to(folder)
                if len(relative.parts)>=5:dirs[:]=[]
                dirs[:]=[d for d in dirs if not d.startswith(".")
                         and d not in ("models","backups","ghidra","node_modules")]
                for name in files:
                    if (name.endswith((".ext4",".img")) or "rootfs" in name.lower()):
                        paths.append(Path(current)/name)
                if len(paths)>350:break
        else:continue
        for path in paths[:400]:
            if str(path) in candidates or not is_regular_ext4(path):continue
            evidence={"path":str(path),"bytes":path.stat().st_size,
                      "path_name_suggests_310":"3.10" in str(path)}
            if shutil.which("debugfs"):
                p=subprocess.run(["debugfs","-R","ls -p /boot",str(path)],
                                 capture_output=True,text=True,timeout=15)
                line=p.stdout
                # Linux version matches the known 3.10 Gen1 rootfs.
                evidence["kernel_618_44"]=bool(re.search(r"6[.]18[.]44",line))
                evidence["kernel_lines"]=[x[:180] for x in line.splitlines()
                                      if "zImage" in x or "6.18" in x][:10]
            candidates[str(path)]=evidence
    vals=list(candidates.values())
    # Prefer evidence from the embedded boot directory to path alone.
    strong=[x for x in vals if x.get("kernel_618_44")]
    selected=strong[0]["path"] if len(strong)==1 else None
    reason=("unique 6.18.44 kernel confirmed" if selected else
            "No unique verified 3.10 rootfs detected; choose explicit override")
    return {"schema":"mpfree-310-rootfs-locator-v1",
            "date_utc":datetime.now(timezone.utc).isoformat(),
            "selected":selected,"reason":reason,
            "candidates":vals[:60],
            "safety":"No image mounted, executed, modified, flashed, or uploaded."}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root",type=Path,default=Path.home()/"mpfree-work")
    parser.add_argument("--explicit",type=Path,default=None)
    parser.add_argument("--report",type=Path,required=True)
    args=parser.parse_args()
    if not args.report.is_absolute():parser.error("Absolute private report path required")
    report=find(args.root.expanduser(),args.explicit)
    args.report.parent.mkdir(parents=True,exist_ok=True)
    old=os.umask(0o077)
    try:args.report.write_text(json.dumps(report,indent=2)+"\n",encoding="utf-8")
    finally:os.umask(old)
    print("Offline rootfs candidates:",len(report["candidates"]))
    print("Selected:",report["selected"] or "(none)")
    print("Reason:",report["reason"])
if __name__=="__main__":main()
