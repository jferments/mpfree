#!/usr/bin/env python3
"""MPFree 3.10.1 vs 3.10.0 deep *offline* security delta inspection.

Input: a prepared lab from setup-firmware-3-10-1-lab.sh, with extracted rootfs
images for both releases. Never mounts an image, executes any extracted ARM
binary, invokes an updater, connects to MPC, or flashes. All artifacts remain
private in lab/artifacts. Only hashes/differences (not firmware) enter ZIP.
"""
import argparse
import datetime
import difflib
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import zipfile
from pathlib import Path

VERSIONS = ('3.10.0','3.10.1')
ROOT = Path.home() / 'mpfree-work/firmware-labs'
FILES = (
 '/usr/bin/az01-bootloader-sig',
 '/usr/bin/az01-signed-fs',
 '/usr/sbin/az0x-loadpin-trustpoints',
 '/usr/lib/az0x/trusted-verity-digests',
 '/usr/lib/systemd/system/az0x-loadpin-trustpoints.service',
 '/usr/lib/systemd/system/acvs.service',
 '/opt/ramfs-update.squashfs',
)


def sha256(path):
    h=hashlib.sha256()
    with open(path,'rb') as f:
        for b in iter(lambda:f.read(1024*1024),b''):
            h.update(b)
    return h.hexdigest()


def run(args, timeout=120):
    return subprocess.run([str(x) for x in args], capture_output=True, text=True,
                          errors='replace', timeout=timeout)


def check_lab(arg):
    p=Path(arg).expanduser()
    if p.is_symlink() or not p.is_dir() or p.resolve().parent != ROOT.resolve():
        raise ValueError('lab must be a real directory directly under '+str(ROOT))
    p=p.resolve()
    for ver in VERSIONS:
        f=p/'reports'/(ver+'-rootfs-path.txt')
        if not f.is_file():raise ValueError('missing '+str(f))
        rootfs=Path(f.read_text().strip())
        if rootfs.is_symlink() or not rootfs.is_file() or p not in rootfs.resolve().parents:
            raise ValueError('unsafe rootfs: '+str(rootfs))
        with rootfs.open('rb') as fd:
            fd.seek(1080)
            if fd.read(2)!=b'\x53\xef':raise ValueError('not ext4: '+str(rootfs))
    return p


def get_rootfs(lab, ver):
    return Path((lab/'reports'/(ver+'-rootfs-path.txt')).read_text().strip())


def dump_regular(rootfs, path, dest):
    s=run(['debugfs','-R','stat '+path,rootfs],60)
    if 'Type: regular' not in s.stdout:return False
    dest.parent.mkdir(parents=True,exist_ok=True)
    if dest.exists() or dest.is_symlink():raise FileExistsError(dest)
    r=run(['debugfs','-R','dump '+path+' '+str(dest),rootfs],60)
    if r.returncode or not dest.is_file() or not stat.S_ISREG(dest.lstat().st_mode):
        raise RuntimeError('debugfs dump failed for '+path+': '+r.stderr[:350])
    return True


def tree_inventory(root):
    result={}
    for base, dirs, files in os.walk(root,followlinks=False):
        # os.walk does not traverse symlink dirs, but does list them as dirs.
        for name in dirs+files:
            p=Path(base)/name
            rel=p.relative_to(root).as_posix()
            st=p.lstat()
            row={'mode':oct(stat.S_IMODE(st.st_mode))}
            if stat.S_ISREG(st.st_mode):
                row.update(type='file',size=st.st_size,sha256=sha256(p))
            elif stat.S_ISLNK(st.st_mode):
                row.update(type='symlink',target=os.readlink(p))
            elif stat.S_ISDIR(st.st_mode):
                row.update(type='directory')
            else:
                row.update(type='special',kind=stat.S_IFMT(st.st_mode))
            result[rel]=row
    return result


