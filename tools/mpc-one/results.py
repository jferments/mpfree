#!/usr/bin/env python3
"""WORKSTATION ONLY: keep the output of every test run in one folder, and pack
that folder into one zip to upload.

  python3 tools/mpc-one/results.py run NAME -- COMMAND [ARGS...]
      Runs COMMAND in this terminal. You can still type answers (such as WRITE)
      and your sudo password, which is not echoed and so not recorded. Everything
      COMMAND prints is saved to ~/mpfree-results/current/NN-NAME.log, which ends
      with its exit code. Any report COMMAND writes is copied next to the log:
      a report.txt in a folder under ~/mpfree-work, or a report in research/ in
      this clone (where collect-over-ssh.sh writes them). Exits with COMMAND's
      exit code.

  python3 tools/mpc-one/results.py zip
      Packs ~/mpfree-results/current/ into ~/mpfree-results/mpfree-results-<UTC>.zip,
      with MANIFEST.txt (this clone's commit, the computer's Python and kernel,
      and a SHA-256 of every file). Then it renames current/ to <UTC>/, so the
      next run starts a new folder. Upload that one zip.

  python3 tools/mpc-one/results.py status
      Lists what is in ~/mpfree-results/current/.

It never contacts the MPC and never copies anything but logs and text reports:
not stick images, device trees, keys or backups. The reports can quote Akai's
files, so treat the zip like the work folders: upload it to this project only,
never publicly.

MPFREE_RESULTS_DIR, MPFREE_WORK_DIR and MPC_REPORT_DIR change the folders
(tests use them). The results folder must be outside this clone.
"""
import datetime
import glob
import hashlib
import os
import platform
import pty
import re
import shlex
import shutil
import subprocess
import sys
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
NAME = re.compile(r"^[a-z0-9][a-z0-9-]{0,40}$")
MAX_REPORT = 64 << 20


class Stop(Exception):
    pass


def results_root():
    root = os.path.realpath(os.environ.get("MPFREE_RESULTS_DIR", os.path.expanduser("~/mpfree-results")))
    if root == REPO or root.startswith(REPO + os.sep):
        raise Stop(f"the results folder {root} is inside this clone; use a folder outside it")
    return root


def work_root():
    return os.path.realpath(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))


def report_dir():
    return os.path.realpath(os.environ.get("MPC_REPORT_DIR", os.path.join(REPO, "research")))


def utc(fmt="%Y%m%dT%H%M%SZ"):
    return datetime.datetime.now(datetime.timezone.utc).strftime(fmt)


