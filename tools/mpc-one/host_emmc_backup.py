#!/usr/bin/env python3
"""Full eMMC backup of the MPC One over SSH. Runs on the workstation only.

Called by `collect-over-ssh.sh --backup` after its interface, route, MAC and
host-key checks. On the MPC it only READS:

  * `sh -s hash` / `sh -s stat` running read-only-backup-preflight.sh
    (sysfs/procfs reads, `dd if=... | sha256sum`, no output files);
  * `dd if=/dev/<device> bs=1048576`, whose only output is SSH stdout.

Those five fixed command strings are the only remote commands this program
sends. Nothing on the MPC is mounted, written, synced or restarted.

On the workstation it creates one new directory under --out-root holding
mmcblk0.img, mmcblk0boot0.img, mmcblk0boot1.img, SHA256SUMS and MANIFEST.txt.
Images are kept only if every check below passes; otherwise the partial
images are deleted and MANIFEST-FAILED.txt says why.

Checks:
  1. The disk layout and eMMC model match the baseline file exactly
     (checked before anything is copied).
  2. The MPC hashes p1-p6, boot0, boot1 and sectors 0-63 (MBR + primary GPT)
     itself. The same regions of the copied stream must hash identically.
  3. Block-layer write and discard counters for p1-p6, boot0 and boot1 must
     not change between the first counter read and the end of the copy, and
     the MPC must not reboot (boot_id) in between.
  4. Each image is re-read from the workstation disk (page cache dropped
     for that file) and must hash to the value computed while streaming.
  5. Boot-critical hashes are also compared with the 2026-11-09 boot map.
     A mismatch keeps the (device-verified) backup but exits with status 3,
     because it means boot storage changed since the boot map.

p7 (content) and p8 (data) are mounted read-write on the MPC. Their write
counters are reported; if they changed during the copy, those partitions
are captured like a power-loss snapshot. SSH's per-packet integrity check
covers their bytes in transit.
"""
import argparse
import hashlib
import os
import re
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone

SECTOR = 512
READ_CHUNK = 1 << 20
PROGRESS_EVERY = 512 << 20
STALL_SECONDS = 120
SPACE_MARGIN = 256 << 20
EXIT_OK, EXIT_FAIL, EXIT_BASELINE_CHANGED = 0, 1, 3
HEX64 = re.compile(r"^[0-9a-f]{64}$")

# The only remote commands. Devices appear solely as dd's input (if=);
# there is no output file on the MPC.
PREFLIGHT_COMMANDS = {"hash": "sh -s hash", "stat": "sh -s stat"}
STREAM_COMMANDS = {
    "mmcblk0": "dd if=/dev/mmcblk0 bs=1048576",
    "mmcblk0boot0": "dd if=/dev/mmcblk0boot0 bs=1048576",
    "mmcblk0boot1": "dd if=/dev/mmcblk0boot1 bs=1048576",
}
BOOT_CRITICAL = ("mmcblk0p1", "mmcblk0p2", "mmcblk0p3", "mmcblk0p4",
                 "mmcblk0p5", "mmcblk0p6", "mmcblk0boot0", "mmcblk0boot1")
GPT_HEAD_SECTORS = 64  # protective MBR, primary GPT, padding up to p1


class BackupError(Exception):
    pass


def log(msg=""):
    print(msg, flush=True)


# ---------------------------------------------------------------- baseline

