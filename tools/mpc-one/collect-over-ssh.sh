#!/usr/bin/env bash
# Host-side runner for the read-only MPFree MPC One inventory.
# Defaults match the tested ML-tower -> MPC One direct Ethernet link.
# Writes reports ONLY on the workstation; sends a read-only POSIX sh program by stdin.
set -Eeuo pipefail
umask 077

MPC_IP="${MPC_IP:-10.42.0.141}"
MPC_IFACE="${MPC_IFACE:-eno1np0}"
MPC_HOST_IP_CIDR="${MPC_HOST_IP_CIDR:-10.42.0.1/24}"
MPC_EXPECT_MAC="${MPC_EXPECT_MAC:-00:05:95:02:d4:d2}"
MPC_PORT="${MPC_PORT:-2222}"
MPC_USER="${MPC_USER:-root}"
MPC_KEY="${MPC_KEY:-/home/jesse/.ssh/mpc_one_autorun_ed25519}"

SCRIPT_DIR="$(cd -- "$(dirname -- "$0")" && pwd -P)"
# Only reviewed, repository-local survey scripts and a bounded host-side benchmark are allowed.
# No arbitrary user-supplied script path or remote command is accepted.
case "${1:-}" in
  "") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-inventory.sh"; REPORT_KIND="mpc-one" ;;
  "--detail") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-detail.sh"; REPORT_KIND="mpc-one-detail" ;;
  "--runtime") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-runtime.sh"; REPORT_KIND="mpc-one-runtime" ;;
  "--io") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-io.sh"; REPORT_KIND="mpc-one-io" ;;
  "--abi") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-abi.sh"; REPORT_KIND="mpc-one-abi" ;;
  "--handoff-preflight") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-handoff-preflight.sh"; REPORT_KIND="mpc-one-handoff-preflight" ;;
  "--process-map") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-process-map.sh"; REPORT_KIND="mpc-one-process-map" ;;
  "--touchscreen") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-touchscreen.sh"; REPORT_KIND="mpc-one-touchscreen" ;;
  "--rtthreads") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-rtthreads.sh"; REPORT_KIND="mpc-one-rtthreads" ;;
  "--capacity") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-capacity.sh"; REPORT_KIND="mpc-one-capacity" ;;
  "--audio-midi") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-audio-midi.sh"; REPORT_KIND="mpc-one-audio-midi" ;;
  "--midi-activity") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-midi-activity" ;;
  "--midi-capture") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-midi-capture" ;;
  "--boot-map") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-boot-map.sh"; REPORT_KIND="mpc-one-boot-map" ;;
  "--boot-menu-survey") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-boot-menu.sh"; REPORT_KIND="mpc-one-boot-menu-survey" ;;
  "--kexec-preflight") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-kexec-preflight.sh"; REPORT_KIND="mpc-one-kexec-preflight" ;;
  "--ethernet") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-ethernet.sh"; REPORT_KIND="mpc-one-ethernet" ;;
  "--hook-canary") REMOTE_SCRIPT="$SCRIPT_DIR/hook-canary-install.sh"; REPORT_KIND="mpc-one-hook-canary" ;;
  "--hook-canary-remove") REMOTE_SCRIPT="$SCRIPT_DIR/hook-canary-remove.sh"; REPORT_KIND="mpc-one-hook-canary-remove" ;;
  "--hook-canary-report") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-hook-canary.sh"; REPORT_KIND="mpc-one-hook-canary-report" ;;
  "--hook-install") REMOTE_SCRIPT="$SCRIPT_DIR/hook-install.sh"; REPORT_KIND="mpc-one-hook-install" ;;
  "--hook-remove") REMOTE_SCRIPT="$SCRIPT_DIR/hook-remove.sh"; REPORT_KIND="mpc-one-hook-remove" ;;
  "--hook-report") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-hook-report.sh"; REPORT_KIND="mpc-one-hook-report" ;;
  "--hook-kexec-probe") REMOTE_SCRIPT="$SCRIPT_DIR/kexec-file-probe.sh"; REPORT_KIND="mpc-one-kexec-file-probe" ;;
  "--kexec-boot") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-kexec-boot" ;;
  "--app") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-app" ;;
  "--devmode-check") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-devmode-check" ;;
  "--app-start-capture") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-app-start-capture" ;;
  "--backup") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-emmc-backup" ;;
  "--ram-smoke-env") REMOTE_SCRIPT="$SCRIPT_DIR/read-only-ram-smoke-env.sh"; REPORT_KIND="mpc-one-smoke-env" ;;
  "--ram-smoke-check") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-smoke-preflight" ;;
  "--ram-smoke") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-smoke-execution" ;;
  "--headless-check") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-headless-preflight" ;;
  "--headless-smoke") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-headless-execution" ;;
  "--session-check") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-session-preflight" ;;
  "--session-smoke") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-session-execution" ;;
  "--touch-axes-check") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-touch-axes-preflight" ;;
  "--touch-axes") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-touch-axes-execution" ;;
  "--touch-capture-check") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-touch-capture-preflight" ;;
  "--touch-capture") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-touch-capture" ;;
  "--netbench") REMOTE_SCRIPT=""; REPORT_KIND="mpc-one-netbench" ;;
  *) printf 'ERROR: Unknown mode. Modes: default baseline, --detail, --runtime, --io (passive link/USB), --abi (read-only userspace), --handoff-preflight (read-only service map), --process-map (read-only process/unit and device ownership), --touchscreen (read-only ILI2117, DRM and input metadata), --rtthreads (read-only task RT/affinity), --capacity (2-second read-only CPU/memory), --boot-map (read-only boot chain, partition hashes, write counters, kexec/U-Boot/update metadata), --boot-menu-survey (read-only facts for the boot menu: framebuffer, controller MIDI card, who holds display/MIDI, acvs and script-runner units, autorun, kexec), --kexec-preflight (read-only kexec, memory, CPU and device-tree facts; copies the running device tree to ~/mpfree-work), --ethernet (read-only Ethernet controller, PHY, reset pin, clock and kernel-log details), --hook-canary (STATE-CHANGING, data partition only, port 22 only: three small files, a udev rule and a modprobe line in Akai'"'"'s writable /etc layer and the script they run, which only log at power-on and start a short service that logs too; needs MPFREE_ACCEPT_HOOK_CANARY=YES), --hook-canary-remove (its undo: deletes them), --hook-canary-report (read-only: did the canary run at power-on, and when), --hook-install (STATE-CHANGING, data partition only, port 22 only: MPFree'"'"'s boot hook, three small files in the same places as the canary'"'"'s, which it replaces; from the next power-on, with MPFree'"'"'s stick in, the MPC starts MPFree from the stick before Akai'"'"'s app; then a check with the stick that loads its kernel into RAM and unloads it; needs MPFREE_ACCEPT_HOOK=YES), --hook-remove (its undo: deletes them), --hook-report (read-only: what the hook did at this power-on), --kexec-boot (STATE-CHANGING, RAM only: boots the test build on the USB stick once by kexec; needs MPFREE_ACCEPT_KEXEC_BOOT=YES and a typed confirmation), --app (STATE-CHANGING, RAM only: Phase 1 mpfree on the running test build, each step started by Enter; needs MPFREE_ACCEPT_APP=YES; MPFREE_APP_STEPS picks steps, e.g. app (read-only: the pad player started at power-on), line (that pad player opens the line input after you type LINE) or pads,lights,drums,setup), --devmode-check (read-only on the 3.10.1 test build made with --akai-app: what Akai'"'"'s developer mode opens on port 22; only after you type LOCK, Akai'"'"'s own lock-down to your key, RAM only), --app-start-capture (STATE-CHANGING on stock 3.6: restarts Akai'"'"'s app once under a passive usbmon capture of the controller; needs MPFREE_ACCEPT_APP_RESTART=YES and a typed confirmation), --backup (read-only on the MPC: full eMMC image to the workstation, about 7.3 GiB), --audio-midi (read-only ALSA/MIDI/IRQ/RT survey), --midi-activity (guided read-only MIDI counter deltas; interactive), --midi-capture (opt-in passive usbmon capture of controller USB-MIDI; interactive), --ram-smoke-env (read-only utility probe), --ram-smoke-check (read-only ABI + tool checks), --ram-smoke (explicit RAM-only execute), --headless-check (read-only M6 ELF/ABI check), --headless-smoke (double opt-in M6 RAM-only execution), --session-check (read-only M7 ABI preflight), --session-smoke (double opt-in 2-second M7 null-only timing), --touch-axes-check (read-only M10 ABI/ILI2117 identity), --touch-axes (double-opt-in metadata-only EVIOCGABS), --touch-capture-check (read-only M12 preflight), --touch-capture (double-opt-in, step-by-step read-only touch capture), --netbench (opt-in active SSH load).\n' >&2
     exit 1 ;;
