#!/usr/bin/env python3
"""Builds the plugin pack: free plugins as CLAP files for the MPC One.

    python3 tools/plugin-pack/build.py --work DIR --out DIR [--play] [NAME ...]

Reads pack.toml beside this file. For each plugin named (all of them when none
is), it:

1. fetches each source at its pinned commit, with its submodules, into
   WORK/src, and checks the commit;
2. builds the plugin's CMake targets for 32-bit ARM with armv7-mpc.cmake, in
   WORK/build;
3. copies each .clap file to OUT/<plugin>/, stripped, and checks it: a 32-bit
   ARM hard-float library that exports clap_entry, needs nothing but the C
   library (libc, libm, the loader and glibc's empty stubs), and no glibc
   symbol newer than the MPC's 2.39;
4. writes OUT/<plugin>/licenses/, OUT/<plugin>/source/ (each source as a
   tarball, and this recipe), and OUT/<plugin>/BUILD.txt (what was built,
   from what, with which tools, and each file's SHA-256);
5. with --play, plays every plugin in each file under qemu-arm with
   mpfree-clap's clap_check example (it needs cargo and the
   armv7-unknown-linux-gnueabihf Rust target), into OUT/<plugin>/PLAY.txt.

The guide is docs/dev/guides/plugin-pack.md. Sources and built files never go
in the repository.

Exit codes: 0 every plugin built and passed, 1 one failed, 2 bad arguments.
"""

import argparse
import fnmatch
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tomllib
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
TOOLCHAIN = HERE / "armv7-mpc.cmake"
CROSS = "arm-linux-gnueabihf-"
# The MPC's C library (docs/dev/hardware/abi-handoff-survey.md).
TARGET_GLIBC = (2, 39)
# What a plugin may need from the MPC: the C library and its stubs.
ALLOWED_NEEDED = {
    "libc.so.6",
    "libm.so.6",
    "ld-linux-armhf.so.3",
    "libpthread.so.0",
    "libdl.so.2",
    "librt.so.1",
}


class Fail(Exception):
    pass


def run(cmd, cwd=None, env=None, log=None, capture=False):
    """Runs a command; with `log`, its output goes there."""
    shown = " ".join(str(c) for c in cmd)
    if log:
        with open(log, "a") as f:
            f.write(f"\n$ {shown}\n")
            f.flush()
            r = subprocess.run(cmd, cwd=cwd, env=env, stdout=f, stderr=subprocess.STDOUT)
    else:
        r = subprocess.run(cmd, cwd=cwd, env=env, text=True, capture_output=capture)
    if r.returncode != 0:
        where = f" (see {log})" if log else ""
        err = (r.stderr or "").strip()[-2000:] if capture else ""
        raise Fail(f"`{shown}` exited {r.returncode}{where}{': ' + err if err else ''}")
    return r.stdout if capture else None


def out_of(cmd, cwd=None):
    return run(cmd, cwd=cwd, capture=True).strip()


def say(msg):
    print(msg, flush=True)


# ------------------------------------------------------------------ fetching


def fetch_source(src, work, log):
    """Checks out `src` at its commit in WORK/src/<name>; reuses a clean one."""
    d = work / "src" / src["name"]
    commit = src["commit"]
    if (d / ".git").exists():
        try:
            head = out_of(["git", "rev-parse", "HEAD"], cwd=d)
            dirty = out_of(["git", "status", "--porcelain", "--ignore-submodules=none"], cwd=d)
        except Fail:
            head, dirty = "", "?"
        if head == commit and not dirty:
            return d
        say(f"  {src['name']}: the checkout isn't {commit[:12]} or isn't clean; fetching again")
        shutil.rmtree(d)
    d.mkdir(parents=True)
    say(f"  fetching {src['name']} {src['tag']} ({commit[:12]})")
    run(["git", "init", "-q"], cwd=d, log=log)
    run(["git", "remote", "add", "origin", src["url"]], cwd=d, log=log)
    run(["git", "fetch", "-q", "--depth", "1", "origin", commit], cwd=d, log=log)
    run(["git", "-c", "advice.detachedHead=false", "checkout", "-q", "--detach", "FETCH_HEAD"], cwd=d, log=log)
    if out_of(["git", "rev-parse", "HEAD"], cwd=d) != commit:
        raise Fail(f"{src['name']}: fetched something other than {commit}")
    if src.get("submodules"):
        try:
            run(["git", "submodule", "update", "-q", "--init", "--recursive", "--depth", "1", "--jobs", "4"], cwd=d, log=log)
        except Fail:
            # A server that won't hand out an unadvertised commit shallowly.
            run(["git", "submodule", "update", "-q", "--init", "--recursive", "--jobs", "4"], cwd=d, log=log)
    return d


