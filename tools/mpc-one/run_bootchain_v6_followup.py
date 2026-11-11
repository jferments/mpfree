#!/usr/bin/env python3
"""One-command, offline-only MPFree V6 broad boot-chain investigation.

Input is the *already generated and SHA-indexed* private V5 ZIP.
Output is ONE new private ZIP including V5 lineage and every V6 report/log.

NEVER contacts the MPC, sends USB requests, runs vendor programs, mounts
rootfs, writes eMMC, changes boot targets, enters MaskROM, or touches OTP.
Only workstation files are read; Ghidra imports are disposable.
"""
import argparse
from datetime import datetime,timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import zipfile

REQUIRED={
 'p1-evidence-graph-v3.json', 'p3-evidence-graph-v3.json',
 'p1-coverage-v4.json', 'p3-coverage-v4.json',
 'firmware-3.10-bootchain-v4-compare.json',
 'v5-ghidra-bootloader_signature.json',
 'v5-ghidra-loadpin_trustpoints.json',
 'v5-ghidra-board_az01config.json',
}
SHA_P1='6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f'
SHA_P3='b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a'
TARGET_SHA={
 'bootloader_signature':('/usr/bin/az01-bootloader-sig',
    'b7a82030c85ea90e31533773538abfc537d46982bdeaead6963ec9e5ea9a5861'),
 'loadpin_trustpoints':('/usr/sbin/az0x-loadpin-trustpoints',
    'fd59c2946b2f8e4bc1c143d5270881aa2168ad2851c4726748642d2f4e040239'),
 'board_az01config':('/usr/sbin/az01config',
    '2a7b6b7d24a2c0f52b66edb9b59447b513f3ba8cea071cbdba0013b12096390f'),
}
MAX_ENTRY=32*1024*1024

def verified_zip(source):
    source=Path(source)
    if source.is_symlink() or not stat.S_ISREG(source.lstat().st_mode):
        raise ValueError('V5 ZIP must be a regular file, never a symlink')
    with zipfile.ZipFile(source) as z:
        if z.testzip():raise ValueError('Corrupted ZIP CRC')
        index=json.loads(z.read('ZIP_INDEX.json'))
        if index.get('schema')!='mpfree-private-audit-zip-index-v1':
            raise ValueError('Unknown private ZIP schema')
        rows=index.get('files',[])
        names={x['name'] for x in rows}
        if len(rows)>150 or not REQUIRED.issubset(names) or len(names)!=len(rows):
            raise ValueError('Missing required V5 reports or duplicate ZIP index')
        if set(z.namelist())!=names|{'ZIP_INDEX.json'}:
            raise ValueError('ZIP has unindexed extra entries')
        out={}
        for row in rows:
            name=row['name']
            if '/' in name or '\\' in name or name.startswith('.') or not name.endswith(('.json','.log','.txt','.md')):
                raise ValueError('Refusing unexpected ZIP filename')
            size=z.getinfo(name).file_size
            if size!=row['bytes'] or size>MAX_ENTRY:
                raise ValueError('Refusing unexpected ZIP entry size')
            buf=z.read(name)
            if hashlib.sha256(buf).hexdigest()!=row['sha256']:
                raise ValueError('Indexed SHA256 mismatch: '+name)
            out[name]=buf
    for stage,sha in (('p1',SHA_P1),('p3',SHA_P3)):
        graph=json.loads(out[stage+'-evidence-graph-v3.json'])
        coverage=json.loads(out[stage+'-coverage-v4.json'])
        if graph.get('sha256')!=sha or coverage.get('source_payload_sha256')!=sha:
            raise ValueError('Wrong previously audited firmware stage')
    v4=json.loads(out['firmware-3.10-bootchain-v4-compare.json'])
    for label,(path,sha) in TARGET_SHA.items():
        e=json.loads(out['v5-ghidra-'+label+'.json'])
        if e.get('source_sha256_verified_by_launcher')!=sha or v4['elfs'][path]['firmware310'].get('sha256')!=sha:
            raise ValueError('ELF source SHA inconsistent '+label)
    return out

