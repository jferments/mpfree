#!/usr/bin/env python3
"""OFFLINE, workstation only: build the files for a test boot of another Akai
firmware version (such as 3.10.0) by kexec, with its root filesystem on a USB
stick and the MPC's eMMC and SD card hidden from it.

It never contacts the MPC and never writes a device. It reads Akai's files
from ~/mpfree-work and writes one new private folder there:

  ~/mpfree-work/kexec-stick-<UTC>-XXXXXX/
    stick.img     disk image for the USB stick (GPT): partition 1 holds a
                  modified COPY of the version's root filesystem, partition 2
                  is an empty log partition the test boot writes its logs to;
                  only with --app, partition 3 holds the drum kits (read-only)
                  and partition 4 ("mpfree-test-data", MPFREE-DATA) is an
                  empty journaled ext4 where the pad player saves its
                  settings and files; write-test-stick.py carries the old
                  stick's partition 4 over
    zImage        the version's kernel, unchanged
    kexec.dtb     its device tree for this unit: Akai's board file plus what
                  U-Boot adds at boot, with the eMMC and SD card controllers
                  disabled (see offline-dtb.py kexec-dtb)
    cmdline.txt   the kernel command line; root is partition 1 of the stick
    known_hosts   the test boot's SSH host key, made here, for strict checking
    SHA256SUMS    hashes of the five files above
    report.txt    everything this tool printed

What changes in the COPY of the root filesystem (the image itself is only read):
  - masks (links to /dev/null in /etc/systemd/system) for the MPC app, the
    touch, XMOS and USB-SATA firmware updaters, the network update server,
    the USB script runner, the /data and content formatters and the firewall,
    and for the generator that mounts the eMMC content partition
  - /data becomes a RAM disk, so the /etc and /var overlays live in RAM
  - Akai's update, flashing and U-Boot-environment programs and kexec are
    replaced by a stub that refuses and notes the call in the kernel log
  - kexec is turned off in the new kernel (sysctl, and on the command line)
  - key-only root SSH on port 2222: a host key made here and your public key
  - a service that copies the boot log, failed units and network state to
    the stick's log partition every 10 seconds (it writes only that partition)
  - a service that shows the live log on the MPC's screen: our program
    mpfree-screen-log (rust/mpfree-screen-log, built for ARM beforehand) reads
    `journalctl -f` and draws it on /dev/fb0, turned by the panel's rotation
    from the device tree; it writes only the framebuffer
  - only with --splash: a startup screen first. Our program mpfree-splash
    (rust/mpfree-splash, built for ARM beforehand) draws the galaxy on
    /dev/fb0 for 6 seconds, in the --splash-font font if one is given, and
    the log display starts when it ends; it writes only the framebuffer
  - only with --app: the pad player starts at power-on instead of the log
    display. Our program mpfree-hello (rust/mpfree-hello, built for ARM
    beforehand) runs `app`: a loading screen (over the galaxy, with --splash)
    with a progress bar, then the pad player. SHIFT + STOP shows the system
    log on the screen. Its drum kits (kits.bin from offline-factory-kit.py)
    go on partition 3, which it mounts read-only. Partition 4 is mounted
    read-write (nosuid, nodev, noexec) at /run/mpfree-data and passed with
    --data; the pad player saves its settings and files only there
    (docs/project/storage-spec.md), and if it does not mount, nothing is
    saved. If the program stops with an error, the log display starts
    instead. Besides the screen, the controller, the sound card and
    partition 4, it writes only /run/mpfree-hello (RAM)
  - a service that records the Ethernet details (read-only-ethernet.sh, the
    same script collect-over-ssh.sh --ethernet runs on 3.6) before the network
    starts and after, and if eth0 failed to start, starts it again at 20, 45
    and 90 seconds; it changes only RAM (it mounts debugfs if systemd has not)
    and writes only /run/mpfree-test-eth, which the log service copies
  - root's password field, only if it marks the account locked ("!"), is set
    to "*" so OpenSSH accepts key logins; no password works either way

Inputs, found automatically:
  the rootfs        the single *rootfs*.img in the newest akai-unpacked-*/
                    folder, or --image FILE
  boot-files        ~/mpfree-work/boot-files/<release>/ (from
                    offline-rootfs-survey.sh) for the image's kernel release
                    and for the release running on the MPC
  the running tree  live.dtb in the newest kexec-preflight-*/ folder (from
                    collect-over-ssh.sh --kexec-preflight)
  your public key   $MPC_KEY.pub (default /home/jesse/.ssh/mpc_one_autorun_ed25519.pub)
  the screen log    rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-screen-log
                    in this clone (or $MPFREE_SCREEN_LOG): a static ARM EABI5
                    hard-float executable, built with
                    CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
                    RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static" \
                    cargo build --manifest-path rust/Cargo.toml --release --locked \
                      --target armv7-unknown-linux-gnueabihf -p mpfree-screen-log

Optional inputs:
  --splash FILE       the startup-screen program (mpfree-splash), a static ARM
                      EABI5 hard-float executable built like the screen log
  --splash-font FILE  a TrueType or OpenType font for its title (only with
                      --splash); without it the title uses built-in letters
  --app FILE          the pad player (mpfree-hello), built like the screen log
  --kits FILE         its drum kits (only with --app); default: kits.bin in
                      the newest ~/mpfree-work/factory-kit-*/ folder. They are
                      Akai's sounds: the stick stays private

The device tree also gets the Ethernet PHY's ID (Realtek RTL8211E, as 3.6.0
reads it on the reference unit). 3.6.0 holds the PHY in reset when it shuts
down for the jump, and without the ID the new kernel reads it from the PHY
before releasing that reset, finds nothing and has no Ethernet.
"""
import argparse
import glob
import hashlib
import importlib.util
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(os.path.dirname(HERE))
SPEC = importlib.util.spec_from_file_location("offline_dtb", os.path.join(HERE, "offline-dtb.py"))
dtb = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(dtb)

DEFAULT_KEY = "/home/jesse/.ssh/mpc_one_autorun_ed25519"
HOST_ALIAS = "mpfree-kexec-test"   # known_hosts name; ssh uses it through HostKeyAlias
LINUX_FS_TYPE = "0FC63DAF-8483-4772-8E79-3D69D8477DE4"
SECTOR = 512
ALIGN = 2048                        # 1 MiB, in sectors
LOG_SIZE = 32 << 20
DATA_SIZE = 64 << 20                # partition 4, the pad player's saved settings and files
DATA_NAME = "mpfree-test-data"      # its GPT name; write-test-stick.py looks for it
ETHERNET = "/ethernet@ff290000"
# The PHY's ID as 3.6.0 reads it on the reference unit (/sys/bus/mdio_bus/devices/stmmac-0:01/phy_id
# 0x001cc915, Realtek RTL8211E; docs/hardware/ethernet-usb.md).
PHY_ID_COMPATIBLE = "ethernet-phy-id001c.c915"
PANEL = "/mipi@ff960000/panel@0"
SCREEN_LOG_BUILT = os.path.join(REPO_ROOT, "rust", "target", "armv7-unknown-linux-gnueabihf", "release",
                                "mpfree-screen-log")
SCREEN_LOG_MAX = 16 << 20
SAFE = re.compile(r"^[A-Za-z0-9@._+/=-]+$")
SAFE_UNIT = re.compile(r"^[A-Za-z0-9@._\\-]+$")

