#!/usr/bin/env python3
"""Bounded offline binary reference search for firmware 3.10 boot helpers.

Not a runtime call trace: a zero-hit scan does NOT prove a binary is never
launched dynamically. Searches selected vendor ELFs plus systemd text in
already extracted owner ext4; never runs any target executable.
"""
import argparse
from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess

ROOTS=('/usr/bin','/usr/sbin','/usr/lib','/opt/az01','/usr/lib/systemd/system')
MAX_BINARY=12*1024*1024
MAX_FILES=600
KEYS={
  'bootloader_sig':(b'az01-bootloader-sig',b'bootloader-sig'),
  'loadpin':(b'az0x-loadpin-trustpoints',b'loadpin/dm-verity'),
  'az01config':(b'az01config',b'program-fuses'),
  'mpfree_boot':(b'kexec',b'boot.scr',b'boot_targets',b'fw_setenv'),
  'rockchip_reboot':(b'reboot-bootloader',b'fw_printenv',b'loader'),
}
PERSONAL=re.compile(rb'(?:serialno|password|unlock|private|credential|ethaddr)',re.I)
def debugfs(root,command,timeout=30):
    p=subprocess.run(['debugfs','-R',command,str(root)],capture_output=True,timeout=timeout)
    if p.returncode or b'File not found' in p.stderr:return None
    return p.stdout
def files_in(root,folder):
    data=debugfs(root,'ls -p '+folder)
    if not data:return []
    result=[]
    for line in data.splitlines():
        parts=line.decode('utf-8','replace').split('/')
        if len(parts)<7:continue
        name=parts[5];mode=parts[2]
        if name in ('.','..','') or not mode.startswith('100'):continue
        try:size=int(parts[6])
        except ValueError:continue
        if size<=MAX_BINARY:
            result.append((folder.rstrip('/')+'/'+name,size))
    return result

def scan(root):
    root=Path(root).expanduser()
    if root.is_symlink() or not stat.S_ISREG(root.lstat().st_mode):raise ValueError('Require extracted regular-file ext4')
    with root.open('rb') as f:
        f.seek(1080)
        if f.read(2)!=b'\x53\xef':raise ValueError('Not ext4')
    entries=[];seen=set()
    for folder in ROOTS:
        for path,size in files_in(root,folder):
            if path not in seen:
                seen.add(path);entries.append((path,size))
    entries.sort(key=lambda x:(
        0 if any(t in x[0].lower() for t in ('az01','az0x','update','mpc','config','loadpin','kexec')) else 1,
        x[1],x[0]))
    out=[];errors=[];scanned=0
    for path,size in entries[:MAX_FILES]:
        b=debugfs(root,'cat '+path,timeout=40)
        if b is None:errors.append(path);continue
        scanned+=1
        hits=[]
        for category,literals in KEYS.items():
            for term in literals:
                # Only exact literal offsets, no surrounding proprietary bytes.
                at=b.lower().find(term.lower())
                if at>=0:
                    hits.append({'category':category,'term':term.decode('ascii'),
                                 'first_offset':at,'count':b.lower().count(term.lower())})
        if hits:
            out.append({'path':path,'bytes':size,
                        'sha256':hashlib.sha256(b).hexdigest(),
                        'elf':b.startswith(b'\x7fELF'),
                        'references':hits,
                        'self_reference': any(t in path.lower() for t in ('bootloader-sig','az01config','loadpin'))})
    return {'schema':'mpfree-v7-offline-binary-reference-scan-v1',
            'scanned_file_count':scanned,'candidate_files_seen':len(entries),
            'scan_limit':MAX_FILES,'unreadable_paths':errors[:30],
            'references':out,
            'limitations':['Literal string in executable does NOT prove it invokes another program.',
               'No string match does NOT prove absence of dynamic exec or generated service files.',
               'This scan is intentionally bounded and omits many nested directories.',
               'Only filenames, hashes and matched keyword offsets are reported.']}

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--rootfs',type=Path,required=True)
    ap.add_argument('--report',type=Path,required=True)
    args=ap.parse_args();data=scan(args.rootfs)
    if not args.report.is_absolute() or args.report.is_symlink():ap.error('Require absolute private report path')
    args.report.parent.mkdir(parents=True,exist_ok=True)
    old=os.umask(0o077)
    try:args.report.write_text(json.dumps(data,indent=2)+'\n')
    finally:os.umask(old)
    print('V7 examined',data['scanned_file_count'],'candidate programs/files; literal hits',len(data['references']))
if __name__=='__main__':main()