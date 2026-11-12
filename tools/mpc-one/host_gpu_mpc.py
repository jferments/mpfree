#!/usr/bin/env python3
"""mpfree-gpu-bench on the kexec test build: what drawing on the MPC One's GPU costs. RAM ONLY on the MPC.

The performance plan's step G2 ("GPU drawing and compute"). Run it after
`collect-over-ssh.sh --kexec-boot` has started the test build from the USB
stick. Like host_bench_mpc.py it talks only to the test build: it uses the
newest ~/mpfree-work/kexec-stick-* folder's known_hosts, so stock 3.6 (a
different host key) is refused before anything runs. Each step after the
first waits for you to press Enter:

  1. Workstation, read-only: checks that the benchmark
     (rust/mpfree-gpu/target/armv7-unknown-linux-gnueabihf/release/mpfree-gpu-bench)
     is an ARM EABI5 hard-float program for the MPC's C library (loader
     /lib/ld-linux-armhf.so.3, only libc and libgcc_s linked, nothing newer
     than glibc 2.34 asked for; it opens libEGL and libGLESv2 itself when it
     runs). Then it makes the pages it will draw: every sample page of the
     app as display lists, written by the workstation's own build of the app
     (`mpfree --display-lists`, rust/target/release/mpfree) into a new folder
     under ~/mpfree-work, or the folder --pages names.
  2. Test build, read-only: host_app_mpc.py's checks (the test build's
     release, no MMC device, kexec off, / read-only, /run has room and allows
     programs, Akai's app is not running, who holds the sound, MIDI, touch
     and screen devices), then that the GPU's render node and the libraries
     the benchmark needs are there, and who holds /dev/dri.
  3. Test build RAM: copies the benchmark and the pages to /run/mpfree-gpu
     and checks their hashes there. Nothing else is written; a power-cycle
     clears it. For a clean run it then stops our own pad player and log
     display (mpfree-test-app and mpfree-test-screen, RAM state only; Akai's
     services are never touched) and starts whichever was running again at
     the end, also when a run fails or you stop it.
  4. `mpfree-gpu-bench describe`, then `compute --iterations 500` (about a
     minute): the GPU's name and versions, and a GPU compute round trip (an
     FIR filter over a stereo block: upload, run, read back) against the same
     job on one CPU core. It stops here if Mesa answers with its software
     renderer instead of the Mali, since the numbers would not be the GPU's.
  5. `mpfree-gpu-bench screens pages --frames 30` (a few minutes): every page
     drawn on the GPU and in software at the panel's size, timed, and the
     GPU's picture checked against the software one.

The benchmark draws offscreen only, through Mesa's surfaceless EGL on the
GPU's render node, as the offscreen GPU tests of 9 Nov did: it opens no
display, takes no DRM master and changes no display setting; the screen
keeps what it shows. It runs as an ordinary process (no real-time priority)
and writes nothing on the MPC; its output goes into this report.

Nothing here writes to the eMMC (the test build cannot see it), the U-Boot
environment, the SD card or the stick. To go back: switch the MPC off with its
power switch, take out the stick, wait 10 s, switch it on.
"""
import argparse
import datetime
import hashlib
import importlib.util
import os
import re
import signal
import struct
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(os.path.dirname(HERE))
_spec = importlib.util.spec_from_file_location("host_app_mpc", os.path.join(HERE, "host_app_mpc.py"))
app = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(app)
kexec = app.kexec
Stop = kexec.Stop

PROGRAM = os.path.join(REPO_ROOT, "rust", "mpfree-gpu", "target", "armv7-unknown-linux-gnueabihf", "release",
                       "mpfree-gpu-bench")