# Units the test boot must never start. A mask for a unit the image lacks is harmless.
MASK_UNITS = (
    "acvs.service",                  # the MPC app: content, and controller firmware updates (DFU)
    "touch-fw-update.service",       # flashes the touch controller
    "xmos-update.service",           # flashes XMOS USB audio devices
    "az01-usbsata-fixer.service",    # writes the configuration of JMicron USB-SATA bridges
    "az0x-webserver.service",        # takes update images over the network
    "az01-script-runner.service",    # runs scripts found on USB drives
    "az0x-data-mkfs.service",        # formats the /data partition
    "acvs-content-resize.service",   # resizes the content partition
    "iptables.service",              # Akai's firewall rules, unknown here: they might block port 2222
    "ip6tables.service",
)
MASK_GENERATORS = ("acva-emmc-content-generator",)
# Programs replaced by the stub, where present. Paths under /usr only: /bin, /sbin
# and /lib are links into /usr on these images (checked below).
STUB_PROGRAMS = (
    "/usr/sbin/az01-update", "/usr/sbin/az01config", "/usr/sbin/midifirmup",
    "/usr/sbin/az0x-data-mkfs", "/usr/sbin/acvs-content-resize", "/usr/sbin/kexec",
    "/usr/bin/dfu-util", "/usr/bin/touch-fw-upd", "/usr/bin/xmosdfu", "/usr/bin/fw_setenv",
    "/usr/bin/az01-usbsata-fixer", "/usr/bin/az0x-webserver", "/usr/bin/az01-launch-MPC",
    "/usr/bin/kexec",
)
REQUIRED = {"/usr/sbin/sshd": "regular", "/usr/lib/systemd/system/data.mount": "regular",
            "/etc/systemd/system": "directory", "/etc/systemd/system/multi-user.target.wants": "directory",
            "/etc/shadow": "regular", "/bin/sh": None}
MADE_IF_MISSING = ("/etc/systemd/system-generators", "/etc/sysctl.d", "/etc/systemd/system/sysinit.target.wants")

STUB = """#!/bin/sh
# MPFree kexec test boot: this program is disabled on the test stick.
echo "mpfree-test: blocked $0 $*" > /dev/kmsg 2>/dev/null
echo "mpfree-test: $0 is disabled on this test boot" >&2
exit 1
"""
DATA_MOUNT = """[Unit]
Description=MPFree test boot: /data in RAM (tmpfs), never the eMMC
Conflicts=umount.target
Before=local-fs.target umount.target

[Mount]
What=tmpfs
Where=/data
Type=tmpfs
Options=mode=0755,size=256M

[Install]
WantedBy=local-fs.target
"""
SYSCTL = "# MPFree kexec test boot: no further kexec from this kernel.\nkernel.kexec_load_disabled = 1\n"
SSHD_CONFIG = """# MPFree kexec test boot: key-only root SSH, as the 3.6.0 USB autorun set it up.
Port 2222
AddressFamily inet
ListenAddress 0.0.0.0
HostKey /etc/mpfree-test/host_ed25519
PidFile /run/mpfree-test-sshd.pid
AuthorizedKeysFile /etc/mpfree-test/authorized_keys
PermitRootLogin prohibit-password
PubkeyAuthentication yes
PasswordAuthentication no
PermitEmptyPasswords no
ChallengeResponseAuthentication no
UsePAM no
AllowUsers root
StrictModes yes
LogLevel VERBOSE
X11Forwarding no
AllowTcpForwarding no
AllowAgentForwarding no
PermitTunnel no
Subsystem sftp internal-sftp
"""
SSHD_UNIT = """[Unit]
Description=MPFree test boot: key-only root SSH on port 2222
After=network.target

[Service]
Type=simple
RuntimeDirectory=sshd
RuntimeDirectoryMode=0755
ExecStartPre=/usr/sbin/sshd -t -f /etc/mpfree-test/sshd_config
ExecStart=/usr/sbin/sshd -D -e -f /etc/mpfree-test/sshd_config
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
"""
LOG_SCRIPT = """#!/bin/sh
# MPFree kexec test boot: copy logs to the stick's log partition (and nothing else).
dev=/dev/disk/by-partuuid/{log_uuid}
i=0
while [ ! -e "$dev" ] && [ "$i" -lt 60 ]; do sleep 1; i=$((i + 1)); done
[ -e "$dev" ] || {{ echo "mpfree-test: log partition not found" > /dev/kmsg; exit 1; }}
mkdir -p /run/mpfree-log
mount -t ext4 -o rw,nosuid,nodev,noexec,sync "$dev" /run/mpfree-log || exit 1
d=/run/mpfree-log/boot-$(cat /proc/sys/kernel/random/boot_id)
mkdir -p "$d"
while :; do
  dmesg > "$d/dmesg.txt" 2>&1
  journalctl -b --no-pager -o short-monotonic > "$d/journal.txt" 2>&1
  systemctl --failed --no-pager > "$d/failed-units.txt" 2>&1
  systemctl list-units --no-pager > "$d/units.txt" 2>&1
  {{ ip addr; ip route; }} > "$d/network.txt" 2>&1
  cat /proc/cmdline > "$d/cmdline.txt" 2>&1
  cp /run/mpfree-test-eth/*.txt "$d/" 2>/dev/null
  cp /run/mpfree-hello/hello.txt "$d/" 2>/dev/null
  sync
  sleep 10
done
"""
# Like the log service: started early and kept through a switch to emergency mode.
SCREEN_UNIT = """[Unit]
Description=MPFree test boot: show the live log on the MPC's screen
DefaultDependencies=no
After=systemd-journald.service{after}
Conflicts=shutdown.target
Before=shutdown.target
IgnoreOnIsolate=yes
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart=/bin/sh -c 'journalctl -b -f -n 100 --no-pager --no-hostname -o short-monotonic 2>&1 | /etc/mpfree-test/mpfree-screen-log --rotation {rotation}'
Restart=always
RestartSec=5
Nice=10

[Install]
WantedBy=sysinit.target
"""
# Only with --splash. The log display waits for it (After= in its unit); if it fails, the log simply starts.
SPLASH_SECONDS = 6
SPLASH_UNIT = """[Unit]
Description=MPFree test boot: startup screen before the live log
DefaultDependencies=no
Conflicts=shutdown.target
Before=shutdown.target mpfree-test-screen.service
IgnoreOnIsolate=yes

[Service]
Type=oneshot
ExecStart=/etc/mpfree-test/mpfree-splash --framebuffer --rotation {rotation} --seconds {seconds}{font}
TimeoutStartSec={timeout}

[Install]
WantedBy=sysinit.target
"""
# Only with --app, in place of the log display (which starts if the app fails). The kits partition
# is mounted read-only; the app finds the screen, the controller and the sound card itself, showing
# its progress on a loading screen.
APP_SCRIPT = """#!/bin/sh
# MPFree test boot: start the pad player. Its drum kits are on the stick's partition 3, mounted read-only.
# Partition 4 holds its saved settings and files, mounted read-write; it saves nothing anywhere else.
dev=/dev/disk/by-partuuid/{kits_uuid}
i=0
while [ ! -e "$dev" ] && [ "$i" -lt 30 ]; do sleep 1; i=$((i + 1)); done
mkdir -p /run/mpfree-kits
if ! grep -q ' /run/mpfree-kits ' /proc/mounts; then
  mount -t ext4 -o ro,nosuid,nodev,noexec "$dev" /run/mpfree-kits ||
    echo "mpfree-test-app: the kit partition did not mount" > /dev/kmsg
fi
data=/dev/disk/by-partuuid/{data_uuid}
i=0
while [ ! -e "$data" ] && [ "$i" -lt 5 ]; do sleep 1; i=$((i + 1)); done
mkdir -p /run/mpfree-data
if grep -q ' /run/mpfree-data ' /proc/mounts ||
    mount -t ext4 -o rw,nosuid,nodev,noexec,noatime "$data" /run/mpfree-data; then
  exec /etc/mpfree-test/mpfree-hello app --kits /run/mpfree-kits/kits.bin{keep} --data /run/mpfree-data
fi
echo "mpfree-test-app: the data partition did not mount, so nothing is saved" > /dev/kmsg
exec /etc/mpfree-test/mpfree-hello app --kits /run/mpfree-kits/kits.bin{keep}
"""
APP_UNIT = """[Unit]
Description=MPFree test boot: the pad player, from power-on
DefaultDependencies=no
After=systemd-journald.service systemd-udevd.service{after}
Conflicts=shutdown.target
Before=shutdown.target
IgnoreOnIsolate=yes
OnFailure=mpfree-test-screen.service

[Service]
Type=simple
ExecStart=/bin/sh /etc/mpfree-test/start-app.sh

[Install]
WantedBy=sysinit.target
"""
KITS_MAX = 48 << 20
KITS_MAGIC = b"MPFKITS1"
# Started early and kept through a switch to emergency mode, so a boot that goes wrong still leaves logs.
LOG_UNIT = """[Unit]
Description=MPFree test boot: copy logs to the USB stick's log partition
DefaultDependencies=no
After=systemd-journald.service systemd-udevd.service
Conflicts=shutdown.target
Before=shutdown.target
IgnoreOnIsolate=yes

[Service]
Type=simple
ExecStart=/bin/sh /etc/mpfree-test/save-logs.sh

[Install]
WantedBy=sysinit.target
"""

