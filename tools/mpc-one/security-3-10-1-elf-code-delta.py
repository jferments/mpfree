#!/usr/bin/env python3
"""MPFree 3.10.1 offline ELF section/security code comparison.

Works against the already-extracted 3.10.0/3.10.1 raw ext4 files in a
firmware-lab directory. Runs debugfs in read-only mode and host-side readelf
and ARM objdump only. No MPC access, mounts, QEMU boots or ARM execution.
All extracted firmware remains private. Only small reports are ZIPped.
"""
from __future__ import annotations
import argparse
import datetime as dt
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
import sys
import zipfile

ROOT = Path.home() / "mpfree-work" / "firmware-labs"
VERSIONS = ("3.10.0", "3.10.1")
FILES = (
    "/usr/bin/az01-bootloader-sig",
    "/usr/bin/az01-signed-fs",
    "/usr/sbin/az0x-loadpin-trustpoints",
    "/usr/bin/az01-image",
    "/usr/sbin/az01-update",
    "/usr/lib/libaz01-prog.so.1.3",
)

def digest(buf):
    return hashlib.sha256(buf).hexdigest()

def run(argv, timeout=180):
    return subprocess.run([str(a) for a in argv], capture_output=True,
                          text=True, errors="replace", timeout=timeout)

def check_lab(path):
    p = Path(path).expanduser()
    if p.is_symlink() or not p.is_dir() or p.resolve().parent != ROOT.resolve():
        raise ValueError("lab must be a real directory directly beneath " + str(ROOT))
    p = p.resolve()
    fs = {}
    for ver in VERSIONS:
        report = p / "reports" / (ver + "-rootfs-path.txt")
        if not report.is_file() or report.is_symlink():
            raise ValueError("missing rootfs path report for " + ver)
        f = Path(report.read_text().strip())
        if f.is_symlink() or not f.is_file() or not stat.S_ISREG(f.lstat().st_mode):
            raise ValueError("not a regular rootfs: " + str(f))
        if p not in f.resolve().parents:
            raise ValueError("rootfs lies outside private lab: " + str(f))
        with f.open("rb") as stream:
            stream.seek(1080)
            if stream.read(2) != b"\x53\xef":
                raise ValueError("missing ext4 magic in " + str(f))
        fs[ver] = f
    return p, fs

def dump_one(rootfs, path, dest):
    st = run(["debugfs", "-R", "stat " + path, rootfs], timeout=45)
    if "Type: regular" not in st.stdout:
        return False
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.exists() or dest.is_symlink():
        raise ValueError("refusing to replace scratch file " + str(dest))
    r = run(["debugfs", "-R", "dump " + path + " " + str(dest), rootfs], timeout=120)
    if r.returncode or not dest.is_file() or dest.stat().st_size == 0:
        dest.unlink(missing_ok=True)
        raise RuntimeError("failed to dump " + path + ": " + r.stderr[:150])
    if not stat.S_ISREG(dest.lstat().st_mode):
        raise ValueError("unexpected extracted special file " + str(dest))
    return True

def sections(data):
    """ELF32 little/big endian parser: mapped sections, code/data and notes."""
    if data[:4] != b"\x7fELF" or data[4] not in (1,2):
        raise ValueError("Not a supported ELF32/ELF64 file")
    bitness = data[4]
    order = "<" if data[5] == 1 else ">" if data[5] == 2 else None
    if order is None:
        raise ValueError("Unrecognized ELF endian")
    if bitness == 1:
        hfmt = order + "16sHHIIIIIHHHHHH"
        sfmt = order + "IIIIIIIIII"
        _, _, machine, _, _, _, shoff, _, _, _, _, shentsize, shnum, stridx = struct.unpack_from(hfmt, data)
    else:
        hfmt = order + "16sHHIQQQIHHHHHH"
        sfmt = order + "IIQQQQIIQQ"
        _, _, machine, _, _, _, shoff, _, _, _, _, shentsize, shnum, stridx = struct.unpack_from(hfmt, data)
    size = struct.calcsize(sfmt)
    if shnum == 0 or shnum > 20000 or shentsize < size or shoff + shnum*shentsize > len(data):
        raise ValueError("ELF section table missing, malformed or unsupported extended indexing")
    raw = [struct.unpack_from(sfmt, data, shoff+i*shentsize) for i in range(shnum)]
    if stridx >= len(raw):
        raise ValueError("bad ELF section string index")
    if bitness == 1:
        names_off,names_size = raw[stridx][4],raw[stridx][5]
    else:
        names_off,names_size = raw[stridx][4],raw[stridx][5]
    names = data[names_off:names_off+names_size]
    out = []
    for row in raw:
        idx, sh_type, flags, vaddr, offset, n, _, _, _, _ = row
        at = names.find(b"\0",idx)
        name = names[idx:at if at >= 0 else None].decode("utf-8", "replace") if idx < len(names) else "(unnamed)"
        if sh_type == 8:  # SHT_NOBITS; not stored in ELF file
            content = b""
        elif offset <= len(data) and n <= len(data)-offset:
            content = data[offset:offset+n]
        else:
            raise ValueError("section offset outside file: " + name)
        out.append({"name":name,"offset":offset,"bytes":n,"address":vaddr,
                    "flags":hex(flags),"type":sh_type,"sha256":digest(content),
                    "executable":bool(flags & 4),"allocated":bool(flags & 2)})
    return {"class":32 if bitness == 1 else 64,"machine":machine,"sections":out}

