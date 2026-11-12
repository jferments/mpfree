#!/usr/bin/env python3
"""Offline ARM ELF backend table candidates. Never runs ELF or accesses MPC.

This is deliberately *hypothesis generation*: a table-shaped run of
R_ARM_RELATIVE pointers is not proof of runtime assignment/selection.
Actual source hash and ELF section bounds are checked by the caller.
"""
import importlib.util
import struct
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
def _elf_helper():
    spec=importlib.util.spec_from_file_location('mpfree_v11_elf_sections',
        ROOT/'tools/mpc-one/offline-bootchain-v6-elf-relocations.py')
    x=importlib.util.module_from_spec(spec);spec.loader.exec_module(x)
    return x

# ARM32 image-backend interface reverse engineered from wrappers:
# +08 open, +18 bootloader, +1c partition, +20 stream,
# +2c commit, +34 reset. Different kinds of tables may have
# similar pointer density, so require compact header + functional roles.
CALLBACKS={'open':0x08,'write_bootloader':0x18,
           'write_partition':0x1c,'stream_partition':0x20,
           'commit_bootloader':0x2c,'reset':0x34}
def candidate_tables(pointers,reader,minimum=3):
    """pointers: mapped ELF vaddr -> {'target': ..., ...};
       reader: returns integer u32 at ELF vaddr or None.
       Traces all candidate base addresses, not hard-coded vendor offsets.
    """
    starts=set()
    for at in pointers:
        for delta in CALLBACKS.values():
            if at>=delta and (at-delta)%4==0:
                starts.add(at-delta)
    out=[]
    for base in starts:
        first=reader(base)
        second=reader(base+4)
        if first is None or second is None or first>8 or second>8:
            continue
        found={}
        for label,offset in CALLBACKS.items():
            row=pointers.get(base+offset)
            if row: found[label]=row['target']
        if len(found)<minimum or 'open' not in found:
            continue
        # Write operations are more informative than generic dispatch tables.
        if not any(k in found for k in ('write_bootloader','write_partition','stream_partition')):
            continue
        score=10*len(found)+(6 if 'write_bootloader' in found else 0)+(
            6 if 'write_partition' in found else 0)
        out.append({'base_vaddr':f'0x{base:08x}',
                    'header_words':[first,second],
                    'matched_callback_count':len(found),
                    'score':score,
                    'callbacks':{k:f'0x{v:08x}' for k,v in found.items()},
                    'evidence':'matching relocated function-pointer slots and compact integer header; selection not proven'})
    return sorted(out,key=lambda x:(-x['score'],int(x['base_vaddr'],16)))[:55]

def inspect(elf):
    h=_elf_helper()
    sections=h.sections(elf)
    def section_at(va):
        return next((s for s in sections if s['addr']<=va<s['addr']+s['size']
                     and s['type']!=8 and s['offset']+s['size']<=len(elf)),None)
    def u32(va):
        s=section_at(va)
        if not s:return None
        off=s['offset']+(va-s['addr'])
        if off<0 or off+4>len(elf):return None
        return struct.unpack_from('<I',elf,off)[0]
    execute=[s for s in sections if (s['flags']&4) and s['name']
             not in ('.plt','.plt.got','.init','.fini')]
    pointers={}
    for sec in sections:
        if sec['type']!=9 or sec['size']>512*1024 or sec['size']%8 or sec['offset']+sec['size']>len(elf):
            continue
        for off in range(sec['offset'],sec['offset']+sec['size'],8):
            slot,info=struct.unpack_from('<II',elf,off)
            if info&0xff!=23:continue
            val=u32(slot)
            if val is None:continue
            clean=val&~1
            if not any(s['addr']<=clean<s['addr']+s['size'] for s in execute):
                continue
            pointers[slot]={'target':clean,'thumb':bool(val&1),
                            'source_section':section_at(slot)['name']}
    tables=candidate_tables(pointers,u32)
    return {
        'schema':'mpfree-v11-raw-ELF-backend-pointer-candidates',
        'relative_code_pointers':len(pointers),
        'tables':tables,
        'strong_candidates':[r for r in tables if r['matched_callback_count']>=4],
        'notes':[
            'ELF virtual addresses here are base-independent, unlike Ghidra imported addresses.',
            'Sections and REL relocations are parsed without executing the firmware.',
            'A matching table header and callbacks does not prove device selection or hardware recovery.',
            'A table may be a format or parser vtable that superficially resembles a backend.',
            'Check table address assignment and callback argument/behavior before labeling roles.']}
