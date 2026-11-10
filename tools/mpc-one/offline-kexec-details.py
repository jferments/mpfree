#!/usr/bin/env python3
"""WORKSTATION ONLY, READ-ONLY: the device-tree nodes and kernel build configs
behind the newest kexec test build, for finding out why a device that works
under the running release fails in the test boot.

It takes the newest ~/mpfree-work/kexec-stick-*/ folder with a stick.img (the
build write-test-stick.py writes) and the files its report.txt names, and
prints:
  - the Ethernet, display, backlight and reserved-memory nodes, and every GPIO
    hog, of four device trees: the running release's board file, the new
    release's board file, the running tree (live.dtb) and the build's
    kexec.dtb. Each comes with the nodes it points to (offline-dtb.py show).
    The values of the MAC address, the serial number and the other properties
    `show` hides are never printed.
  - every difference between the two board files, with values. These are
    Akai's own files and hold nothing unit-specific.
  - the full kernel build config of the new kernel (the build's zImage) and
    of the running release's kernel (offline-kernel-config.py).

It only reads files and prints to stdout. It never contacts the MPC. Run it
through results.py, so its output lands in the round's folder.
"""
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
NODES = ("/ethernet@ff290000", "/reserved-memory", "/mipi-backlight", "/mipi@ff960000", "/vop@ff940000",
         "/display-subsystem")
REPORT_LINES = {
    "new release": r"^its kernel release: (\S+)$",
    "running release": r"^running release: (\S+)",
    "running tree": r"^running tree: (/\S+) \(",
    "new board file": r"^board file for the new kernel: (/\S+)$",
    "running board file": r"^board file of the running release: (/\S+)$",
}


class Stop(Exception):
    pass


def module(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


dtb = module("offline_dtb", "offline-dtb.py")
kconfig = module("offline_kernel_config", "offline-kernel-config.py")


def newest_build(work):
    """The newest kexec-stick folder with a stick.img (the rule write-test-stick.py uses) and its report's paths."""
    folders = sorted(d for d in os.listdir(work) if d.startswith("kexec-stick-")) if os.path.isdir(work) else []
    folders = [d for d in folders if os.path.isfile(os.path.join(work, d, "stick.img"))]
    if not folders:
        raise Stop(f"no kexec-stick folder with a stick.img under {work}")
    folder = os.path.join(work, folders[-1])
    with open(os.path.join(folder, "report.txt")) as fh:
        report = fh.read()
    found = {}
    for key, pattern in REPORT_LINES.items():
        hits = re.findall(pattern, report, re.M)
        if len(hits) != 1:
            raise Stop(f"{folder}/report.txt does not name the {key} once")
        found[key] = hits[0]
    return folder, found


def kernel_config(path, out):
    out.write(f"--- {path}\n")
    if not os.path.isfile(path):
        out.write("(no such file)\n")
        return
    with open(path, "rb") as fh:
        data = fh.read(kconfig.MAX_KERNEL + 1)
    try:
        if len(data) > kconfig.MAX_KERNEL:
            raise kconfig.ConfigError(f"larger than {kconfig.MAX_KERNEL} bytes")
        compression, text = kconfig.extract(data)
    except kconfig.ConfigError as err:
        out.write(f"(no config read: {err})\n")
        return
    out.write(f"(config found: {compression}; {len(text.splitlines())} lines)\n")
    out.write(text.rstrip("\n") + "\n")


def main(argv, out=sys.stdout):
    if argv:
        out.write(__doc__.strip() + "\n")
        return 2
    work = os.path.realpath(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    folder, paths = newest_build(work)
    trees = (("board file of the running release " + paths["running release"], paths["running board file"]),
             ("board file of the new release " + paths["new release"], paths["new board file"]),
             ("running tree, as U-Boot gave it to the running release", paths["running tree"]),
             ("the test build's kexec.dtb", os.path.join(folder, "kexec.dtb")))
    out.write("MPFree: device trees and kernel configs behind the newest kexec test build (read-only)\n")
    out.write(f"build: {folder}\n")
    for label, path in trees:
        out.write(f"{label}: {path}\n")
    for number, (label, path) in enumerate(trees, 1):
        out.write(f"\n=== {number}. Nodes of the {label} ===\n")
        try:
            tree = dtb.load(path)
        except (OSError, dtb.DtbError) as err:
            out.write(f"--- {path}\n(not read: {err})\n")
            continue
        hogs = [p for p, node in tree.walk() if node.prop("gpio-hog") is not None]
        dtb.show_nodes(path, list(NODES) + hogs, out)
        out.write(f"GPIO hogs: {', '.join(hogs) or 'none'}\n")
    out.write("\n=== 5. Differences between the two board files, with values ===\n")
    try:
        dtb.diff(paths["running board file"], paths["new board file"], True, out)
    except (OSError, dtb.DtbError) as err:
        out.write(f"(not compared: {err})\n")
    new_kernel = os.path.join(folder, "zImage")
    old_kernel = os.path.join(os.path.dirname(paths["running board file"]), "zImage-" + paths["running release"])
    out.write(f"\n=== 6. Kernel build config of the new release {paths['new release']} ===\n")
    kernel_config(new_kernel, out)
    out.write(f"\n=== 7. Kernel build config of the running release {paths['running release']} ===\n")
    kernel_config(old_kernel, out)
    out.write("\nDONE. Only files on this computer were read.\n")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
