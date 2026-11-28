#!/usr/bin/env python3
"""Incremental, comprehensive *OFFLINE* MPFree bootchain V7 audit.

Reads a prior owner-private SHA-indexed V6 ZIP and optional *previously
extracted* regular-file 3.10 ext4. Reuses V3/V4/V5/V6 evidence, analyzes
az01config command dispatch, ELF unwind metadata and other binary users
of the new integrity utilities. Produces ONE NEW private indexed ZIP.

Never contacts MPC, USB or SSH. No user-data collection, vendor ELF
execution, mount, firmware update, eMMC write, MaskROM request or OTP write.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import time

MANDATORY=('v6-static-evidence.json','v6-elf-relative-pointers.json',
          'v6-startup-usage.json','v6-run-stages.json',
          'v6-elf-bootloader_signature-main-recovery.json',
          'v6-elf-loadpin_trustpoints-main-recovery.json',
          'v6-elf-board_az01config-main-recovery.json')

def imp(path,name):
    spec=importlib.util.spec_from_file_location(name,path)
    module=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def run(argv,log,timeout=1200):
    try:
        p=subprocess.run(argv,capture_output=True,timeout=timeout)
        log.write_bytes((p.stdout+b'\n---- stderr ----\n'+p.stderr)[:1024*1024])
        return p.returncode
    except (OSError,subprocess.TimeoutExpired) as e:
        log.write_text(type(e).__name__+': '+str(e)[:1200]+'\n')
        return -1

def create_run(prior,repo,rootfs_override=None,skip_reference_scan=False):
    # Reuse V6's stringent full SHA256 ZIP manifest verification and known
    # original P1/P3 and Akai ELF SHA256 owner-provenance assertions.
    v6=imp(repo/'tools/mpc-one/run_bootchain_v6_followup.py','mpfree_v7_prior')
    sources=v6.verified_zip(prior)
    if not set(MANDATORY).issubset(sources):
        raise ValueError('Expected a completed V6 ZIP; one or more source reports are missing')
    if json.loads(sources['v6-static-evidence.json']).get('schema')!='mpfree-v6-cross-layer-evidence-v1':
        raise ValueError('Unexpected V6 static evidence schema')
    for name in ('bootloader_signature','loadpin_trustpoints','board_az01config'):
        doc=json.loads(sources['v6-elf-'+name+'-main-recovery.json'])
        if doc.get('schema')!='mpfree-v6-relocation-function-recovery':
            raise ValueError('Incomplete owner ELF main recovery report for '+name)
        expected=v6.TARGET_SHA[name][1]
        if doc.get('sha256')!=expected:
            raise ValueError('Wrong verified SHA of V6 ELF '+name)
    base=Path.home()/'mpfree-work'
    if not base.is_dir():raise ValueError('MPFree work root missing')
    out=base/('bootchain-wide-'+time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+'-v7-'+str(os.getpid()))
    out.mkdir(mode=0o700)
    old=os.umask(0o077)
    try:
        for name,buf in sources.items():
            f=out/name
            f.write_bytes(buf)
            f.chmod(0o600)
        (out/'v7-lineage.json').write_text(json.dumps({
            'schema':'mpfree-v7-incremental-lineage-v1',
            'previous_zip_name':prior.name,
            'previous_zip_sha256':hashlib.sha256(prior.read_bytes()).hexdigest(),
            'all_source_index_entries_verified':len(sources),
            'created_utc':datetime.now(timezone.utc).isoformat(),
            'live_device_contact':False,
        },indent=2)+'\n')
        status={}
        def stage(name,cmd,timeout=1200):
            print('V7:',name,flush=True)
            code=run(cmd,out/(name+'-console.txt'),timeout)
            status[name]={'returncode':code,'ok':code==0}
            if code:print('WARNING:',name,'failed; see ZIP log',file=sys.stderr)
        # Keep a full report even when local optional rootfs is not present.
        path=Path(rootfs_override).expanduser() if rootfs_override else v6.locate_rootfs(sources)
        if v6.rootfs_ok(path):
            names=[
              ('v7-elf-dispatch','offline-bootchain-v7-elf-dispatch.py','v7-elf-dispatch.json',90),
              ('v7-arm-unwind','offline-bootchain-v7-unwind.py','v7-arm-unwind.json',180),
            ]
            if not skip_reference_scan:
                names.append(('v7-binary-references','offline-bootchain-v7-reference-scan.py',
                              'v7-binary-references.json',1300))
            for tag,script,result,timeout in names:
                stage(tag,[sys.executable,str(repo/'tools/mpc-one'/script),
                           '--rootfs',str(path),'--report',str(out/result)],timeout)
        else:
            (out/'v7-rootfs-skipped.txt').write_text(
                'No existing regular-file 3.10 rootfs was verified. Optional ELF scans skipped.\n')
        stage('v7-synthesis',[sys.executable,
               str(repo/'tools/mpc-one/offline-bootchain-v7-synthesis.py'),str(out)],120)
        (out/'v7-run-stages.json').write_text(json.dumps({
            'stages':status,'offline_only':True,
            'rootfs_source':'previously extracted regular-file ext4 or skipped',
            'hardware_access':'none',
        },indent=2)+'\n')
    finally:
        # Package even partial results and error logs, without ever including
        # source firmware binaries or rootfs images.
        logfile=out/'v7-packaging-console.txt'
        logfile.write_text('V7 private ZIP packaging; offline reports only.\n')
        try:
            pack=imp(repo/'tools/mpc-one/package-private-boot-audit.py','mpfree_v7_zip_packager')
            zipfile,index=pack.package(out)
            print('V7 SINGLE PRIVATE ZIP:',zipfile,flush=True)
            print('INDEXED REPORTS:',len(index['files']),flush=True)
        except Exception as exc:
            logfile.write_text(type(exc).__name__+': '+str(exc)[:500]+'\n')
            print('ERROR: package failed; reports remain at',out,file=sys.stderr)
            raise
        finally:os.umask(old)
    return out.with_suffix('.zip')

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('prior_v6_zip',type=Path)
    ap.add_argument('--rootfs-310',type=Path,default=None)
    ap.add_argument('--skip-reference-scan',action='store_true')
    args=ap.parse_args()
    create_run(args.prior_v6_zip.expanduser(),Path(__file__).resolve().parents[2],
               args.rootfs_310,args.skip_reference_scan)
if __name__=='__main__':main()