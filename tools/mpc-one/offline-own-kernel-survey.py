#!/usr/bin/env python3
"""OFFLINE, workstation only: step 1 of MPFree's own kernel (docs/dev/hardware/own-kernel.md).

Reads Akai's kernel, board file and module lists out of the root filesystem image
the stick is built from, and prints what MPFree's own kernel has to keep, drop or
replace:

  1. the kernel file, /boot/zImage-<release>: its hash and its build settings
     (IKCONFIG, read as offline-kernel-config.py does), with the symbols that
     matter for our kernel first, whether modules must be signed
     (CONFIG_MODULE_SIG_FORCE), and every symbol with a vendor name in it;
  2. the board file, /boot/rk3288-az01-acva.dtb (--board): every node with a
     compatible string, its status, and which driver of Akai's kernel claims it
     (built in or a module, from the module lists' device-tree aliases), so a
     part that only Akai's own driver handles stands out;
  3. the nodes for the sound, screen, backlight, touch, USB, Ethernet and
     storage in full (offline-dtb.py show);
  4. the modules: which are built in, which are loadable, and any with a
     vendor name;
  5. the whole build configuration, the starting point for our own.

It only reads. debugfs without -w opens the image read-only; the files it reads
are copied into a private temporary folder, which is removed at the end. It
never contacts the MPC. The board file in the image carries no serial number or
MAC address (U-Boot adds those at boot), so the output holds none.

Usage:
  python3 tools/mpc-one/offline-own-kernel-survey.py [--image ROOTFS.img] [--board NAME.dtb]

Default image: the single *rootfs*.img in the newest ~/mpfree-work/akai-unpacked-*/
folder ($MPFREE_WORK_DIR), the same one offline-kexec-stick.py builds the stick from.
"""
import argparse
import importlib.util
import glob
import hashlib
import os
import re
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


config_tool = load("offline_kernel_config", "offline-kernel-config.py")
dtb = load("offline_dtb", "offline-dtb.py")
stick = load("offline_kexec_stick", "offline-kexec-stick.py")
Stop = stick.Stop

# The build settings that matter for our own kernel. read-only-drivers.sh selects the same
# symbols from /proc/config.gz on the MPC; a test keeps the two lists equal.
SELECT = re.compile(
    r"^(# )?CONFIG_(LOCALVERSION|CC_VERSION_TEXT|PREEMPT[A-Z_]*|HZ[A-Z_0-9]*|NO_HZ[A-Z_]*|HIGH_RES_TIMERS|SMP|"
    r"NR_CPUS|HIGHMEM|VMSPLIT_[0-9A-Z_]+|ARM_LPAE|VFP[A-Z0-9_]*|NEON|CPU_FREQ[A-Z_]*|ARM_RK3288_CPUFREQ|"
    r"CPU_IDLE[A-Z_]*|MODULES|MODULE_SIG[A-Z_]*|MODULE_COMPRESS[A-Z_]*|KEXEC[A-Z_]*|SECURITY_LOADPIN[A-Z_]*|"
    r"SECURITY_LOCKDOWN_LSM|LSM|IKCONFIG[A-Z_]*|CMDLINE[A-Z_]*|BLK_DEV_INITRD|MMC[A-Z0-9_]*|SND_SOC|"
    r"SND_SOC_ROCKCHIP[A-Z0-9_]*|SND_SOC_SIMPLE[A-Z_]*|SND_SIMPLE[A-Z_]*|SND_SOC_GPIO[A-Z_]*|SND_SOC_SPDIF|"
    r"SND_USB_AUDIO|SND_HRTIMER|SND_SEQ[A-Z_]*|SND_RAWMIDI|DRM|DRM_ROCKCHIP|ROCKCHIP_[A-Z0-9_]+|DRM_DW_MIPI_DSI|"
    r"DRM_PANEL[A-Z0-9_]*|DRM_PANFROST|DRM_LIMA|DRM_FBDEV_EMULATION|FB|FRAMEBUFFER_CONSOLE|BACKLIGHT_[A-Z0-9_]+|"
    r"TOUCHSCREEN_ILI210X|INPUT_TOUCHSCREEN|USB_DWC2[A-Z_]*|USB_EHCI[A-Z_]*|USB_OHCI[A-Z_]*|USB_STORAGE|USB_UAS|"
    r"STMMAC_ETH|STMMAC_PLATFORM|DWMAC_ROCKCHIP|REALTEK_PHY|CFG80211|PSTORE[A-Z_]*|ANDROID_RAM_CONSOLE|"
    r"MAGIC_SYSRQ|PRINTK_TIME|IKHEADERS|DEBUG_FS)(=| is not set)"
)
# A symbol, module or compatible string that names the vendor or this product line: these are
# the parts mainline Linux has no driver for.
VENDOR = re.compile(r"inmusic|akai|az0[0-9]|acv[a-z0-9]|jp07", re.I)
SYMBOL = re.compile(r"^(?:# )?(CONFIG_[A-Za-z0-9_]+)(?:=| is not set)")
# A device-tree alias in the module lists: of:N*T*C<compatible>, sometimes ending in C*.
OF_ALIAS = re.compile(r"^of:N\*T\*C(.+?)(?:C\*)?$")
DEFAULT_BOARD = "rk3288-az01-acva.dtb"
# The nodes printed in full: by compatible (a regular expression) or by unit address.
DETAIL_COMPATIBLE = re.compile(r"inmusic|audio|i2s|codec|dsi|panel|backlight|vop|ilitek|ili2|mali|gpu|"
                               r"dwc2|dw-mshc|mmc|gmac|ethernet-phy|pwm", re.I)