def parse_baseline(path):
    base = {"state": None, "emmc": None, "disk": None, "parts": [], "hw": []}
    with open(path, encoding="utf-8") as fh:
        for lineno, raw in enumerate(fh, 1):
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            f = line.split()
            try:
                if f[0] in ("state", "emmc") and len(f) == 2:
                    base[f[0]] = f[1]
                elif f[0] == "disk" and len(f) == 3:
                    base["disk"] = {"dev": f[1], "sectors": int(f[2])}
                elif f[0] == "part" and len(f) == 6:
                    base["parts"].append({
                        "dev": f[1], "name": f[2], "start": int(f[3]),
                        "sectors": int(f[4]),
                        "sha256": None if f[5] == "-" else f[5]})
                elif f[0] == "hw" and len(f) == 4:
                    base["hw"].append({"dev": f[1], "sectors": int(f[2]),
                                       "sha256": f[3]})
                else:
                    raise ValueError
            except ValueError:
                raise BackupError(f"{path}:{lineno}: cannot parse baseline line")
    if not (base["state"] and base["emmc"] and base["disk"]):
        raise BackupError(f"{path}: needs state, emmc and disk lines")
    if base["disk"]["dev"] != "mmcblk0":
        raise BackupError(f"{path}: only mmcblk0 is supported")
    if sorted(p["dev"] for p in base["parts"]) != [f"mmcblk0p{i}" for i in range(1, 9)]:
        raise BackupError(f"{path}: needs exactly mmcblk0p1..mmcblk0p8")
    if sorted(h["dev"] for h in base["hw"]) != ["mmcblk0boot0", "mmcblk0boot1"]:
        raise BackupError(f"{path}: needs exactly mmcblk0boot0 and mmcblk0boot1")
    end = GPT_HEAD_SECTORS
    for p in sorted(base["parts"], key=lambda p: p["start"]):
        if p["start"] < end or p["sectors"] <= 0:
            raise BackupError(f"{path}: {p['dev']} overlaps or is empty")
        end = p["start"] + p["sectors"]
    if end > base["disk"]["sectors"]:
        raise BackupError(f"{path}: partitions extend past the disk")
    for item in base["parts"] + base["hw"]:
        if item["sha256"] is not None and not HEX64.match(item["sha256"]):
            raise BackupError(f"{path}: bad sha256 for {item['dev']}")
    return base


# ------------------------------------------------------------- remote side

def ssh_base(a):
    return ["ssh", "-T",
            "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes",
            "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=8",
            "-o", "ForwardAgent=no", "-o", "ClearAllForwardings=yes",
            "-o", "Compression=no",
            "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=6",
            "-i", a.key, "-p", str(a.port), f"{a.user}@{a.ip}"]


def parse_preflight(text, mode):
    out = {"mode": None, "boot_id": None, "stat": {}, "geom": {}, "hash": {},
           "absent": [], "emmc": None, "kernel": None, "ext_csd": None,
           "end": False}
    for line in text.splitlines():
        f = line.split()
        if not f:
            continue
        tag = f[0]
        if tag == "ERROR":
            raise BackupError(f"preflight reported: {line}")
        if tag == "MODE" and len(f) == 2:
            out["mode"] = f[1]
        elif tag == "BOOT_ID" and len(f) == 2:
            out["boot_id"] = f[1]
        elif tag == "STAT" and len(f) >= 13:
            out["stat"][f[1]] = [int(x) for x in f[2:]]
        elif tag == "GEOM" and len(f) == 5:
            out["geom"][f[1]] = {"start": f[2], "sectors": int(f[3]), "name": f[4]}
        elif tag == "HASH" and len(f) == 3:
            out["hash"][f[1]] = f[2]
        elif tag == "ABSENT" and len(f) == 2:
            out["absent"].append(f[1])
        elif tag == "EMMC" and len(f) == 2:
            out["emmc"] = f[1]
        elif tag == "KERNEL":
            out["kernel"] = " ".join(f[1:])
        elif tag == "EXT_CSD":
            out["ext_csd"] = f[1] if len(f) == 2 else "-"
        elif tag == "END" and len(f) == 2:
            out["end"] = f[1] == mode
    if out["mode"] != mode or not out["end"]:
        raise BackupError(f"preflight '{mode}' output was incomplete")
    if not out["boot_id"] or out["boot_id"] == "-":
        raise BackupError("preflight could not read the MPC boot_id")
    return out


def run_preflight(a, script, mode):
    cmd = ssh_base(a) + [PREFLIGHT_COMMANDS[mode]]
    try:
        r = subprocess.run(cmd, input=script, capture_output=True, timeout=900)
    except subprocess.TimeoutExpired:
        raise BackupError(f"preflight '{mode}' timed out")
    if r.returncode != 0:
        err = r.stderr.decode("utf-8", "replace").strip()[-1500:]
        raise BackupError(f"preflight '{mode}' failed (exit {r.returncode}): {err}")
    return parse_preflight(r.stdout.decode("utf-8", "replace"), mode)


def counters(stat):
    """(writes completed, sectors written, discards, sectors discarded)."""
    disc = stat[11] if len(stat) > 11 else 0
    disc_sectors = stat[13] if len(stat) > 13 else 0
    return (stat[4], stat[6], disc, disc_sectors)


