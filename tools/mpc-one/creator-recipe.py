#!/usr/bin/env python3
"""OFFLINE, workstation only: write the MPFree USB creator's recipe, rust/mpfree-stick/src/recipe.json.

The creator (rust/mpfree-stick) makes the same changes to a copy of Akai's root filesystem as
offline-kexec-stick.py does for the soft-reboot boot with the pad player as the mpfree user and MPFree's
startup screen (--app --app-user --splash mpfree-startup --fetch --plughost, no --akai-app, no kexec files). To keep the two
from drifting, this tool doesn't restate those changes: it RUNS the builder's own build_rootfs on a tiny
fake root filesystem, with placeholders such as @LOG_UUID@ for every value that differs from stick to
stick, and records what build_rootfs wrote (paths, modes, contents and links). The creator fills in the
placeholders.

What build_rootfs decides from the image itself is not recorded as content but as rules the creator
applies to the real image: which of STUB_PROGRAMS exist (they become the stub), /etc/shadow (root's
"!" becomes "*", only with developer SSH), /etc/passwd and /etc/group (the mpfree user and its groups),
and MADE_IF_MISSING folders.

The developer routes (--kexec-file, --kexec-file --loadpin-off, and --chain with both) are recorded the
same way, each from its own build_rootfs run, as what differs from the soft-reboot stick ("routes"):
files and links added, links not made, stub programs left as Akai's, and the hook's list. More
placeholders there: @ROOT_UUID@ (partition 1), @KERNEL_SHA256@ (the image's kernel) and @KEXEC_DTB@ (the
chain's device tree, which the creator builds as build_kexec_dtb does). /boot/mpfree-kexec/SHA256SUMS is a
rule, like the hook's list: the names it lists, hashed by the creator.

It also writes an example for the creator's device-tree code (rust/mpfree-stick/tests/data/
kexec-dtb-example.json): a made-up board file and running tree (no real values), and the kexec.dtb and
notes build_kexec_dtb makes from them. The creator's tests build the same bytes.

tests/test_creator_recipe.py checks that both files in the repository are what this writes now, so a
change to the builder fails the tests until they are written again:
  python3 tools/mpc-one/creator-recipe.py --write
Needs e2fsprogs (mke2fs, debugfs, e2fsck). It never touches a device.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
OUT = os.path.join(REPO, "rust", "mpfree-stick", "src", "recipe.json")
EXAMPLE_OUT = os.path.join(REPO, "rust", "mpfree-stick", "tests", "data", "kexec-dtb-example.json")
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
STARTUP = b"@STARTUP@"
# The Plugins place's download helper (--fetch): the creator writes it only when its programs folder has one.
FETCH = b"@FETCH@"
# The process each plugin runs in (--plughost): the creator writes it only when its programs folder has one.
PLUGHOST = b"@PLUGHOST@"
# Only in the developer routes.
ROOT_UUID = "@ROOT_UUID@"
KERNEL_SHA256 = "@KERNEL_SHA256@"
KEXEC_DTB = b"@KEXEC_DTB@"
KERNEL = b"# FAKE KERNEL\n"   # the fake root's /boot/zImage-@RELEASE@

# The creator's developer routes: name -> (--kexec-file, --loadpin-off, --chain).
ROUTES = {
    "kexec-file": (True, False, False),
    "kexec-file-loadpin-off": (True, True, False),
    "chain": (True, True, True),
}


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
    files["/boot/zImage-" + RELEASE] = KERNEL.decode()
    files[b.SHUTDOWN_PROGRAM] = "# FAKE: runs /usr/sbin/kexec for kexec.target\n"
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


class Quiet:
    """The builder's report, kept but not printed (it would mix with the recipe on stdout)."""

    def __init__(self):
        self.lines = []

    def write(self, text):
        self.lines.append(text)

    def say(self, text=""):
        self.write(text + "\n")


def sums_listed(text):
    """The names a SHA256SUMS file lists, in its order."""
    return [line.split("  ", 1)[1] for line in text.splitlines()]


