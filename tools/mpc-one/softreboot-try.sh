R=''
# MPFree soft-reboot into the stick. Streamed over SSH stdin by collect-over-ssh.sh (developer mode, port 22,
# an installed Akai 3.10.1 or later). The R='' line is an empty root prefix; only tests change it.
#
# Why soft-reboot: on the installed 3.10.1 the kexec route is refused by LoadPin (it will not load a kernel
# that is not from its trusted dm-verity source; see docs/dev/hardware/stick-autoboot.md). systemd's
# soft-reboot loads NO new kernel: it tears down all of userspace and starts the OS tree at /run/nextroot on
# the SAME running kernel, so LoadPin's kexec refusal never applies. MPFree already runs on Akai's kernel
# when it boots from the stick by kexec, so the stick's root can serve as that new userspace.
#
# Two modes, one script:
#   check  (RAM only, REVERSIBLE): does the whole setup a real soft-reboot needs and validates it, then
#          TEARS IT DOWN without rebooting. It mounts the stick's partition 1 read-only at /run/nextroot,
#          checks it is a valid MPFree OS tree for the running kernel, checks this systemd can soft-reboot,
#          looks at the internal-storage controllers, then unmounts and removes /run/nextroot. It changes
#          only RAM and leaves nothing behind; Akai's system keeps running.
#   try    (STATE-CHANGING: restarts userspace): the same setup and checks, then the real soft-reboot into
#          the stick. It writes NOTHING to storage. If MPFree does not come up, a power cycle with the stick
#          out returns to the stock Akai system (soft-reboot persists nothing). It refuses to reboot if a
#          firmware updater is running, or if any check fails.
#
# It never writes the root filesystem, the boot partitions, the U-Boot environment, /etc, or ANY storage.
# The only thing it writes is the /run/nextroot mount point (RAM), and in try mode it asks systemd to
# soft-reboot. The internal eMMC and SD card are never mounted or written; the stick's own root is built to
# leave them alone (offline-kexec-stick.py masks Akai's content-mount generator and makes /data a RAM disk).
[ -n "$R" ] || PATH=/usr/sbin:/usr/bin:/sbin:/bin   # a bare login shell may give no useful PATH
export PATH
set -u
umask 022
LABEL=mpfree-test-root
NEXTROOT="$R/run/nextroot"
UPDATERS='touch-fw-update.service xmos-update.service az01-usbsata-fixer.service'
FLASHERS='dfu-util touch-fw-upd xmosdfu midifirmup az01-update az01-usbsata-fixer'
kind=${1:-}
case "$kind" in check|try) ;; *) echo "usage: softreboot-try.sh check|try" >&2; exit 2 ;; esac
say() { printf '%s\n' "$*"; }
mnt=''; ours=''; made=''
cleanup() {
  if [ "$ours" = yes ]; then
    umount "$NEXTROOT" 2>/dev/null || say "note: could not unmount $NEXTROOT (it is only a RAM mount point)"
    ours=''
  fi
  # Only remove /run/nextroot if we made it; a pre-existing one may have been staged by something else.
  [ "$made" = yes ] && rmdir "$NEXTROOT" 2>/dev/null
  made=''
  return 0
}
fail() { say "STOP: $1"; cleanup; say "nothing was started: Akai's system keeps running"; exit 1; }

say "MPFREE SOFT-REBOOT INTO THE STICK - $kind"
say "mode: $([ "$kind" = check ] && echo 'RAM only, reversible (sets up and validates, then tears down; no restart)' || echo 'STATE-CHANGING: restarts userspace into the stick (writes no storage; a power cycle undoes it)')"

# --- the running system must be Akai's installed 3.10.1 --------------------------------------------------
rel=$(cat "$R/proc/sys/kernel/osrelease" 2>/dev/null)
say "running release: $rel"
case "$rel" in
  6.18.*-az01-*) ;;
  *) fail "this is for Akai's 3.10.1 (kernel 6.18, az01) installed on the MPC; it runs $rel" ;;
