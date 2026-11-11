#!/usr/bin/env python3
"""Read-only 3.6/3.10 boot configuration and userspace security provenance audit.

Inputs: verified regular-file owner eMMC backup plus already extracted,
regular-file 3.10 rootfs. Uses debugfs, dtc, readelf, objdump, unsquashfs
ONLY as passive parsers. Never mounts, executes, flashes or connects to MPC.

Outputs private JSON with normalized boot-script comparison, board DTB
property changes, ELF dynamic-linkage and selected disassembly. Do not
publish generated report (contains proprietary code and possibly secrets).
"""
import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import subprocess
import tempfile
from datetime import datetime, timezone
P1_SHA="aa36e0e4033ce7386a210ad3e41f405a0d952a5b2a2c2711328450017414f1ec"
P6_START=17408*512
P6_LENGTH=1024000*512
BOARD_DTBS=("/boot/rk3288-az01-acva.dtb","/boot/rk3288-az01-acva-c.dtb")
ELFS=(
 "/usr/bin/az01-bootloader-sig", "/usr/sbin/az01-update",
 "/usr/lib/libaz01-prog.so.1.3", "/usr/sbin/az0x-loadpin-trustpoints",
 "/usr/bin/az01-launch-MPC", "/usr/bin/az01-script-runner",
 "/usr/sbin/az01config",
)
SERVICES=(
 "/usr/lib/systemd/system/az0x-loadpin-trustpoints.service",
 "/usr/lib/systemd/system/az0x-info.service",
 "/usr/lib/systemd/system/acvs.service",
 "/usr/lib/systemd/system/az01-script-runner.service",
 "/usr/lib/systemd/system/touch-fw-update.service",
 "/usr/lib/systemd/system/xmos-update.service",
)
FUNCTIONS=(
 "az01_verify_image","az01_verify_signature","az01_write_bootloader",
 "az01_commit_bootloader","az01_write_partition","az01_write_partition_image",
 "az01_usb_update_mode","az01_sign_update","az01_erase_partition",
 "az01_device_get_serial","az01_image_for_each_signature",
)
DTB_KEYWORDS=("reboot-mode","dwmmc@","usb@ff540000","usb@ff580000",
              "saradc@ff100000","efuse","otp","reserved-memory","chosen",
              "memory@","pinctrl","gpio-keys","serial@ff690000")
PRIVATE=re.compile(r"(?:serial-number|mac-address|password|secret|token|"
                   r"unlock|private.key|ethaddr|serialno|partuuid|uuid=)",re.I)

def cmd(args,**kwargs):
    return subprocess.run(args,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                          timeout=35,check=False,**kwargs)
def verify_regular(path):
    path=Path(path).expanduser()
    if path.is_symlink() or not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError("Input must be an existing regular file, not a symlink or block device: "+str(path))
    return path
def debugfs_cat(img,path):
    p=cmd(["debugfs","-R","cat "+path,str(img)])
    if p.returncode or b"File not found" in p.stderr or b"not found by ext2_lookup" in p.stderr:
        return None
    return p.stdout
def sha(b):return hashlib.sha256(b).hexdigest()
def sanitized(s):
    return "\n".join("[IDENTIFIER OR SECRET REDACTED]" if PRIVATE.search(x) else x
                     for x in s.splitlines())
def boot_script(raw):
    if not raw:return None
    data=raw
    hdr={"container":"plain"}
    if raw[:4]==b"\x27\x05\x19\x56" and len(raw)>=64:
        expected=struct.unpack_from(">I",raw,12)[0]
        if expected>len(raw)-64:raise ValueError("U-Boot script length invalid")
        hdr={"container":"legacy uImage","payload_size":expected,
             "header_sha256":sha(raw[:64]),"image_bytes":len(raw)}
        data=raw[64:64+expected]
        # U-Boot legacy script uses a multi-image table: [length][zero]
        # followed by the script bytes. Never include those bytes in text.
        if len(data)>=8:
            script_len,zero=struct.unpack_from(">II",data,0)
            if zero==0 and 0<script_len<=len(data)-8:
                data=data[8:8+script_len]
                hdr["multi_image_script_length"]=script_len
    script=data.decode("utf-8","replace")
    return {"header":hdr,"sha256":sha(data),
            "normalized_sha256":sha(script.rstrip("\r\n").encode()),
            "commands":sanitized(script[:20000]),
            "truncated":len(script)>20000}