def write_all(fd, data):
    view = memoryview(data)
    while view:
        n = os.write(fd, view)
        view = view[n:]


def stream(a, dev, path, total, regions):
    """Copy /dev/<dev> from the MPC into path. Returns whole + region hashes."""
    cmd = ssh_base(a) + [STREAM_COMMANDS[dev]]
    whole = hashlib.sha256()
    hashers = {name: hashlib.sha256() for name, _, _ in regions}
    received = 0
    started = time.monotonic()
    next_report = PROGRESS_EVERY
    out_fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    errfile = tempfile.TemporaryFile()
    proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=errfile)
    try:
        src = proc.stdout.fileno()
        while True:
            ready, _, _ = select.select([src], [], [], STALL_SECONDS)
            if not ready:
                raise BackupError(f"no data from the MPC for {STALL_SECONDS} s")
            buf = os.read(src, READ_CHUNK)
            if not buf:
                break
            if received + len(buf) > total:
                raise BackupError(f"{dev}: MPC sent more than {total} bytes")
            write_all(out_fd, buf)
            whole.update(buf)
            view = memoryview(buf)
            for name, lo, hi in regions:
                s, e = max(lo, received), min(hi, received + len(buf))
                if s < e:
                    hashers[name].update(view[s - received:e - received])
            received += len(buf)
            if received >= next_report:
                elapsed = time.monotonic() - started
                rate = received / elapsed if elapsed > 0 else 0
                left = (total - received) / rate if rate else 0
                log(f"  {dev}: {received >> 20} / {total >> 20} MiB "
                    f"({100 * received // total}%), {rate / 1e6:.1f} MB/s, "
                    f"about {left / 60:.0f} min left")
                next_report += PROGRESS_EVERY
        rc = proc.wait(timeout=60)
        os.fsync(out_fd)
    except BaseException:
        proc.kill()
        proc.wait()
        raise
    finally:
        os.close(out_fd)
        proc.stdout.close()
    errfile.seek(0)
    err = errfile.read().decode("utf-8", "replace").strip()
    errfile.close()
    if rc != 0:
        raise BackupError(f"{dev}: remote dd/ssh exit status {rc}: {err[-1500:]}")
    if received != total:
        raise BackupError(f"{dev}: expected {total} bytes, received {received}")
    elapsed = time.monotonic() - started
    return {"sha256": whole.hexdigest(),
            "regions": {n: h.hexdigest() for n, h in hashers.items()},
            "seconds": elapsed, "dd_summary": " / ".join(err.splitlines()[-3:])}


def rehash_from_disk(path):
    digest = hashlib.sha256()
    fd = os.open(path, os.O_RDONLY)
    try:
        try:  # file was fsynced, so this drops its clean pages and forces a disk read
            os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
        except (AttributeError, OSError):
            pass
        while True:
            buf = os.read(fd, 8 << 20)
            if not buf:
                break
            digest.update(buf)
    finally:
        os.close(fd)
    return digest.hexdigest()


# ------------------------------------------------------------------ checks

def check_layout(pre, base):
    problems = []
    if pre["absent"]:
        problems.append("missing on the MPC: " + ", ".join(pre["absent"]))
    if pre["emmc"] != base["emmc"]:
        problems.append(f"eMMC model {pre['emmc']} != {base['emmc']}")
    disk = pre["geom"].get("mmcblk0")
    if not disk or disk["sectors"] != base["disk"]["sectors"]:
        problems.append(f"mmcblk0 size {disk and disk['sectors']} != {base['disk']['sectors']}")
    for p in base["parts"]:
        g = pre["geom"].get(p["dev"])
        want = (str(p["start"]), p["sectors"], p["name"])
        got = g and (g["start"], g["sectors"], g["name"])
        if got != want:
            problems.append(f"{p['dev']} is {got}, expected {want}")
    for h in base["hw"]:
        g = pre["geom"].get(h["dev"])
        if not g or g["sectors"] != h["sectors"]:
            problems.append(f"{h['dev']} size {g and g['sectors']} != {h['sectors']}")
    for dev in BOOT_CRITICAL + ("gpt-head",):
        if not HEX64.match(pre["hash"].get(dev, "")):
            problems.append(f"the MPC could not hash {dev}")
    for dev in BOOT_CRITICAL + ("mmcblk0", "mmcblk0p7", "mmcblk0p8"):
        if len(pre["stat"].get(dev, [])) < 11:
            problems.append(f"no write counters for {dev}")
    if problems:
        raise BackupError("the MPC does not match the baseline; nothing was copied:\n  - "
                          + "\n  - ".join(problems))