# Records the Ethernet details (tools/mpc-one/read-only-ethernet.sh, copied in as
# ethernet-details.sh) early, before the network starts, and later; and if eth0 failed to
# start, starts it again a few times. It changes only RAM: it mounts debugfs if systemd has
# not, and runs `ip link set eth0 up`. It writes only to /run/mpfree-test-eth, which the log
# service copies to the stick.
ETHERNET_SCRIPT = """#!/bin/sh
# MPFree kexec test boot: record the Ethernet details; start eth0 again if it failed to start.
out=/run/mpfree-test-eth
mkdir -p "$out"
say() { echo "mpfree-test-ethernet: $*"; }
grep -q ' /sys/kernel/debug debugfs ' /proc/mounts || mount -t debugfs debugfs /sys/kernel/debug
uptime_s() { cut -d. -f1 /proc/uptime; }
wait_until() { while [ "$(uptime_s)" -lt "$1" ]; do sleep 1; done; }
details() { sh /etc/mpfree-test/ethernet-details.sh > "$out/ethernet-$1.txt" 2>&1; say "recorded ethernet-$1.txt"; }
eth0_up() { f=$(cat /sys/class/net/eth0/flags 2>/dev/null) || return 1; [ $((f & 1)) = 1 ]; }
details "1-early-$(uptime_s)s"
wait_until 15
details 2-at-15s
n=3
for at in 20 45 90; do
  wait_until "$at"
  if eth0_up; then
    say "eth0 is up at $at s (operstate $(cat /sys/class/net/eth0/operstate)); no retry needed"
    details "$n-at-${at}s-eth0-up"
    exit 0
  fi
  say "eth0 is not up at $at s; starting it again"
  ip link set eth0 up 2>&1 | sed 's/^/mpfree-test-ethernet: ip: /'
  sleep 3
  details "$n-at-${at}s-after-retry"
  n=$((n + 1))
done
say "eth0 retries done"
"""
ETHERNET_UNIT = """[Unit]
Description=MPFree test boot: record the Ethernet details, and start eth0 again if it failed
DefaultDependencies=no
After=systemd-journald.service sys-kernel-debug.mount
Conflicts=shutdown.target
Before=shutdown.target
IgnoreOnIsolate=yes

[Service]
Type=simple
ExecStart=/bin/sh /etc/mpfree-test/ethernet-retry.sh

[Install]
WantedBy=sysinit.target
"""
ETHERNET_DETAILS = os.path.join(HERE, "read-only-ethernet.sh")


class Stop(Exception):
    pass


class Report:
    def __init__(self):
        self.lines = []

    def write(self, text):
        sys.stdout.write(text)
        sys.stdout.flush()
        self.lines.append(text)

    def say(self, text=""):
        self.write(text + "\n")


OUT = Report()


def sha256(path, start=0, length=None):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        fh.seek(start)
        left = length
        while left is None or left > 0:
            chunk = fh.read(1 << 20 if left is None else min(1 << 20, left))
            if not chunk:
                break
            h.update(chunk)
            if left is not None:
                left -= len(chunk)
    if left:
        raise Stop(f"{path} is shorter than expected")
    return h.hexdigest()


def run(cmd, **kw):
    return subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, **kw)


def quote(path):
    """A path for a debugfs command line. Names are checked first, so quotes suffice."""
    if '"' in path or "\n" in path or " " in path:
        raise Stop(f"refusing an unusual path: {path!r}")
    return f'"{path}"'


class Image:
    """An ext4 image, through debugfs. Without -w, debugfs opens it read-only."""

    def __init__(self, path):
        self.path = path

    def cmd(self, command):
        res = run(["debugfs", "-R", command, self.path])
        # debugfs prints its version banner on stderr; the rest is the answer.
        return "\n".join(line for line in res.stdout.splitlines() if not line.startswith("debugfs "))

    def kind(self, path):
        m = re.search(r"^Inode: \d+\s+Type: (\w+)", self.cmd(f"stat {quote(path)}"), re.M)
        return m.group(1) if m else None

    def link(self, path):
        m = re.search(r'^Fast link dest: "(.*)"$', self.cmd(f"stat {quote(path)}"), re.M)
        return m.group(1) if m else None

    def stat(self, path):
        text = self.cmd(f"stat {quote(path)}")
        mode = re.search(r"Mode:\s+(\d+)", text)
        owner = re.search(r"User:\s+(\d+)\s+Group:\s+(\d+)", text)
        return (int(mode.group(1), 8) if mode else None,
                (int(owner.group(1)), int(owner.group(2))) if owner else None)

    def read(self, path, scratch):
        target = os.path.join(scratch, "read.tmp")
        if os.path.exists(target):
            os.unlink(target)
        run(["debugfs", "-R", f"dump {quote(path)} {quote(target)}", self.path])
        if not os.path.exists(target):
            raise Stop(f"could not read {path} from the image")
        with open(target, "rb") as fh:
            data = fh.read()
        os.unlink(target)
        return data

    def names(self, directory):
        out = []
        for line in self.cmd(f"ls -p {quote(directory)}").splitlines():
            parts = line.split("/")
            if len(parts) >= 7 and parts[5] not in (".", ".."):
                out.append(parts[5])
        return out

    def free(self):
        text = self.cmd("stats")
        blocks = re.search(r"^Free blocks:\s+(\d+)", text, re.M)
        inodes = re.search(r"^Free inodes:\s+(\d+)", text, re.M)
        size = re.search(r"^Block size:\s+(\d+)", text, re.M)
        if not (blocks and inodes and size):
            raise Stop("debugfs could not read the image's superblock (not ext2/3/4?)")
        return int(blocks.group(1)) * int(size.group(1)), int(inodes.group(1))