def safe_run(argv,logfile,timeout):
    try:
        p=subprocess.run(argv,capture_output=True,timeout=timeout,check=False)
        data=p.stdout+b'\n---stderr---\n'+p.stderr
        logfile.write_bytes(data[:2*1024*1024])
        return p.returncode
    except (OSError,subprocess.TimeoutExpired) as exc:
        logfile.write_text(type(exc).__name__+': '+str(exc)[:800]+'\n')
        return -1

def locate_rootfs(data,explicit=None):
    if explicit:return Path(explicit).expanduser()
    b=data.get('firmware-3.10-rootfs-locator.json')
    if not b:return None
    candidate=json.loads(b).get('selected')
    return Path(candidate) if candidate else None

def rootfs_ok(path):
    if path is None:return False
    try:
        if path.is_symlink() or not stat.S_ISREG(path.lstat().st_mode):return False
        with path.open('rb') as stream:
            stream.seek(1080)
            return stream.read(2)==b'\x53\xef'
    except OSError:return False

def ghidra_runner():
    gh=os.environ.get('GHIDRA_HOME')
    if gh:
        path=Path(gh)/'support'/'pyghidraRun'
        if path.is_file():return [str(path),'-H']
    if shutil.which('snap') and subprocess.run(['snap','list','ghidra'],
             capture_output=True,timeout=15).returncode==0:
        return ['snap','run','ghidra.ghidra','-H']
    return None

def elf_bytes(rootfs,path):
    process=subprocess.run(['debugfs','-R','cat '+path,str(rootfs)],
                           capture_output=True,timeout=40,check=False)
    if process.returncode or b'File not found' in process.stderr:
        return None
    return process.stdout

def run_deep_elf(repo,rootfs,report,relocations):
    launcher=ghidra_runner()
    if not launcher:return {'status':'Ghidra not installed; skipped'}
    summary={'status':'attempted temporary offline Ghidra imports','targets':{}}
    with tempfile.TemporaryDirectory(prefix='mpfree-v6-temp-ghidra-',
                                     dir=str(Path.home()/'mpfree-work')) as scratch:
        folder=Path(scratch);projects=folder/'projects';projects.mkdir()
        for label,(sourcepath,sha) in TARGET_SHA.items():
            entry={'sourcepath':sourcepath}
            summary['targets'][label]=entry
            cand=relocations['data'].get(sourcepath,{})
            if cand.get('status')!='ok' or not cand.get('function_candidates'):
                entry['status']='no verified text relocation candidates';continue
            data=elf_bytes(rootfs,sourcepath)
            if not data or hashlib.sha256(data).hexdigest()!=sha:
                entry['status']='source ELF SHA mismatch';continue
            source=folder/(label+'.elf');source.write_bytes(data);source.chmod(0o400)
            output=report/('v6-elf-'+label+'-main-recovery.json')
            logfile=report/('v6-elf-'+label+'-main-recovery.log')
            console=report/('v6-elf-'+label+'-main-recovery-console.txt')
            call=launcher+[str(projects),'MPFreeV6_'+label,'-import',str(source),
                           '-scriptPath',str(repo/'tools'/'ghidra'),
                           '-postScript','MPFreeELFMainRecoveryV6.py',
                           str(report/'v6-elf-relative-pointers.json'),sourcepath,
                           str(output),sha,
                           '-log',str(logfile)]
            code=safe_run(call,console,timeout=750)
            entry['code']=code
            entry['status']='ok' if code==0 and output.is_file() else 'Ghidra failure; see logs'
            if entry['status']=='ok':
                try:
                    d=json.loads(output.read_text())
                    entry['candidate_count']=len(d['candidates'])
                    entry['new_functions']=d['found_new_functions']
                    entry['decompiled']=d['successfully_decompiled']
                except Exception:entry['status']='malformed Ghidra output'
    return summary