def parse_dtb(raw):
    if not raw or raw[:4]!=b"\xd0\x0d\xfe\xed":return {"status":"missing or not FDT"}
    if not shutil.which("dtc"):return {"status":"dtc missing"}
    p=cmd(["dtc","-I","dtb","-O","dts","-"],input=raw)
    if p.returncode:return {"status":"dtc failed"}
    # Preserve hierarchy by maintaining braces stack. Filter properties
    # to boot-relevant nodes only, ignore volatile phandle numbers for
    # printed diffs but preserve SHA for actual binary-level change.
    nodes={};stack=[]
    for line in p.stdout.decode("utf-8","replace").splitlines():
        t=line.strip()
        if t.endswith("{"):
            name=t[:-1].strip()
            stack.append(name)
            continue
        if t.startswith("}"):
            if stack:stack.pop()
            continue
        if not t or not t.endswith(";") or not stack:continue
        path="/"+"/".join(stack)
        if not any(k in path.lower() for k in DTB_KEYWORDS):continue
        if t.startswith(("phandle =","linux,phandle =")):continue
        if PRIVATE.search(t):continue
        nodes.setdefault(path,[]).append(t[:700])
    return {"status":"parsed","sha256":sha(raw),"nodes":nodes,
            "boot_related_node_count":len(nodes)}
def diff_dtb(before,after):
    a=before.get("nodes",{});b=after.get("nodes",{})
    result={}
    for node in sorted(set(a)|set(b)):
        first=a.get(node,[]);second=b.get(node,[])
        if first==second:continue
        result[node]={"before":first[:65],"after":second[:65],
                      "before_truncated":len(first)>65,
                      "after_truncated":len(second)>65}
    return {"before_sha":before.get("sha256"),"after_sha":after.get("sha256"),
            "changed_boot_nodes":result,
            "nodes_changed":len(result),
            "warning":"Numerical phandle references can differ without a functional difference; changed properties require review."}
def get_elf_details(binary,td):
    if not binary or not binary.startswith(b"\x7fELF"):
        return {"present":bool(binary),"elf":False}
    path=Path(td)/"analysis-ELF"
    path.write_bytes(binary)
    details={"present":True,"elf":True,"bytes":len(binary),"sha256":sha(binary)}
    if shutil.which("readelf"):
        syms=cmd(["readelf","-Ws",str(path)]).stdout.decode("utf-8","replace")
        named=[row.strip()[:230] for row in syms.splitlines()
               if any(n in row for n in FUNCTIONS)
               or any(n in row for n in ("EVP_DigestVerify","PEM_read_PUBKEY",
                                        "SHA256_Update","az01_erase_partition",
                                        "az01_usb_update_mode","kexec"))]
        details["selected_symbols"]=named[:140]
        details["symbols_truncated"]=len(named)>140
    if shutil.which("objdump"):
        details["function_disassembly"]={}
        for name in FUNCTIONS:
            p=cmd(["objdump","-d","--no-show-raw-insn",
                   "--disassemble="+name,str(path)])
            t=p.stdout.decode("utf-8","replace")
            # Empty objdump output often prints only section headings.
            lines=[x for x in t.splitlines() if x.strip().endswith(">:") or
                   re.search(r":\s+[a-z][a-z0-9.]*\s",x)]
            if not lines:continue
            details["function_disassembly"][name]={"lines":lines[:190],
                                                  "truncated":len(lines)>190}
        # For stripped standalone ELF, imports and GOT calls can locate the
        # relevant hash/partition paths; a full Ghidra import is later possible.
        reloc=cmd(["objdump","-R",str(path)]).stdout.decode("utf-8","replace")
        details["selected_dynamic_relocations"]=[
            row[:180] for row in reloc.splitlines()
            if any(term in row for term in ("SHA","EVP_","az01_","PEM_",
                                           "reboot","kexec","fopen","open64"))
        ][:130]
    return details