def fetch_file(f, work):
    d = work / "files"
    d.mkdir(parents=True, exist_ok=True)
    path = d / f["name"]
    if not path.exists() or sha256(path) != f["sha256"]:
        say(f"  fetching {f['name']}")
        with urllib.request.urlopen(f["url"], timeout=60) as r:
            path.write_bytes(r.read())
    got = sha256(path)
    if got != f["sha256"]:
        raise Fail(f"{f['name']}: SHA-256 {got}, expected {f['sha256']}")
    return path


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


# ------------------------------------------------------------------ building


def build(plugin, srcs, work, jobs, log):
    """Configures and builds; returns the build folder."""
    b = work / "build" / plugin["name"]
    project = plugin["project"]
    if project.startswith("src:"):
        project_dir = srcs[project[4:]]
    else:
        project_dir = HERE / project

    def subst(arg):
        arg = arg.replace("{recipe}", str(HERE))
        return re.sub(r"\{src:([a-z0-9-]+)\}", lambda m: str(srcs[m.group(1)]), arg)

    env = dict(os.environ)
    main = srcs[plugin["sources"][0]]
    env["SOURCE_DATE_EPOCH"] = out_of(["git", "log", "-1", "--format=%ct"], cwd=main)
    flags = f"-ffile-prefix-map={work}=. -ffile-prefix-map={HERE}=recipe {plugin.get('cflags', '')}".strip()
    env["CFLAGS"] = flags
    env["CXXFLAGS"] = flags
    # A fresh build folder each time: CMake keeps the flags of its first run.
    if b.exists():
        shutil.rmtree(b)
    say(f"  configuring {plugin['name']}")
    run(
        [
            "cmake",
            "-S",
            project_dir,
            "-B",
            b,
            "-G",
            "Ninja",
            f"-DCMAKE_TOOLCHAIN_FILE={TOOLCHAIN}",
            "-DCMAKE_BUILD_TYPE=Release",
            *[subst(a) for a in plugin.get("cmake", [])],
        ],
        env=env,
        log=log,
    )
    say(f"  building {', '.join(plugin['targets'])} (this takes a while)")
    run(["cmake", "--build", b, "-j", str(jobs), "--target", *plugin["targets"]], env=env, log=log)
    return b


def find_output(b, pattern):
    hits = sorted(p for p in b.rglob("*") if p.is_file() and fnmatch.fnmatch(str(p.relative_to(b)), pattern))
    hits = [p for p in hits if "CMakeFiles" not in p.parts]
    if len(hits) != 1:
        raise Fail(f"{pattern}: {len(hits)} files match in {b}")
    return hits[0]


# ------------------------------------------------------------------ checks


def check_clap(path):
    """Returns lines describing the file; raises Fail on anything wrong."""
    header = out_of([CROSS + "readelf", "-h", path])
    if "ELF32" not in header or "ARM" not in header:
        raise Fail(f"{path.name}: not a 32-bit ARM file")
    if "hard-float" not in header:
        raise Fail(f"{path.name}: not hard-float")
    if "DYN (Shared object" not in header:
        raise Fail(f"{path.name}: not a shared library")
    dynamic = out_of([CROSS + "readelf", "-d", path])
    needed = re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", dynamic)
    extra = sorted(set(needed) - ALLOWED_NEEDED)
    if extra:
        raise Fail(f"{path.name}: needs libraries the MPC may not have: {', '.join(extra)}")
    versions = out_of([CROSS + "readelf", "-V", "-W", path])
    glibc = sorted(
        {tuple(int(x) for x in v.split(".")) for v in re.findall(r"Name: GLIBC_([0-9.]+)", versions)}
    )
    if glibc and glibc[-1] > TARGET_GLIBC:
        newest = ".".join(map(str, glibc[-1]))
        raise Fail(f"{path.name}: needs glibc {newest}, newer than the MPC's 2.39")
    others = sorted(set(re.findall(r"Name: ((?:GLIBCXX|CXXABI|GCC)_[0-9.]+)", versions)))
    if others:
        raise Fail(f"{path.name}: needs the C++ library or libgcc from the MPC: {', '.join(others)}")
    exported = out_of([CROSS + "nm", "-D", "--defined-only", path]).split("\n")
    names = {line.split()[-1] for line in exported if line.strip()}
    if "clap_entry" not in names:
        raise Fail(f"{path.name}: doesn't export clap_entry")
    attrs = out_of([CROSS + "readelf", "-A", path])
    fp = re.search(r"Tag_FP_arch: (.*)", attrs)
    simd = re.search(r"Tag_Advanced_SIMD_arch: (.*)", attrs)
    newest = ".".join(map(str, glibc[-1])) if glibc else "none"
    return [
        f"needs: {', '.join(needed)}",
        f"newest glibc symbol: {newest} (the MPC has 2.39)",
        f"exports: {', '.join(sorted(names))}",
        f"floating point: {fp.group(1).strip() if fp else '?'}; SIMD: {simd.group(1).strip() if simd else 'none'}",
    ]


