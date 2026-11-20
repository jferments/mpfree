#!/usr/bin/env python3
"""Derive ARM32 PIE .text function-pointer candidates from ELF relocations.

This does NOT execute ELF or guess 'main' from a string table. It records
R_ARM_RELATIVE slot -> relocated code address (including Thumb bit) and
imports only small, SHA-verified private 3.10 binaries from offline ext4.
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
import sys

TARGET_SHA={
 '/usr/bin/az01-bootloader-sig':'b7a82030c85ea90e31533773538abfc537d46982bdeaead6963ec9e5ea9a5861',
 '/usr/sbin/az0x-loadpin-trustpoints':'fd59c2946b2f8e4bc1c143d5270881aa2168ad2851c4726748642d2f4e040239',
 '/usr/sbin/az01config':'2a7b6b7d24a2c0f52b66edb9b59447b513f3ba8cea071cbdba0013b12096390f',
}
def sections(elf):
    if len(elf)<52 or elf[:4]!=b'\x7fELF' or elf[4]!=1 or elf[5]!=1:
        raise ValueError('Require ELF32 LE')
    if struct.unpack_from('<H',elf,18)[0]!=40:raise ValueError('Require ARM ELF machine 40')
    offset=struct.unpack_from('<I',elf,32)[0]
    width,count,namesidx=struct.unpack_from('<HHH',elf,46)
    if width<40 or not(0<count<300) or offset+count*width>len(elf) or namesidx>=count:
        raise ValueError('Unsafe ELF32 section table')
    out=[]
    for i in range(count):
        sh=struct.unpack_from('<IIIIIIIIII',elf,offset+i*width)
        out.append({'index':i,'nameoff':sh[0],'type':sh[1],'flags':sh[2],
                    'addr':sh[3],'offset':sh[4],'size':sh[5],'entsize':sh[9]})
    st=out[namesidx]
    strings=elf[st['offset']:st['offset']+st['size']]
    for row in out:
        off=row['nameoff']
        row['name']=(strings[off:strings.find(b'\0',off)] if off<len(strings)
                    else b'').decode('ascii','replace')
    return out

def candidates(elf):
    sec=sections(elf)
    execute=[s for s in sec if s['flags']&4 and s['name'] not in ('.plt','.plt.got','.init','.fini')]
    if not execute:raise ValueError('No executable .text section')
    def section_for(va):
        return next((s for s in sec if s['addr']<=va<s['addr']+s['size']
                     and s['type']!=8),None)
    found=[];seen=set()
    for secrow in sec:
        if secrow['type'] not in (9,4) or secrow['size']>65536:continue
        w=8 if secrow['type']==9 else 12
        if secrow['offset']+secrow['size']>len(elf):continue
        for offset in range(secrow['offset'],secrow['offset']+secrow['size']-w+1,w):
            where,info=struct.unpack_from('<II',elf,offset)
            if info&255!=23:continue  # R_ARM_RELATIVE
            targetsec=section_for(where)
            if not targetsec:continue
            data_offset=targetsec['offset']+(where-targetsec['addr'])
            if data_offset+4>len(elf):continue
            value=struct.unpack_from('<I',elf,data_offset)[0]
            actual=value &~1
            textsec=next((s for s in execute if s['addr']<=actual<s['addr']+s['size']),None)
            if not textsec or actual in seen:continue
            seen.add(actual)
            found.append({'candidate_vaddr':f'0x{actual:08x}',
                          'thumb':bool(value&1),
                          'relocation_slot':f'0x{where:08x}',
                          'section':textsec['name'],
                          'confidence':'ELF R_ARM_RELATIVE points into executable .text, not necessarily main'})
    return sorted(found,key=lambda x:int(x['candidate_vaddr'],16))[:250]

def cat(rootfs,path):
    p=subprocess.run(['debugfs','-R','cat '+path,str(rootfs)],
                     capture_output=True,timeout=30)
    if p.returncode or b'File not found' in p.stderr:return None
    return p.stdout

def inspect_rootfs(rootfs):
    rootfs=Path(rootfs).expanduser()
    if rootfs.is_symlink() or not stat.S_ISREG(rootfs.lstat().st_mode):
        raise ValueError('Require existing regular ext4 image')
    with rootfs.open('rb') as f:
        f.seek(1080)
        if f.read(2)!=b'\x53\xef':raise ValueError('Not ext4')
    boot=subprocess.run(['debugfs','-R','ls -p /boot',str(rootfs)],
                        capture_output=True,timeout=20)
    if b'6.18.44' not in boot.stdout:raise ValueError('Not verified 3.10 kernel')
    out={}
    for name,sha in TARGET_SHA.items():
        data=cat(rootfs,name)
        if data is None:out[name]={'status':'missing'};continue
        got=hashlib.sha256(data).hexdigest()
        if got!=sha:out[name]={'status':'sha mismatch','actual_sha256':got};continue
        out[name]={'status':'ok','sha256':sha,'file_size':len(data),
                   'function_candidates':candidates(data)}
    return out

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rootfs',type=Path,required=True)
    parser.add_argument('--report',type=Path,required=True)
    a=parser.parse_args()
    res={'schema':'mpfree-offline-elf-arm-relative-function-candidates-v6',
         'data':inspect_rootfs(a.rootfs),'limitations':[
           'RELATIVE pointer does not prove target is main or executed.',
           'ARM/Thumb entry mode requires correct context in Ghidra.',
           'No vendor program was executed, mounted, or written to.',
         ]}
    old=os.umask(0o077)
    try:a.report.write_text(json.dumps(res,indent=2)+'\n')
    finally:os.umask(old)
    for k,v in res['data'].items():print(k, v['status'],len(v.get('function_candidates',[])))
if __name__=='__main__':main()