#!/usr/bin/env python3
"""One-command MPFree offline bootchain + ARM QEMU research bundle.

Runs the existing broad SPL/U-Boot audit, V11 focused updater audit,
and a scripted read-only QEMU ARM guest probe. Captures all stage stdout,
Ghidra logs, guest serial output and SHA-256 indexed reports into ONE ZIP.
No SSH, USB, device passthrough, firmware execution or physical writes.
QEMU is a generic ARM virt machine, NOT an RK3288/MaskROM emulator.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import pty
import re
import select
import shutil
import signal
import stat
import subprocess
import sys
import time
import traceback
import zipfile

ROOT = Path(__file__).resolve().parents[2]
WORK = Path.home() / "mpfree-work"
ALLOWED = {".txt", ".json", ".log", ".md", ".csv", ".tsv"}
MAX_FILE = 96 * 1024 * 1024
MAX_TOTAL = 384 * 1024 * 1024
DEFAULT_V10 = WORK / "bootchain-wide-20261112T014736Z-v10-3708829.zip"


def stamp():
    return time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())


def sha256(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def regular(path):
    p = Path(path).expanduser()
    if p.is_symlink() or not p.is_file() or not stat.S_ISREG(p.lstat().st_mode):
        raise ValueError("Expected regular file, not link/device: " + str(p))
    return p


def load_module(rel, name):
    spec = importlib.util.spec_from_file_location(name, ROOT / rel)
    obj = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(obj)
    return obj


class Lab:
    def __init__(self, args):
        self.args = args
        WORK.mkdir(parents=True, exist_ok=True)
        self.out = WORK / ("bootchain-wide-integrated-" + stamp() + "-" + str(os.getpid()))
        self.out.mkdir(mode=0o700)
        self.full_file = (self.out / "session-console.log").open("w", encoding="utf-8")
        self.stages = {}
        self.notes = []
        self.new_archives = []
        self.start = time.time()
        self.manifest = {
            "schema": "mpfree-bootchain-integrated-v1",
            "date_utc": stamp(),
            "repository": "jferments/mpfree",
            "branch": "research/wide-bootchain-audit-20261111",
            "physical_device_access": False,
            "vendor_updater_executed": False,
            "qemu_board": "generic ARM virt (NOT MPC/RK3288)",
            "stages": self.stages,
            "notes": self.notes,
        }

    def say(self, value):
        line = str(value)
        print(line, flush=True)
        self.full_file.write(line + "\n")
        self.full_file.flush()

    def run(self, name, argv, env=None, cwd=None):
        dest = self.out / ("stage-" + name + "-console.log")
        self.say("\n" + "=" * 65)
        self.say("START " + name + ": " + " ".join(map(str, argv)))
        self.say("=" * 65)
        started = time.monotonic()
        rc = None
        try:
            with dest.open("w", encoding="utf-8") as f:
                p = subprocess.Popen(
                    list(map(str, argv)), cwd=cwd, env=env,
                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                    text=True, errors="replace", bufsize=1,
                    start_new_session=True,
                )
                try:
                    for line in p.stdout:
                        f.write(line)
                        self.full_file.write(line)
                        self.full_file.flush()
                        sys.stdout.write(line)
                        sys.stdout.flush()
                    rc = p.wait()
                except KeyboardInterrupt:
                    os.killpg(p.pid, signal.SIGTERM)
                    p.wait(timeout=15)
                    raise
        except Exception as exc:
            self.say("STAGE EXCEPTION " + name + ": " + repr(exc))
            self.notes.append(name + ": " + str(exc))
        self.stages[name] = {
            "success": rc == 0, "exit_code": rc,
            "duration_seconds": round(time.monotonic() - started, 1),
            "console": dest.name,
        }
        self.say("END " + name + " rc=" + str(rc))
        return rc == 0

    def snapshot(self):
        return {p.name for p in WORK.iterdir() if p.name.startswith("bootchain-wide-")}

    def import_archive(self, archive, label):
        """Copy only SHA-indexed, verified text reports; never binary firmware."""
        archive = regular(archive)
        target = self.out / "static" / label
        target.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(archive) as z:
            if z.testzip():
                raise ValueError("ZIP CRC verification failed: " + archive.name)
            idx = json.loads(z.read("ZIP_INDEX.json"))
            rows = idx.get("files")
            if not isinstance(rows, list) or not rows:
                raise ValueError("Archive has no SHA-indexed files")
            names = set()
            total = 0
            for row in rows:
                name = row["name"]
                rel = PurePosixPath(name)
                if (rel.is_absolute() or ".." in rel.parts or
                        not rel.parts or name in names or
                        Path(name).suffix.lower() not in ALLOWED):
                    raise ValueError("Unexpected report path: " + name)
                names.add(name)
                meta = z.getinfo(name)
                if meta.is_dir() or meta.file_size != row["bytes"] or meta.file_size > MAX_FILE:
                    raise ValueError("Unexpected report size: " + name)
                total += meta.file_size
                if total > MAX_TOTAL:
                    raise ValueError("Too many report bytes in input")
                data = z.read(name)
                if hashlib.sha256(data).hexdigest() != row["sha256"]:
                    raise ValueError("SHA mismatch: " + name)
                result = target.joinpath(*rel.parts)
                result.parent.mkdir(parents=True, exist_ok=True)
                result.write_bytes(data)
            (target / "archive-provenance.json").write_text(json.dumps({
                "source_archive_name": archive.name,
                "source_archive_sha256": sha256(archive),
                "verified_files": len(rows),
                "verified_bytes": total,
            }, indent=2) + "\n")
        self.new_archives.append(archive.name)
        self.say("Imported " + str(len(rows)) + " SHA-verified " + label + " reports.")

    def import_new_archives(self, before, label):
        after = self.snapshot()
        found = sorted(
            (WORK / name for name in after - before
             if name.endswith(".zip") and (WORK / name).is_file()),
            key=lambda x: x.stat().st_mtime,
        )
        if not found:
            self.notes.append("No new " + label + " ZIP found; stage may have failed.")
            return False
        ok = True
        for n, archive in enumerate(found):
            try:
                self.import_archive(archive, label if len(found) == 1 else label + "-" + str(n))
            except Exception as exc:
                self.say("Unable to import " + archive.name + ": " + repr(exc))
                self.notes.append("Could not import " + archive.name + ": " + str(exc))
                ok = False
        return ok

    def get_prior_v10(self):
        candidates = []
        if self.args.prior_v10:
            candidates = [self.args.prior_v10]
        else:
            if DEFAULT_V10.exists():
                candidates.append(DEFAULT_V10)
            candidates += sorted(WORK.glob("bootchain-wide-*-v10-*.zip"),
                                 key=lambda p: p.stat().st_mtime, reverse=True)
        v11 = load_module("tools/mpc-one/run_boot_updater_v11.py", "mpfree_integrated_v11")
        errors = []
        seen = set()
        for candidate in candidates:
            if str(candidate) in seen:
                continue
            seen.add(str(candidate))
            try:
                result = v11.check_previous(candidate)
                self.say("Verified prior V10 evidence: " + str(result))
                return Path(candidate)
            except Exception as exc:
                errors.append(str(candidate) + ": " + str(exc))
        if errors:
            self.notes.extend(["Rejected V10 archive: " + item for item in errors[:3]])
        return None

    def static(self):
        env = os.environ.copy()
        # V11 separately imports updater ELFs. Avoid duplicate V5 import in broad pass.
        env["MPFREE_V5_SKIP_ELF_GHIDRA"] = "1"
        env["MPFREE_AUTO_EXTRACT_310"] = "0"
        self.say("[STATIC] Broad P1/P2/P3/DTB/boot environment/function coverage audit.")
        before = self.snapshot()
        self.run("wide-ghidra",
                 ["bash", ROOT / "tools/ghidra/run_wide_bootchain_audit.sh"],
                 env=env, cwd=ROOT)
        self.import_new_archives(before, "wide")
        self.say("[STATIC] Targeted updater, ROM/recovery and callback-table V11 audit.")
        prior = self.get_prior_v10()
        if prior is None:
            self.say("No verified V10 archive found. Running fresh V10 baseline first.")
            before = self.snapshot()
            ok = self.run("v10-baseline", [
                sys.executable, ROOT / "tools/mpc-one/run_boot_updater_v10.py",
            ], cwd=ROOT)
            self.import_new_archives(before, "v10")
            if ok:
                prior = self.get_prior_v10()
        if prior is None:
            self.stages["v11-ghidra"] = {"success": False, "skipped": True,
                                         "reason": "Verified prior V10 archive unavailable"}
            self.notes.append("V11 skipped: verified V10 prerequisite missing.")
            return
        before = self.snapshot()
        self.run("v11-ghidra", [
            sys.executable, ROOT / "tools/mpc-one/run_boot_updater_v11.py",
            "--prior-v10", prior,
        ], cwd=ROOT)
        self.import_new_archives(before, "v11")

    def preflight_qemu(self):
        qemu = load_module("tools/mpc-one/arm_qemu_lab.py", "mpfree_integrated_qemu")
        for binary in ("qemu-system-arm", "qemu-img"):
            if not shutil.which(binary):
                raise RuntimeError(binary + " not installed")
        args = qemu.qemu_args(qemu.KERNEL)
        # -snapshot avoids writing to the QEMU lab overlay; this is a clone
        # of a verified user rootfs and is never a host block device.
        args.insert(1, "-snapshot")
        return args, qemu

    def qemu(self):
        self.say("[QEMU] Generic ARM guest boot and read-only inventory. No vendor executable.")
        guest = self.out / "qemu-serial-console.log"
        result = {"success": False, "booted": False,
                  "probe_complete": False, "no_vendor_binary_executed": True,
                  "read_only_host_access": True, "ephemeral_disk": True,
                  "no_network": True, "no_usb_or_host_device_passthrough": True,
                  "limitations": "Generic virt tests Linux userspace; not RK3288 BootROM/SPL/U-Boot."}
        proc = None
        master = None
        slave = None
        transcript = []
        try:
            command, q = self.preflight_qemu()
            result["kernel_sha256"] = sha256(q.KERNEL)
            result["lab_rootfs_sha256"] = sha256(q.BASE)
            self.say("QEMU boot: cloned 3.10 rootfs, Cortex-A15, no services, -snapshot.")
            master, slave = pty.openpty()
            proc = subprocess.Popen(
                command, stdin=slave, stdout=slave, stderr=slave,
                start_new_session=True, close_fds=True,
            )
            os.close(slave)
            slave = None
            token = str(os.getpid()) + "X" + str(int(self.start))
            begin = "MPFREE_VM_BEGIN_" + token
            end = "MPFREE_VM_DONE_" + token
            lines = [
                "echo " + begin,
                "echo MPFREE_VM_SECTION_kernel",
                "uname -a",
                "cat /proc/version",
                "cat /proc/cmdline",
                "echo MPFREE_VM_SECTION_mounts",
                "mount -t proc proc /proc 2>&1 || true",
                "mount -t sysfs sysfs /sys 2>&1 || true",
                "cat /proc/mounts",
                "echo MPFREE_VM_SECTION_cpu_memory",
                "cat /proc/cpuinfo",
                "cat /proc/meminfo",
                "echo MPFREE_VM_SECTION_vendor_paths",
                "for p in /usr/sbin/az01-update /usr/lib/libaz01-prog.so.1.3 /usr/sbin/init /opt/ramfs-update.squashfs /usr/bin/az01-bootloader-sig /usr/sbin/az01config; do ls -ld \"\$p\" 2>&1; done",
                "echo MPFREE_VM_SECTION_vendor_hashes",
                "for p in /usr/sbin/az01-update /usr/lib/libaz01-prog.so.1.3 /opt/ramfs-update.squashfs; do if test -f \"\$p\"; then sha256sum \"\$p\"; fi; done",
                "echo MPFREE_VM_SECTION_init_and_libraries",
                "cat /etc/os-release 2>&1 || true",
                "ls -l /lib/ld-linux-armhf.so.3 /lib/libc.so.6 /usr/lib/libaz01-prog.so.1.3 2>&1",
                "echo MPFREE_VM_SECTION_virtual_devices",
                "ls -l /dev/vda /dev/mmcblk0 /dev/bus/usb /sys/bus/usb/devices 2>&1",
                "echo MPFREE_VM_SECTION_available_tools",
                "for p in sha256sum readelf strace base64 awk sed grep; do command -v \"\$p\" || true; done",
                "echo MPFREE_VM_SECTION_completed",
                "echo " + end,
            ]
            # Only shell builtins and passive inspection tools. Do not run
            # az01-update, az01config, program-fuses, or AKAI init/services.
            # The guest's rootfs writes (such as mounts) are ephemeral.
            boot_deadline = time.monotonic() + self.args.qemu_boot_timeout
            idle_deadline = None
            sent = False
            data = ""
            with guest.open("w", encoding="utf-8", errors="replace") as log:
                while True:
                    now = time.monotonic()
                    if proc.poll() is not None:
                        result["error"] = "QEMU exited unexpectedly: " + str(proc.returncode)
                        break
                    if not sent and now > boot_deadline:
                        result["error"] = "Guest shell not detected before boot timeout"
                        break
                    if sent and idle_deadline and now > idle_deadline:
                        result["error"] = "Guest scripted probe timed out"
                        break
                    ready, _, _ = select.select([master], [], [], 0.3)
                    if ready:
                        try:
                            chunk = os.read(master, 65536)
                        except OSError:
                            break
                        if not chunk:
                            break
                        text = chunk.decode("utf-8", errors="replace")
                        log.write(text)
                        log.flush()
                        data += text
                        if len(data) > 8 * 1024 * 1024:
                            data = data[-8 * 1024 * 1024:]
                        if not sent and re.search(r"(?:^|[\r\n])(?:/ # |# )", data[-4000:]):
                            result["booted"] = True
                            sent = True
                            idle_deadline = time.monotonic() + self.args.qemu_probe_timeout
                            for line in lines:
                                wire = (line + "\n").encode("utf-8")
                                # pty user input: keep under terminal line length
                                os.write(master, wire)
                                time.sleep(0.065)
                            self.say("QEMU guest shell detected. Sent passive probes.")
                        if sent and re.search(r"(?m)^" + re.escape(end) + r"\r?$", data):
                            result["probe_complete"] = True
                            result["success"] = True
                            break
                    if not sent and now + 5 > boot_deadline:
                        # Final harmless shell-input nudge for nonstandard prompt.
                        os.write(master, b"\n")
                transcript = data.splitlines()
            result["serial_log"] = guest.name
            result["observed_sections"] = [
                s.strip() for s in re.findall(r"MPFREE_VM_SECTION_([a-z_]+)", data)
            ]
            result["observed_sections"] = sorted(set(result["observed_sections"]))
            result["guest_probe_commands"] = lines
        except Exception as exc:
            result["error"] = repr(exc)
            self.say("QEMU probe exception: " + repr(exc))
        finally:
            if proc is not None and proc.poll() is None:
                try:
                    os.killpg(proc.pid, signal.SIGTERM)
                    proc.wait(timeout=15)
                except Exception:
                    try:
                        os.killpg(proc.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    proc.wait(timeout=15)
            if slave is not None:
                os.close(slave)
            if master is not None:
                os.close(master)
            (self.out / "qemu-probe-results.json").write_text(
                json.dumps(result, indent=2) + "\n")
            self.stages["qemu-guest"] = {
                "success": result["success"],
                "booted": result["booted"],
                "probe_complete": result["probe_complete"],
                "error": result.get("error"),
                "console": guest.name,
            }
            self.say("QEMU RESULT: " + json.dumps(self.stages["qemu-guest"]))
        return result["success"]

    def package(self):
        self.manifest["duration_seconds"] = round(time.time() - self.start, 1)
        self.manifest["source_archives_imported"] = self.new_archives
        self.manifest["safety_limitations"] = [
            "No claimed silicon MaskROM proof, eMMC restore or original SPL execution.",
            "Guest commands never invoke vendor maintenance or update executables.",
            "Do not commit private Ghidra pseudocode or derived evidence to public GitHub.",
            "QEMU TCG alone is not a hardened sandbox for unknown vendor binaries.",
        ]
        (self.out / "session-summary.json").write_text(
            json.dumps(self.manifest, indent=2) + "\n")
        rows = []
        size = 0
        for p in sorted(self.out.rglob("*")):
            if p.is_symlink():
                raise ValueError("Refusing symlink in output tree: " + str(p))
            if p.is_dir():
                continue
            if not stat.S_ISREG(p.lstat().st_mode) or p.suffix.lower() not in ALLOWED:
                raise ValueError("Unexpected artifact: " + str(p))
            n = p.stat().st_size
            size += n
            if n > MAX_FILE or size > MAX_TOTAL:
                raise ValueError("Large output, review report folder: " + str(p))
            rows.append({
                "name": p.relative_to(self.out).as_posix(),
                "bytes": n,
                "sha256": sha256(p),
            })
        index = {
            "schema": "mpfree-bootchain-integrated-zip-index-v1",
            "private": True, "files": rows,
        }
        path = self.out.with_suffix(".zip")
        if path.exists():
            raise FileExistsError("Refusing to overwrite: " + str(path))
        with zipfile.ZipFile(path, "x", compression=zipfile.ZIP_DEFLATED,
                             compresslevel=6, allowZip64=False) as z:
            z.writestr("ZIP_INDEX.json", json.dumps(index, indent=2) + "\n")
            for row in rows:
                z.write(self.out / row["name"], arcname=row["name"])
        with zipfile.ZipFile(path) as z:
            if z.testzip():
                raise RuntimeError("Final ZIP CRC verification failed")
            if set(z.namelist()) != {r["name"] for r in rows} | {"ZIP_INDEX.json"}:
                raise RuntimeError("Unexpected ZIP members")
        (path.with_suffix(".zip.sha256")).write_text(
            sha256(path) + "  " + path.name + "\n")
        self.say("\n" + "=" * 75)
        self.say("ONE ZIP TO UPLOAD: " + str(path))
        self.say("SHA256: " + sha256(path))
        self.say("Reports/log files: " + str(len(rows)))
        self.say("Stages: " + json.dumps(self.stages, indent=2))
        self.say("All results remain PRIVATE. Do not push this ZIP to the repository.")
        self.say("=" * 75)
        return path


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--prior-v10", type=Path, help="Optional known V10 SHA-indexed private ZIP")
    p.add_argument("--qemu-boot-timeout", type=int, default=240)
    p.add_argument("--qemu-probe-timeout", type=int, default=90)
    args = p.parse_args()
    if args.qemu_boot_timeout < 30 or args.qemu_probe_timeout < 15:
        p.error("QEMU timeouts too small")
    os.umask(0o077)
    lab = Lab(args)
    try:
        lab.say("MPFree single-run research lab: offline Ghidra + ephemeral ARM QEMU")
        lab.say("Source tree: " + str(ROOT))
        lab.say("Output directory: " + str(lab.out))
        lab.say("No actual MPC device, USB interface or updater executable will be used.")
        try:
            lab.static()
        except Exception:
            lab.say("STATIC ERROR: " + traceback.format_exc())
            lab.notes.append("Static stage exception; subsequent QEMU and packaging continue.")
        try:
            lab.qemu()
        except Exception:
            lab.say("QEMU ERROR: " + traceback.format_exc())
            lab.notes.append("QEMU stage exception; package available Ghidra data.")
    finally:
        try:
            lab.full_file.flush()
            lab.full_file.close()
            lab.package()
        except Exception:
            print("PACKAGE ERROR: " + traceback.format_exc(), file=sys.stderr)
            print("Unpacked output remains at: " + str(lab.out), file=sys.stderr)
            raise


if __name__ == "__main__":
    main()
