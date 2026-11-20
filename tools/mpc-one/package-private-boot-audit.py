#!/usr/bin/env python3
"""Create ONE private ZIP with every generated MPFree boot audit report.

Only archives files under a specified regular directory. Refuses symlinks,
oversized reports and any file with device semantics; never copies backups,
firmware images, or Ghidra projects into the archive.
Writes ZIP one level above the report directory.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import zipfile

MAX_TOTAL = 128 * 1024 * 1024
MAX_FILE = 32 * 1024 * 1024
ALLOW_SUFFIXES = {".json", ".txt", ".log", ".md", ".csv", ".tsv"}
EXCLUDE_NAMES = {"private-key", "credentials"}

def package(root):
    root = Path(root).expanduser()
    if not root.is_absolute():
        raise ValueError("Report directory must be an absolute path")
    if not root.is_dir() or root.is_symlink():
        raise ValueError("Require a real existing report directory")
    if not root.name.startswith("bootchain-wide-"):
        raise ValueError("Refusing a directory not named bootchain-wide-*")
    files=[]
    total=0
    for candidate in sorted(root.rglob("*")):
        if candidate.is_symlink():
            raise ValueError("Refusing symlink in report tree: "+str(candidate))
        if candidate.is_dir():continue
        if not stat.S_ISREG(candidate.lstat().st_mode):
            raise ValueError("Refusing non-regular file in report tree")
        if candidate.suffix.lower() not in ALLOW_SUFFIXES:
            raise ValueError("Refusing unexpected file extension: "+str(candidate))
        if any(secret in candidate.name.lower() for secret in EXCLUDE_NAMES):
            raise ValueError("Refusing credential-like file name: "+str(candidate))
        size=candidate.stat().st_size
        if size>MAX_FILE:
            raise ValueError("Unexpectedly large report file: "+str(candidate))
        total+=size
        if total>MAX_TOTAL:
            raise ValueError("Aggregate reports exceed safe archive size")
        files.append(candidate)
    if not files:raise ValueError("No audit files to archive")
    manifest={"schema":"mpfree-private-audit-zip-index-v1","private":True,
              "note":"Do not commit this ZIP or its contents to a public repository.",
              "files":[]}
    for path in files:
        data=path.read_bytes()
        manifest["files"].append({
            "name":str(path.relative_to(root)).replace(os.sep,"/"),
            "bytes":len(data),"sha256":hashlib.sha256(data).hexdigest()})
    zip_path=root.with_suffix(".zip")
    if zip_path.exists():raise FileExistsError("Never overwrite existing ZIP: "+str(zip_path))
    old=os.umask(0o077)
    try:
        with zipfile.ZipFile(zip_path,"x",compression=zipfile.ZIP_DEFLATED,
                             compresslevel=6,allowZip64=False) as archive:
            archive.writestr("ZIP_INDEX.json",json.dumps(manifest,indent=2)+"\n")
            for path in files:
                archive.write(path,arcname=str(path.relative_to(root)))
        with zipfile.ZipFile(zip_path,"r") as archive:
            bad=archive.testzip()
            if bad:raise RuntimeError("ZIP integrity check failed: "+bad)
            names=set(archive.namelist())
            if len(names)!=len(files)+1:
                raise RuntimeError("ZIP entry count mismatch")
        sha=hashlib.sha256(zip_path.read_bytes()).hexdigest()
        checksum=zip_path.with_suffix(".zip.sha256")
        checksum.write_text(sha+"  "+zip_path.name+"\n",encoding="ascii")
    finally:
        os.umask(old)
    return zip_path,manifest

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report_directory",type=Path)
    args=parser.parse_args()
    zip_path,index=package(args.report_directory)
    print("SINGLE PRIVATE UPLOAD ZIP:",zip_path)
    print("ZIP includes %d files. Size: %d bytes" %
          (len(index["files"]),zip_path.stat().st_size))
    print("Checksum:",zip_path.with_suffix(".zip.sha256"))
    print("Upload the ZIP only (not all JSON/log files separately).")

if __name__=="__main__":
    main()
