#!/usr/bin/env python3
"""Find and inspect an FDT/DTB embedded in the verified RK3288 SPL image.

The owner backup is only read as a regular file. Any embedded board tree
is interpreted in memory using dtc; no source code runs, no device access.
The PRIVATE JSON may include vendor board parameters, so do not commit it.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import subprocess
from datetime import datetime, timezone

P1_START=64*512
P1_BYTES=128*512
SPL_OFFSET=2048
SPL_BYTES=32768
HASH="aa36e0e4033ce7386a210ad3e41f405a0d952a5b2a2c2711328450017414f1ec"
MAGIC=b"\xd0\x0d\xfe\xed"
KEYWORDS=("sdram","ddr","dram","pctl","phy-timing","pinctrl","regulator",
          "clock","uart","serial","mmc","sdhci","grf","cru","pmu",
          "noc","memory","sysreset","reset","sram","rockchip")
PRIVATE=re.compile(r"serial|ethaddr|macaddr|uuid|partuuid|password|token|secret|unlock|key-id",re.I)

def inspect(backup):
    path=Path(backup).expanduser()
    if not path.is_file() or not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError("Input must be an existing regular file, not a block device or symlink")
    with path.open("rb") as f:
        f.seek(P1_START)
        p1=f.read(P1_BYTES)
    if len(p1)!=P1_BYTES or hashlib.sha256(p1).hexdigest()!=HASH:
        raise ValueError("Reference P1 hash mismatch; refuse to guess format")
    if p1[SPL_OFFSET:SPL_OFFSET+4]!=b"RK32":
        raise ValueError("Expected RK32 SRAM SPL at P1 offset 2048")
    spl=p1[SPL_OFFSET:SPL_OFFSET+SPL_BYTES]
    result={"schema":"mpfree-private-spl-fdt-v1",
            "status":"offline verified owner backup only",
            "date_utc":datetime.now(timezone.utc).isoformat(),
            "p1_sha256":HASH,"sram_stage_sha256":hashlib.sha256(spl).hexdigest(),
            "sram_stage_bytes":len(spl),"dtbs":[],
            "limitations":["An embedded FDT does not prove the board uses each listed node.",
                           "DDR timings are board-specific and must not be applied to hardware without verification.",
                           "Cannot prove MaskROM or owner SPL support."]}
    offset=0
    while offset<len(spl)-8 and len(result["dtbs"])<12:
        found=spl.find(MAGIC,offset)
        if found<0:break
        size=struct.unpack(">I",spl[found+4:found+8])[0]
        if 40<=size<=len(spl)-found:
            candidate=spl[found:found+size]
            obj={"sram_offset":found,
                 "p1_partition_offset":SPL_OFFSET+found,
                 "bytes":size,"sha256":hashlib.sha256(candidate).hexdigest()}
            try:
                proc=subprocess.run(["dtc","-I","dtb","-O","dts","-"],
                                    input=candidate,stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE,check=False)
                if proc.returncode==0:
                    lines=proc.stdout.decode("utf-8","replace").splitlines()
                    # Extract boot-critical lines plus nearby node context,
                    # bounded to keep the JSON report reasonably sized.
                    selected=[]
                    for index,line in enumerate(lines):
                        if any(term in line.lower() for term in KEYWORDS):
                            selected.extend(lines[max(0,index-2):min(len(lines),index+5)])
                    # Preserve order but remove repeated lines from overlapping windows.
                    dedup=[]
                    seen=set()
                    for line in selected:
                        if line in seen:continue
                        seen.add(line)
                        dedup.append("[PRIVATE FIELD REDACTED]" if PRIVATE.search(line) else line)
                    obj["selected_lines"]=dedup[:750]
                    obj["total_dts_lines"]=len(lines)
                    obj["truncated"]=len(dedup)>750
                    obj["dtc_status"]="parsed"
                else:
                    obj["dtc_status"]="failed"
                    obj["dtc_stderr"]=proc.stderr.decode("utf-8","replace")[:1000]
            except FileNotFoundError:
                obj["dtc_status"]="not installed"
                obj["matching_binary_strings"]=[
                    m.group().decode("ascii","replace")[:160]
                    for m in re.finditer(rb"[\x20-\x7e]{5,}",candidate)
                    if any(k.encode() in m.group().lower() for k in KEYWORDS)
                ][:150]
            result["dtbs"].append(obj)
            offset=found+size
        else:
            offset=found+4
    if not result["dtbs"]:
        result["warning"]="No plausible embedded FDT found; not proof that SPL lacks board data."
    return result

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--backup",type=Path,required=True)
    ap.add_argument("--report",type=Path,required=True)
    args=ap.parse_args()
    if not args.report.is_absolute():ap.error("Use absolute report path")
    report=inspect(args.backup)
    args.report.parent.mkdir(parents=True,exist_ok=True)
    old=os.umask(0o077)
    try:args.report.write_text(json.dumps(report,indent=2)+"\n",encoding="utf-8")
    finally:os.umask(old)
    print("Embedded SPL FDT scan",args.report,"plausible DTBs:",len(report["dtbs"]))
    for fdt in report["dtbs"]:
        print(" SRAM offset 0x%x, %d bytes, dtc=%s" %
              (fdt["sram_offset"],fdt["bytes"],fdt.get("dtc_status")))

if __name__=="__main__":main()
