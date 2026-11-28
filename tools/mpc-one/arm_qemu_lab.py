#!/usr/bin/env python3
"""Build disposable *separate-copy* guest disk and optionally launch QEMU ARM.

Default is report-only. --prepare makes ONE private read-only copy and a
COW overlay. --launch starts ARM QEMU with NO network, USB, share, monitor,
physical device or vendor service autostart. Never calls an updater.
This is generic ARM virt, not an RK3288/BootROM emulator.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys

WORK=Path.home()/'mpfree-work/mpfree-arm-qemu-lab'
DEFAULT_ROOT=Path.home()/'mpfree-work/akai-unpacked-20261110T024913Z-1hjja3_r/rootfs.img'
BASE=WORK/'mpc-310-lab-base.ext4'
OVERLAY=WORK/'mpc-310-lab.qcow2'
KERNEL=WORK/'linux-arm-build/arch/arm/boot/zImage'

def regular(path):
    path=Path(path).expanduser()
    if path.is_symlink() or not path.is_file() or not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError('Refuse non-regular file or symlink: '+str(path))
    return path

def digest(path):
    h=hashlib.sha256()
    with regular(path).open('rb') as f:
        for chunk in iter(lambda:f.read(1024*1024),b''):h.update(chunk)
    return h.hexdigest()

def ext4(path):
    path=regular(path)
    with path.open('rb') as f:
        f.seek(1080)
        if f.read(2)!=b'\x53\xef':raise ValueError('Not raw ext4: '+str(path))
    if not 20*1024*1024 < path.stat().st_size < 4*1024**3:
        raise ValueError('Unexpected rootfs size')
    return path

def prepare(rootfs):
    rootfs=ext4(rootfs)
    if BASE.is_symlink() or OVERLAY.is_symlink():raise ValueError('Symlink in guest path')
    if BASE.exists() or OVERLAY.exists():
        raise ValueError('Guest lab base/overlay already exists; refusing overwrite. Existing preflight mpc-310.qcow2 is unaffected.')
    if not shutil.which('qemu-img'):raise ValueError('Install qemu-utils')
    WORK.mkdir(mode=0o700,parents=True,exist_ok=True)
    mask=os.umask(0o077)
    try:
        # Standalone copy so guest overlay never refers to owner source.
        # cp --reflink=auto may use CoW extents; it does not modify the source.
        p=subprocess.run(['cp','--reflink=auto','--sparse=always','--',str(rootfs),str(BASE)],
                         capture_output=True,text=True,timeout=90)
        if p.returncode:raise RuntimeError('Copy failed: '+p.stderr[:300])
        if digest(BASE)!=digest(rootfs):raise ValueError('Rootfs copy hash mismatch')
        BASE.chmod(0o400)
        p=subprocess.run(['qemu-img','create','-f','qcow2','-F','raw',
                          '-b',str(BASE.resolve()),str(OVERLAY)],
                         capture_output=True,text=True,timeout=60)
        if p.returncode:raise RuntimeError('qemu-img create failed: '+p.stderr[:300])
        OVERLAY.chmod(0o600)
    finally:os.umask(mask)
    return {'prepared':True,'base':str(BASE),'overlay':str(OVERLAY),
            'base_sha256':digest(BASE),'copy_from_owner_source':str(rootfs)}

def verify_overlay():
    ext4(BASE);regular(OVERLAY)
    p=subprocess.run(['qemu-img','info','--output=json',str(OVERLAY)],capture_output=True,
                     text=True,timeout=30)
    if p.returncode:raise RuntimeError('Unable to inspect qcow2 overlay')
    state=json.loads(p.stdout)
    if state.get('format')!='qcow2':raise ValueError('Wrong overlay format')
    if Path(state.get('backing-filename','')).resolve()!=BASE.resolve():
        raise ValueError('Overlay must back onto lab clone, NOT owner rootfs')
    return state

def qemu_args(kernel):
    regular(kernel);verify_overlay()
    return ['qemu-system-arm','-machine','virt','-cpu','cortex-a15',
            '-accel','tcg','-smp','2','-m','2048',
            '-display','none','-serial','stdio','-monitor','none',
            '-nic','none','-no-reboot',
            '-kernel',str(kernel),
            '-append','console=ttyAMA0,115200 earlycon=pl011,0x09000000 root=/dev/vda rw rootwait rootfstype=ext4 init=/bin/sh loglevel=6',
            '-drive','if=none,id=mpfree0,format=qcow2,file='+str(OVERLAY),
            '-device','virtio-blk-device,drive=mpfree0']

def main():
    p=argparse.ArgumentParser(description=__doc__)
    group=p.add_mutually_exclusive_group()
    group.add_argument('--prepare',action='store_true')
    group.add_argument('--launch',action='store_true',help='Explicitly start QEMU ARM system with disposable disk')
    group.add_argument('--launch-ephemeral',action='store_true',
                       help='Boot with a further temporary QEMU snapshot; discard ALL guest writes on exit')
    p.add_argument('--rootfs',type=Path,default=DEFAULT_ROOT)
    p.add_argument('--kernel',type=Path,default=KERNEL)
    args=p.parse_args()
    if args.prepare:
        print(json.dumps(prepare(args.rootfs),indent=2))
        print('No QEMU boot performed; cloned disk only.')
        return
    if args.launch or args.launch_ephemeral:
        if not shutil.which('qemu-system-arm'):p.error('Missing qemu-system-arm; install qemu-system-arm')
        cmd=qemu_args(args.kernel)
        if args.launch_ephemeral:
            cmd.insert(1,'-snapshot')
        print('Launching QEMU ARM virt: no network or host physical device. Ctrl-C exits.',flush=True)
        if args.launch_ephemeral:
            print('Ephemeral snapshot: guest changes discarded at QEMU exit, even the lab qcow2 stays untouched.',flush=True)
        print('First boot uses init=/bin/sh; no Akai services started.',flush=True)
        os.execvp(cmd[0],cmd)
    report={'qemu-system-arm':shutil.which('qemu-system-arm'),
            'qemu-img':shutil.which('qemu-img'),'kernel_exists':KERNEL.is_file(),
            'lab_base_exists':BASE.is_file(),'lab_overlay_exists':OVERLAY.is_file(),
            'existing_preflight_overlay':(WORK/'mpc-310.qcow2').is_file(),
            'no_changes_made':True}
    print(json.dumps(report,indent=2))
    print('Use --prepare once, build the ARM kernel, then --launch-ephemeral (recommended) or --launch.')
if __name__=='__main__':main()