def diff_binary(a,b):
    x=a.read_bytes();y=b.read_bytes()
    out={'bytes_a':len(x),'bytes_b':len(y),'sha_a':sha256(a),'sha_b':sha256(b)}
    same_size=len(x)==len(y)
    out['same_size']=same_size
    out['different_bytes_in_common_length']=sum(q!=r for q,r in zip(x,y))
    # Offsets only: do not put proprietary bytes into the report ZIP.
    spans=[];start=None;last=-1
    for i,(q,r) in enumerate(zip(x,y)):
        if q!=r:
            if start is None:start=i
            last=i
        elif start is not None:
            spans.append([start,last+1]);start=None
    if start is not None:spans.append([start,last+1])
    out['changed_offset_ranges']=spans[:250]
    out['changed_range_count']=len(spans)
    for label, f in [('old',a),('new',b)]:
        if shutil.which('readelf'):
            r=run(['readelf','-n',f],60)
            m=re.search(r'Build ID:\s*([0-9a-f]+)',r.stdout)
            out[label+'_build_id']=m.group(1) if m else None
    return out


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--lab',type=Path,required=True)
    args=ap.parse_args()
    os.umask(0o077)
    for x in ('debugfs','unsquashfs'):
        if not shutil.which(x):raise RuntimeError('required program missing: '+x)
    lab=check_lab(args.lab)
    stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    name='deep-delta-'+stamp+'-'+str(os.getpid())
    scratch=lab/'artifacts'/name
    reports=lab/'reports'/name
    scratch.mkdir(mode=0o700)
    reports.mkdir(mode=0o700)
    warnings=[]
    selected={}
    for ver in VERSIONS:
        rootfs=get_rootfs(lab,ver)
        selected[ver]={}
        print('Extracting selected files from '+ver+' (read-only debugfs)',flush=True)
        for path in FILES:
            dest=scratch/ver/path.lstrip('/')
            exists=dump_regular(rootfs,path,dest)
            selected[ver][path]={'status':'ok','sha256':sha256(dest),'bytes':dest.stat().st_size} if exists else {'status':'missing/nonregular'}
        fs=scratch/ver/'opt/ramfs-update.squashfs'
        if fs.is_file():
            dest=scratch/ver/'update-ramfs-extracted'
            print('Unpacking '+ver+' update RAM filesystem (not executing)',flush=True)
            r=run(['unsquashfs','-no-xattrs','-d',dest,fs],180)
            if r.returncode:
                warnings.append(ver+' unsquashfs exit '+str(r.returncode)+': '+r.stderr[:1000])
            if not dest.is_dir():raise RuntimeError('No squashfs extraction for '+ver)
    (reports/'selected-hashes.json').write_text(json.dumps(selected,indent=2)+'\n')
    for path in FILES:
        p0=scratch/VERSIONS[0]/path.lstrip('/')
        p1=scratch/VERSIONS[1]/path.lstrip('/')
        if p0.is_file() and p1.is_file() and p0.stat().st_size<5_000_000 and p1.stat().st_size<5_000_000:
            info=diff_binary(p0,p1)
            (reports/('byte-delta-'+p0.name+'.json')).write_text(json.dumps(info,indent=2)+'\n')
            if path.endswith('.service'):
                d='\n'.join(difflib.unified_diff(p0.read_text(errors='replace').splitlines(),
                    p1.read_text(errors='replace').splitlines(), fromfile='3.10.0'+path,tofile='3.10.1'+path))
                (reports/('service-diff-'+p0.name+'.txt')).write_text(d+'\n')
    inv={ver:tree_inventory(scratch/ver/'update-ramfs-extracted') for ver in VERSIONS}
    for ver in VERSIONS:
        (reports/(ver+'-ramfs-file-hashes.json')).write_text(json.dumps(inv[ver],indent=2)+'\n')
    before,after=inv[VERSIONS[0]],inv[VERSIONS[1]]
    allpaths=sorted(before.keys()|after.keys())
    changes=[]
    for k in allpaths:
        a=before.get(k); b=after.get(k)
        if a!=b:
            label=('added' if a is None else 'removed' if b is None else
                   'content' if a.get('sha256')!=b.get('sha256') else 'metadata')
            changes.append({'path':k,'category':label,'old':a,'new':b})
    (reports/'ramfs-changes.json').write_text(json.dumps(changes,indent=2)+'\n')
    counts={key:sum(x['category']==key for x in changes) for key in ('added','removed','content','metadata')}
    lines=['MPFree 3.10.1 deep security delta (host only; no MPC writes)',
           'Compared full uncompressed file contents and symlinks inside updater SquashFS, ignoring mtimes.',
           '3.10.0 items: '+str(len(before)), '3.10.1 items: '+str(len(after)),
           'Changed entries: '+str(len(changes)), 'Types: '+json.dumps(counts), '',
           'Changed paths:']
    for x in changes:lines.append('  '+x['category'].ljust(9)+x['path'])
    lines.extend(['','Rootfs selected file deltas:'])
    for k in FILES:
        x=selected['3.10.0'][k];y=selected['3.10.1'][k]
        state='SAME' if x.get('sha256') and x.get('sha256')==y.get('sha256') else 'CHANGED / ABSENT'
        lines.append('  '+state.ljust(20)+k)
    lines.extend(['','Unresolved: code-level cause of binary deltas; live bootloader state; signed rollback; ability to load MPFree kernel.',
                  'No firmware or proprietary extracted files are included in the report ZIP.',''])
    (reports/'SUMMARY.txt').write_text('\n'.join(lines))
    (reports/'warnings.txt').write_text('\n'.join(warnings)+'\n')
    archive=lab/('mpc-gen1-3101-deep-delta-'+stamp+'.zip')
    if archive.exists():raise FileExistsError(archive)
    with zipfile.ZipFile(archive,'x',zipfile.ZIP_DEFLATED,compresslevel=6) as zipf:
        for report in sorted(reports.iterdir()):
            if report.is_symlink() or not report.is_file() or report.suffix not in ('.txt','.json'):
                raise ValueError('unsafe report '+str(report))
            zipf.write(report,report.name)
    print('\n'.join(lines))
    print('PRIVATE ZIP TO UPLOAD:',archive)
    return 0

if __name__=='__main__':
    try:sys.exit(main())
    except (ValueError,RuntimeError,OSError,subprocess.TimeoutExpired) as e:
        print('ERROR:',str(e),file=sys.stderr);sys.exit(1)
