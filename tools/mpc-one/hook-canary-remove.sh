R=''
# MPFree boot-hook canary: REMOVE (the undo of hook-canary-install.sh). STATE-CHANGING on the MPC's
# data partition only: deletes the canary's two files and the folders the install made, and nothing else.
# Streamed over SSH stdin by `collect-over-ssh.sh --hook-canary-remove`. The R='' line is an empty root
# prefix; only tests change it. Running it again when nothing is left is harmless.
set -u
say() { printf '%s\n' "$*"; }

UPPER="$R/data/system/etc/overlay"
HOOK="$R/data/mpfree-hook"
RULE="$UPPER/udev/rules.d/99-mpfree-canary.rules"
SCRIPT="$HOOK/canary.sh"
CREATED="$HOOK/created-folders.txt"
MARK='# MPFree boot-hook canary (test only), written by tools/mpc-one/hook-canary-install.sh.'

say "MPFREE BOOT-HOOK CANARY - REMOVE (changes the data partition only)"
fail=0
# Only files this canary wrote: their first comment line names it.
for f in "$RULE" "$UPPER/udev/rules.d/.99-mpfree-canary.rules.new"; do
  [ -e "$f" ] || continue
  if grep -qxF "$MARK" "$f"; then rm -f "$f" && say "deleted ${f#$R}" || { say "PROBLEM: could not delete ${f#$R}"; fail=1; }
  else say "PROBLEM: ${f#$R} is not the canary's rule; left in place"; fail=1; fi
done
for f in "$SCRIPT" "$HOOK/.canary.sh.new"; do
  [ -e "$f" ] || continue
  if grep -qxF "$MARK" "$f"; then rm -f "$f" && say "deleted ${f#$R}" || { say "PROBLEM: could not delete ${f#$R}"; fail=1; }
  else say "PROBLEM: ${f#$R} is not the canary's script; left in place"; fail=1; fi
done
# The folders the install made (listed in created-folders.txt), deepest first, only if empty now.
made=""
[ -f "$CREATED" ] && made=$(cat "$CREATED")
rm -f "$CREATED"
for d in /data/system/etc/overlay/udev/rules.d /data/system/etc/overlay/udev /data/mpfree-hook; do
  printf '%s\n' "$made" | grep -qxF "$d" || continue
  [ -d "$R$d" ] || continue
  if rmdir "$R$d" 2>/dev/null; then say "removed the folder $d"; else say "kept the folder $d (not empty)"; fi
done
sync
left=0
for f in "$RULE" "$SCRIPT"; do [ -e "$f" ] && { say "still there: ${f#$R}"; left=1; }; done
if [ "$fail" = 0 ] && [ "$left" = 0 ]; then
  say "RESULT: the canary is gone. From the next power-on udev has no MPFree rule."
else
  say "RESULT: PROBLEM, see above."
  exit 1
fi