def git(*args):
    try:
        res = subprocess.run(["git", "-C", REPO, *args], capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return ""
    return res.stdout.strip() if res.returncode == 0 else ""


def report_candidates():
    """{path: (mtime_ns, size)} of every report a tool may write."""
    found = {}
    for pattern in (os.path.join(work_root(), "*", "report.txt"), os.path.join(report_dir(), "*.txt")):
        for path in glob.glob(pattern):
            try:
                st = os.stat(path)
            except OSError:
                continue
            found[path] = (st.st_mtime_ns, st.st_size)
    return found


def is_text(path):
    with open(path, "rb") as fh:
        while True:
            chunk = fh.read(1 << 20)
            if not chunk:
                return True
            if b"\x00" in chunk:
                return False


def exit_code(status):
    if hasattr(os, "waitstatus_to_exitcode"):
        return os.waitstatus_to_exitcode(status)
    return os.WEXITSTATUS(status) if os.WIFEXITED(status) else -os.WTERMSIG(status)


def run(name, cmd):
    if not NAME.match(name):
        raise Stop(f"step name {name!r}: use lowercase letters, digits and dashes")
    if not cmd:
        raise Stop("nothing to run: put the command after --")
    if not shutil.which(cmd[0]):
        raise Stop(f"{cmd[0]}: command not found")
    current = os.path.join(results_root(), "current")
    os.makedirs(current, mode=0o700, exist_ok=True)
    number = 1 + len([f for f in os.listdir(current) if f.endswith(".log")])
    prefix = f"{number:02d}-{name}"
    log_path = os.path.join(current, prefix + ".log")
    before = report_candidates()
    head = git("log", "-1", "--format=%h %s")
    branch = git("rev-parse", "--abbrev-ref", "HEAD")
    with open(log_path, "wb") as log:
        log.write((f"# mpfree results: {name}\n# command: {shlex.join(cmd)}\n# folder: {os.getcwd()}\n"
                   f"# clone: {head or '?'} (branch {branch or '?'})\n# started: {utc('%Y-%m-%d %H:%M:%S UTC')}\n"
                   ).encode())
        log.flush()
        eof_sent = []

        def from_child(fd):
            data = os.read(fd, 65536)
            log.write(data)
            # At once: a terminal closed mid-run used to lose the last 8 KB
            # (live test 3).
            log.flush()
            return data

        def from_user(fd):
            data = os.read(fd, 1024)
            if not data and not eof_sent:  # no terminal: pass the end of input on as Ctrl-D
                eof_sent.append(1)
                return b"\x04"
            return data

        os.environ["PAGER"] = os.environ["GIT_PAGER"] = "cat"  # a pager would wait for keys
        status = pty.spawn(cmd, from_child, from_user)
        rc = exit_code(status)
        log.write(f"\n# finished: {utc('%Y-%m-%d %H:%M:%S UTC')}, exit code {rc}\n".encode())
    with open(log_path, "rb") as fh:
        text = fh.read().replace(b"\r\n", b"\n")
    with open(log_path, "wb") as fh:
        fh.write(text)
    copied = []
    for path, sig in sorted(report_candidates().items()):
        if before.get(path) == sig:
            continue
        if os.path.getsize(path) > MAX_REPORT or not is_text(path):
            print(f"results: not copied (not a text report): {path}")
            continue
        label = os.path.basename(os.path.dirname(path)) + "-report.txt" if path.endswith("/report.txt") \
            else os.path.basename(path)
        dest = os.path.join(current, f"{prefix}--{label}")
        shutil.copyfile(path, dest)
        copied.append(dest)
    print(f"\nresults: exit code {rc}; saved {log_path}")
    for dest in copied:
        print(f"results: report copied to {dest}")
    print("results: when you're done, run: python3 tools/mpc-one/results.py zip")
    return rc


def manifest(current, files):
    changed = len(git("status", "--porcelain", "--untracked-files=no").splitlines())
    lines = [f"mpfree results packed {utc('%Y-%m-%d %H:%M:%S UTC')}",
             f"clone: {git('log', '-1', '--format=%H %s') or '?'}",
             f"branch: {git('rev-parse', '--abbrev-ref', 'HEAD') or '?'}",
             f"changed tracked files in the clone: {changed}",
             f"python: {platform.python_version()}",
             f"kernel: {platform.system()} {platform.release()} {platform.machine()}",
             "", "sha256 of each file:"]
    for name in files:
        with open(os.path.join(current, name), "rb") as fh:
            lines.append(f"{hashlib.sha256(fh.read()).hexdigest()}  {name}")
    return "\n".join(lines) + "\n"


def pack():
    root = results_root()
    current = os.path.join(root, "current")
    files = sorted(f for f in os.listdir(current) if f != "MANIFEST.txt") if os.path.isdir(current) else []
    if not files:
        raise Stop(f"nothing to pack: {current} is empty. Run steps with: results.py run NAME -- COMMAND")
    stamp = utc()
    zip_path = os.path.join(root, f"mpfree-results-{stamp}.zip")
    if os.path.exists(zip_path) or os.path.exists(os.path.join(root, stamp)):
        raise Stop(f"{zip_path} already exists; wait a second and run zip again")
    with open(os.path.join(current, "MANIFEST.txt"), "w") as fh:
        fh.write(manifest(current, files))
    tmp = zip_path + ".part"
    with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as zf:
        for name in ["MANIFEST.txt"] + files:
            zf.write(os.path.join(current, name), f"mpfree-results-{stamp}/{name}")
    with zipfile.ZipFile(tmp) as zf:
        bad = zf.testzip()
        if bad:
            raise Stop(f"the zip failed its own check at {bad}; nothing was moved")
    os.replace(tmp, zip_path)
    os.rename(current, os.path.join(root, stamp))
    print(f"packed {len(files)} file(s):")
    for name in files:
        print(f"  {name}")
    print(f"\nUPLOAD THIS ONE FILE: {zip_path}")
    print(f"(the files stay in {os.path.join(root, stamp)}; the next run starts a new current/ folder)")
    return 0


def status():
    current = os.path.join(results_root(), "current")
    files = sorted(os.listdir(current)) if os.path.isdir(current) else []
    print(f"{current}: {len(files)} file(s)")
    for name in files:
        print(f"  {name}")
    return 0


def main(argv):
    if argv[:1] == ["run"] and len(argv) >= 3 and argv[2] == "--":
        return run(argv[1], argv[3:])
    if argv == ["zip"]:
        return pack()
    if argv == ["status"]:
        return status()
    print(__doc__.strip())
    return 2


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
