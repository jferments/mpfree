#!/usr/bin/env python3
"""Preflight a cloned MPC ARM rootfs for generic QEMU ARM 'virt' lab use.

This is NOT an RK3288 SoC emulator. It never runs Akai binaries,
reboots MPC, contacts USB/network devices, or modifies owner backup.
Default = inspect only. --create-overlay explicitly creates a NEW qcow2
copy-on-write layer using a read-only regular-file ext4 backing image.
--kernel only prints a cautious QEMU command: it NEVER executes QEMU.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys

def is_regular_file(path):
    path=Path(path).expanduser()
    if path.is_symlink() or not path.is_file() or not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError("Require an existing regular file; never a symlink/device: "+str(path))
    return path

def ext4_image(path):
    path=is_regular_file(path)
    if not (20*1024*1024 <= path.stat().st_size <= 16*1024**3):
        raise ValueError("Unexpected rootfs image size: "+str(path.stat().st_size))
    with path.open("rb") as f:
        f.seek(1024+56)
        if f.read(2)!=b"\x53\xef":
            raise ValueError("Expected RAW ext4 superblock (not whole eMMC backup)")
    return path

def safe_workspace(value):
    base=(Path.home()/"mpfree-work").resolve()
    path=Path(value).expanduser()
    # The real resolved path must stay within the user's MPFree workdir.
    absolute=path.resolve()
    if absolute==base or base not in absolute.parents:
        raise ValueError("VM workspace must be a subdirectory of ~/mpfree-work")
    if path.is_symlink():
        raise ValueError("Refusing symlink workspace")
    return path

def boot_command(kernel,overlay):
    kernel=is_regular_file(kernel)
    overlay=is_regular_file(overlay)
    # This is an explicit example, not an assertion that stock Akai zImage
    # supports QEMU's PL011 / virtio / generic 'virt' DTB.
    return [
      "qemu-system-arm","-machine","virt","-cpu","cortex-a15",
      "-accel","tcg","-smp","2","-m","1024",
      "-nographic","-monitor","none","-nic","none","-no-reboot",
      "-kernel",str(kernel),
      "-append","console=ttyAMA0 root=/dev/vda rw rootfstype=ext4 init=/bin/sh",
      "-drive","if=none,id=rootdisk,format=qcow2,file="+str(overlay),
      "-device","virtio-blk-device,drive=rootdisk"
    ]

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--rootfs",type=Path,required=True,
                   help="An extracted RAW ext4 image, such as 3.10 rootfs.img, NOT mmcblk0.img")
    p.add_argument("--workspace",type=Path,
                   default=Path.home()/"mpfree-work"/"mpfree-arm-qemu-lab")
    p.add_argument("--name",default="mpc-310",help="overlay stem in workspace")
    p.add_argument("--create-overlay",action="store_true",
                   help="Opt in to creating a new COW layer; original rootfs stays untouched")
    p.add_argument("--kernel",type=Path,
                   help="Optional GENERIC QEMU 'virt' ARM32 zImage; only print boot command")
    args=p.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_.-]{1,60}",args.name):
        p.error("Overlay name must contain only alphanumeric characters, dot, underscore, hyphen")
    src=ext4_image(args.rootfs)
    workspace=safe_workspace(args.workspace)
    overlay=workspace/(args.name+".qcow2")
    tools={x:shutil.which(x) for x in ("qemu-system-arm","qemu-img","debugfs","qemu-arm-static")}
    report={
       "source":str(src),"image_bytes":src.stat().st_size,
       "source_kind":"RAW ext4 filesystem; NOT an RK3288 bootable disk/ROM image",
       "tools":tools,
       "qemu_board":"virt, ARMv7 Cortex-A15, NOT actual RK3288 Cortex-A17",
       "overlay":str(overlay),
       "limitations":[
         "Native Akai SPL/U-Boot and Rockchip-specific kernel/DTBs will not boot unmodified on QEMU virt.",
         "Use a dedicated generic ARM32 kernel with PL011, ext4 and virtio-block built in.",
         "Do not attach physical devices or give QEMU write access to owner original backup.",
         "No machine-mode probing, eFuse, boot0/boot1, USB ROM or DDR training is emulated.",
         "This tool never starts an emulator or executes an Akai binary.",
       ]
    }
    if args.create_overlay:
        if not tools["qemu-img"]:p.error("qemu-img not found; install qemu-utils on the Linux workstation")
        if overlay.exists() or overlay.is_symlink():
            p.error("Overlay already exists; refusing overwrite: "+str(overlay))
        workspace.mkdir(parents=True,exist_ok=True,mode=0o700)
        old=os.umask(0o077)
        try:
            command=["qemu-img","create","-f","qcow2","-F","raw",
                     "-b",str(src.resolve()),str(overlay)]
            result=subprocess.run(command,capture_output=True,text=True,check=False,timeout=60)
            if result.returncode:
                raise RuntimeError("qemu-img create failed: "+result.stderr[:500])
            overlay.chmod(0o600)
        finally:
            os.umask(old)
        report["overlay_created"]=True
    else:
        report["overlay_created"]=False
    if args.kernel:
        if overlay.is_file() and not overlay.is_symlink():
            import shlex
            report["qemu_boot_example_not_executed"]=" ".join(
                shlex.quote(arg) for arg in boot_command(args.kernel,overlay))
        else:
            report["qemu_boot_example_not_executed"] = (
                "Overlay not yet present; rerun --create-overlay to prepare. "
                "A QEMU 'virt'-compatible ARM32 kernel is required."
            )
    print(json.dumps(report,indent=2))
    print("\nNO MPC hardware or Akai executable was touched.")
    if report["overlay_created"]:
        print("Original extracted rootfs was NOT modified; the disposable qcow2 is separate.")
        print("For stronger isolation, run QEMU as a dedicated unprivileged account with "
              "NO write permission to the backing file and NO host device passthrough.")
    return 0

if __name__=="__main__":
    sys.exit(main())
