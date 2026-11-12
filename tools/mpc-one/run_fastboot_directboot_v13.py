#!/usr/bin/env python3
"""V13 owner-private research round: concurrent Ghidra pointer/dispatch analysis + ARM QEMU.

Read-only saved Ghidra projects, verified offline regular-file firmware, and
synthetic-only QEMU ARM virt. NO MPC, Fastboot transport, USB passthrough, live
block devices, original updater execution, or firmware changes. ONE private ZIP.
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
import re
import subprocess
import tempfile
import time
import traceback
import zipfile

ROOT = Path(__file__).resolve().parents[2]
WORK = Path.home() / "mpfree-work"
BRANCH = "research/fastboot-directboot-v13-20261112"
SCRIPT = "MPFreeFastbootDirectBootV13.py"
def load(path, key):
    spec = importlib.util.spec_from_file_location(key, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
v12 = load("tools/mpc-one/run_fastboot_directboot_v12.py", "mpfree_v13_import_v12")
def record(dst, name, obj):
    v12.write_json(dst / name, obj)
def ghidra(role, dst):
    result = {"stage": "ghidra_" + role, "success": False,
              "source_verified": False, "vendor_executed": False,
              "no_device_contact": True}
    report = dst / ("ghidra-" + role + "-v13.json")
    log = dst / ("ghidra-" + role + "-v13-console.log")
    try:
        launcher = v12.ghidra_launcher()
        if not launcher: raise RuntimeError("No PyGhidra launcher available")
        name = v12.project(role)
        input_name = ("p1-rk3288-sram-stage.bin" if role == "p1"
                      else "p3-uimage-payload.bin")
        command = launcher + [
            str(v12.PROJECTS), name, "-process", input_name,
            "-readOnly", "-noanalysis",
            "-scriptPath", str(ROOT / "tools/ghidra"),
            "-postScript", SCRIPT, role, str(report),
            "-log", str(dst / ("ghidra-" + role + "-v13-engine.log")),
        ]
        with log.open("w") as handle:
            proc = subprocess.run(command, stdout=handle, stderr=subprocess.STDOUT,
                                  timeout=1600, check=False)
        if proc.returncode != 0:
            raise RuntimeError("Ghidra process returned " + str(proc.returncode))
        out = json.loads(report.read_text())
        if (out.get("schema") != "mpfree-v13-fastboot-pointer-dispatch"
                or out.get("role") != role or not out.get("source_verified")):
            raise RuntimeError("Ghidra output missing verified V13 source metadata")
        result.update({"success": True, "source_verified": True,
                       "source_sha256": out["source_sha256"],
                       "decompiled": out["statistics"]["reported_functions"],
                       "pointer_cells": out["statistics"]["pointer_cells"],
                       "dispatch_entries": out["statistics"]["dispatch_entries"],
                       "report": report.name})
        if role == "p3":
            result["unlock_permission_same_address"] = (
                out.get("oem_permission_alias", {}).get("same_nonzero_address"))
    except Exception as exc:
        result["error"] = str(exc)
        with log.open("a") as handle:
            handle.write("ERROR:\n" + traceback.format_exc()[-2000:])
    return result

def load_previous_questions(dst, previous):
    """Import QUESTION TEXT only from an existing SHA-verified V12 ZIP.
    Do not copy proprietary firmware reports or assume 'LEAD' means answered.
    """
    entries = []
    source = {"present": False}
    if previous:
        path = v12.regular(previous)
        with zipfile.ZipFile(path) as z:
            if z.testzip(): raise ValueError("Prior V12 ZIP failed CRC")
            index = json.loads(z.read("ZIP_INDEX.json"))
            names = set()
            for row in index["files"]:
                name = row["name"]
                if name in names or name.startswith("/") or ".." in Path(name).parts:
                    raise ValueError("Untrusted prior ZIP index")
                names.add(name)
                file_bytes = z.read(name)
                if (len(file_bytes) != row["bytes"]
                        or hashlib.sha256(file_bytes).hexdigest() != row["sha256"]):
                    raise ValueError("Prior V12 member SHA mismatch: " + name)
            matrix_file = "V12-RESEARCH-QUESTION-MATRIX.json"
            if matrix_file not in names: raise ValueError("Prior V12 question matrix missing")
            prior = json.loads(z.read(matrix_file))
            for row in prior.get("questions", []):
                entries.append({"id": row["id"], "question": row["question"],
                                "status": "OPEN", "evidence": [], "next_probe": row.get("next_probe")})
            source = {"present": True, "zip_sha256": v12.digest(path),
                      "filename": path.name, "question_count": len(entries)}
    if not entries:
        for key, questions in v12.EXTRA_QUESTIONS.items():
            for i, q in enumerate(questions, 1):
                entries.append({"id": key.upper() + "-" + str(i).zfill(2),
                                "question": q, "status": "OPEN",
                                "evidence": [], "next_probe": "Directed Ghidra review / owner-approved physical observation"})
    record(dst, "V13-PRIOR-QUESTION-LINEAGE.json", source)
    return entries

def evaluate(dst, results, questions):
    path = dst / "ghidra-p3-v13.json"
    p3 = {}
    if path.is_file():
        try: p3 = json.loads(path.read_text())
        except Exception: pass
    path = dst / "ghidra-p1-v13.json"
    p1 = {}
    if path.is_file():
        try: p1 = json.loads(path.read_text())
        except Exception: pass
    storage = {}
    path = dst / "offline-rootfs-and-boot-map-v12.json"
    if path.is_file():
        try: storage = json.loads(path.read_text())
        except Exception: pass
    exact = {}
    cells = p3.get("pointer_cells", {})
    alias = p3.get("oem_permission_alias", {})
    dispatch = p3.get("dispatch_table", {})
    functions = p3.get("function_details", {})
    def is_call(src, target):
        record = functions.get(src, {})
        return any(target in item.get("targets", []) for item in record.get("calls", []))
    exact["oem_unlock_permission_pointer_alias"] = {
        "status": ("ANSWERED_STATIC" if alias.get("same_nonzero_address")
                   else "UNRESOLVED"),
        "gate": alias.get("gate_pointer"),
        "writer": alias.get("unlock_writer_pointer"),
        "evidence": "Verified P3 pointer literal cells 0x0012284c and 0x00122928",
        "caveat": "Static image equality only; no runtime vendor code was executed.",
    }
    d = dispatch.get("entries", [])
    exact["eight_slot_fastboot_dispatch"] = {
        "status": "ANSWERED_STATIC" if len(d) == 8 else "UNRESOLVED",
        "entries": [{"index": row["index"], "name": row.get("command"),
                     "handler": row.get("handler_pointer")} for row in d],
        "caveat": "Dispatch table is read from verified image, not runtime memory.",
    }
    exact["fastboot_download_permission_check"] = {
        "status": "ANSWERED_STATIC" if is_call("0x00122980", "00122834") else "UNRESOLVED",
        "evidence": "Direct call to permission function at 0x00122834 from download handler.",
    }
    exact["fastboot_flash_permission_check"] = {
        "status": "ANSWERED_STATIC" if is_call("0x00122c5c", "00122834") else "UNRESOLVED",
        "evidence": "Direct call to permission function at 0x00122834 from flash handler.",
    }
    exact["fastboot_erase_permission_check"] = {
        "status": "ANSWERED_STATIC" if is_call("0x00122bf8", "00122834") else "UNRESOLVED",
        "evidence": "Direct call to permission function at 0x00122834 from erase handler.",
    }
    exact["fastboot_bootm_direct_call"] = {
        "status": "ANSWERED_STATIC" if is_call("0x001226b4", "00101744") else "UNRESOLVED",
        "evidence": "Direct call from fastboot_boot_done to bootm.",
    }
    scripts = storage.get("observations", {}).get("boot_script_flags", {})
    exact["stock_bootscript_bootz"] = {
        "status": "ANSWERED_STATIC" if scripts.get("contains_bootz") else "UNRESOLVED",
        "boot_script_sha256": scripts.get("sha256"),
        "caveat": "Only presence is verified; target image success is not.",
    }
    counts = p3.get("constant_occurrences", {})
    x = counts.get("pmu_bootcount_address_candidate", {})
    exact["pmu_bootcount_at_0xff7300a0"] = {
        "status": "UNRESOLVED",
        "static_candidate_literal_occurrences": x.get("locations", []),
        "limitation": "Missing literal is not proof of absence; computed MMIO possible.",
    }
    exact["ram_image_safe_bounds"] = {
        "status": "UNRESOLVED",
        "limitation": "Parsed bootloader pointers and addresses do not imply safe DRAM availability.",
    }
    exact["maskrom_independent_restore"] = {
        "status": "NEEDS_HARDWARE",
        "limitation": "Generic ARM QEMU does not emulate RK3288 BootROM, USB PHY or eFuse policy.",
    }
    for entry in questions:
        q = entry["question"].lower()
        def mark(name):
            fact = exact[name]
            if fact["status"] == "ANSWERED_STATIC":
                entry["status"] = "ANSWERED_STATIC"
                entry["evidence"].append(name)
        if "which handler dispatches" in q or "unlock command" in q and "present" in q:
            # String presence is recorded from V12; new round does not prove a
            # complete handler-to-runtime-state transition on this query alone.
            if exact["oem_unlock_permission_pointer_alias"]["status"] == "ANSWERED_STATIC":
                entry["evidence"].append("oem_unlock_permission_pointer_alias")
        if "download" in q and "permission" in q:
            mark("fastboot_download_permission_check")
        if "bootm" in q and ("completion" in q or "fastboot" in q):
            mark("fastboot_bootm_direct_call")
        if "boot" in q and "script" in q and "bootz" in q:
            mark("stock_bootscript_bootz")
        if "maskrom" in q or "emmc restore" in q:
            entry["status"] = "NEEDS_HARDWARE"
    record(dst, "V13-EXACT-FINDINGS.json", exact)
    record(dst, "V13-RESEARCH-QUESTION-MATRIX.json", {
        "schema": "mpfree-v13-evidence-classification",
        "total_questions": len(questions),
        "answered_static": sum(row["status"] == "ANSWERED_STATIC" for row in questions),
        "hardware_required": sum(row["status"] == "NEEDS_HARDWARE" for row in questions),
        "question_rows": questions,
        "exact_propositions": exact,
        "stages": results,
    })
    return exact

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--prior-v12", type=Path,
                    help="Optional existing private V12 ZIP, for verified 85-question lineage")
    ap.add_argument("--rootfs", type=Path,
                    default=WORK / "akai-unpacked-20261110T024913Z-1hjja3_r/rootfs.img")
    ap.add_argument("--backup", type=Path,
                    default=Path.home() /
                    "mpfree-backups/mpc-one-emmc-20261109T233604Z-datprr67/mmcblk0.img")
    ap.add_argument("--skip-qemu", action="store_true")
    ap.add_argument("--qemu-boot-timeout", type=int, default=240)
    ap.add_argument("--qemu-probe-timeout", type=int, default=100)
    args = ap.parse_args()
    os.umask(0o077)
    WORK.mkdir(parents=True, exist_ok=True)
    dst = WORK / ("bootchain-wide-" + time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
                  + "-v13-" + str(os.getpid()))
    dst.mkdir(mode=0o700)
    start = time.monotonic()
    results = {}
    print("MPFree V13: concurrent Ghidra + ARM QEMU, no live device access", flush=True)
    print("Owner-private report directory:", dst, flush=True)
    try:
        prior = args.prior_v12
        if not prior:
            candidates = sorted(WORK.glob("bootchain-wide-*-v12-*.zip"),
                                key=lambda p: p.stat().st_mtime, reverse=True)
            if candidates: prior = candidates[0]
        try:
            questions = load_previous_questions(dst, prior)
        except Exception as exc:
            print("Previous V12 ZIP unavailable or invalid:", str(exc), flush=True)
            record(dst, "V13-PRIOR-IMPORT-ERROR.json", {"error": str(exc)})
            questions = load_previous_questions(dst, None)
        with tempfile.TemporaryDirectory(prefix="mpfree-v13-no-device-", dir=WORK) as td:
            jobs = {
                "ghidra_p1": (ghidra, ("p1", dst)),
                "ghidra_p3": (ghidra, ("p3", dst)),
                "offline_rootfs": (v12.run_offline_rootfs, (dst, args.rootfs, args.backup)),
                "native_synthetic": (v12.run_native_model, (dst, Path(td))),
            }
            if not args.skip_qemu:
                jobs["qemu_synthetic"] = (v12.run_qemu, (dst, Path(td), args))
            with ThreadPoolExecutor(max_workers=5) as pool:
                future_map = {pool.submit(fn, *params): key
                              for key, (fn, params) in jobs.items()}
                for fut in as_completed(future_map):
                    key = future_map[fut]
                    try:
                        results[key] = fut.result()
                    except Exception as exc:
                        results[key] = {"success": False, "error": str(exc),
                                        "trace": traceback.format_exc()[-2200:]}
                    print("V13", key, "SUCCESS" if results[key].get("success")
                          else "INCOMPLETE: " + str(results[key].get("error")), flush=True)
    except KeyboardInterrupt:
        results["interrupted"] = {"success": False, "reason": "keyboard interrupt"}
        print("Interrupted; packaging completed stages.", flush=True)
    finally:
        try:
            if "questions" not in locals():
                questions = []
            exact = evaluate(dst, results, questions)
            lines = [
                "# MPFree V13 — verified owner-private Fastboot evidence", "",
                "This archive contains original SHA-verified firmware-derived static reports.",
                "All original vendor code remained unexecuted.", "",
                "## Static propositions",
            ]
            for name, row in exact.items():
                lines.append("- **%s:** %s" % (name, row["status"]))
                if name == "oem_unlock_permission_pointer_alias":
                    lines.append("  - gate=%s; unlock_writer=%s" %
                                 (row.get("gate"), row.get("writer")))
            lines += ["", "## Results", "QEMU only exercised an original synthetic model.",
                      "Generic ARM virt is not RK3288 and cannot demonstrate direct boot.",
                      "Bootloader Flash/Erase remain forbidden for device tests.",
                      "Private reports are not suitable for the public repository.", "",
                      "## Next research",
                      "Verify original Fastboot command handler flow and image format;",
                      "reconstruct full physical RAM allocations from bootloader and device tree;",
                      "prove SD selection without environment storage writes;",
                      "establish real independent recovery before any bootloader replacement.", ""]
            (dst / "V13-READ-ME-FIRST.md").write_text("\n".join(lines), encoding="utf8")
            record(dst, "V13-STAGE-MANIFEST.json", {
                "branch": BRANCH, "date_utc": datetime.now(timezone.utc).isoformat(),
                "stages": results, "questions": len(questions),
                "duration_seconds": round(time.monotonic() - start, 1),
                "hardware_contact": False, "vendor_binary_executed": False,
                "qemu_board": "generic ARM virt (not RK3288)",
                "read_only_owner_images": True,
            })
            pkg = load("tools/mpc-one/package-private-boot-audit.py",
                       "mpfree_v13_zip_packager")
            output, index = pkg.package(dst)
            print("\nSINGLE PRIVATE ZIP:", output, flush=True)
            print("SHA256:", v12.digest(output), flush=True)
            print("SHA-indexed reports:", len(index["files"]), flush=True)
            print("Upload the ZIP here, not the proprietary results to GitHub.", flush=True)
        except Exception:
            print("PACKAGING FAILED; private files at " + str(dst),
                  flush=True)
            raise

if __name__ == "__main__":
    main()
