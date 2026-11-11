#!/usr/bin/env python3
"""Read-only ARM .ARM.exidx unwind metadata as candidate function starts.

Unlike blind code scanning, function starts are derived from prel31
unwind-table records in the ELF itself. The result is evidence of possible
function boundaries, not proof of runtime execution or valid decompilation.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import struct
import subprocess

TARGETS={
 '/usr/bin/az01-bootloader-sig':'b7a82030c85ea90e31533773538abfc537d46982bdeaead6963ec9e5ea9a5861',
 '/usr/sbin/az0x-loadpin-trustpoints':'fd59c2946b2f8e4bc1c143d5270881aa2168ad2851c4726748642d2f4e040239',
 '/usr/sbin/az01config':'2a7b6b7d24a2c0f52b66edb9b59447b513f3ba8cea071cbdba0013b12096390f',
}
def sections(elf):
    path=Path(__file__).with_name('offline-bootchain-v6-elf-relocations.py')
    spec=importlib.util.spec_from_file_location('mpfree_v7_elf_sections',path)
    mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
    return mod.sections(elf)
def prel31(raw,word_va):
    value=raw&0x7fffffff
    if value&0x40000000:value-=(1<<31)
    return (word_va+value)&0xffffffff
def inspect_elf(elf):
    sects=sections(elf)
    executable=[x for x in sects if x['flags']&4 and x['name'] not in ('.plt','.init','.fini')]
    found=[];exidx=[]
    for s in sects:
        if s['name']!='.ARM.exidx' or s['size']>32768 or s['offset']+s['size']>len(elf):continue
        exidx.append({'section_va':f'0x{s["addr"]:08x}','entries':s['size']//8})
        for i in range(s['size']//8):
            address=s['offset']+i*8;va=s['addr']+i*8
            funcptr,unwind=struct.unpack_from('<II',elf,address)
            target=prel31(funcptr,va)
            section=next((x['name'] for x in executable
                          if x['addr']<=target<x['addr']+x['size']),None)
            if section:
                found.append({'vaddr':f'0x{target:08x}',
                              'unwind_word':f'0x{unwind:08x}',
                              'section':section,
                              'source_entry':f'0x{va:08x}',
                              'confidence':'ARM EHABI .ARM.exidx prel31 reference into .text'})
    unique=list({f['vaddr']:f for f in found}.values())
    return {'exidx_sections':exidx,'candidate_functions':sorted(unique,key=lambda x:int(x['vaddr'],16)),
            'warning':'Unwind metadata may omit leaf functions; entries are possible starts, not verified callsites.'}

def scan(root):
    root=Path(root).expanduser()
    if root.is_symlink() or not stat.S_ISREG(root.lstat().st_mode):raise ValueError('Require regular-file ext4')
    with root.open('rb') as f:
        f.seek(1080)
        if f.read(2)!=b'\x53\xef':raise ValueError('Not ext4')
    results={}
    for path,expected in TARGETS.items():
        p=subprocess.run(['debugfs','-R','cat '+path,str(root)],capture_output=True,timeout=50)
        if p.returncode or b'File not found' in p.stderr:
            results[path]={'status':'missing'};continue
        if hashlib.sha256(p.stdout).hexdigest()!=expected:
            results[path]={'status':'sha mismatch'};continue
        results[path]={'status':'verified','sha256':expected,**inspect_elf(p.stdout)}
    return {'schema':'mpfree-v7-arm-exidx-function-starts-v1','executables':results,
            'limitations':['No binary or source material is placed in the report',
                           'This does not execute any ELF or interact with MPC hardware.']}

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--rootfs',required=True,type=Path)
    ap.add_argument('--report',required=True,type=Path)
    args=ap.parse_args();report=scan(args.rootfs)
    if not args.report.is_absolute() or args.report.is_symlink():ap.error('Require absolute private report path')
    args.report.parent.mkdir(parents=True,exist_ok=True)
    prev=os.umask(0o077)
    try:args.report.write_text(json.dumps(report,indent=2)+'\n')
    finally:os.umask(prev)
    for k,v in report['executables'].items():print(k,v['status'],len(v.get('candidate_functions',[])))
if __name__=='__main__':main()