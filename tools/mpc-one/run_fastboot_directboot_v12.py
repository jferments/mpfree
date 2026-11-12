#!/usr/bin/env python3
"""MPFree V12: concurrent offline Ghidra + disposable ARM QEMU Fastboot/direct-boot audit.

ONE command -> ONE private SHA-indexed ZIP.  Uses the existing saved, pinned
P1/P3 Ghidra projects, an existing QEMU ARM guest clone, and original synthetic
test code.  It NEVER contacts an MPC, opens /dev/mmc*, executes Akai maintenance
code, transmits Fastboot commands, or writes original firmware/backup images.

The ARM virt guest does NOT emulate RK3288 silicon BootROM/SPL/U-Boot/MMIO.
Synthetic policy tests are engineering invariants, NOT observed vendor behavior.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
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

ROOT = Path(__file__).resolve().parents[2]
WORK = Path.home() / "mpfree-work"
PROJECTS = WORK / "ghidra/projects"
BRANCH = "research/fastboot-directboot-lab-20261112"
SCRIPT = "MPFreeFastbootDirectBootV12.py"
PINNED_P1 = "aa36e0e4033ce7386a210ad3e41f405a0d952a5b2a2c2711328450017414f1ec"
SOURCE = ROOT / "tools/mpc-one/v12_fastboot_boot_policy_model.c"
OUT = None

EXTRA_QUESTIONS = {
    "fastboot_runtime": [
        "Is the exact original MPC One OEM authorization command accepted in runtime Fastboot?",
        "Does an OEM authorization request have persistent eMMC/OTP side effects?",
        "Can Fastboot getvar return partition sizes without authorization?",
        "Is Fastboot download limited by safe memory topology rather than its declared maximum?",
        "What region holds the received payload and how does it avoid relocated U-Boot?",
        "Does Fastboot boot require an authenticated state on this firmware?",
        "Are uImage, FIT, Android and raw zImage handled differently?",
        "Does legacy uImage CRC32 validation differ from cryptographic verification?",
        "Where does bootm put the FDT and its ATAG/kernel command line?",
        "What reset behavior follows unsuccessful bootm?",
        "Is a transient Fastboot RAM-boot path possible with all eMMC partitions unchanged?",
        "Can unauthorized flash and erase be rejected in a simulated, no-device model?",
        "Do the public unlock reports match this exact installed P3 U-Boot binary?",
    ],
    "removable_directboot": [
        "Can existing U-Boot boot a kernel directly from SD without P2 writes?",
        "Can the user interrupt autoboot with bootdelay=0 using serial or buttons?",
        "Which boot sources are enumerated by P1 SPL versus P3 U-Boot?",
        "Does an inserted SD affect BootROM/SPL loader priority with intact eMMC?",
        "Does MMC index 1 refer to physical MPC SD reader in the deployed U-Boot?",
        "Is USB mass-storage a boot candidate or solely an updater device?",
        "Does the vendor boot script require hardcoded ext4 UUIDs?",
        "Can U-Boot load a boot script temporarily without running saveenv?",
        "What are the precise kernel, FDT, ramdisk and framebuffer RAM reservations?",
        "Can the host boot from SD without modifying P1/P2/P3/P6?",
    ],
    "rootfs_compatibility": [
        "Does MPFree's target board use rk3288-az01-acva.dtb instead of the -c variant?",
        "Does the currently installed rootfs include its own boot.scr.uimg?",
        "Does P6 contain the vendor zImage, matching DTB and required drivers?",
        "Can MPFree retain the proven kernel while replacing userspace entirely?",
        "Does bootz accept vendor and owner-produced ARM zImage without a FIT signature?",
        "Which bootloader DTB fixups are required for display/touch/Ethernet?",
        "What kernel/module ABI does 3.10.0 require?",
        "Is LoadPin/verity a Linux runtime policy or mandatory U-Boot boot verification?",
        "Are startup power, boot-counter and hardware watchdog acknowledgments required?",
        "Does an independent rootfs boot in the generic ARM VM?",
        "Is the audio controller usable with existing vendor kernel/DTB after custom userspace init?",
        "Can the proprietary data/content partitions remain unmounted?",
        "Which vendor firmware blobs must be privately extracted for legitimate owner installation?",
    ],
    "recovery_security": [
        "Does 0xff7300a0 hold bootcount_syscon on this exact MPC One firmware?",
        "Does repeated warm reboot independently select Fastboot recovery?",
        "Which successful-boot service or kernel driver clears bootcount?",
        "Does reboot mode 0x5242c301 select loader rather than immutable MaskROM?",
        "Does 3.10 mode-update alias mode-loader?",
        "Is true BootROM USB enumeration proven with intact eMMC?",
        "Can an owner-compiled SRAM first stage execute under the actual fuse policy?",
        "Is independent full eMMC readback/restoration proven without vendor P1/P3?",
        "Can vendor Fastboot restore P6 while P1/P3 are unchanged?",
        "Do rootfs writes target exactly the P6 LBA range?",
        "Can the official 3.10 updater modify early bootloader partitions?",
        "Does U-Boot or SPL enforce an owner-unavailable signature on the raw kernel?",
        "Which GPIO/ADC button combination enters recovery and which stage checks it?",
        "What happens to Linux power/boot counter state during repeated kexec?",
        "Are physical USB-B signals confirmed to attach to ff580000?",
        "What happens if a rootfs replacement's /boot assets are missing?",
        "Can recovery be attempted without any eMMC or OTP modification?",
    ],
    "vm_validation": [
        "Does generic ARM virt boot the cloned 3.10 rootfs from a disposable overlay?",
        "Can a separately compiled ARM synthetic model run inside the QEMU guest?",
        "Does the guest have network and host USB passthrough disabled?",
        "Does a fake Fastboot policy correctly deny non-rootfs partition writes?",
        "Does synthetic reboot clear volatile authorization as specified?",
        "Does synthetic image classification separate uImage/FIT/zImage?",
        "Does synthetic memory bound checking reject wrapping ranges?",
        "Are synthetic PMU bootcount states tested without pretending to emulate vendor U-Boot?",
        "Does guest model result agree with the native model's cases?",
        "Are all private reports packaged into one verified SHA256-indexed ZIP?",
    ],
}

def write_json(path: Path, obj):
    path.write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n", encoding="utf-8")

def digest(path: Path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()

def regular(path: Path):
    path = path.expanduser()
    if (path.is_symlink() or not path.is_file() or
            not stat.S_ISREG(path.lstat().st_mode) or
            str(path).startswith(("/dev/", "/proc/", "/sys/", "/run/media/"))):
        raise ValueError("Require existing regular, non-device, non-symlink input: " + str(path))
    return path

def load(rel, name):
    spec = importlib.util.spec_from_file_location(name, ROOT / rel)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def ghidra_launcher():
    if os.environ.get("GHIDRA_HOME"):
        candidate = Path(os.environ["GHIDRA_HOME"]) / "support/pyghidraRun"
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return [str(candidate), "-H"]
    if shutil.which("snap"):
        p = subprocess.run(["snap", "list", "ghidra"], capture_output=True, timeout=15)
        if p.returncode == 0:
            return ["snap", "run", "ghidra.ghidra", "-H"]
    return None

def project(role):
    name = os.getenv("MPFREE_P1_PROJECT" if role == "p1" else "MPFREE_P3_PROJECT")
    pattern = "MPFree_P1_SRAM_*.gpr" if role == "p1" else "MPFree_P3_Uboot_*.gpr"
    if name:
        matches = [PROJECTS / (name + ".gpr")]
    else:
        matches = sorted(PROJECTS.glob(pattern), key=lambda p: p.stat().st_mtime,
                         reverse=True) if PROJECTS.is_dir() else []
    if not matches or not regular(matches[0]):
        raise FileNotFoundError("Missing pinned Ghidra project " + role)
    return matches[0].stem

def run_ghidra(role, dst: Path):
    result = {"stage": "ghidra_" + role, "success": False,
              "vendor_executed": False, "device_contact": False}
    log = dst / ("ghidra-" + role + "-console.log")
    try:
        launcher = ghidra_launcher()
        if not launcher:
            raise RuntimeError("No Ghidra Snap or GHIDRA_HOME/support/pyghidraRun")
        name = project(role)
        filename = ("p1-rk3288-sram-stage.bin" if role == "p1"
                    else "p3-uimage-payload.bin")
        output = dst / ("ghidra-" + role + "-fastboot-v12.json")
        argv = launcher + [
            str(PROJECTS), name, "-process", filename, "-readOnly", "-noanalysis",
            "-scriptPath", str(ROOT / "tools/ghidra"),
            "-postScript", SCRIPT, role, str(output),
            "-log", str(dst / ("ghidra-" + role + "-engine.log"))
        ]
        with log.open("w") as f:
            p = subprocess.run(argv, stdout=f, stderr=subprocess.STDOUT,
                               timeout=1800, check=False)
        if p.returncode != 0 or not output.is_file():
            raise RuntimeError("Ghidra exit %s or missing output" % p.returncode)
        data = json.loads(output.read_text())
        if data.get("schema") != "mpfree-fastboot-directboot-ghidra-v12" or not data.get("verified_read_only"):
            raise RuntimeError("Ghidra report does not meet V12 verification schema")
        result.update({"success": True, "report": output.name,
                       "verified_source_sha256": data["image_sha256"],
                       "decompiled": data["statistics"]["decompiled_functions"],
                       "markers_found": data["statistics"]["marker_categories_hit"]})
    except Exception as exc:
        result["error"] = str(exc)
        with log.open("a") as f:
            f.write("\nV12 stage failed: " + traceback.format_exc())
    return result

def run_native_model(dst: Path, tmp: Path):
    result = {"stage": "native_synthetic", "success": False,
              "vendor_executed": False, "physical_devices": False}
    log = dst / "native-synthetic-results.log"
    try:
        cc = shutil.which("cc") or shutil.which("gcc")
        if not cc: raise RuntimeError("Native C compiler unavailable")
        binary = tmp / "native-v12-model"
        c = subprocess.run([cc, "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
                            "-o", str(binary), str(regular(SOURCE))],
                           capture_output=True, text=True, timeout=60)
        if c.returncode:
            raise RuntimeError("Native C compile failed: " + c.stderr[:1200])
        p = subprocess.run([str(binary)], capture_output=True, text=True, timeout=30)
        log.write_text(p.stdout + p.stderr)
        rows = [json.loads(x) for x in p.stdout.splitlines() if x.startswith('{"test":')]
        result.update({"success": p.returncode == 0 and len(rows) >= 35 and
                       all(row.get("pass") is True for row in rows),
                       "tests": len(rows), "passed": sum(x["pass"] for x in rows),
                       "model_source_sha256": digest(SOURCE),
                       "guest_execution": False})
    except Exception as exc:
        result["error"] = str(exc)
        log.write_text("Native model failed: " + traceback.format_exc())
    return result

def run_offline_rootfs(dst: Path, rootfs: Path | None, backup: Path | None):
    result = {"stage": "offline_storage", "success": False,
              "writes_to_originals": False, "no_device_contact": True}
    report = {"verified": {}, "observations": {}, "hypotheses": [],
              "limitations": ["This is static source inspection, not proof of U-Boot bootability."]}
    try:
        if backup and backup.is_file():
            backup = regular(backup)
            with backup.open("rb") as f:
                f.seek(64 * 512)
                p1 = f.read(65536)
            actual = hashlib.sha256(p1).hexdigest()
            report["verified"]["p1_partition_sha256"] = actual
            report["verified"]["p1_matches_owner_backup"] = (actual == PINNED_P1)
            if actual != PINNED_P1:
                raise ValueError("Backup P1 differs from verified owner hash; do not infer boot facts")
        else:
            report["observations"]["backup"] = "not present; Ghidra source hash still checked independently"
        if rootfs and rootfs.is_file():
            rootfs = regular(rootfs)
            with rootfs.open("rb") as f:
                f.seek(1080)
                if f.read(2) != b"\x53\xef":
                    raise ValueError("Not a raw ext4 image; refusing debugfs scan")
            report["verified"]["rootfs_size"] = rootfs.stat().st_size
            report["verified"]["rootfs_sha256"] = digest(rootfs)
            if shutil.which("debugfs"):
                for label, command in [
                    ("boot_listing", "ls -l /boot"),
                    ("boot_script_stat", "stat /boot/boot.scr.uimg"),
                    ("zimage_stat", "stat /boot/zImage"),
                    ("dtb_acva_stat", "stat /boot/rk3288-az01-acva.dtb"),
                    ("dtb_acvac_stat", "stat /boot/rk3288-az01-acva-c.dtb"),
                    ("fstab", "cat /etc/fstab"),
                    ("release", "cat /etc/os-release"),
                    ("bootcount_services", "ls -l /etc/systemd/system"),
                ]:
                    p = subprocess.run(["debugfs", "-R", command, str(rootfs)],
                                       capture_output=True, timeout=30)
                    raw = p.stdout.decode("utf-8", "replace")
                    # Never package raw arbitrary config/identity strings.
                    if label in ("fstab", "release"):
                        report["observations"][label] = {
                            "readable": p.returncode == 0, "sha256": hashlib.sha256(p.stdout).hexdigest(),
                            "bytes": len(p.stdout)}
                    else:
                        report["observations"][label] = raw[:7000]
                script = subprocess.run(
                    ["debugfs", "-R", "cat /boot/boot.scr.uimg", str(rootfs)],
                    capture_output=True, timeout=30)
                data = script.stdout
                report["observations"]["boot_script_flags"] = {
                    "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                    "contains_bootz": b"bootz" in data,
                    "contains_bootm": b"bootm" in data,
                    "contains_fdtfile": b"fdtfile" in data,
                    "contains_mmc": b"mmc" in data,
                    "contains_extlinux": b"extlinux" in data,
                }
            else:
                report["observations"]["debugfs"] = "not installed"
        else:
            report["observations"]["rootfs"] = "not found; set --rootfs regular-file.ext4"
        result["success"] = True
    except Exception as exc:
        result["error"] = str(exc)
        report["error"] = traceback.format_exc()[-1800:]
    write_json(dst / "offline-rootfs-and-boot-map-v12.json", report)
    return result

def run_qemu(dst: Path, temp_root: Path, args):
    result = {"stage": "qemu_synthetic", "success": False, "guest_model": False,
              "vendor_executed": False, "hardware_access": False,
              "board": "generic ARM virt; NOT RK3288", "no_network": True,
              "physical_usb_passthrough": False}
    serial = dst / "qemu-v12-console.log"
    proc = None
    master = slave = None
    try:
        q = load("tools/mpc-one/arm_qemu_lab.py", "mpfree_v12_qemu")
        integrated = load("tools/mpc-one/run_bootchain_integrated_lab.py",
                          "mpfree_v12_qemu_storage_mapping")
        for executable in ("qemu-system-arm", "qemu-img", "arm-linux-gnueabihf-gcc",
                           "mkfs.ext4"):
            if not shutil.which(executable):
                raise RuntimeError(executable + " unavailable; QEMU stage skipped")
        source = regular(SOURCE)
        arm = temp_root / "v12_fastboot_model"
        build = subprocess.run([
            "arm-linux-gnueabihf-gcc", "-std=c11", "-O2", "-static", "-s",
            "-Wall", "-Wextra", "-Werror", "-o", str(arm), str(source)],
            capture_output=True, text=True, timeout=90)
        if build.returncode:
            raise RuntimeError("ARM compile failed: " + build.stderr[:800])
        if len(arm.read_bytes()) > 6 * 1024 * 1024:
            raise RuntimeError("Unexpected ARM model executable size")
        staged = temp_root / "staging"
        staged.mkdir(mode=0o700)
        shutil.copy2(arm, staged / "v12_fastboot_model")
        disk = temp_root / "synthetic-only.ext4"
        with disk.open("wb") as f:
            f.truncate(24 * 1024 * 1024)
        mk = subprocess.run(["mkfs.ext4", "-F", "-q", "-d", str(staged), str(disk)],
                            capture_output=True, text=True, timeout=60)
        if mk.returncode:
            raise RuntimeError("Ext4 fixture failed: " + mk.stderr[:800])
        command = q.qemu_args(q.KERNEL)  # verifies cloned rootfs overlay
        command.insert(1, "-snapshot")
        command = integrated.qemu_storage_for_synthetic_model(command, disk)
        if ("-nic" not in command or command[command.index("-nic")+1] != "none" or
                "-monitor" not in command or command[command.index("-monitor")+1] != "none"):
            raise ValueError("QEMU security arguments missing")
        result["guest_kernel_sha256"] = digest(q.KERNEL)
        result["guest_rootfs_clone_sha256"] = digest(q.BASE)
        result["synthetic_arm_model_sha256"] = digest(arm)
        master, slave = pty.openpty()
        proc = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                                close_fds=True, start_new_session=True)
        os.close(slave)
        slave = None
        boot_deadline = time.monotonic() + args.qemu_boot_timeout
        cmd_deadline = None
        sent = False
        transcript = ""
        token = "MPFREE_V12_DONE_" + str(os.getpid())
        lines = [
            "echo MPFREE_V12_VM_BEGIN",
            "mount -t proc proc /proc 2>/dev/null || true",
            "mount -t sysfs sysfs /sys 2>/dev/null || true",
            "mkdir -p /mnt/mpfree-v12",
            "mount -t ext4 -o ro /dev/vda /mnt/mpfree-v12",
            "/mnt/mpfree-v12/v12_fastboot_model",
            "echo " + token,
        ]
        with serial.open("w", encoding="utf-8", errors="replace") as f:
            while True:
                now = time.monotonic()
                if proc.poll() is not None:
                    result["error"] = "Guest exited: " + str(proc.returncode)
                    break
                if not sent and now >= boot_deadline:
                    result["error"] = "Guest did not show root shell"
                    break
                if sent and now >= cmd_deadline:
                    result["error"] = "Guest model timed out"
                    break
                ready, _, _ = select.select([master], [], [], 0.25)
                if ready:
                    try: data = os.read(master, 65536)
                    except OSError: break
                    if not data: break
                    t = data.decode("utf-8", "replace")
                    f.write(t)
                    f.flush()
                    transcript += t
                    if len(transcript) > 2 * 1024 * 1024:
                        transcript = transcript[-2 * 1024 * 1024:]
                    if not sent and re.search(r"(?:^|[\r\n])(?:/ # |# |~ # )", transcript[-4000:]):
                        for line in lines:
                            os.write(master, (line + "\n").encode())
                        sent = True
                        cmd_deadline = time.monotonic() + args.qemu_probe_timeout
                    if sent and token in transcript and "MPFREE_V12_SYNTH_PASS" in transcript:
                        result["guest_model"] = True
                        break
                    if sent and token in transcript:
                        result["error"] = "Guest synthetic model did not pass"
                        break
        rows = []
        for line in transcript.replace("\r", "").splitlines():
            if line.startswith('{"test":'):
                try: rows.append(json.loads(line))
                except ValueError: pass
        result["guest_tests"] = len(rows)
        result["guest_passes"] = sum(int(r.get("pass") is True) for r in rows)
        result["success"] = bool(result["guest_model"] and len(rows) >= 35 and
                                 all(r.get("pass") is True for r in rows))
        if not result["success"] and "error" not in result:
            result["error"] = "Not all test rows were observed"
    except Exception as exc:
        result["error"] = str(exc)
        with serial.open("a") as f:
            f.write("\nQEMU stage exception:\n" + traceback.format_exc()[-3000:])
    finally:
        if proc and proc.poll() is None:
            try:
                os.killpg(proc.pid, signal.SIGTERM)
                proc.wait(timeout=10)
            except Exception:
                try: os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError: pass
                proc.wait(timeout=10)
        if slave is not None:
            os.close(slave)
        if master is not None:
            os.close(master)
        write_json(dst / "qemu-v12-stage.json", result)
    return result

def question_matrix(dst, stage_results):
    file = dst / "ghidra-p3-fastboot-v12.json"
    gh = {}
    if file.is_file():
        try: gh = json.loads(file.read_text())
        except Exception: pass
    existing = gh.get("questions") or {}
    topics = {}
    for k, values in {**existing, **EXTRA_QUESTIONS}.items():
        topics[k] = values
    matrix = []
    markers = gh.get("markers", {})
    for topic, questions in topics.items():
        for index, question in enumerate(questions, 1):
            status = "OPEN - no hardware conclusion"
            leads = []
            if topic in ("fastboot", "fastboot_runtime") and markers.get("unlock", {}).get("present"):
                leads.append("Exact unlock command prefix occurs in verified P3 bytes.")
            if topic == "boot_selection" and markers.get("boot_targets", {}).get("present"):
                leads.append("Saved P3 image contains boot_targets string.")
            if topic == "boot_counter":
                x = gh.get("registers", {}).get("pmu_bootcount_new_hypothesis", {})
                if x.get("total_raw_literal_sites", 0):
                    leads.append("PMU bootcount candidate literal occurs in P3.")
            if topic == "vm_validation" and stage_results.get("qemu_synthetic", {}).get("success"):
                leads.append("ARM guest executed synthetic-only model; not vendor code.")
            if leads:
                status = "STATIC/SYNTHETIC LEAD - runtime behavior unproved"
            matrix.append({"id": topic.upper() + "-" + str(index).zfill(2),
                           "question": question, "status": status, "leads": leads,
                           "next_probe": ("Inspect Ghidra call graph and isolated mock backend; "
                                          "physical verification requires separate owner approval.")})
    write_json(dst / "V12-RESEARCH-QUESTION-MATRIX.json",
               {"schema":"mpfree-v12-question-matrix", "questions": matrix,
                "total_questions": len(matrix), "status_not_a_physical_proof": True,
                "stages": stage_results})
    return matrix

def summarize(dst, stages, questions, elapsed):
    p3 = dst / "ghidra-p3-fastboot-v12.json"
    p1 = dst / "ghidra-p1-fastboot-v12.json"
    def stats(p):
        if not p.is_file(): return "not available"
        try:
            x = json.loads(p.read_text())
            return ("%d decompiled, %d marker categories, unlock marker: %s" %
                    (x["statistics"]["decompiled_functions"],
                     x["statistics"]["marker_categories_hit"],
                     x["markers"]["unlock"]["present"]))
        except Exception: return "invalid report"
    text = [
        "# MPFree V12 owner-private Fastboot/direct-boot research round", "",
        "Execution date (UTC): " + datetime.now(timezone.utc).isoformat(),
        "Branch: " + BRANCH,
        "Elapsed seconds: " + str(round(elapsed, 1)), "",
        "## Concurrent jobs",
    ]
    for key, x in stages.items():
        text.append("- %s: %s%s" % (key, "SUCCESS" if x.get("success") else "INCOMPLETE",
                                   ("; " + x["error"]) if x.get("error") else ""))
    text.extend(["", "## Verified static-project outputs",
                 "- P3: " + stats(p3), "- P1: " + stats(p1),
                 "", "## Question inventory",
                 "- %d questions enumerated in V12-RESEARCH-QUESTION-MATRIX.json." % len(questions),
                 "- Markers/calls provide leads, NOT evidence of executed vendor behavior.",
                 "- See ghidra-p3-fastboot-v12.json for the exact OEM state-machine investigation.",
                 "", "## Scope boundary",
                 "- No MPC, Fastboot, RockUSB, eMMC or USB device commands were issued.",
                 "- Ghidra opened saved owner projects read-only with pinned SHA checks.",
                 "- VM used generic ARM virt, no network/USB passthrough, snapshot rootfs.",
                 "- VM synthetic tests are NOT actual vendor authorization or boot behavior.",
                 "- No claim of MaskROM, independent eMMC restoration or safe flashing.",
                 "- Private firmware-derived reports must not be pushed to GitHub.", "",
                 "## Next evidence",
                 "1. Tie P3 OEM string match and comparisons to the permission-global writer.",
                 "2. Prove authorization/bootm call reachability and runtime RAM limits.",
                 "3. Trace boot_targets, serial interruption, bootcount and board-specific FDT fixups.",
                 "4. Reconcile P6 boot-script/kernel metadata with an external MPFree kernel image.",
                 "5. Require independent recovery proof before bootloader partition changes.", ""])
    (dst / "V12-READ-ME-FIRST.md").write_text("\n".join(text))

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--rootfs", type=Path, default=WORK / "akai-unpacked-20261110T024913Z-1hjja3_r/rootfs.img")
    p.add_argument("--backup", type=Path, default=Path.home() /
                   "mpfree-backups/mpc-one-emmc-20261109T233604Z-datprr67/mmcblk0.img")
    p.add_argument("--qemu-boot-timeout", type=int, default=240)
    p.add_argument("--qemu-probe-timeout", type=int, default=100)
    p.add_argument("--skip-qemu", action="store_true",
                   help="Host-only smoke test; does not change static P1/P3 coverage")
    args = p.parse_args()
    if args.qemu_boot_timeout < 30 or args.qemu_probe_timeout < 20:
        p.error("Timeouts too small")
    if not SOURCE.is_file():
        p.error("Missing synthetic test source (update research branch)")
    os.umask(0o077)
    WORK.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    dst = WORK / ("bootchain-wide-" + stamp + "-v12-" + str(os.getpid()))
    dst.mkdir(mode=0o700)
    started = time.monotonic()
    stages = {}
    print("MPFree V12 CONCURRENT offline Ghidra + isolated ARM QEMU", flush=True)
    print("Research branch:", BRANCH, flush=True)
    print("Output directory:", dst, flush=True)
    print("No physical MPC access or vendor executable invocation.", flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="mpfree-v12-synthetic-", dir=WORK) as folder:
            td = Path(folder)
            jobs = {
                "ghidra_p1": (run_ghidra, ("p1", dst)),
                "ghidra_p3": (run_ghidra, ("p3", dst)),
                "offline_storage": (run_offline_rootfs, (dst, args.rootfs, args.backup)),
                "native_synthetic": (run_native_model, (dst, td)),
            }
            if not args.skip_qemu:
                jobs["qemu_synthetic"] = (run_qemu, (dst, td, args))
            # QEMU, Ghidra P1, Ghidra P3 and non-device static analysis overlap.
            with ThreadPoolExecutor(max_workers=5, thread_name_prefix="mpfree-v12") as pool:
                futures = {pool.submit(fn, *values): name
                           for name, (fn, values) in jobs.items()}
                for future in as_completed(futures):
                    name = futures[future]
                    try: value = future.result()
                    except Exception as exc:
                        value = {"stage": name, "success": False,
                                 "error": str(exc), "traceback": traceback.format_exc()[-1800:]}
                    stages[name] = value
                    print("COMPLETED", name, "SUCCESS" if value.get("success") else
                          "INCOMPLETE: " + value.get("error", "see reports"), flush=True)
    except KeyboardInterrupt:
        print("INTERRUPTED: packaging all reports already produced.", file=sys.stderr)
        stages["interrupted"] = {"success": False, "error": "KeyboardInterrupt"}
    finally:
        try:
            questions = question_matrix(dst, stages)
            summarize(dst, stages, questions, time.monotonic() - started)
            write_json(dst / "V12-STAGE-MANIFEST.json", {
                "schema": "mpfree-v12-concurrent-audit",
                "date_utc": datetime.now(timezone.utc).isoformat(),
                "repository_branch": BRANCH,
                "no_physical_device_access": True,
                "no_vendor_executable_run": True,
                "no_firmware_written": True,
                "qemu_board": "generic ARM virt only",
                "stages": stages, "open_questions": len(questions),
            })
            pkg = load("tools/mpc-one/package-private-boot-audit.py",
                       "mpfree_v12_private_zip_packager")
            archive, index = pkg.package(dst)
            print("\nONE PRIVATE ZIP TO UPLOAD:", archive, flush=True)
            print("ZIP SHA256:", digest(archive), flush=True)
            print("SHA-indexed members:", len(index["files"]), flush=True)
            print("Please upload only that ZIP; keep it out of the public repository.", flush=True)
        except Exception:
            print("PACKAGING ERROR; private files remain in " + str(dst),
                  file=sys.stderr)
            raise

if __name__ == "__main__":
    main()
