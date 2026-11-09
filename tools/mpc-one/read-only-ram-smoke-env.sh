#!/bin/sh
# MPC One RAM-only executable prerequisites. Strictly READ-ONLY.
# Intended to be called through collect-over-ssh.sh after Ethernet/SSH identity
# verification. No mount, file creation, service mutation or device opens.
set -u
printf 'MPFREE RAM SMOKE ENVIRONMENT READ ONLY\n'
printf 'No software is installed, uploaded, started or modified by this survey.\n'

printf '\n=== /run mount type (exact mount point) ===\n'
run_type=''
if [ -r /proc/mounts ]; then
  while read -r source target fstype options rest; do
    if [ "$target" = "/run" ]; then
      run_type="$fstype"
      break
    fi
  done < /proc/mounts
fi
if [ -z "$run_type" ]; then
  printf 'run_mount_type=not-found\n'
else
  printf 'run_mount_type=%s\n' "$run_type"
fi

printf '\n=== Required external utilities ===\n'
# sh and printf/command are handled by the shell itself. Every tool below is
# used by the opt-in volatile launcher; missing tools must be reported by name.
for tool in awk mktemp cat sha256sum timeout nice chmod rm rmdir systemctl; do
  if command -v "$tool" >/dev/null 2>&1; then
    printf 'tool.%s=present\n' "$tool"
  else
    printf 'tool.%s=MISSING\n' "$tool"
  fi
done

printf '\n=== Factory application (read-only) ===\n'
if command -v systemctl >/dev/null 2>&1; then
  state="$(systemctl is-active acvs.service 2>/dev/null || :)"
  if [ -z "$state" ]; then state='unknown'; fi
  printf 'factory_service_active=%s\n' "$state"
  pid="$(systemctl show acvs.service -p MainPID --value 2>/dev/null || :)"
  case "$pid" in
    ''|0|*[!0-9]*) printf 'factory_main_pid=unavailable\n' ;;
    *) if [ -d "/proc/$pid" ]; then
         printf 'factory_main_pid=present\n'
       else
         printf 'factory_main_pid=not-running\n'
       fi ;;
  esac
else
  printf 'factory_service_active=unavailable\n'
  printf 'factory_main_pid=unavailable\n'
fi

printf '\n=== Interpreter and target kernel (read-only) ===\n'
if command -v uname >/dev/null 2>&1; then
  printf 'kernel_machine='
  uname -m 2>/dev/null || :
fi
if [ -e /lib/ld-linux-armhf.so.3 ]; then
  printf 'armhf_interpreter=present\n'
else
  printf 'armhf_interpreter=MISSING\n'
fi

printf 'END MPFREE RAM SMOKE ENVIRONMENT\n'