def baseline_differences(pre, base):
    diffs = []
    for item in base["parts"] + base["hw"]:
        if item["sha256"] and pre["hash"].get(item["dev"]) != item["sha256"]:
            diffs.append(item["dev"])
    return diffs


# -------------------------------------------------------------------- main

def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--ip", required=True)
    ap.add_argument("--port", required=True, type=int)
    ap.add_argument("--user", required=True)
    ap.add_argument("--key", required=True)
    ap.add_argument("--out-root", required=True)
    ap.add_argument("--baseline", required=True)
    ap.add_argument("--preflight-script", required=True)
    ap.add_argument("--tool-version", default="unknown")
    a = ap.parse_args()

    base = parse_baseline(a.baseline)
    with open(a.preflight_script, "rb") as fh:
        script = fh.read()
    os.makedirs(a.out_root, mode=0o700, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_dir = tempfile.mkdtemp(prefix=f"mpc-one-emmc-{stamp}-", dir=a.out_root)
    manifest = [f"MPFree eMMC backup, tool {a.tool_version}",
                f"started_utc {stamp}", f"expected_state {base['state']}"]
    partials = []

    def note(line):
        manifest.append(line)
        log(line)

    try:
        log(f"Backup directory: {run_dir}")
        log("Step 1/5: read-only preflight on the MPC (layout, write counters, "
            "boot-critical hashes; hashing the 500 MiB rootfs takes a little while)")
        pre = run_preflight(a, script, "hash")
        check_layout(pre, base)
        note(f"mpc_kernel {pre['kernel']}")
        note(f"emmc_model {pre['emmc']}")
        manifest.append(f"ext_csd {pre['ext_csd']}")
        log("EXT_CSD (eMMC boot configuration): " + (
            "saved to the manifest" if pre["ext_csd"] not in (None, "-")
            else "not available (debugfs is not mounted; left as is)"))
        drift = baseline_differences(pre, base)
        if drift:
            note("BASELINE MISMATCH (boot storage changed since the 2026-11-09 boot map): "
                 + ", ".join(drift))
        else:
            note("baseline: p1, p3-p6, boot0, boot1 match the 2026-11-09 boot map")

        disk_bytes = base["disk"]["sectors"] * SECTOR
        hw_bytes = sum(h["sectors"] for h in base["hw"]) * SECTOR
        free = shutil.disk_usage(run_dir).free
        need = disk_bytes + hw_bytes + SPACE_MARGIN
        if free < need:
            raise BackupError(f"not enough space in {a.out_root}: "
                              f"{free >> 20} MiB free, {need >> 20} MiB needed")

        regions = [("gpt-head", 0, GPT_HEAD_SECTORS * SECTOR)]
        regions += [(p["dev"], p["start"] * SECTOR, (p["start"] + p["sectors"]) * SECTOR)
                    for p in base["parts"]]
        results = {}
        jobs = [("mmcblk0", disk_bytes, regions)]
        jobs += [(h["dev"], h["sectors"] * SECTOR, []) for h in base["hw"]]
        for i, (dev, size, regs) in enumerate(jobs, 2):
            path = os.path.join(run_dir, f"{dev}.img.partial")
            partials.append(path)
            log(f"Step {i}/5: copying /dev/{dev} ({size >> 20} MiB, read-only on the MPC)")
            results[dev] = stream(a, dev, path, size, regs)
            r = results[dev]
            log(f"  done in {r['seconds']:.0f} s; remote dd: {r['dd_summary']}")

        log("Step 5/5: read-only write-counter check on the MPC, then verification")
        post = run_preflight(a, script, "stat")
        failures = []
        if post["boot_id"] != pre["boot_id"]:
            failures.append("the MPC rebooted during the backup")
        missing = [d for d in BOOT_CRITICAL + ("mmcblk0p7", "mmcblk0p8")
                   if len(post["stat"].get(d, [])) < 11]
        if missing:
            raise BackupError("no write counters after the copy for: " + ", ".join(missing))
        for dev in BOOT_CRITICAL:
            before, after = counters(pre["stat"][dev]), counters(post["stat"][dev])
            if before != after:
                failures.append(f"{dev} was written or discarded during the backup "
                                f"(counters {before} -> {after})")
        device_hashes = dict(pre["hash"])
        streamed = dict(results["mmcblk0"]["regions"])
        streamed["mmcblk0boot0"] = results["mmcblk0boot0"]["sha256"]
        streamed["mmcblk0boot1"] = results["mmcblk0boot1"]["sha256"]
        for dev in BOOT_CRITICAL + ("gpt-head",):
            if streamed[dev] != device_hashes[dev]:
                failures.append(f"{dev}: copied data does not match the MPC's own hash")
        for dev, r in results.items():
            path = os.path.join(run_dir, f"{dev}.img.partial")
            if rehash_from_disk(path) != r["sha256"]:
                failures.append(f"{dev}: image on the workstation disk does not "
                                "match what was received")
        if failures:
            raise BackupError("verification failed:\n  - " + "\n  - ".join(failures))

        for dev in ("mmcblk0p7", "mmcblk0p8"):
            b, e = counters(pre["stat"][dev]), counters(post["stat"][dev])
            written = e[1] - b[1]
            if b == e:
                note(f"{dev}: no writes during the copy (consistent)")
            else:
                note(f"{dev}: {written} sectors written during the copy; this partition "
                     "is captured like a power-loss snapshot")

        names = {}
        for dev in results:
            final = os.path.join(run_dir, f"{dev}.img")
            os.replace(os.path.join(run_dir, f"{dev}.img.partial"), final)
            names[dev] = final
        partials.clear()
        sums = "".join(f"{results[d]['sha256']}  {d}.img\n" for d in results)
        note("")
        note("layout (512-byte sectors) and SHA-256 of each region of mmcblk0.img:")
        note(f"  gpt-head  start 0 sectors {GPT_HEAD_SECTORS} {streamed['gpt-head']}")
        for p in base["parts"]:
            note(f"  {p['dev']} {p['name']} start {p['start']} sectors {p['sectors']} "
                 f"{streamed[p['dev']]}")
        note("whole images (also in SHA256SUMS):")
        for d in results:
            note(f"  {d}.img {results[d]['sha256']}")
        note(f"finished_utc {datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')}")
        note("status " + ("OK, BASELINE CHANGED" if drift else "OK"))
        with open(os.path.join(run_dir, "SHA256SUMS"), "w") as fh:
            fh.write(sums)
        with open(os.path.join(run_dir, "MANIFEST.txt"), "w") as fh:
            fh.write("\n".join(manifest) + "\n")
        for name in os.listdir(run_dir):
            os.chmod(os.path.join(run_dir, name), 0o400)
        log("")
        log(f"BACKUP VERIFIED: {run_dir}")
        log("Make a second copy on another disk, then check it with: "
            "sha256sum -c SHA256SUMS")
        if drift:
            log("STOP: boot-critical storage differs from the 2026-11-09 boot map "
                f"({', '.join(drift)}). Keep this backup and report it before doing "
                "anything else on the MPC.")
            return EXIT_BASELINE_CHANGED
        return EXIT_OK
    except (BackupError, OSError, subprocess.SubprocessError, KeyboardInterrupt) as exc:
        deleted = 0
        for path in partials:
            try:
                os.unlink(path)
                deleted += 1
            except FileNotFoundError:
                pass
        reason = "interrupted" if isinstance(exc, KeyboardInterrupt) else str(exc)
        manifest.append(f"status FAILED: {reason}")
        with open(os.path.join(run_dir, "MANIFEST-FAILED.txt"), "w") as fh:
            fh.write("\n".join(manifest) + "\n")
        log("")
        log(f"BACKUP FAILED: {reason}")
        if deleted:
            log("Partial images were deleted.")
        log(f"Notes: {run_dir}/MANIFEST-FAILED.txt")
        log("Nothing was written on the MPC.")
        return 130 if isinstance(exc, KeyboardInterrupt) else EXIT_FAIL


def _stop_on_sigterm(signum, frame):
    # `timeout` sends SIGTERM; treat it like Ctrl-C so partial images are deleted.
    raise KeyboardInterrupt


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, _stop_on_sigterm)
    try:
        sys.exit(main())
    except BackupError as exc:  # baseline or argument problems before a run directory exists
        print(f"BACKUP NOT STARTED: {exc}", file=sys.stderr)
        sys.exit(EXIT_FAIL)
