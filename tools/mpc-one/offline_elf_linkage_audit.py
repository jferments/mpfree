#!/usr/bin/env python3
"""Parse static ARM ELF import/export/relocation metadata from verified bytes.

Uses GNU binutils readelf as a DATA parser, never invokes vendor ELF.
The private verified input is created as a temporary, read-only file,
and deleted before this function returns. Safe output is bounded
to az01_* symbols and library names; no vendor binaries are published.
"""
from __future__ import annotations
import hashlib
import json
import re
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

SCHEMA="mpfree-arm-elf-dynamic-linkage-v1"
TARGETS=("az01_usb_update_mode","az01_apply_image","az01_download_image")
SYMBOL=re.compile(r"\baz01_[A-Za-z0-9_]+\b")
NEEDED=re.compile(r"Shared library:\s*\[([^\]]{1,128})\]")
MAX_OUTPUT=12*1024*1024

def validate_elf(blob:bytes,expected_sha:str)->str:
    if (not isinstance(blob,bytes) or not 64<=len(blob)<=64*1024*1024):
        raise ValueError("Expected bounded private ARM ELF bytes")
    if hashlib.sha256(blob).hexdigest()!=expected_sha:
        raise ValueError("Original ELF SHA-256 does not match authenticated source")
    if (blob[:4]!=b"\x7fELF" or blob[4]!=1 or blob[5]!=1
        or struct.unpack_from("<H",blob,18)[0]!=40):
        raise ValueError("Expected ARM ELF32 little-endian, not arbitrary executable")
    return expected_sha

def parse_readelf(text:str)->dict:
    if not isinstance(text,str) or len(text)>MAX_OUTPUT:
        raise ValueError("Unexpected unbounded readelf output")
    imports=set()
    exports=set()
    relocations=[]
    needed=set()
    for line in text.splitlines():
        m=NEEDED.search(line)
        if m:
            needed.add(m.group(1))
        symbols=SYMBOL.findall(line)
        if not symbols:
            continue
        if " R_ARM_" in line:
            # Preserve only symbol/offset/type, not unrelated vendor details.
            cols=line.strip().split()
            if len(cols)>=3 and re.fullmatch(r"[0-9a-fA-F]+",cols[0]):
                relocations.append({
                    "offset":"0x"+cols[0].lower().rjust(8,"0"),
                    "relocation_type":next((v for v in cols if v.startswith("R_ARM_")),None),
                    "symbols":sorted(set(symbols)),
                })
        elif re.match(r"^\s*\d+:\s",line):
            if re.search(r"\sUND\s",line):
                imports.update(symbols)
            else:
                exports.update(symbols)
    return {
        "needed_sonames":sorted(needed),
        "az01_import_symbols":sorted(imports),
        "az01_export_symbols":sorted(exports),
        "az01_relocations":relocations[:120],
        "az01_relocations_truncated":len(relocations)>120,
        "target_symbol_evidence":{
            name:{
                "undefined_import":name in imports,
                "defined_export":name in exports,
                "relocation_offsets":[row["offset"] for row in relocations
                                      if name in row["symbols"]][:12],
            }
            for name in TARGETS
        },
    }

def inspect(blob:bytes,expected_sha:str,role:str)->dict:
    validate_elf(blob,expected_sha)
    reader=shutil.which("readelf")
    if not reader:
        raise RuntimeError("System readelf not installed; no metadata check attempted")
    with tempfile.TemporaryDirectory(prefix="mpfree-elfdata-") as tempdir:
        source=Path(tempdir)/"verified-original-elf.bin"
        source.write_bytes(blob)
        source.chmod(0o400)
        try:
            result=subprocess.run(
                [reader,"-W","-r","-s","-d",str(source)],
                capture_output=True,text=True,errors="replace",
                timeout=30,check=False,
            )
        finally:
            source.chmod(0o600)
        if result.returncode!=0:
            raise RuntimeError("readelf static metadata failed: "+str(result.returncode))
        if len(result.stdout)>MAX_OUTPUT:
            raise ValueError("Too much readelf output")
        data=parse_readelf(result.stdout)
    return {
        "schema":SCHEMA,
        "role":role,
        "source_sha256_verified":expected_sha,
        "arm_elf32_little_endian_verified":True,
        "readelf_system_parser_only":True,
        "vendor_executable_executed":False,
        "physical_mpc_access":False,
        "target_symbols":TARGETS,
        **data,
        "actual_dynamic_runtime_binding_proven":False,
        "actual_updater_verify_write_flow_proven":False,
    }

if __name__=="__main__":
    raise SystemExit("Library module; invoked only with authenticated owner bytes")