esac
[ "$#" -le 1 ] || { printf 'ERROR: Too many arguments\n' >&2; exit 1; }
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd -P)"
MPC_REPORT_DIR="${MPC_REPORT_DIR:-$REPO_ROOT/research}"
# Backups default to OUTSIDE the git clone: they hold keys, serial/MAC and user data,
# and `git clean -fdx` must not be able to delete them.
MPC_BACKUP_DIR="${MPC_BACKUP_DIR:-$HOME/mpfree-backups}"
BACKUP_BASELINE="$SCRIPT_DIR/baselines/mpc-one-3.6.0-reference.txt"

die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
command -v ip >/dev/null 2>&1 || die 'iproute2 not installed on workstation'
command -v ping >/dev/null 2>&1 || die 'ping missing on workstation'
command -v ssh >/dev/null 2>&1 || die 'OpenSSH client missing on workstation'
command -v timeout >/dev/null 2>&1 || die 'timeout missing on workstation'
command -v sha256sum >/dev/null 2>&1 || die 'sha256sum missing on workstation'
if [[ "${1:-}" == "--touch-axes-check" || "${1:-}" == "--touch-axes" ]]; then
  command -v python3 >/dev/null 2>&1 || die 'Python3 missing on workstation'
  command -v arm-linux-gnueabihf-readelf >/dev/null 2>&1 || die 'ARM hard-float readelf missing'
  for helper in host_touch_axes.py host_ram_smoke.py check-arm-abi.py \
                read-only-abi.sh read-only-ram-smoke-env.sh \
                read-only-touch-axes-env.sh; do
    [[ -r "$SCRIPT_DIR/$helper" ]] || die "missing M10 preflight: $helper"
  done
  touch_binary="$REPO_ROOT/rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-touch"
  [[ -f "$touch_binary" && -r "$touch_binary" ]] ||
    die 'build exact ARMv7 mpfree-touch release binary before M10 preflight'
  if [[ "${1:-}" == "--touch-axes" && (
        "${MPFREE_ACCEPT_VOLATILE_EXEC:-}" != 'YES' ||
        "${MPFREE_ACCEPT_TOUCH_AXES:-}" != 'YES' ) ]]; then
    die 'M10 dual opt-in required: MPFREE_ACCEPT_VOLATILE_EXEC=YES MPFREE_ACCEPT_TOUCH_AXES=YES bash tools/mpc-one/collect-over-ssh.sh --touch-axes'
  fi