def phy_node(tree):
    """The Ethernet PHY node the MAC points to, checked to be the layout this fix is for."""
    gmac = tree.find(ETHERNET)
    handle = gmac.prop("phy-handle") if gmac is not None else None
    path = dtb.phandles(tree).get(struct.unpack(">I", handle)[0]) if handle and len(handle) == 4 else None
    node = tree.find(path) if path else None
    if node is None or not path.startswith(ETHERNET + "/"):
        raise Stop(f"the new board file's {ETHERNET} has no phy-handle to a PHY node under it")
    if node.prop("reg") != struct.pack(">I", 1) or node.prop("reset-gpios") is None or \
            dtb.strings_of(node.prop("compatible") or b"") != ["ethernet-phy-ieee802.3-c22"]:
        raise Stop(f"the PHY node {path} is not at address 1 with compatible \"ethernet-phy-ieee802.3-c22\" "
                   "and a reset-gpios line; this tool does not know this layout")
    return path


def panel_rotation(tree):
    """The panel's rotation property (degrees counter-clockwise), or 0 without one."""
    node = tree.find(PANEL)
    value = node.prop("rotation") if node is not None else None
    if value is None:
        return 0
    degrees = struct.unpack(">I", value)[0] if len(value) == 4 else None
    if degrees not in (0, 90, 180, 270):
        raise Stop(f"{PANEL}/rotation is not 0, 90, 180 or 270")
    return degrees


def screen_log_program():
    """Our screen log program, checked to be a static ARM EABI5 hard-float executable. Returns (path, bytes)."""
    path = os.path.realpath(os.environ.get("MPFREE_SCREEN_LOG", SCREEN_LOG_BUILT))
    if not os.path.isfile(path):
        raise Stop(f"missing {path}: build the screen log program first (see the top of this file, or the docs)")
    return path, static_arm_program(path)


FONT_MAGIC = (b"\x00\x01\x00\x00", b"OTTO", b"true")


def splash_inputs(args):
    """The startup-screen program and its font, if asked for. Returns (program bytes, font name, font bytes)."""
    if args.splash_font and not args.splash:
        raise Stop("--splash-font needs --splash")
    if not args.splash:
        return None, None, None
    path = os.path.realpath(args.splash)
    if not os.path.isfile(path):
        raise Stop(f"missing {path}: build the startup-screen program first")
    program = static_arm_program(path)
    if not args.splash_font:
        return program, None, None
    font_path = os.path.realpath(args.splash_font)
    with open(font_path, "rb") as fh:
        font = fh.read(SCREEN_LOG_MAX + 1)
    ext = os.path.splitext(font_path)[1].lower()
    if len(font) > SCREEN_LOG_MAX or font[:4] not in FONT_MAGIC or ext not in (".ttf", ".otf"):
        raise Stop(f"{font_path} is not a single TrueType or OpenType font (.ttf or .otf) under 16 MiB")
    return program, "splash-font" + ext, font


def app_inputs(args, work):
    """The pad player and its kits, if asked for. Returns (program path, bytes, kits path, kits bytes)."""
    if args.kits and not args.app:
        raise Stop("--kits needs --app")
    if not args.app:
        return None, None, None, None
    path = os.path.realpath(args.app)
    if not os.path.isfile(path):
        raise Stop(f"missing {path}: build the pad player (mpfree-hello) first")
    program = static_arm_program(path)
    kits = args.kits
    if not kits:
        folders = sorted(glob.glob(os.path.join(work, "factory-kit-[0-9]*", "kits.bin")))
        if not folders:
            raise Stop(f"no drum kits under {work}: run offline-factory-kit.py first, or pass --kits")
        kits = folders[-1]
    kits = os.path.realpath(kits)
    with open(kits, "rb") as fh:
        data = fh.read(KITS_MAX + 1)
    if len(data) > KITS_MAX or len(data) < 12 or data[:8] != KITS_MAGIC:
        raise Stop(f"{kits} is not a kits file made by offline-factory-kit.py, or is over {KITS_MAX} bytes")
    return path, program, kits, data


def make_kits_fs(kits, scratch):
    """A small ext4 filesystem (no journal, so a read-only mount never writes) holding only /kits.bin,
    owned by root and read back. Returns its path."""
    path = os.path.join(scratch, "kits.img")
    size_k = (len(kits) + (8 << 20)) // (1 << 20) * 1024
    res = run(["mke2fs", "-q", "-F", "-t", "ext4", "-O", "^has_journal", "-L", "MPFREE-KITS",
               "-E", "root_owner=0:0", path, f"{size_k}k"])
    if res.returncode != 0:
        raise Stop("mke2fs failed for the kits partition:\n" + res.stdout)
    local = os.path.join(scratch, "kits.bin")
    with open(local, "wb") as fh:
        fh.write(kits)
    commands = [f"write {quote(local)} /kits.bin", "sif /kits.bin mode 0100644", "sif /kits.bin uid 0",
                "sif /kits.bin gid 0"]
    script = os.path.join(scratch, "debugfs-kits.txt")
    with open(script, "w") as fh:
        fh.write("\n".join(commands) + "\n")
    res = run(["debugfs", "-w", "-f", script, path])
    expected = {f"debugfs: {c}" for c in commands}
    for line in res.stdout.splitlines():
        if not line.strip() or line.startswith("debugfs ") or line in expected or \
                re.match(r"^Allocated inode: \d+$", line):
            continue
        raise Stop(f"debugfs reported a problem in the kits partition: {line}")
    fs = Image(path)
    if res.returncode != 0 or fs.kind("/kits.bin") != "regular" or fs.read("/kits.bin", scratch) != kits or \
            fs.stat("/kits.bin") != (0o644, (0, 0)):
        raise Stop("the kits partition did not read back as written")
    fsck = run(["e2fsck", "-fn", path])
    if fsck.returncode != 0:
        raise Stop("e2fsck found problems in the kits partition:\n" + fsck.stdout)
    os.unlink(local)
    return path


def make_data_fs(scratch):
    """Partition 4: an empty ext4 with a journal (the pad player writes it, and the MPC is switched off
    without unmounting it), labelled MPFREE-DATA, owned by root. Returns its path."""
    path = os.path.join(scratch, "data.img")
    res = run(["mke2fs", "-q", "-F", "-t", "ext4", "-L", "MPFREE-DATA", "-E", "root_owner=0:0", path,
               f"{DATA_SIZE // 1024}k"])
    if res.returncode != 0:
        raise Stop("mke2fs failed for the data partition:\n" + res.stdout)
    with open(path, "rb") as fh:
        fh.seek(1024)
        sb = fh.read(1024)
    if os.path.getsize(path) != DATA_SIZE or sb[0x38:0x3A] != b"\x53\xef" or \
            sb[0x78:0x88].rstrip(b"\0") != b"MPFREE-DATA" or not struct.unpack_from("<I", sb, 0x5C)[0] & 0x4:
        raise Stop("the data partition is not the empty journaled ext4 asked for")
    fsck = run(["e2fsck", "-fn", path])
    if fsck.returncode != 0:
        raise Stop("e2fsck found problems in the data partition:\n" + fsck.stdout)
    return path


def static_arm_program(path):
    """The file's bytes, checked to be a static ARM EABI5 hard-float executable."""
    with open(path, "rb") as fh:
        data = fh.read(SCREEN_LOG_MAX + 1)
    problem = None
    if len(data) > SCREEN_LOG_MAX or len(data) < 52 or data[:7] != b"\x7fELF\x01\x01\x01":
        problem = "not a 32-bit little-endian ELF file of a plausible size"
    else:
        e_type, e_machine = struct.unpack_from("<HH", data, 16)
        phoff, = struct.unpack_from("<I", data, 28)
        flags, = struct.unpack_from("<I", data, 36)
        phentsize, phnum = struct.unpack_from("<HH", data, 42)
        if e_type != 2 or e_machine != 40:
            problem = "not an ARM executable"
        elif flags & 0xFF000000 != 0x05000000 or not flags & 0x400:
            problem = "not EABI5 hard-float"
        elif phentsize < 32 or phoff + phnum * phentsize > len(data):
            problem = "its program headers run past the end"
        elif any(struct.unpack_from("<I", data, phoff + i * phentsize)[0] in (2, 3) for i in range(phnum)):
            problem = "dynamically linked (it must be static: 3.10.0's C library may differ)"
    if problem:
        raise Stop(f"{path} is {problem}; rebuild it as shown at the top of this file")
    return data


