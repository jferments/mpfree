#!/usr/bin/env python3
"""Offline, read-only 3.6-versus-3.10 MPC One boot-related rootfs audit.

Only reads existing local *regular files*. No mounting, loop devices,
MPC access, firmware execution, or write access to any source image.
Stores PRIVATE findings in an owner-only JSON report.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import struct
import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path

FILES = (
 "/boot/boot.scr.uimg",
 "/boot/rk3288-az01-acva.dtb",
 "/boot/rk3288-az01-acva-c.dtb",
 "/usr/bin/az01-bootloader-sig",
 "/usr/sbin/az01config",
 "/usr/sbin/az01-update",
 "/usr/bin/az01-launch-MPC",
 "/usr/bin/az01-script-runner",
 "/usr/lib/libaz01-prog.so.1.3",
 "/opt/ramfs-update.squashfs",
 "/etc/fw_env.config",
 "/usr/lib/systemd/system/az0x-loadpin-trustpoints.service",
 "/usr/lib/systemd/system/az0x-info.service",
 "/usr/lib/systemd/system/acvs.service",
 "/lib/systemd/system/az0x-loadpin-trustpoints.service",
)
PATTERNS=("reboot","loader","rom","efuse","otp","bootloader","signature",
          "verify","fastboot","kexec","secure","bootmode","mmc","usb",
          "serial","gpio","recovery","fdt","update","flash","trustpoint",
          "pubkey","keyhash","rootfs","partition")
DT_NAMES=("reboot-mode","usb@ff580000","usb@ff540000","serial@ff690000",
          "gpio-keys","recovery","adc","efuse","otp","chosen","memory@",
          "reserved-memory","rockchip,boot-mode","pinctrl","sdmmc",
          "sdhci","dwmmc@ff0f0000","dwmmc@ff0c0000")
PRIVATE=re.compile(r"serialno|serial#|ethaddr|macaddr|partuuid|uuid=|"
                   r"unlock|password|passphrase|token|private.key|"
                   r"credential|secret",re.I)

def read_file(rootfs,path):
    p=subprocess.run(["debugfs","-R","cat "+path,str(rootfs)],
                     capture_output=True)
    err=p.stderr.decode("utf-8","replace")
    if p.returncode or "File not found" in err or "not found by ext2_lookup" in err:
        return None
    return p.stdout

def scripts(data):
    if data[:4]==b"\x27\x05\x19\x56":
        size=struct.unpack_from(">I",data,12)[0]
        if not (64 <= size+64 <= len(data)):
            return {"error":"invalid legacy-script header length"}
        data=data[64:64+size]
    text=data.decode("utf-8","replace")
    text=re.sub(r"(?i)(?:PART)?UUID=[^ \t\n]+","[REDACTED_IDENTIFIER]",text)
    text="\n".join("[REDACTED]" if PRIVATE.search(row) else row
                   for row in text.splitlines())
    return {"script":text[:12000],"truncated":len(text)>12000}

def fdt_nodes(data):
    if not shutil.which("dtc"):
        return {"status":"dtc unavailable", "strings": [
            s.decode("ascii","replace")[:90] for s in re.findall(rb"[\x20-\x7e]{5,}",data)
            if any(k.encode() in s.lower() for k in DT_NAMES)
        ][:80]}
    with tempfile.NamedTemporaryFile(suffix=".dtb") as t:
        t.write(data);t.flush()
        p=subprocess.run(["dtc","-I","dtb","-O","dts",t.name],capture_output=True)
    if p.returncode:
        return {"status":"failed","dtc_stderr":p.stderr.decode("utf-8","replace")[:500]}
    lines=p.stdout.decode("utf-8","replace").splitlines()
    sections=[]
    for i,row in enumerate(lines):
        if not (row.strip().endswith("{") and any(k in row.lower() for k in DT_NAMES)):
            continue
        depth=0
        segment=[]
        for piece in lines[i:i+250]:
            depth+=piece.count("{")-piece.count("}")
            segment.append(piece)
            if depth==0:break
        text="\n".join(segment)
        if PRIVATE.search(text):
            text="\n".join("[REDACTED]" if PRIVATE.search(line) else line for line in segment)
        sections.append({"node_heading":row.strip(),"dts":text[:16000],
                         "truncated":len(text)>16000})
        if len(sections)>=35:break
    return {"status":"parsed", "selected_nodes":sections}

def candidate_strings(data):
    matches=[]
    for m in re.finditer(rb"[\x20-\x7e]{5,}",data):
        line=m.group().decode("ascii","replace")
        if not any(word in line.lower() for word in PATTERNS):continue
        matches.append({"offset":m.start(),"text":
            "[REDACTED_SENSITIVE_LITERAL]" if PRIVATE.search(line) else line[:160]})
        if len(matches)>=120:break
    return {"matches":matches,"truncated":len(matches)>=120}

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--rootfs-310",type=Path,required=True)
    ap.add_argument("--baseline-report",type=Path,required=True)
    ap.add_argument("--report",type=Path,required=True)
    args=ap.parse_args()
    rootfs=args.rootfs_310.expanduser()
    report=args.report.expanduser()
    if not report.is_absolute():ap.error("Report must be an absolute path")
    if not rootfs.is_file() or not stat.S_ISREG(rootfs.lstat().st_mode):
        ap.error("Rootfs must be a regular file, not a device or symlink")
    if not shutil.which("debugfs"):ap.error("debugfs missing; install e2fsprogs")
    with rootfs.open("rb") as src:
        src.seek(1080)
        if src.read(2)!=b"\x53\xef":ap.error("Not an ext2/3/4 root filesystem")
    baseline=json.loads(args.baseline_report.read_text(encoding="utf-8"))
    old_dtb=baseline.get("rootfs",{}).get("device_trees",{})
    old_boot=baseline.get("rootfs",{}).get("boot_scripts",{})
    old_bin=baseline.get("rootfs",{}).get("startup_and_update_binaries",{})
    out={"schema":"mpfree-offline-310-boot-audit-v1",
         "date_utc":datetime.now(timezone.utc).isoformat(),
         "mode":"existing image read-only via debugfs",
         "source_size":rootfs.stat().st_size,
         "sources":"Private workstation images; no original path or bytes uploaded",
         "files":{}, "scope_limits":[
            "Comparing 3.6 installed rootfs to a distinct 3.10 extracted firmware rootfs.",
            "U-Boot P1/P3 is not present in this 3.10 firmware update.",
            "Binary strings are clues, not execution proof.",
            "Kernel device tree and U-Boot runtime device tree can differ.",
            "No hardware operations or firmware execution.",
         ]}
    for path in FILES:
        b=read_file(rootfs,path)
        if b is None:
            out["files"][path]={"present":False}
            continue
        sha=hashlib.sha256(b).hexdigest()
        old=(old_dtb.get(path) or old_boot.get(path) or old_bin.get(path) or {})
        record={"present":True, "size":len(b), "sha256":sha,
                "baseline_sha256":old.get("sha256"),
                "changed_from_baseline":sha!=old["sha256"] if old.get("sha256") else None}
        if path.endswith(".dtb"):record["device_tree"]=fdt_nodes(b)
        elif path.endswith(".uimg"):record["boot_script"]=scripts(b)
        elif b.startswith(b"\x7fELF"):record["ELF"]=True;record["string_leads"]=candidate_strings(b)
        elif path.endswith(".service") or path.endswith("fw_env.config"):
            text=b.decode("utf-8","replace")
            record["redacted_text"]="\n".join(
                "[REDACTED]" if PRIVATE.search(line) else line
                for line in text.splitlines())[:16000]
        out["files"][path]=record
    report.parent.mkdir(parents=True,exist_ok=True)
    old=os.umask(0o077)
    try:report.write_text(json.dumps(out,indent=2)+"\n",encoding="utf-8")
    finally:os.umask(old)
    changed=[k for k,v in out["files"].items() if v.get("changed_from_baseline")]
    print("MPFree 3.10 ROOTFS BOOT AUDIT:",report,
          "; files inspected=",len(out["files"]),"; changed vs baseline=",len(changed))
if __name__=="__main__":main()
