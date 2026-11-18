#!/usr/bin/env python3
"""Tests for collect-over-ssh.sh --sd-card against a fake MPC. No device, no network.

As in test_read_only_drivers.py: ip, ping and ssh are stand-ins, and the fake ssh runs the real
read-only-sd-card.sh under BusyBox applets (as on the MPC) against a fake /proc, /sys, /run and
/etc tree, with the kernel log and the journal read from files in that tree. The card is an
exFAT SD card mounted by Akai's disk service, beside the eMMC, which the survey must leave out.

Set MPFREE_REQUIRE_BUSYBOX=1 (CI does) to fail instead of skip without BusyBox.
"""
import gzip
import os
import re
import shutil
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_kexec_preflight import write  # noqa: E402
from test_read_only_drivers import Base, link  # noqa: E402

TOOLS = os.path.dirname(HERE)
REMOTE = os.path.join(TOOLS, "read-only-sd-card.sh")
SECRET_SERIAL = "0x1a2b3c4d"
SECRET_VOLUME_ID = "1A2B-3C4D"
SECRET_UUID = "0a1b2c3d-4e5f-4a6b-8c7d-0e1f2a3b4c5d"
SECRET_DISK_ID = "931ad49d"
SECRET_MAC = "02:11:22:33:44:55"
EXTRA_APPLETS = ("awk", "ls", "tail")

CONFIG = """CONFIG_MMC=y
CONFIG_MMC_BLOCK=y
CONFIG_MMC_DW=y
CONFIG_MMC_DW_ROCKCHIP=y
CONFIG_VFAT_FS=y
CONFIG_FAT_DEFAULT_IOCHARSET="iso8859-1"
CONFIG_EXFAT_FS=y
# CONFIG_NTFS3_FS is not set
CONFIG_EXT4_FS=y
CONFIG_NLS_UTF8=y
CONFIG_SND_SOC=y
"""

KERNEL_LOG = f"""[    1.100000] mmc0: new HS200 MMC card at address 0001
[    1.110000] mmcblk0: mmc0:0001 8GTF4R 7.28 GiB
[    1.300000] dwmmc_rockchip ff0c0000.mmc: Successfully tuned phase to 162
[    1.310000] mmc1: new high speed SDXC card at address aaaa
[    1.320000] mmcblk1: mmc1:aaaa SD64G 59.5 GiB
[    1.330000]  mmcblk1: p1
[    1.400000] rk_gmac-dwmac ff290000.ethernet eth0: device MAC address {SECRET_MAC}
[    9.000000] exFAT-fs (mmcblk1p1): Volume was not properly unmounted. Some data may be corrupt.
"""

JOURNAL = f"""[    9.100000] az01 edisksd[512]: found /dev/mmcblk1p1 type exfat uuid {SECRET_VOLUME_ID}
[    9.200000] az01 edisksd[512]: mounted /dev/mmcblk1p1 at /media/{SECRET_VOLUME_ID}
[    9.300000] az01 systemd[1]: Started unrelated.service.
[    9.400000] az01 edisksd[512]: disk {SECRET_UUID} ignored
"""

UDEV_DISK = f"""S:disk/by-id/mmc-SD64G_{SECRET_SERIAL}
E:ID_NAME=SD64G
E:ID_SERIAL=SD64G_{SECRET_SERIAL}
E:ID_PART_TABLE_UUID={SECRET_DISK_ID}
E:ID_PART_TABLE_TYPE=dos
E:ID_DRIVE_FLASH_SD=1
"""

UDEV_PART = f"""E:ID_FS_UUID={SECRET_VOLUME_ID}
E:ID_FS_UUID_ENC={SECRET_VOLUME_ID}
E:ID_FS_VERSION=1.0
E:ID_FS_TYPE=exfat
E:ID_FS_USAGE=filesystem
E:ID_FS_LABEL=MPC_SD
E:ID_PART_ENTRY_UUID={SECRET_DISK_ID}-01
E:ID_PART_ENTRY_TYPE=0x7
E:ID_PART_ENTRY_NUMBER=1
E:SYSTEMD_WANTS=edisksd-mount@mmcblk1p1.service
"""