def build(b, base, name, route=(False, False, False)):
    """build_rootfs on a fresh fake root, as the creator's stick with this route. Returns what it wrote:
    {"files": {path: entry}, "links": {path: target}, "stubbed": [...], "sums": {path: listed}}."""
    kexec_file, loadpin_off, chain = route
    work = os.path.join(base, name)
    scratch = os.path.join(work, "scratch")
    os.makedirs(scratch)
    img = b.Image(fake_root(b, work))
    host_key = os.path.join(work, "host_ed25519")
    with open(host_key, "wb") as fh:
        fh.write(HOSTKEY)
    cmdline, kexec_file_cmdline = b.command_lines(ROOT_UUID, kexec_file, loadpin_off, chain)
    kexec_files = (RELEASE, KEXEC_DTB, (cmdline + "\n").encode()) if chain else None
    _, stubbed, absent, links, written = b.build_rootfs(
        img, scratch, PUBKEY, host_key, LOG_UUID, SCREEN_LOG, ROTATION, app=APP, kits_uuid=KITS_UUID,
        data_uuid=DATA_UUID, kexec_files=kexec_files, softreboot=(RELEASE, (), kexec_file_cmdline),
        app_user=True, splash=(STARTUP, None, None), chain=chain, fetch=FETCH, plughost=PLUGHOST)
    if absent:
        raise SystemExit(f"the fake root lacks stub programs: {absent}")
    kernel_sum = hashlib.sha256(KERNEL).hexdigest()
    from_image = {"/etc/shadow", "/etc/passwd", "/etc/group", *stubbed}
    files, sums = {}, {}
    for path in written:
        if path in from_image:
            continue
        data = img.read(path, scratch)
        mode, owner = img.stat(path)
        if owner != (0, 0):
            raise SystemExit(f"{path} is not root's")
        if path.endswith("/SHA256SUMS"):
            sums[path] = sums_listed(data.decode())
        entry = {"mode": mode}
        if data in (APP, SCREEN_LOG, STARTUP, HOSTKEY, FETCH, PLUGHOST, KEXEC_DTB):
            entry["placeholder"] = data.decode().strip()
        else:
            entry["text"] = data.decode("utf-8", "strict").replace(kernel_sum, KERNEL_SHA256)
        files[path] = entry
    return {"files": files, "links": {path: img.link(path) for path in links}, "stubbed": list(stubbed),
            "sums": sums}


def route_record(b, soft, made, chain):
    """What a route's stick has that the soft-reboot stick does not, and the other way round."""
    hook_sums = b.SOFTREBOOT_DIR + "/SHA256SUMS"
    kexec_sums = b.KEXEC_DIR + "/SHA256SUMS"
    for path in set(soft["files"]) & set(made["files"]):
        if soft["files"][path] != made["files"][path] and path != hook_sums:
            raise SystemExit(f"{path} differs between the soft-reboot stick and a route; record it as one")
    for path in set(soft["links"]) & set(made["links"]):
        if soft["links"][path] != made["links"][path]:
            raise SystemExit(f"the link {path} differs between the soft-reboot stick and a route")
    gone = set(soft["files"]) - set(made["files"])
    if gone:
        raise SystemExit(f"a route does not write {sorted(gone)}; record it")
    added = {p: e for p, e in made["files"].items() if p not in soft["files"] and p != kexec_sums}
    for path in list(added) + [p for p in made["links"] if p not in soft["links"]]:
        if "sshd" in path or "authorized_keys" in path or "host_ed25519" in path or "ethernet" in path:
            raise SystemExit(f"{path} looks like developer SSH; the routes record none")
    return {
        "files": added,
        "links": {p: t for p, t in made["links"].items() if p not in soft["links"]},
        "unlinked": sorted(set(soft["links"]) - set(made["links"])),
        "unstubbed": sorted(set(soft["stubbed"]) - set(made["stubbed"])),
        "mkdirs": [b.KEXEC_DIR] if chain else [],
        "softreboot_listed": made["sums"][hook_sums],
        "kexec_sums": {"path": kexec_sums, "dir": b.KEXEC_DIR, "names": made["sums"][kexec_sums]}
        if chain else None,
        "chain_check": {"kexec_program": b.KEXEC_PROGRAM, "shutdown_program": b.SHUTDOWN_PROGRAM,
                        "kexec_paths": [p.decode() for p in b.KEXEC_PATHS]} if chain else None,
    }


