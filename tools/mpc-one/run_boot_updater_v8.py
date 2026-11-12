#!/usr/bin/env python3
"""One ZIP, offline-only owner Rockchip P1/P3 + Akai 3.10 updater Ghidra V8.

Reads ONLY owner regular-file backup and extracted 3.10 rootfs; never runs
vendor code, mounts filesystems, contacts USB/MPC, flashes or changes ROM.
Temporary ELF imports/Ghidra projects are deleted. All reports stay PRIVATE.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time

P1_SHA='aa36e0e4033ce7386a210ad3e41f405a0d952a5b2a2c2711328450017414f1ec'
P3_SHA='b4d3940c9a6a7b75590c094278aa3735c5f97e84885a9f4780dd922d27d27b7a'
DEFAULT_BACKUP=Path.home()/'mpfree-backups/mpc-one-emmc-20261109T233604Z-datprr67/mmcblk0.img'
DEFAULT_ROOTFS=Path.home()/'mpfree-work/akai-unpacked-20261110T024913Z-1hjja3_r/rootfs.img'
SCRIPT='MPFreeDirectedBootUpdaterV8.py'
FILES={'az01-update':'/usr/sbin/az01-update',
       'libaz01-prog':'/usr/lib/libaz01-prog.so.1.3'}
KEYWORDS=('update','flash','boot','partition','program','erase','verify','signature',
          'recovery','fastboot','emmc','firmware','fw_','rescue','upgrade','install')

def regular(p):
    p=Path(p).expanduser()
    if p.is_symlink() or not p.exists() or not stat.S_ISREG(p.lstat().st_mode):
        raise ValueError('Only existing private regular files accepted: '+str(p))
    return p

def sha(data):return hashlib.sha256(data).hexdigest()

def subprocess_log(argv,log,timeout=900):
    try:
        r=subprocess.run(argv,capture_output=True,timeout=timeout,check=False)
        log.write_bytes((r.stdout+b'\n---stderr---\n'+r.stderr)[:4*1024*1024])
        return r.returncode
    except (OSError,subprocess.TimeoutExpired) as e:
        log.write_text(type(e).__name__+': '+str(e)[:800]+'\n')
        return -1

def grab_debugfs(ext4,filename):
    r=subprocess.run(['debugfs','-R','cat '+filename,str(ext4)],capture_output=True,timeout=50)
    if r.returncode or b'File not found' in r.stderr or b'not found by ext2_lookup' in r.stderr:
        raise ValueError('debugfs failed to read '+filename+': '+r.stderr.decode('utf8','replace')[:180])
    return r.stdout

def get_ghidra():
    env=os.environ.get('GHIDRA_HOME')
    if env:
        exe=Path(env)/'support/pyghidraRun'
        if exe.is_file():return [str(exe),'-H']
    if shutil.which('snap'):
        p=subprocess.run(['snap','list','ghidra'],capture_output=True,timeout=15)
        if p.returncode==0:return ['snap','run','ghidra.ghidra','-H']
    return None

def projects_from(folder,pattern):
    matches=sorted(folder.glob(pattern),key=lambda p:p.stat().st_mtime,reverse=True)
    return matches[0].stem if matches else None

def run_boot_project(runner,root,report,project_dir,kind,pattern,filename,sha256,limit):
    project=projects_from(project_dir,pattern)
    status={'source_stage':kind,'project_found':bool(project),'source_sha256':sha256}
    if not project:return status
    dest=report/('v8-ghidra-'+kind+'.json')
    cmd=runner+[str(project_dir),project,'-process',filename,'-readOnly','-noanalysis',
        '-scriptPath',str(root/'tools/ghidra'),'-postScript',SCRIPT,kind,str(dest),sha256,str(limit),
        '-log',str(report/('v8-ghidra-'+kind+'.log'))]
    code=subprocess_log(cmd,report/('v8-ghidra-'+kind+'-console.txt'),timeout=950)
    status.update({'project':project,'exit_code':code,'success':code==0 and dest.is_file()})
    return status

def run_imported_elf(runner,root,report,temporary,label,data,max_functions):
    tag=label if label!='ramfs-updater' else label+'-'+sha(data)[:12]
    exe=temporary/(tag+'.elf');exe.write_bytes(data);exe.chmod(0o400)
    project_dir=temporary/'ghidra-projects';project_dir.mkdir(exist_ok=True)
    dst=report/('v8-ghidra-'+tag+'.json')
    cmd=runner+[str(project_dir),'MPFreeV8_'+tag,'-import',str(exe),
         '-scriptPath',str(root/'tools/ghidra'),'-postScript',SCRIPT,label,str(dst),sha(data),str(max_functions),
         '-log',str(report/('v8-ghidra-'+tag+'.log'))]
    code=subprocess_log(cmd,report/('v8-ghidra-'+tag+'-console.txt'),timeout=1200)
    return {'sha256':sha(data),'bytes':len(data),'exit_code':code,
            'report':dst.name,'success':code==0 and dst.is_file()}

def inspect_squashfs(rootfs,temporary,report):
    result={'present':False,'files':[],'selected':[],'warnings':[]}
    try:
        blob=grab_debugfs(rootfs,'/opt/ramfs-update.squashfs')
    except ValueError as exc:
        result['warnings'].append(type(exc).__name__+' (updater RAMFS missing)')
        return result,[]
    result.update({'present':True,'source_bytes':len(blob),'source_sha256':sha(blob)})
    path=temporary/'ramfs-update.squashfs';path.write_bytes(blob)
    if not shutil.which('unsquashfs'):
        result['warnings'].append('unsquashfs not installed; apt install squashfs-tools')
        return result,[]
    p=subprocess.run(['unsquashfs','-l',str(path)],capture_output=True,timeout=120)
    if p.returncode:
        result['warnings'].append('Unable to list ramfs-update.squashfs')
        return result,[]
    # Full RAMFS path inventory, *not* only updater-named files.
    all_paths=[]
    for row in p.stdout.decode('utf8','replace').splitlines():
        line=row.strip()
        if not line.startswith('squashfs-root/'):continue
        relative=line.removeprefix('squashfs-root/')
        if not relative or '..' in Path(relative).parts or relative.startswith('/') or '\x00' in relative:
            continue
        all_paths.append(relative)
    result['total_listed_paths']=len(all_paths)
    result['all_paths']=all_paths[:2400]
    result['all_paths_truncated']=len(all_paths)>2400
    items=[path for path in all_paths if any(k in path.lower() for k in KEYWORDS)]
    result['file_count_with_update_names']=len(items)
    result['files']=items[:350]
    # Minimal startup/config candidates, including /init and rcS where
    # names lack any explicit update-related keyword.
    def priority(path):
        low=path.lower()
        if low in ('init','linuxrc','etc/inittab','etc/init.d/rcs'):return 0
        if any(k in low for k in ('update','reboot','init.d','boot','program','flash','config')):return 1
        if Path(low).suffix in ('.sh','.service','.conf','.json','.py','.txt'):return 2
        return 3
    inspect_paths=sorted(set(items+[
        path for path in all_paths
        if path.lower() in ('init','linuxrc','etc/inittab','etc/init.d/rcs')
        or path.lower().startswith(('etc/init.d/','etc/systemd/'))
        or Path(path).suffix.lower() in ('.sh','.service','.conf','.json','.py','.txt')
    ]),key=lambda path:(priority(path),path))
    selected=[]
    # Extract bytes only through unsquashfs -cat to temporary memory/files.
    # Never extract or follow links in the squashfs image.
    for name in inspect_paths[:120]:
        ext=Path(name).suffix.lower()
        if not (ext in ('.sh','.service','.conf','.json','.py','.txt','.elf','.so') or
                any(k in name.lower() for k in ('update','flash','bootloader','init','fastboot','reboot','linuxrc','inittab'))):
            continue
        p=subprocess.run(['unsquashfs','-cat',str(path),name],capture_output=True,timeout=35)
        if p.returncode or not p.stdout or len(p.stdout)>8*1024*1024:continue
        record={'path':name,'bytes':len(p.stdout),'sha256':sha(p.stdout),
                'is_elf':p.stdout.startswith(b'\x7fELF')}
        if not record['is_elf']:
            printable=p.stdout[:18000].decode('utf-8','replace')
            # Prefer a list of matching lines, never full scripts or keys.
            record['interesting_lines']=[re.sub(r'(?i)(password|token|secret|key)=\S+',
                                r'\1=[REDACTED]',x)[:240]
                for x in printable.splitlines()
                if any(k in x.lower() for k in KEYWORDS)][:80]
        selected.append((name,p.stdout,record))
        result['selected'].append(record)
    return result,selected

def write_report(folder,name,data):
    file=folder/name;file.write_text(json.dumps(data,indent=2)+'\n');file.chmod(0o600)

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--rootfs',type=Path,default=DEFAULT_ROOTFS)
    ap.add_argument('--backup',type=Path,default=DEFAULT_BACKUP)
    ap.add_argument('--no-boot-ghidra',action='store_true')
    ap.add_argument('--max-inner-elfs',type=int,default=5)
    args=ap.parse_args()
    if not 0<=args.max_inner_elfs<=10:ap.error('--max-inner-elfs must be 0..10')
    repo=Path(__file__).resolve().parents[2]
    backup=regular(args.backup);rootfs=regular(args.rootfs)
    with backup.open('rb') as f:
        f.seek(64*512)
        if sha(f.read(128*512))!=P1_SHA:ap.error('P1 hash mismatch against known owner backup')
        f.seek(256*512+64)
        if sha(f.read(363649))!=P3_SHA:ap.error('P3 image SHA mismatch against known owner backup')
    with rootfs.open('rb') as f:
        f.seek(1080)
        if f.read(2)!=b'\x53\xef':ap.error('Expected regular-file raw ext4 rootfs')
    boot_info=subprocess.run(['debugfs','-R','ls -p /boot',str(rootfs)],capture_output=True,timeout=30)
    if b'6.18.44' not in boot_info.stdout:ap.error('Unknown rootfs (expected 3.10 6.18.44 kernel listing)')
    work=Path.home()/'mpfree-work'
    if not work.is_dir():ap.error('Missing ~/mpfree-work')
    stamp=time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())
    report=work/('bootchain-wide-'+stamp+'-v8-'+str(os.getpid()))
    report.mkdir(mode=0o700)
    runner=get_ghidra()
    index={'schema':'mpfree-v8-owner-updater-boot-investigation',
           'utc':datetime.now(timezone.utc).isoformat(),
           'no_device_access':True,'rootfs_size':rootfs.stat().st_size,
           'stages':{},'limits':['All work offline; executable files only analyzed, never run.',
             'Static presence of partition-writing code is NOT proof of updater behavior.',
             'Reports contain private proprietary excerpts and must not be committed.',
             'Genuine MaskROM, security fuse state, full eMMC recovery require physical evidence.']}
    try:
        with tempfile.TemporaryDirectory(prefix='mpfree-v8-updater-',dir=str(work)) as tmp:
            temporary=Path(tmp)
            squash,files=inspect_squashfs(rootfs,temporary,report)
            write_report(report,'v8-updater-ramfs-inventory.json',squash)
            index['stages']['ramfs_inventory']={'ok':True,'entries':len(squash['selected'])}
            for role,filename in FILES.items():
                try:
                    data=grab_debugfs(rootfs,filename)
                    if data[:4]!=b'\x7fELF' or len(data)>18*1024*1024:raise ValueError('Not expected ELF')
                    statinfo={'elf_path':filename,'sha256':sha(data),'bytes':len(data),'is_elf':True}
                    if runner:
                        statinfo.update(run_imported_elf(runner,repo,report,temporary,role,data,
                                       240 if role=='az01-update' else 340))
                    else:statinfo['skipped']='No Ghidra launcher detected'
                    index['stages'][role]=statinfo
                except Exception as exc:index['stages'][role]={'error':type(exc).__name__+': '+str(exc)[:200]}
            inner_elfs=0
            for path,blob,entry in files:
                if not entry['is_elf'] or inner_elfs>=args.max_inner_elfs:continue
                role='ramfs-updater';inner_elfs+=1
                tag='ramfs-updater-'+str(inner_elfs)
                # Ghidra script treats role as ramfs-updater; output names
                # derived from path tag are unique but contain NO vendor binary.
                if runner:
                    try:
                        res=run_imported_elf(runner,repo,report,temporary,role,blob,180)
                        index['stages'][tag]={'path':path,**res}
                    except Exception as exc:index['stages'][tag]={'path':path,'error':str(exc)[:180]}
            if not args.no_boot_ghidra and runner:
                project_dir=work/'ghidra/projects'
                if project_dir.is_dir():
                    for role,pattern,filename,digest,limit in (
                      ('spl','MPFree_P1_SRAM_*.gpr','p1-rk3288-sram-stage.bin',
                        '6faade107d83bb8f0782bf4df0cf86e3aee772b3e52c587d6a1ef83062fa652f',180),
                      ('uboot','MPFree_P3_Uboot_*.gpr','p3-uimage-payload.bin',P3_SHA,260)):
                        index['stages'][role]=run_boot_project(runner,repo,report,project_dir,
                                                 role,pattern,filename,digest,limit)
                else:index['stages']['boot']='No saved P1/P3 Ghidra projects available'
            elif not runner:index['stages']['ghidra']='No headless PyGhidra launcher detected'
            else:index['stages']['boot']='Explicit --no-boot-ghidra'
    finally:
        write_report(report,'v8-stage-manifest.json',index)
        packager=repo/'tools/mpc-one/package-private-boot-audit.py'
        if packager.is_file():
            import importlib.util
            spec=importlib.util.spec_from_file_location('mpfree_private_v8_pack',packager)
            pack=importlib.util.module_from_spec(spec);spec.loader.exec_module(pack)
            output,idx=pack.package(report)
            print('V8 PRIVATE ZIP:',output,'files:',len(idx['files']))
        else:print('WARNING: packager absent, files remain in',report)
    print('Completed offline. No MPC touched. Private Ghidra reports:',report)
if __name__=='__main__':main()