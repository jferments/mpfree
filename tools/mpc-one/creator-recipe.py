#!/usr/bin/env python3
"""OFFLINE, workstation only: write the MPFree USB creator's recipe, rust/mpfree-stick/src/recipe.json.

The creator (rust/mpfree-stick) makes the same changes to a copy of Akai's root filesystem as
offline-kexec-stick.py does for the soft-reboot boot with the pad player as the mpfree user (--app
--app-user, no --splash, no --akai-app, no kexec files). To keep the two from drifting, this tool doesn't
restate those changes: it RUNS the builder's own build_rootfs on a tiny fake root filesystem, with
placeholders such as @LOG_UUID@ for every value that differs from stick to stick, and records what
build_rootfs wrote (paths, modes, contents and links). The creator fills in the placeholders.

What build_rootfs decides from the image itself is not recorded as content but as rules the creator
applies to the real image: which of STUB_PROGRAMS exist (they become the stub), /etc/shadow (root's
"!" becomes "*", only with developer SSH), /etc/passwd and /etc/group (the mpfree user and its groups),
and MADE_IF_MISSING folders.

tests/test_creator_recipe.py checks that the recipe in the repository is what this writes now, so a
change to the builder fails the tests until the recipe is written again:
  python3 tools/mpc-one/creator-recipe.py --write
Needs e2fsprogs (mke2fs, debugfs, e2fsck). It never touches a device.
"""
import argparse
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
OUT = os.path.join(REPO, "rust", "mpfree-stick", "src", "recipe.json")
SBIN_PATH = os.pathsep.join(["/usr/sbin", "/sbin", "/usr/bin", "/bin", os.environ.get("PATH", "")])

# The placeholders, filled in by the creator for each stick.
LOG_UUID = "@LOG_UUID@"
KITS_UUID = "@KITS_UUID@"
DATA_UUID = "@DATA_UUID@"
RELEASE = "@RELEASE@"
ROTATION = "@ROTATION@"
PUBKEY = "@AUTHORIZED_KEYS@\n"
HOSTKEY = b"@HOST_KEY@\n"
APP = b"@PAD_PLAYER@"
SCREEN_LOG = b"@SCREEN_LOG@"