def create_run(v5zip,repo,skip_ghidra=False,skip_rootfs_scan=False,rootfs_override=None):
    sources=verified_zip(v5zip)
    root=Path.home()/'mpfree-work'
    if not root.is_dir():raise ValueError('~/mpfree-work must exist')
    now=time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())
    report=root/('bootchain-wide-'+now+'-v6-'+str(os.getpid()))
    report.mkdir(mode=0o700)
    for name,contents in sources.items():
        destination=report/name
        destination.write_bytes(contents)
        destination.chmod(0o600)
    source_hash=hashlib.sha256(Path(v5zip).read_bytes()).hexdigest()
    (report/'v6-run-lineage.json').write_text(json.dumps({
        'source_zip_name':Path(v5zip).name,
        'source_zip_sha256':source_hash,
        'source_index_validated_entries':len(sources),
        'time_utc':datetime.now(timezone.utc).isoformat(),
        'no_device_access':True,
    },indent=2)+'\n')
    result={'stages':{},'rootfs_path':None}
    def stage(name,argv,timeout=600):
        code=safe_run(argv,report/(name+'-console.txt'),timeout)
        result['stages'][name]={'exit_code':code,'success':code==0}
        print(name,'OK' if code==0 else 'ERROR (archived logs)')
    try:
        stage('v6-static',[sys.executable,str(repo/'tools/mpc-one/offline-bootchain-v6-investigate.py'),str(report)])
        path=locate_rootfs(sources,rootfs_override)
        if rootfs_ok(path):
            # Rootfs path stays private and is NOT embedded in reports.
            result['rootfs_path']='verified regular-file ext4 (name withheld)'
            if not skip_rootfs_scan:
                stage('v6-startup-scan',[sys.executable,
                    str(repo/'tools/mpc-one/offline-bootchain-v6-startup-search.py'),
                    '--rootfs',str(path),'--report',str(report/'v6-startup-usage.json')],timeout=600)
            stage('v6-elf-relocations',[sys.executable,
                  str(repo/'tools/mpc-one/offline-bootchain-v6-elf-relocations.py'),
                  '--rootfs',str(path),'--report',str(report/'v6-elf-relative-pointers.json')],timeout=180)
            candfile=report/'v6-elf-relative-pointers.json'
            if not skip_ghidra and candfile.is_file():
                try:
                    info=json.loads(candfile.read_text())
                    result['stages']['v6-ghidra-elf-main']=run_deep_elf(repo,path,report,info)
                except Exception as exc:
                    (report/'v6-ghidra-elf-main-error.txt').write_text(type(exc).__name__+': '+str(exc)+'\n')
                    result['stages']['v6-ghidra-elf-main']={'error':type(exc).__name__}
        else:
            (report/'v6-no-verified-rootfs.txt').write_text(
             'No verified 3.10 ext4 rootfs; additional V6 rootfs/ELF tasks skipped.\n')
        result['offline_guarantee']='No MPC network/USB connections, ROM actions, eMMC writes, or vendor ELF execution.'
        (report/'v6-run-stages.json').write_text(json.dumps(result,indent=2)+'\n')
    finally:
        packager=repo/'tools/mpc-one/package-private-boot-audit.py'
        if packager.is_file():
            ret=safe_run([sys.executable,str(packager),str(report)],
                         report/'v6-packaging-console.txt',timeout=240)
            # The packager archives its own log as present at time of call;
            # any later writes to that log are not part of the index.
            if ret!=0:print('WARNING: ZIP packaging failed; private reports:',report)
    print('V6 single ZIP:',report.with_suffix('.zip'))
    return report.with_suffix('.zip')

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('prior_v5_zip',type=Path)
    p.add_argument('--skip-ghidra',action='store_true')
    p.add_argument('--skip-rootfs-scan',action='store_true')
    p.add_argument('--rootfs-310',type=Path)
    args=p.parse_args()
    repo=Path(__file__).resolve().parents[2]
    create_run(args.prior_v5_zip.expanduser(),repo,args.skip_ghidra,
               args.skip_rootfs_scan,args.rootfs_310)
if __name__=='__main__':main()