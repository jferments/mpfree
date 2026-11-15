#!/usr/bin/env python3
"""WORKSTATION ONLY: scan the owner's eMMC backup, read-only, for three questions
about Akai's own instruments. It never contacts the MPC.

  python3 tools/mpc-one/offline-akai-scan.py [--backup FOLDER]

1. Are Akai's instruments (Hype, TubeSynth, Mellotron, Solina ...) separate
   plugin files, or built into Akai's app (/usr/bin/MPC)?
2. Where does the activation live? File NAMES only: no licence or activation
   file is opened, and nothing from one is copied.
3. What would "guest mode" need (MPFree running Akai's own, unmodified app in
   the background as a sound engine)? How the app links ALSA, DRM and EGL,
   which devices it names, whether it locks its memory, what Akai's kernel
   offers (vkms, uinput, snd-virmidi, namespaces, cgroups), and the app's
   memory and voice settings.

How it reads: the GPT of the newest ~/mpfree-backups/mpc-one-emmc-*/mmcblk0.img
(or --backup FOLDER), then the rootfs (p6), content (p7) and data (p8) ext4
file systems with debugfs, which opens the image read-only (no -w) at each
partition's offset. Nothing is mounted and no sudo is needed. Only Akai's app,
its launcher and unit, plugin candidates, the kernel and small settings files
are copied into a temporary folder to be read, and deleted at the end.

It writes ~/mpfree-work/akai-scan-<UTC>-XXXXXXXX/report.txt. The report names
Akai's files and quotes short strings from Akai's app (names, paths, device
names); it holds no audio, no licence file contents, and no settings values
whose names suggest accounts, keys or serials. Read it before you upload it.

MPFREE_BACKUPS_DIR and MPFREE_WORK_DIR change the folders (tests use them).
"""
import argparse
import hashlib
import importlib.util
import os
import re
import struct
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))


