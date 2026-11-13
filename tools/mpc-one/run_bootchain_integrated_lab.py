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
import tempfile
import time
import traceback
import zipfile

ROOT = Path(__file__).resolve().parents[2]
WORK = Path.home() / "mpfree-work"
ALLOWED = {".txt", ".json", ".log", ".md", ".csv", ".tsv"}
MAX_FILE = 96 * 1024 * 1024
MAX_TOTAL = 384 * 1024 * 1024
DEFAULT_V10 = WORK / "bootchain-wide-20261112T014736Z-v10-3708829.zip"


def guest_marker_seen(transcript, marker):
    """Match a completed guest command, ignoring PTY CRCRLF normalization."""
    normalized = transcript.replace("\r", "")
    return re.search(r"(?m)^" + re.escape(marker) + r"[ \t]*(?:\n|$)", normalized) is not None


def qemu_storage_for_synthetic_model(command, mock_disk):
    """Wire two ARM virtio-MMIO disks using observed QEMU device enumeration.

    With the existing root-drive device followed by the fixture device in
    -device arguments, this QEMU ARM virt kernel enumerates the final virtio
    disk as /dev/vda (synthetic) and the first one as /dev/vdb (rootfs).
    Verified from the owner's 2026-11-12 serial log. Do NOT silently assume
    the original root=/dev/vda is still correct when adding the second disk.
    """
    argv = list(command)
    if argv.count("-append") != 1:
        raise ValueError("Unexpected QEMU kernel argument layout")
    idx = argv.index("-append") + 1
    if argv[idx].count("root=/dev/vda") != 1 or "root=/dev/vdb" in argv[idx]:
        raise ValueError("Unexpected QEMU root target; refusing unverified device mapping")
    if any("mpfreemock" in arg for arg in argv):
        raise ValueError("Synthetic disk already attached")
    argv[idx] = argv[idx].replace("root=/dev/vda", "root=/dev/vdb")
    argv.extend([
        "-drive", "if=none,id=mpfreemock,format=raw,readonly=on,file=" + str(mock_disk),
        "-device", "virtio-blk-device,drive=mpfreemock",
    ])
    return argv


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
        self.mock_tempdir = None
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
        if not self.full_file.closed:
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

    def prepare_synthetic_usb_model(self):
        """Build an ARM executable and a private *regular-file* read-only ext4 fixture.

        This is an independently written model of USB descriptor selection.
        It is NOT AKAI's updater and never opens USB/eMMC or host devices.
        """
        source = regular(ROOT / "tools/mpc-one/usb_descriptor_mock_model.c")
        endpoint_source = regular(ROOT / "tools/mpc-one/usb_endpoint_mock_model.c")
        cc = shutil.which("arm-linux-gnueabihf-gcc")
        mke2fs = shutil.which("mkfs.ext4") or shutil.which("mke2fs")
        if not cc or not mke2fs:
            raise RuntimeError("ARM cross GCC and e2fsprogs required for optional synthetic model")
        self.mock_tempdir = tempfile.TemporaryDirectory(
            prefix="mpfree-qemu-mock-", dir=str(WORK))
        folder = Path(self.mock_tempdir.name)
        staging = folder / "staging"
        staging.mkdir(mode=0o700)
        binary = staging / "usb_descriptor_model"
        endpoint_binary = staging / "usb_endpoint_model"
        disk = folder / "usb-model.ext4"
        build_log = self.out / "qemu-synthetic-model-build.log"
        commands = [
            [cc, "-std=c11", "-O2", "-static", "-s",
             "-Wall", "-Wextra", "-Werror",
             "-o", str(binary), str(source)],
            [cc, "-std=c11", "-O2", "-static", "-s",
             "-Wall", "-Wextra", "-Werror",
             "-o", str(endpoint_binary), str(endpoint_source)],
        ]
        with build_log.open("w") as log:
            for cmd in commands:
                log.write("$ " + " ".join(cmd) + "\n")
                p = subprocess.run(cmd, capture_output=True, text=True, timeout=90)
                log.write(p.stdout + p.stderr)
                if p.returncode:
                    raise RuntimeError("Synthetic ARM test model compile failed; check " + build_log.name)
            binary.chmod(0o555)
            endpoint_binary.chmod(0o555)
            with disk.open("wb") as fh:
                fh.truncate(24 * 1024 * 1024)
            cmd = [mke2fs, "-F", "-q", "-t", "ext4", "-d", str(staging), str(disk)]
            log.write("$ " + " ".join(cmd) + "\n")
            p = subprocess.run(cmd, capture_output=True, text=True, timeout=90)
            log.write(p.stdout + p.stderr)
            if p.returncode:
                raise RuntimeError("Synthetic ARM model ext4 construction failed; check " + build_log.name)
        with disk.open("rb") as handle:
            handle.seek(1080)
            if handle.read(2) != b"\x53\xef":
                raise RuntimeError("Synthetic ext4 magic not recognized")
        self.stages["synthetic-usb-model-build"] = {
            "success": True, "binary_sha256": sha256(binary),
            "model_source_sha256": sha256(source),
            "endpoint_binary_sha256": sha256(endpoint_binary),
            "endpoint_source_sha256": sha256(endpoint_source),
            "guest_disk_regular_file": True,
            "host_build_log": build_log.name,
        }
        self.say("Built independent ARM USB-descriptor model for guest execution.")
        return disk

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
            synthetic_mock_enabled = False
            try:
                mock_disk = self.prepare_synthetic_usb_model()
                # QEMU ARM virt enumerated the newly attached fixture as
                # vda and the original cloned rootfs as vdb in the owner's
                # latest serial log. Configure the root argument accordingly.
                command = qemu_storage_for_synthetic_model(command, mock_disk)
                synthetic_mock_enabled = True
            except Exception as exc:
                self.notes.append("Synthetic USB model not prepared: " + str(exc))
                self.stages["synthetic-usb-model-build"] = {
                    "success": False, "reason": str(exc),
                }
                self.say("Synthetic USB model unavailable: " + str(exc))
            result["synthetic_usb_model_enabled"] = synthetic_mock_enabled
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
                "mount -t proc proc /proc 2>&1 || true",
                "mount -t sysfs sysfs /sys 2>&1 || true",
                "cat /proc/version",
                "cat /proc/cmdline",
                "echo MPFREE_VM_SECTION_mounts",
                "cat /proc/mounts",
                "echo MPFREE_VM_SECTION_cpu_memory",
                "cat /proc/cpuinfo",
                "cat /proc/meminfo",
                "echo MPFREE_VM_SECTION_vendor_paths",
                "for p in /usr/sbin/az01-update /usr/lib/libaz01-prog.so.1.3 /usr/sbin/init /opt/ramfs-update.squashfs /usr/bin/az01-bootloader-sig /usr/sbin/az01config; do ls -ld \"$p\" 2>&1; done",
                "echo MPFREE_VM_SECTION_vendor_hashes",
                "for p in /usr/sbin/az01-update /usr/lib/libaz01-prog.so.1.3 /opt/ramfs-update.squashfs; do if test -f \"$p\"; then sha256sum \"$p\"; fi; done",
                "echo MPFREE_VM_SECTION_init_and_libraries",
                "cat /etc/os-release 2>&1 || true",
                "ls -l /lib/ld-linux-armhf.so.3 /lib/libc.so.6 /usr/lib/libaz01-prog.so.1.3 2>&1",
                "echo MPFREE_VM_SECTION_elf_linker_dependencies",
                "if test -x /lib/ld-linux-armhf.so.3; then /lib/ld-linux-armhf.so.3 --list /usr/sbin/az01-update 2>&1; fi",
                "if test -x /lib/ld-linux-armhf.so.3; then /lib/ld-linux-armhf.so.3 --list /usr/sbin/init 2>&1; fi",
                "echo MPFREE_VM_SECTION_boot_and_services",
                "cat /etc/fstab 2>&1 || true",
                "ls -l /boot /etc/systemd/system/multi-user.target.wants 2>&1",
                "echo MPFREE_VM_SECTION_direct_boot_rootfs",
                "ls -la /boot 2>&1",
                "for p in /boot/boot.scr.uimg /boot/zImage /boot/rk3288-az01-acva.dtb /boot/rk3288-az01-acva-c.dtb; do if test -f \"$p\"; then sha256sum \"$p\" 2>&1; else echo MISSING_BOOT_ASSET:\"$p\"; fi; done",
                "echo MPFREE_VM_SECTION_boot_script_hex",
                "od -v -b /boot/boot.scr.uimg 2>&1",
                "echo MPFREE_VM_SECTION_kernel_modules",
                "ls -la /lib/modules /lib/firmware 2>&1",
                "echo MPFREE_VM_SECTION_310_trustpoint_unit",
                "cat /usr/lib/systemd/system/az0x-loadpin-trustpoints.service 2>&1 || true",
                "echo MPFREE_VM_SECTION_310_policy_assets",
                "for p in /usr/bin/az01-bootloader-sig /usr/sbin/az0x-loadpin-trustpoints /opt/ramfs-update.squashfs; do if test -f \"$p\"; then ls -l \"$p\" 2>&1; fi; done",
                "echo MPFREE_VM_SECTION_security_state",
                "cat /proc/sys/kernel/kexec_load_disabled 2>&1 || true",
                "cat /sys/kernel/security/lsm 2>&1 || true",
                "echo MPFREE_VM_SECTION_virtual_devices",
                "ls -l /dev/vda /dev/mmcblk0 /dev/bus/usb /sys/bus/usb/devices 2>&1",
                "echo MPFREE_VM_SECTION_available_tools",
                "for p in sha256sum readelf strace base64 awk sed grep; do command -v \"$p\" || true; done",
                "echo MPFREE_VM_SECTION_synthetic_usb_model",
                "echo MPFREE_MODEL_BEGIN",
            ]
            if synthetic_mock_enabled:
                lines.extend([
                    "mkdir -p /mnt/mpfree-usb-model",
                    "mount -t ext4 -o ro /dev/vda /mnt/mpfree-usb-model 2>&1",
                    "if /mnt/mpfree-usb-model/usb_descriptor_model 2>&1; then echo MPFREE_MODEL_EXEC_OK; else echo MPFREE_MODEL_EXEC_FAIL; fi",
                    "echo MPFREE_VM_SECTION_synthetic_usb_endpoint",
                    "if /mnt/mpfree-usb-model/usb_endpoint_model 2>&1; then echo MPFREE_ENDPOINT_EXEC_OK; else echo MPFREE_ENDPOINT_EXEC_FAIL; fi",
                ])
            else:
                lines.append("echo MPFREE_MODEL_SKIPPED")
            lines.extend([
                "echo MPFREE_VM_SECTION_completed",
                "echo " + end,
            ])
            # Only shell builtins and passive inspection tools. Do not run
            # az01-update, az01config, program-fuses, or AKAI init/services.
            # The guest's rootfs writes (such as mounts) are ephemeral.
            boot_deadline = time.monotonic() + self.args.qemu_boot_timeout
            idle_deadline = None
            sent = False
            probe_cursor = 0
            awaiting_marker = None
            data = ""

            def send_guest_probe(index):
                # Two guest shell commands: requested inspection, then a
                # unique completion ACK. Only send the next inspection
                # after the preceding ACK was actually printed.
                marker = "MPFREE_VM_ACK_" + token + "_" + str(index)
                os.write(master, (lines[index] + "\n" + "echo " + marker + "\n").encode("utf-8"))
                return marker
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
                        if re.search(r"Kernel panic - not syncing|Requested init /bin/sh failed", data[-12000:]):
                            result["error"] = (
                                "QEMU guest kernel panic before probe; inspect serial log. "
                                "Likely wrong virtual rootfs selection or missing init."
                            )
                            break
                        if len(data) > 8 * 1024 * 1024:
                            data = data[-8 * 1024 * 1024:]
                        if not sent and re.search(r"(?:^|[\r\n])(?:/ # |# |~ # )", data[-4000:]):
                            result["booted"] = True
                            sent = True
                            idle_deadline = time.monotonic() + self.args.qemu_probe_timeout
                            awaiting_marker = send_guest_probe(probe_cursor)
                            self.say("QEMU guest shell detected. Running passive probes sequentially.")
                        elif sent and awaiting_marker and guest_marker_seen(data, awaiting_marker):
                            probe_cursor += 1
                            if probe_cursor == len(lines):
                                # The last command emits an independent DONE
                                # marker, followed by its own ACK marker.
                                result["probe_complete"] = guest_marker_seen(data, end)
                                result["success"] = result["probe_complete"]
                                if not result["success"]:
                                    result["error"] = "Final ACK reached without guest DONE marker"
                                break
                            awaiting_marker = send_guest_probe(probe_cursor)
                            idle_deadline = time.monotonic() + self.args.qemu_probe_timeout
                    if not sent and now + 5 > boot_deadline:
                        # Final harmless shell-input nudge for nonstandard prompt.
                        os.write(master, b"\n")
                transcript = data.splitlines()
            result["serial_log"] = guest.name
            normalized = data.replace("\r", "")
            split_sections = re.split(r"(?m)^MPFREE_VM_SECTION_([a-z0-9_]+)[ ]*$", normalized)
            sections = {}
            for k in range(1, len(split_sections) - 1, 2):
                section = split_sections[k]
                body = split_sections[k + 1]
                section_file = self.out / ("qemu-guest-" + section + ".txt")
                section_file.write_text(body)
                sections[section] = section_file.name
            result["observed_sections"] = sorted(sections)
            result["section_files"] = sections
            result["guest_probe_commands"] = lines
            result["synthetic_usb_model_test_results"] = [
                json.loads(match) for match in re.findall(
                    r'(?m)^(\{"test":.*\})[ \t]*$', normalized
                )
            ]
            descriptor_results = [
                row for row in result["synthetic_usb_model_test_results"]
                if not row["test"].startswith("endpoint_")
            ]
            endpoint_results = [
                row for row in result["synthetic_usb_model_test_results"]
                if row["test"].startswith("endpoint_")
            ]
            result["synthetic_usb_descriptor_test_results"] = descriptor_results
            result["synthetic_usb_endpoint_test_results"] = endpoint_results
            result["synthetic_usb_model_guest_pass"] = (
                "MPFREE_USB_DESCRIPTOR_MODEL_PASS" in normalized
                and "MPFREE_MODEL_EXEC_OK" in normalized
                and len(descriptor_results) == 13
                and all(row.get("pass") is True for row in descriptor_results)
            ) if synthetic_mock_enabled else None
            result["synthetic_usb_endpoint_guest_pass"] = (
                "MPFREE_USB_ENDPOINT_MODEL_PASS" in normalized
                and "MPFREE_ENDPOINT_EXEC_OK" in normalized
                and len(endpoint_results) == 21
                and all(row.get("pass") is True for row in endpoint_results)
            ) if synthetic_mock_enabled else None
            if synthetic_mock_enabled:
                self.stages["synthetic-usb-model-guest"] = {
                    "success": result["synthetic_usb_model_guest_pass"],
                    "tests_observed": len(descriptor_results),
                    "console": sections.get("synthetic_usb_model"),
                }
                self.stages["synthetic-usb-endpoint-guest"] = {
                    "success": result["synthetic_usb_endpoint_guest_pass"],
                    "tests_observed": len(endpoint_results),
                    "console": sections.get("synthetic_usb_endpoint"),
                }
            result["guest_probes_acknowledged"] = probe_cursor
            result["guest_probes_expected"] = len(lines)
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
            if self.mock_tempdir is not None:
                self.mock_tempdir.cleanup()
                self.mock_tempdir = None
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

    def synthesize(self):
        """Correlate guest executable hashes with the verified static ELF inputs."""
        static_path = self.out / "static" / "v11" / "v11-stage-manifest.json"
        wide_path = self.out / "static" / "wide" / "manifest.json"
        guest_path = self.out / "qemu-probe-results.json"
        hashes_path = self.out / "qemu-guest-vendor_hashes.txt"
        record = {
            "schema": "mpfree-integrated-cross-evidence-v1",
            "static_v11_manifest": static_path.is_file(),
            "broad_static_manifest": wide_path.is_file(),
            "qemu_guest_probe": guest_path.is_file(),
            "match_tests": [],
            "confirmed_boundaries": [
                "Booting the QEMU guest does NOT execute MPC SPL, U-Boot, or MaskROM.",
                "Guest loader --list does not execute the updater application entrypoint.",
                "Any live physical recovery, ROM USB acceptance, fuse policy or DDR inference requires separate evidence.",
            ],
        }
        if wide_path.is_file():
            try:
                src = json.loads(wide_path.read_text())
                record["spl_ghidra_recognized_functions"] = src.get("p1_functions")
                record["uboot_ghidra_recognized_functions"] = src.get("p3_functions")
                record["wide_static_source_mode"] = src.get("source_mode")
            except Exception as exc:
                record["wide_parse_error"] = str(exc)
        guest_hashes = {}
        if hashes_path.is_file():
            for line in hashes_path.read_text(errors="replace").splitlines():
                found = re.match(r"^([a-f0-9]{64})[ \t]+(/\S+)", line.strip())
                if found:
                    guest_hashes[found.group(2)] = found.group(1)
        if static_path.is_file():
            try:
                src = json.loads(static_path.read_text())
                record["v11_static_stage_success"] = {
                    k: v.get("success") for k, v in src.get("stages", {}).items()
                }
                expected = src.get("source_sha256", {})
                for role, guest_name in (
                    ("az01-update", "/usr/sbin/az01-update"),
                    ("libaz01-prog", "/usr/lib/libaz01-prog.so.1.3"),
                ):
                    actual = guest_hashes.get(guest_name)
                    want = expected.get(role)
                    record["match_tests"].append({
                        "role": role, "guest_path": guest_name,
                        "guest_sha256": actual, "static_sha256": want,
                        "match": actual == want if actual and want else None,
                        "interpretation": (
                            "Guest and static executable input match byte-for-byte"
                            if actual and want and actual == want else
                            "Cannot prove guest/static source identity; investigate missing or differing hashes"
                        ),
                    })
            except Exception as exc:
                record["v11_parse_error"] = str(exc)
        else:
            record["match_tests_note"] = "V11 report missing; no comparison asserted"
        # Derive a protocol-specific evidence map from verified V11 Ghidra
        # output without copying any decompiled proprietary source into Git.
        library_report = self.out / "static" / "v11" / "v11-ghidra-libaz01-prog.json"
        if library_report.is_file():
            try:
                module = load_module(
                    "tools/mpc-one/synthesize_usb_backend_protocols.py",
                    "mpfree_private_usb_evidence",
                )
                blob = library_report.read_bytes()
                protocol = module.summarize(json.loads(blob), hashlib.sha256(blob).hexdigest())
                (self.out / "usb-protocol-evidence.json").write_text(
                    json.dumps(protocol, indent=2) + "\n")
                (self.out / "usb-protocol-evidence.md").write_text(
                    module.render_markdown(protocol))
                record["usb_protocol_evidence"] = {
                    "report": "usb-protocol-evidence.json",
                    "interfaces": len(protocol["verified_backend_interfaces"]),
                    "identifications": [
                        {"descriptor": e["usb_descriptor"],
                         "family": e["upstream_protocol_family"],
                         "constructor_confirmed": e["initializer_references_table"]}
                        for e in protocol["verified_backend_interfaces"]
                    ],
                    "limits": "Static and upstream reference only; no original vendor code executed.",
                }
                self.stages["usb-protocol-evidence"] = {
                    "success": True, "interfaces": len(protocol["verified_backend_interfaces"]),
                }
            except Exception as exc:
                record["usb_protocol_evidence_error"] = str(exc)
                self.stages["usb-protocol-evidence"] = {
                    "success": False, "error": str(exc),
                }
                self.notes.append("USB protocol evidence synthesis: " + str(exc))
        else:
            record["usb_protocol_evidence_error"] = "V11 library report unavailable"
            self.stages["usb-protocol-evidence"] = {
                "success": False, "error": "V11 library report unavailable",
            }
        # FF/61/7A custom framing evidence is derived from Ghidra only.
        # Never run the original updater on the host or guest.
        if library_report.is_file():
            try:
                ff_module = load_module(
                    "tools/mpc-one/synthesize_custom_usb_framing.py",
                    "mpfree_private_ff617a_framing",
                )
                blob = library_report.read_bytes()
                frame_report = ff_module.validated_report(
                    json.loads(blob), hashlib.sha256(blob).hexdigest())
                (self.out / "usb-ff617a-framing.json").write_text(
                    json.dumps(frame_report, indent=2) + "\n")
                (self.out / "usb-ff617a-framing.md").write_text(
                    ff_module.render_markdown(frame_report) + "\n")
                observed = frame_report["response_validation_checks"]
                instructions = sum(
                    item["instruction_evidence"]["captured"]
                    for item in frame_report["focused_function_evidence"])
                identified_ids = frame_report["magic_evidence"]
                record["ff617a_framing_evidence"] = {
                    "report": "usb-ff617a-framing.json",
                    "compared_message_identifiers": len(
                        identified_ids["equality_matched_message_identifiers"]),
                    "constructed_outgoing_values": len(
                        identified_ids["constructed_outgoing_control_values"]),
                    "range_bound_values": len(
                        identified_ids["comparison_only_range_bounds"]),
                    "all_response_checks_observed": all(observed.values()),
                    "response_checks": observed,
                    "machine_instructions_captured": instructions,
                    "decompiler_signature_warnings": len(
                        frame_report["decompiler_return_type_warnings"]),
                    "ioctls_verified": all(
                        item["seen_in_verified_pseudocode"]
                        for item in frame_report["linux_usbdevfs_ioctls"]),
                    "protocol_identity": "UNKNOWN",
                    "no_original_vendor_execution": True,
                }
                evidence = record["ff617a_framing_evidence"]
                self.stages["ff617a-framing-evidence"] = {
                    "success": (
                        evidence["all_response_checks_observed"] and
                        evidence["machine_instructions_captured"] > 0 and
                        evidence["ioctls_verified"] and
                        evidence["compared_message_identifiers"] == 6 and
                        evidence["constructed_outgoing_values"] == 2
                    ),
                    "compared_ids": evidence["compared_message_identifiers"],
                    "constructed_values": evidence["constructed_outgoing_values"],
                    "arm_instructions": instructions,
                }
            except Exception as exc:
                record["ff617a_framing_error"] = str(exc)
                self.stages["ff617a-framing-evidence"] = {
                    "success": False, "error": str(exc),
                }
                self.notes.append("FF/61/7A framing synthesis: " + str(exc))
        else:
            record["ff617a_framing_error"] = "V11 library report unavailable"
            self.stages["ff617a-framing-evidence"] = {
                "success": False, "error": "V11 library report unavailable",
            }
        # Analyze captured real ARM instruction evidence for callsite/return
        # contracts. The source Ghidra JSON stays owner-private in this ZIP.
        if library_report.is_file():
            try:
                arm_module = load_module(
                    "tools/mpc-one/analyze_arm_return_contracts.py",
                    "mpfree_private_arm_contracts",
                )
                arm_report = arm_module.summarize(
                    json.loads(library_report.read_text()),
                    {"indexed_members_verified": None,
                     "crc_passed": None,
                     "note": "Directly verified current private Ghidra JSON; ZIP hash index is built at packaging"},
                    frame_report if "frame_report" in locals() else None,
                )
                (self.out / "arm-return-contracts.json").write_text(
                    json.dumps(arm_report, indent=2) + "\n")
                (self.out / "arm-return-contracts.md").write_text(
                    arm_module.to_markdown(arm_report))
                arm_sites = [
                    call for fn in arm_report["functions"]
                    for call in fn["observed_callsites"]
                ]
                r0_checked = sum(bool(call["immediate_r0_test"]) for call in arm_sites)
                r0_copied = sum(bool(call["r0_copies"]) for call in arm_sites)
                r4_propagated = sum(
                    bool(saved.get("propagates_to_normal_return"))
                    for call in arm_sites for saved in call["r0_copies"]
                )
                r0_returned = sum(
                    bool(call["r0_preserved_to_normal_return"]) for call in arm_sites
                )
                cmp_only = sum(bool(call["immediate_r0_compare"]) for call in arm_sites)
                self.stages["arm-return-contract-evidence"] = {
                    "success": (r0_checked >= 6 and r0_copied >= 2
                                and r4_propagated >= 2 and r0_returned >= 3),
                    "captured_status_test_callsites": r0_checked,
                    "explicit_cmp_callsites": cmp_only,
                    "subs_zero_test_callsites": r0_checked - cmp_only,
                    "r0_captured_for_later_use": r0_copied,
                    "r4_copy_status_return_passthroughs": r4_propagated,
                    "normal_fallthrough_return_passthroughs": r0_returned,
                    "functions": len(arm_report["functions"]),
                }
                record["arm_return_contract_evidence"] = {
                    "report": "arm-return-contracts.json",
                    "status_test_callsites": r0_checked,
                    "explicit_cmp_callsites": cmp_only,
                    "subs_zero_test_callsites": r0_checked - cmp_only,
                    "r0_copy_callsites": r0_copied,
                    "r4_copy_status_return_passthroughs": r4_propagated,
                    "normal_fallthrough_r0_passthroughs": r0_returned,
                    "safety": "Read-only static-derived facts, no vendor execution",
                }
            except Exception as exc:
                self.stages["arm-return-contract-evidence"] = {
                    "success": False, "error": str(exc)}
                record["arm_return_contract_evidence_error"] = str(exc)
                self.notes.append("ARM return contracts: " + str(exc))
        # Seven-track evidence matrix, not just USB. Derive from source
        # Ghidra + boot + update reports; no original vendor execution.
        try:
            matrix_module = load_module(
                "tools/mpc-one/synthesize_wide_research_matrix.py",
                "mpfree_multitrack_questions",
            )
            matrix = matrix_module.build(
                lambda rel: (self.out / rel).read_bytes()
            )
            (self.out / "wide-research-matrix.json").write_text(
                json.dumps(matrix, indent=2) + "\n")
            (self.out / "wide-research-matrix.md").write_text(
                matrix_module.markdown(matrix))
            matrix_tracks = len(set(
                q["track"] for q in matrix["questions"]))
            matrix_count = len(matrix["questions"])
            record["wide_research_matrix"] = {
                "report": "wide-research-matrix.json",
                "questions": matrix_count,
                "tracks": matrix_tracks,
                "counts": matrix["counts"],
                "hardware_gated": matrix["counts"]["hardware-gated"],
                "no_vendor_execution": True,
            }
            self.stages["wide-multitrack-question-evidence"] = {
                "success": matrix_count >= 67 and matrix_tracks == 7,
                "questions": matrix_count, "tracks": matrix_tracks,
            }
        except Exception as exc:
            record["wide_research_matrix_error"] = str(exc)
            self.stages["wide-multitrack-question-evidence"] = {
                "success": False, "error": str(exc),
            }
            self.notes.append("Wide multi-track question synthesis: " + str(exc))
        # Direct-boot focus: tie the community Fastboot/OpenAZ0x leads to
        # this exact owner SHA-verified P3 U-Boot binary. Do not send commands.
        try:
            direct_module = load_module(
                "tools/mpc-one/synthesize_direct_boot_focus.py",
                "mpfree_directboot_evidence",
            )
            direct = direct_module.build(
                lambda path: (self.out / path).read_bytes()
            )
            (self.out / "direct-boot-focus.json").write_text(
                json.dumps(direct, indent=2) + "\n")
            (self.out / "direct-boot-focus.md").write_text(
                direct_module.render_md(direct))
            topic_count = len(set(q["topic"] for q in direct["questions"]))
            required = (
                "token_prefix_comparison_37_bytes",
                "oem_grants_permission_pointer",
                "global_0x14d690_written_at_0x12289e",
                "global_0x14d690_read_at_0x122838",
                "download_gate_present",
                "flash_gate_present",
                "erase_gate_present",
                "boot_completion_calls_bootm",
                "normal_script_bootz",
            )
            passed = all(direct["checks"].get(key) for key in required)
            exact_checks = (
                "literal_pool_both_pointers_equal",
                "literal_pool_both_point_to_global",
                "published_oem_token_binary_sha256_matches",
            )
            literal_proof = all(direct["checks"].get(key) is True
                                for key in exact_checks)
            dispatch_checks = (
                "fastboot_dispatch_table_eight_entries",
                "fastboot_boot_command_callback_resolved",
                "fastboot_download_command_callback_resolved",
                "all_eight_fastboot_prefixes_exact_and_unique",
                "getvar_has_no_local_permission_call",
                "continue_has_no_local_permission_call",
                "reboot_has_no_local_permission_call",
                "reboot_mode_clear_and_one_nine_branch",
                "separate_boot_threshold_below_100_observed",
                "all_three_host_usb_selector_families_verified",
                "fastboot_host_backend_has_okay_fail_literals",
                "ff617a_is_not_proven_rockusb",
            )
            dispatch_proof = all(direct["checks"].get(key) is True
                                 for key in dispatch_checks)
            storage_checks = (
                "flash_partition_capacity_guard",
                "flash_sparse_four_chunk_kinds",
                "sparse_raw_fill_partition_bound",
                "erase_partition_block_range_guard",
            )
            storage_guard_proof = all(direct["checks"].get(key) is True
                                      for key in storage_checks)
            record["direct_boot_focus"] = {
                "report": "direct-boot-focus.json",
                "questions":len(direct["questions"]),
                "topics":topic_count,
                "required_owner_evidence":passed,
                "literal_pointer_and_token_proven":literal_proof,
                "eight_entry_fastboot_dispatch_proven":dispatch_proof,
                "partition_guard_and_sparse_path_proven":storage_guard_proof,
                "permission_pointer_probe":direct.get("probe",{}),
            }
            self.stages["direct-boot-focus-evidence"] = {
                "success":(passed and literal_proof and dispatch_proof
                           and storage_guard_proof
                           and len(direct["questions"])>=42 and topic_count==10),
                "questions":len(direct["questions"]),"topics":topic_count,
                "required_static_checks_passed":passed,
                "new_literal_pointer_and_token_proof":literal_proof,
                "new_boot_and_download_dispatch_proof":dispatch_proof,
                "raw_sparse_and_erase_guard_proof":storage_guard_proof,
            }
        except Exception as exc:
            record["direct_boot_focus_error"]=str(exc)
            self.stages["direct-boot-focus-evidence"] = {
                "success":False,"error":str(exc)
            }
            self.notes.append("Direct-boot focus synthesis: " + str(exc))
        # SHA-backed P6 boot-layout observations from guest file metadata.
        # Generic ARM virt does NOT validate the real MPC One board.
        layout_path = self.out / "qemu-guest-direct_boot_rootfs.txt"
        modules_path = self.out / "qemu-guest-kernel_modules.txt"
        trust_path = self.out / "qemu-guest-310_trustpoint_unit.txt"
        if layout_path.is_file() and modules_path.is_file():
            try:
                compared = json.loads((
                    self.out / "static" / "wide" /
                    "firmware-3.10-bootchain-v4-compare.json"
                ).read_text())
                layout = layout_path.read_text(errors="replace")
                module_text = modules_path.read_text(errors="replace")
                checksums = {}
                for line in layout.splitlines():
                    parts = line.strip().split()
                    if (len(parts) >= 2 and len(parts[0]) == 64 and
                            all(c in "0123456789abcdefABCDEF" for c in parts[0]) and
                            parts[1].startswith("/boot/")):
                        checksums[parts[1]] = parts[0].lower()
                required = (
                    "/boot/boot.scr.uimg", "/boot/zImage",
                    "/boot/rk3288-az01-acva.dtb",
                    "/boot/rk3288-az01-acva-c.dtb",
                )
                source_script_sha = compared["boot_script"]["firmware310"]["sha256"]
                actual = checksums.get("/boot/boot.scr.uimg")
                info = {
                    "schema": "mpfree-qemu-directboot-rootfs-compat-v1",
                    "guest_rootfs": "cloned 3.10 P6 under generic ARM virt",
                    "not_actual_mpc_hardware": True,
                    "not_independent_mpfree_rootfs": True,
                    "no_vendor_code_executed": True,
                    "boot_asset_sha256": checksums,
                    "required_boot_assets": {
                        path: path in checksums for path in required},
                    "boot_script_matches_offline_3_10": None,
                    "source_310_raw_script_sha256": source_script_sha,
                    "official_full_image_sha256_not_provided": True,
                    "observed_modules_kernel_6_18_44": "6.18.44" in module_text,
                    "trustpoint_unit_captured": trust_path.is_file(),
                    "limitations": [
                        "QEMU virt boot does not prove MPC hardware compatibility.",
                        "An independent MPFree rootfs must be separately tested.",
                    ],
                }
                hex_path = self.out / "qemu-guest-boot_script_hex.txt"
                if hex_path.is_file():
                    try:
                        wrapper = load_module(
                            "tools/mpc-one/verify_uboot_script_wrapper.py",
                            "mpfree_uboot_script_wrapper",
                        )
                        script_report = wrapper.compare_guest_to_update(
                            hex_path.read_text(errors="replace"),
                            compared["boot_script"]["firmware310"],
                        )
                        if (script_report["full_image_sha256"] != actual):
                            raise ValueError(
                                "QEMU od bytes disagree with guest sha256sum")
                        info["verified_uboot_script"] = script_report
                        info["boot_script_commands_match_official"] = (
                            script_report["script_commands_match_official"])
                        info["boot_script_header_and_raw_script_match_official"] = (
                            script_report["canonical_header_and_script_bytes_match_official"])
                        info["boot_script_matches_offline_3_10"] = (
                            script_report["full_image_matches_official"])
                    except Exception as exc:
                        info["boot_script_decode_error"] = str(exc)
                else:
                    info["boot_script_decode_error"] = (
                        "QEMU guest boot-script hexadecimal section missing")
                (self.out / "qemu-directboot-rootfs-compat.json").write_text(
                    json.dumps(info, indent=2) + "\n")
                record["qemu_directboot_rootfs_compat"] = info
                self.stages["qemu-directboot-rootfs-compat"] = {
                    "success": (
                        info.get("boot_script_header_and_raw_script_match_official") is True
                        and all(info["required_boot_assets"].values())),
                    "assets_found": sum(info["required_boot_assets"].values()),
                    "assets_expected": len(required),
                    "boot_script_full_sha_match": info["boot_script_matches_offline_3_10"],
                    "header_and_raw_script_sha_match": info.get(
                        "boot_script_header_and_raw_script_match_official"),
                    "boot_script_commands_match": info.get(
                        "boot_script_commands_match_official"),
                }
            except Exception as exc:
                self.stages["qemu-directboot-rootfs-compat"] = {
                    "success": False, "error": str(exc)}
                self.notes.append("QEMU boot asset audit: " + str(exc))
        else:
            self.stages["qemu-directboot-rootfs-compat"] = {
                "success": False,
                "error": "Guest boot layout files unavailable",
            }
        # Independent 3.10 userspace security utility static pass.
        # These are small vendor binaries, so analyze their pre-existing
        # SHA-verified ARM disassembly only; NEVER run their entrypoints.
        try:
            security_module = load_module(
                "tools/mpc-one/synthesize_310_security_utilities.py",
                "mpfree_310_security_static",
            )
            source_compare = json.loads((
                self.out / "static" / "wide" /
                "firmware-3.10-bootchain-v4-compare.json"
            ).read_text())
            security_report = security_module.inspect_310(source_compare)
            (self.out / "security-310-utilities.json").write_text(
                json.dumps(security_report, indent=2) + "\n")
            (self.out / "security-310-utilities.md").write_text(
                security_module.markdown(security_report))
            signature = security_report["bootloader_sig"]
            loadpin = security_report["loadpin_trustpoints"]
            code_evidence = (
                signature["rc4_observed_length_512"]
                and signature["memcmp_observed_length_32"]
                and bool(signature["sha256_function_calls"]["SHA256_Update@plt"])
                and loadpin["ioctl_request"] == "0x40044c00"
            )
            record["security_310_utilities"] = {
                "report": "security-310-utilities.json",
                "code_evidence_verified": bool(code_evidence),
                "actual_kernel_enforcement_proven": False,
            }
            self.stages["security-310-utilities-static"] = {
                "success": bool(code_evidence),
                "rc4_512_sha256_digest_comparison": (
                    signature["rc4_observed_length_512"]
                    and signature["memcmp_observed_length_32"]),
                "loadpin_verity_ioctl": loadpin["ioctl_request"],
            }
        except Exception as exc:
            record["security_310_utilities_error"] = str(exc)
            self.stages["security-310-utilities-static"] = {
                "success":False,"error":str(exc)}
            self.notes.append("Security 3.10 utility static audit: "+str(exc))
        # Cross-stage stock recovery: original ARM ADC <=99, three
        # diagnostic strings, board_setup flag and saved Fastboot env.
        # No real button polling, device access or bootloader execution.
        try:
            adc_module = load_module(
                "tools/mpc-one/synthesize_recovery_adc.py",
                "mpfree_stock_recovery_adc",
            )
            adc_report = adc_module.build(
                lambda path: (self.out / path).read_bytes()
            )
            (self.out / "recovery-adc-evidence.json").write_text(
                json.dumps(adc_report, indent=2) + "\n")
            (self.out / "recovery-adc-evidence.md").write_text(
                adc_module.markdown(adc_report))
            record["recovery_adc_evidence"] = {
                "report": "recovery-adc-evidence.json",
                "threshold_unsigned_max": adc_report[
                    "adc_recovery_threshold_unsigned_max_inclusive"],
                "arm_and_string_evidence": adc_report[
                    "all_existing_cross_checks_passed"],
                "same_flag_exported_to_dorecovery": adc_report[
                    "all_pointer_checks_passed"],
                "actual_button_tested": False,
                "maskrom_restoration_proven": False,
            }
            self.stages["recovery-adc-cross-stage-evidence"] = {
                "success": bool(
                    adc_report["all_existing_cross_checks_passed"] and
                    adc_report["all_pointer_checks_passed"]),
                "adc_threshold_inclusive": adc_report[
                    "adc_recovery_threshold_unsigned_max_inclusive"],
                "same_global_flag_pointer": adc_report[
                    "all_pointer_checks_passed"],
            }
        except Exception as exc:
            record["recovery_adc_error"] = str(exc)
            self.stages["recovery-adc-cross-stage-evidence"] = {
                "success": False, "error": str(exc)}
            self.notes.append("Recovery ADC static proof: " + str(exc))
        # Separate broad evidence for SRAM SPL branch pointers and
        # updater GOT/PLT trampolines. A mapped code pointer is NOT
        # proof of ROM acceptance, runtime binding or safe updates.
        try:
            pointers = {}
            for role in ("spl", "az01-update"):
                source = json.loads((
                    self.out / "static" / "v11" /
                    ("v11-ghidra-" + role + ".json")).read_text())
                data = source.get("direct_boot_probe", {})
                if data.get("role") != role:
                    raise ValueError("Missing verified pointer probe: " + role)
                words = data.get("original_image_pointers", {})
                if not words:
                    raise ValueError("Missing original stage pointer words")
                pointers[role] = {
                    "original_image_pointers": words,
                    "pointers_resolved_from_image": sum(
                        bool(x.get("original_word")) for x in words.values()),
                    "runtime_target_validation": False,
                    "physical_stage_executed": False,
                }
            all_pointers = {
                "schema":"mpfree-spl-updater-original-pointer-evidence-v1",
                "source_only":True, "vendor_binary_executed":False,
                "maskrom_or_updater_acceptance_proven":False,
                "roles":pointers,
            }
            (self.out / "spl-updater-pointer-evidence.json").write_text(
                json.dumps(all_pointers, indent=2) + "\n")
            record["spl_updater_pointer_evidence"] = {
                "report":"spl-updater-pointer-evidence.json",
                "spl_entries":len(pointers["spl"]["original_image_pointers"]),
                "updater_entries":len(pointers["az01-update"][
                    "original_image_pointers"]),
                "no_vendor_execution":True,
            }
            self.stages["spl-updater-pointer-evidence"] = {
                "success":(len(pointers["spl"]["original_image_pointers"])>=5
                           and len(pointers["az01-update"][
                               "original_image_pointers"])>=3
                           and all(
                               v["pointers_resolved_from_image"] >= 1
                               for v in pointers.values())),
                "original_words_reported":sum(len(x["original_image_pointers"])
                                               for x in pointers.values()),
                "resolved_original_words_by_stage":{
                    role: v["pointers_resolved_from_image"]
                    for role, v in pointers.items()},
                "original_code_executed":False,
            }
        except Exception as exc:
            record["spl_updater_pointer_error"] = str(exc)
            self.stages["spl-updater-pointer-evidence"] = {
                "success":False,"error":str(exc)}
            self.notes.append("SPL/update pointer audit: " + str(exc))
        # Resolve SPL's FIRST indirect call (Thumb selector), inspect
        # SECOND context-pointer call and boot-media tables, identify
        # Fastboot power-source ADC, and classify 3.10 ELF PLT veneers
        # as linkage, not vendor updater implementation execution.
        try:
            indirect = load_module(
                "tools/mpc-one/synthesize_stage_indirection.py",
                "mpfree_stage_indirection_static",
            )
            indirect_report = indirect.build(
                lambda path: (self.out / path).read_bytes()
            )
            (self.out / "cross-stage-indirection.json").write_text(
                json.dumps(indirect_report, indent=2) + "\n")
            (self.out / "cross-stage-indirection.md").write_text(
                indirect.markdown(indirect_report))
            spl_checks = indirect_report["spl"]["checks"]
            power = indirect_report["fastboot_getvar_power_source"]
            update = indirect_report["updater_import_linkage"]
            supported = (
                spl_checks["initial_spl_indirect_function_pointer_matches_selector"]
                and spl_checks["selector_passes_context_to_indirect_handler"]
                and spl_checks["second_handoff_indirect_dereference_observed"]
                and spl_checks["second_handoff_slot_is_context_plus_0x0c"]
                and spl_checks["five_device_order_entries_captured"]
                and spl_checks["eight_media_mapping_entries_captured"]
                and spl_checks["mapping_table_ends_at_device_order_start"]
                and power["power_source_key_verified"]
                and power["fastboot_getvar_dispatches_to_adc_probe"]
                and power["saradc_channel_0_used"]
                and len(update)==3
                and all(v["stub_decompilation_bad_or_missing"]
                        and not v["dynamic_library_implementation_resolved"]
                        for v in update.values())
            )
            record["cross_stage_indirection"] = {
                "report":"cross-stage-indirection.json",
                "initial_spl_to_boot_selector_static":indirect_report["spl"][
                    "first_indirect_boot_selector_proven"],
                "second_spl_handoff_runtime_proven":False,
                "power_source_getvar_key":power["getvar_query_key"],
                "updater_library_implementation_resolved":False,
                "no_physical_execution":True,
            }
            self.stages["cross-stage-indirection-evidence"] = {
                "success":bool(supported),
                "first_spl_boot_selector_resolved":indirect_report["spl"][
                    "first_indirect_boot_selector_proven"],
                "opaque_spl_media_table_words":sum(len(v.get("entries",[]))
                    for v in indirect_report["spl"]["opaque_media_tables"].values()),
                "power_source_adc_key":power["getvar_query_key"],
                "updater_actual_implementation_resolved":False,
            }
        except Exception as exc:
            record["cross_stage_indirection_error"] = str(exc)
            self.stages["cross-stage-indirection-evidence"] = {
                "success":False,"error":str(exc)}
            self.notes.append("Cross-stage indirection: "+str(exc))
        # Exact ELF ARM relocation metadata from the SHA-verified
        # updater and libaz01-prog. A matching symbol import/export
        # is not proof of dynamic binding, verification or actual IO.
        try:
            elf_files={
                role:self.out/"static"/"v11"/(
                    "v11-elf-linkage-"+role+".json")
                for role in ("az01-update","libaz01-prog")
            }
            els={role:json.loads(path.read_text())
                 for role,path in elf_files.items()}
            for role,report in els.items():
                if (report.get("role")!=role or
                    report.get("vendor_executable_executed") is not False):
                    raise ValueError("Unexpected ELF metadata report: "+role)
            imported=set(els["az01-update"]["az01_import_symbols"])
            exported=set(els["libaz01-prog"]["az01_export_symbols"])
            names=(
                "az01_usb_update_mode","az01_apply_image",
                "az01_download_image",
            )
            target=[]
            for name in names:
                offsets=els["az01-update"]["target_symbol_evidence"][
                    name]["relocation_offsets"]
                target.append({
                    "symbol":name,
                    "imported_by_updater":name in imported,
                    "defined_by_libaz01_prog":name in exported,
                    "updater_relocation_offsets":offsets,
                    "actual_runtime_binding_tested":False,
                })
            result={
                "schema":"mpfree-owner-elf-relocation-crosscheck-v1",
                "owner_sources_sha256":{
                    role:r["source_sha256_verified"] for role,r in els.items()},
                "updater_needed_libraries":els["az01-update"]["needed_sonames"],
                "updater_vs_library_import_export_intersection":sorted(
                    imported&exported)[:100],
                "target_update_functions":target,
                "verified_from_elf_metadata_only":True,
                "vendor_executable_or_shared_object_executed":False,
                "updater_stubs_are_not_implementations":True,
            }
            (self.out/"elf-relocation-crosscheck.json").write_text(
                json.dumps(result,indent=2)+"\n")
            record["elf_relocation_crosscheck"]={
                "report":"elf-relocation-crosscheck.json",
                "matched_target_symbols":sum(
                    v["imported_by_updater"] and
                    v["defined_by_libaz01_prog"] for v in target),
                "live_binding_proven":False,
            }
            self.stages["elf-relocation-crosscheck"]={
                "success":True,
                "matched_target_imports_and_exports":record[
                    "elf_relocation_crosscheck"]["matched_target_symbols"],
                "live_vendor_execution":False,
            }
        except Exception as exc:
            record["elf_relocation_crosscheck_error"]=str(exc)
            self.stages["elf-relocation-crosscheck"]={
                "success":False,"error":str(exc)}
            self.notes.append("ELF relocation crosscheck: "+str(exc))
        # Independent direct checks across SPL media initialization,
        # exact Fastboot ADC thresholds, and the genuine libaz01-prog
        # verification/compatibility/write orchestration functions.
        # Source-only: do NOT run updater, loader or any device backend.
        try:
            follow = load_module(
                "tools/mpc-one/synthesize_multitrack_followups.py",
                "mpfree_verified_multitrack_followups",
            )
            follow_report = follow.build(
                lambda path: (self.out / path).read_bytes()
            )
            (self.out / "multitrack-followups.json").write_text(
                json.dumps(follow_report, indent=2) + "\n")
            (self.out / "multitrack-followups.md").write_text(
                follow.markdown(follow_report))
            record["multitrack_followups"] = {
                "report":"multitrack-followups.json",
                "all_original_source_checks":follow_report[
                    "all_static_preconditions_passed"],
                "spl_boot_order_initially_sentinel":follow_report[
                    "spl"]["checks"]["boot_order_five_original_words_are_deadbeef"],
                "power_source_thresholds":follow_report[
                    "power_source"]["original_threshold_words"],
                "onboard_key_and_verification_code_observed":follow_report[
                    "updater"]["checks"]["onboard_key_lookup_precedes_download_verification"],
                "real_maskrom_or_updater_execution_proven":False,
            }
            self.stages["multitrack-owner-followups"] = {
                "success":bool(follow_report[
                    "all_static_preconditions_passed"]),
                "spl_media_mapping_strings":[
                    x["original_string"] for x in follow_report[
                        "spl"]["mapping_name_pairs"][:3]],
                "power_source_threshold_values":follow_report[
                    "power_source"]["original_threshold_words"],
                "openssl_sha256_verify_256byte_signature":follow_report[
                    "updater"]["checks"]["openssl_sha256_256byte_signature_verify"],
                "vendor_executed":False,
            }
        except Exception as exc:
            record["multitrack_followups_error"]=str(exc)
            self.stages["multitrack-owner-followups"]={
                "success":False,"error":str(exc)}
            self.notes.append("SPL/ADC/updater verification: "+str(exc))
        # Connect the three original USB descriptor selectors to their
        # candidate callback tables and Ghidra-resolved function bodies.
        # A missing operation slot means no callback in THIS backend
        # table; it is not a claim about physical device capabilities.
        try:
            usb_module = load_module(
                "tools/mpc-one/synthesize_usb_backend_coverage.py",
                "mpfree_usb_backend_storage_dispatch_static",
            )
            usb_report = usb_module.build(
                lambda path: (self.out / path).read_bytes()
            )
            (self.out / "usb-backend-callback-coverage.json").write_text(
                json.dumps(usb_report, indent=2) + "\n")
            (self.out / "usb-backend-callback-coverage.md").write_text(
                usb_module.markdown(usb_report))
            record["usb_backend_callback_coverage"] = {
                "report":"usb-backend-callback-coverage.json",
                "source_provenance_matched":usb_report[
                    "all_source_only_backends_matched"],
                "backend_count":len(usb_report["backends"]),
                "physical_usb_or_maskrom_execution_proven":False,
            }
            self.stages["usb-backend-callback-coverage"] = {
                "success":bool(usb_report["all_source_only_backends_matched"]),
                "backend_count":len(usb_report["backends"]),
                "physical_device_tested":False,
            }
        except Exception as exc:
            record["usb_backend_callback_coverage_error"]=str(exc)
            self.stages["usb-backend-callback-coverage"]={
                "success":False,"error":str(exc)}
            self.notes.append("USB backend callback coverage: "+str(exc))
        # Offline cross-stage image format / verified state / XZ / SPL
        # analysis. This consumes only SHA-indexed Ghidra and preverified
        # USB callback reports; NO vendor binary or physical MPC is run.
        try:
            fmt = load_module(
                "tools/mpc-one/synthesize_update_format_and_spl.py",
                "mpfree_update_image_formats_source_only",
            )
            format_report = fmt.build(
                lambda path: (self.out / path).read_bytes()
            )
            (self.out / "update-format-and-spl.json").write_text(
                json.dumps(format_report, indent=2) + "\n")
            (self.out / "update-format-and-spl.md").write_text(
                fmt.markdown(format_report))
            record["update_format_and_spl"] = {
                "report":"update-format-and-spl.json",
                "source_checks":format_report[
                    "all_checkable_static_claims_passed"],
                "image_format_signatures":format_report[
                    "image_format_prefixes"],
                "compressed_bootloader_branch_max_bytes":format_report[
                    "xz_bootloader_check_max_bytes_on_this_path"],
                "xz_streaming_buffer_count":format_report[
                    "xz_partition_buffer_count_observed"],
                "real_vendor_execution_or_maskrom_proven":False,
            }
            self.stages["update-format-and-spl-evidence"] = {
                "success":bool(format_report[
                    "all_checkable_static_claims_passed"]),
                "checks":len(format_report["checks"]),
                "xz_streaming_buffers":format_report[
                    "xz_partition_buffer_count_observed"],
                "physical_device_tested":False,
            }
        except Exception as exc:
            record["update_format_and_spl_error"]=str(exc)
            self.stages["update-format-and-spl-evidence"]={
                "success":False,"error":str(exc)}
            self.notes.append("Format/verification/SPL audit: "+str(exc))
        # Static U-Boot Fastboot download -> fixed RAM buffer -> bootm.
        # Accepted length is not a safe memory region and no command
        # has been issued to the physical MPC during this investigation.
        try:
            ram_model = load_module(
                "tools/mpc-one/synthesize_fastboot_ram_path.py",
                "mpfree_static_fastboot_ram_path",
            )
            ram_report = ram_model.build(
                lambda path: (self.out / path).read_bytes()
            )
            (self.out / "fastboot-ram-path.json").write_text(
                json.dumps(ram_report, indent=2) + "\n")
            (self.out / "fastboot-ram-path.md").write_text(
                ram_model.markdown(ram_report))
            record["fastboot_ram_path"] = {
                "report":"fastboot-ram-path.json",
                "static_evidence_passed":ram_report["all_static_evidence_passed"],
                "receive_base":ram_report["ram_receive_base"],
                "advertised_max_bytes":ram_report["max_advertised_download_bytes"],
                "physical_ram_layout_verified":False,
            }
            self.stages["fastboot-ram-path-evidence"] = {
                "success":bool(ram_report["all_static_evidence_passed"]),
                "source_checks":len(ram_report["checks"]),
                "safe_ram_boot_tested":False,
            }
        except Exception as exc:
            record["fastboot_ram_path_error"]=str(exc)
            self.stages["fastboot-ram-path-evidence"]={
                "success":False,"error":str(exc)}
            self.notes.append("Fastboot RAM boot static path: "+str(exc))
        (self.out / "cross-evidence.json").write_text(json.dumps(record, indent=2) + "\n")
        summary = [
            "# MPFree integrated bootchain run",
            "",
            "This report is automatically derived from a single offline static/QEMU session.",
            "All findings are private and firmware-derived; DO NOT commit this ZIP to GitHub.",
            "",
            "## Status",
        ]
        for name, stage in self.stages.items():
            summary.append("- " + name + ": " +
                           ("PASS" if stage.get("success") else "INCOMPLETE/FAIL") +
                           (" (" + str(stage.get("error") or stage.get("reason")) + ")"
                            if stage.get("error") or stage.get("reason") else ""))
        summary += ["", "## USB protocol identity (offline static + upstream)"]
        for entry in record.get("usb_protocol_evidence", {}).get("identifications", []):
            desc = entry["descriptor"]
            summary.append("- " + "/".join(desc[k] for k in ("class", "subclass", "protocol"))
                           + ": " + entry["family"]
                           + " (constructor-xref=" + str(entry["constructor_confirmed"]) + ")")
        summary.append("- Physical target runtime stage, ROM state and write authorization unproven.")
        summary += ["", "## FF/61/7A custom-framed USB backend"]
        framing = record.get("ff617a_framing_evidence")
        if framing:
            summary.append("- Equality-compared message IDs: " + str(framing["compared_message_identifiers"]))
            summary.append("- Outgoing constructed control words: " + str(framing["constructed_outgoing_values"]))
            summary.append("- Range bound (not a command): " + str(framing["range_bound_values"]))
            summary.append("- ARM instructions captured in Ghidra: " + str(framing["machine_instructions_captured"]))
            summary.append("- Response predicates verified: " + str(framing["all_response_checks_observed"]))
            summary.append("- Linux usbdevfs async URB constants present: " + str(framing["ioctls_verified"]))
            summary.append("- See usb-ff617a-framing.json for detailed provenance and type-inference warnings.")
            summary.append("- Protocol identity UNKNOWN; no original vendor code executed.")
        summary += ["", "## ARM return-register evidence"]
        arm = record.get("arm_return_contract_evidence")
        if arm:
            summary.append("- Direct BL callsites immediately testing r0: " +
                           str(arm["status_test_callsites"]))
            summary.append("- CMP/other explicit comparisons: " + str(arm["explicit_cmp_callsites"]) +
                           "; SUBS-zero flag tests: " + str(arm["subs_zero_test_callsites"]))
            summary.append("- BL callsites copying r0 into r4: " +
                           str(arm["r0_copy_callsites"]))
            summary.append("- Of those, r4 restored into r0 at normal return: " +
                           str(arm["r4_copy_status_return_passthroughs"]))
            summary.append("- Normal fallthrough paths returning the callee r0 unchanged: " +
                           str(arm["normal_fallthrough_r0_passthroughs"]))
            summary.append("- Return passthrough proves only the normal fallthrough, not all branches.")
            summary.append("- See arm-return-contracts.json and .md for addresses and caveats.")
        summary += ["", "## Broad ROM / SPL / U-Boot / Update / USB / Recovery question matrix"]
        wide = record.get("wide_research_matrix")
        if wide:
            summary.append("- Questions tracked: " + str(wide["questions"]) +
                           "; tracks: " + str(wide["tracks"]))
            for label, count in wide["counts"].items():
                summary.append("- " + label + ": " + str(count))
            summary.append("- See wide-research-matrix.md for findings, evidence and next actions.")
            summary.append("- Physical MaskROM, fuse policy and independent restore remain unproven.")
        summary += ["", "## Direct boot from existing vendor U-Boot"]
        direct = record.get("direct_boot_focus")
        if direct:
            summary.append("- Focused questions: " + str(direct["questions"]) +
                           " across " + str(direct["topics"]) + " topics.")
            summary.append("- Existing OEM grant, download gate, flash/erase, bootm/bootz evidence: " +
                           str(direct["required_owner_evidence"]))
            summary.append("- Exact U-Boot OEM token and shared permission pointer proof: " +
                           str(direct["literal_pointer_and_token_proven"]))
            summary.append("- Eight-entry Fastboot boot/download dispatch proof: " +
                           str(direct["eight_entry_fastboot_dispatch_proven"]))
            summary.append("- Raw/sparse/erase partition guard evidence: " +
                           str(direct["partition_guard_and_sparse_path_proven"]))
            summary.append("- See direct-boot-focus.md for individual evidence, limitations and follow-ups.")
            summary.append("- No Fastboot commands sent; no rootfs, eMMC or device modified.")
        summary += ["", "## ARM guest P6 boot/kernel/module compatibility"]
        compat = record.get("qemu_directboot_rootfs_compat")
        if compat:
            summary.append("- Required stock boot assets: " +
                           str(sum(compat["required_boot_assets"].values())) +
                           "/" + str(len(compat["required_boot_assets"])))
            summary.append("- Official report's SHA is raw script payload, not full uImage.")
            summary.append("- Guest header and exact raw script SHA match official 3.10: " +
                           str(compat.get("boot_script_header_and_raw_script_match_official")))
            summary.append("- Guest boot script executable commands match official 3.10: " +
                           str(compat.get("boot_script_commands_match_official")))
            summary.append("- Independent full original-image SHA available: " +
                           str(not compat["official_full_image_sha256_not_provided"]))
            if compat.get("boot_script_decode_error"):
                summary.append("- Boot script decode requires investigation: " +
                               str(compat["boot_script_decode_error"]))
            summary.append("- Modules inventory includes 6.18.44: " +
                           str(compat["observed_modules_kernel_6_18_44"]))
            summary.append("- Vendor P6 inspected only; no independent MPFree image tested.")
        summary += ["", "## 3.10 LoadPin / bootloader digest static evidence"]
        security = record.get("security_310_utilities")
        if security:
            summary.append("- Static ARM evidence for RC4/SHA256 and LoadPin ioctl: " +
                           str(security["code_evidence_verified"]))
            summary.append("- Linux LoadPin ioctl 0x40044c00 sets trusted dm-verity digests.")
            summary.append("- Kernel enforcement and vendor tool invocation were NOT tested.")
            summary.append("- See security-310-utilities.json and .md for details.")
        summary += ["", "## Physical recovery button: U-Boot ADC evidence"]
        adc = record.get("recovery_adc_evidence")
        if adc:
            summary.append("- Original ARM + diagnostics: ADC channel 1 <= " +
                           str(adc["threshold_unsigned_max"]) +
                           " sets the recovery flag.")
            summary.append("- ADC and board environment export share flag pointer: " +
                           str(adc["same_flag_exported_to_dorecovery"]))
            summary.append("- Saved dorecovery env selects stock Fastboot.")
            summary.append("- No physical button / immutable MaskROM recovery test.")
        summary += ["", "## SPL indirect handoff and 3.10 updater trampoline pointers"]
        cross_stage = record.get("spl_updater_pointer_evidence")
        if cross_stage:
            summary.append("- SPL original-image pointer slots: " +
                           str(cross_stage["spl_entries"]))
            summary.append("- Updater original-image callback/trampoline slots: " +
                           str(cross_stage["updater_entries"]))
            summary.append("- Original-word evidence only; ROM load and updater execution not tested.")
        summary += ["", "## SPL Thumb entry, power-source ADC and updater linkage"]
        indirect = record.get("cross_stage_indirection")
        if indirect:
            summary.append("- Initial SPL indirect branch reaches boot selector: " +
                           str(indirect["initial_spl_to_boot_selector_static"]))
            summary.append("- Next SPL context-based handoff has NOT been run or verified.")
            summary.append("- Fastboot getvar query on ADC channel 0: " +
                           str(indirect["power_source_getvar_key"]))
            summary.append("- Updater linkage stubs are not library implementations.")
            summary.append("- See cross-stage-indirection.json and .md.")
        summary += ["", "## Firmware updater ARM ELF linkage metadata"]
        links = record.get("elf_relocation_crosscheck")
        if links:
            summary.append("- Updater symbols also exported by libaz01-prog: " +
                           str(links["matched_target_symbols"]) + "/3 targets.")
            summary.append("- Read-only ELF metadata; actual linker bindings and updater execution unproven.")
            summary.append("- See elf-relocation-crosscheck.json for explicit relocation offsets.")
        summary += ["", "## Owner source followups: SPL, ADC, update verification"]
        owner = record.get("multitrack_followups")
        if owner:
            summary.append("- SPL stored device priority is sentinel, not live order: " +
                           str(owner["spl_boot_order_initially_sentinel"]))
            summary.append("- Original power-source boundaries: " +
                           str(owner["power_source_thresholds"]))
            summary.append("- Update-library onboard key / verify-before-apply observed: " +
                           str(owner["onboard_key_and_verification_code_observed"]))
            summary.append("- Image verification is cached/flag-dependent; runtime security unproven.")
            summary.append("- See multitrack-followups.json/.md.")
        summary += ["", "## Three USB backend callback capabilities"]
        usb_slots = record.get("usb_backend_callback_coverage")
        if usb_slots:
            summary.append("- Three original table and Ghidra callback sets match: " +
                           str(usb_slots["source_provenance_matched"]))
            summary.append("- Fastboot has stream+commit slots; FF/61/7A lacks commit; RockUSB-like lacks stream+commit.")
            summary.append("- Static slots only: neither physical MaskROM nor recovery is proven.")
        summary += ["", "## 3.10 image format / verification / XZ and SPL"]
        formats = record.get("update_format_and_spl")
        if formats:
            summary.append("- Source image signatures: " +
                           str(formats["image_format_signatures"]))
            summary.append("- Observed xz bootloader branch size check: " +
                           str(formats["compressed_bootloader_branch_max_bytes"]) +
                           " bytes.")
            summary.append("- XZ partition streaming allocates " +
                           str(formats["xz_streaming_buffer_count"]) +
                           " x 1 MiB buffers in this branch.")
            summary.append("- Verification state 2 and option 2 bypass *rechecking*;")
            summary.append("  error -13 meaning/object reuse and actual policy unproven.")
            summary.append("- See update-format-and-spl.json and .md.")
        summary += ["", "## U-Boot Fastboot download / RAM / bootm"]
        ram = record.get("fastboot_ram_path")
        if ram:
            summary.append("- Static upload base: " + hex(ram["receive_base"]))
            summary.append("- Advertised maximum bytes (NOT known free RAM): " +
                           hex(ram["advertised_max_bytes"]))
            summary.append("- Download permission and legacy/FDT bootm paths: " +
                           str(ram["static_evidence_passed"]))
            summary.append("- No verified physical RAM/FDT layout or Fastboot execution.")
            summary.append("- See fastboot-ram-path.json and .md.")
        summary += ["", "## Guest/static image consistency"]
        if record["match_tests"]:
            for entry in record["match_tests"]:
                summary.append("- " + entry["role"] + ": " +
                               ("MATCH" if entry["match"] is True else
                                "MISMATCH" if entry["match"] is False else
                                "NOT ESTABLISHED"))
        else:
            summary.append("- Not established; see cross-evidence.json.")
        summary += [
            "",
            "## Research scope",
            "- Broad Ghidra: SPL entry, DDR/FDT, ROM/boot choice, U-Boot/Fastboot, permission, RAM, recovery.",
            "- V11 Ghidra: updater verification states, function-pointer backend selectors, RockUSB framing.",
            "- QEMU: real ARMv7 Linux/userspace boot, virtual hardware boundary, loader dependencies and passive file inventory.",
            "- QEMU synthetic USB models: independent ARM executables test interface selection (13 cases) and endpoint discovery (21 cases) on a read-only guest fixture disk, not vendor code.",
            "",
            "## Still not proven",
            "- Silicon MaskROM entry, unsigned owner-SPL execution, eMMC hardware restoration.",
            "- Real RK3288 peripheral behavior, U-Boot Fastboot runtime authorization or safe physical flash writes.",
            "- Live original updater state transitions: the guest did not execute vendor updater code.",
            "",
            "Use cross-evidence.json, session-summary.json, stage console logs and Ghidra JSON for the next analysis.",
            "",
        ]
        (self.out / "RESEARCH-SUMMARY.md").write_text("\n".join(summary))
        self.say("Cross-evidence synthesis complete: RESEARCH-SUMMARY.md / cross-evidence.json")

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
        try:
            lab.synthesize()
        except Exception:
            lab.say("CROSS-EVIDENCE ERROR: " + traceback.format_exc())
            lab.notes.append("Cross-evidence synthesis failed; source reports still packaged.")
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