def newest(pattern):
    found = sorted(glob.glob(pattern))
    return found[-1] if found else None


def check_work_root(work):
    work = os.path.realpath(work)
    if (work + "/").startswith(REPO_ROOT + "/"):
        ignored = subprocess.run(["git", "-C", REPO_ROOT, "check-ignore", "-q", "--", os.path.join(work, "probe")])
        if ignored.returncode != 0:
            raise Stop(f"MPFREE_WORK_DIR is inside the git clone and not git-ignored: {work}")
    if not SAFE.match(work):
        raise Stop(f"MPFREE_WORK_DIR must not contain spaces, quotes or other unusual characters: {work}")
    return work


def checked_copy(folder, name):
    """A file from a boot-files folder, checked against that folder's SHA256SUMS."""
    sums = os.path.join(folder, "SHA256SUMS")
    want = None
    with open(sums) as fh:
        for line in fh:
            parts = line.split()
            if len(parts) == 2 and parts[1] == name:
                want = parts[0]
    path = os.path.join(folder, name)
    if want is None or not os.path.isfile(path) or sha256(path) != want:
        raise Stop(f"{path} is missing or does not match {sums}")
    return path


def same_board(live_tree, folder):
    want = dtb.compatible(live_tree)
    hits = [p for p in sorted(glob.glob(os.path.join(folder, "*.dtb"))) if dtb.compatible(dtb.load(p)) == want]
    if len(hits) != 1:
        raise Stop(f"expected one board file in {folder} with the running tree's compatible list "
                   f"({', '.join(want)}), found {len(hits)}")
    return hits[0]


def find_inputs(args, work):
    image = args.image
    if not image:
        folder = newest(os.path.join(work, "akai-unpacked-[0-9]*/"))
        if not folder:
            raise Stop(f"no unpacked update under {work}: run offline-update-extract.py first, or pass --image")
        found = glob.glob(os.path.join(folder, "*rootfs*.img"))
        if len(found) != 1:
            raise Stop(f"expected one *rootfs*.img in {folder}, found {len(found)}; pass --image")
        image = found[0]
    image = os.path.realpath(image)
    if not (os.path.isfile(image) and SAFE.match(image)):
        raise Stop(f"cannot use {image} (missing, or an unusual path)")
    pre = newest(os.path.join(work, "kexec-preflight-[0-9]*/"))
    if not pre or not os.path.isfile(os.path.join(pre, "live.dtb")):
        raise Stop(f"no running device tree under {work}: run collect-over-ssh.sh --kexec-preflight first")
    return image, os.path.join(pre, "live.dtb"), pre


def running_release(pre, work, target_release):
    path = os.path.join(pre, "release.txt")
    if os.path.isfile(path):
        with open(path) as fh:
            release = fh.read().strip()
        how = "recorded by the preflight"
    else:
        others = [os.path.basename(p.rstrip("/")) for p in glob.glob(os.path.join(work, "boot-files", "*/"))]
        others = [r for r in others if r != target_release]
        if len(others) != 1:
            raise Stop("cannot tell which release the MPC runs: rerun collect-over-ssh.sh --kexec-preflight "
                       "(it records the release now)")
        release, how = others[0], "the only other boot-files release; the preflight predates release.txt"
    if not SAFE.match(release) or release == target_release:
        raise Stop(f"unusable running release {release!r}")
    return release, how


