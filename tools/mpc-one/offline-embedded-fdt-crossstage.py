#!/usr/bin/env python3
"""Offline FDT inventory of both verified MPC One bootloader stages.

Reads only existing regular-file eMMC backup. Finds and validates FDT
containers in the SRAM SPL and U-Boot executable and extracts *selected*
boot-relevant nodes/parameters without writing any extracted vendor files.
Requires only Python 3; 'dtc' is optional for richer inspection.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import subprocess
from datetime import datetime, timezone

PART1_SHA="aa36e0e4033ce7386a210ad3e41f405a0d952a5b2a2c2711328450017414f1ec"
P3_PAYLOAD_SHA="b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a"
FDT=b"\xd0\x0d\xfe\xed"
TAGS=("memory","reserved-memory","chosen","u-boot","dmc","sdram",
      "pctl","phy-timing","ram","ddr","clock-controller","regulator",
      "saradc","recovery","gpio-keys","reboot-mode","efuse","otp",
      "usb@ff580000","usb@ff540000","mmc","emmc","dwmmc",
      "serial@ff690000","sram","secure","bootargs","pinctrl",
      "vop","panel","uart","rockchip,sdram-params","rockchip,phy-timing",
      "rockchip,pctl-timing","usb-phy","syscon","power-management")
SENSITIVE=re.compile(r"serial-number|local-mac-address|mac-address|partuuid|"
                     r"uuid|password|secret|token|credential|private.key|"
                     r"unlock",re.I)
def get_region(path,start,size):
    with path.open("rb") as f:
        f.seek(start)
        b=f.read(size)
    if len(b)!=size:raise ValueError("Truncated owner image")
    return b
def find_dtbs(blob,origin):
    result=[];cursor=0
    while cursor<len(blob)-40 and len(result)<20:
        at=blob.find(FDT,cursor)
        if at<0:break
        total=struct.unpack_from(">I",blob,at+4)[0]
        if not(40<=total<=min(2*1024*1024,len(blob)-at)):
            cursor=at+4;continue
        fragment=blob[at:at+total]
        record={"offset_in_"+origin:at,"bytes":total,
                "sha256":hashlib.sha256(fragment).hexdigest(),
                "magic":"0xd00dfeed"}
        if shutil.which("dtc"):
            p=subprocess.run(["dtc","-I","dtb","-O","dts","-"],
                             input=fragment,capture_output=True)
            record["dtc_success"]=p.returncode==0
            if p.returncode==0:
                dts=p.stdout.decode("utf-8","replace")
                lines=dts.splitlines()
                # Preserve hierarchy by including context before and after
                # interesting node/property lines while removing duplicates.
                indexes=set()
                for i,line in enumerate(lines):
                    if any(term in line.lower() for term in TAGS):
                        indexes.update(range(max(0,i-3),min(len(lines),i+7)))
                ordered=sorted(indexes)
                chosen=[]
                for i in ordered[:850]:
                    line=lines[i]
                    chosen.append("[IDENTIFIER REDACTED]" if SENSITIVE.search(line) else line)
                record["selected_dts_lines"]=chosen
                record["total_dts_lines"]=len(lines)
                record["selected_truncated"]=len(ordered)>850
                # Reboot-mode flags and memory geometry are summarized
                # as raw values without claiming any runtime interpretation.
                for name in ("rockchip,pctl-timing","rockchip,phy-timing",
                             "rockchip,sdram-params","mode-normal","mode-loader",
                             "mode-recovery","mode-bootloader","dr_mode",
                             "bootph-pre-ram","u-boot,dm-pre-reloc"):
                    record.setdefault("selected_properties",{})[name]=[
                        x.strip()[:450] for x in lines
                        if re.search(r"^\s*"+re.escape(name)+r"\s*(?:=|;)",x)
                    ][:18]
            else:
                record["dtc_error_excerpt"]=p.stderr.decode("utf-8","replace")[:250]
        result.append(record)
        cursor=at+total
    return result

def scan(path):
    if path.is_symlink() or not path.is_file() or not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError("Only regular-file offline backups are supported")
    p1=get_region(path,64*512,128*512)
    if hashlib.sha256(p1).hexdigest()!=PART1_SHA:raise ValueError("Wrong P1 hash")
    if p1[2048:2052]!=b"RK32":raise ValueError("Unexpected SPL envelope")
    spl=p1[2048:2048+32768]
    p3=get_region(path,256*512,768*512)
    if p3[:4]!=b"\x27\x05\x19\x56":raise ValueError("Wrong P3 magic")
    size=struct.unpack_from(">I",p3,12)[0]
    if not (0<size<=len(p3)-64):raise ValueError("Wrong P3 image size")
    uboot=p3[64:64+size]
    if hashlib.sha256(uboot).hexdigest()!=P3_PAYLOAD_SHA:raise ValueError("Wrong P3 hash")
    return {
       "schema":"mpfree-private-embedded-fdt-crossstage-v1",
       "date_utc":datetime.now(timezone.utc).isoformat(),
       "mode":"offline, source hash checked, no raw firmware exported",
       "spl_stage_fdt":find_dtbs(spl,"spl"),
       "uboot_stage_fdt":find_dtbs(uboot,"uboot"),
       "limitations":[
         "FDT text may refer to generic compatibles not active on this exact board.",
         "Linux DTB, U-Boot DTB and SPL DTB differ and should not be conflated.",
         "FDT configuration is not proof of the BootROM's internal behavior.",
         "Raw timing information must not be used for live DDR writes.",
       ]
    }
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backup",type=Path,required=True)
    parser.add_argument("--report",type=Path,required=True)
    args=parser.parse_args()
    if not args.report.is_absolute():parser.error("Report path must be absolute")
    results=scan(args.backup.expanduser())
    args.report.parent.mkdir(parents=True,exist_ok=True)
    old=os.umask(0o077)
    try:args.report.write_text(json.dumps(results,indent=2)+"\n",encoding="utf-8")
    finally:os.umask(old)
    print("SPL embedded DTBs:",len(results["spl_stage_fdt"]),
          "U-Boot embedded DTBs:",len(results["uboot_stage_fdt"]))
    print("PRIVATE REPORT",args.report)
if __name__=="__main__":main()