# ------------------------------------------------------------------ the pack


def source_tarball(name, d, dest, epoch, log):
    """A tarball of every file git tracks, submodules included, made the same
    way each time."""
    files = out_of(["git", "ls-files", "--recurse-submodules", "-z"], cwd=d).split("\0")
    listing = dest.with_suffix(".list")
    listing.write_text("\0".join(f for f in files if f))
    tar = dest.with_suffix("")  # .tar
    run(
        [
            "tar",
            "--create",
            "--file",
            tar,
            "--directory",
            d,
            "--null",
            "--files-from",
            listing,
            "--sort=name",
            f"--mtime=@{epoch}",
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "--format=gnu",
            f"--transform=s,^,{name}/,",
        ],
        log=log,
    )
    listing.unlink()
    if dest.exists():
        dest.unlink()
    run(["xz", "-T0", "-6", tar], log=log)
    return dest


def recipe_tarball(dest, log):
    run(
        [
            "tar",
            "--create",
            "--xz",
            "--file",
            dest,
            "--directory",
            HERE.parent,
            "--sort=name",
            "--mtime=@0",
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "--exclude=__pycache__",
            HERE.name,
        ],
        log=log,
    )


def tool_versions():
    lines = []
    for cmd in ([CROSS + "gcc", "--version"], ["cmake", "--version"], ["ninja", "--version"]):
        try:
            lines.append(f"{cmd[0]}: {out_of(cmd).splitlines()[0]}")
        except (Fail, FileNotFoundError):
            lines.append(f"{cmd[0]}: not found")
    return lines


def checker(work, log):
    """Builds clap_check for ARM; returns its path."""
    rust = REPO / "rust"
    env = dict(os.environ)
    env.setdefault("CARGO_INCREMENTAL", "0")
    env.setdefault("CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER", CROSS + "gcc")
    say("  building clap_check for ARM")
    run(
        [
            "cargo",
            "build",
            "--locked",
            "--release",
            "-p",
            "mpfree-clap",
            "--example",
            "clap_check",
            "--target",
            "armv7-unknown-linux-gnueabihf",
        ],
        cwd=rust,
        env=env,
        log=log,
    )
    target = Path(env.get("CARGO_TARGET_DIR", rust / "target"))
    return target / "armv7-unknown-linux-gnueabihf" / "release" / "examples" / "clap_check"


def play(check, clap, log):
    """Runs clap_check on one file under qemu; returns (passed, output)."""
    prefix = os.environ.get("QEMU_LD_PREFIX", "/usr/arm-linux-gnueabihf")
    r = subprocess.run(["qemu-arm", "-L", prefix, check, clap], text=True, capture_output=True, timeout=3600)
    with open(log, "a") as f:
        f.write(f"\n$ qemu-arm -L {prefix} {check} {clap}\n{r.stdout}{r.stderr}")
    return r.returncode == 0, (r.stdout + r.stderr).strip()