HOST_APP = os.path.join(REPO_ROOT, "rust", "target", "release", "mpfree")
REMOTE_DIR = "/run/mpfree-gpu"
LOADER = "/lib/ld-linux-armhf.so.3"
# The libraries the benchmark may link; EGL and GLES are opened by the program itself.
LINKED_OK = {"libc.so.6", "libgcc_s.so.1", "libm.so.6", "libdl.so.2", "libpthread.so.0"}
GLIBC_MAX = (2, 34)
PAGE_NAME = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}\.mpdl$")
PAGE_MAX = 4 << 20
PAGES_MAX = 32 << 20
# The only runs this tool asks for: fixed arguments, nothing from the command line. (args, timeout in s)
DESCRIBE = ("describe", 120)
COMPUTE = ("compute --iterations 500", 600)
SCREENS = ("screens pages --frames 30", 1500)
DEFAULT_SSH = {
    "ip": os.environ.get("MPC_IP", "10.42.0.141"),
    "port": int(os.environ.get("MPC_PORT", "2222")),
    "user": os.environ.get("MPC_USER", "root"),
    "key": os.environ.get("MPC_KEY", "/home/jesse/.ssh/mpc_one_autorun_ed25519"),
}

# Every remote script starts with R='' (an empty root prefix). Only the tests' fake ssh changes it.
# Read-only: the GPU's render node, the loader and libraries the benchmark needs, and who holds /dev/dri.
GPU_CHECK = r"""R=''
ok=1
say() { printf '%s\n' "$*"; }
node=''
for f in "$R"/dev/dri/renderD*; do [ -e "$f" ] && node=${f#"$R"} && break; done
if [ -n "$node" ]; then say "render node: $node"; else say "PROBLEM: no /dev/dri/renderD* (the GPU's render node)"; ok=0; fi
if [ -e "$R/lib/ld-linux-armhf.so.3" ]; then say "found: /lib/ld-linux-armhf.so.3"; else say "PROBLEM: missing /lib/ld-linux-armhf.so.3"; ok=0; fi
for n in libc.so.6 libgcc_s.so.1 libEGL.so.1 libGLESv2.so.2; do
  p=''
  for d in /lib /usr/lib; do [ -e "$R$d/$n" ] && { p="$d/$n"; break; }; done
  if [ -n "$p" ]; then say "found: $p"; else say "PROBLEM: missing $n (in /lib or /usr/lib)"; ok=0; fi
done
say "programs holding /dev/dri:"
n=0
for f in "$R"/proc/[0-9]*/fd/*; do
  t=$(readlink "$f" 2>/dev/null) || continue
  case "$t" in
    /dev/dri/*) p=${f#"$R"/proc/}; p=${p%%/*}; say "  $t: pid $p ($(cat "$R/proc/$p/comm" 2>/dev/null))"; n=$((n + 1)) ;;
  esac
done
[ "$n" = 0 ] && say "  none"
[ "$ok" = 1 ] && say GPU-OK || say GPU-STOP
"""
# Writes only into /run/mpfree-gpu (RAM).
PREPARE = r"""R=''
umask 077
d="$R/run/mpfree-gpu"
[ -d "$d" ] || mkdir "$d" || exit 1
[ -d "$d/pages" ] || mkdir "$d/pages" || exit 1
rm -f "$d/mpfree-gpu-bench" "$d/mpfree-gpu-bench.part" "$d/pages/"*
"""
RECEIVE = r"""R=''; umask 077; cat > "$R/run/mpfree-gpu/@@NAME@@.part" && chmod @@MODE@@ "$R/run/mpfree-gpu/@@NAME@@.part" && mv "$R/run/mpfree-gpu/@@NAME@@.part" "$R/run/mpfree-gpu/@@NAME@@" """
HASHES = r"""R=''
cd "$R/run/mpfree-gpu" || exit 1
sha256sum mpfree-gpu-bench pages/*
"""
RUN = r"""R=''
cd "$R/run/mpfree-gpu" || exit 1
exec "$R/run/mpfree-gpu/mpfree-gpu-bench" @@ARGS@@
"""


