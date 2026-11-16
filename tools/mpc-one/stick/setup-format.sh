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
# Every program it runs is in NEEDS (Akai's BusyBox has no od -t, so od -b): it touches nothing when one is
# missing, and the USB creator will not make a stick from an update whose system lacks one.
# The journal is 64 MiB (-J size=64): mke2fs zeroes it, and its default for a 64 GB stick, 256 MiB, took
# about 37 s of the second creator stick's first start. The rest it writes is under 0.01 % of the partition.
# One inode per 128 KiB (-i 131072; the default is one per 16 KiB): the kernel zeroes the inode tables in
# the background after the first mount (ext4lazyinit), about 0.95 GB on a 64 GB stick by default, all during
# the first start's backup on the same stick. This makes it about 120 MB, and still leaves room for over
# 100,000 files on a 16 GB stick.
# How it goes: /run/mpfree-setup/format-status (state: checking, running, done, none = nothing to do, failed;
# done and total: bytes written to the partition, from the block layer, and the bytes expected). The
# startup screen shows it on the creator's sticks (mpfree_startup::setup), until the state is final.
[ -n "$R" ] || PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
DEV=/dev/disk/by-partuuid/@DATA_UUID@
ST="$R/run/mpfree-setup"
NEEDS='awk cat dd grep mke2fs mkdir mv od readlink sleep sync'
say() { { echo "mpfree-setup-format: $*" > "$R/dev/kmsg"; } 2>/dev/null; echo "$*" >> "$R/run/mpfree-setup-format.txt"; }
report() {   # state done total
  { echo "state=$1"; echo "done=$2"; echo "total=$3"; } > "$ST/format-status.new" 2>/dev/null &&
    mv "$ST/format-status.new" "$ST/format-status" 2>/dev/null
}
finish() {   # state exit-code [note]
  [ -z "${3:-}" ] || say "$3"
  report "$1" "${wrote:-0}" "${total:-0}"
  exit "$2"
}
mkdir -p "$ST" 2>/dev/null
report checking 0 0
missing=''
for p in $NEEDS; do command -v "$p" > /dev/null 2>&1 || missing="$missing $p"; done
[ -z "$missing" ] || finish failed 1 "this system lacks programs this needs:$missing; partition 4 not touched"
i=0
while [ ! -e "$R$DEV" ] && [ "$i" -lt 30 ]; do sleep 1; i=$((i + 1)); done
[ -e "$R$DEV" ] || finish none 0 "partition 4 not found; nothing to do"
dev=$(readlink -f "$R$DEV")
name=${dev##*/}
case "$name" in sd[a-z]4) ;; *) finish none 0 "partition 4 is $name, not partition 4 of a USB disk; not touched" ;; esac
case "$(readlink -f "$R/sys/class/block/$name")" in */usb*) ;; *) finish none 0 "/dev/$name is not on USB; not touched" ;; esac
token=$(cat "$R/etc/mpfree-test/format-token" 2>/dev/null)
case "$token" in ''|*[!0-9a-f]*) finish none 0 "no format token in this system; partition 4 not touched" ;; esac
mark=$(dd if="$dev" bs=49 count=1 2>/dev/null)
[ "$mark" = "MPFREE-FORMAT-ME $token" ] || finish none 0
grep -q "^$dev " "$R/proc/mounts" && finish none 0 "/dev/$name is mounted; not touched"
magic=$(dd if="$dev" bs=1 skip=1080 count=2 2>/dev/null | od -b | awk 'NR == 1 { print $2 $3 }')
if [ "$magic" = 123357 ]; then   # ext2/3/4's magic number, 0xef53, in octal bytes
  say "partition 4 holds a filesystem already; only its format mark is cleared"
else
  say "making partition 4's filesystem (ext4, MPFREE-DATA) on /dev/$name"
  written() { awk '{ print $7 + 0 }' "$R/sys/class/block/$name/stat" 2>/dev/null || echo 0; }   # sectors
  sectors=$(cat "$R/sys/class/block/$name/size" 2>/dev/null)
  case "$sectors" in ''|*[!0-9]*) sectors=0 ;; esac
  total=$(( 67108864 + sectors * 512 / 8192 ))
  before=$(written)
  wrote=0
  report running 0 "$total"
  mke2fs -q -F -t ext4 -m 0 -i 131072 -J size=64 -L MPFREE-DATA -E root_owner=0:0 "$dev" \
    > "$R/run/mpfree-setup-format-mke2fs.txt" 2>&1 &
  maker=$!
  while kill -0 "$maker" 2>/dev/null; do
    wrote=$(( ($(written) - before) * 512 ))
    report running "$wrote" "$total"
    sleep 1
  done
  wait "$maker" ||
    finish failed 1 "mke2fs failed (see /run/mpfree-setup-format-mke2fs.txt); nothing is saved this boot"
  wrote=$total
fi
# Written through the shell's read-write redirection, which never truncates: Akai's BusyBox dd has no conv=
# (built without its ibs/obs options; the second creator stick stopped here on 2026-11-16).
dd if=/dev/zero bs=512 count=1 1<> "$dev" 2> "$R/run/mpfree-setup-format-dd.txt" && sync ||
  finish failed 1 "could not clear the format mark: $(cat "$R/run/mpfree-setup-format-dd.txt")"
[ "$(dd if="$dev" bs=49 count=1 2>/dev/null)" = "MPFREE-FORMAT-ME $token" ] && finish failed 1 "the format mark is still there"
finish done 0 "partition 4 is ready"
