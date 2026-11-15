#!/bin/sh
# fxprobe.sh: run FXProbe on the MPC, under Akai's MPC OS 3.10.1 in developer
# mode, from the FXProbe kit on the MPFree test stick. BusyBox sh.
#
#   sh /media/<kits partition>/mpfree-dev/fxprobe/fxprobe.sh MODE [OPTIONS]
#
# The kit (this script's folder, made by stage.py) holds both plugins, the
# test programmes and this script. Akai's system mounts every partition of a
# USB stick noexec, so plugins can't load from the stick: install copies them
# to the MPC's internal drive once. The work folder, where commands, status
# and captures go, is FXProbe/ on the same stick's data partition (the
# partition after the kit's), found by device, whatever name it is mounted
# under. Each installed plugin holds a pointer (workfolder.txt) that finds the
# work folder by its marker on any drive under /media.
#
# Modes (each says whether it changes anything, and where):
#   check                read-only: the kit, the drives, the installed plugins,
#                        the work folder and the instances that report
#   install [--drive D]  CHANGES the internal drive: copies the two plugin
#                        folders into D/VST2 Plugins (default
#                        /media/az01-internal). Undo: remove
#   remove [--drive D]   CHANGES the internal drive: deletes exactly those two
#                        plugin folders (and VST2 Plugins if install made it
#                        and it is empty). The undo of install
#   setup                CHANGES the stick only: makes FXProbe/ on the data
#                        partition and copies the programmes into it. Undo: clean.
#                        Needs MPFree's FORMAT file there (MPFree has started
#                        from the stick once): MPFree's store refuses a data
#                        partition that holds other files and no FORMAT
#   status               read-only: what every instance reports
#   capture NAME [--programme short|standard] [--gains 0,-12] [--slots 1,2]
#                [--tail SECONDS] [--timing]
#                        CHANGES the stick only (control.txt, new captures): one
#                        capture per gain of every slot, as each is set now
#   clean                CHANGES the stick only: deletes FXProbe/ from the data
#                        partition, captures included. Undo: setup (captures
#                        are gone; copy them off first)
# Global option, before MODE: --work DIR uses DIR as the work folder.
#
# Lines starting STOP: say why nothing (more) was done. The last line is
# CHECK-OK, INSTALL-OK, REMOVE-OK, SETUP-OK, CAPTURE-OK or CLEAN-OK when the
# mode did all it should. Exit status 0 then, 1 after STOP or a problem, 2 for
# a usage error.
#
# Tests set FXPROBE_TEST_ROOT to a fake root (its proc/mounts and media/).
# SPDX-License-Identifier: MIT

R=${FXPROBE_TEST_ROOT:-}
# Everything this makes is readable by everyone (the shell's umask on the MPC
# is unknown), so a workstation reads the stick without root.
umask 022
SOURCE_DIR="MPFree - VST - FXProbe Source"
TAP_DIR="MPFree - VST - FXProbe Tap"
MARKER=.fxprobe-work
POINTER="$R/media/*/FXProbe"

stop() { echo "STOP: $*"; exit 1; }
usage() { echo "usage: sh fxprobe.sh [--work DIR] check|install|remove|setup|status|capture NAME|clean [options]" >&2; exit 2; }

