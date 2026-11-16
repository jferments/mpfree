#!/usr/bin/env python3
"""stage.py: make the FXProbe kit, the folder the MPFree test stick carries so
FXProbe can be run on the MPC under Akai's MPC OS (developer mode).

WORKSTATION ONLY. It never contacts the MPC and writes only the new kit folder
(and the plugins' own build folders, as the SDK's build does).

  python3 tools/fxprobe/stage.py --sdk ~/code/mpc-plugin-builder-sdk --out DIR

  DIR/                                  (must not exist yet)
    fxprobe.sh                          the on-device tool: sh DIR/fxprobe.sh check|install|...
    README.txt                          what the kit is and the order of the steps
    VERSION                             the MPFree commit, the SDK commit, when it was made
    SHA256SUMS                          every other file, checked by fxprobe.sh before it installs
    plugins/MPFree - VST - FXProbe Source/   plugin.json, version.xml, VST/*-arm32.so, VST/*-arm64.so, Plugin Skins/
    plugins/MPFree - VST - FXProbe Tap/
    programmes/short.wav, short.tsv     fxlab's short programme (13 s) and its schedule
    programmes/standard.wav, standard.tsv   the standard programme (about 50 s)

The plugin folders sit in plugins/, not "VST2 Plugins/": Akai's system mounts
a stick's partitions noexec, so fxprobe.sh install copies them to the MPC's
internal drive instead.

Steps:
  1. builds both plugins with the SDK's tools/build.py (arm32, arm64, its
     validator), at the SDK commit FXProbe is tested at (febd2ca5); the SDK
     fetches its Zig compiler the first time. --plugins-from DIR uses two
     plugin folders already built instead (DIR holds both);
  2. checks each .so is a shared library for the right ARM (32-bit EM_ARM,
     64-bit EM_AARCH64);
  3. renders the programmes with fxlab (rust/target/release/fxlab, or --fxlab;
     build it with: cargo build --release -p mpfree-fxlab, in rust/);
  4. writes VERSION and SHA256SUMS and lists every file with its size.

--native-test (desktop tests only) skips the ARM check, for a kit holding
native builds.
"""

import argparse
import datetime
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
SDK_COMMIT = "febd2ca5"
PLUGINS = [("FXProbeSource", "MPFree - VST - FXProbe Source", "fxprobe-source"),
           ("FXProbeTap", "MPFree - VST - FXProbe Tap", "fxprobe-tap")]
EM_ARM, EM_AARCH64 = 40, 183

README = """FXProbe kit (MPFree developer tools, run under Akai's MPC OS in developer mode)

Two plugins made with Akai's plugin SDK, the fxlab test programmes and
fxprobe.sh, which runs on the MPC from this folder:

  sh <this folder>/fxprobe.sh check     read-only: what is where, and what reports
  sh <this folder>/fxprobe.sh install   copies the plugins to the MPC's internal drive (undo: remove)
  sh <this folder>/fxprobe.sh setup     makes FXProbe/ on this stick's data partition (undo: clean)
  sh <this folder>/fxprobe.sh status
  sh <this folder>/fxprobe.sh capture NAME --gains 0,-12
  sh <this folder>/fxprobe.sh remove    deletes the two plugin folders from the internal drive
  sh <this folder>/fxprobe.sh clean     deletes FXProbe/ (captures included) from the data partition

Order: install with the stick in once Akai's app is up; switch off, take the
stick out, switch on (the app lists the plugins at start); put the stick in
once the app is up; setup; put FXProbe Source on a plugin track, the effect
under test in insert 1 and FXProbe Tap (Output: Mute) in insert 2; status;
capture. Captures are in FXProbe/captures/ on the data partition: read them on
a workstation and measure them with fxlab analyse (schedules in programmes/).

Captures of Akai's effects are Akai's output: keep them out of the MPFree
repository. Design and safety rules: docs/dev/design/fxlab.md in the MPFree
repository.
"""


class Stop(Exception):
    pass