def builder():
    spec = importlib.util.spec_from_file_location("offline_kexec_stick", os.path.join(HERE, "offline-kexec-stick.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run(cmd):
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                         env=dict(os.environ, PATH=SBIN_PATH, LC_ALL="C"))
    if res.returncode != 0:
        raise SystemExit(f"{' '.join(cmd)} failed:\n{res.stdout}")
    return res


def fake_root(b, base):
    """The smallest root filesystem build_rootfs takes: what it requires, every program it stubs, the
    groups the mpfree user joins, and no MADE_IF_MISSING folder (so it makes them all)."""
    root = os.path.join(base, "root")
    files = {path: "# FAKE\n" for path, kind in b.REQUIRED.items() if kind == "regular"}
    files.update({prog: "#!/bin/sh\n# FAKE\n" for prog in b.STUB_PROGRAMS})
    files.update({f"/usr/bin/{prog}": "#!/bin/sh\n# FAKE\n" for prog in b.APP_USER_PROGRAMS})
    files["/etc/passwd"] = "root:x:0:0:root:/home/root:/bin/sh\n"
    files["/etc/group"] = "".join(f"{g}:x:{40 + i}:\n" for i, g in enumerate(("root",) + b.APP_GROUPS +
                                                                            b.APP_OPTIONAL_GROUPS))
    files["/etc/shadow"] = "root:!:19000:0:99999:7:::\n"
    files["/usr/bin/sh"] = "#!/fake/shell\n"   # /bin/sh, through the /bin link
    for path, text in files.items():
        full = os.path.join(root, path.lstrip("/"))
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w") as fh:
            fh.write(text)
    for path, kind in b.REQUIRED.items():
        if kind == "directory":
            os.makedirs(os.path.join(root, path.lstrip("/")), exist_ok=True)
    os.makedirs(os.path.join(root, "boot"), exist_ok=True)
    for top in ("bin", "sbin", "lib"):
        os.makedirs(os.path.join(root, "usr", top), exist_ok=True)
        if not os.path.lexists(os.path.join(root, top)):
            os.symlink("usr/" + top, os.path.join(root, top))
    for folder in b.MADE_IF_MISSING:
        shutil.rmtree(os.path.join(root, folder.lstrip("/")), ignore_errors=True)
    image = os.path.join(base, "rootfs.img")
    with open(image, "wb") as fh:
        fh.truncate(32 << 20)
    run(["mke2fs", "-q", "-F", "-t", "ext4", "-O", "^has_journal", "-E", "root_owner=0:0", "-d", root, image])
    return image


def recipe():
    b = builder()
    base = tempfile.mkdtemp(prefix="mpfree-creator-recipe-")
    try:
        scratch = os.path.join(base, "scratch")
        os.makedirs(scratch)
        img = b.Image(fake_root(b, base))
        host_key = os.path.join(base, "host_ed25519")
        with open(host_key, "wb") as fh:
            fh.write(HOSTKEY)
        b.OUT.lines = []
        _, stubbed, absent, links, written = b.build_rootfs(
            img, scratch, PUBKEY, host_key, LOG_UUID, SCREEN_LOG, ROTATION, app=APP, kits_uuid=KITS_UUID,
            data_uuid=DATA_UUID, softreboot=(RELEASE, ()), app_user=True)
        if absent:
            raise SystemExit(f"the fake root lacks stub programs: {absent}")
        from_image = {"/etc/shadow", "/etc/passwd", "/etc/group", *stubbed}
        files = {}
        for path in written:
            if path in from_image:
                continue
            data = img.read(path, scratch)
            mode, owner = img.stat(path)
            if owner != (0, 0):
                raise SystemExit(f"{path} is not root's")
            entry = {"mode": mode}
            if data in (APP, SCREEN_LOG, HOSTKEY):
                entry["placeholder"] = data.decode().strip()
            else:
                entry["text"] = data.decode("utf-8", "strict")
            files[path] = entry
        link_targets = {path: img.link(path) for path in links}
        ssh = sorted(p for p in list(files) + list(link_targets)
                     if "sshd" in p or p.startswith("/etc/mpfree-test/authorized_keys")
                     or p.startswith("/etc/mpfree-test/host_ed25519") or "ethernet" in p)
        mkdirs = ["/etc/mpfree-test", b.SOFTREBOOT_DIR]
        return {
            "about": "Written by tools/mpc-one/creator-recipe.py from offline-kexec-stick.py's build_rootfs "
                     "(--app --app-user, soft-reboot); do not edit by hand.",
            "placeholders": {"log_uuid": LOG_UUID, "kits_uuid": KITS_UUID, "data_uuid": DATA_UUID,
                             "release": RELEASE, "rotation": ROTATION, "authorized_keys": PUBKEY.strip(),
                             "host_key": HOSTKEY.decode().strip(), "pad_player": APP.decode(),
                             "screen_log": SCREEN_LOG.decode()},
            "files": files,
            "links": link_targets,
            "mkdirs": mkdirs,
            "made_if_missing": list(b.MADE_IF_MISSING),
            "required": b.REQUIRED,
            "stub": b.STUB,
            "stub_programs": list(b.STUB_PROGRAMS),
            "only_with_ssh": ssh,
            "softreboot_sums": b.SOFTREBOOT_DIR + "/SHA256SUMS",
            "softreboot_listed": list(b.INTERNAL_OFF_FILES) + [b.SOFTREBOOT_DIR + "/PROTOCOL",
                                                              b.SOFTREBOOT_DIR + "/release"],
            "app_user": {"name": b.APP_USER, "uid": b.APP_UID, "groups": list(b.APP_GROUPS),
                         "optional_groups": list(b.APP_OPTIONAL_GROUPS), "programs": list(b.APP_USER_PROGRAMS),
                         "gecos": "MPFree pad player", "home": "/run/mpfree-app", "shell": "/sbin/nologin"},
            "data_name": b.DATA_NAME,
            "data_start_sectors": b.DATA_START,
            "log_size": b.LOG_SIZE,
            "softreboot_protocol": b.SOFTREBOOT_PROTOCOL,
            "kits_max": b.KITS_MAX,
        }
    finally:
        shutil.rmtree(base, ignore_errors=True)


def text(r):
    return json.dumps(r, indent=1, sort_keys=True) + "\n"


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--write", action="store_true", help=f"write {os.path.relpath(OUT, REPO)}")
    args = ap.parse_args(argv)
    out = text(recipe())
    if args.write:
        with open(OUT, "w") as fh:
            fh.write(out)
        print(f"wrote {os.path.relpath(OUT, REPO)} ({len(out)} bytes)")
    else:
        sys.stdout.write(out)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