case "$0" in */*) KIT=${0%/*} ;; *) KIT=. ;; esac
KIT=$(cd "$KIT" && pwd) || stop "cannot find the kit folder"

# ---- mounts ---------------------------------------------------------------

# Prints "device mountpoint type options" for every mount, with \040 as space.
mounts() {
  sed 's/\\040/ /g' "$R/proc/mounts" 2>/dev/null
}

# The mount line whose mount point is $1 (a path under $R), or nothing.
mount_line() {
  mounts | while read -r dev mnt type opts rest; do
    [ "$R$mnt" = "$1" ] && { echo "$dev $mnt $type $opts"; break; }
  done
}

# The mount line holding path $1: the longest mount point that is a prefix of it.
mount_of() {
  best="" bestlen=0
  while read -r dev mnt type opts rest; do
    m="$R$mnt"
    case "$1/" in
      "$m"/*|"$m"/) n=${#m}; [ "$n" -gt "$bestlen" ] && { best="$dev $mnt $type $opts"; bestlen=$n; } ;;
    esac
  done <<EOF
$(mounts)
EOF
  echo "$best"
}

has_opt() {  # has_opt OPTIONS NAME
  case ",$1," in *",$2,"*) return 0 ;; esac
  return 1
}

# The stick's data partition: the partition after the kit's on the same disk
# (partition 4 for a kit on partition 3). Sets DATA_LINE and DATA_MNT.
find_data() {
  kl=$(mount_of "$KIT")
  [ -n "$kl" ] || stop "the kit at $KIT is on no mount"
  set -- $kl
  kdev=$1
  case "$kdev" in
    *[0-9]) ;;
    *) stop "the kit is on $kdev, not a partition of a disk" ;;
  esac
  num=${kdev##*[!0-9]}
  ddev=${kdev%"$num"}$((num + 1))
  DATA_LINE=$(mounts | while read -r dev mnt type opts rest; do
    [ "$dev" = "$ddev" ] && { echo "$dev $mnt $type $opts"; break; }; done)
  [ -n "$DATA_LINE" ] || stop "the stick's data partition ($ddev, the one after the kit's $kdev) is not mounted"
  set -- $DATA_LINE
  DATA_MNT="$R$2"
}

work_folder() {
  if [ -n "$WORK" ]; then return; fi
  find_data
  WORK="$DATA_MNT/FXProbe"
}

# ---- the kit --------------------------------------------------------------

kit_ok() {
  [ -f "$KIT/SHA256SUMS" ] || stop "the kit has no SHA256SUMS"
  out=$(cd "$KIT" && sha256sum -c SHA256SUMS 2>&1) || {
    echo "$out" | grep -v ': OK$'
    stop "the kit does not match its SHA256SUMS (rebuild the stick)"
  }
}

# True if folder $1 is one of ours: its plugin.json names MPFree and uid $2.
ours() {
  [ -f "$1/plugin.json" ] || return 1
  grep -q '"manufacturer": "MPFree"' "$1/plugin.json" && grep -q "\"uid\": \"$2\"" "$1/plugin.json"
}

same_file() {
  [ "$(sha256sum < "$1")" = "$(sha256sum < "$2")" ]
}

uid_of() {
  case "$1" in "$SOURCE_DIR") echo MFps ;; *) echo MFpt ;; esac
}

# ---- check ----------------------------------------------------------------

cmd_check() {
  echo "kit: $KIT"
  [ -f "$KIT/VERSION" ] && sed 's/^/  /' "$KIT/VERSION"
  kit_ok
  echo "  files match SHA256SUMS"
  echo "  mounted: $(mount_of "$KIT")"
  if [ -z "$WORK" ]; then
    find_data
    echo "data partition: $DATA_LINE"
    if [ -f "$DATA_MNT/FORMAT" ]; then echo "  FORMAT: $(head -n 1 "$DATA_MNT/FORMAT")"; else echo "  no FORMAT file (MPFree has not saved to it yet)"; fi
    WORK="$DATA_MNT/FXProbe"
  fi
  drive=${DRIVE:-$R/media/az01-internal}
  line=$(mount_line "$drive")
  echo "internal drive: ${line:-$drive is not mounted}"
  if [ -n "$line" ]; then
    set -- $line
    has_opt "$4" noexec && echo "  noexec: plugins would not load from it"
    for p in "$SOURCE_DIR" "$TAP_DIR"; do
      d="$drive/VST2 Plugins/$p"
      if [ ! -d "$d" ]; then echo "  $p: not installed"; continue; fi
      if ! ours "$d" "$(uid_of "$p")"; then echo "  $p: a folder of that name that is not FXProbe's"; continue; fi
      same=same
      for so in "$d"/VST/*.so; do
        rel="plugins/$p/VST/${so##*/}"
        want=$(awk -v r="$rel" 'substr($0, 67) == r { print $1 }' "$KIT/SHA256SUMS")
        have=$(sha256sum "$so" | cut -d' ' -f1)
        [ -n "$want" ] && [ "$want" = "$have" ] || same=different
      done
      echo "  $p: installed, $same as the kit; pointer $(head -n 1 "$d/workfolder.txt" 2>/dev/null || echo none)"
    done
  fi
  echo "work folder: $WORK"
  if [ -d "$WORK" ]; then
    [ -f "$WORK/$MARKER" ] && echo "  marked" || echo "  NOT marked (run setup)"
    echo "  programmes: $(ls "$WORK/programmes" 2>/dev/null | tr '\n' ' ')"
    echo "  captures: $(ls "$WORK/captures" 2>/dev/null | grep -c '\.wav$') WAV file(s)"
    n=0
    for f in "$R"/media/*/FXProbe/"$MARKER"; do [ -f "$f" ] && n=$((n + 1)); done
    [ "$n" -gt 1 ] && echo "  WARNING: $n marked FXProbe folders under /media; the plugins use none until only one is left"
    cmd_status
  else
    echo "  not made yet (run setup)"
  fi
  echo "CHECK-OK"
}

# ---- install / remove -----------------------------------------------------

writable_drive() {
  line=$(mount_line "$1")
  [ -n "$line" ] || stop "$1 is not a mount point"
  echo "drive: $line"
  set -- $line
  has_opt "$4" rw || stop "$2 is not mounted read-write"
  has_opt "$4" noexec && stop "$2 is mounted noexec: the MPC app could not load plugins from it"
  return 0
}

cmd_install() {
  drive=${DRIVE:-$R/media/az01-internal}
  kit_ok
  writable_drive "$drive"
  target="$drive/VST2 Plugins"
  for p in "$SOURCE_DIR" "$TAP_DIR"; do
    [ -d "$KIT/plugins/$p" ] || stop "the kit has no plugins/$p"
    if [ -e "$target/$p" ] && ! ours "$target/$p" "$(uid_of "$p")"; then
      stop "$target/$p exists and is not FXProbe's; nothing changed"
    fi
  done
  stage="$drive/.fxprobe-staging"
  rm -rf "$stage"
  mkdir "$stage" || stop "cannot make $stage; nothing changed"
  for p in "$SOURCE_DIR" "$TAP_DIR"; do
    if ! cp -r "$KIT/plugins/$p" "$stage/"; then rm -rf "$stage"; stop "copy of $p failed (disk full?); nothing changed"; fi
    printf '%s\n' "$POINTER" > "$stage/$p/workfolder.txt" || { rm -rf "$stage"; stop "cannot write the pointer; nothing changed"; }
  done
  grep '  plugins/' "$KIT/SHA256SUMS" | sed 's#  plugins/#  #' > "$stage/SHA256SUMS"
  out=$(cd "$stage" && sha256sum -c SHA256SUMS 2>&1) || {
    echo "$out" | grep -v ': OK$'; rm -rf "$stage"; stop "the copy does not match the kit; nothing changed"; }
  made=0
  if [ ! -d "$target" ]; then mkdir "$target" || { rm -rf "$stage"; stop "cannot make $target; nothing changed"; }; made=1; fi
  for p in "$SOURCE_DIR" "$TAP_DIR"; do
    if [ -d "$target/$p" ]; then
      if [ -f "$target/$p/fxprobe-install.txt" ] && grep -q '^made-vst2-folder 1$' "$target/$p/fxprobe-install.txt"; then made=1; fi
      mv "$target/$p" "$stage/old-$(uid_of "$p")" || stop "cannot move the old $p aside (nothing replaced yet for it)"
      echo "install: replacing the installed $p"
    fi
    printf 'installed-by fxprobe.sh\nmade-vst2-folder %s\n' "$made" > "$stage/$p/fxprobe-install.txt"
    mv "$stage/$p" "$target/$p" || stop "cannot move $p into $target (run install again, or remove)"
    echo "install: $target/$p"
  done
  rm -rf "$stage"
  sync
  echo "install: each plugin finds its work folder at $POINTER (the folder holding $MARKER)"
  echo "install: the MPC app lists them after its next start: switch the MPC off and on with the stick out"
  echo "INSTALL-OK"
}

cmd_remove() {
  drive=${DRIVE:-$R/media/az01-internal}
  target="$drive/VST2 Plugins"
  made=0
  for p in "$SOURCE_DIR" "$TAP_DIR"; do
    d="$target/$p"
    if [ ! -e "$d" ]; then echo "remove: $p is not installed"; continue; fi
    ours "$d" "$(uid_of "$p")" || { echo "remove: left $d alone: it is not FXProbe's"; continue; }
    grep -q '^made-vst2-folder 1$' "$d/fxprobe-install.txt" 2>/dev/null && made=1
    rm -rf "$d" || stop "cannot delete $d"
    echo "remove: deleted $d"
  done
  [ -d "$drive/.fxprobe-staging" ] && rm -rf "$drive/.fxprobe-staging" && echo "remove: deleted $drive/.fxprobe-staging"
  if [ "$made" = 1 ] && [ -d "$target" ] && rmdir "$target" 2>/dev/null; then
    echo "remove: deleted the empty $target, which install made"
  fi
  sync
  echo "REMOVE-OK"
}

# ---- the work folder ------------------------------------------------------

cmd_setup() {
  if [ -z "$WORK" ]; then
    work_folder
    # MPFree's store refuses a data partition that holds other files and no
    # FORMAT, so FXProbe/ goes only where MPFree has made its FORMAT.
    case "$(head -n 1 "$DATA_MNT/FORMAT" 2>/dev/null)" in
      "mpfree-data "*) ;;
      *) stop "$DATA_MNT has no MPFree FORMAT file: start MPFree from this stick once first (MPFree would not use a data partition holding FXProbe/ and no FORMAT), or give --work DIR" ;;
    esac
  fi
  parent=${WORK%/*}
  [ -d "$parent" ] || stop "$parent does not exist"
  for p in short standard; do
    [ -f "$KIT/programmes/$p.wav" ] && [ -f "$KIT/programmes/$p.tsv" ] || stop "the kit has no programmes/$p.wav and $p.tsv"
  done
  mkdir -p "$WORK/programmes" "$WORK/captures" "$WORK/status" || stop "cannot make $WORK"
  chmod 755 "$WORK" "$WORK/programmes" "$WORK/captures" "$WORK/status"
  for f in short.wav short.tsv standard.wav standard.tsv; do
    if [ -f "$WORK/programmes/$f" ] && same_file "$KIT/programmes/$f" "$WORK/programmes/$f"; then continue; fi
    cp "$KIT/programmes/$f" "$WORK/programmes/$f.tmp" && mv "$WORK/programmes/$f.tmp" "$WORK/programmes/$f" ||
      stop "cannot copy programmes/$f (disk full?)"
    echo "setup: programmes/$f"
  done
  printf 'FXProbe work folder (fxprobe.sh setup); delete with fxprobe.sh clean\n' > "$WORK/$MARKER" || stop "cannot write the marker"
  sync
  echo "setup: work folder $WORK; instances report in status/ within a second or two"
  echo "SETUP-OK"
}

# Value of KEY in a key-value file (first match), or nothing.
val() {
  [ -f "$1" ] && awk -v k="$2" '$1 == k { sub(/^[^ ]+ /, ""); print; exit }' "$1"
}

cmd_status() {
  work_folder
  found=0
  for f in "$WORK"/status/*.txt; do
    [ -f "$f" ] || continue
    found=1
    n=${f##*/}
    what=$(val "$f" capture)
    [ -n "$what" ] || what=$(val "$f" programme)
    err=$(val "$f" error)
    printf '%-14s state %-10s seq %-6s %s%s\n' "${n%.txt}" "$(val "$f" state)" "$(val "$f" seq)" "$what" "${err:+  error: $err}"
  done
  [ "$found" = 1 ] || echo "no instance has reported (are FXProbe Source and Tap on tracks, with the stick in?)"
}

