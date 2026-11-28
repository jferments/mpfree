#!/usr/bin/env python3
"""Bounded text-file search inside existing verified 3.10 ext4, via debugfs.

No mounting, firmware execution, device contact or writes to input ext4.
Finds references to bootloader-signature, LoadPin, az01config, kexec,
reboot-mode, storage updates, systemd startup and ROM/USB staging.
"""
import argparse
from collections import deque
import json
import os
from pathlib import Path
import re
import stat
import subprocess

KEYWORDS={
 'bootloader_signature':re.compile(rb'az01-bootloader-sig|bootloader.sig|bootloader-signature',re.I),
 'fuse_programming':re.compile(rb'program-fuses|az01config',re.I),
 'security_services':re.compile(rb'az0x-loadpin-trustpoints|loadpin|dm-verity|trusted-verity-digests',re.I),
 'bootmode_and_recovery':re.compile(rb'(?:reboot-mode|reset_bootloader|maskrom|rockusb|reboot.?loader)',re.I),
 'kernel_handoff':re.compile(rb'\bkexec\b|\bbootm\b|\bbootz\b|extlinux|boot\.scr',re.I),
 'firmware_update':re.compile(rb'az01-update|az01_commit_bootloader|az01_write_bootloader',re.I),
}
START_DIRS=['/etc/systemd/system','/usr/lib/systemd/system','/lib/systemd/system',
            '/etc/init.d','/etc/udev/rules.d','/usr/lib/udev/rules.d',
            '/etc','/usr/lib/az0x','/opt']
DIR_LIMIT=180;TOTAL_LIMIT=450;TEXT_BYTES_LIMIT=300*1024
ALLOWED=re.compile(r'\.(?:service|target|socket|path|timer|mount|conf|rules|sh|txt|ini|toml|cfg|preset|wants)$',re.I)
DETECT_ONLY=re.compile(r'^(?:fstab|inittab|rc\.local|profile|environment|hostname)$',re.I)
PRIVATE=re.compile(rb'(?:password|private[_ -]?key|secret|token|credential|auth[_ -]?key)',re.I)

def query(image,statement,limit=30):
    p=subprocess.run(['debugfs','-R',statement,str(image)],
                     capture_output=True,timeout=limit)
    return p.stdout,p.stderr,p.returncode

def listdir(image,path):
    data,err,_=query(image,'ls -p '+path)
    if b'not found' in err.lower():return []
    result=[]
    for raw in data.decode('utf8','replace').splitlines():
        line=raw.strip()
        if not line.startswith('/') or not line.endswith('/'):
            continue
        parts=line.split('/')
        if len(parts)<7 or not parts[1].isdigit():continue
        name=parts[5]  # debugfs ls -p: /inode/mode/uid/gid/name/size/
        if name in ('','.', '..') or '/' in name:continue
        # names from filesystem are untrusted, but they are passed to
        # debugfs as one shell-free -R argument (no evaluation by bash).
        if not re.fullmatch(r'[\w.\-@+]+',name):continue
        result.append(name)
        if len(result)>=DIR_LIMIT:break
    return result

def small_text(image,path):
    data,err,_=query(image,'stat '+path)
    if b'not found' in err.lower():return None
    mat=re.search(rb'\bSize:\s*(\d+)\b',data)
    if not mat or int(mat.group(1))>TEXT_BYTES_LIMIT:return None
    data,err,_=query(image,'cat '+path)
    if len(data)>TEXT_BYTES_LIMIT or b'\0' in data[:4096] or b'not found' in err.lower():return None
    return data

def inspect(image):
    image=Path(image).expanduser()
    if image.is_symlink() or not stat.S_ISREG(image.lstat().st_mode):
        raise ValueError('Existing regular-file rootfs required')
    with image.open('rb') as f:
        f.seek(1080)
        if f.read(2)!=b'\x53\xef':raise ValueError('Expected ext4')
    data,_,_=query(image,'ls -p /boot')
    if b'6.18.44' not in data:raise ValueError('3.10 rootfs 6.18.44 not confirmed')
    paths=[]
    for base in START_DIRS:
        for name in listdir(image,base):
            if ALLOWED.search(name) or DETECT_ONLY.fullmatch(name):paths.append(base+'/'+name)
    paths=list(dict.fromkeys(paths))[:TOTAL_LIMIT]
    findings=[];stats={'candidate_paths':len(paths),'text_files_examined':0,'keywords_found':{k:0 for k in KEYWORDS}}
    for path in paths:
        content=small_text(image,path)
        if content is None:continue
        stats['text_files_examined']+=1
        hits=[]
        for key,pattern in KEYWORDS.items():
            m=pattern.search(content)
            if not m:continue
            stats['keywords_found'][key]+=1
            line_no=content[:m.start()].count(b'\n')+1
            line=content.splitlines()[line_no-1][:260]
            # Never emit credentials or whole service-file content.
            excerpt=('[SENSITIVE LINE REDACTED]' if PRIVATE.search(line)
                     else line.decode('utf8','replace'))
            hits.append({'category':key,'first_line':line_no,'excerpt':excerpt})
        if hits:findings.append({'file':path,'file_sha256':__import__('hashlib').sha256(content).hexdigest(),
                                 'hits':hits})
    return {'schema':'mpfree-v6-startup-reference-search-v1',
            'status':'private verified ext4 metadata/text search only',
            'statistics':stats,'matches':findings,
            'limitations':[
              'This bounded service/config search cannot prove a helper never runs.',
              'Opaque binaries, deferred shell calls, generated units and unscanned directories may invoke helpers.',
              'No Akai executable was run; no rootfs image was mounted or modified.',
              'A service mentioning a utility does not prove it runs under the active target.',
            ]}

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--rootfs',required=True,type=Path)
    ap.add_argument('--report',required=True,type=Path)
    a=ap.parse_args()
    v=inspect(a.rootfs)
    old=os.umask(0o077)
    try:a.report.write_text(json.dumps(v,indent=2)+'\n',encoding='utf8')
    finally:os.umask(old)
    print('Startup candidate files:',v['statistics']['candidate_paths'],
          'inspected:',v['statistics']['text_files_examined'],
          'matched:',len(v['matches']))
if __name__=='__main__':main()