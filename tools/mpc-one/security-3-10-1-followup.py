#!/usr/bin/env python3
"""Host-only, read-only MPC 3.10.1 vs 3.10.0 security/boot follow-up.

Uses existing extracted ext4 files from setup-firmware-3-10-1-lab.sh.
No mount, SSH, USB, flashing, updater execution, or QEMU guest execution.
Extracts individual files to a PRIVATE scratch directory in the lab, then
packages only TEXT findings (not proprietary binaries) as one ZIP.
"""
import argparse
import datetime as dt
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

HERE = Path(__file__).resolve().parent
LAB_BASE = Path.home() / "mpfree-work" / "firmware-labs"
VERSIONS = ("3.10.0", "3.10.1")
SELECTED_PATHS = (
    "/usr/sbin/az01-update",
    "/usr/bin/az01-bootloader-sig",
    "/usr/bin/az01-image",
    "/usr/bin/az01-signed-fs",
    "/usr/lib/libaz01-prog.so.1.3",
    "/usr/bin/az01-script-runner",
    "/usr/bin/az01-launch-MPC",
    "/usr/sbin/sshd",
    "/usr/lib/systemd/system/acvs.service",
    "/usr/lib/systemd/system/az01-script-runner.service",
    "/usr/lib/systemd/system/sshd.service",
    "/etc/ssh/sshd_config",
    "/etc/fw_env.config",
    "/opt/ramfs-update.squashfs",
)
INTERESTING = re.compile(
    r"kexec|lockdown|loadpin|trust|signature|signing|verify|verity|hash|secure|"
    r"bootloader|bootm|u-boot|recovery|fastboot|mmcblk|partition|rollback|"
    r"autorun|script|sudo|sshd|ssh|CAP_SYS_BOOT|CAP_SYS_ADMIN|image|update|flash",
    re.IGNORECASE,
)