def changed_offsets(old, new):
    spans=[]; start=None; n=min(len(old),len(new)); differing=0
    for i in range(n):
        if old[i]!=new[i]:
            differing+=1
            if start is None:start=i
        elif start is not None:
            spans.append((start,i));start=None
    if start is not None:spans.append((start,n))
    if len(old)!=len(new):spans.append((n,max(len(old),len(new))))
    return spans,differing

def disassemble(path, objdump):
    if not objdump:
        return None, "ARM-capable objdump missing (install binutils-arm-linux-gnueabihf)"
    r = run([objdump, "-d", "-w", str(path)], timeout=240)
    if r.returncode:
        return None,"objdump failed: " + r.stderr[:350]
    # Preserve only disassembled mnemonic and operands, rather than full firmware code.
    # This is a rough structural heuristic; addresses and relocations can differ.
    result=[]
    for line in r.stdout.splitlines():
        m=re.match(r"^\s*[0-9a-f]+:\s+(?:(?:[0-9a-f]{2,8}\s+)+)\s*([A-Za-z][\w.]*)\s*(.*)$",line)
        if m:
            mnemonic, ops=m.groups()
            ops=re.sub(r"\s+", " ",ops.split(";",1)[0].strip())
            ops=re.sub(r"\b0x[0-9a-fA-F]+\b","IMM",ops)
            ops=re.sub(r"(?<![A-Za-z0-9_])\d{3,}(?![A-Za-z0-9_])","N",ops)
            result.append(mnemonic+" "+ops)
    return result,None

