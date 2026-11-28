#!/usr/bin/env python3
"""Read-only MPFree V7: resolve owner Akai ARM32 PIE command dispatch tables.

One verified extracted 3.10 rootfs (regular ext4) is read through debugfs.
No ELF execution, filesystem mount, MPC access, or storage modification.
Map possible 12-byte records [command string pointer, .text handler pointer,
optional extra pointer] to discover az01config CLI commands safely.
ELF relocation-backed targets and raw absolute addresses are reported
with confidence labels, not interpreted as runtime execution evidence.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import struct
import subprocess

NAME='/usr/sbin/az01config'
EXPECTED='2a7b6b7d24a2c0f52b66edb9b59447b513f3ba8cea071cbdba0013b12096390f'
ALLOWED=re.compile(r'^[a-zA-Z0-9_+./:-]{3,65}$')
SENSITIVE=re.compile(r'(?:password|private|secret|token|credential|unlock|key=)',re.I)

def mod():
    source=Path(__file__).with_name('offline-bootchain-v6-elf-relocations.py')
    spec=importlib.util.spec_from_file_location('mpfree_v7_elf_sections',source)
    obj=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj

def file_offset(elf,sections,va,size=4):
    for s in sections:
        if s['type']!=8 and s['addr']<=va and va+size<=s['addr']+s['size']:
            at=s['offset']+va-s['addr']
            if at+size<=len(elf):return at
    return None

def relocate_slots(elf,sections):
    """Use only ELF32 REL R_ARM_RELATIVE (r_type=23) slots."""
    out={}
    for s in sections:
        if s['type']!=9 or s['size']>131072 or s['offset']+s['size']>len(elf):continue
        for off in range(s['offset'],s['offset']+s['size']-7,8):
            where,info=struct.unpack_from('<II',elf,off)
            if info&255!=23:continue
            at=file_offset(elf,sections,where)
            if at is not None:out[where]=struct.unpack_from('<I',elf,at)[0]
    return out

def read_pointer(elf,sections,address,relocs):
    at=file_offset(elf,sections,address)
    if at is None:return None
    return {'value':struct.unpack_from('<I',elf,at)[0],
            'relocation_backed':address in relocs}

def read_string(elf,sections,va):
    at=file_offset(elf,sections,va,size=1)
    if at is None:return None
    end=elf.find(b'\0',at,min(len(elf),at+67))
    if end<0:return None
    data=elf[at:end]
    if not 3<=len(data)<=65:return None
    try:txt=data.decode('ascii')
    except UnicodeError:return None
    if not ALLOWED.fullmatch(txt) or SENSITIVE.search(txt):return None
    return txt

def code_pointer(sections,value):
    actual=value&~1
    for s in sections:
        if s['flags']&4 and s['name'] not in ('.plt','.init','.fini') and s['addr']<=actual<s['addr']+s['size']:
            return {'virtual_address':f'0x{actual:08x}',
                    'thumb':bool(value&1),'section':s['name']}
    return None

def map_records(elf,sections,relocs):
    """Scan writable sections for 12-byte name -> function records.

    Command dispatch in recovered az01config main compares up to 5 names,
    stepping 12 bytes per record and using the second 4-byte field as
    function callback. We require relocation-backed pointers and strings.
    """
    found=[]
    for sec in sections:
        if not(sec['flags']&2) or sec['type']==8 or sec['size']>300000:continue
        first=sec['addr'];last=sec['addr']+sec['size']
        for base in range((first+3)&~3,last-12,4):
            entries=[]
            for i in range(0,8):
                at=base+i*12
                if at+12>last:break
                name=read_pointer(elf,sections,at,relocs)
                handler=read_pointer(elf,sections,at+4,relocs)
                if not name or not handler:break
                text=read_string(elf,sections,name['value'])
                target=code_pointer(sections,handler['value'])
                if not text or not target:break
                # We require both pointers to be static relocation targets,
                # thus ignoring random numeric coincidences in .data.
                if not(name['relocation_backed'] and handler['relocation_backed']):break
                opt=read_pointer(elf,sections,at+8,relocs)
                entries.append({'record_va':f'0x{at:08x}',
                                'name':text,'name_pointer':f'0x{name["value"]:08x}',
                                'handler':target,
                                'auxiliary_field_va':f'0x{at+8:08x}',
                                'auxiliary_raw':f'0x{opt["value"]:08x}' if opt else None})
            if len(entries)>=3 and len({e['name'] for e in entries})==len(entries):
                found.append({'table_va':f'0x{base:08x}',
                              'section':sec['name'],'record_stride':12,
                              'entries':entries,
                              'evidence':'ELF32 RELATIVE relocations + ASCII command pointers + executable targets'})
    # Prefer precise 5-entry table but preserve other candidate tables.
    return sorted(found,key=lambda t:(-int(len(t['entries'])==5),-len(t['entries']),t['table_va']))[:24]

def analyze_bytes(blob):
    if hashlib.sha256(blob).hexdigest()!=EXPECTED:
        raise ValueError('Wrong az01config owner ELF SHA256')
    m=mod();sects=m.sections(blob);rel=relocate_slots(blob,sects)
    tables=map_records(blob,sects,rel)
    names={entry['name'] for t in tables for entry in t['entries']}
    return {'schema':'mpfree-v7-elf-command-dispatch-v1',
            'source_path':NAME,'source_sha256':EXPECTED,
            'relative_relocation_slots':len(rel),
            'candidate_tables':tables,
            'program_fuses_mapping':[
                {'table':t['table_va'],'handler':entry['handler']}
                for t in tables for entry in t['entries']
                if entry['name']=='program-fuses'],
            'confidence':('candidate handler target supported by ELF relocation; '
                          'the command has NOT been executed, and target behavior remains unverified'),
            'limitations':[
                'ELF pointers identify candidate handlers, not runtime permissions or complete side effects.',
                'Absence of a table may mean the compiler used a different layout or relocation type.',
                'Never run a fuse or write command to test a static hypothesis.',
                'This is owner-only proprietary ELF analysis, keep generated report private.',
            ]}

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--rootfs',type=Path,required=True)
    ap.add_argument('--report',type=Path,required=True)
    a=ap.parse_args()
    source=a.rootfs.expanduser()
    if source.is_symlink() or not stat.S_ISREG(source.lstat().st_mode):
        ap.error('Require previously extracted regular ext4 rootfs; never a live device')
    with source.open('rb') as f:
        f.seek(1080)
        if f.read(2)!=b'\x53\xef':ap.error('Not ext4')
    proc=subprocess.run(['debugfs','-R','cat '+NAME,str(source)],capture_output=True,timeout=45)
    if proc.returncode or b'File not found' in proc.stderr:ap.error('az01config unavailable in owner rootfs')
    result=analyze_bytes(proc.stdout)
    target=a.report.expanduser()
    if not target.is_absolute() or target.is_symlink():ap.error('Require absolute regular report path')
    target.parent.mkdir(parents=True,exist_ok=True)
    old=os.umask(0o077)
    try:target.write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
    finally:os.umask(old)
    print('V7 dispatch candidate tables',len(result['candidate_tables']),
          'fuse mappings',len(result['program_fuses_mapping']))
if __name__=='__main__':main()