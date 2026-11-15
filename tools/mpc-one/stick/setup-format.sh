R=''
# MPFree stick, first boot: make the empty filesystem on partition 4 (the pad player's saved settings, files,
# library and eMMC backups). STATE-CHANGING on the stick's partition 4 only, and only once.
# (The R='' line is an empty root prefix; only tests change it.)
# The USB creator does not write partition 4's filesystem (it fills the rest of the stick); it writes a mark
# at the partition's start instead: "MPFREE-FORMAT-ME <token>", with the same token in this system's
# /etc/mpfree-test/format-token. At boot, before partition 4 is mounted (mpfree-setup-format.service runs
# before mpfree-test-storage.service), this
#   - does nothing unless partition 4 (found by its partition UUID, on USB) starts with that exact mark;
#   - with the mark: makes an ext4 filesystem labelled MPFREE-DATA with mke2fs from the system's own
#     e2fsprogs, unless partition 4 already holds an ext2/3/4 filesystem; then clears the mark (the first
#     512 bytes, which ext4 does not use), so it never runs again.
# A stick whose partition 4 was kept by the creator (an MPFree data partition was there) has no mark, so
# its settings and files are never touched. If the power is cut before the mark is cleared, the next boot
# makes the (still empty) filesystem again.
[ -n "$R" ] || PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
DEV=/dev/disk/by-partuuid/@DATA_UUID@
say() { { echo "mpfree-setup-format: $*" > "$R/dev/kmsg"; } 2>/dev/null; echo "$*" >> "$R/run/mpfree-setup-format.txt"; }
i=0
while [ ! -e "$R$DEV" ] && [ "$i" -lt 30 ]; do sleep 1; i=$((i + 1)); done
[ -e "$R$DEV" ] || { say "partition 4 not found; nothing to do"; exit 0; }
dev=$(readlink -f "$R$DEV")
name=${dev##*/}
case "$name" in sd[a-z]4) ;; *) say "partition 4 is $name, not partition 4 of a USB disk; not touched"; exit 0 ;; esac
case "$(readlink -f "$R/sys/class/block/$name")" in */usb*) ;; *) say "/dev/$name is not on USB; not touched"; exit 0 ;; esac
token=$(cat "$R/etc/mpfree-test/format-token" 2>/dev/null)
case "$token" in ''|*[!0-9a-f]*) say "no format token in this system; partition 4 not touched"; exit 0 ;; esac
mark=$(dd if="$dev" bs=49 count=1 2>/dev/null)
[ "$mark" = "MPFREE-FORMAT-ME $token" ] || exit 0
grep -q "^$dev " "$R/proc/mounts" && { say "/dev/$name is mounted; not touched"; exit 0; }
magic=$(dd if="$dev" bs=1 skip=1080 count=2 2>/dev/null | od -An -tx1 | tr -d ' \n')
if [ "$magic" = 53ef ]; then
  say "partition 4 holds a filesystem already; only its format mark is cleared"
else
  say "making partition 4's filesystem (ext4, MPFREE-DATA) on /dev/$name"
  mke2fs -q -F -t ext4 -m 0 -L MPFREE-DATA -E root_owner=0:0 "$dev" > "$R/run/mpfree-setup-format-mke2fs.txt" 2>&1 || {
    say "mke2fs failed (see /run/mpfree-setup-format-mke2fs.txt); nothing is saved this boot"
    exit 1
  }
fi
dd if=/dev/zero of="$dev" bs=512 count=1 conv=notrunc 2>/dev/null && sync ||
  { say "could not clear the format mark"; exit 1; }
[ "$(dd if="$dev" bs=49 count=1 2>/dev/null)" = "MPFREE-FORMAT-ME $token" ] && { say "the format mark is still there"; exit 1; }
say "partition 4 is ready"
exit 0
