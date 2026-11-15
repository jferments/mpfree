R=''
# MPFree kexec file-syscall probe. STATE-CHANGING IN RAM ONLY, and only for an instant: it loads a kernel
# into memory with kexec and unloads it again, several ways, to find which load LoadPin and this kernel
# accept. It NEVER starts kexec.target, NEVER calls `kexec -e`, and NEVER restarts the MPC. It writes no
# file on any disk (a temporary kernel copy goes under /run, a RAM filesystem, and is deleted). The R='' line
# is an empty root prefix; only tests change it.
#
# Why: on the installed 3.10.1, LoadPin (enforce + verity) denies the old kexec_load buffer syscall
# ("kexec-image old-api-denied") and wants the file-based syscall (kexec_file_load, -s) from a kernel file on
# a filesystem it trusts. The MPC's own kernel, /boot/zImage-<release> on the root filesystem, is such a
# file. This probe finds out, without booting anything, whether a file-based load of that trusted kernel is
# accepted, whether a custom command line and device tree can go with it, and that a kernel from an untrusted
# place (a copy in /run) is refused. From the results the boot hook's load step can be set to what works.
# Streamed over SSH stdin by `collect-over-ssh.sh --hook-kexec-probe` (port 22, MPFREE_ACCEPT_KEXEC_PROBE=YES).
set -u
[ -n "$R" ] || PATH=/usr/sbin:/usr/bin:/sbin:/bin
say() { printf '%s\n' "$*"; }
KX="$R/usr/sbin/kexec"
LOADED="$R/sys/kernel/kexec_loaded"
WORK="$R/run/mpfree-kexec-probe"

loaded_now() { cat "$LOADED" 2>/dev/null; }
unload() { "$KX" -u --kexec-syscall 2>/dev/null; "$KX" -u --kexec-file-syscall 2>/dev/null; }
# Always leave the machine with no kernel staged, whatever happens.
cleanup() { unload; rm -rf "$WORK" 2>/dev/null; }
trap cleanup EXIT INT TERM

say "MPFREE KEXEC FILE-SYSCALL PROBE - RAM ONLY (loads then unloads a kernel; never restarts)"
rel=$(uname -r 2>/dev/null)
say "running release: $rel"

[ -x "$KX" ] || { say "STOP: no $KX"; exit 1; }
[ "$(cat "$R/proc/sys/kernel/kexec_load_disabled" 2>/dev/null)" = 0 ] || { say "STOP: kexec is turned off (kexec_load_disabled is not 0)"; exit 1; }
if [ "$(loaded_now)" != 0 ]; then say "STOP: a kernel is already staged (kexec_loaded is not 0); not touching it"; exit 1; fi

# The trusted kernel: the MPC's own, on the root filesystem (the filesystem LoadPin pins to).
TRUSTED="$R/boot/zImage-$rel"
case "$rel" in ''|*[!A-Za-z0-9._+-]*) say "STOP: the running release has an unusual name"; exit 1 ;; esac
[ -f "$TRUSTED" ] && [ ! -L "$TRUSTED" ] || { say "STOP: no $R/boot/zImage-$rel to load (looked for the running kernel on the root filesystem)"; exit 1; }
rootdev=$(df -P "$R/" 2>/dev/null | awk 'NR==2{print $1}')
kdev=$(df -P "$TRUSTED" 2>/dev/null | awk 'NR==2{print $1}')
say "trusted kernel: /boot/zImage-$rel ($(wc -c < "$TRUSTED" | tr -cd 0-9) bytes, on $kdev; root is on $rootdev)"
[ "$kdev" = "$rootdev" ] || say "note: the kernel is not on the root filesystem device; LoadPin may still accept it if that filesystem is trusted"

# A benign command line for the probes. Nothing is ever executed, so its values do not matter; it only tests
# whether a custom command line is accepted with the load.
CMDLINE="root=PARTUUID=00000000-0000-0000-0000-000000000000 ro rootwait=60 panic=10 sysctl.kernel.kexec_load_disabled=1"
mkdir -p "$WORK" 2>/dev/null

# Run one load, report whether it staged a kernel, then unload and confirm nothing is left.
# $1 = label, rest = kexec arguments (without --load, which is added here).
probe() {
  label=$1; shift
  : > "$WORK/err"
  "$KX" --load "$@" 2>"$WORK/err"
  rc=$?
  st=$(loaded_now)
  if [ "$rc" = 0 ] && [ "$st" = 1 ]; then
    say "  $label: LOADED (this load is accepted)"
  else
    say "  $label: refused (exit $rc)"
    sed 's/^/      /' "$WORK/err" 2>/dev/null | head -n 4
  fi
  unload
  [ "$(loaded_now)" = 0 ] || say "      WARNING: a kernel is still staged after unload; run this again or power-cycle"
}

say ""
say "=== Probes (each loads then immediately unloads; nothing boots) ==="
probe "old buffer syscall (what the hook used; expected to be denied by LoadPin)" -c --kexec-syscall --command-line="$CMDLINE" "$TRUSTED"
probe "file syscall, our command line" -s --kexec-file-syscall --command-line="$CMDLINE" "$TRUSTED"
probe "file syscall, our command line + a device tree" -s --kexec-file-syscall --command-line="$CMDLINE" --dtb="$R/sys/firmware/fdt" "$TRUSTED"

# Control: the same kernel from an untrusted place (a copy in /run, a RAM filesystem) should be refused,
# which is why the hook must load the on-disk trusted kernel, not a copy from the stick.
if cp "$TRUSTED" "$WORK/zImage" 2>/dev/null; then
  probe "file syscall, SAME kernel but copied to /run (untrusted place; expected refused)" -s --kexec-file-syscall --command-line="$CMDLINE" "$WORK/zImage"
fi

say ""
say "=== LoadPin lines in the kernel log from these probes ==="
dmesg 2>/dev/null | grep -iE 'loadpin|kexec' | tail -n 20 || say "(none)"

rm -rf "$WORK" 2>/dev/null
if [ "$(loaded_now)" = 0 ]; then
  say ""
  say "RESULT: probe done, nothing is staged, the MPC was not restarted. The 'LOADED' lines above are the loads this kernel accepts; a file-syscall load of the trusted kernel that LOADED is the path the hook can use."
else
  say "RESULT: a kernel is still staged after the probe; power-cycle the MPC to clear it (it will NOT boot into anything)."
  exit 1
fi