def arm_dynamic_problem(data):
    """None if `data` is a dynamically linked 32-bit ARM EABI5 hard-float program the MPC's C library can
    run (its loader, only LINKED_OK libraries, nothing past GLIBC_MAX), else what is wrong."""
    if len(data) > app.PROGRAM_MAX or len(data) < 52 or data[:7] != b"\x7fELF\x01\x01\x01":
        return "not a 32-bit little-endian ELF file of a plausible size"
    e_type, e_machine = struct.unpack_from("<HH", data, 16)
    phoff, = struct.unpack_from("<I", data, 28)
    flags, = struct.unpack_from("<I", data, 36)
    phentsize, phnum = struct.unpack_from("<HH", data, 42)
    if e_type not in (2, 3) or e_machine != 40:
        return "not an ARM executable"
    if flags & 0xFF000000 != 0x05000000 or not flags & 0x400:
        return "not EABI5 hard-float"
    if phnum == 0 or phentsize < 32 or phoff + phnum * phentsize > len(data):
        return "missing program headers, or they run past the end"
    heads = [struct.unpack_from("<IIIIII", data, phoff + i * phentsize) for i in range(phnum)]
    interp = [h for h in heads if h[0] == 3]
    dynamic = [h for h in heads if h[0] == 2]
    if not interp or not dynamic:
        return "statically linked (this benchmark must load the MPC's EGL, so it is built dynamic)"
    _, off, _, _, size, _ = interp[0]
    if off + size > len(data) or data[off:off + size].rstrip(b"\0").decode("latin-1") != LOADER:
        return f"not for the loader {LOADER}"

    def offset(vaddr):
        for kind, off, va, _, filesz, _ in heads:
            if kind == 1 and va <= vaddr < va + filesz:
                return off + vaddr - va
        return None

    _, doff, _, _, dsize, _ = dynamic[0]
    entries = [struct.unpack_from("<iI", data, doff + i) for i in range(0, min(dsize, len(data) - doff) - 7, 8)]
    strtab = next((offset(v) for t, v in entries if t == 5), None)
    if strtab is None:
        return "no string table in its dynamic section"

    def string(at):
        end = data.find(b"\0", strtab + at)
        return data[strtab + at:end].decode("latin-1") if end >= 0 else ""

    needed = [string(v) for t, v in entries if t == 1]
    extra = sorted(set(needed) - LINKED_OK)
    if extra:
        return f"linked against {', '.join(extra)} (only {', '.join(sorted(LINKED_OK))} are allowed)"
    versions = [tuple(int(x) for x in m) for m in re.findall(rb"GLIBC_(\d+)\.(\d+)", data)]
    if versions and max(versions) > GLIBC_MAX:
        v = max(versions)
        return f"asks for GLIBC_{v[0]}.{v[1]}, newer than {GLIBC_MAX[0]}.{GLIBC_MAX[1]}"
    return None


def check_program(out, path):
    """Step 1: the benchmark's size, hash and ELF kind. Returns (path, sha256)."""
    path = os.path.realpath(path)
    if not os.path.isfile(path):
        raise Stop(f"missing {path}: build it first (see rust/mpfree-gpu/README.md)")
    with open(path, "rb") as fh:
        data = fh.read(app.PROGRAM_MAX + 1)
    problem = arm_dynamic_problem(data)
    if problem:
        raise Stop(f"{path} is {problem}; rebuild it as rust/mpfree-gpu/README.md shows")
    digest = hashlib.sha256(data).hexdigest()
    out.say(f"  benchmark: {path}")
    out.say(f"    {len(data)} bytes, sha256 {digest}")
    out.say(f"    ARM EABI5 hard-float, loader {LOADER}, links only the C library, glibc <= 2.34")
    return path, digest


def make_pages(out, work, host_app):
    """The pages as display lists, written now by the workstation's build of the app into a new folder."""
    host_app = os.path.realpath(host_app)
    if not os.path.isfile(host_app):
        raise Stop(f"missing {host_app}: build the app for this workstation first "
                   "(cargo build --release --manifest-path rust/Cargo.toml -p mpfree-app)")
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    folder = os.path.join(work, f"gpu-pages-{stamp}")
    out.say(f"  making the pages: {host_app} --display-lists {folder}")
    try:
        res = subprocess.run([host_app, "--display-lists", folder], capture_output=True, text=True, timeout=900)
    except subprocess.TimeoutExpired:
        raise Stop("the app took over 15 minutes to write the pages")
    if res.returncode != 0:
        tail = (res.stdout + res.stderr).strip().splitlines()[-3:]
        raise Stop(f"the app could not write the pages ({' / '.join(tail)})")
    return folder