def build_rootfs(img, scratch, pubkey, host_key, log_uuid, screen_log, rotation, splash=(None, None, None),
                 app=None, kits_uuid=None, data_uuid=None):
    """Change the copy with debugfs -w, then read every change back. Returns the root-account note.

    With `app` (the pad player's bytes), `kits_uuid` (partition 3) and `data_uuid` (partition 4), the
    pad player starts at power-on in place of the log display, which stays installed for when the app fails."""
    if not ((app is None) == (kits_uuid is None) == (data_uuid is None)):
        raise Stop("the pad player needs its kits and data partitions, and the other way round")
    for top in ("/bin", "/sbin", "/lib"):
        kind = img.kind(top)
        if kind not in ("symlink", None) or (kind == "symlink" and img.link(top) not in (f"usr{top}", f"/usr{top}")):
            raise Stop(f"{top} is not a link into /usr ({kind}); the stub paths would miss programs there")
    for path, kind in REQUIRED.items():
        found = img.kind(path)
        if found is None or (kind and found != kind):
            raise Stop(f"the image has no {kind or 'file'} {path}; this tool does not know this layout")
    splash_program, font_name, font = splash
    extra = len(splash_program or b"") + len(font or b"") + len(app or b"")
    free_bytes, free_inodes = img.free()
    if free_bytes < len(screen_log) + extra + (4 << 20) or free_inodes < 64:
        raise Stop(f"the image has too little room for the changes ({free_bytes} bytes, {free_inodes} inodes free)")
    new_paths = (["/etc/systemd/system/" + u for u in MASK_UNITS] +
                 ["/etc/systemd/system/data.mount", "/etc/systemd/system/mpfree-test-sshd.service",
                  "/etc/systemd/system/mpfree-test-logs.service", "/etc/mpfree-test",
                  "/etc/systemd/system/mpfree-test-screen.service",
                  "/etc/systemd/system/mpfree-test-ethernet.service",
                  "/etc/systemd/system/sysinit.target.wants/mpfree-test-ethernet.service",
                  "/etc/systemd/system/multi-user.target.wants/mpfree-test-sshd.service",
                  "/etc/systemd/system/sysinit.target.wants/mpfree-test-logs.service",
                  "/etc/systemd/system/sysinit.target.wants/mpfree-test-screen.service",
                  "/etc/systemd/system/mpfree-test-app.service",
                  "/etc/systemd/system/sysinit.target.wants/mpfree-test-app.service",
                  "/etc/sysctl.d/90-mpfree-test.conf"] +
                 ["/etc/systemd/system-generators/" + g for g in MASK_GENERATORS] +
                 (["/etc/systemd/system/mpfree-test-splash.service",
                   "/etc/systemd/system/sysinit.target.wants/mpfree-test-splash.service"] if splash_program else []))
    for path in new_paths:
        if img.kind(path) is not None:
            raise Stop(f"{path} already exists in the image; refusing to guess what it is")

    files = {}   # image path -> (bytes, mode)

    def add(path, data, mode):
        files[path] = (data.encode() if isinstance(data, str) else data, mode)

    add("/etc/systemd/system/data.mount", DATA_MOUNT, 0o644)
    add("/etc/sysctl.d/90-mpfree-test.conf", SYSCTL, 0o644)
    add("/etc/mpfree-test/sshd_config", SSHD_CONFIG, 0o600)
    add("/etc/mpfree-test/authorized_keys", pubkey, 0o600)
    with open(host_key, "rb") as fh:
        add("/etc/mpfree-test/host_ed25519", fh.read(), 0o600)
    add("/etc/mpfree-test/save-logs.sh", LOG_SCRIPT.format(log_uuid=log_uuid), 0o755)
    add("/etc/systemd/system/mpfree-test-sshd.service", SSHD_UNIT, 0o644)
    add("/etc/systemd/system/mpfree-test-logs.service", LOG_UNIT, 0o644)
    add("/etc/mpfree-test/mpfree-screen-log", screen_log, 0o755)
    add("/etc/systemd/system/mpfree-test-screen.service",
        SCREEN_UNIT.format(rotation=rotation, after=" mpfree-test-splash.service" if splash_program else ""), 0o644)
    if splash_program:
        add("/etc/mpfree-test/mpfree-splash", splash_program, 0o755)
        font_args = ""
        if font:
            add(f"/etc/mpfree-test/{font_name}", font, 0o644)
            font_args = f" --font /etc/mpfree-test/{font_name} --tracking 0.15"
        add("/etc/systemd/system/mpfree-test-splash.service",
            SPLASH_UNIT.format(rotation=rotation, seconds=SPLASH_SECONDS, font=font_args,
                               timeout=SPLASH_SECONDS + 14), 0o644)
    if app is not None:
        add("/etc/mpfree-test/mpfree-hello", app, 0o755)
        add("/etc/mpfree-test/start-app.sh",
            APP_SCRIPT.format(kits_uuid=kits_uuid, data_uuid=data_uuid,
                              keep=" --keep-screen" if splash_program else ""), 0o755)
        add("/etc/systemd/system/mpfree-test-app.service",
            APP_UNIT.format(after=" mpfree-test-splash.service" if splash_program else ""), 0o644)
    with open(ETHERNET_DETAILS, "rb") as fh:
        add("/etc/mpfree-test/ethernet-details.sh", fh.read(), 0o755)
    add("/etc/mpfree-test/ethernet-retry.sh", ETHERNET_SCRIPT, 0o755)
    add("/etc/systemd/system/mpfree-test-ethernet.service", ETHERNET_UNIT, 0o644)
    stubbed, absent = [], []
    for prog in STUB_PROGRAMS:
        kind = img.kind(prog)
        if kind is None:
            absent.append(prog)
        elif kind in ("regular", "symlink"):
            stubbed.append(prog)
            add(prog, STUB, 0o755)
        else:
            raise Stop(f"{prog} is a {kind}; refusing to replace it")

    shadow = img.read("/etc/shadow", scratch).decode("utf-8", "strict")
    shadow_mode = img.stat("/etc/shadow")[0]
    lines = shadow.split("\n")
    root_lines = [i for i, line in enumerate(lines) if line.startswith("root:")]
    if len(root_lines) != 1:
        raise Stop("expected one root line in /etc/shadow")
    fields = lines[root_lines[0]].split(":")
    if len(fields) < 2:
        raise Stop("the root line in /etc/shadow is malformed")
    if fields[1].startswith("!"):
        fields[1] = "*"
        lines[root_lines[0]] = ":".join(fields)
        add("/etc/shadow", "\n".join(lines), shadow_mode & 0o7777)
        root_note = 'root account was marked locked ("!"); set to "*" on the copy (key login only, no password)'
    else:
        root_note = "root account is not marked locked; /etc/shadow unchanged"

    links = {"/etc/systemd/system/" + u: "/dev/null" for u in MASK_UNITS}
    links.update({"/etc/systemd/system-generators/" + g: "/dev/null" for g in MASK_GENERATORS})
    links["/etc/systemd/system/multi-user.target.wants/mpfree-test-sshd.service"] = \
        "/etc/systemd/system/mpfree-test-sshd.service"
    links["/etc/systemd/system/sysinit.target.wants/mpfree-test-logs.service"] = \
        "/etc/systemd/system/mpfree-test-logs.service"
    if app is None:
        links["/etc/systemd/system/sysinit.target.wants/mpfree-test-screen.service"] = \
            "/etc/systemd/system/mpfree-test-screen.service"
    else:
        links["/etc/systemd/system/sysinit.target.wants/mpfree-test-app.service"] = \
            "/etc/systemd/system/mpfree-test-app.service"
    links["/etc/systemd/system/sysinit.target.wants/mpfree-test-ethernet.service"] = \
        "/etc/systemd/system/mpfree-test-ethernet.service"
    if splash_program:
        links["/etc/systemd/system/sysinit.target.wants/mpfree-test-splash.service"] = \
            "/etc/systemd/system/mpfree-test-splash.service"

    commands = ["mkdir /etc/mpfree-test"]
    for folder in MADE_IF_MISSING:
        kind = img.kind(folder)
        if kind is None:
            commands.append(f"mkdir {folder}")
        elif kind != "directory":
            raise Stop(f"{folder} is a {kind}, not a folder")
    for n, (path, (data, mode)) in enumerate(sorted(files.items())):
        local = os.path.join(scratch, f"file{n}")
        with open(local, "wb") as fh:
            fh.write(data)
        if img.kind(path) is not None:
            commands.append(f"rm {quote(path)}")
        commands += [f"write {quote(local)} {quote(path)}", f"sif {quote(path)} mode 0{0o100000 | mode:o}",
                     f"sif {quote(path)} uid 0", f"sif {quote(path)} gid 0"]
    for path, target in sorted(links.items()):
        commands.append(f"symlink {quote(path)} {target}")
    script = os.path.join(scratch, "debugfs-commands.txt")
    with open(script, "w") as fh:
        fh.write("\n".join(commands) + "\n")
    res = run(["debugfs", "-w", "-f", script, img.path])
    expected = {f"debugfs: {c}" for c in commands}
    for line in res.stdout.splitlines():
        if not line.strip() or line.startswith("debugfs ") or line in expected or \
                re.match(r"^Allocated inode: \d+$", line):
            continue
        raise Stop(f"debugfs reported a problem: {line}")
    if res.returncode != 0:
        raise Stop(f"debugfs -w failed (rc {res.returncode})")

    # Read every change back.
    for path, (data, mode) in files.items():
        got_mode, owner = img.stat(path)
        if img.kind(path) != "regular" or img.read(path, scratch) != data or \
                got_mode != mode or owner != (0, 0):
            raise Stop(f"{path} did not read back as written")
    for path, target in links.items():
        if img.kind(path) != "symlink" or img.link(path) != target:
            raise Stop(f"{path} is not a link to {target}")
    fsck = run(["e2fsck", "-fn", img.path])
    if fsck.returncode != 0:
        raise Stop("e2fsck found problems in the changed copy:\n" + fsck.stdout)
    return root_note, stubbed, absent, sorted(links), sorted(files)