def fake_root(root, card=True, debugfs=True):
    def put(rel, data):
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        write(path, data)

    put("kernel-log", KERNEL_LOG)
    put("journal", JOURNAL)
    put("proc/sys/kernel/osrelease", "6.18.44-az01-fake-rt6\n")
    put("proc/filesystems", "nodev\tsysfs\nnodev\tproc\n\text4\n\tvfat\n\texfat\nnodev\tfuse\n\tfuseblk\n")
    put("proc/config.gz", gzip.compress(CONFIG.encode()))
    dt = "sys/firmware/devicetree/base/mmc@ff0c0000"
    put(f"{dt}/compatible", b"rockchip,rk3288-dw-mshc\0")
    put(f"{dt}/status", b"okay\0")
    for prop in ("cd-gpios", "cap-sd-highspeed", "bus-width", "max-frequency"):
        put(f"{dt}/{prop}", b"\0\0\0\1")
    put("sys/firmware/devicetree/base/mmc@ff0f0000/non-removable", b"")
    plat = "sys/devices/platform/ff0c0000.mmc"
    os.makedirs(os.path.join(root, plat, "mmc_host/mmc1"))
    os.makedirs(os.path.join(root, "sys/bus/platform/drivers/dwmmc_rockchip"))
    link("../../../bus/platform/drivers/dwmmc_rockchip", os.path.join(root, plat, "driver"))
    link("../../../devices/platform/ff0c0000.mmc", os.path.join(root, "sys/bus/platform/devices/ff0c0000.mmc"))
    # The eMMC, on its own controller: never the card.
    emmc = "sys/devices/platform/ff0f0000.mmc/mmc_host/mmc0/mmc0:0001/block/mmcblk0"
    put(f"{emmc}/size", "15269888\n")
    link(f"../{emmc[len('sys/'):]}", os.path.join(root, "sys/block/mmcblk0"))
    mounts = "proc /proc proc rw 0 0\n/dev/mmcblk0p8 /data ext4 rw,relatime 0 0\n"
    if debugfs:
        mounts += "debugfs /sys/kernel/debug debugfs rw,nosuid,nodev,noexec,relatime 0 0\n"
        put("sys/kernel/debug/mmc1/ios", "clock:\t\t50000000 Hz\nbus width:\t2 (4 bits)\n"
                                         "timing spec:\t2 (sd high-speed)\nsignal voltage:\t0 (3.30 V)\n")
    if card:
        dev = f"{plat}/mmc_host/mmc1/mmc1:aaaa"
        blk = f"{dev}/block/mmcblk1"
        for name, text in (("size", "124735488"), ("ro", "0"), ("force_ro", "0"), ("removable", "0"),
                           ("dev", "179:32")):
            put(f"{blk}/{name}", text + "\n")
        for name, text in (("type", "SD"), ("name", "SD64G"), ("manfid", "0x000003"), ("oemid", "0x5344"),
                           ("date", "06/2024"), ("serial", SECRET_SERIAL), ("cid", "035344534436344780" + "1a2b3c4d")):
            put(f"{dev}/{name}", text + "\n")
        link("../../../mmc1:aaaa", os.path.join(root, blk, "device"))
        for name, text in (("start", "32768"), ("size", "124702720"), ("ro", "0"), ("dev", "179:33")):
            put(f"{blk}/mmcblk1p1/{name}", text + "\n")
        link(f"../{blk[len('sys/'):]}", os.path.join(root, "sys/block/mmcblk1"))
        put("run/udev/data/b179:32", UDEV_DISK)
        put("run/udev/data/b179:33", UDEV_PART)
        mounts += (f"/dev/mmcblk1p1 /media/{SECRET_VOLUME_ID} exfat rw,nosuid,nodev,noexec,relatime,uid=0,gid=0,"
                   "fmask=0022,dmask=0022,iocharset=utf8,errors=remount-ro 0 0\n")
        top = os.path.join(root, "media", SECRET_VOLUME_ID)
        for folder in ("Expansions", "Samples", "Projects"):
            os.makedirs(os.path.join(top, folder))
        put(f"media/{SECRET_VOLUME_ID}/notes.txt", "x")
    put("proc/mounts", mounts)
    put("etc/fstab", "# nothing for disks\nproc /proc proc defaults 0 0\n")
    put("lib/systemd/system/edisksd.service",
        "[Unit]\nDescription=Akai disk service\n\n[Service]\n# a comment\nExecStart=/usr/bin/edisksd\n")
    put("lib/systemd/system/no-reserved-blocks@.service",
        "[Service]\nType=oneshot\nExecStart=/sbin/tune2fs -r 0 /dev/%i\n")
    put("lib/udev/rules.d/99-edisksd.rules",
        'SUBSYSTEM=="block", ENV{ID_FS_TYPE}=="ext4", ENV{SYSTEMD_WANTS}+="no-reserved-blocks@%k.service"\n'
        'KERNEL=="loop*", GOTO="end"\n')


