#!/usr/bin/env python3
"""Tests for offline-akai-scan.py against a fake backup: a disk image with a real
GPT whose rootfs, content and data partitions are ext4 file systems made by
mke2fs -d. They hold made-up files (not Akai's): a fake app and plugins built
here as small ELF32 ARM files, licence files whose contents must never reach the
report, and settings with keys that must be left out.

Needs mke2fs and debugfs (e2fsprogs). They are skipped without them unless
MPFREE_REQUIRE_OFFLINE_TOOLS=1, which CI sets.
"""
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "offline-akai-scan.py")
sys.path.insert(0, HERE)
from test_offline_factory_kit import HAVE, MKE2FS, gpt_disk  # noqa: E402


def elf32(needed=(), exports=(), imports=(), symtab=(), blob=b"", pie=False, machine=40):
    """A small, valid-enough ELF32 little-endian file: .dynstr, .dynsym, .dynamic, .symtab, .strtab, .rodata."""
    def strtab(names):
        data, at = b"\0", {}
        for n in names:
            at[n] = len(data)
            data += n.encode() + b"\0"
        return data, at

    dynstr, dat = strtab(list(needed) + list(exports) + list(imports))
    dynsym = b"\0" * 16
    for n in exports:
        dynsym += struct.pack("<IIIBBH", dat[n], 0x1000, 4, (1 << 4) | 2, 0, 6)  # GLOBAL FUNC in .rodata
    for n in imports:
        dynsym += struct.pack("<IIIBBH", dat[n], 0, 0, (1 << 4) | 2, 0, 0)  # undefined
    dynamic = b"".join(struct.pack("<iI", 1, dat[n]) for n in needed)
    if pie:
        dynamic += struct.pack("<iI", 0x6FFFFFFB, 0x08000000)
    dynamic += struct.pack("<iI", 0, 0)
    strt, sat = strtab(list(symtab))
    symt = b"\0" * 16 + b"".join(struct.pack("<IIIBBH", sat[n], 0x1000, 4, (1 << 4) | 2, 0, 6) for n in symtab)
    names = [".dynstr", ".dynsym", ".dynamic", ".strtab", ".symtab", ".rodata", ".shstrtab"]
    shstr, shat = strtab(names)
    # (name, type, data, link, entsize); section 0 is the null section
    secs = [(".dynstr", 3, dynstr, 0, 0), (".dynsym", 11, dynsym, 1, 16), (".dynamic", 6, dynamic, 1, 8),
            (".strtab", 3, strt, 0, 0), (".symtab", 2, symt, 4, 16), (".rodata", 1, blob, 0, 0),
            (".shstrtab", 3, shstr, 0, 0)]
    body, offsets = b"", []
    for _n, _t, data, _l, _e in secs:
        offsets.append(52 + len(body))
        body += data + b"\0" * (-len(data) % 4)
    shoff = 52 + len(body)
    head = b"\x7fELF\x01\x01\x01" + b"\0" * 9
    head += struct.pack("<HHIIIIIHHHHHH", 3 if pie else 2, machine, 1, 0, 0, shoff, 0, 52, 0, 0, 40,
                        len(secs) + 1, len(secs))
    shdrs = b"\0" * 40
    for (n, t, data, link, ent), off in zip(secs, offsets):
        shdrs += struct.pack("<IIIIIIIIII", shat[n], t, 0, 0, off, len(data), link, 0, 4, ent)
    return head + body + shdrs


APP = elf32(needed=["libasound.so.2", "libdrm.so.2", "libEGL.so.1", "libc.so.6"],
            imports=["mlockall", "snd_pcm_open", "drmModeSetCrtc", "sched_setaffinity", "printf"],
            symtab=["_ZN4Hype9ProcessorC1Ev", "_ZN9TubeSynth5Voice6renderEv"],
            blob=b"\0Hype Factory Presets\0Mellotron M400\0/dev/dri/card0\0/dev/snd/pcmC0D0p\0hw:ACVA\0"
                 b"/media/az01-internal/Settings/Activations/inmusic.lic\0libluajit.so.2\0"
                 b"https://api.example.invalid/v1/activate\0JUCE v7.0.5\0", pie=True)
PLUGIN = elf32(needed=["libc.so.6"], exports=["VSTPluginMain", "helper"], blob=b"\0Fabric Piano\0")
OTHER_LIB = elf32(needed=["libc.so.6"], exports=["png_read"])
SECRET = "SECRET-LICENCE-CONTENT-1234"

