#!/usr/bin/env python3
"""MPFree V10 owner-private static audit. NEVER runs firmware or contacts MPC."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import zipfile
import importlib.util

ROOT=Path(__file__).resolve().parents[2]
def load(rel,key):
    spec=importlib.util.spec_from_file_location(key,ROOT/rel)
    x=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(x)
    return x
V9=load('tools/mpc-one/run_boot_updater_v9.py','mpfree_v9_helpers_for_v10')
V8=V9.V8
ELF=V9.ELF
SCRIPT='MPFreeDirectedUpdaterV10.py'
SCHEMA='mpfree-v10-directed-static'
SOURCES=V9.SRC_SHA
def lineage(path):
    if path is None:return None
    p=V8.regular(path)
    with zipfile.ZipFile(p) as z:
        if z.testzip():raise ValueError('Corrupt ZIP CRC')
        j=json.loads(z.read('ZIP_INDEX.json'))
        if len(j['files'])<10 or len(j['files'])>100:
            raise ValueError('Unexpected previous archive size')
        checked={}
        for item in j['files']:
            n=item['name']
            if n!=Path(n).name or n in checked:raise ValueError('Unsafe indexed filename')
            info=z.getinfo(n)
            if info.file_size!=item['bytes'] or info.file_size>16000000:
                raise ValueError('Unsafe indexed length')
            data=z.read(n)
            if hashlib.sha256(data).hexdigest()!=item['sha256']:
                raise ValueError('Previous SHA index mismatch')
            checked[n]=data
        for role,sha in SOURCES.items():
            name='v9-ghidra-'+role+'.json'
            if name not in checked:raise ValueError('Previous V9 Ghidra missing: '+name)
            d=json.loads(checked[name])
            if d.get('role')!=role or d.get('source_sha256_verified_offline')!=sha:
                raise ValueError('Previous V9 source SHA mismatch: '+role)
        return {'prior_zip':p.name,'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),
                'verified_members':len(checked)}
def stage_result(path,role,code):
    x={'exit_code':code,'report':path.name,'success':False}
    if not path.exists():
        x['error']='Ghidra postscript made no report; inspect console log'
        return x
    try:
        r=json.loads(path.read_text())
        if r.get('schema')!=SCHEMA or r.get('role')!=role:
            raise ValueError('V10 schema or role mismatch')
        if r.get('source_sha256_prechecked_by_launcher')!=SOURCES[role]:
            raise ValueError('source SHA mismatch')
        if role in ('spl','uboot') and not r.get('boot_stage_memory_sha_verified'):
            raise ValueError('boot Ghidra in-memory SHA missing')
        if code!=0 or r.get('decompiled',0)<1:
            raise ValueError('Ghidra failed or no decompiled target')
        x.update({'success':True,'decompiled':r['decompiled'],
                  'recovered':len(r.get('recovered',[])),
                  'relocation_neighbors':len(r.get('relocation_slot_neighbors',[]))})
    except (ValueError,KeyError,TypeError) as e:x['error']=str(e)[:220]
    return x
def invoke(runner,work,report,role,blob,candidates):
    dest=report/('v10-ghidra-'+role+'.json')
    logfile=report/('v10-ghidra-'+role+'.log')
    if blob is None:
        projectroot=Path.home()/'mpfree-work/ghidra/projects'
        pattern={'spl':'MPFree_P1_SRAM_*.gpr','uboot':'MPFree_P3_Uboot_*.gpr'}[role]
        filename={'spl':'p1-rk3288-sram-stage.bin','uboot':'p3-uimage-payload.bin'}[role]
        project=V8.projects_from(projectroot,pattern)
        if not project:return {'success':False,'error':'Ghidra boot project absent'}
        argv=runner+[str(projectroot),project,'-process',filename,'-readOnly','-noanalysis',
                     '-scriptPath',str(ROOT/'tools/ghidra'),
                     '-postScript',SCRIPT,role,str(dest),SOURCES[role],'none',
                     '-log',str(logfile)]
    else:
        projectroot=work/'ghidra-projects'
        projectroot.mkdir(parents=True,exist_ok=True)
        binary=work/(role+'.elf')
        binary.write_bytes(blob);binary.chmod(0o400)
        argv=runner+[str(projectroot),'MPFreeV10_'+role,'-import',str(binary),
                     '-scriptPath',str(ROOT/'tools/ghidra'),
                     '-postScript',SCRIPT,role,str(dest),SOURCES[role],str(candidates),
                     '-log',str(logfile)]
    code=V8.subprocess_log(argv,report/('v10-ghidra-'+role+'-console.txt'),timeout=1450)
    return stage_result(dest,role,code)
def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--prior-v9',type=Path)
    ap.add_argument('--rootfs',type=Path,default=V8.DEFAULT_ROOTFS)
    ap.add_argument('--backup',type=Path,default=V8.DEFAULT_BACKUP)
    ap.add_argument('--no-boot-ghidra',action='store_true')
    a=ap.parse_args()
    prior=lineage(a.prior_v9)
    blobs=V9.verify_source(a.rootfs,a.backup)
    work=Path.home()/'mpfree-work'
    if not work.is_dir():ap.error('Missing owner MPFree work directory')
    out=work/('bootchain-wide-'+time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+
              '-v10-'+str(os.getpid()))
    out.mkdir(mode=0o700)
    manifest={'schema':'mpfree-v10-directed-update-boot-audit','lineage':prior,
              'verified_source_sha256':SOURCES,'stages':{},'no_device_access':True,
              'safety':'offline static, disposable ELF projects, owner saved P1/P3 projects read-only'}
    try:
        with tempfile.TemporaryDirectory(prefix='mpfree-v10-private-',dir=str(work)) as td:
            tmp=Path(td)
            candidate={'schema':'mpfree-v10-arm-relocation-evidence','data':{}}
            for role,b in blobs.items():
                candidate['data'][role]={'sha256':SOURCES[role],'status':'ok',
                    'function_candidates':ELF.candidates(b)}
            candidate_file=out/'v10-verified-elf-relocations.json'
            candidate_file.write_text(json.dumps(candidate,indent=2)+'\n')
            run=V8.get_ghidra()
            if run is None:
                manifest['stages']['ghidra']={'success':False,'error':'Ghidra launcher not available'}
            else:
                for role,b in blobs.items():
                    manifest['stages'][role]=invoke(run,tmp,out,role,b,candidate_file)
                if not a.no_boot_ghidra:
                    for role in ('spl','uboot'):
                        manifest['stages'][role]=invoke(run,tmp,out,role,None,candidate_file)
            (out/'v10-goals.json').write_text(json.dumps({
                'verification':[
                    'Find all callers of az01_verify_image where flags include bit 1',
                    'Map image verifier error -13 and state=2 to format and key lookup',
                    'Determine which actual write callbacks need signed source',
                    'Distinguish AZ01, AZ0x and FDT FIT verifier callbacks'],
                'device_backends':[
                    'Map USB VID=0x2207 PID=0x0006, protocol negotiation and command framing',
                    'Map RockUSB 0xa15/0x80000a14 and common transport error codes',
                    'Identify function tables assigned to device context +0x18',
                    'Trace bootloader commit offset +0x2c and recovery button states'],
                'independent_recovery':[
                    'Inspect read-only SPL/U-Boot ROM and boot source decision paths',
                    'Do not assert true silicon MaskROM or brick recovery from emulator'],
                'vm':[
                    'QEMU Linux 7.0.14 ARM guest boot confirmed, not RK3288 emulation',
                    'Mount /proc and /sys for inspection, never execute Akai updater',
                    'PID1 /bin/sh exit causes expected Linux panic, not failed filesystem']},
                indent=2)+'\n')
    finally:
        (out/'v10-stage-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
        privatepack=load('tools/mpc-one/package-private-boot-audit.py','mpfree_v10_privatepack')
        zipname,index=privatepack.package(out)
        print('ONE PRIVATE V10 ZIP:',zipname,'SHA indexed reports',len(index['files']))
    print('Stages:',{key:value.get('success') for key,value in manifest['stages'].items()})
    print('No vendor executable was run. No MPC hardware was contacted.')
if __name__=='__main__':main()