def compare_one(a,b,objdump):
    aa=a.read_bytes();bb=b.read_bytes()
    spans,n_diff=changed_offsets(aa,bb)
    info={"bytes_a":len(aa),"bytes_b":len(bb),"hash_a":digest(aa),"hash_b":digest(bb),
          "different_bytes":n_diff,"change_spans":len(spans),"first_changed_offsets":[[x,y] for x,y in spans[:40]]}
    ea,eb=sections(aa),sections(bb)
    info["elf_machine"]=[ea["machine"],eb["machine"]]
    sa={x["name"]:x for x in ea["sections"] if x["name"]}
    sb={x["name"]:x for x in eb["sections"] if x["name"]}
    rows=[]
    for name in sorted(set(sa)|set(sb)):
        x,y=sa.get(name),sb.get(name)
        state="same" if x and y and x["sha256"]==y["sha256"] and x["bytes"]==y["bytes"] else "changed" if x and y else "missing"
        rows.append({"section":name,"state":state,
                     "old_bytes":x["bytes"] if x else None,"new_bytes":y["bytes"] if y else None,
                     "old_offset":x["offset"] if x else None,"new_offset":y["offset"] if y else None,
                     "executable":bool((x or y).get("executable")),
                     "allocated":bool((x or y).get("allocated"))})
    info["sections"]=rows
    info["changed_executable_sections"]=[r["section"] for r in rows if r["executable"] and r["state"]!="same"]
    old_sig, e1=disassemble(a,objdump);new_sig,e2=disassemble(b,objdump)
    if old_sig is not None and new_sig is not None:
        if old_sig == new_sig:
            ratio, edits = 1.0, []
        elif max(len(old_sig),len(new_sig)) > 12000:
            ratio, edits = None, []  # avoid quadratic diff on a large firmware library
        else:
            matcher=difflib.SequenceMatcher(None,old_sig,new_sig,autojunk=True)
            edits=[{"kind":tag,"old_instruction_indexes":[i,j],"new_instruction_indexes":[k,l]} for tag,i,j,k,l in matcher.get_opcodes() if tag!="equal"]
            ratio=round(matcher.ratio(),4)
        info["disassembly"]={"old_parsed_instructions":len(old_sig),"new_parsed_instructions":len(new_sig),
                              "matching_ratio_heuristic":ratio,
                              "change_hunks":len(edits) if ratio is not None else None,
                              "first_changes":edits[:30],
                              "caveat":"Addresses and immediate constants are normalized. This is NOT a semantic equivalence proof; large changed disassemblies may skip quadratic alignment."}
    else:
        info["disassembly"]={"status":"not available","errors":[e1,e2]}
    info["first_change_section_mappings"]=[]
    for start,end in spans[:30]:
        old_sections=[s["name"] for s in ea["sections"] if s["type"]!=8 and s["bytes"]>0 and s["offset"]<=start<s["offset"]+s["bytes"]]
        new_sections=[s["name"] for s in eb["sections"] if s["type"]!=8 and s["bytes"]>0 and s["offset"]<=start<s["offset"]+s["bytes"]]
        info["first_change_section_mappings"].append({"range":[start,end],"old":old_sections,"new":new_sections})
    return info

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lab",required=True,type=Path)
    args=ap.parse_args()
    os.umask(0o077)
    if not shutil.which("debugfs"):
        ap.error("Missing debugfs (e2fsprogs)")
    objdump=shutil.which("arm-linux-gnueabihf-objdump") or shutil.which("llvm-objdump")
    lab,fs=check_lab(args.lab)
    stamp=dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    suffix=stamp+"-"+str(os.getpid())
    scratch=lab/"artifacts"/("elf-code-delta-"+suffix)
    reports=lab/"reports"/("elf-code-delta-"+suffix)
    scratch.mkdir(mode=0o700)
    reports.mkdir(mode=0o700)
    findings={};warnings=[]
    for rel in FILES:
        name=Path(rel).name
        print("Comparing",rel,flush=True)
        files={}
        for ver in VERSIONS:
            dest=scratch/ver/rel.lstrip("/")
            if dump_one(fs[ver],rel,dest):files[ver]=dest
            else:warnings.append(ver+" missing "+rel)
        if len(files)==2:
            try:
                findings[rel]=compare_one(files[VERSIONS[0]],files[VERSIONS[1]],objdump)
                (reports/(name+"-code-delta.json")).write_text(json.dumps(findings[rel],indent=2)+"\n")
            except (ValueError,struct.error) as exc:
                warnings.append(rel+": "+str(exc))
    summary=["MPFree 3.10.1 vs 3.10.0 ELF SECURITY CODE DELTA", "All analysis offline; no MPC access or firmware execution.",""]
    for rel,j in findings.items():
        changes=[x["section"] for x in j["sections"] if x["state"]!="same"]
        summary.append(f"{rel}: {j['different_bytes']} different bytes / {j['bytes_a']} original bytes")
        summary.append("  changed executable sections: "+(", ".join(j["changed_executable_sections"]) or "NONE"))
        summary.append("  all changed sections: "+(", ".join(changes) or "NONE"))
        dis=j["disassembly"]
        summary.append("  disassembly: "+(f"{dis['change_hunks']} normalized difference hunks; similarity {dis['matching_ratio_heuristic']}" if dis.get("matching_ratio_heuristic") is not None else "large disassembly alignment skipped or unavailable"))
    summary.extend(["","LIMITATIONS:","- Equal section hashes do not guarantee equal boot behavior or installed bootloader state.",
                    "- Normalized objdump is a heuristic, not proof of equivalent verification logic.",
                    "- This compares rootfs ELF files, not raw bootloader partitions or verified signatures.",
                    "- Analyze changed executable sections in Ghidra before deciding to flash.",""])
    (reports/"SUMMARY.txt").write_text("\n".join(summary))
    (reports/"warnings.txt").write_text("\n".join(warnings)+"\n")
    archive=lab/("mpc-gen1-3101-elf-code-delta-"+stamp+".zip")
    if archive.exists():raise FileExistsError(archive)
    with zipfile.ZipFile(archive,"x",compression=zipfile.ZIP_DEFLATED,compresslevel=6) as z:
        for f in sorted(reports.iterdir()):
            if not f.is_file() or f.is_symlink() or f.suffix not in (".json",".txt"):
                raise ValueError("unsafe report " + str(f))
            z.write(f,f.name)
    print("\n".join(summary))
    print("PRIVATE ZIP TO UPLOAD:",archive)
    print("Local extracted ELF files (PRIVATE, Ghidra inputs):",scratch)
    return 0

if __name__=="__main__":
    try:sys.exit(main())
    except (ValueError, RuntimeError, OSError, subprocess.TimeoutExpired, struct.error) as e:
        print("ERROR:",e,file=sys.stderr);sys.exit(1)