def make_plugin(plugin, pack, work, out, jobs, check):
    name = plugin["name"]
    dest = out / name
    if dest.exists():
        shutil.rmtree(dest)
    (dest / "licenses").mkdir(parents=True)
    (dest / "source").mkdir()
    log = work / "logs" / f"{name}.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    log.write_text("")

    sources = {s["name"]: s for s in pack["source"]}
    files = {f["name"]: f for f in pack.get("file", [])}
    srcs = {}
    for s in plugin["sources"]:
        srcs[s] = fetch_source(sources[s], work, log)
    fetched = {}
    for spec in plugin.get("licenses", {}):
        if spec.startswith("file:"):
            fetched[spec[5:]] = fetch_file(files[spec[5:]], work)

    b = build(plugin, srcs, work, jobs, log)

    report = [f"# {plugin['title']}", "", f"license: {plugin['license']}", ""]
    report.append("## Files")
    sums = []
    played_ok = True
    plays = []
    for pattern, outname in plugin["outputs"].items():
        built = find_output(b, pattern)
        target = dest / outname
        run([CROSS + "strip", "--strip-unneeded", "-o", target, built], log=log)
        lines = check_clap(target)
        size = target.stat().st_size
        digest = sha256(target)
        sums.append(f"{digest}  {outname}")
        report += ["", f"{outname} ({size / 1e6:.1f} MB)", f"sha256: {digest}", *lines]
        say(f"  {outname}: {size / 1e6:.1f} MB, checks pass")
        if check:
            ok, text = play(check, target, log)
            played_ok &= ok
            plays.append(f"## {outname}\n\n{text}\n")
            last = text.splitlines()[-1] if text else "no output"
            say(f"  played {outname}: {last}")

    for spec, outname in plugin.get("licenses", {}).items():
        if spec.startswith("file:"):
            shutil.copyfile(fetched[spec[5:]], dest / "licenses" / outname)
        else:
            src_name, rel = spec.split("/", 1)
            shutil.copyfile(srcs[src_name] / rel, dest / "licenses" / outname)

    report += ["", "## Sources", ""]
    for s in plugin["sources"]:
        info = sources[s]
        d = srcs[s]
        epoch = out_of(["git", "log", "-1", "--format=%ct"], cwd=d)
        ball = source_tarball(s, d, dest / "source" / f"{s}-{info['tag']}.tar.xz", epoch, log)
        report.append(f"{s}: {info['url']} {info['tag']} {info['commit']}")
        report.append(f"  tarball: source/{ball.name} sha256 {sha256(ball)}")
        if info.get("submodules"):
            subs = out_of(["git", "submodule", "status", "--recursive"], cwd=d)
            report += [f"  {line.strip()}" for line in subs.splitlines()]
    for spec in plugin.get("licenses", {}):
        if spec.startswith("file:"):
            f = files[spec[5:]]
            report.append(f"{f['name']}: {f['url']} sha256 {f['sha256']}")
    recipe_tarball(dest / "source" / "mpfree-plugin-pack-recipe.tar.xz", log)
    try:
        commit = out_of(["git", "rev-parse", "HEAD"], cwd=REPO)
        dirty = " (with changes not committed)" if out_of(["git", "status", "--porcelain", "--", str(HERE)], cwd=REPO) else ""
    except Fail:
        commit, dirty = "unknown", ""
    report += ["", "## Recipe", "", f"MPFree {commit}{dirty}, tools/plugin-pack (also in source/)"]
    report += [f"cmake: {' '.join(plugin.get('cmake', []))}", f"extra flags: {plugin.get('cflags', 'none')}"]
    report += ["", "## Tools", "", *tool_versions()]
    (dest / "BUILD.txt").write_text("\n".join(report) + "\n")
    if plays:
        (dest / "PLAY.txt").write_text("\n".join(plays))
    (dest / "SHA256SUMS").write_text("\n".join(sums) + "\n")
    if not played_ok:
        raise Fail(f"{name}: a plugin failed to play (see {dest / 'PLAY.txt'})")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--work", required=True, type=Path, help="folder for sources, builds and logs")
    ap.add_argument("--out", required=True, type=Path, help="folder for the pack")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 2)
    ap.add_argument("--play", action="store_true", help="play every plugin under qemu-arm afterwards")
    ap.add_argument("names", nargs="*", help="plugins to build (all when none)")
    args = ap.parse_args()

    pack = tomllib.loads((HERE / "pack.toml").read_text())
    plugins = {p["name"]: p for p in pack["plugin"]}
    wanted = args.names or list(plugins)
    unknown = [n for n in wanted if n not in plugins]
    if unknown:
        print(f"unknown plugin(s): {', '.join(unknown)}; the pack has {', '.join(plugins)}", file=sys.stderr)
        return 2
    work, out = args.work.resolve(), args.out.resolve()
    for d in (work, out):
        if d == REPO or REPO in d.parents:
            print(f"{d} is inside the repository; sources and builds stay out of it", file=sys.stderr)
            return 2
        d.mkdir(parents=True, exist_ok=True)

    (work / "logs").mkdir(exist_ok=True)
    check = None
    failed = []
    if args.play:
        try:
            check = checker(work, work / "logs" / "clap_check.log")
        except Fail as e:
            print(f"can't build clap_check: {e}", file=sys.stderr)
            return 1
    for n in wanted:
        say(f"{n}:")
        try:
            make_plugin(plugins[n], pack, work, out, args.jobs, check)
            say(f"  done: {out / n}")
        except Fail as e:
            say(f"  FAILED: {e}")
            failed.append(n)
    if failed:
        say(f"failed: {', '.join(failed)}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
