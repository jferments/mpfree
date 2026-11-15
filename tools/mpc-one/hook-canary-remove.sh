R=''
# MPFree boot-hook canary: REMOVE (the undo of hook-canary-install.sh). STATE-CHANGING on the MPC's
# data partition only: deletes the canary's three files and the folders the install made, and nothing else.
# Streamed over SSH stdin by `collect-over-ssh.sh --hook-canary-remove`. The R='' line is an empty root
# prefix; only tests change it. Running it again when nothing is left is harmless. It also removes an
# earlier version of the canary (two files, no modprobe line).
set -u
say() { printf '%s\n' "$*"; }

UPPER="$R/data/system/etc/overlay"
HOOK="$R/data/mpfree-hook"
RULE="$UPPER/udev/rules.d/99-mpfree-canary.rules"
CONF="$UPPER/modprobe.d/99-mpfree-canary.conf"
SCRIPT="$HOOK/canary.sh"
CREATED="$HOOK/created-folders.txt"
MARK='# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh.'

say "MPFREE BOOT-HOOK CANARY - REMOVE (changes the data partition only)"
fail=0
# Only files this canary wrote: they carry its marker line. The modprobe line and the rule go first, so
# that nothing is left pointing at the script.
for f in "$CONF" "$UPPER/modprobe.d/.99-mpfree-canary.conf.new" "$RULE" "$UPPER/udev/rules.d/.99-mpfree-canary.rules.new" \
         "$SCRIPT" "$HOOK/.canary.sh.new"; do
  [ -e "$f" ] || continue
  if [ -f "$f" ] && grep -qxF "$MARK" "$f"; then
    rm -f "$f" && say "deleted ${f#$R}" || { say "PROBLEM: could not delete ${f#$R}"; fail=1; }
  else
    say "PROBLEM: ${f#$R} is not the canary's; left in place"; fail=1
  fi
done
# The folders the install made (listed in created-folders.txt), deepest first, only if empty now; kept
# (with their list) when a file could not be deleted, so that running this again can finish.
if [ "$fail" = 0 ]; then
  made=""
  [ -f "$CREATED" ] && made=$(cat "$CREATED")
  rm -f "$CREATED"
  for d in /data/system/etc/overlay/udev/rules.d /data/system/etc/overlay/udev /data/system/etc/overlay/modprobe.d \
           /data/mpfree-hook; do
    printf '%s\n' "$made" | grep -qxF "$d" || continue
    [ -d "$R$d" ] || continue
    if rmdir "$R$d" 2>/dev/null; then say "removed the folder $d"; else say "kept the folder $d (not empty)"; fi
  done
fi
sync
left=0
for f in "$CONF" "$RULE" "$SCRIPT"; do [ -e "$f" ] && { say "still there: ${f#$R}"; left=1; }; done
if [ "$fail" = 0 ] && [ "$left" = 0 ]; then
  say "RESULT: the canary is gone. From the next power-on neither udev nor modprobe runs anything of MPFree's."
else
  say "RESULT: PROBLEM, see above."
  exit 1
fi
