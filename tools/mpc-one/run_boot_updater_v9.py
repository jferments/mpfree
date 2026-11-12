#!/usr/bin/env python3
"""MPFree offline updater/ROM V9: reusable verified V8 evidence + new Ghidra.

Only reads pre-existing regular-file owner eMMC backup / extracted 3.10 ext4,
and optional previous private ZIP. Imported ELFs live in temporary Ghidra
projects. P1/P3 projects are -readOnly -noanalysis. No MPC, USB, Flash,
Fastboot, MaskROM, eFuse or vendor executable is ever contacted/executed.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile

ROOT=Path(__file__).resolve().parents[2]
SCRIPT="MPFreeUpdaterFocusedV9.py"
SRC_SHA={
 'az01-update':'57511e1d503f3ceaae3cb9bc40174e073b7ae410e65c986f1991731c77ae655b',
 'libaz01-prog':'af613a88f1b089069ed81a5b5fdf1ceb8c8f68bcd43c7d26b31c9ec9590e4623',
 'spl':'6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f',
 'uboot':'b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a'
}
ELF_PATHS={'az01-update':'/usr/sbin/az01-update','libaz01-prog':'/usr/lib/libaz01-prog.so.1.3'}
QUESTIONS={
 'U23':'Which real az01-update function is main and how does option dispatch work?',
 'U24':'Which backend vtable is chosen for eMMC versus RockUSB?',
 'U25':'What device probe, VID/PID and command framing does rockusb.c use?',
 'U26':'Is RockUSB a ROM protocol or loader protocol for the detected device?',
 'U27':'Which backend implements az01_write_bootloader and az01_commit_bootloader?',
 'U28':'Which verification predicates must pass on each backend before writes?',
 'U29':'What do flags 0x2, 0x11 and image verification-state 2 represent?',
 'U30':'Which image types map to bootloader versus partitions?',
 'U31':'Which AZ01/AZ0x/FDT container variants are valid, and which trust paths apply?',
 'U32':'Does the updater enter a separate update initramfs via kexec, reboot or both?',
 'U33':'Does /opt/ramfs-update.squashfs have an executable init or only BusyBox tooling?',
 'U34':'Which commands use oem flash-bootloader or rockusb_write_block and when?',
 'U35':'What power-source and rollback checks precede bootloader commits?',
 'U36':'What can be tested in isolated virtual storage versus only genuine RK3288 silicon?',
 'B201':'Is BootROM return-to-ROM separate from actual USB MaskROM enumeration?',
 'B202':'Can any U-Boot boot path choose mmc1 transiently when mmc0 succeeds?',
 'B203':'What is the exact Fastboot authorization state before any OEM command?',
 'B204':'Can an owner-controlled SPL or bootloader be recovered without Akai P1/P3?'
}
def module(rel,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/rel)
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
    return m
V8=module('tools/mpc-one/run_boot_updater_v8.py','mpfree_v8_helpers_for_v9')
ELF=module('tools/mpc-one/offline-bootchain-v6-elf-relocations.py','mpfree_v6_elf_pointer_parser_for_v9')

def validate_prior(path):
    if path is None:return None
    path=V8.regular(path)
    with zipfile.ZipFile(path) as z:
        if z.testzip():raise ValueError('Prior V8 ZIP CRC failure')
        index=json.loads(z.read('ZIP_INDEX.json'))
        if not 7<=len(index['files'])<=120:raise ValueError('Unexpected prior index size')
        known={}
        for row in index['files']:
            name=row['name']
            if Path(name).name!=name or name in known:
                raise ValueError('Unsafe or duplicate indexed member')
            info=z.getinfo(name)
            if info.file_size!=row['bytes'] or info.file_size>16*1024*1024:
                raise ValueError('Unsafe indexed member length')
            blob=z.read(name)
            if hashlib.sha256(blob).hexdigest()!=row['sha256']:
                raise ValueError('Prior V8 index SHA mismatch')
            known[name]=blob
        for role in ('az01-update','libaz01-prog','spl','uboot'):
            label=f'v8-ghidra-{role}.json'
            if label not in known:raise ValueError('Missing V8 Ghidra report '+label)
            data=json.loads(known[label])
            if data.get('role')!=role or data.get('source_sha256_prechecked_by_launcher')!=SRC_SHA[role]:
                raise ValueError('V8 stage role/source SHA not confirmed: '+role)
        return {'sha256':V8.sha(path.read_bytes()),'files_verified':len(known),'name':path.name}

def verify_source(rootfs,backup):
    backup=V8.regular(backup);rootfs=V8.regular(rootfs)
    with backup.open('rb') as f:
        f.seek(64*512)
        if V8.sha(f.read(128*512))!=V8.P1_SHA:raise ValueError('P1 backup SHA mismatch')
        f.seek(256*512+64)
        if V8.sha(f.read(363649))!=SRC_SHA['uboot']:raise ValueError('P3 backup SHA mismatch')
    with rootfs.open('rb') as f:
        f.seek(1080)
        if f.read(2)!=b'\x53\xef':raise ValueError('Expected ext4 raw rootfs')
    b=subprocess.run(['debugfs','-R','ls -p /boot',str(rootfs)],capture_output=True,timeout=35)
    if b'6.18.44' not in b.stdout:raise ValueError('Expected verified 3.10 Gen1 rootfs')
    blobs={}
    for role,path in ELF_PATHS.items():
        contents=V8.grab_debugfs(rootfs,path)
        if contents[:4]!=b'\x7fELF' or len(contents)>1024*1024:
            raise ValueError('Unexpected ELF content for '+role)
        if V8.sha(contents)!=SRC_SHA[role]:
            raise ValueError('Owner ELF SHA changed for '+role)
        blobs[role]=contents
    return blobs

def Ghidra_run(runner,work,report,role,source,sha,candidates):
    out=report/('v9-ghidra-'+role+'.json')
    logs=report/('v9-ghidra-'+role+'.log')
    projectdir=work/'v9-ghidra-projects'
    projectdir.mkdir(parents=True,exist_ok=True)
    sourcefile=work/(role+'.elf')
    sourcefile.write_bytes(source);sourcefile.chmod(0o400)
    args=runner+[str(projectdir),'MPFreeV9_'+role,'-import',str(sourcefile),
                 '-scriptPath',str(ROOT/'tools/ghidra'),
                 '-postScript',SCRIPT,role,str(out),sha,str(candidates),
                 '-log',str(logs)]
    code=V8.subprocess_log(args,report/('v9-ghidra-'+role+'-console.txt'),timeout=1250)
    return verify_stage(out,role,code)

def boot_run(runner,report,role):
    folder=Path.home()/'mpfree-work/ghidra/projects'
    if not folder.is_dir():return {'success':False,'error':'saved boot projects missing'}
    pattern={'spl':'MPFree_P1_SRAM_*.gpr','uboot':'MPFree_P3_Uboot_*.gpr'}[role]
    filename={'spl':'p1-rk3288-sram-stage.bin','uboot':'p3-uimage-payload.bin'}[role]
    name=V8.projects_from(folder,pattern)
    if name is None:return {'success':False,'error':'project not found'}
    out=report/('v9-ghidra-'+role+'.json')
    args=runner+[str(folder),name,'-process',filename,'-readOnly','-noanalysis',
                 '-scriptPath',str(ROOT/'tools/ghidra'),
                 '-postScript',SCRIPT,role,str(out),SRC_SHA[role],'none',
                 '-log',str(report/('v9-ghidra-'+role+'.log'))]
    code=V8.subprocess_log(args,report/('v9-ghidra-'+role+'-console.txt'),timeout=1100)
    return {'project':name,**verify_stage(out,role,code)}

def verify_stage(path,role,code):
    result={'exit_code':code,'report':path.name,'success':False}
    if not path.is_file():
        result['reason']='No Ghidra report (check private console log)'
        return result
    try:
        d=json.loads(path.read_text())
        if d.get('schema')!='mpfree-v9-updater-backend-and-rom-static' or d.get('role')!=role or d.get('source_sha256_verified_offline')!=SRC_SHA[role]:
            raise ValueError('Bad report schema / source SHA / role')
        if code!=0:raise ValueError('Ghidra exit nonzero')
        result.update({'success':True,'recognized_functions':d.get('recognized_functions'),
                       'decompiled':d.get('decompiled'),
                       'recovered_candidates':len(d.get('recovered_relocation_candidates',[]))})
    except (ValueError,KeyError,TypeError) as e:result['reason']=str(e)[:250]
    return result

def ramfs_meta(rootfs,tmp,report):
    result={'status':'not_found'}
    try:
        blob=V8.grab_debugfs(rootfs,'/opt/ramfs-update.squashfs')
        out=tmp/'ramfs-update.squashfs';out.write_bytes(blob)
        result={'status':'found','sha256':V8.sha(blob),'bytes':len(blob)}
        if shutil.which('unsquashfs'):
            p=subprocess.run(['unsquashfs','-ll',str(out)],capture_output=True,timeout=90)
            result['list_exit']=p.returncode
            lines=p.stdout.decode('utf8','replace').splitlines()
            result['entries']=len(lines)
            result['file_types']={'regular':sum(row.startswith('-') for row in lines),
                                  'symlink':sum(row.startswith('l') for row in lines),
                                  'directories':sum(row.startswith('d') for row in lines)}
            # No binaries, raw scripts, or symlink targets from firmware
            # will be packed. Keep only names of executable candidates.
            result['startup_names']=[
                row[-190:] for row in lines if re.search(r'/init(?:\s|$)|/linuxrc|/rcS|/reboot|/boot(?:\s|$)',row)
            ][:60]
    except Exception as e:result={'status':'error','error':type(e).__name__+': '+str(e)[:200]}
    (report/'v9-ramfs-filetypes.json').write_text(json.dumps(result,indent=2)+'\n')
    return result

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--rootfs',type=Path,default=V8.DEFAULT_ROOTFS)
    ap.add_argument('--backup',type=Path,default=V8.DEFAULT_BACKUP)
    ap.add_argument('--prior-zip',type=Path)
    ap.add_argument('--no-boot-ghidra',action='store_true')
    args=ap.parse_args()
    lineage=validate_prior(args.prior_zip) if args.prior_zip else None
    blobs=verify_source(args.rootfs,args.backup)
    work=Path.home()/'mpfree-work'
    if not work.is_dir():ap.error('Missing owner MPFree workdir')
    report=work/('bootchain-wide-'+time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+'-v9-'+str(os.getpid()))
    report.mkdir(mode=0o700)
    runner=V8.get_ghidra()
    manifest={'schema':'mpfree-v9-offline-updater-ROM',
              'sources':{role:SRC_SHA[role] for role in SRC_SHA},
              'lineage':lineage,'stages':{},'no_device_access':True,
              'warnings':['Static capability does not prove runtime execution.',
                          'RC4/SHA, write/erase/USB code can target real flash: never execute vendor utilities on host or MPC.',
                          'Never publish proprietary private Ghidra reports.',
                          'MaskROM and independent hardware restore not established.']}
    try:
        with tempfile.TemporaryDirectory(prefix='mpfree-v9-private-',dir=str(work)) as tmpstr:
            tmp=Path(tmpstr)
            manifest['stages']['ramfs']=ramfs_meta(args.rootfs,tmp,report)
            doc={'schema':'mpfree-v9-ELF-relocation-functions','data':{},'notes':'owner-private read-only candidate addresses, not verified main'}
            for role,data in blobs.items():
                try:
                    rows=ELF.candidates(data)
                    doc['data'][role]={'status':'ok','sha256':SRC_SHA[role],'function_candidates':rows}
                except Exception as e:
                    doc['data'][role]={'status':'error','reason':type(e).__name__+': '+str(e)[:150]}
            candidate_file=report/'v9-elf-entry-candidates.json'
            candidate_file.write_text(json.dumps(doc,indent=2)+'\n')
            if not runner:
                manifest['stages']['ghidra']={'success':False,'reason':'PyGhidra launcher unavailable'}
            else:
                for role,data in blobs.items():
                    manifest['stages'][role]=Ghidra_run(runner,tmp,report,role,data,SRC_SHA[role],candidate_file)
                if not args.no_boot_ghidra:
                    for role in ('spl','uboot'):
                        manifest['stages'][role]=boot_run(runner,report,role)
            (report/'v9-next-questions.json').write_text(json.dumps(QUESTIONS,indent=2)+'\n')
    finally:
        (report/'v9-stage-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
        pack=module('tools/mpc-one/package-private-boot-audit.py','mpfree_v9_private_zip_pack')
        output,index=pack.package(report)
        print('ONE PRIVATE V9 ZIP:',output,'SHA indexed files:',len(index['files']))
    print('V9 offline only. No Akai program executed. No MPC contacted.')
    print('Stage success:',{k:v.get('success',None) for k,v in manifest['stages'].items()})
if __name__=='__main__':main()