elif [[ "${1:-}" == "--backup" ]]; then
  command -v python3 >/dev/null 2>&1 || die 'python3 is required for --backup on workstation'
  for helper in host_emmc_backup.py read-only-backup-preflight.sh; do
    [[ -r "$SCRIPT_DIR/$helper" ]] || die "missing backup helper: $helper"
  done
  [[ -r "$BACKUP_BASELINE" ]] || die "missing backup baseline: $BACKUP_BASELINE"
  backup_abs="$(realpath -m -- "$MPC_BACKUP_DIR")"
  case "$backup_abs/" in
    "$REPO_ROOT"/*)
      git -C "$REPO_ROOT" check-ignore -q -- "$backup_abs/probe" 2>/dev/null ||
        die "MPC_BACKUP_DIR is inside the git clone and not git-ignored: $backup_abs"
      ;;
  esac
elif [[ "${1:-}" == "--kexec-preflight" || "${1:-}" == "--kexec-boot" || "${1:-}" == "--app" ||
        "${1:-}" == "--devmode-check" ]]; then
  command -v python3 >/dev/null 2>&1 || die "python3 is required for ${1:-} on workstation"
  for helper in read-only-kexec-preflight.sh offline-dtb.py host_kexec_boot.py host_app_mpc.py \
                host_devmode_check.py; do
    [[ -r "$SCRIPT_DIR/$helper" ]] || die "missing kexec helper: $helper"
  done
  if [[ "${1:-}" == "--kexec-boot" ]]; then
    [[ -t 0 ]] || die '--kexec-boot asks you to confirm: run it from a terminal'
    [[ "${MPFREE_ACCEPT_KEXEC_BOOT:-}" == 'YES' ]] ||
      die 'Opt-in required (boots the test build from the USB stick; RAM only): MPFREE_ACCEPT_KEXEC_BOOT=YES bash tools/mpc-one/collect-over-ssh.sh --kexec-boot'
  fi
  if [[ "${1:-}" == "--devmode-check" ]]; then
    [[ -t 0 ]] || die '--devmode-check asks you before the lock-down: run it from a terminal'
  fi
  if [[ "${1:-}" == "--app" ]]; then
    [[ -t 0 ]] || die '--app waits for you to press Enter before each step: run it from a terminal'
    [[ "${MPFREE_ACCEPT_APP:-}" == 'YES' ]] ||
      die 'Opt-in required (runs mpfree on the test build from the USB stick; RAM only): MPFREE_ACCEPT_APP=YES bash tools/mpc-one/collect-over-ssh.sh --app'
  fi
  # The running device tree carries the unit's serial number and MAC: keep it outside the clone.
  work_abs="$(realpath -m -- "${MPFREE_WORK_DIR:-$HOME/mpfree-work}")"
  case "$work_abs/" in
    "$REPO_ROOT"/*)
      git -C "$REPO_ROOT" check-ignore -q -- "$work_abs/probe" 2>/dev/null ||
        die "MPFREE_WORK_DIR is inside the git clone and not git-ignored: $work_abs" ;;
  esac
  [[ "$work_abs" != *[[:space:]\"\']* ]] || die "MPFREE_WORK_DIR must not contain spaces or quotes: $work_abs"
elif [[ "${1:-}" == "--touch-capture-check" || "${1:-}" == "--touch-capture" ]]; then
  command -v python3 >/dev/null 2>&1 || die 'Python3 missing on workstation'
  command -v arm-linux-gnueabihf-readelf >/dev/null 2>&1 || die 'ARM hard-float readelf missing'
  for helper in host_touch_capture.py host_ram_smoke.py check-arm-abi.py \
                read-only-abi.sh read-only-ram-smoke-env.sh \
                read-only-touch-capture-env.sh; do
    [[ -r "$SCRIPT_DIR/$helper" ]] || die "missing M12 preflight: $helper"
  done
  capture_binary="$REPO_ROOT/rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-touch-capture"
  [[ -f "$capture_binary" && -r "$capture_binary" ]] ||
    die 'build the ARMv7 mpfree-touch release binaries before the M12 preflight'
  if [[ "${1:-}" == "--touch-capture" && (
        "${MPFREE_ACCEPT_VOLATILE_EXEC:-}" != 'YES' ||
        "${MPFREE_ACCEPT_TOUCH_CAPTURE:-}" != 'YES' ) ]]; then
    die 'M12 dual opt-in required: MPFREE_ACCEPT_VOLATILE_EXEC=YES MPFREE_ACCEPT_TOUCH_CAPTURE=YES bash tools/mpc-one/collect-over-ssh.sh --touch-capture'
  fi
elif [[ "${1:-}" == "--midi-activity" ]]; then
  command -v python3 >/dev/null 2>&1 || die 'python3 is required for --midi-activity on workstation'
  [[ -r "$SCRIPT_DIR/host_midi_activity.py" ]] || die 'Missing host_midi_activity.py'
  [[ -r "$SCRIPT_DIR/read-only-midi-counters.sh" ]] || die 'Missing read-only-midi-counters.sh'
  [[ -t 0 ]] || die '--midi-activity is interactive: run it from a terminal'
elif [[ "${1:-}" == "--midi-capture" ]]; then
  # Which guided phases: controller (default), midi-cv (the DIN MIDI jacks and the CV/Gate jacks)
  # cv-scale (known keyboard notes into the CV track, and MIDI Out again), midi-out (MIDI Out only)
  # or transport (the lights Akai's app sends for REC, OVERDUB, PLAY, PLAY START and STOP).
  MIDI_CAPTURE_PHASES="${MPFREE_CAPTURE_PHASES:-controller}"
  case "$MIDI_CAPTURE_PHASES" in
    controller|midi-cv|cv-scale|midi-out|transport) ;;
    *) die "MPFREE_CAPTURE_PHASES must be controller, midi-cv, cv-scale, midi-out or transport, not: $MIDI_CAPTURE_PHASES" ;;
  esac
  command -v python3 >/dev/null 2>&1 || die 'python3 is required for --midi-capture on workstation'
  [[ -r "$SCRIPT_DIR/host_midi_capture.py" ]] || die 'Missing host_midi_capture.py'
  [[ -r "$SCRIPT_DIR/passive-usbmon-capture.sh" ]] || die 'Missing passive-usbmon-capture.sh'
  [[ -t 0 ]] || die '--midi-capture is interactive: run it from a terminal'
  [[ "${MPFREE_ACCEPT_USBMON:-}" == 'YES' ]] ||
    die 'Opt-in required (passive usbmon reader, max 240 s): MPFREE_ACCEPT_USBMON=YES bash tools/mpc-one/collect-over-ssh.sh --midi-capture'
elif [[ "${1:-}" == "--app-start-capture" ]]; then
  command -v python3 >/dev/null 2>&1 || die 'python3 is required for --app-start-capture on workstation'
  for helper in host_app_start_capture.py host_kexec_boot.py host_midi_capture.py; do
    [[ -r "$SCRIPT_DIR/$helper" ]] || die "missing helper: $helper"
  done
  [[ -t 0 ]] || die '--app-start-capture waits for you at each step: run it from a terminal'
  [[ "${MPFREE_ACCEPT_APP_RESTART:-}" == 'YES' ]] ||
    die "Opt-in required (restarts Akai's app once on stock 3.6 while a passive usbmon reader records the controller): MPFREE_ACCEPT_APP_RESTART=YES bash tools/mpc-one/collect-over-ssh.sh --app-start-capture"
elif [[ "${1:-}" == "--netbench" ]]; then
  [[ -r "$SCRIPT_DIR/network-bench.py" ]] || die "Missing host-only network benchmark: $SCRIPT_DIR/network-bench.py"
  command -v python3 >/dev/null 2>&1 || die "python3 is required for --netbench on workstation"
elif [[ "${1:-}" == "--ram-smoke-check" || "${1:-}" == "--ram-smoke" || "${1:-}" == "--headless-check" || "${1:-}" == "--headless-smoke" || "${1:-}" == "--session-check" || "${1:-}" == "--session-smoke" ]]; then
  command -v python3 >/dev/null 2>&1 || die 'python3 is required for RAM smoke check'
  command -v arm-linux-gnueabihf-readelf >/dev/null 2>&1 || die 'ARM GNU readelf not installed on workstation'
  [[ -r "$SCRIPT_DIR/host_ram_smoke.py" ]] || die 'Missing guarded RAM smoke orchestrator'
  [[ -r "$SCRIPT_DIR/check-arm-abi.py" ]] || die 'Missing host ARM ABI checker'
  [[ -r "$SCRIPT_DIR/read-only-abi.sh" ]] || die 'Missing read-only target ABI probe'
  [[ -r "$SCRIPT_DIR/read-only-ram-smoke-env.sh" ]] || die 'Missing read-only RAM environment probe'
  if [[ "${1:-}" == "--headless-check" || "${1:-}" == "--headless-smoke" ]]; then
    smoke_binary="$REPO_ROOT/rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-headless"
    smoke_profile="headless"
  elif [[ "${1:-}" == "--session-check" || "${1:-}" == "--session-smoke" ]]; then
    smoke_binary="$REPO_ROOT/rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-session"
    smoke_profile="session"
  else
    smoke_binary="$REPO_ROOT/rust/target/armv7-unknown-linux-gnueabihf/release/mpfree-device-smoke"
    smoke_profile="device-smoke"
  fi
  [[ -f "$smoke_binary" && -r "$smoke_binary" ]] ||
    die "Missing cross-compiled $smoke_profile ELF; build that exact ARM target before launch"
  if [[ "${1:-}" == "--ram-smoke" && "${MPFREE_ACCEPT_VOLATILE_EXEC:-}" != 'YES' ]]; then
    die 'Explicit opt-in required: MPFREE_ACCEPT_VOLATILE_EXEC=YES bash tools/mpc-one/collect-over-ssh.sh --ram-smoke'
  fi
  if [[ "${1:-}" == "--headless-smoke" && (
        "${MPFREE_ACCEPT_VOLATILE_EXEC:-}" != 'YES' ||
        "${MPFREE_ACCEPT_HEADLESS_EXEC:-}" != 'YES' ) ]]; then
    die 'Explicit dual opt-in required: MPFREE_ACCEPT_VOLATILE_EXEC=YES MPFREE_ACCEPT_HEADLESS_EXEC=YES bash tools/mpc-one/collect-over-ssh.sh --headless-smoke'
  fi
  if [[ "${1:-}" == "--session-smoke" && (
        "${MPFREE_ACCEPT_VOLATILE_EXEC:-}" != 'YES' ||
        "${MPFREE_ACCEPT_SESSION_EXEC:-}" != 'YES' ) ]]; then
    die 'Explicit dual opt-in required: MPFREE_ACCEPT_VOLATILE_EXEC=YES MPFREE_ACCEPT_SESSION_EXEC=YES bash tools/mpc-one/collect-over-ssh.sh --session-smoke'
  fi
else
  [[ -f "$REMOTE_SCRIPT" && -r "$REMOTE_SCRIPT" ]] || die "Missing read-only inventory script: $REMOTE_SCRIPT"
fi
[[ -f "$MPC_KEY" && -r "$MPC_KEY" ]] || die "SSH private key is missing or unreadable: $MPC_KEY"

# MPC_PORT=22 is Akai's developer mode on an installed 3.10.1 (or later): Akai's own SSH server, which
# lets root in with no key and no password until Akai's lock-down. Only the modes that stream one
# read-only script run there. Its host key is pinned in a known_hosts file of its own, outside the
# clone, on first contact (behind the MAC check below) and must match on every later run.
DEVMODE=0
case "${1:-}" in
  --hook-*) [[ "$MPC_PORT" == 22 ]] ||
    die "${1:-} is for Akai's developer mode on an installed 3.10.1: run it with MPC_PORT=22" ;;
esac
if [[ "${1:-}" == "--hook-canary" && "${MPFREE_ACCEPT_HOOK_CANARY:-}" != 'YES' ]]; then
  die "Opt-in required (writes three small files to the MPC's data partition; undo: --hook-canary-remove): MPC_PORT=22 MPFREE_ACCEPT_HOOK_CANARY=YES bash tools/mpc-one/collect-over-ssh.sh --hook-canary"
fi
if [[ "${1:-}" == "--hook-install" && "${MPFREE_ACCEPT_HOOK:-}" != 'YES' ]]; then
  die "Opt-in required (writes MPFree's boot hook, three small files, to the MPC's data partition; undo: --hook-remove): MPC_PORT=22 MPFREE_ACCEPT_HOOK=YES bash tools/mpc-one/collect-over-ssh.sh --hook-install"
fi
if [[ "${1:-}" == "--hook-kexec-probe" && "${MPFREE_ACCEPT_KEXEC_PROBE:-}" != 'YES' ]]; then
  die "Opt-in required (RAM only: loads a kernel into memory and unloads it to test which load LoadPin accepts; never restarts the MPC, writes no disk): MPC_PORT=22 MPFREE_ACCEPT_KEXEC_PROBE=YES bash tools/mpc-one/collect-over-ssh.sh --hook-kexec-probe"
fi
if [[ "$MPC_PORT" == 22 ]]; then
  DEVMODE=1
  [[ -n "$REMOTE_SCRIPT" ]] ||
    die "${1:-} does not run on developer mode's port 22; only the modes that stream one script do (the read-only reports such as --boot-map and --kexec-preflight, and the --hook ones). Nothing was run."
  command -v ssh-keygen >/dev/null 2>&1 || die 'ssh-keygen missing on workstation'
  devmode_work="$(realpath -m -- "${MPFREE_WORK_DIR:-$HOME/mpfree-work}")"
  case "$devmode_work/" in
    "$REPO_ROOT"/*)
      git -C "$REPO_ROOT" check-ignore -q -- "$devmode_work/probe" 2>/dev/null ||
        die "MPFREE_WORK_DIR is inside the git clone and not git-ignored: $devmode_work" ;;
  esac
  DEVMODE_KNOWN="$devmode_work/mpc-devmode-known_hosts"
fi
[[ -n "$MPC_EXPECT_MAC" && -n "$MPC_HOST_IP_CIDR" ]] || die 'Device identity gates must be supplied'

if ! ip -4 addr show dev "$MPC_IFACE" | grep -Fq "inet $MPC_HOST_IP_CIDR"; then
  die "Expected $MPC_HOST_IP_CIDR not found on $MPC_IFACE"
fi
route="$(ip -4 route get "$MPC_IP" 2>/dev/null)" || die 'No IPv4 route to MPC'
case "$route" in
  *" dev $MPC_IFACE "*) ;;
  *) die "IPv4 route to MPC is not through $MPC_IFACE: $route" ;;
esac
ping -I "$MPC_IFACE" -c 1 -W 2 "$MPC_IP" >/dev/null || die 'MPC does not answer direct-interface ICMP'

# Keep the device identity gate: never silently accept missing neighbor data.
# In Linux, ARP records can be absent/transient despite a successful ping.
# Read both netlink and /proc/net/arp; safely refresh with bounded ICMP retries.
# The parser checks exact IP + interface and finds "lladdr" by name, not $5.
PARSER="$SCRIPT_DIR/lib/neighbor-identity.sh"
[[ -r "$PARSER" ]] || die "Missing neighbor parser: $PARSER"
# shellcheck source=lib/neighbor-identity.sh
source "$PARSER"

found_mac=""
for attempt in 1 2 3 4; do
  neighbor_lines="$(
    {
      ip -4 neigh show dev "$MPC_IFACE" || :
      ip -4 neigh get "$MPC_IP" dev "$MPC_IFACE" || :
    } 2>/dev/null
  )"
  netlink_mac="$(printf '%s\n' "$neighbor_lines" |
    mpfree_mac_from_ip_neigh_lines "$MPC_IP" "$MPC_IFACE" | sort -u)"
  proc_mac=""
  if [[ -r /proc/net/arp ]]; then
    proc_mac="$(mpfree_mac_from_proc_arp_lines "$MPC_IP" "$MPC_IFACE" < /proc/net/arp | sort -u)"
  fi

  if [[ -n "$netlink_mac" && -n "$proc_mac" && "$netlink_mac" != "$proc_mac" ]]; then
    die "Conflicting neighbor identities for $MPC_IP on $MPC_IFACE: ip=$netlink_mac, proc=$proc_mac"
  fi
  found_mac="${netlink_mac:-$proc_mac}"
  if [[ -n "$found_mac" ]]; then
    break
  fi
  if (( attempt < 4 )); then
    printf 'No complete ARP neighbor entry yet (attempt %s/4); refreshing with ICMP...\n' "$attempt" >&2
    sleep 1
    ping -I "$MPC_IFACE" -c 1 -W 2 "$MPC_IP" >/dev/null ||
      die "MPC stopped responding on $MPC_IFACE during identity check"
  fi
done

if [[ -z "$found_mac" ]]; then
  printf 'Diagnostic ip-neigh output:\n' >&2
  ip -4 neigh show dev "$MPC_IFACE" >&2 || true
  printf 'Diagnostic /proc/net/arp record(s):\n' >&2
  if [[ -r /proc/net/arp ]]; then
    awk -v ip="$MPC_IP" -v dev="$MPC_IFACE" 'NR==1 || ($1==ip && $6==dev)' /proc/net/arp >&2
  fi
  die "No complete ARP/MAC identity for $MPC_IP on $MPC_IFACE (not a mismatched MAC). No SSH or MPC commands were run. Do not bypass the identity check."
fi
if [[ ! "$found_mac" =~ ^([[:xdigit:]]{2}:){5}[[:xdigit:]]{2}$ ]]; then
  die "Unexpected MAC format from neighbor tables: $found_mac"
fi
if [[ "$found_mac" != "${MPC_EXPECT_MAC,,}" ]]; then
  die "Device identity mismatch: expected ${MPC_EXPECT_MAC,,}, observed $found_mac"
fi

# The SSH options every read-only report uses. On port 22 the host key is checked against its own file:
# accepted and saved only when that file has none for this address yet, strictly after that.
ssh_options() {
  ssh_opts=(-T -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 -o ForwardAgent=no
            -o ClearAllForwardings=yes -i "$MPC_KEY" -p "$MPC_PORT")
  if (( DEVMODE )); then
    ssh_opts=(-F /dev/null "${ssh_opts[@]}" -o "UserKnownHostsFile=$DEVMODE_KNOWN" -o GlobalKnownHostsFile=/dev/null
              -o CheckHostIP=no -o UpdateHostKeys=no -o "StrictHostKeyChecking=$1")
  else
    ssh_opts+=(-o StrictHostKeyChecking=yes)
  fi
}
if (( DEVMODE )); then
  mkdir -p -- "$devmode_work"
  if [[ -f "$DEVMODE_KNOWN" ]] && ssh-keygen -F "$MPC_IP" -f "$DEVMODE_KNOWN" >/dev/null 2>&1; then
    echo "Developer mode SSH (port 22): its host key must match the one saved in $DEVMODE_KNOWN"
    ssh_options yes
  else
    echo "Developer mode SSH (port 22), first contact: its host key will be saved in $DEVMODE_KNOWN"
    ssh_options accept-new
  fi
else
  ssh_options yes
fi

mkdir -p -- "$MPC_REPORT_DIR"
REPORT="$(mktemp "$MPC_REPORT_DIR/${REPORT_KIND}-$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX.txt")"
echo "Verified direct Ethernet: $MPC_IFACE ($MPC_HOST_IP_CIDR) -> $MPC_IP ($found_mac)"
if [[ "${1:-}" == "--midi-capture" ]]; then
  echo "OWNER OPT-IN: passive usbmon capture of the internal controller (max 240 s) to $REPORT"
elif [[ "${1:-}" == "--midi-activity" ]]; then
  echo "Guided READ-ONLY MIDI counter map (procfs reads only) to $REPORT"
elif [[ "${1:-}" == "--netbench" ]]; then
  echo "Saving ACTIVE NETWORK BENCHMARK measurements to $REPORT"
elif [[ "${1:-}" == "--backup" ]]; then
  echo "READ-ONLY on the MPC: full eMMC backup. Log: $REPORT"
elif [[ "${1:-}" == "--kexec-boot" ]]; then
  echo "OWNER OPT-IN: kexec test boot from the USB stick (RAM only on the MPC). Log: $REPORT"
elif [[ "${1:-}" == "--app" ]]; then
  echo "OWNER OPT-IN: mpfree on the test build (RAM only on the MPC). Log: $REPORT"
elif [[ "${1:-}" == "--hook-canary" ]]; then
  echo "OWNER OPT-IN, STATE-CHANGING on the data partition only: the boot-hook canary (three small files). Log: $REPORT"
elif [[ "${1:-}" == "--hook-canary-remove" ]]; then
  echo "STATE-CHANGING on the data partition only: removing the boot-hook canary. Log: $REPORT"
elif [[ "${1:-}" == "--hook-install" ]]; then
  echo "OWNER OPT-IN, STATE-CHANGING on the data partition only: MPFree's boot hook (three small files), then a check with the stick (RAM only). Log: $REPORT"
elif [[ "${1:-}" == "--hook-remove" ]]; then
  echo "STATE-CHANGING on the data partition only: removing MPFree's boot hook. Log: $REPORT"
elif [[ "${1:-}" == "--hook-kexec-probe" ]]; then
  echo "OWNER OPT-IN, RAM ONLY: probe which kexec load LoadPin accepts (loads then unloads a kernel; never restarts). Log: $REPORT"
elif [[ "${1:-}" == "--devmode-check" ]]; then
  echo "Developer mode check on the test build (read-only; RAM only after a typed LOCK). Log: $REPORT"
elif [[ "${1:-}" == "--app-start-capture" ]]; then
  echo "OWNER OPT-IN: restart Akai's app once on stock 3.6 under a passive usbmon capture. Log: $REPORT"
elif [[ "${1:-}" == "--touch-axes-check" ]]; then
  echo "READ-ONLY M10 exact MPC/ILI2117/ARM ABI preflight (NO upload): $REPORT"
elif [[ "${1:-}" == "--touch-capture-check" ]]; then
  echo "READ-ONLY M12 MPC/ILI2117/ARM ABI preflight (NO upload): $REPORT"
elif [[ "${1:-}" == "--touch-capture" ]]; then
  echo "OWNER DOUBLE OPT-IN: step-by-step read-only touch capture from /run: $REPORT"
elif [[ "${1:-}" == "--touch-axes" ]]; then
  echo "OWNER DOUBLE OPT-IN: volatile M10 metadata-only EVIOCGABS /run: $REPORT"
elif [[ "${1:-}" == "--ram-smoke-check" || "${1:-}" == "--headless-check" || "${1:-}" == "--session-check" ]]; then
  echo "READ-ONLY ELF/ABI compatibility preflight (no remote upload) to $REPORT"
elif [[ "${1:-}" == "--ram-smoke" || "${1:-}" == "--headless-smoke" || "${1:-}" == "--session-smoke" ]]; then
  echo "Explicit VOLATILE /run Rust execution, ONLY after live ABI validation: $REPORT"
else
  echo "Collecting passive/read-only report to $REPORT"
fi

if [[ "${1:-}" == "--touch-axes-check" || "${1:-}" == "--touch-axes" ]]; then
  touch_args=(
    --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" --key "$MPC_KEY"
    --binary "$touch_binary"
    --abi-script "$SCRIPT_DIR/read-only-abi.sh"
    --abi-checker "$SCRIPT_DIR/check-arm-abi.py"
    --env-script "$SCRIPT_DIR/read-only-ram-smoke-env.sh"
    --touch-script "$SCRIPT_DIR/read-only-touch-axes-env.sh"
  )
  if [[ "${1:-}" == "--touch-axes" ]]; then
    touch_args+=(--execute)
  fi
  if ! timeout 90 python3 "$SCRIPT_DIR/host_touch_axes.py" \
      "${touch_args[@]}" > "$REPORT" 2>&1; then
    echo "M10 guarded touch metadata failed/blocked; PRIVATE report: $REPORT" >&2
    sed -n '1,50p' "$REPORT" >&2
    exit 1
  fi
  sed -n '1,85p' "$REPORT"
elif [[ "${1:-}" == "--backup" ]]; then
  echo "Copies the whole eMMC (about 7.3 GiB) and both boot areas into a new folder under $MPC_BACKUP_DIR."
  echo "The MPC only reads. Expect about 4 minutes (first run: 193 s); keep the MPC idle (no playback or recording)."
  tool_version="$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  backup_rc=0
  timeout 7200 python3 "$SCRIPT_DIR/host_emmc_backup.py" \
    --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" --key "$MPC_KEY" \
    --out-root "$MPC_BACKUP_DIR" --baseline "$BACKUP_BASELINE" \
    --preflight-script "$SCRIPT_DIR/read-only-backup-preflight.sh" \
    --tool-version "$tool_version" 2>&1 | tee "$REPORT" || backup_rc=$?
  if (( backup_rc != 0 )); then
    echo "Backup did not finish cleanly (status $backup_rc). Log: $REPORT" >&2
    exit "$backup_rc"
  fi
elif [[ "${1:-}" == "--touch-capture-check" || "${1:-}" == "--touch-capture" ]]; then
  capture_args=(
    --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" --key "$MPC_KEY"
    --binary "$capture_binary"
    --abi-script "$SCRIPT_DIR/read-only-abi.sh"
    --abi-checker "$SCRIPT_DIR/check-arm-abi.py"
    --env-script "$SCRIPT_DIR/read-only-ram-smoke-env.sh"
    --capture-script "$SCRIPT_DIR/read-only-touch-capture-env.sh"
    --report "$REPORT"
  )
  if [[ "${1:-}" == "--touch-capture" ]]; then
    capture_args+=(--execute)
  fi
  # Not redirected: the touch prompts and Enter key must reach the terminal.
  # The helper writes the same lines to $REPORT itself. --foreground keeps
  # the helper in the terminal's process group so it can read Enter and gets
  # Ctrl-C; 1800 s is an overall ceiling while the owner works through steps.
  if ! timeout --foreground 1800 python3 "$SCRIPT_DIR/host_touch_capture.py" "${capture_args[@]}" \
      2> >(tee -a "$REPORT" >&2); then
    echo "M12 touch capture failed/blocked; PRIVATE report: $REPORT" >&2
    exit 1
  fi
  events_log="${REPORT%.txt}.events.log"
  if [[ -f "$events_log" ]]; then
    sha256sum "$events_log" > "$events_log.sha256"
  fi
elif [[ "${1:-}" == "--kexec-boot" ]]; then
  # Not redirected: the confirmation must reach the terminal. The helper writes the same lines to $REPORT.
  if ! timeout --foreground 1800 python3 -I "$SCRIPT_DIR/host_kexec_boot.py" \
      --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" --key "$MPC_KEY" \
      --iface "$MPC_IFACE" --expect-mac "$MPC_EXPECT_MAC" --work "$work_abs" --report "$REPORT"; then
    echo "The kexec test boot stopped or did not pass its checks; log: $REPORT" >&2
    exit 1
  fi
elif [[ "${1:-}" == "--app" ]]; then
  # Not redirected: each step waits for Enter in the terminal. The helper writes the same lines to $REPORT.
  # Up to three hours: the app step's phases each wait for you, and all nineteen took longer than one
  # hour in live test 3. MPFREE_APP_PHASES picks fewer.
  if ! timeout --foreground 10800 python3 -I "$SCRIPT_DIR/host_app_mpc.py" \
      --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" --key "$MPC_KEY" \
      --work "$work_abs" --report "$REPORT"; then
    echo "mpfree stopped early or a step failed; log: $REPORT" >&2
    exit 1
  fi
elif [[ "${1:-}" == "--devmode-check" ]]; then
  # Not redirected: the LOCK question must reach the terminal. The helper writes the same lines to $REPORT.
  if ! timeout --foreground 600 python3 -I "$SCRIPT_DIR/host_devmode_check.py" \
      --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" --key "$MPC_KEY" \
      --work "$work_abs" --report "$REPORT"; then
    echo "The developer mode check stopped; log: $REPORT" >&2
    exit 1
  fi
elif [[ "${1:-}" == "--app-start-capture" ]]; then
  # Not redirected: each step waits in the terminal. The helper writes the same lines to $REPORT.
  if ! timeout --foreground 1200 python3 -I "$SCRIPT_DIR/host_app_start_capture.py" \
      --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" --key "$MPC_KEY" --report "$REPORT"; then
    echo "The app start capture stopped early or a check failed; log: $REPORT" >&2
    exit 1
  fi
elif [[ "${1:-}" == "--midi-capture" ]]; then
  if ! timeout --foreground 600 python3 "$SCRIPT_DIR/host_midi_capture.py" \
      --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" --key "$MPC_KEY" \
      --script "$SCRIPT_DIR/passive-usbmon-capture.sh" --report "$REPORT" \
      --phases "$MIDI_CAPTURE_PHASES"; then
    echo "MIDI capture stopped or failed; partial report kept: $REPORT" >&2
    exit 1
  fi
elif [[ "${1:-}" == "--midi-activity" ]]; then
  # --foreground keeps the prompts readable from the terminal.
  if ! timeout --foreground 900 python3 "$SCRIPT_DIR/host_midi_activity.py" \
      --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" --key "$MPC_KEY" \
      --script "$SCRIPT_DIR/read-only-midi-counters.sh" --report "$REPORT"; then
    echo "MIDI activity map stopped or failed; partial report kept: $REPORT" >&2
    exit 1
  fi
elif [[ "${1:-}" == "--netbench" ]]; then
  echo "ACTIVE BENCHMARK REQUESTED: two 4 MiB SSH data transfers plus 20 ICMP pings."
  echo "Avoid running while recording/playing; it may briefly load the MPC CPU and network."
  if ! timeout 95 python3 "$SCRIPT_DIR/network-bench.py" \
    --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" \
    --key "$MPC_KEY" --iface "$MPC_IFACE" > "$REPORT" 2>&1; then
    echo "Benchmark failed safely; output retained: $REPORT" >&2
    sed -n '1,50p' "$REPORT" >&2
    exit 1
  fi
elif [[ "${1:-}" == "--ram-smoke-check" || "${1:-}" == "--ram-smoke" || "${1:-}" == "--headless-check" || "${1:-}" == "--headless-smoke" || "${1:-}" == "--session-check" || "${1:-}" == "--session-smoke" ]]; then
  smoke_args=(
    --ip "$MPC_IP" --port "$MPC_PORT" --user "$MPC_USER" --key "$MPC_KEY"
    --binary "$smoke_binary" --profile "$smoke_profile"
    --abi-script "$SCRIPT_DIR/read-only-abi.sh"
    --abi-checker "$SCRIPT_DIR/check-arm-abi.py"
    --env-script "$SCRIPT_DIR/read-only-ram-smoke-env.sh"
  )
  if [[ "${1:-}" == "--ram-smoke" || "${1:-}" == "--headless-smoke" || "${1:-}" == "--session-smoke" ]]; then smoke_args+=(--execute); fi
  if ! timeout 90 python3 "$SCRIPT_DIR/host_ram_smoke.py" "${smoke_args[@]}" > "$REPORT" 2>&1; then
    echo "RAM smoke failed/blocked; PRIVATE report retained: $REPORT" >&2
    sed -n '1,35p' "$REPORT" >&2
    exit 1
  fi
  sed -n '1,45p' "$REPORT"
else
if ! timeout 180 ssh "${ssh_opts[@]}" "$MPC_USER@$MPC_IP" 'sh -s' < "$REMOTE_SCRIPT" > "$REPORT" 2>&1; then
  if [[ "${1:-}" == --hook-* ]] && grep -q '^RESULT: ' "$REPORT"; then
    sed -n '1,80p' "$REPORT" >&2
    echo "The step stopped; see its RESULT line above. Log: $REPORT" >&2
    exit 1
  fi
  echo "SSH or remote inventory failed; report retained for diagnosis: $REPORT" >&2
  sed -n '1,30p' "$REPORT" >&2
  if (( DEVMODE )) && grep -q 'Host key verification failed' "$REPORT"; then
    echo "Developer mode's SSH answered with another host key than the one saved in $DEVMODE_KNOWN, so nothing" >&2
    echo "was run on it. Its key changes only if the MPC's /data was reset or the MPC was reinstalled. If that" >&2
    echo "happened, remove the old key with: ssh-keygen -R $MPC_IP -f $DEVMODE_KNOWN  and run this again." >&2
  elif (( DEVMODE )) && grep -qE 'Connection refused|Permission denied|timed out|No route' "$REPORT"; then
    echo "On port 22 this needs Akai's developer mode switched on (SHIFT + MENU > Developer)." >&2
  else
    echo "For a passphrase-protected key, first run: ssh-add $MPC_KEY" >&2
  fi
  exit 1
fi
fi
if (( DEVMODE )); then
  ssh_options yes   # the key is saved now: every later connection must match it
  printf 'Developer mode SSH host key: %s\n' "$(ssh-keygen -l -F "$MPC_IP" -f "$DEVMODE_KNOWN" | grep -v '^#' | head -n 1)"
fi

if [[ "${1:-}" == --hook-* ]]; then
  sed -n '1,80p' "$REPORT"
fi

if [[ "${1:-}" == "--kexec-preflight" ]]; then
  # Read-only on the MPC: `cat /sys/firmware/fdt` and `uname -r`. The comparison runs here.
  ssh_ro=(timeout 60 ssh "${ssh_opts[@]}" "$MPC_USER@$MPC_IP")
  mkdir -p -- "$work_abs"
  fdt_dir="$(mktemp -d "$work_abs/kexec-preflight-$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")"
  running="$("${ssh_ro[@]}" 'uname -r')" || die "could not read the running kernel release; report: $REPORT"
  [[ "$running" =~ ^[A-Za-z0-9._+-]+$ ]] || die "unexpected kernel release from the MPC; report: $REPORT"
  "${ssh_ro[@]}" 'cat /sys/firmware/fdt' > "$fdt_dir/live.dtb" ||
    die "could not copy the running device tree; report: $REPORT"
  printf '%s\n' "$running" > "$fdt_dir/release.txt"  # which boot-files folder live.dtb came from
  {
    echo
    echo "=== Running device tree, compared on the workstation ==="
    echo "private binary copy (identifies the unit, never share it): $fdt_dir/live.dtb"
    python3 -I "$SCRIPT_DIR/offline-dtb.py" summary "$fdt_dir/live.dtb" || echo "(could not read the copied device tree)"
    shopt -s nullglob
    releases=("$work_abs"/boot-files/*/)
    shopt -u nullglob
    (( ${#releases[@]} )) ||
      echo "(no Akai boot files under $work_abs/boot-files: run offline-rootfs-survey.sh, with and without --from-backup, first)"
    for folder in "${releases[@]}"; do
      shopt -s nullglob
      dtbs=("$folder"*.dtb)
      shopt -u nullglob
      (( ${#dtbs[@]} )) || continue
      while read -r ref; do
        [[ -n "$ref" ]] || continue
        if [[ "$folder" == "$work_abs/boot-files/$running/" ]]; then
          python3 -I "$SCRIPT_DIR/offline-dtb.py" diff "$ref" "$fdt_dir/live.dtb"
        else
          printf '%s vs the running tree: ' "$ref"
          python3 -I "$SCRIPT_DIR/offline-dtb.py" diff "$ref" "$fdt_dir/live.dtb" | tail -n 1
        fi
      done < <(python3 -I "$SCRIPT_DIR/offline-dtb.py" match inmusic,acva "${dtbs[@]}")
    done
  } >> "$REPORT" 2>&1
  printf 'Running device tree copied to %s (private).\n' "$fdt_dir/live.dtb"
fi

sha256sum "$REPORT" > "$REPORT.sha256"
printf '\nSUCCESS: %s\n' "$REPORT"
printf 'SHA256: '; sha256sum "$REPORT" | awk '{print $1}'
printf 'Review/redact this report before sharing; NEVER commit raw logs or device identifiers.\n'