def sha256(path):
    h = hashlib.sha256()
    with path.open("rb") as fd:
        for block in iter(lambda: fd.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def run(argv, timeout=150, env=None):
    return subprocess.run(argv, capture_output=True, text=True, errors="replace", timeout=timeout, env=env)


def safe_lab(arg):
    p = Path(arg).expanduser()
    if p.is_symlink() or not p.is_dir():
        raise ValueError("lab must be a real, existing directory: " + str(p))
    p = p.resolve()
    root = LAB_BASE.resolve()
    if p.parent != root:
        raise ValueError("lab must be directly under " + str(root))
    if not (p / "reports" / "3.10.1-rootfs-path.txt").is_file():
        raise ValueError("not a prepared 3.10.1 lab: " + str(p))
    return p


def source_rootfs(lab, version):
    f = lab / "reports" / (version + "-rootfs-path.txt")
    if not f.is_file():
        return None
    p = Path(f.read_text().strip())
    if p.is_symlink() or not p.is_file() or not stat.S_ISREG(p.lstat().st_mode):
        raise ValueError("unsafe/nonexistent filesystem: " + str(p))
    p = p.resolve()
    if lab not in p.parents:
        raise ValueError("source filesystem not within lab: " + str(p))
    with p.open("rb") as fd:
        fd.seek(1080)
        if fd.read(2) != b"\x53\xef":
            raise ValueError("not an ext filesystem: " + str(p))
    return p


def debugfs_read(rootfs, path, dest, log):
    stat_res = run(["debugfs", "-R", "stat " + path, str(rootfs)], timeout=30)
    if "Type: regular" not in stat_res.stdout:
        return False
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.exists():
        raise ValueError("scratch destination already exists: " + str(dest))
    result = run(["debugfs", "-R", "dump " + path + " " + str(dest), str(rootfs)], timeout=60)
    if result.returncode or not dest.is_file() or dest.stat().st_size == 0:
        log.append("Could not dump " + path + ": " + result.stderr[:200])
        dest.unlink(missing_ok=True)
        return False
    return True


def selected_strings(path, max_matches=260):
    if shutil.which("strings") is None:
        return ["strings command unavailable"]
    lines = []
    # subprocess streams output: do not collect entire binary string dump in memory.
    proc = subprocess.Popen(["strings", "-a", "-n", "6", str(path)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                            text=True, errors="replace")
    for line in proc.stdout:
        line = line.rstrip("\r\n")
        if INTERESTING.search(line):
            if len(lines) < max_matches:
                lines.append(line[:260])
    proc.stdout.close()
    proc.wait(timeout=90)
    return lines


def find_kernel_name(report):
    if not report.is_file():
        return "zImage-6.18.44-az01-2026-08-10-rt6"
    m = re.findall(r"(?m)^[a-f0-9]{64} (zImage-[A-Za-z0-9._-]+)$", report.read_text(errors="replace"))
    return m[0] if m else "zImage-6.18.44-az01-2026-08-10-rt6"


def squashfs_inventory(path):
    if not shutil.which("unsquashfs"):
        return "unsquashfs not installed\n"
    r = run(["unsquashfs", "-ll", str(path)], timeout=120)
    # Path inventory only. No scripts from the squashfs are executed.
    return (r.stdout[:1_500_000] + ("\n[truncated]\n" if len(r.stdout) > 1_500_000 else "") +
            ("\nEXIT " + str(r.returncode) + ": " + r.stderr[:500] if r.returncode else ""))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lab", type=Path, required=True, help="existing mpc-gen1-3101-* lab directory")
    ap.add_argument("--rerun-survey", action="store_true", help="rerun the corrected 3.10.1 rootfs survey")
    args = ap.parse_args()
    os.umask(0o077)
    lab = safe_lab(args.lab)
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out = lab / "reports" / ("security-followup-" + stamp + "-" + str(os.getpid()))
    scratch = lab / "artifacts" / ("security-followup-" + stamp + "-" + str(os.getpid()))
    out.mkdir(mode=0o700)
    scratch.mkdir(mode=0o700)
    log = []
    manifests = {}
    kernel_names = {}
    survey_status = "skipped"
    for ver in VERSIONS:
        rootfs = source_rootfs(lab, ver)
        if not rootfs:
            log.append(ver + ": baseline rootfs not found")
            continue
        print("Inspecting", ver, "rootfs", rootfs, flush=True)
        manifest = {}
        for path in SELECTED_PATHS:
            dst = scratch / ver / path.lstrip("/")
            if not debugfs_read(rootfs, path, dst, log):
                manifest[path] = {"status": "not regular, absent, or unreadable"}
                continue
            manifest[path] = {"status": "ok", "sha256": sha256(dst), "bytes": dst.stat().st_size}
            with dst.open("rb") as fd:
                is_elf = fd.read(4) == b"\x7fELF"
            if is_elf:
                (out / (ver + "-" + dst.name + "-security-strings.txt")).write_text(
                    "\n".join(selected_strings(dst)) + "\n")
            if path.endswith("ramfs-update.squashfs"):
                (out / (ver + "-update-ramfs-list.txt")).write_text(squashfs_inventory(dst))
        name = find_kernel_name(lab / "reports" / (ver + "-rootfs-survey-console.log"))
        kernel_names[ver] = name
        kernel = scratch / ver / "boot" / name
        if debugfs_read(rootfs, "/boot/" + name, kernel, log):
            manifest["/boot/" + name] = {"status": "ok", "sha256": sha256(kernel), "bytes": kernel.stat().st_size}
            config = run([sys.executable, str(HERE / "offline-kernel-config.py"), str(kernel)], timeout=90)
            (out / (ver + "-kernel.config.txt")).write_text(config.stdout + "\n" + config.stderr)
            selected = [x for x in config.stdout.splitlines() if re.match(
                r"^(?:# )?CONFIG_(KEXEC|KEXEC_CORE|KEXEC_FILE|KEXEC_FILE_UIMAGE|PREEMPT_RT|MODULE_SIG|SECURITY_"
                r"|LOCK_DOWN|LSM|IKCONFIG|EFI|IMA|INTEGRITY|BPF_LSM|CMDLINE|ARM_APPENDED_DTB)", x)]
            (out / (ver + "-kernel-security-config.txt")).write_text("\n".join(selected) + "\n")
            manifest["/boot/" + name]["config_extract_rc"] = config.returncode
        else:
            manifest["/boot/" + name] = {"status": "not found"}
        manifests[ver] = manifest
        (out / (ver + "-critical-hashes.json")).write_text(json.dumps(manifest, indent=2) + "\n")

    lines = ["MPFree 3.10.1 offline security follow-up", "No MPC or firmware image was modified.",
             "All extracted binaries are PRIVATE and excluded from the report ZIP.", ""]
    old, new = manifests.get("3.10.0", {}), manifests.get("3.10.1", {})
    for path in sorted(set(old) | set(new)):
        a, b = old.get(path, {}), new.get(path, {})
        if a.get("sha256") and b.get("sha256"):
            state = "SAME" if a["sha256"] == b["sha256"] else "CHANGED"
        else:
            state = "UNKNOWN/MISSING"
        lines.append(state.ljust(16) + " " + path)
    for suffix in ["kernel-security-config.txt", "update-ramfs-list.txt"]:
        p0 = out / ("3.10.0-" + suffix)
        p1 = out / ("3.10.1-" + suffix)
        if p0.is_file() and p1.is_file():
            d = "\n".join(difflib.unified_diff(p0.read_text().splitlines(), p1.read_text().splitlines(),
                                                fromfile="3.10.0/"+suffix, tofile="3.10.1/"+suffix))
            (out / ("diff-" + suffix)).write_text(d + "\n")
    for short in ["az01-update", "az01-bootloader-sig", "az01-image", "az01-signed-fs", "libaz01-prog.so.1.3", "ramfs-update.squashfs"]:
        s0, s1 = out / ("3.10.0-" + short + "-security-strings.txt"), out / ("3.10.1-" + short + "-security-strings.txt")
        if s0.is_file() and s1.is_file():
            (out / ("diff-" + short + "-security-strings.txt")).write_text("\n".join(difflib.unified_diff(
                s0.read_text().splitlines(), s1.read_text().splitlines(), fromfile="3.10.0/"+short,
                tofile="3.10.1/"+short)) + "\n")

    if args.rerun_survey and source_rootfs(lab, "3.10.1"):
        env = dict(os.environ, MPFREE_WORK_DIR=str(lab / "artifacts" / "3.10.1"))
        cmd = ["bash", str(HERE / "offline-rootfs-survey.sh"), "--image", str(source_rootfs(lab, "3.10.1"))]
        try:
            r = run(cmd, timeout=600, env=env)
            (out / "3.10.1-rootfs-survey-retry.log").write_text(r.stdout + "\n" + r.stderr)
            survey_status = "success" if r.returncode == 0 and "DONE. Report:" in r.stdout else "failed (see log)"
        except subprocess.TimeoutExpired as e:
            survey_status = "timeout"
            log.append(str(e))
        lines.append("3.10.1 full survey rerun: " + survey_status)
    lines.extend(["", "NOT PROVEN: physical kexec execution, bootloader state, rollback, firmware updater policies.",
                  "NOTE: equal rootfs kernel hashes do not prove the installed kernel is the same.", ""])
    (out / "SUMMARY.txt").write_text("\n".join(lines))
    (out / "warnings.txt").write_text("\n".join(log) + "\n")
    (out / "manifest.json").write_text(json.dumps({"lab": str(lab), "kernel_names": kernel_names,
                                                    "survey_status": survey_status, "notes": log}, indent=2) + "\n")
    archive = lab / ("mpc-gen1-3101-security-followup-" + stamp + ".zip")
    if archive.exists():
        raise FileExistsError(str(archive))
    with zipfile.ZipFile(archive, "x", zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for file in sorted(out.iterdir()):
            if not file.is_file() or file.is_symlink() or file.suffix not in (".txt", ".json", ".log"):
                raise ValueError("unexpected report file " + str(file))
            z.write(file, file.name)
    print("\n" + "\n".join(lines), flush=True)
    print("PRIVATE ZIP TO UPLOAD:", archive, flush=True)
    print("Generated report directory:", out, flush=True)
    return 0

if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.TimeoutExpired) as e:
        print("ERROR:", e, file=sys.stderr)
        sys.exit(1)