# The next command number: one more than control.txt's.
next_seq() {
  last=$(val "$WORK/control.txt" seq)
  case "$last" in ''|*[!0-9]*) echo 1 ;; *) echo $((last + 1)) ;; esac
}

# Writes control.txt, one argument per line, through a temporary file and a
# rename, so the plugins never read half a command.
write_control() {
  printf '%s\n' "$@" > "$WORK/control.txt.tmp" && mv "$WORK/control.txt.tmp" "$WORK/control.txt" ||
    stop "cannot write control.txt"
}

# Seconds of a programme, from its schedule: the end of its last segment.
programme_seconds() {
  awk -F '\t' 'NR == 1 { rate = $2; next } { e = $3 + $4 + $5; if (e > m) m = e } END { if (rate > 0) printf "%.3f", m / rate }' "$1"
}

cmd_capture() {
  work_folder
  [ -f "$WORK/$MARKER" ] || stop "$WORK is not set up (run setup)"
  case "$NAME" in ''|.*|*[!A-Za-z0-9._-]*) stop "capture names are letters, digits, . _ and - (not starting with .)" ;; esac
  prog=$PROGRAMME
  [ -f "$WORK/programmes/$prog.wav" ] && [ -f "$WORK/programmes/$prog.tsv" ] || stop "no programmes/$prog.wav and $prog.tsv in $WORK"
  secs=$(programme_seconds "$WORK/programmes/$prog.tsv")
  [ -n "$secs" ] || stop "cannot read programmes/$prog.tsv"
  seconds=$(awk -v s="$secs" -v t="$TAIL" 'BEGIN { printf "%.3f", 0.5 + s + t + 0.5 }')
  taps="" sources="" ntaps=0 slotlist=""
  for f in "$WORK"/status/tap-s*.txt "$WORK"/status/source-s*.txt; do
    [ -f "$f" ] || continue
    n=${f##*/}; n=${n%.txt}; slot=${n#*-s}
    if [ -n "$SLOTS" ]; then case ",$SLOTS," in *",$slot,"*) ;; *) continue ;; esac; fi
    case "$n" in tap-*) taps="$taps $n"; ntaps=$((ntaps + 1)); slotlist="$slotlist $slot" ;; *) sources="$sources $n" ;; esac
  done
  [ "$ntaps" -gt 0 ] && [ -n "$sources" ] || stop "no Source and Tap instances reporting (status:$(ls "$WORK/status" 2>/dev/null | tr '\n' ' '))"
  ngains=$(echo "$GAINS" | awk -F, '{ print NF }')
  echo "slots:$slotlist; $ngains capture(s) of $seconds s each"
  # Each Source loads the programme first, so a slow read from the stick
  # can't start it late into a Tap's fixed-length recording.
  need=0
  for t in $sources; do [ "$(val "$WORK/status/$t.txt" programme)" = "$prog.wav" ] || need=1; done
  if [ "$need" = 1 ]; then
    seq=$(next_seq)
    write_control "seq $seq" "action load" "programme $prog.wav" ${SLOTS:+"slots $SLOTS"}
    deadline=$(( $(date +%s) + 60 ))
    while :; do
      sleep 1
      loaded=1 errs=""
      for t in $sources; do
        f="$WORK/status/$t.txt"
        e=$(val "$f" error)
        [ -n "$e" ] && errs="$errs $t: $e;"
        [ "$(val "$f" programme)" = "$prog.wav" ] || loaded=0
      done
      [ -z "$errs" ] || stop "${errs# }"
      [ "$loaded" = 1 ] && break
      [ "$(date +%s)" -gt "$deadline" ] && { cmd_status; stop "the Sources did not load $prog.wav within 60 s"; }
    done
    echo "loaded $prog.wav in every Source"
  fi
  rc=0
  for gain in $(echo "$GAINS" | tr ',' ' '); do
    g=$(awk -v g="$gain" 'BEGIN { printf "%g", g + 0 }')
    case "$g" in -*) tag="m${g#-}" ;; *) tag=$g ;; esac
    if [ "$ngains" -gt 1 ]; then name="$NAME-g$tag"; else name=$NAME; fi
    seq=$(next_seq)
    timing=""
    [ "$TIMING" = 1 ] && timing="timing 1"
    write_control "seq $seq" "action run" "programme $prog.wav" "gain $g" "preroll 0.5" "capture $name" \
      "seconds $seconds" ${timing:+"$timing"} ${SLOTS:+"slots $SLOTS"}
    deadline=$(( $(date +%s) + $(awk -v s="$seconds" 'BEGIN { printf "%d", 3 * s + 15 }') ))
    while :; do
      sleep 1
      done_n=0 errs=""
      for t in $taps $sources; do
        e=$(val "$WORK/status/$t.txt" error)
        [ -n "$e" ] && errs="$errs $t: $e;"
      done
      [ -z "$errs" ] || stop "${errs# }"
      for t in $taps; do
        f="$WORK/status/$t.txt"
        [ "$(val "$f" capture)" = "$name" ] && [ "$(val "$f" seq)" = "$seq" ] || continue
        case "$(val "$f" state)" in done|stopped) done_n=$((done_n + 1)) ;; esac
      done
      [ "$done_n" -eq "$ntaps" ] && break
      if [ "$(date +%s)" -gt "$deadline" ]; then
        cmd_status
        stop "timed out waiting for $name"
      fi
    done
    bad=""
    for t in $taps; do
      r="$WORK/captures/$name-${t#tap-}.txt"
      if [ "$(val "$r" overrun_frames)" != 0 ] || [ "$(val "$r" nonfinite_samples)" != 0 ] ||
         [ "$(val "$r" stopped_early)" != 0 ] || [ "$(val "$r" frames_written)" != "$(val "$r" frames_requested)" ]; then
        bad="$bad ${t#tap-}: written $(val "$r" frames_written) of $(val "$r" frames_requested), overrun $(val "$r" overrun_frames), non-finite $(val "$r" nonfinite_samples), stopped early $(val "$r" stopped_early);"
      fi
    done
    if [ -n "$bad" ]; then echo "captured $name from $ntaps slot(s); PROBLEMS:$bad"; rc=1
    else echo "captured $name from $ntaps slot(s)"; fi
  done
  sync
  [ "$rc" = 0 ] || exit 1
  echo "CAPTURE-OK"
}

