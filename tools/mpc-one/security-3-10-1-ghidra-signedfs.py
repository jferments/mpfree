#!/usr/bin/env python3
"""Private offline Ghidra comparison of MPC 3.10.0/3.10.1 az01-signed-fs.

Fresh Ghidra projects and all raw/decompiled firmware-derived content remain
in ~/mpfree-work/firmware-labs/<lab>/. No firmware executed, mounted or flashed,
no MPC network/USB access, no Ghidra changes to ongoing MPFree projects.
"""
from __future__ import annotations
import argparse
from datetime import datetime, timezone
import difflib
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import zipfile

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "tools/ghidra/MPFreeSignedFSForensics.py"
LABROOT = (Path.home() / "mpfree-work/firmware-labs").resolve()
TARGET = "/usr/bin/az01-signed-fs"
SHA = {
    "3.10.0": "8219d59e4074541950089c5ad7177d871a3c0be87797ebc6c5ff5c0629aff3de",
    "3.10.1": "a8edf0e4d7df90a2d1f9c02b3f8b2f6bd267c1a0f9ba2877e53ae46af229817c",
}
CUES = re.compile(r"sign|verify|verity|mount|hash|trust|digest|crypto|rsa|sha|key|"
                  r"secure|loadpin|root|loop|dm-|/dev/|/secure|ioctl|memcmp|strcmp|"
                  r"open|read|write|stat|error", re.I)


def hashfile(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for piece in iter(lambda: f.read(1 << 20), b""):
            h.update(piece)
    return h.hexdigest()


def run(cmd, log: Path, timeout=1800):
    with log.open("w", encoding="utf-8") as f:
        p = subprocess.run([str(x) for x in cmd], stdout=f, stderr=subprocess.STDOUT,
                           timeout=timeout, check=False)
    return p.returncode


def check_lab(arg: Path):
    p = arg.expanduser()
    if p.is_symlink() or not p.is_dir() or p.resolve().parent != LABROOT:
        raise ValueError("--lab must be an existing real directory directly in " + str(LABROOT))
    return p.resolve()


def source(lab, ver):
    note = lab / "reports" / (ver + "-rootfs-path.txt")
    if not note.is_file() or note.is_symlink():
        raise ValueError("missing rootfs path report: " + str(note))
    p = Path(note.read_text(encoding="utf-8").strip())
    if p.is_symlink() or not p.is_file() or not stat.S_ISREG(p.lstat().st_mode):
        raise ValueError("refusing nonregular rootfs " + str(p))
    if lab not in p.resolve().parents:
        raise ValueError("rootfs is outside the private lab")
    with p.open("rb") as f:
        f.seek(1080)
        if f.read(2) != b"\x53\xef":
            raise ValueError("not ext4 rootfs: " + str(p))
    return p


def extract(src, dest):
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.exists() or dest.is_symlink():
        raise ValueError("refusing to overwrite " + str(dest))
    cmd = ["debugfs", "-R", "dump " + TARGET + " " + str(dest), str(src)]
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=90)
    if p.returncode != 0 or not dest.is_file() or not stat.S_ISREG(dest.lstat().st_mode):
        raise RuntimeError("debugfs failed: " + p.stderr[:400])


def launcher():
    env = os.environ.get("GHIDRA_HEADLESS")
    if env:
        path = Path(env).expanduser()
        if not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError("GHIDRA_HEADLESS is not an executable path: " + str(path))
        if path.name == "analyzeHeadless":
            raise ValueError("PyGhidra Python3 runtime requires pyghidraRun -H (not analyzeHeadless)")
        return [str(path), "-H"]
    home = os.environ.get("GHIDRA_HOME")
    if home:
        p = Path(home).expanduser() / "support/pyghidraRun"
        if p.is_file() and os.access(p, os.X_OK):
            return [str(p), "-H"]
    if shutil.which("snap"):
        res = subprocess.run(["snap", "list", "ghidra"], capture_output=True, timeout=30)
        if res.returncode == 0:
            return ["snap", "run", "ghidra.ghidra", "-H"]
    raise ValueError("No PyGhidra headless launcher: configure GHIDRA_HOME=/your/ghidra-directory")