def run(cmd, cwd=None):
    r = subprocess.run(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if r.returncode != 0:
        raise Stop("%s failed:\n%s" % (" ".join(cmd), r.stdout[-4000:]))
    return r.stdout


def git_head(path):
    r = subprocess.run(["git", "-C", path, "rev-parse", "HEAD"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                       text=True)
    return r.stdout.strip() if r.returncode == 0 else ""


def git_dirty(path):
    r = subprocess.run(["git", "-C", path, "status", "--porcelain", "--untracked-files=no"], stdout=subprocess.PIPE,
                       stderr=subprocess.DEVNULL, text=True)
    return r.returncode == 0 and bool(r.stdout.strip())


def elf_machine(path):
    """(class, machine) of an ELF shared library, or None."""
    with open(path, "rb") as fh:
        head = fh.read(20)
    if len(head) < 20 or head[:4] != b"\x7fELF" or head[5] != 1:
        return None
    etype, machine = struct.unpack("<HH", head[16:20])
    return (head[4], machine) if etype == 3 else None


def build_plugins(sdk, out_plugins):
    head = git_head(sdk)
    if not head.startswith(SDK_COMMIT):
        raise Stop("the SDK at %s is at %s, not %s, the commit FXProbe is tested at "
                   "(git -C %s checkout %s)" % (sdk, head or "an unknown commit", SDK_COMMIT, sdk, SDK_COMMIT))
    build = os.path.join(sdk, "tools", "build.py")
    for project, folder, _ in PLUGINS:
        src = os.path.join(HERE, project)
        print("building %s with the SDK (arm32, arm64, validator)..." % project, flush=True)
        run([sys.executable, build, src], cwd=sdk)
        built = os.path.join(src, "build", folder)
        if not os.path.isdir(built):
            raise Stop("the SDK build left no %s" % built)
        shutil.copytree(built, os.path.join(out_plugins, folder))
    return head


def copy_plugins(src_dir, out_plugins):
    for _, folder, _ in PLUGINS:
        src = os.path.join(src_dir, folder)
        if not os.path.isdir(src):
            raise Stop("--plugins-from %s has no %s" % (src_dir, folder))
        shutil.copytree(src, os.path.join(out_plugins, folder))


def check_plugins(out_plugins, native):
    for _, folder, slug in PLUGINS:
        d = os.path.join(out_plugins, folder)
        if not os.path.isfile(os.path.join(d, "plugin.json")):
            raise Stop("%s has no plugin.json" % d)
        if os.path.exists(os.path.join(d, "workfolder.txt")):
            raise Stop("%s holds a workfolder.txt; fxprobe.sh install writes that on the MPC" % d)
        if native:
            continue
        for bits, want in ((32, (1, EM_ARM)), (64, (2, EM_AARCH64))):
            so = os.path.join(d, "VST", "%s-arm%d.so" % (slug, bits))
            if not os.path.isfile(so):
                raise Stop("%s is missing" % so)
            if elf_machine(so) != want:
                raise Stop("%s is not a %d-bit ARM shared library" % (so, bits))


def render_programmes(fxlab, out_progs):
    if not os.path.isfile(fxlab):
        raise Stop("no fxlab at %s (in rust/: cargo build --release -p mpfree-fxlab)" % fxlab)
    tmp = tempfile.mkdtemp(prefix="fxprobe-stage-")
    try:
        run([fxlab, "programme", "--short", "--out", tmp])
        run([fxlab, "programme", "--out", tmp])
        for src, dst in (("short.wav", "short.wav"), ("short.tsv", "short.tsv"),
                         ("programme.wav", "standard.wav"), ("schedule.tsv", "standard.tsv")):
            shutil.copyfile(os.path.join(tmp, src), os.path.join(out_progs, dst))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def files_under(root):
    out = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        for name in sorted(filenames):
            out.append(os.path.relpath(os.path.join(dirpath, name), root).replace(os.sep, "/"))
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, help="the kit folder to make (must not exist)")
    ap.add_argument("--sdk", help="a checkout of inmusic-software/mpc-plugin-builder-sdk at %s" % SDK_COMMIT)
    ap.add_argument("--plugins-from", help="a folder holding both plugin folders, already built")
    ap.add_argument("--fxlab", default=os.path.join(REPO, "rust", "target", "release", "fxlab"))
    ap.add_argument("--native-test", action="store_true", help=argparse.SUPPRESS)
    args = ap.parse_args(argv)
    if bool(args.sdk) == bool(args.plugins_from):
        ap.error("give exactly one of --sdk DIR or --plugins-from DIR")
    out = os.path.abspath(args.out)
    try:
        if os.path.lexists(out):
            raise Stop("%s exists; give a new folder" % out)
        parent = os.path.dirname(out)
        if not os.path.isdir(parent):
            raise Stop("%s does not exist" % parent)
        tmp = tempfile.mkdtemp(prefix=".fxprobe-kit-", dir=parent)
        try:
            plugins = os.path.join(tmp, "plugins")
            progs = os.path.join(tmp, "programmes")
            os.makedirs(plugins)
            os.makedirs(progs)
            if args.sdk:
                sdk_head = build_plugins(os.path.abspath(args.sdk), plugins)
            else:
                copy_plugins(args.plugins_from, plugins)
                sdk_head = "prebuilt (--plugins-from)"
            check_plugins(plugins, args.native_test)
            render_programmes(os.path.abspath(args.fxlab), progs)
            shutil.copyfile(os.path.join(HERE, "fxprobe.sh"), os.path.join(tmp, "fxprobe.sh"))
            with open(os.path.join(tmp, "README.txt"), "w") as fh:
                fh.write(README)
            head = git_head(REPO) or "unknown"
            with open(os.path.join(tmp, "VERSION"), "w") as fh:
                fh.write("mpfree %s%s\n" % (head, " (with uncommitted changes)" if git_dirty(REPO) else ""))
                fh.write("sdk %s\n" % sdk_head)
                fh.write("made %s\n" % datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))
            names = files_under(tmp)
            with open(os.path.join(tmp, "SHA256SUMS"), "w") as fh:
                for rel in names:
                    fh.write("%s  %s\n" % (sha256(os.path.join(tmp, rel)), rel))
            for dirpath, dirnames, filenames in os.walk(tmp):
                os.chmod(dirpath, 0o755)
                for name in filenames:
                    os.chmod(os.path.join(dirpath, name), 0o644)
            os.rename(tmp, out)
        except BaseException:
            shutil.rmtree(tmp, ignore_errors=True)
            raise
    except Stop as e:
        print("STOP: %s" % e)
        return 1
    total = 0
    print("kit: %s" % out)
    for rel in files_under(out):
        size = os.path.getsize(os.path.join(out, rel))
        total += size
        print("  %10d  %s" % (size, rel))
    print("total %d bytes (%.1f MB) in %d files" % (total, total / 1e6, len(files_under(out))))
    print("STAGE-OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