SAFE = re.compile(r"^[A-Za-z0-9@._+/=-]+$")


class Out:
    def __init__(self, stream):
        self.stream = stream

    def write(self, text):
        self.stream.write(text)

    def say(self, text=""):
        self.stream.write(text + "\n")


def default_image():
    work = stick.check_work_root(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    folder = stick.newest(os.path.join(work, "akai-unpacked-[0-9]*/"))
    if not folder:
        raise Stop(f"no unpacked update under {work}: run offline-update-extract.py first, or pass --image")
    found = glob.glob(os.path.join(folder, "*rootfs*.img"))
    if len(found) != 1:
        raise Stop(f"expected one *rootfs*.img in {folder}, found {len(found)}; pass --image")
    return found[0]


def read_optional(img, path, scratch):
    """The file's bytes, or None if the image has no regular file there."""
    if img.kind(path) != "regular":
        return None
    return img.read(path, scratch)


def module_lists(img, release, scratch):
    """(folder, {name: bytes}) for the release's module lists, or (None, {})."""
    for base in ("/usr/lib/modules/", "/lib/modules/"):
        folder = base + release
        if img.kind(folder) == "directory":
            files = {}
            for name in ("modules.builtin", "modules.order", "modules.alias", "modules.builtin.modinfo"):
                data = read_optional(img, f"{folder}/{name}", scratch)
                if data is not None:
                    files[name] = data
            return folder, files
    return None, {}


def module_name(path):
    """kernel/sound/soc/foo/snd-soc-foo.ko(.xz) -> snd_soc_foo"""
    base = os.path.basename(path.strip())
    base = re.sub(r"\.ko(\.[a-z0-9]+)?$", "", base)
    return base.replace("-", "_")


def claims(files):
    """{compatible: {module: 'built in' or 'module'}} from the device-tree aliases in the module lists."""
    found = {}

    def add(compat, module, how):
        found.setdefault(compat, {})[module] = how

    for line in files.get("modules.alias", b"").decode("utf-8", "replace").splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[0] == "alias":
            m = OF_ALIAS.match(parts[1])
            if m:
                add(m.group(1), parts[2], "module")
    for entry in files.get("modules.builtin.modinfo", b"").split(b"\0"):
        text = entry.decode("utf-8", "replace")
        name, sep, value = text.partition(".alias=")
        if sep:
            m = OF_ALIAS.match(value)
            if m:
                add(m.group(1), name, "built in")
    return found


def enabled(node):
    status = dtb.strings_of(node.prop("status") or b"")
    return not status or status[0] in ("okay", "ok")


def survey(image, board, out):
    if not os.path.isfile(image) or not SAFE.match(os.path.realpath(image)):
        raise Stop(f"cannot use {image} (missing, or an unusual path)")
    if not re.match(r"^[A-Za-z0-9._+-]+\.dtb$", board):
        raise Stop(f"--board must be a plain .dtb file name, not {board!r}")
    for tool in ("debugfs",):
        if not shutil.which(tool):
            raise Stop(f"missing '{tool}'. Install e2fsprogs with your package manager")
    img = stick.Image(os.path.realpath(image))
    scratch = tempfile.mkdtemp(prefix="mpfree-own-kernel-survey-")
    try:
        out.say("MPFREE OWN KERNEL SURVEY - OFFLINE, READ ONLY (workstation only, no MPC contact)")
        out.say(f"image: {os.path.realpath(image)} (opened read-only by debugfs)")
        kernels = [n for n in img.names("/boot") if n.startswith("zImage-")]
        if len(kernels) != 1:
            raise Stop(f"expected one /boot/zImage-<release> in the image, found {len(kernels)}")
        release = kernels[0][len("zImage-"):]
        if not re.match(r"^[A-Za-z0-9._+-]+$", release):
            raise Stop(f"unexpected kernel release in the image: {release!r}")
        kernel = img.read("/boot/" + kernels[0], scratch)
        out.say(f"kernel: /boot/zImage-{release}, {len(kernel)} bytes, sha256 {hashlib.sha256(kernel).hexdigest()}")

        out.say("\n=== 1. Build settings (IKCONFIG in the kernel file) ===")
        try:
            where, config = config_tool.extract(kernel)
        except config_tool.ConfigError as err:
            raise Stop(f"could not read the kernel's build settings: {err}") from None
        lines = config.splitlines()
        built_in = sum(1 for line in lines if re.match(r"^CONFIG_\w+=y$", line))
        modules = sum(1 for line in lines if re.match(r"^CONFIG_\w+=m$", line))
        out.say(f"found: {where}; lines: {len(lines)}, built in (=y): {built_in}, modules (=m): {modules}")
        # The same bytes as /proc/config.gz on the running kernel: read-only-drivers.sh prints the same hash.
        out.say(f"sha256 of the whole config: {hashlib.sha256(config.encode()).hexdigest()}")
        if "CONFIG_MODULE_SIG_FORCE=y" in lines:
            out.say("signed modules: REQUIRED (CONFIG_MODULE_SIG_FORCE=y)")
        elif "CONFIG_MODULE_SIG=y" in lines:
            out.say("signed modules: checked but not required (CONFIG_MODULE_SIG=y, no CONFIG_MODULE_SIG_FORCE)")
        else:
            out.say("signed modules: not checked (no CONFIG_MODULE_SIG)")
        out.say("--- the symbols that matter for our own kernel")
        for line in lines:
            if SELECT.match(line):
                out.say(line)
        out.say("--- symbols with a vendor name in them (no mainline driver)")
        vendor = [line for line in lines if SYMBOL.match(line) and VENDOR.search(SYMBOL.match(line).group(1))]
        out.say("\n".join(vendor) if vendor else "(none)")

        folder, files = module_lists(img, release, scratch)
        claimed = claims(files)

        out.say(f"\n=== 2. Board file /boot/{board}: every node with a compatible, and who claims it ===")
        blob = read_optional(img, "/boot/" + board, scratch)
        if blob is None:
            others = sorted(n for n in img.names("/boot") if n.endswith(".dtb"))
            raise Stop(f"the image has no /boot/{board}; its board files: {', '.join(others) or 'none'}")
        board_path = os.path.join(scratch, board)
        with open(board_path, "wb") as fh:
            fh.write(blob)
        tree = dtb.parse(blob)
        out.say(f"model: {dtb.model(tree)}; compatible: {' '.join(dtb.compatible(tree))}")
        if not claimed:
            out.say(f"(no device-tree aliases in the module lists{'' if folder else ': no /lib/modules/' + release}; "
                    f"the driver column is empty)")
        out.say("path  status  compatible  ->  driver (module name, built in or module)")
        detail, unclaimed = [], []
        for path, node in tree.walk():
            compat = dtb.strings_of(node.prop("compatible") or b"")
            if not compat or path == "/":
                continue
            state = "okay" if enabled(node) else "disabled"
            owners = []
            for c in compat:
                for module, how in sorted(claimed.get(c, {}).items()):
                    owners.append(f"{module} ({how}, for {c})")
            mark = ""
            if any(VENDOR.search(c) for c in compat):
                mark = "  [vendor]"
            if claimed and not owners and state == "okay":
                unclaimed.append(path)
            out.say(f"{path}  {state}  {' '.join(compat)}  ->  {'; '.join(owners) or '(none)'}{mark}")
            if DETAIL_COMPATIBLE.search(" ".join(compat)):
                detail.append(path)
        if claimed:
            out.say("--- enabled nodes no driver in this kernel claims by compatible (often simple buses, "
                    "regulators, clocks or pin groups that their parent's driver handles)")
            out.say("\n".join(unclaimed) if unclaimed else "(none)")

        out.say("\n=== 3. The nodes for sound, screen, backlight, touch, GPU, USB, Ethernet and storage, in full ===")
        dtb.show_nodes(board_path, detail, out)

        out.say(f"\n=== 4. Modules ({folder or 'no module folder in the image'}) ===")
        if folder:
            builtin = [module_name(p) for p in files.get("modules.builtin", b"").decode().splitlines() if p.strip()]
            loadable = [module_name(p) for p in files.get("modules.order", b"").decode().splitlines() if p.strip()]
            out.say(f"built in: {len(builtin)}; loadable: {len(loadable)}")
            out.say("--- with a vendor name")
            vendor_mods = sorted({m for m in builtin + loadable if VENDOR.search(m)})
            out.say("\n".join(f"{m} ({'built in' if m in builtin else 'module'})" for m in vendor_mods)
                    if vendor_mods else "(none)")
            out.say("--- loadable modules")
            out.say("\n".join(loadable) if loadable else "(none)")
            out.say("--- built in")
            out.say("\n".join(builtin) if builtin else "(none)")

        out.say("\n=== 5. Build settings, whole ===")
        out.say(config.rstrip("\n"))
        out.say("\nEND OF OWN KERNEL SURVEY")
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--image", help="the rootfs image (default: as described above)")
    ap.add_argument("--board", default=DEFAULT_BOARD, help=f"the board file in /boot (default {DEFAULT_BOARD})")
    args = ap.parse_args(argv[1:])
    out = Out(sys.stdout)
    try:
        survey(args.image or default_image(), args.board, out)
    except Stop as err:
        sys.stdout.flush()
        print(f"STOP: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