def tokens(row):
    return row.get("mnemonics", [])


def compare_reports(old, new):
    fa = old["functions"]
    fb = new["functions"]
    pairs = []
    assigned_a, assigned_b = set(), set()
    # Exact instruction-sequence matches; a changed address alone should
    # not cause a false alarm, but matching mnemonics is not semantic proof.
    for i, f in enumerate(fa):
        choices = [(abs(len(tokens(f)) - len(tokens(g))), j)
                   for j, g in enumerate(fb)
                   if j not in assigned_b and f.get("mnemonic_sha256") == g.get("mnemonic_sha256")]
        if choices:
            _, j = min(choices)
            assigned_a.add(i); assigned_b.add(j)
            pairs.append((i, j, 1.0, "same mnemonic sequence (not byte equivalence)"))
    # For unmatched functions, use a bounded structural similarity pass.
    cand = []
    for i, f in enumerate(fa):
        if i in assigned_a: continue
        t1 = tokens(f)
        for j, g in enumerate(fb):
            if j in assigned_b: continue
            t2 = tokens(g)
            if not t1 or not t2: continue
            ratio = difflib.SequenceMatcher(None, t1, t2, autojunk=False).ratio()
            if ratio >= .45:
                cand.append((ratio, i, j))
    for ratio,i,j in sorted(cand, reverse=True):
        if i in assigned_a or j in assigned_b: continue
        assigned_a.add(i); assigned_b.add(j)
        pairs.append((i, j, ratio, "heuristically aligned; verify in Ghidra"))
    paired = []
    for i,j,ratio,note in sorted(pairs):
        x,y=fa[i],fb[j]
        calls0 = {c["name"] for c in x.get("calls",[])}
        calls1 = {c["name"] for c in y.get("calls",[])}
        interesting = bool(CUES.search(x.get("name", "") + y.get("name", "") +
                                       " ".join(calls0 | calls1) +
                                       x.get("decompiled_c", "")[:1500] +
                                       y.get("decompiled_c", "")[:1500]))
        paired.append({"old": {"name": x["name"], "entry": x["entry"], "instructions":x["instructions"]},
                       "new": {"name": y["name"], "entry": y["entry"], "instructions":y["instructions"]},
                       "mnemonic_similarity":round(ratio,4), "matching_method":note,
                       "changed_named_call_targets":{"removed":sorted(calls0-calls1),"added":sorted(calls1-calls0)},
                       "security_cue":interesting})
    return {"old_function_count":len(fa), "new_function_count":len(fb),
            "pairs":paired, "unmatched_old":[{"name": f["name"],"entry": f["entry"]}
                    for i,f in enumerate(fa) if i not in assigned_a],
            "unmatched_new":[{"name": f["name"],"entry": f["entry"]}
                    for j,f in enumerate(fb) if j not in assigned_b],
            "limitations":["Function pairing is heuristic; different function recovery can look like code changes.",
                           "Mnemonic equality does not mean equivalent operands, control flow, or security behavior.",
                           "Open original Ghidra projects and check matched functions before conclusions."]}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lab", type=Path, required=True)
    args = ap.parse_args()
    os.umask(0o077)
    lab = check_lab(args.lab)
    if not SCRIPT.is_file():
        raise RuntimeError("Missing Ghidra companion script at " + str(SCRIPT))
    if not shutil.which("debugfs"):
        raise RuntimeError("Install e2fsprogs (debugfs) first")
    cmd = launcher()
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    target = lab / "reports" / ("signedfs-ghidra-" + stamp + "-" + str(os.getpid()))
    bins = lab / "artifacts" / target.name
    target.mkdir(mode=0o700)
    bins.mkdir(mode=0o700)
    notes=[]
    results={}
    for ver in ("3.10.0", "3.10.1"):
        binary = bins / ver / "az01-signed-fs"
        extract(source(lab, ver), binary)
        value = hashfile(binary)
        if value != SHA[ver]:
            raise RuntimeError("Unexpected " + ver + " signed-fs SHA256: " + value +
                               " (expected " + SHA[ver] + ")")
        project_dir = bins / "ghidra-projects"
        project_dir.mkdir(exist_ok=True)
        file_report = target / (ver + "-ghidra-functions.json")
        command = cmd + [str(project_dir), "MPFree_SignedFS_" + ver.replace(".", "_"),
                         "-import", str(binary),
                         "-analysisTimeoutPerFile", "1800", "-scriptPath", str(SCRIPT.parent),
                         "-postScript", SCRIPT.name, ver, str(file_report)]
        log = target / (ver + "-ghidra-console.log")
        print("Analyzing " + ver + " with isolated Ghidra project...", flush=True)
        rc = run(command, log, timeout=2100)
        if rc != 0 or not file_report.exists():
            raise RuntimeError("Ghidra failed for " + ver + "; inspect " + str(log))
        results[ver] = json.loads(file_report.read_text(encoding="utf-8"))
        notes.append(ver + ": " + str(results[ver]["function_count"]) +
                     " functions, " + str(results[ver]["string_count"]) + " defined strings")
    delta=compare_reports(results["3.10.0"], results["3.10.1"])
    (target / "function-matches.json").write_text(json.dumps(delta,indent=2)+"\n")
    sa={x["text"] for x in results["3.10.0"].get("strings",[])}
    sb={x["text"] for x in results["3.10.1"].get("strings",[])}
    (target / "string-differences.json").write_text(json.dumps({"removed":sorted(sa-sb),
        "added":sorted(sb-sa), "common":len(sa&sb)},indent=2)+"\n")
    low=[p for p in delta["pairs"] if p["mnemonic_similarity"] < .999]
    top=sorted(low,key=lambda p:(not p["security_cue"],p["mnemonic_similarity"]))
    summary=["MPFree 3.10.1 az01-signed-fs Ghidra comparison", "PRIVATE. STATIC ONLY. NO MPC ACCESS.",
             "Source binary SHA-256 values validated against previous ELF delta report.",
             *notes, "", "Function matches: " + str(len(delta["pairs"])),
             "Structurally different paired functions: " + str(len(low)),
             "Unmatched old/new functions: " + str(len(delta["unmatched_old"])) + "/" + str(len(delta["unmatched_new"])),
             "Strings removed/added: " + str(len(sa-sb)) + "/" + str(len(sb-sa)), "",
             "Most changed paired functions (investigation priorities; NOT confirmed regressions):"]
    for row in top[:40]:
        summary.append("  " + row["old"]["name"] + "@" + row["old"]["entry"] +
                       " => " + row["new"]["name"] + "@" + row["new"]["entry"] +
                       " similarity=" + str(row["mnemonic_similarity"]) +
                       " security_cue=" + str(row["security_cue"]))
        if row["changed_named_call_targets"]["added"] or row["changed_named_call_targets"]["removed"]:
            summary.append("    calls: " + str(row["changed_named_call_targets"]))
    summary.extend(["", "Review both private Ghidra projects and JSON decompilation before asserting altered signature policy.",
                    "The official update's bootloader state, firmware rollback, and runtime kexec have NOT been tested."])
    (target/"SUMMARY.txt").write_text("\n".join(summary)+"\n")
    (target/"notes.json").write_text(json.dumps(notes,indent=2)+"\n")
    archive=lab/("mpc-gen1-3101-signedfs-ghidra-"+stamp+".zip")
    with zipfile.ZipFile(archive,"x",compression=zipfile.ZIP_DEFLATED,compresslevel=6) as z:
        for f in sorted(target.iterdir()):
            if f.is_symlink() or not f.is_file() or f.suffix not in (".json", ".txt", ".log"):
                raise RuntimeError("Unexpected report: "+str(f))
            z.write(f,f.name)
    print("\n".join(summary))
    print("PRIVATE ZIP TO UPLOAD:", archive)
    print("Ghidra projects (not zipped):", bins/"ghidra-projects")
    return 0

if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, RuntimeError, OSError, subprocess.TimeoutExpired) as exc:
        print("ERROR: " + str(exc),file=sys.stderr)
        sys.exit(1)