esac

# --- this systemd must be able to soft-reboot -----------------------------------------------------------
# systemd 254 and later restart userspace into /run/nextroot on the running kernel.
verb="soft-reboot"
svc="systemd-$verb.service"
sysd_ver=$(systemctl --version 2>/dev/null | sed -n '1s/^systemd \([0-9]*\).*/\1/p')
case "$sysd_ver" in
  ''|*[!0-9]*) fail "could not read systemd's version" ;;
esac
[ "$sysd_ver" -ge 254 ] 2>/dev/null || fail "systemd $sysd_ver is too old for a userspace-only restart (need 254+)"
[ -e "$R/usr/lib/systemd/system/$svc" ] || [ -e "$R/lib/systemd/system/$svc" ] ||
  fail "$svc is not present, so this systemd cannot restart userspace into another root"
say "systemd $sysd_ver can restart userspace into /run/nextroot ($svc present)"
[ ! -e "$NEXTROOT" ] || fail "/run/nextroot already exists; something else may be staging a restart"

# --- find MPFree's stick (partition 1 of a USB disk, named mpfree-test-root) ------------------------------
dev=$(readlink -f "$R/dev/disk/by-partlabel/$LABEL" 2>/dev/null)
[ -n "$dev" ] && [ -e "$dev" ] || fail "MPFree's stick is not in (no partition named $LABEL)"
name=${dev##*/}
case "$name" in sd[a-z]1) ;; *) fail "the partition named $LABEL is $name, not partition 1 of a USB disk" ;; esac
case "$(readlink -f "$R/sys/class/block/$name")" in */usb*) ;; *) fail "/dev/$name is not on USB" ;; esac
uuid=''
for l in "$R"/dev/disk/by-partuuid/*; do
  [ "$(readlink -f "$l")" = "$dev" ] && uuid=${l##*/}
done
case "$uuid" in [0-9a-f]*-*-*-*-*) ;; *) fail "/dev/$name has no partition UUID link" ;; esac
say "stick: /dev/$name (partition 1 of a USB disk, PARTUUID $uuid)"

# --- mount the stick's root at /run/nextroot, read-only ---------------------------------------------------
# soft-reboot pivots into /run/nextroot, so the new root must be mounted there specifically. Akai may also
# have it mounted at /media/<uuid>; mounting the same device again read-only is fine. Read-only, no journal
# replay, and the mount options forbid devices, setuid and execution of the mount point itself.
mkdir "$NEXTROOT" 2>/dev/null || fail "could not make /run/nextroot"
made=yes
mount -t ext4 -o ro,noload,nodev,nosuid "$dev" "$NEXTROOT" || fail "could not mount /dev/$name at /run/nextroot read-only"
ours=yes

# --- the mounted tree must be a valid MPFree root for THIS kernel -----------------------------------------
# A real init, the MPFree marker the stick build writes, at least one MPFree service, and kernel modules
# that match the running kernel (so built-in and matching drivers are available; LoadPin blocks loading any
# module FROM the stick, so MPFree relies on what is built into Akai's kernel plus matching modules Akai
# already loaded before the restart).
[ -x "$NEXTROOT/sbin/init" ] || [ -x "$NEXTROOT/lib/systemd/systemd" ] || [ -x "$NEXTROOT/usr/lib/systemd/systemd" ] ||
  fail "the stick's root has no init (/sbin/init or systemd); it is not a bootable OS tree"
[ -e "$NEXTROOT/etc/mpfree-test" ] || fail "the stick's root has no /etc/mpfree-test marker; it is not an MPFree stick"
svc_found=''
for u in mpfree-test-splash.service mpfree-test-app.service mpfree-test-screen.service; do
  [ -e "$NEXTROOT/etc/systemd/system/$u" ] && { svc_found=$u; break; }