def recipe():
    b = builder()
    quiet = Quiet()
    b.OUT = quiet
    base = tempfile.mkdtemp(prefix="mpfree-creator-recipe-")
    try:
        soft = build(b, base, "soft-reboot")
        files, link_targets = soft["files"], soft["links"]
        routes = {name: route_record(b, soft, build(b, base, name, route), route[2])
                  for name, route in ROUTES.items()}
        ssh = sorted(p for p in list(files) + list(link_targets)
                     if "sshd" in p or p.startswith("/etc/mpfree-test/authorized_keys")
                     or p.startswith("/etc/mpfree-test/host_ed25519") or "ethernet" in p)
        mkdirs = ["/etc/mpfree-test", b.SOFTREBOOT_DIR]
        dtb = b.dtb
        return {
            "about": "Written by tools/mpc-one/creator-recipe.py from offline-kexec-stick.py's build_rootfs "
                     "(--app --app-user --splash, soft-reboot; and each developer route); do not edit by hand.",
            "placeholders": {"log_uuid": LOG_UUID, "kits_uuid": KITS_UUID, "data_uuid": DATA_UUID,
                             "release": RELEASE, "rotation": ROTATION, "authorized_keys": PUBKEY.strip(),
                             "host_key": HOSTKEY.decode().strip(), "pad_player": APP.decode(),
                             "screen_log": SCREEN_LOG.decode(), "startup": STARTUP.decode(),
                             "fetch": FETCH.decode(), "plughost": PLUGHOST.decode(), "root_uuid": ROOT_UUID, "kernel_sha256": KERNEL_SHA256,
                             "kexec_dtb": KEXEC_DTB.decode()},
            "files": files,
            "links": link_targets,
            "mkdirs": mkdirs,
            "made_if_missing": list(b.MADE_IF_MISSING),
            "required": b.REQUIRED,
            "stub": b.STUB,
            "stub_programs": list(b.STUB_PROGRAMS),
            "only_with_ssh": ssh,
            "softreboot_sums": b.SOFTREBOOT_DIR + "/SHA256SUMS",
            "softreboot_listed": soft["sums"][b.SOFTREBOOT_DIR + "/SHA256SUMS"],
            "routes": routes,
            "kexec_dtb": {"ethernet": b.ETHERNET, "phy_compatible": b.PHY_ID_COMPATIBLE,
                          "emmc_content": b.EMMC_CONTENT,
                          "hidden_controllers": [list(c) for c in dtb.HIDDEN_CONTROLLERS],
                          "chosen_not_copied": list(dtb.CHOSEN_NOT_COPIED)},
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


def example_trees(dtb):
    """A made-up MPC One board file, and the running tree U-Boot would make of it: a serial number, a MAC
    address, the memory size, a command line, a node and a memory reservation. No real unit's values."""
    def strings(*texts):
        return b"".join(t.encode() + b"\x00" for t in texts)

    def cells(*values):
        return struct.pack(f">{len(values)}I", *values)

    def node(name, props=(), children=()):
        n = dtb.Node(name)
        n.props = [[k, v] for k, v in props]
        n.children = list(children)
        return n

    board = dtb.Tree(node("", [
        ("model", strings("Example MPC One board file")),
        ("compatible", strings("inmusic,acva", "inmusic,az01", "rockchip,rk3288")),
        ("#address-cells", cells(1)), ("#size-cells", cells(1))], [
        node("aliases", [("mmc0", strings("/dwmmc@ff0f0000")), ("mmc1", strings("/dwmmc@ff0c0000")),
                         ("ethernet0", strings("/ethernet@ff290000"))]),
        node("chosen", [("stdout-path", strings("serial2:115200n8")), ("inmusic,acvs-emmc-content", b"")]),
        node("memory@0", [("device_type", strings("memory")), ("reg", cells(0, 0))]),
        node("dwmmc@ff0c0000", [("compatible", strings("rockchip,rk3288-dw-mshc")),
                                ("reg", cells(0xFF0C0000, 0x4000))]),
        node("dwmmc@ff0f0000", [("compatible", strings("rockchip,rk3288-dw-mshc")),
                                ("reg", cells(0xFF0F0000, 0x4000)), ("status", strings("okay")),
                                ("non-removable", b"")]),
        node("ethernet@ff290000", [("compatible", strings("rockchip,rk3288-gmac")),
                                   ("reg", cells(0xFF290000, 0x10000)), ("phy-handle", cells(0x41))], [
            node("mdio", [("#address-cells", cells(1)), ("#size-cells", cells(0))], [
                node("ethernet-phy@1", [("compatible", strings("ethernet-phy-ieee802.3-c22")), ("reg", cells(1)),
                                        ("reset-gpios", cells(0x42, 8, 1)), ("phandle", cells(0x41))])])]),
        node("mipi@ff960000", [], [node("panel@0", [("rotation", cells(270))])]),
    ]), [(0x10000000, 0x100000)], 0)
    live = dtb.parse(dtb.build(board))
    live.root.set_prop("serial-number", strings("EXAMPLE-SERIAL-0"))
    live.find("/ethernet@ff290000").set_prop("local-mac-address", bytes.fromhex("020000000001"))
    live.find("/memory@0").set_prop("reg", cells(0, 0x80000000))
    live.find("/chosen").set_prop("bootargs", strings("root=PARTUUID=EXAMPLE rootwait ro"))
    live.find("/chosen").set_prop("rng-seed", bytes(range(32)))
    live.root.children.append(node("reserved-memory", [("ranges", b"")], [
        node("ramoops@8000000", [("reg", cells(0x8000000, 0x100000))])]))
    live.reserved.append((0x8F000000, 0xD000))
    return dtb.build(board), dtb.build(live)


def example():
    """build_kexec_dtb on the made-up trees, the running release's board file being the stick's own (as
    the creator has it): the tree and the notes the creator's own code must give."""
    b = builder()
    quiet = Quiet()
    b.OUT = quiet
    base = tempfile.mkdtemp(prefix="mpfree-creator-example-")
    try:
        board, live = example_trees(b.dtb)
        paths = {name: os.path.join(base, name) for name in ("board.dtb", "live.dtb", "kexec.dtb")}
        for name, data in (("board.dtb", board), ("live.dtb", live)):
            with open(paths[name], "wb") as fh:
                fh.write(data)
        phy = b.build_kexec_dtb(paths["board.dtb"], paths["board.dtb"], paths["live.dtb"], paths["kexec.dtb"])
        with open(paths["kexec.dtb"], "rb") as fh:
            out = fh.read()
        notes = [line for line in "".join(quiet.lines).splitlines() if not line.startswith("written: ")]
        return {
            "about": "Written by tools/mpc-one/creator-recipe.py: offline-kexec-stick.py's build_kexec_dtb on a "
                     "made-up board file and running tree (no real unit's values); do not edit by hand.",
            "board": board.hex(), "live": live.hex(), "kexec_dtb": out.hex(), "notes": notes, "phy": phy,
        }
    finally:
        shutil.rmtree(base, ignore_errors=True)


def text(r):
    return json.dumps(r, indent=1, sort_keys=True) + "\n"


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--write", action="store_true",
                    help=f"write {os.path.relpath(OUT, REPO)} and {os.path.relpath(EXAMPLE_OUT, REPO)}")
    args = ap.parse_args(argv)
    out = text(recipe())
    if args.write:
        for path, data in ((OUT, out), (EXAMPLE_OUT, text(example()))):
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as fh:
                fh.write(data)
            print(f"wrote {os.path.relpath(path, REPO)} ({len(data)} bytes)")
    else:
        sys.stdout.write(out)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