class SdCardTests(Base):
    def setUp(self):
        super().setUp()
        for applet in EXTRA_APPLETS:
            path = os.path.join(self.remote, applet)
            if not os.path.exists(path):
                os.symlink(self.busybox, path)
        # A disk tool Akai's system has; the survey only looks it up, never runs it.
        write(os.path.join(self.remote, "fsck.exfat"), "#!/bin/sh\necho RAN >> \"$FAKE_LOG\"\nexit 1\n", 0o755)

    def test_report(self):
        fake_root(self.root)
        result, report = self.run_wrapper("--sd-card", "mpc-one-sd-card")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("Collecting passive/read-only report", result.stdout)
        for text in ("MPFREE SD CARD SURVEY - READ ONLY", "release: 6.18.44-az01-fake-rt6",
                     "filesystems it reads now (/proc/filesystems): vfat exfat ext4 fuse fuseblk\n",
                     "  CONFIG_EXFAT_FS=y", "  # CONFIG_NTFS3_FS is not set", "  CONFIG_MMC_DW_ROCKCHIP=y",
                     "node: /mmc@ff0c0000", "  compatible: rockchip,rk3288-dw-mshc", "  status: okay",
                     "  properties: bus-width cap-sd-highspeed cd-gpios compatible max-frequency status",
                     "  cd-gpios: yes", "  broken-cd: no", "  non-removable: no", "  disable-wp: no",
                     "  cap-sd-highspeed: yes",
                     "platform device: ff0c0000.mmc, driver: dwmmc_rockchip", "MMC host: mmc1",
                     "  timing spec:\t2 (sd high-speed)",
                     "card: mmcblk1 (179:32), 124735488 sectors (59.5 GiB), read-only 0, force_ro 0, removable 0",
                     "  type: SD", "  name: SD64G", "  date: 06/2024",
                     "    ID_PART_TABLE_TYPE=dos", "    ID_DRIVE_FLASH_SD=1", "    ID_NAME=SD64G",
                     "partition mmcblk1p1 (179:33): start 32768, 124702720 sectors, read-only 0",
                     "    ID_FS_TYPE=exfat", "    ID_FS_VERSION=1.0", "    ID_FS_LABEL=MPC_SD",
                     "    ID_PART_ENTRY_TYPE=0x7", "    SYSTEMD_WANTS=edisksd-mount@mmcblk1p1.service",
                     "/dev/mmcblk1p1 on /media/<uuid> type exfat (rw,nosuid,nodev,noexec,relatime,",
                     "  top folder: 4 names", "    Expansions/", "    Samples/", "    notes.txt",
                     "fsck.exfat: " + self.remote + "/fsck.exfat", "mkfs.exfat: missing", "tune2fs: missing",
                     "--- /lib/systemd/system/edisksd.service", "ExecStart=/usr/bin/edisksd",
                     "--- /lib/systemd/system/no-reserved-blocks@.service", "ExecStart=/sbin/tune2fs -r 0 /dev/%i",
                     "/lib/udev/rules.d/99-edisksd.rules:1:SUBSYSTEM==\"block\", ENV{ID_FS_TYPE}==\"ext4\"",
                     "mmc1: new high speed SDXC card at address aaaa", "mmcblk1: mmc1:aaaa SD64G 59.5 GiB",
                     "dwmmc_rockchip ff0c0000.mmc: Successfully tuned phase",
                     "exFAT-fs (mmcblk1p1): Volume was not properly unmounted",
                     "edisksd[512]: mounted /dev/mmcblk1p1 at /media/<uuid>",
                     "edisksd[512]: found /dev/mmcblk1p1 type exfat uuid <uuid>",
                     "edisksd[512]: disk <uuid> ignored", "END OF SD CARD SURVEY"):
            self.assertIn(text, report)
        for secret in (SECRET_SERIAL, SECRET_VOLUME_ID, SECRET_UUID, SECRET_DISK_ID, SECRET_MAC, "1a2b3c4d"):
            self.assertNotIn(secret, report)
        for absent in ("mmc0: new HS200", "mmcblk0", "ID_SERIAL", "ID_FS_UUID", "PART_ENTRY_UUID", "S:disk",
                       "a comment", "Started unrelated", "eth0", "loop*", "/data ext4"):
            self.assertNotIn(absent, report)
        with open(self.log) as fh:
            self.assertNotIn("RAN", fh.read())
        calls = self.ssh_calls()
        self.assertEqual(len(calls), 1)
        self.assertTrue(calls[0].endswith(" sh -s"))
        self.assertIn(" -p 22 ", calls[0])

    def test_no_card_and_no_debugfs(self):
        fake_root(self.root, card=False, debugfs=False)
        result, report = self.run_wrapper("--sd-card", "mpc-one-sd-card")
        self.assertEqual(result.returncode, 0, result.stdout)
        for text in ("MMC host: mmc1", "bus settings: debugfs is not mounted; this script does not mount it",
                     "card: no block device under the SD controller (no card, or the controller is not bound)",
                     "nothing from the card is mounted", "END OF SD CARD SURVEY"):
            self.assertIn(text, report)
        self.assertNotIn("mmcblk0", report)