ROOTFS = {
    "usr/bin/MPC": APP,
    "usr/bin/az01-launch-MPC": "#!/bin/sh\nexec /usr/bin/MPC --fullscreen\n",
    "lib/systemd/system/acvs.service": "[Service]\nExecStart=/usr/bin/az01-launch-MPC\nLimitMEMLOCK=infinity\n",
    "etc/version": "3.6.0.99\n",
    "usr/lib/libpng16.so.16": OTHER_LIB,
    "usr/lib/air/libAIRHype.so": PLUGIN,
    "usr/lib/alsa-lib/libasound_module_pcm_pulse.so": OTHER_LIB,
    "usr/share/licenses/zlib/LICENSE": "zlib licence text",
    "lib/modules/6.6.95/kernel/drivers/gpu/drm/vkms/vkms.ko": b"\0ko",
    "lib/modules/6.6.95/modules.builtin": "kernel/drivers/input/misc/uinput.ko\nkernel/fs/ext4/ext4.ko\n",
}
CONTENT = {
    "Synths/Vendor - VST - Thing/x.so": PLUGIN,
    "Synths/Vendor - VST - Thing/LICENSE.txt": "plain licence text",
    "Expansions/Pack/sound.wav": b"RIFF" + b"w" * 100,
}
DATA = {
    "Settings/MPC/MPC.settings": '<?xml version="1.0"?>\n<PROPERTIES>\n'
                                 '<VALUE name="sampleMemoryMB" val="512"/>\n'
                                 '<VALUE name="maxVoices" val="128"/>\n'
                                 '<VALUE name="userAccountToken" val="' + SECRET + '"/>\n'
                                 '<VALUE name="pluginList-arm" val="me@example.com list"/>\n'
                                 '<VALUE name="theme" val="dark"/>\n</PROPERTIES>\n',
    "Settings/Activations/inmusic.lic": SECRET,
    "Settings/registration.dat": SECRET,
    "etc/upper/ssh/id_ed25519": "PRIVATE KEY",
}