done
[ -n "$svc_found" ] || fail "the stick's root has no MPFree startup service (no mpfree-test-splash/app/screen)"
[ -d "$NEXTROOT/lib/modules/$rel" ] || fail "the stick's root has no /lib/modules/$rel; it was built for a different kernel"
# Two masks on the stick are what keep the internal eMMC untouched in the new userspace, so require them:
# acvs.service (Akai's app, which also mounts/writes content) and the eMMC content-mount generator must
# each be a mask (a link to /dev/null). Without them the factory app or an eMMC mount could start.
masked() { m="$NEXTROOT/$1"; [ -L "$m" ] && [ "$(readlink "$m" 2>/dev/null)" = /dev/null ]; }
masked etc/systemd/system/acvs.service || fail "Akai's app (acvs.service) is not masked on the stick; the factory app could start"
masked etc/systemd/system-generators/acva-emmc-content-generator ||
  fail "the eMMC content-mount generator is not masked on the stick; the internal eMMC could be mounted"
say "stick root: init present, /etc/mpfree-test marker, startup service $svc_found, modules for $rel"
say "stick masks in place: Akai's app (acvs.service) and the eMMC content-mount generator"

# --- internal storage: what stays visible under soft-reboot ----------------------------------------------
# soft-reboot keeps the running device tree, so the eMMC and SD controllers stay enabled (unlike the kexec
# route, which hid them). MPFree must not mount them. The stick's root does not (its content-mount generator
# is masked and /data is a RAM disk). Report the controllers so the isolation is visible; this does not
# change them.
say "internal storage controllers still bound (MPFree's root must not mount these):"
for d in /sys/bus/platform/drivers/dwmmc_rockchip /sys/bus/platform/drivers/rockchip-dw-mshc; do
  [ -d "$R$d" ] || continue
  for b in "$R$d"/*; do
    case "${b##*/}" in *dwmmc@*|*mmc@*|ff0*) [ -e "$b/driver" ] || [ -L "$b" ] && say "  ${b##*/}" ;; esac
  done
done
blk=''
for b in "$R"/sys/block/mmcblk[0-9]; do [ -d "$b" ] && blk="$blk ${b##*/}"; done
say "  eMMC/SD block devices present:${blk:- none}"

# --- a firmware updater must not be mid-write ------------------------------------------------------------
busy() {
  for u in $UPDATERS; do
    case "$(systemctl is-active "$u" 2>/dev/null)" in activating|deactivating|reloading) echo "$u"; return 0 ;; esac
  done
  for p in $FLASHERS; do pidof "$p" > /dev/null 2>&1 && { echo "$p"; return 0; }; done
  grep -qs 'SME0/Firmware/update.sh' "$R"/proc/[0-9]*/cmdline && { echo "Akai's controller update.sh"; return 0; }
  return 1
}
what=$(busy) && fail "$what is running now; a restart must never cut a firmware write short"

say "all checks passed: /run/nextroot is the stick's MPFree root, and this kernel can soft-reboot into it"

if [ "$kind" = check ]; then
  cleanup
  [ -e "$NEXTROOT" ] && { say "STOP: /run/nextroot is still present after teardown"; exit 1; }
  say "CHECK-OK: torn down, nothing left behind. NEXT: run --softreboot-try to actually start MPFree this way."
  exit 0
fi

# --- try: the real soft-reboot ---------------------------------------------------------------------------
# From here the running userspace is replaced by the stick's. Nothing is written to storage. systemd stops
# everything, keeps the running kernel, and starts the stick's init from /run/nextroot. This message may be
# the last thing this SSH session prints.
say "starting MPFree now: systemd will stop Akai's userspace and start the stick's root on this kernel."
say "if MPFree does not appear, switch the MPC off, take the stick out, and switch on for the stock system."
sync
systemctl "$verb" ||
  fail "systemd refused the userspace restart (systemctl $verb)"
exit 0