def check_pages(out, folder):
    """The .mpdl files to copy: names, sizes and hashes. Returns [(name, path, sha256)]."""
    folder = os.path.realpath(folder)
    if not os.path.isdir(folder):
        raise Stop(f"no folder {folder}")
    names = sorted(n for n in os.listdir(folder) if n.endswith(".mpdl"))
    if not names:
        raise Stop(f"no .mpdl files in {folder}")
    pages, total = [], 0
    for name in names:
        path = os.path.join(folder, name)
        size = os.path.getsize(path)
        if not PAGE_NAME.match(name) or not os.path.isfile(path) or os.path.islink(path):
            raise Stop(f"{path}: not a page file this tool copies")
        if size > PAGE_MAX:
            raise Stop(f"{path} is {size} bytes, over {PAGE_MAX}")
        with open(path, "rb") as fh:
            data = fh.read()
        if data[:4] != b"MPDL":
            raise Stop(f"{path} is not a display list")
        total += size
        pages.append((name, path, hashlib.sha256(data).hexdigest()))
    if total > PAGES_MAX:
        raise Stop(f"the pages are {total} bytes in all, over {PAGES_MAX}")
    out.say(f"  pages: {len(pages)} in {folder}, {total // 1024} KiB in all")
    return pages


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--ip", default=DEFAULT_SSH["ip"])
    p.add_argument("--port", type=int, default=DEFAULT_SSH["port"])
    p.add_argument("--user", default=DEFAULT_SSH["user"])
    p.add_argument("--key", default=DEFAULT_SSH["key"])
    p.add_argument("--work", default=os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    p.add_argument("--program", default=PROGRAM)
    p.add_argument("--host-app", default=HOST_APP)
    p.add_argument("--pages", help="a folder of .mpdl pages already written by mpfree --display-lists")
    p.add_argument("--report")
    args = p.parse_args(argv)
    if os.environ.get("MPFREE_ACCEPT_GPU_BENCH") != "YES":
        raise Stop("opt-in required (copies the GPU benchmark into the test build's RAM and runs it): "
                   "MPFREE_ACCEPT_GPU_BENCH=YES python3 tools/mpc-one/host_gpu_mpc.py")
    if not app.TESTING and not sys.stdin.isatty():
        raise Stop("run this from a terminal: each step waits for you to press Enter")
    out = kexec.Out(args.report)
    out.say("MPFree GPU benchmark on the test build (changes RAM on the MPC only; draws offscreen)")
    total = 5

    out.say(f"\n[1/{total}] Workstation checks (read-only on the MPC)")
    program, digest = check_program(out, args.program)
    work = os.path.realpath(args.work)
    folder = kexec.newest_build(work)
    out.say(f"  test build: {folder}")
    with open(os.path.join(folder, "report.txt")) as fh:
        found = re.search(r"^its kernel release: (\S+)$", fh.read(), re.M)
    if not found or not kexec.RELEASE.match(found.group(1)):
        raise Stop(f"{folder}/report.txt does not name the test build's kernel release")
    pages = check_pages(out, args.pages or make_pages(out, work, args.host_app))
    mpc = kexec.Mpc(args, os.path.join(folder, "known_hosts"))

    out.say(f"\n[2/{total}] Checks on the test build (read-only; only its own host key is accepted)")
    rc, text = mpc.run(kexec.fill(app.CHECK, NEW=found.group(1)), test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc == 255 and "CHECKS-" not in text:
        raise Stop("the test build did not answer. Start it first with the --kexec-boot block; "
                   "on stock 3.6 this tool runs nothing")
    if rc != 0 or text.strip().splitlines()[-1:] != ["CHECKS-OK"]:
        raise Stop("the test build is not ready; nothing was changed on it")
    rc, text = mpc.run(GPU_CHECK, test=True)
    for line in text.splitlines():
        out.say(f"  {line}")
    if rc != 0 or text.strip().splitlines()[-1:] != ["GPU-OK"]:
        raise Stop("the GPU or its libraries are not there on this build; nothing was changed on it")

    signal.signal(signal.SIGINT, signal.default_int_handler)
    stopped = []
    try:
        out.say(f"\n[3/{total}] Copy the benchmark and the pages into the test build's RAM ({REMOTE_DIR})")
        if app.ready(out, f"Copies one program of about 1 MB and {len(pages)} page files.") != "go":
            raise Stop("stopped by you")
        rc, text = mpc.run(PREPARE, test=True)
        if rc != 0:
            raise Stop(f"could not make {REMOTE_DIR} ({text.strip()})")
        want = {"mpfree-gpu-bench": digest}
        for name, path, mode in [("mpfree-gpu-bench", program, "700")] + [
                (f"pages/{n}", pth, "600") for n, pth, _ in pages]:
            rc, text = mpc.run(None, test=True, stdin_path=path,
                               command=kexec.fill(RECEIVE, NAME=name, MODE=mode))
            if rc != 0:
                raise Stop(f"copying {name} failed ({text.strip()})")
        want.update({f"pages/{n}": d for n, _, d in pages})
        rc, text = mpc.run(HASHES, test=True)
        got = {}
        for line in text.splitlines():
            parts = line.split()
            if len(parts) == 2:
                got[parts[1]] = parts[0]
        if rc != 0 or got != want:
            bad = sorted(k for k in set(want) | set(got) if want.get(k) != got.get(k))
            raise Stop(f"the copies on the MPC do not match ({', '.join(bad[:5]) or text.strip()})")
        out.say(f"  copied the benchmark and {len(pages)} pages; every hash on the MPC matches")

        # A clean run: our own pad player and log display are stopped, and started again at the end.
        rc, text = mpc.run(app.SCREEN_STOP, test=True)
        if "pad player: was active" in text:
            stopped.append(app.APP_START)
        if "log display: was active" in text:
            stopped.append(app.SCREEN_START)
        for line in text.strip().splitlines():
            out.say(f"  {line}")
        out.say("  (for a clean run; whichever was running starts again at the end)")

        out.say(f"\n[4/{total}] The GPU's name, then a compute round trip against one CPU core")
        answer = app.ready(out, "Takes about a minute; the screen keeps what it shows.")
        if answer == "quit":
            raise Stop("stopped by you")
        if answer == "go":
            run_args, timeout = DESCRIBE
            rc, text = app.stream(mpc, kexec.fill(RUN, ARGS=run_args), timeout, out)
            if rc != 0:
                raise Stop(f"describe ended with exit code {rc}")
            renderer = next((ln for ln in text.splitlines() if ln.startswith("gpu: ")), "")
            if "panfrost" not in renderer.lower():
                raise Stop("Mesa answered without the Mali (Panfrost): the numbers would not be the GPU's")
            run_args, timeout = COMPUTE
            rc, _ = app.stream(mpc, kexec.fill(RUN, ARGS=run_args), timeout, out)
            if rc != 0:
                raise Stop(f"compute ended with exit code {rc}")

        out.say(f"\n[5/{total}] Every page on the GPU and in software, timed and compared")
        answer = app.ready(out, "Takes a few minutes; the screen keeps what it shows.")
        if answer == "quit":
            raise Stop("stopped by you")
        if answer == "go":
            run_args, timeout = SCREENS
            rc, _ = app.stream(mpc, kexec.fill(RUN, ARGS=run_args), timeout, out)
            if rc != 0:
                raise Stop(f"screens ended with exit code {rc}")
        result = 0
    except (Stop, KeyboardInterrupt) as err:
        out.say(f"\nSTOP: {str(err) or 'interrupted (Ctrl-C)'}")
        result = 1
    for script in stopped:
        rc, text = mpc.run(script, test=True)
        out.say(f"  {text.strip() or f'no answer, exit code {rc}'} (started again)")
    out.say("\nRESULT: " + ("the GPU benchmark ran (numbers above)." if result == 0 else "stopped early."))
    out.say("NEXT: carry on with the round, or switch the MPC off with its power switch, take out the stick,")
    out.say("wait 10 s, switch it on, and run the boot-map block to confirm nothing on the eMMC changed.")
    return result


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
    except KeyboardInterrupt:  # only reaches here in steps 1 and 2, before anything is copied
        print("\nSTOP: interrupted (Ctrl-C). Nothing was copied to the MPC.")
        sys.exit(130)