@unittest.skipUnless(HAVE, "needs mke2fs and debugfs")
class AkaiScanTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.backups = os.path.join(self.tmp, "backups")
        self.work = os.path.join(self.tmp, "work")
        os.makedirs(self.work)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def ext4(self, files, name, links=()):
        src = tempfile.mkdtemp(dir=self.tmp)
        for rel, data in files.items():
            p = os.path.join(src, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as fh:
                fh.write(data if isinstance(data, bytes) else data.encode())
        for rel, target in links:
            p = os.path.join(src, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            os.symlink(target, p)
        img = os.path.join(self.tmp, f"{name}.img")
        subprocess.run([MKE2FS, "-q", "-F", "-t", "ext4", "-d", src, img, "8M"], check=True)
        with open(img, "rb") as fh:
            return fh.read()

    def backup(self, rootfs=ROOTFS, links=()):
        folder = os.path.join(self.backups, "mpc-one-emmc-20261109T233604Z-abc")
        os.makedirs(folder)
        image = os.path.join(folder, "mmcblk0.img")
        gpt_disk(image, [("rootfs", 2048, self.ext4(rootfs, "rootfs", links)),
                         ("content", 20480, self.ext4(CONTENT, "content")),
                         ("data", 40960, self.ext4(DATA, "data"))])
        os.chmod(image, 0o400)
        return image

    def run_tool(self, *args):
        env = dict(os.environ, MPFREE_BACKUPS_DIR=self.backups, MPFREE_WORK_DIR=self.work)
        res = subprocess.run([sys.executable, TOOL, *args], capture_output=True, text=True, env=env, timeout=300)
        runs = sorted(d for d in os.listdir(self.work) if d.startswith("akai-scan-"))
        self.assertEqual(len(runs), 1, res.stdout + res.stderr)
        out = os.path.join(self.work, runs[0])
        with open(os.path.join(out, "report.txt")) as fh:
            report = fh.read()
        return res, out, report

    def test_scan_reports_and_keeps_secrets_out(self):
        image = self.backup()
        before = os.stat(image)
        res, out, report = self.run_tool()
        self.assertEqual(res.returncode, 0, report)
        # the app: links, imports, instrument names in strings and symbols
        self.assertIn("ELF32 ARM shared object or PIE, PIE", report)
        self.assertIn("NOT stripped: 2 symbols", report)
        self.assertIn("needs (DT_NEEDED): libasound.so.2, libdrm.so.2, libEGL.so.1, libc.so.6", report)
        self.assertIn("mlockall, sched_setaffinity, snd_pcm_open, drmModeSetCrtc", report)
        self.assertIn("Hype: 2 strings, 1 symbols", report)  # the .strtab name is a string too
        self.assertIn("symbol: _ZN9TubeSynth5Voice6renderEv", report)
        self.assertIn("Mellotron: 1 strings, 0 symbols", report)
        # plugins: the AIR-named library and the card plugin with VSTPluginMain; plain system libraries skipped
        self.assertIn("rootfs:/usr/lib/air/libAIRHype.so", report)
        self.assertIn("content:/Synths/Vendor - VST - Thing/x.so", report)
        self.assertIn("plugin entry: VSTPluginMain", report)
        self.assertIn("instrument names inside: Fabric", report)
        self.assertNotIn("libpng16", report)
        self.assertIn("content:/Synths (3 entries)", report)
        # activation: names listed, contents never; licence texts of libraries are noise
        self.assertIn("data:/Settings/Activations", report)
        self.assertIn("data:/Settings/registration.dat (27 bytes)", report)
        self.assertIn("/media/az01-internal/Settings/Activations/inmusic.lic", report)
        self.assertIn("https://api.example.invalid/v1/activate", report)
        self.assertNotIn("zlib/LICENSE", report)
        self.assertNotIn("/usr/share/licenses", report)
        self.assertNotIn("Thing/LICENSE.txt", report)
        # guest mode
        self.assertIn("libasound: linked", report)
        self.assertIn("libQt: not linked", report)
        self.assertIn("/dev/dri/card0", report)
        self.assertIn("hw:ACVA", report)
        self.assertIn("libluajit.so.2", report)
        self.assertIn("exec /usr/bin/MPC --fullscreen", report)
        self.assertIn("LimitMEMLOCK=infinity", report)
        self.assertIn("libasound_module_pcm_pulse.so", report)
        self.assertIn("loadable modules for guest mode: vkms.ko", report)
        self.assertIn("built into the kernel", report)
        self.assertIn("uinput.ko", report)
        self.assertIn("no /boot/zImage-* in the rootfs", report)
        self.assertIn("--- /etc/asound.conf: not in the image", report)
        # settings: memory and voices shown; account keys and e-mail addresses not
        self.assertIn("sampleMemoryMB = 512", report)
        self.assertIn("maxVoices = 128", report)
        self.assertIn("pluginList-arm = <email> list", report)
        self.assertNotIn("theme", report)
        self.assertNotIn("userAccountToken", report)
        for text in (SECRET, "PRIVATE KEY", "me@example.com", "id_ed25519"):
            self.assertNotIn(text, report)
            self.assertNotIn(text, res.stdout)
        # nothing left behind but the report, and the backup untouched
        self.assertEqual(os.listdir(out), ["report.txt"])
        after = os.stat(image)
        self.assertEqual((before.st_size, before.st_mtime_ns), (after.st_size, after.st_mtime_ns))

    def test_app_behind_a_link_is_followed(self):
        rootfs = dict(ROOTFS)
        rootfs["usr/bin/MPC-3.6.0"] = rootfs.pop("usr/bin/MPC")
        self.backup(rootfs, links=[("usr/bin/MPC", "MPC-3.6.0")])
        res, _, report = self.run_tool()
        self.assertEqual(res.returncode, 0, report)
        self.assertIn("/usr/bin/MPC is a link to /usr/bin/MPC-3.6.0", report)
        self.assertIn("Hype: 2 strings, 1 symbols", report)

    def test_rootfs_image_without_a_backup(self):
        rootfs = dict(ROOTFS, **{"etc/version": "3.10.1.7\n"})
        img = os.path.join(self.tmp, "updated-rootfs.img")
        with open(img, "wb") as fh:
            fh.write(self.ext4(rootfs, "rootfs-new"))
        res, _, report = self.run_tool("--rootfs-image", img)
        self.assertEqual(res.returncode, 0, report)
        self.assertIn(f"rootfs image: {img}", report)
        self.assertIn("no backup: content and data are not scanned", report)
        self.assertIn("/etc/version: 3.10.1.7", report)
        self.assertIn("Hype: 2 strings, 1 symbols", report)
        self.assertNotIn("content:", report)

    def test_newest_unpacked_rootfs_replaces_the_backups(self):
        self.backup()
        unpacked = os.path.join(self.work, "akai-unpacked-20261115T050000Z-xyz")
        os.makedirs(unpacked)
        with open(os.path.join(unpacked, "az01-rootfs.img"), "wb") as fh:
            fh.write(self.ext4(dict(ROOTFS, **{"etc/version": "3.10.1.7\n"}), "rootfs-new"))
        res, _, report = self.run_tool("--rootfs-image", "newest")
        self.assertEqual(res.returncode, 0, report)
        self.assertIn("/etc/version: 3.10.1.7", report)
        self.assertIn("rootfs: from ", report)
        self.assertIn("content:/Synths/Vendor - VST - Thing/x.so", report)
        self.assertIn("data:/Settings/registration.dat", report)

    def test_no_backup_stops_cleanly(self):
        env = dict(os.environ, MPFREE_BACKUPS_DIR=self.backups, MPFREE_WORK_DIR=self.work)
        res = subprocess.run([sys.executable, TOOL], capture_output=True, text=True, env=env, timeout=60)
        self.assertEqual(res.returncode, 1)
        self.assertIn("no backup folder", res.stdout)


if __name__ == "__main__":
    unittest.main()