def inspect(backup,rootfs310):
    if not shutil.which("debugfs"):raise RuntimeError("debugfs required")
    backup=verify_regular(backup); rootfs310=verify_regular(rootfs310)
    with backup.open("rb") as fh:
        fh.seek(64*512)
        if sha(fh.read(128*512))!=P1_SHA:raise ValueError("eMMC backup P1 hash mismatch")
    with rootfs310.open("rb") as fh:
        fh.seek(1080)
        if fh.read(2)!=b"\x53\xef":raise ValueError("3.10 rootfs not ext4")
    result={"schema":"mpfree-bootchain-v4-cross-version-security-v1",
            "date_utc":datetime.now(timezone.utc).isoformat(),
            "mode":"owner backup and 3.10 rootfs, both read-only; temp P6 regular-file copy",
            "boot_script":{},"board_dtbs":{},"elfs":{},"services":{},"limitations":[
                "Do not infer signature enforcement from imported OpenSSL functions.",
                "Userspace bootloader integrity utilities do not establish silicon secure boot.",
                "Boot-command text equivalence does not prove kernel ABI or DTB compatibility.",
                "Bootloader IDs/partition numbers found in ELF strings need path-level tracing.",
                "Never commit this private report or its proprietary disassembly.",
            ]}
    with tempfile.TemporaryDirectory(prefix="mpfree-v4-p6-") as td:
        p6=Path(td)/"factory-p6.ext4"
        with backup.open("rb") as src,p6.open("wb") as sink:
            src.seek(P6_START)
            remaining=P6_LENGTH
            while remaining:
                part=src.read(min(8*1024*1024,remaining))
                if not part:raise ValueError("Truncated P6")
                sink.write(part);remaining-=len(part)
        a=boot_script(debugfs_cat(p6,"/boot/boot.scr.uimg"))
        b=boot_script(debugfs_cat(rootfs310,"/boot/boot.scr.uimg"))
        commands_a=(a or {}).get("commands","")
        commands_b=(b or {}).get("commands","")
        result["boot_script"]={"factory":a,"firmware310":b,
           "normalized_commands_equal":(
               bool(a and b) and a["normalized_sha256"]==b["normalized_sha256"]),
           "command_diff":list(difflib.unified_diff(
               commands_a.splitlines(),commands_b.splitlines(),
               fromfile="factory-3.6",tofile="firmware-3.10",lineterm=""))[:150]}
        for name in BOARD_DTBS:
            a=parse_dtb(debugfs_cat(p6,name))
            b=parse_dtb(debugfs_cat(rootfs310,name))
            result["board_dtbs"][name]={"factory":{k:v for k,v in a.items() if k!="nodes"},
                                       "firmware310":{k:v for k,v in b.items() if k!="nodes"},
                                       "boot_node_diff":diff_dtb(a,b)}
        for name in ELFS:
            result["elfs"][name]={}
            for tag,fs in (("factory",p6),("firmware310",rootfs310)):
                b=debugfs_cat(fs,name)
                result["elfs"][name][tag]=get_elf_details(b,td)
        for name in SERVICES:
            result["services"][name]={}
            for tag,fs in (("factory",p6),("firmware310",rootfs310)):
                b=debugfs_cat(fs,name)
                result["services"][name][tag]={"present":b is not None,
                  "sha256":sha(b) if b else None,
                  "contents":sanitized(b.decode("utf-8","replace"))[:7000] if b else None}
        # The 3.10 update filesystem contains a ramdisk update service,
        # whose file inventory can reveal optional partition writes.
        updater=debugfs_cat(rootfs310,"/opt/ramfs-update.squashfs")
        if updater and shutil.which("unsquashfs"):
            raw=Path(td)/"ramfs-update.squashfs";raw.write_bytes(updater)
            p=cmd(["unsquashfs","-ll",str(raw)])
            lines=p.stdout.decode("utf-8","replace").splitlines()
            result["ramfs_updater"]={"present":True,"bytes":len(updater),
                "sha256":sha(updater),"file_list":[x[:280] for x in lines[:400]],
                "files_truncated":len(lines)>400,
                "warning":"Only listings, not execution, signature validation or a writing operation."}
        else:
            result["ramfs_updater"]={"present":bool(updater),
                "status":"unsquashfs unavailable or image absent"}
    return result

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backup",type=Path,required=True)
    parser.add_argument("--rootfs-310",type=Path,required=True)
    parser.add_argument("--report",type=Path,required=True)
    args=parser.parse_args()
    if not args.report.is_absolute():parser.error("Absolute output path required")
    data=inspect(args.backup,args.rootfs_310)
    args.report.parent.mkdir(parents=True,exist_ok=True)
    old=os.umask(0o077)
    try:args.report.write_text(json.dumps(data,indent=2)+"\n",encoding="utf-8")
    finally:os.umask(old)
    print("v4 comparison report:",args.report)
    print("Normalized boot scripts equal:",data["boot_script"]["normalized_commands_equal"])
    for k,v in data["board_dtbs"].items():
        print(k,"changed boot-related DTB nodes:",v["boot_node_diff"]["nodes_changed"])
if __name__=="__main__":main()