cmd_clean() {
  work_folder
  case "$WORK" in */FXProbe) ;; *) stop "refusing to delete $WORK (not an FXProbe folder)" ;; esac
  if [ -d "$WORK" ]; then
    n=$(ls "$WORK/captures" 2>/dev/null | grep -c '\.wav$')
    rm -rf "$WORK" || stop "cannot delete $WORK"
    echo "clean: deleted $WORK ($n capture(s))"
  else
    echo "clean: $WORK is not there"
  fi
  sync
  echo "CLEAN-OK"
}

# ---- arguments ------------------------------------------------------------

WORK="" DRIVE="" NAME="" PROGRAMME=short GAINS=0 SLOTS="" TAIL=2 TIMING=0
if [ "$1" = --work ]; then [ -n "$2" ] || usage; WORK=${2%/}; shift 2; fi
MODE=$1
[ -n "$MODE" ] || usage
shift
[ "$MODE" = capture ] && { NAME=$1; [ -n "$NAME" ] || usage; shift; }
while [ $# -gt 0 ]; do
  case "$1" in
    --drive) [ -n "$2" ] || usage; DRIVE=${2%/}; shift 2 ;;
    --programme) PROGRAMME=$2; shift 2 ;;
    --gains) GAINS=$2; shift 2 ;;
    --slots) SLOTS=$2; shift 2 ;;
    --tail) TAIL=$2; shift 2 ;;
    --timing) TIMING=1; shift ;;
    *) usage ;;
  esac
done
case "$PROGRAMME" in short|standard) ;; *) stop "--programme is short or standard" ;; esac
case "$GAINS" in ''|*[!0-9.,-]*) stop "--gains is a list like 0,-12" ;; esac
case "$SLOTS" in *[!0-9,]*) stop "--slots is a list like 1,2" ;; esac
case "$TAIL" in ''|*[!0-9.]*) stop "--tail is seconds" ;; esac

case "$MODE" in
  check) cmd_check ;;
  install) cmd_install ;;
  remove) cmd_remove ;;
  setup) cmd_setup ;;
  status) cmd_status ;;
  capture) cmd_capture ;;
  clean) cmd_clean ;;
  *) usage ;;
esac