class StaticTests(unittest.TestCase):
    def test_script_only_reads(self):
        with open(REMOTE) as fh:
            lines = [line for line in fh.read().splitlines() if not line.lstrip().startswith("#")]
        # Text in single quotes is printed or is a grep or sed pattern, never a command.
        code = "\n".join(re.sub(r"'[^']*'", "''", line) for line in lines)
        # The disk tools are only looked up (command -v), in one list.
        tool_lists = re.findall(r"for t in ([^;]*); do", code, re.S)
        self.assertEqual(len(tool_lists), 1)
        self.assertIn("fsck.exfat", tool_lists[0])
        self.assertIn('p=$(command -v "$t" 2>/dev/null)', code)
        code = code.replace(tool_lists[0], "")
        for line in code.splitlines():
            stripped = line.replace("2>/dev/null", "").replace("2>&1", "").replace(">/dev/null", "")
            self.assertNotIn(">", stripped, f"the script redirects output: {line}")
        for forbidden in (r"\bmount\b", r"\bumount\b", r"\bblkid\b", r"\bfsck", r"\bmkfs", r"\btune2fs\b",
                          r"\be2fsck\b", r"\bsystemctl\b", r"\becho\b", r"\btee\b", r"\bdd\b", r"\brm\b",
                          r"\bmv\b", r"\bcp\b", r"\bmkdir\b", r"\btouch\b", r"\bmodprobe\b", r"\binsmod\b",
                          r"\brmmod\b", r"\bsysctl\b", r"/bind\b", r"/unbind\b", r"drivers_probe", r"dmesg\s+-",
                          r"/dev/mmcblk", r"\bhdparm\b", r"\budevadm\b", r"journalctl\s+.*--(vacuum|rotate|flush)"):
            self.assertIsNone(re.search(forbidden, code), forbidden)

    def test_wrapper_runs_it_on_akais_system(self):
        with open(os.path.join(TOOLS, "collect-over-ssh.sh")) as fh:
            text = fh.read()
        self.assertIn('"--sd-card") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-sd-card.sh"; REPORT_KIND="mpc-one-sd-card" ;;',
                      text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