def _load(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


fk = _load("offline_factory_kit", "offline-factory-kit.py")
kc = _load("offline_kernel_config", "offline-kernel-config.py")
Stop = fk.Stop
SECTOR = fk.SECTOR

PARTS = ("rootfs", "content", "data")
APP = "/usr/bin/MPC"
LAUNCHER = "/usr/bin/az01-launch-MPC"
UNIT_DIRS = ("/lib/systemd/system", "/usr/lib/systemd/system", "/etc/systemd/system")
MAX_BINARY = 512 << 20
MAX_CANDIDATES = 300
MAX_SETTINGS = 2 << 20

# Akai's instruments, as their names may show up in strings and symbols. Some are common words
# (Electric, Organ, Stage): their counts are noisy and the examples say more.
INSTRUMENTS = (
    ("Hype", rb"Hype"), ("TubeSynth", rb"Tube ?[Ss]ynth"), ("Bassline", rb"Bassline"),
    ("Electric", rb"Electric"), ("Mellotron", rb"Mellotron"), ("Solina", rb"Solina"),
    ("Odyssey", rb"Odyssey"), ("DrumSynth", rb"Drum ?[Ss]ynth"), ("Fabric", rb"Fabric"),
    ("OPx-4", rb"OPx"), ("Studio Strings", rb"Studio ?Strings"), ("Stage", rb"Stage ?(?:Piano|EP|Keys)"),
    ("Organ", rb"(?:AIR ?)?Organ"), ("AIR", rb"\bAIR\b"),
)
PLUGIN_EXT = re.compile(r"(?i)\.(so(\.[0-9]+)*|vst3?|clap|component|bundle)$")
PLUGIN_WORDS = re.compile(r"(?i)(akai|air|inmusic|az0|mpc|plug-?in|synth|vst|juce|instrument|effect|fx)")
PLUGIN_DIRS = re.compile(r"(?i)^(plug-?ins?|vst[0-9]? ?plug-?ins?|vst[0-9]?|synths?|instruments?|effects?|clap|air)$")
SYSTEM_LIB = re.compile(r"^/(usr/)?lib(32|64)?/")
INTERESTING_EXPORTS = ("VSTPluginMain", "main", "clap_entry", "GetPluginFactory", "InitDll", "ModuleEntry",
                       "JUCEApplicationBase", "createPlugin", "PluginEntry")
LICENCE = re.compile(r"(?i)(licen[cs]|activat|authori[sz]|regist|entitle|serial|token|credential|account|ilok|"
                     r"\bpace\b|\.lic$|keychain|purchase|unlock|inmusic.?profile)")
LICENCE_NOISE = re.compile(r"(?i)(^/usr/share/(common-)?licenses(/|$)|^/usr/share/doc(/|$)|"
                           r"/(licen[cs]e|copying|notice)([-_.][^/]*)?$|/licen[cs]es?/)")
IMPORTS = ("mlockall", "mlock", "munlockall", "sched_setaffinity", "pthread_setaffinity_np", "sched_setscheduler",
           "pthread_setschedparam", "snd_pcm_open", "snd_rawmidi_open", "snd_seq_open", "snd_ctl_open",
           "drmModeSetCrtc", "drmModePageFlip", "drmModeAtomicCommit", "gbm_create_device", "eglGetDisplay",
           "eglGetPlatformDisplay", "eglGetPlatformDisplayEXT", "fbdev_window", "libinput_udev_create_context",
           "udev_new", "dlopen", "inotify_init", "prctl", "setrlimit")
KCONFIG = re.compile(
    r"^(# )?CONFIG_(DRM_VKMS|SND_ALOOP|SND_VIRMIDI|SND_SEQ_DUMMY|SND_SEQUENCER|SND_DUMMY|INPUT_UINPUT|"
    r"INPUT_EVDEV|NAMESPACES|USER_NS|PID_NS|NET_NS|IPC_NS|UTS_NS|CGROUPS|MEMCG|CPUSETS|CGROUP_SCHED|"
    r"RT_GROUP_SCHED|CGROUP_FREEZER|CGROUP_DEVICE|SECCOMP|MODULES|MODULE_SIG[A-Z_]*|ZRAM|SWAP|ZSWAP|"
    r"DRM_LIMA|DRM_PANFROST|MALI[A-Z0-9_]*|FB|DRM_FBDEV_EMULATION|FUSE_FS|PREEMPT_RT|NR_CPUS)(=| is not set)")
KMODS = re.compile(r"(vkms|aloop|virmidi|uinput|seq-dummy|snd-dummy|fuse|zram)")
SETTINGS_FILE = re.compile(r"(?i)(\.settings$|preferences|\.prefs$|^mpc\.xml$|settings\.xml$)")
SETTINGS_KEEP = re.compile(r"(?i)(memory|\bram|pool|buffer|cache|voice|polyph|thread|cpu|core|latency|"
                           r"sample.?rate|block|plugin|disk|stream|preload|content.?loc|synth)")
SETTINGS_DROP = re.compile(r"(?i)(licen|token|account|user|mail|serial|passw|secret|key|auth|activ|"
                           r"credential|session|cookie|uuid|guid|device.?id)")
EMAIL = re.compile(r"[^/@\s\"']+@[^/\s\"']+")
STRINGS = re.compile(rb"[\x20-\x7e]{6,}")


def redact(text):
    return EMAIL.sub("<email>", text)


def short(text, limit=120):
    text = redact(text)
    return text if len(text) <= limit else text[:limit - 3] + "..."


# ------------------------------------------------------------------ ELF (32 and 64 bit, little endian)

class Elf:
    """Enough of an ELF file to say what it is, what it needs, exports and imports."""

    def __init__(self, data):
        self.ok = data[:4] == b"\x7fELF" and data[5:6] == b"\x01"
        self.needed, self.soname, self.exports, self.imports = [], None, [], []
        self.symtab_names, self.pie, self.kind, self.machine, self.bits = [], False, "?", "?", 0
        if not self.ok:
            return
        self.bits = 64 if data[4] == 2 else 32
        try:
            self._parse(data)
        except (struct.error, IndexError, ValueError):
            self.ok = False

    def _parse(self, d):
        b64 = self.bits == 64
        e_type, e_machine = struct.unpack_from("<HH", d, 16)
        self.kind = {2: "executable", 3: "shared object or PIE"}.get(e_type, f"type {e_type}")
        self.machine = {40: "ARM", 183: "AArch64", 62: "x86-64", 3: "x86"}.get(e_machine, f"machine {e_machine}")
        if b64:
            shoff, = struct.unpack_from("<Q", d, 40)
            shentsize, shnum, shstrndx = struct.unpack_from("<HHH", d, 58)
        else:
            shoff, = struct.unpack_from("<I", d, 32)
            shentsize, shnum, shstrndx = struct.unpack_from("<HHH", d, 46)
        secs = []
        for i in range(min(shnum, 4096)):
            at = shoff + i * shentsize
            if b64:
                name, typ, _f, _a, off, size, link, _i, _al, entsize = struct.unpack_from("<IIQQQQIIQQ", d, at)
            else:
                name, typ, _f, _a, off, size, link, _i, _al, entsize = struct.unpack_from("<IIIIIIIIII", d, at)
            secs.append((name, typ, off, size, link, entsize))

        def cstr(off):
            end = d.find(b"\0", off)
            return d[off:end if end >= 0 else off + 256].decode("utf-8", "replace")

        def syms(sec):
            _n, _t, off, size, link, ent = sec
            stroff = secs[link][2]
            out = []
            for at in range(off, off + size, ent or (24 if b64 else 16)):
                if b64:
                    st_name, info, _o, shndx = struct.unpack_from("<IBBH", d, at)
                else:
                    st_name, _v, _s, info, _o, shndx = struct.unpack_from("<IIIBBH", d, at)
                if st_name:
                    out.append((cstr(stroff + st_name), info >> 4, info & 15, shndx))
            return out

        for sec in secs:
            _n, typ, off, size, link, ent = sec
            if typ == 6:  # SHT_DYNAMIC
                stroff = secs[link][2]
                step = 16 if b64 else 8
                for at in range(off, off + size, step):
                    tag, val = struct.unpack_from("<qQ" if b64 else "<iI", d, at)
                    if tag == 0:
                        break
                    if tag == 1:
                        self.needed.append(cstr(stroff + val))
                    elif tag == 14:
                        self.soname = cstr(stroff + val)
                    elif tag == 0x6FFFFFFB and val & 0x08000000:
                        self.pie = True
            elif typ == 11:  # SHT_DYNSYM
                for name, bind, styp, shndx in syms(sec):
                    if shndx == 0:
                        self.imports.append(name)
                    elif bind in (1, 2) and styp in (1, 2):
                        self.exports.append(name)
            elif typ == 2:  # SHT_SYMTAB: present only if not stripped
                self.symtab_names = [n for n, _b, _t, _s in syms(sec)]


def strings_of(data):
    return [m.group().decode("ascii") for m in STRINGS.finditer(data)]


# ------------------------------------------------------------------ reading the partitions

class Part:
    def __init__(self, name, img, tree):
        self.name, self.img, self.tree = name, img, tree

    def is_file(self, path):
        hit = self.tree.get(path)
        return bool(hit) and not hit[0]

    def link_target(self, path):
        q = fk.quoted(path)
        if q is None:
            return None
        out, _ = self.img.batch([f"stat {q}"], timeout=120)
        for _cmd, lines in out:
            for line in lines:
                m = re.match(r'^\s*Fast link dest: "(.*)"$', line)
                if m:
                    return m.group(1)
        return None

    def read(self, path, folder, limit, hops=5):
        """(final path, bytes) of a regular file, following symlinks inside this partition, or (path, None)."""
        for _ in range(hops):
            got = fk.dump(self.img, [path], folder)
            data = None
            if path in got:
                size = os.path.getsize(got[path])
                if size <= limit:
                    with open(got[path], "rb") as fh:
                        data = fh.read()
                os.unlink(got[path])
            target = self.link_target(path)
            if target is None:
                return path, data
            path = os.path.normpath(os.path.join(os.path.dirname(path), target))
        return path, None


def open_parts(image, debugfs, say):
    parts = {n: (a, b) for n, a, b in fk.gpt_partitions(image)}
    out = {}
    for name in PARTS:
        if name not in parts:
            say(f"{name}: not in the GPT ({', '.join(parts)})")
            continue
        first, last = parts[name]
        img = fk.Image(debugfs, image, first * SECTOR)
        try:
            img.stats()
        except Stop as err:
            say(f"{name}: could not be read ({err})")
            continue
        tree, skipped = fk.walk(img)
        say(f"{name}: sectors {first} to {last}, {len(tree) - 1} entries"
            + (f" ({skipped} folders skipped for odd characters)" if skipped else ""))
        out[name] = Part(name, img, tree)
    return out


# ------------------------------------------------------------------ the sections

def instrument_hits(strs, symbols, say, examples=4):
    for label, rx in INSTRUMENTS:
        r = re.compile(rx.decode())
        hits = [s for s in strs if r.search(s)]
        syms = [s for s in symbols if r.search(s)]
        if not hits and not syms:
            say(f"  {label}: no strings, no symbols")
            continue
        say(f"  {label}: {len(hits)} strings, {len(syms)} symbols")
        for s in sorted(set(x for x in hits if len(x) <= 120), key=len)[:examples]:
            say(f"      string: {short(s)}")
        for s in sorted(set(syms), key=len)[:examples]:
            say(f"      symbol: {short(s)}")


def section_app(parts, tmp, say):
    say("\n=== 1. Akai's app ===")
    root = parts.get("rootfs")
    if not root:
        say("no rootfs partition: skipped")
        return None, []
    path, data = root.read(APP, tmp, MAX_BINARY)
    if path != APP:
        say(f"{APP} is a link to {path}")
    if data is None:
        say(f"{path}: not in the image or larger than {MAX_BINARY >> 20} MiB")
        return None, []
    elf = Elf(data)
    say(f"{path}: {len(data)} bytes ({len(data) / 1048576:.1f} MiB of file pages if fully loaded), "
        f"sha256 {hashlib.sha256(data).hexdigest()}")
    if not elf.ok:
        say("not an ELF file this tool can read")
        return None, []
    say(f"ELF{elf.bits} {elf.machine} {elf.kind}{', PIE' if elf.pie else ''}; "
        f"{'NOT stripped: ' + str(len(elf.symtab_names)) + ' symbols in .symtab' if elf.symtab_names else 'stripped'}; "
        f"{len(elf.exports)} dynamic exports, {len(elf.imports)} imports")
    say("needs (DT_NEEDED): " + (", ".join(elf.needed) or "(none: static?)"))
    wanted = [i for i in IMPORTS if i in set(elf.imports)]
    say("imports that matter for guest mode: " + (", ".join(wanted) or "(none of the listed ones)"))
    vst = [e for e in elf.exports if e in INTERESTING_EXPORTS]
    say("plugin-style exports: " + (", ".join(vst) or "(none)"))
    strs = strings_of(data)
    say("\ninstrument names in its strings and symbols:")
    instrument_hits(strs, elf.symtab_names + elf.exports, say)
    return elf, strs


def section_plugins(parts, tmp, say):
    say("\n=== 2. Plugin files and folders on rootfs, content and data ===")
    cands, dirs = [], []
    for name, part in parts.items():
        for path, (is_dir, size) in sorted(part.tree.items()):
            base = path.rsplit("/", 1)[-1]
            if is_dir:
                if PLUGIN_DIRS.match(base):
                    inside = sum(1 for p in part.tree if p.startswith(path + "/"))
                    dirs.append(f"{name}:{path} ({inside} entries)")
                continue
            if not PLUGIN_EXT.search(base):
                continue
            if name == "rootfs" and SYSTEM_LIB.match(path) and not PLUGIN_WORDS.search(base):
                continue
            cands.append((name, path, size))
    say(f"folders named like plugin folders: {len(dirs)}")
    for d in dirs[:100]:
        say(f"  {redact(d)}")
    say(f"plugin-like files: {len(cands)}" + (f" (first {MAX_CANDIDATES} read)" if len(cands) > MAX_CANDIDATES else ""))
    for name, path, size in cands[:MAX_CANDIDATES]:
        _, data = parts[name].read(path, tmp, MAX_BINARY)
        if data is None:
            say(f"  {name}:{redact(path)} ({size} bytes): could not be read (a link out of this partition?)")
            continue
        elf = Elf(data)
        if not elf.ok:
            say(f"  {name}:{redact(path)} ({size} bytes): not ELF")
            continue
        vst = [e for e in elf.exports if e in INTERESTING_EXPORTS]
        say(f"  {name}:{redact(path)} ({size} bytes) ELF{elf.bits} {elf.machine}; exports {len(elf.exports)}"
            + (f", plugin entry: {', '.join(vst)}" if vst else "")
            + (f"; needs {', '.join(elf.needed[:8])}" if elf.needed else ""))
        hits = [lbl for lbl, rx in INSTRUMENTS[:12] if re.search(rx, data)]
        if hits:
            say(f"      instrument names inside: {', '.join(hits)}")


def section_licence(parts, app_strs, say):
    say("\n=== 3. Where activation might live (names only; no file opened) ===")
    rows = []
    for name, part in parts.items():
        for path, (is_dir, size) in sorted(part.tree.items()):
            if LICENCE.search(path.rsplit("/", 1)[-1]) and not LICENCE_NOISE.search(path):
                rows.append(f"  {name}:{short(path, 200)} ({'folder' if is_dir else f'{size} bytes'})")
    say(f"file and folder names that look like licence or activation data: {len(rows)}")
    for r in rows[:300]:
        say(r)
    paths = sorted({s for s in app_strs if LICENCE.search(s) and ("/" in s or "." in s) and len(s) <= 160})
    say(f"\nstrings in Akai's app that name such paths or addresses: {len(paths)}" + (" (first 60)" if len(paths) > 60 else ""))
    for s in paths[:60]:
        say(f"  {short(s, 160)}")


def show_text(part, path, tmp, say, limit=200):
    real, data = part.read(path, tmp, 1 << 20)
    if data is None:
        say(f"--- {path}: not in the image")
        return
    if b"\0" in data[:4096]:
        say(f"--- {real}: binary, {len(data)} bytes, sha256 {hashlib.sha256(data).hexdigest()}")
        return
    lines = data.decode("utf-8", "replace").splitlines()
    say(f"--- {real}" + (f" (first {limit} of {len(lines)} lines)" if len(lines) > limit else ""))
    for line in lines[:limit]:
        say("    " + redact(line))


def section_guest(parts, elf, strs, tmp, say):
    say("\n=== 4. What guest mode needs ===")
    root = parts.get("rootfs")
    if not root:
        say("no rootfs partition: skipped")
        return
    if elf is not None:
        libs = " ".join(elf.needed)
        for lib, what in (("libasound", "audio and MIDI through alsa-lib: a private asound.conf can redirect it"),
                          ("libdrm", "screen through DRM/KMS"), ("libEGL", "EGL"), ("libGLESv2", "GLES"),
                          ("libgbm", "GBM buffers"), ("libudev", "udev device discovery"),
                          ("libinput", "libinput touch"), ("libQt", "Qt"), ("libjuce", "a shared JUCE")):
            say(f"  {lib}: {'linked' if lib in libs else 'not linked'} ({what})")
        dl = sorted({s for s in strs if re.match(r"^lib[A-Za-z0-9_+-]+\.so(\.[0-9]+)*$", s)})
        say("  library names in its strings (dlopen candidates): " + (", ".join(dl[:60]) or "(none)"))
        for label, rx, cap in (("device files", r"^/dev/[A-Za-z0-9_./%-]+$", 120),
                               ("/sys paths", r"^/sys/[A-Za-z0-9_./%-]+$", 60),
                               ("/proc paths", r"^/proc/[A-Za-z0-9_./%-]+$", 40),
                               ("ALSA device names", r"^(hw|plughw|default|dmix|dsnoop|sysdefault)(:.*)?$|ACVA", 40),
                               ("display and toolkit hints", r"(QT_QPA|eglfs|linuxfb|JUCE v[0-9]|KMS|DRM_|"
                                                             r"fbdev|/dev/fb|wayland|X11)", 40),
                               ("memory and scheduling hints", r"(?i)(mlock|memory pool|sample memory|"
                                                               r"ram ?(size|limit|pool)|affinity|SCHED_)", 40)):
            r = re.compile(rx)
            hits = sorted({s for s in strs if r.search(s) and len(s) <= 120})
            say(f"  {label}: {len(hits)}" + (f" (first {cap})" if len(hits) > cap else ""))
            for s in hits[:cap]:
                say(f"      {short(s)}")
    show_text(root, LAUNCHER, tmp, say)
    for d in UNIT_DIRS:
        if root.is_file(f"{d}/acvs.service") or f"{d}/acvs.service" in root.tree:
            show_text(root, f"{d}/acvs.service", tmp, say)
    say(f"--- /usr/share/alsa/alsa.conf: {'present' if '/usr/share/alsa/alsa.conf' in root.tree else 'absent'}")
    show_text(root, "/etc/asound.conf", tmp, say, limit=80)
    alsa_plugins = sorted(p for p in root.tree if re.match(r"^/usr/lib/alsa-lib/[^/]+$", p))
    say(f"--- ALSA plugins in /usr/lib/alsa-lib: {len(alsa_plugins)}")
    for p in alsa_plugins[:60]:
        say(f"    {p.rsplit('/', 1)[-1]}")

    say("\n--- kernel")
    kernels = sorted(p for p in root.tree if re.match(r"^/boot/zImage-[A-Za-z0-9._+-]+$", p))
    if not kernels:
        say("no /boot/zImage-* in the rootfs")
    for k in kernels:
        _, data = root.read(k, tmp, kc.MAX_KERNEL)
        if data is None:
            say(f"{k}: could not be read")
            continue
        try:
            where, text = kc.extract(data)
        except kc.ConfigError as err:
            say(f"{k}: {err}")
            continue
        say(f"{k}: config found ({where}); symbols for guest mode:")
        for line in text.splitlines():
            if KCONFIG.match(line):
                say(f"    {line}")
    mods = sorted(p for p in root.tree if p.startswith(("/lib/modules/", "/usr/lib/modules/"))
                  and KMODS.search(p.rsplit("/", 1)[-1]) and not root.tree[p][0])
    say("loadable modules for guest mode: " + (", ".join(m.rsplit("/", 1)[-1] for m in mods) or "(none)"))
    for p in sorted(root.tree):
        if re.match(r"^/(usr/)?lib/modules/[^/]+/modules\.builtin$", p):
            _, data = root.read(p, tmp, 4 << 20)
            if data:
                hits = [x for x in data.decode("utf-8", "replace").splitlines() if KMODS.search(x)]
                say(f"built into the kernel ({p}): " + (", ".join(h.rsplit("/", 1)[-1] for h in hits) or "(none)"))


def section_settings(parts, tmp, say):
    say("\n=== 5. Akai's app settings that bear on memory and CPU ===")
    say("(only keys about memory, voices, buffers, threads, plugins and content folders; keys that suggest "
        "accounts, keys, serials or activation are left out)")
    seen = 0
    for name in ("data", "content", "rootfs"):
        part = parts.get(name)
        if not part:
            continue
        for path, (is_dir, size) in sorted(part.tree.items()):
            base = path.rsplit("/", 1)[-1]
            if is_dir or size > MAX_SETTINGS or not SETTINGS_FILE.search(base) or LICENCE.search(base):
                continue
            if name == "rootfs" and not path.startswith(("/etc/", "/usr/share/Akai")):
                continue
            seen += 1
            if seen > 12:
                say("(more settings files not shown)")
                return
            _, data = part.read(path, tmp, MAX_SETTINGS)
            if data is None:
                continue
            text = data.decode("utf-8", "replace")
            pairs = re.findall(r'name="([^"]{1,80})"\s+val(?:ue)?="([^"]{0,400})"', text)
            pairs += re.findall(r"^\s*([A-Za-z0-9_. -]{1,80}?)\s*[=:]\s*(.{0,400})$", text, re.M)
            kept = [(k, v) for k, v in pairs if SETTINGS_KEEP.search(k) and not SETTINGS_DROP.search(k)]
            say(f"--- {name}:{short(path, 200)} ({size} bytes): {len(pairs)} keys, {len(kept)} shown")
            for k, v in kept[:80]:
                say(f"    {short(k, 80)} = {short(v, 80)}")


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--backup", help="backup folder (default: the newest one)")
    args = p.parse_args(argv)
    backups = os.path.realpath(os.environ.get("MPFREE_BACKUPS_DIR", os.path.expanduser("~/mpfree-backups")))
    work = os.path.realpath(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    repo = os.path.dirname(os.path.dirname(HERE))
    if work == repo or work.startswith(repo + os.sep):
        raise Stop("the work folder must be outside this clone")
    folder = os.path.realpath(args.backup) if args.backup else fk.newest_backup(backups)
    image = os.path.join(folder, "mmcblk0.img")
    if not os.path.isfile(image):
        raise Stop(f"{image} does not exist")
    debugfs = fk.find_debugfs()
    os.makedirs(work, exist_ok=True)
    out_dir = tempfile.mkdtemp(prefix=f"akai-scan-{fk.utc()}-", dir=work)
    lines = []

    def say(text=""):
        print(text, flush=True)
        lines.append(text)

    try:
        say("MPFree: read-only scan of the eMMC backup for Akai's instruments and guest mode (no MPC contact)")
        say(f"backup: {folder}")
        say(f"image: {image} ({os.path.getsize(image)} bytes, opened read-only)")
        parts = open_parts(image, debugfs, say)
        if not parts:
            raise Stop("no readable rootfs, content or data partition")
        root = parts.get("rootfs")
        if root:
            for f in ("/etc/version", "/etc/timestamp"):
                with tempfile.TemporaryDirectory(dir=out_dir) as tmp:
                    _, data = root.read(f, tmp, 4096)
                if data:
                    say(f"{f}: {short(data.decode('utf-8', 'replace').strip(), 100)}")
        with tempfile.TemporaryDirectory(dir=out_dir) as tmp:
            elf, strs = section_app(parts, tmp, say)
            section_plugins(parts, tmp, say)
            section_licence(parts, strs, say)
            section_guest(parts, elf, strs, tmp, say)
            section_settings(parts, tmp, say)
        say("\nRESULT: done. Read report.txt, then upload it to the project thread.")
        say("It names Akai's files and quotes short strings from Akai's app: keep it private, never commit it.")
        return 0
    except Stop as err:
        say(f"\nSTOP: {err}")
        return 1
    finally:
        with open(os.path.join(out_dir, "report.txt"), "w") as fh:
            fh.write("\n".join(lines) + "\n")
        print(f"report: {os.path.join(out_dir, 'report.txt')}")


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main(sys.argv[1:]))
    except Stop as err:
        print(f"STOP: {err}")
        sys.exit(1)