def make_stick(fs_image, out_path, root_uuid, log_uuid, scratch, kits_image=None, kits_uuid=None,
               data_image=None, data_uuid=None):
    fs_bytes = os.path.getsize(fs_image)
    if fs_bytes % SECTOR:
        raise Stop("the rootfs image size is not a whole number of sectors")
    root_sectors = -(-fs_bytes // SECTOR // ALIGN) * ALIGN
    log_sectors = LOG_SIZE // SECTOR
    log_img = os.path.join(scratch, "log.img")
    res = run(["mke2fs", "-q", "-F", "-t", "ext4", "-L", "MPFREE-LOG", "-E", "root_owner=0:0", log_img,
               f"{LOG_SIZE // 1024}k"])
    if res.returncode != 0:
        raise Stop("mke2fs failed for the log partition:\n" + res.stdout)
    root_start, log_start = ALIGN, ALIGN + root_sectors
    kits_start = log_start + log_sectors
    kits_sectors = -(-os.path.getsize(kits_image) // SECTOR // ALIGN) * ALIGN if kits_image else 0
    if (kits_image is None) != (data_image is None):
        raise Stop("the kits and data partitions come together (with --app)")
    data_start = kits_start + kits_sectors
    data_sectors = DATA_SIZE // SECTOR if data_image else 0
    total = data_start + data_sectors + ALIGN
    with open(out_path, "xb") as fh:
        fh.truncate(total * SECTOR)
    table = (f"label: gpt\nunit: sectors\nfirst-lba: {ALIGN}\n"
             f"start={root_start}, size={root_sectors}, type={LINUX_FS_TYPE}, uuid={root_uuid}, "
             f"name=\"mpfree-test-root\"\n"
             f"start={log_start}, size={log_sectors}, type={LINUX_FS_TYPE}, uuid={log_uuid}, "
             f"name=\"mpfree-test-log\"\n")
    if kits_image:
        table += (f"start={kits_start}, size={kits_sectors}, type={LINUX_FS_TYPE}, uuid={kits_uuid}, "
                  f"name=\"mpfree-test-kits\"\n"
                  f"start={data_start}, size={data_sectors}, type={LINUX_FS_TYPE}, uuid={data_uuid}, "
                  f"name=\"{DATA_NAME}\"\n")
    res = subprocess.run(["sfdisk", "-q", "--no-reread", "--no-tell-kernel", out_path], input=table,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    if res.returncode != 0:
        raise Stop("sfdisk failed on the image file:\n" + res.stdout)
    copies = [(fs_image, root_start), (log_img, log_start)] + \
        ([(kits_image, kits_start), (data_image, data_start)] if kits_image else [])
    for src, start in copies:
        with open(src, "rb") as fin, open(out_path, "r+b") as fout:
            fout.seek(start * SECTOR)
            shutil.copyfileobj(fin, fout, 1 << 20)
    res = run(["sfdisk", "--json", out_path])
    parts = json.loads(res.stdout)["partitiontable"]["partitions"] if res.returncode == 0 else []
    want = [(root_start, root_sectors, root_uuid), (log_start, log_sectors, log_uuid)]
    if kits_image:
        want += [(kits_start, kits_sectors, kits_uuid), (data_start, data_sectors, data_uuid)]
    got = [(p["start"], p["size"], p["uuid"].lower()) for p in parts]
    if got != want or any(p["type"].upper() != LINUX_FS_TYPE for p in parts):
        raise Stop(f"the stick image's partition table is not what was asked for: {got}")
    if sha256(out_path, root_start * SECTOR, fs_bytes) != sha256(fs_image):
        raise Stop("partition 1 of the stick image does not match the changed rootfs")
    if sha256(out_path, log_start * SECTOR, LOG_SIZE) != sha256(log_img):
        raise Stop("partition 2 of the stick image does not match the log filesystem")
    if kits_image and sha256(out_path, kits_start * SECTOR, os.path.getsize(kits_image)) != sha256(kits_image):
        raise Stop("partition 3 of the stick image does not match the kits filesystem")
    if data_image and (parts[3].get("name") != DATA_NAME or
                       sha256(out_path, data_start * SECTOR, DATA_SIZE) != sha256(data_image)):
        raise Stop("partition 4 of the stick image does not match the data filesystem")
    return total * SECTOR


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--image", help="the version's rootfs image (default: found as described above)")
    ap.add_argument("--splash", help="the startup-screen program (mpfree-splash, static ARM)")
    ap.add_argument("--splash-font", help="a .ttf or .otf font for the startup screen's title (with --splash)")
    ap.add_argument("--app", help="the pad player (mpfree-hello, static ARM), started at power-on")
    ap.add_argument("--kits", help="its drum kits (kits.bin; default: the newest factory-kit-* folder's)")
    args = ap.parse_args(argv)
    for tool in ("debugfs", "e2fsck", "mke2fs", "sfdisk", "ssh-keygen"):
        if not shutil.which(tool):
            raise Stop(f"missing '{tool}'. Install e2fsprogs, util-linux (fdisk) and openssh with your package manager")
    work = check_work_root(os.environ.get("MPFREE_WORK_DIR", os.path.expanduser("~/mpfree-work")))
    image, live_path, pre = find_inputs(args, work)
    key = os.environ.get("MPC_KEY", DEFAULT_KEY)
    pub_path = key + ".pub"
    if not os.path.isfile(pub_path):
        raise Stop(f"missing your public key {pub_path} (the SSH key used for the MPC, plus .pub)")
    with open(pub_path) as fh:
        pubkey = fh.read().strip() + "\n"
    if len(pubkey.splitlines()) != 1 or run(["ssh-keygen", "-l", "-f", pub_path]).returncode != 0:
        raise Stop(f"{pub_path} is not a single public key")
    screen_path, screen_log = screen_log_program()
    splash = splash_inputs(args)
    app_path, app, kits_path, kits = app_inputs(args, work)

    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    os.makedirs(work, exist_ok=True)
    outdir = tempfile.mkdtemp(prefix=f"kexec-stick-{stamp}-", dir=work)
    scratch = os.path.join(outdir, "tmp")
    os.mkdir(scratch, 0o700)
    ok = False
    try:
        OUT.say("MPFree kexec test-boot builder (workstation only, no MPC contact, no device writes)")
        OUT.say(f"output folder: {outdir}")
        need = 2 * os.path.getsize(image) + 3 * len(kits or b"") + (256 << 20)
        if shutil.disk_usage(outdir).free < need:
            raise Stop(f"not enough free space in {work}: {need >> 20} MiB needed")
        source_sha = sha256(image)
        OUT.say(f"source rootfs: {image}")
        OUT.say(f"  sha256 {source_sha} (only read)")
        copy = os.path.join(scratch, "rootfs.img")
        shutil.copyfile(image, copy)
        if sha256(copy) != source_sha:
            raise Stop("the copy of the rootfs does not match the source")
        img = Image(copy)
        kernels = [n for n in img.names("/boot") if n.startswith("zImage-")]
        if len(kernels) != 1 or not SAFE.match(kernels[0]):
            raise Stop(f"expected one /boot/zImage-<release> in the image, found {len(kernels)}")
        release = kernels[0][len("zImage-"):]
        OUT.say(f"its kernel release: {release}")

        live = dtb.load(live_path)
        board = dtb.compatible(live)
        OUT.say(f"running tree: {live_path} ({dtb.model(live)}; {', '.join(board)})")
        running, how = running_release(pre, work, release)
        OUT.say(f"running release: {running} ({how})")
        new_folder = os.path.join(work, "boot-files", release)
        old_folder = os.path.join(work, "boot-files", running)
        for folder in (new_folder, old_folder):
            if not os.path.isfile(os.path.join(folder, "SHA256SUMS")):
                raise Stop(f"no {folder}/SHA256SUMS: run offline-rootfs-survey.sh (and --from-backup) first")
        target = same_board(live, new_folder)
        reference = same_board(live, old_folder)
        zimage = checked_copy(new_folder, "zImage-" + release)
        checked_copy(new_folder, os.path.basename(target))
        checked_copy(old_folder, os.path.basename(reference))
        for name, path in (("/boot/zImage-" + release, zimage), ("/boot/" + os.path.basename(target), target)):
            if hashlib.sha256(img.read(name, scratch)).hexdigest() != sha256(path):
                raise Stop(f"{path} is not the image's {name}: boot-files belong to another build")
        OUT.say(f"board file for the new kernel: {target}")
        OUT.say(f"board file of the running release: {reference}")
        OUT.say()

        OUT.say("=== Device tree for the kexec boot (names only, no values) ===")
        drops = []
        target_tree = dtb.load(target)
        chosen = target_tree.find("/chosen")
        if chosen is not None and chosen.prop("inmusic,acvs-emmc-content") is not None:
            drops = ["--drop", "/chosen/inmusic,acvs-emmc-content"]  # tells Akai's units the content is on eMMC
        phy = phy_node(target_tree)
        rotation = panel_rotation(target_tree)
        dtb_out = os.path.join(outdir, "kexec.dtb")
        rc = dtb.kexec_dtb_main(["--target", target, "--reference", reference, "--live", live_path,
                                 "--out", dtb_out, *drops, "--add-compatible", f"{phy}={PHY_ID_COMPATIBLE}"], OUT)
        if rc != 0:
            raise Stop("offline-dtb.py kexec-dtb stopped; see above")
        if dtb.strings_of(dtb.load(dtb_out).find(phy).prop("compatible"))[:1] != [PHY_ID_COMPATIBLE]:
            raise Stop(f"{phy} in kexec.dtb does not start with {PHY_ID_COMPATIBLE}")
        OUT.say(f"Ethernet PHY {phy}: the kernel takes its ID ({PHY_ID_COMPATIBLE}) from the tree, then resets it")
        OUT.say(f"panel rotation (from {PANEL}): {rotation} degrees; the screen log is drawn to match")
        OUT.say()

        OUT.say("=== Changes to the copy of the root filesystem ===")
        host_key = os.path.join(scratch, "host_ed25519")
        res = run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", HOST_ALIAS, "-f", host_key])
        if res.returncode != 0:
            raise Stop("ssh-keygen failed:\n" + res.stdout)
        root_uuid, log_uuid = str(uuid.uuid4()), str(uuid.uuid4())
        OUT.say(f"screen log program: {screen_path} ({len(screen_log)} bytes, "
                f"sha256 {hashlib.sha256(screen_log).hexdigest()}, static ARM EABI5 hard-float)")
        if splash[0]:
            OUT.say(f"startup screen program: {os.path.realpath(args.splash)} ({len(splash[0])} bytes, "
                    f"sha256 {hashlib.sha256(splash[0]).hexdigest()}, static ARM EABI5 hard-float), "
                    f"shown for {SPLASH_SECONDS} s before the log")
            if splash[2]:
                OUT.say(f"startup screen font: {os.path.realpath(args.splash_font)} ({len(splash[2])} bytes, "
                        f"sha256 {hashlib.sha256(splash[2]).hexdigest()})")
            else:
                OUT.say("startup screen font: none given, so the title uses built-in letters")
        else:
            OUT.say("startup screen: none (no --splash)")
        kits_uuid = str(uuid.uuid4()) if app else None
        data_uuid = str(uuid.uuid4()) if app else None
        if app:
            OUT.say(f"pad player: {app_path} ({len(app)} bytes, sha256 {hashlib.sha256(app).hexdigest()}, "
                    "static ARM EABI5 hard-float), started at power-on in place of the log display"
                    + (", over the startup screen" if splash[0] else ""))
            OUT.say(f"drum kits: {kits_path} ({struct.unpack_from('<I', kits, 8)[0]} kits, {len(kits)} bytes, "
                    f"sha256 {hashlib.sha256(kits).hexdigest()}), on partition 3, mounted read-only")
            OUT.say(f"saved settings and files: partition 4 ({DATA_SIZE >> 20} MiB, empty here), mounted "
                    "read-write at /run/mpfree-data; write-test-stick.py carries the old stick's over")
        else:
            OUT.say("pad player: none (no --app); the log display starts at power-on")
        root_note, stubbed, absent, links, written = build_rootfs(img, scratch, pubkey, host_key, log_uuid,
                                                                  screen_log, rotation, splash, app, kits_uuid,
                                                                  data_uuid)
        for path in links:
            OUT.say(f"link     {path} -> {img.link(path)}")
        for path in written:
            OUT.say(f"written  {path}" + ("  (stub)" if path in stubbed else ""))
        for path in absent:
            OUT.say(f"absent   {path} (nothing to stub)")
        OUT.say(root_note)
        OUT.say("e2fsck -fn: clean")
        OUT.say(f"changed rootfs sha256 {sha256(copy)}")
        OUT.say()

        OUT.say("=== Stick image ===")
        stick = os.path.join(outdir, "stick.img")
        kits_image = make_kits_fs(kits, scratch) if app else None
        data_image = make_data_fs(scratch) if app else None
        size = make_stick(copy, stick, root_uuid, log_uuid, scratch, kits_image, kits_uuid, data_image, data_uuid)
        OUT.say(f"stick.img: {size} bytes ({-(-size // (1 << 20))} MiB); partition 1 root PARTUUID {root_uuid}, "
                f"partition 2 logs PARTUUID {log_uuid}")
        if kits_image:
            OUT.say(f"partition 3 kits PARTUUID {kits_uuid} (ext4 without a journal, only /kits.bin, "
                    f"sha256 {sha256(kits_image)})")
            OUT.say(f"partition 4 data PARTUUID {data_uuid} ({DATA_NAME}, ext4 with a journal, label "
                    f"MPFREE-DATA, {DATA_SIZE >> 20} MiB, empty)")
        shutil.copyfile(zimage, os.path.join(outdir, "zImage"))
        cmdline = (f"root=PARTUUID={root_uuid} rootwait=60 ro panic=10 rfkill.default_state=0 "
                   "sysctl.kernel.kexec_load_disabled=1")
        with open(os.path.join(outdir, "cmdline.txt"), "w") as fh:
            fh.write(cmdline + "\n")
        with open(host_key + ".pub") as fh:
            host_pub = fh.read().split()
        with open(os.path.join(outdir, "known_hosts"), "w") as fh:
            fh.write(f"{HOST_ALIAS} {host_pub[0]} {host_pub[1]}\n")
        OUT.say(f"kernel command line: {cmdline}")
        OUT.say("(the kernel appends its built-in isolcpus=2-3)")
        fp = run(["ssh-keygen", "-l", "-f", host_key + ".pub"]).stdout.strip()
        OUT.say(f"test boot host key: {fp}")
        with open(os.path.join(outdir, "SHA256SUMS"), "w") as fh:
            for name in ("stick.img", "zImage", "kexec.dtb", "cmdline.txt", "known_hosts"):
                fh.write(f"{sha256(os.path.join(outdir, name))}  {name}\n")
        OUT.say()
        OUT.say(f"DONE. Files in {outdir}")
        OUT.say("Nothing was written to any device. The stick image holds a test SSH host key and")
        OUT.say("your public key, and the folder quotes Akai's files" + (" and holds Akai's sounds" if app else "")
                + ": keep it private, never commit it.")
        ok = True
        return 0
    except (Stop, dtb.DtbError, OSError) as err:
        OUT.say(f"STOP: {err}")
        return 1
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
        with open(os.path.join(outdir, "report.txt"), "w") as fh:
            fh.write("".join(OUT.lines))
        if not ok:
            for name in ("stick.img", "kexec.dtb", "zImage", "cmdline.txt", "known_hosts", "SHA256SUMS"):
                path = os.path.join(outdir, name)
                if os.path.exists(path):
                    os.unlink(path)
            print(f"No stick image was made. Report: {os.path.join(outdir, 'report.txt')}")


if __name__ == "__main__":
    os.umask(0o077)
    os.environ["PATH"] = os.environ.get("PATH", "/usr/bin:/bin") + ":/usr/sbin:/sbin"
    try:
        sys.exit(main(sys.argv[1:]))
    except (Stop, dtb.DtbError, OSError) as err:
        print(f"STOP: {err}")
        sys.exit(1)
