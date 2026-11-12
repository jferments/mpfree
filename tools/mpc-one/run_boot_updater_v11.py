#!/usr/bin/env python3
"""MPFree V11 static verification of update backend tables and BootROM paths.

Owner backup/rootfs read only. ARM ELF executed: NEVER.
Ghidra P1/P3 saved projects opened read-only. No physical devices.
Creates a single private, SHA-indexed ZIP in ~/mpfree-work/.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import time
import zipfile
ROOT=Path(__file__).resolve().parents[2]
def load(path,name):
    spec=importlib.util.spec_from_file_location(name,ROOT/path)
    obj=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj
V10=load('tools/mpc-one/run_boot_updater_v10.py','mpfree_v10_for_v11')
TBL=load('tools/mpc-one/offline-bootchain-v11-backend-tables.py','mpfree_v11_raw_tables')
V8=V10.V8
SHA=V10.SOURCES
SCRIPT='MPFreeBackendSelectorV11.py'
SCHEMA='mpfree-v11-backend-selector-and-boot-recovery-static'

def check_previous(path):
    p=V8.regular(path)
    with zipfile.ZipFile(p) as f:
        if f.testzip():raise ValueError('Corrupt owner V10 zip')
        idx=json.loads(f.read('ZIP_INDEX.json'))
        rows=idx['files']
        if not 8<len(rows)<=90 or len({i['name'] for i in rows})!=len(rows):
            raise ValueError('Duplicate/unexpected ZIP index members')
        for row in rows:
            name=row['name']
            if Path(name).name!=name or name.startswith('.'):
                raise ValueError('Unsafe indexed filename')
            item=f.getinfo(name)
            if item.file_size!=row['bytes'] or item.file_size>32*1024*1024:
                raise ValueError('Untrusted V10 member size')
            if hashlib.sha256(f.read(name)).hexdigest()!=row['sha256']:
                raise ValueError('V10 indexed member SHA invalid')
        for role,sha in SHA.items():
            name='v10-ghidra-'+role+'.json'
            report=json.loads(f.read(name))
            if report.get('schema')!='mpfree-v10-directed-static' or report.get('role')!=role:
                raise ValueError('Prior V10 stage schema/role invalid')
            if report.get('source_sha256_prechecked_by_launcher')!=sha:
                raise ValueError('Prior V10 stage source SHA invalid')
            if role in ('spl','uboot') and report.get('boot_stage_memory_sha_verified')!=True:
                raise ValueError('Prior V10 boot project did not verify Ghidra bytes')
        return {'filename':p.name,'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),
                'indexed_members_verified':len(rows)}

def invoke_v11(run,tmp,out,role,blob,candidate_file):
    # V10.invoke is a tested subprocess wrapper that still names outputs
    # v10-ghidra-*. Rename its actual reports and logs to v11 before
    # packaging, leaving validation based on V11 schema untouched.
    result=V10.invoke(run,tmp,out,role,blob,candidate_file)
    oldprefix='v10-ghidra-'+role
    newprefix='v11-ghidra-'+role
    for suffix in ('.json','.log','-console.txt'):
        old=out/(oldprefix+suffix)
        new=out/(newprefix+suffix)
        if old.exists():
            if new.exists():raise RuntimeError('Refusing to overwrite V11 report')
            old.rename(new)
    result['report']=newprefix+'.json'
    return result

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--prior-v10',type=Path,required=True,
                    help='Private verified owner V10 ZIP to establish evidence lineage')
    ap.add_argument('--rootfs',type=Path,default=V8.DEFAULT_ROOTFS)
    ap.add_argument('--backup',type=Path,default=V8.DEFAULT_BACKUP)
    ap.add_argument('--skip-boot-ghidra',action='store_true',
                    help='Optional; skip P1/P3 Ghidra only')
    a=ap.parse_args()
    lineage=check_previous(a.prior_v10)
    blobs=V10.V9.verify_source(a.rootfs,a.backup)
    work=Path.home()/'mpfree-work'
    if not work.is_dir():ap.error('Missing owner /mpfree-work directory')
    out=work/('bootchain-wide-'+time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+
              '-v11-'+str(os.getpid()))
    out.mkdir(mode=0o700)
    manifest={'schema':'mpfree-v11-private-updater-backend-audit','lineage':lineage,
              'source_sha256':SHA,'stages':{},'no_device_access':True,
              'warnings':[
                'Ghidra inferred callback tables still require xref/backend selector proof.',
                'Ghidra imported ELF pointers may already contain the image load base.',
                'No live updater, RockUSB, Fastboot, BootROM or eMMC writes.',
                'Physical independent recovery and kexec-proof install not yet verified.']}
    V10.SCRIPT=SCRIPT
    V10.SCHEMA=SCHEMA
    try:
        with tempfile.TemporaryDirectory(prefix='mpfree-v11-private-',dir=str(work)) as td:
            tmp=Path(td)
            evidence={}
            for role,blob in blobs.items():
                candidate=TBL.inspect(blob)
                candidate['role']=role
                candidate['source_sha256']=SHA[role]
                evidence[role]=candidate
                file=out/('v11-raw-backend-tables-'+role+'.json')
                file.write_text(json.dumps(candidate,indent=2)+'\n')
                # GNU readelf is invoked as a static DATA parser only.
                # The owner bytes were already SHA-verified by verify_source.
                # This does not run the updater or dynamically load the .so.
                try:
                    linkage=load(
                        "tools/mpc-one/offline_elf_linkage_audit.py",
                        "mpfree_owner_elf_linkage_static"
                    ).inspect(blob,SHA[role],role)
                    path=out/('v11-elf-linkage-'+role+'.json')
                    path.write_text(json.dumps(linkage,indent=2)+'\n')
                    manifest['stages']['elf-linkage-'+role]={
                        'success':True,
                        'imports':len(linkage['az01_import_symbols']),
                        'exports':len(linkage['az01_export_symbols']),
                        'az01_relocations':len(linkage['az01_relocations']),
                        'vendor_executable_executed':False,
                    }
                except Exception as exc:
                    manifest['stages']['elf-linkage-'+role]={
                        'success':False,'error':str(exc)}
                    manifest['warnings'].append(
                        'Static ELF relocation parsing unavailable for '+
                        role+': '+str(exc))
            run=V8.get_ghidra()
            if not run:
                manifest['stages']['ghidra']={'success':False,
                    'error':'No Ghidra launcher available'}
            else:
                for role,blob in blobs.items():
                    candidate_file=out/('v11-raw-backend-tables-'+role+'.json')
                    manifest['stages'][role]=invoke_v11(run,tmp,out,role,blob,candidate_file)
                    manifest['stages'][role]['raw_table_candidates']=len(evidence[role]['tables'])
                if not a.skip_boot_ghidra:
                    for role in ('spl','uboot'):
                        manifest['stages'][role]=invoke_v11(run,tmp,out,role,None,'none')
            (out/'v11-questions.json').write_text(json.dumps({
                'backend':[
                    'Where is the selected table address installed into device->backend?',
                    'Does marker 2 at ELF 0x12a74 identify Fastboot device mode?',
                    'Does marker 1 at ELF 0x12bd8 identify RockUSB loader mode?',
                    'Do the +0x18, +0x1c, +0x20 callbacks actually perform writes?',
                    'How do open, write, commit, verify and reset report failure?',
                    'Does the MPC use RockUSB BootROM mode or an already-running loader?'],
                'image_trust':[
                    'What does signature verifier state=2 after -13 mean?',
                    'Can state=2 ever reach az01_write_image with flag 2 and with what format?',
                    'Is verification enforced separately on AZ01, AZ0x and FIT/FDT formats?',
                    'Are cryptographic keys unique to the model, firmware or device?'],
                'boot_recovery':[
                    'What is the independent RK3288 ROM boot source and USB recovery mechanism?',
                    'Does unmodified SPL allow an ephemeral, no-write SD boot?',
                    'How does firmware 3.10 affect bootloader and kexec availability?',
                    'No write testing until independent physical restore is demonstrated.']
                },indent=2)+'\n')
    finally:
        (out/'v11-stage-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
        pack=load('tools/mpc-one/package-private-boot-audit.py','mpfree_v11_package')
        filename,index=pack.package(out)
        print('ONE PRIVATE V11 ZIP:',filename,'sha-indexed files:',len(index['files']))
    print('V11 stages:',{k:v.get('success') for k,v in manifest['stages'].items()})
    print('Only verified owner backups and offline Ghidra; no MPC hardware contacted.')
if __name__=='__main__':main()
