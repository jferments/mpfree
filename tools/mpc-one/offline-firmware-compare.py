#!/usr/bin/env python3
"""OFFLINE, workstation only: compare the peripheral firmware files of two
firmware versions, from two offline-rootfs-survey.sh reports.

Akai's updaters and app can flash chips outside the eMMC: the touch
controller, the pad and button controller, XMOS audio parts. A kexec test
boot hides the eMMC, but not those chips. If a newer version ships the same
firmware files as the version the MPC runs, its updaters have nothing new
to write. This tool checks that, file by file, from section 10 of two
survey reports ("fw <sha256> <path>" lines).

  python3 tools/mpc-one/offline-firmware-compare.py [OLD_REPORT NEW_REPORT]

With no arguments it takes, under $MPFREE_WORK_DIR (default ~/mpfree-work):
  OLD  the newest rootfs-survey-*/report.txt made with --from-backup
       (the rootfs the MPC runs now, cut from the verified eMMC backup)
  NEW  the newest rootfs-survey-*/report.txt made from an image
Both must come from a survey that has section 10; run the survey again if not.

It prints every file as same, DIFFERENT, NEW (only in the new version) or
gone (only in the old one), then one verdict line:
  VERDICT: SAME ...   exit 0: no firmware file is new or different
  VERDICT: STOP ...   exit 1: something differs, is new, or could not be
                      compared; do not let the new version's app or
                      updaters run against the hardware until it is understood
Exit 2 means the reports could not be read.

It only reads the two reports. It never contacts the MPC and writes nothing.
SAME is about files only: firmware built into a program (not a separate file)
is not covered, so the stick keeps the flashing programs blocked either way.
"""
import argparse
import glob
import os
import re
import sys

SECTION = "=== 10. Peripheral firmware files"
FW_LINE = re.compile(r"^fw ([0-9a-f]{64}) (/\S+)$")
SOURCE = re.compile(r"^source: (.*)$")


class ReportError(Exception):
    pass


def read_report(path):
    """Returns (source line, version text, {path: sha}, [lines that could not be compared])."""
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            lines = fh.read().splitlines()
    except OSError as e:
        raise ReportError(f"cannot read {path}: {e.strerror}")
    source = next((m.group(1) for m in map(SOURCE.match, lines) if m), None)
    if source is None:
        raise ReportError(f"{path} is not an offline-rootfs-survey.sh report (no source line)")
    starts = [i for i, line in enumerate(lines) if line.startswith(SECTION)]
    if len(starts) != 1:
        raise ReportError(f"{path} has no section 10 (made before it existed?): run the survey again")
    version = ""
    for i, line in enumerate(lines):
        if line == "--- /etc/version" and i + 1 < len(lines):
            version = lines[i + 1].strip()
            break
    files, odd = {}, []
    for line in lines[starts[0] + 1:]:
        if line.startswith("=== ") or line.startswith("DONE."):
            break
        if not line.strip():
            continue
        m = FW_LINE.match(line)
        if m:
            if m.group(2) in files:
                raise ReportError(f"{path} lists {m.group(2)} twice")
            files[m.group(2)] = m.group(1)
        elif line.startswith("fw-link "):
            continue  # a link names a file that is listed itself, or one outside these folders
        else:
            odd.append(line)
    return source, version, files, odd


def newest(work, from_backup):
    found = []
    for report in sorted(glob.glob(os.path.join(work, "rootfs-survey-*", "report.txt"))):
        try:
            with open(report, encoding="utf-8", errors="replace") as fh:
                head = fh.read(4096)
        except OSError:
            continue
        m = re.search(r"^source: (.*)$", head, re.M)
        if m and m.group(1).startswith("p6 of ") == from_backup:
            found.append(report)
    if not found:
        kind = "--from-backup" if from_backup else "--image"
        raise ReportError(f"no rootfs-survey-*/report.txt made with {kind} under {work}: run the survey first")
    return found[-1]  # folder names start with the UTC time, so the last sorts newest


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("reports", nargs="*", metavar="REPORT", help="OLD_REPORT NEW_REPORT (default: found as above)")
    args = ap.parse_args(argv)
    if len(args.reports) not in (0, 2):
        ap.error("give both reports or neither")
    try:
        if args.reports:
            old_path, new_path = args.reports
        else:
            work = os.path.realpath(os.environ.get("MPFREE_WORK_DIR") or os.path.expanduser("~/mpfree-work"))
            old_path, new_path = newest(work, True), newest(work, False)
        old = read_report(old_path)
        new = read_report(new_path)
    except ReportError as e:
        print(f"ERROR: {e}")
        return 2
    print("MPFree offline firmware comparison (reads two survey reports; no MPC contact)")
    for label, path, (source, version, files, _) in (("old", old_path, old), ("new", new_path, new)):
        print(f"{label}: {path}")
        print(f"     source {source}; version {version or '(no /etc/version)'}; {len(files)} firmware files")
    old_files, new_files = old[2], new[2]
    counts = {"same": 0, "DIFFERENT": 0, "NEW": 0, "gone": 0}
    print()
    for path in sorted(set(old_files) | set(new_files)):
        if path not in new_files:
            state = "gone"
        elif path not in old_files:
            state = "NEW"
        elif old_files[path] == new_files[path]:
            state = "same"
        else:
            state = "DIFFERENT"
        counts[state] += 1
        print(f"{state:9} {path}")
    for label, (_, _, _, odd) in (("old", old), ("new", new)):
        for line in odd:
            print(f"not compared ({label}): {line}")
    not_compared = len(new[3])
    print()
    print(f"{counts['same']} same, {counts['DIFFERENT']} different, {counts['NEW']} new, {counts['gone']} gone, "
          f"{not_compared} in the new version not compared")
    if not new_files:
        print("VERDICT: STOP. The new report lists no firmware files at all, which is not plausible for an MPC image.")
        return 1
    if counts["DIFFERENT"] or counts["NEW"] or not_compared:
        print("VERDICT: STOP. The new version ships firmware the MPC's chips may not have. Its app and updaters "
              "must not run against the hardware until each file above is understood.")
        return 1
    print("VERDICT: SAME. Every firmware file in the new version matches the old one, so its updaters have "
          "nothing new to write from these files.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
