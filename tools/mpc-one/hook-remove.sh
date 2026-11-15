R=''
# MPFree boot hook: REMOVE (the undo of hook-install.sh). STATE-CHANGING on the MPC's data partition only:
# deletes the hook's three files, the canary's if any are left, and the folders the installs made, and
# nothing else. Streamed over SSH stdin by `collect-over-ssh.sh --hook-remove`. The R='' line is an empty
# root prefix; only tests change it. Running it again when nothing is left is harmless.
set -u
say() { printf '%s\n' "$*"; }

UPPER="$R/data/system/etc/overlay"
HOOK="$R/data/mpfree-hook"
RULES="$UPPER/udev/rules.d"
MODS="$UPPER/modprobe.d"
CREATED="$HOOK/created-folders.txt"
MARK='# MPFree boot hook, written by tools/mpc-one/hook-install.sh.'
CANARY_MARK='# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh.'

say "MPFREE BOOT HOOK - REMOVE (changes the data partition only)"
fail=0
# Only files the hook or the canary wrote: they carry its marker line. The modprobe lines and the rules go
# first, so that nothing is left pointing at a script.
remove() {   # $1 = marker, $2... = files
  mark=$1; shift
  for f in "$@"; do
    [ -e "$f" ] || continue
    if [ -f "$f" ] && grep -qxF "$mark" "$f"; then
      rm -f "$f" && say "deleted ${f#$R}" || { say "PROBLEM: could not delete ${f#$R}"; fail=1; }
    else
      say "PROBLEM: ${f#$R} is not MPFree's; left in place"; fail=1
    fi
  done
}
remove "$MARK" "$MODS/99-mpfree-hook.conf" "$MODS/.99-mpfree-hook.conf.new" \
  "$RULES/99-mpfree-hook.rules" "$RULES/.99-mpfree-hook.rules.new" "$HOOK/hook.sh" "$HOOK/.hook.sh.new"
remove "$CANARY_MARK" "$MODS/99-mpfree-canary.conf" "$MODS/.99-mpfree-canary.conf.new" \
  "$RULES/99-mpfree-canary.rules" "$RULES/.99-mpfree-canary.rules.new" "$HOOK/canary.sh" "$HOOK/.canary.sh.new"
# The folders the installs made (listed in created-folders.txt), deepest first, only if empty now; kept
# (with their list) when a file could not be deleted, so that running this again can finish.
if [ "$fail" = 0 ]; then
  made=""
  [ -f "$CREATED" ] && made=$(cat "$CREATED")
  rm -f "$CREATED"
  for d in /data/system/etc/overlay/udev/rules.d /data/system/etc/overlay/udev /data/system/etc/overlay/modprobe.d \
           /data/mpfree-hook; do
    printf '%s\n' "$made" | grep -qxF "$d" || [ "$d" = /data/mpfree-hook ] || continue
    [ -d "$R$d" ] || continue
    if rmdir "$R$d" 2>/dev/null; then say "removed the folder $d"; else say "kept the folder $d (not empty)"; fi
  done
fi
sync
left=0
for f in "$MODS/99-mpfree-hook.conf" "$RULES/99-mpfree-hook.rules" "$HOOK/hook.sh"; do
  [ -e "$f" ] && { say "still there: ${f#$R}"; left=1; }
done
if [ "$fail" = 0 ] && [ "$left" = 0 ]; then
  say "RESULT: the hook is gone. From the next power-on the MPC starts Akai's system, stick in or not."
else
  say "RESULT: PROBLEM, see above."
  exit 1
fi
