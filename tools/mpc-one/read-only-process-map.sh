#!/bin/sh
# MPFree MPC One passive stock process/service/resource ownership map.
# NO service writes, signals, device opens, mounts or scheduling changes.
# Reports are private and saved only to host ignored research directory.
set -u
printf 'MPFREE FACTORY PROCESS SERVICE MAP - READ ONLY\n'
printf 'PIDs and unit metadata may expose private local details; do not publish raw.\n'
printf 'No individual factory thread is a separately restorable service.\n'

printf '\n=== Process tree: 180 PID cap; NO argv or environment collected ===\n'
count=0
for proc in /proc/[0-9]*; do
    [ -r "$proc/status" ] || continue
    count=$((count + 1))
    if [ "$count" -gt 180 ]; then printf 'PROCESS_CAP_REACHED=180\n'; break; fi
    pid=$(basename "$proc")
    printf '\n--- PID=%s ---\n' "$pid"
    grep -E '^(Name|State|Tgid|PPid|Threads|VmRSS|VmLck):' "$proc/status" 2>/dev/null || :
    if [ -L "$proc/exe" ]; then
        printf 'exe='
        readlink "$proc/exe" 2>/dev/null || :
    fi
    if [ -r "$proc/cgroup" ]; then
        printf 'cgroup='
        sed -n '1p' "$proc/cgroup" 2>/dev/null || :
    fi
done

printf '\n=== Active/inactive service names (first 120 only) ===\n'
if command -v systemctl >/dev/null 2>&1; then
    systemctl list-units --type=service --all --no-pager --no-legend \
        --plain 2>/dev/null | sed -n '1,120p' || :
else
    printf 'systemctl=unavailable\n'
fi

printf '\n=== Installed az01 unit filenames only ===\n'
for unit in /usr/lib/systemd/system/az01*.service \
            /usr/lib/systemd/system/acvs.service \
            /etc/systemd/system/az01*.service \
            /etc/systemd/system/acvs.service; do
    [ -e "$unit" ] || [ -L "$unit" ] || continue
    printf '%s\n' "$unit"
done

printf '\n=== Known factory units: lifecycle, dependency and helper MainPID ===\n'
for unit in acvs.service az01-network-midi.service \
            az01-script-runner.service az01-power-button.service \
            az01-unsupported-device-daemon.service az01-usbsata-fixer.service; do
    printf '\n--- unit=%s ---\n' "$unit"
    if command -v systemctl >/dev/null 2>&1; then
        systemctl show "$unit" --no-pager \
            -p Id -p LoadState -p ActiveState -p SubState -p Type \
            -p FragmentPath -p DropInPaths -p MainPID -p ExecMainPID \
            -p ExecStart -p ExecStartPre -p ExecStop -p ExecStopPost \
            -p Restart -p RestartUSec -p TimeoutStopUSec \
            -p KillMode -p SendSIGKILL -p RemainAfterExit \
            -p Requires -p Requisite -p Wants -p PartOf -p BindsTo \
            -p ConsistsOf -p StopWhenUnneeded -p Conflicts \
            -p After -p Before -p ConditionResult 2>/dev/null || :
    else
        printf 'systemctl=unavailable\n'
    fi
done

printf '\n=== Factory MainPID and its shared-address-space threads ===\n'
pid=''
if command -v systemctl >/dev/null 2>&1; then
    pid=$(systemctl show acvs.service -p MainPID --value 2>/dev/null || :)
fi
case "$pid" in
    ''|0|*[!0-9]*) printf 'FACTORY_MAINPID=unavailable\n' ;;
    *)
        if [ -r "/proc/$pid/status" ]; then
            printf 'FACTORY_MAINPID=%s\n' "$pid"
            grep -E '^(Name|Tgid|PPid|Threads|State|VmRSS|VmLck):' \
                "/proc/$pid/status" 2>/dev/null || :
            printf 'Factory main executable path: '
            readlink "/proc/$pid/exe" 2>/dev/null || :
            printf 'Factory task names (first 80; NOT separate processes):\n'
            n=0
            for tid in /proc/"$pid"/task/[0-9]*; do
                [ -r "$tid/comm" ] || continue
                n=$((n+1))
                [ "$n" -le 80 ] || break
                printf '%s: ' "$(basename "$tid")"
                cat "$tid/comm" 2>/dev/null || :
            done
        else
            printf 'FACTORY_MAINPID=disappeared-during-survey\n'
        fi ;;
esac

printf '\n=== Device fd ownership by process (NO device opens) ===\n'
process_count=0
for proc in /proc/[0-9]*; do
    [ -d "$proc/fd" ] || continue
    process_count=$((process_count+1))
    [ "$process_count" -le 180 ] || break
    reported=0
    fd_count=0
    for fd in "$proc"/fd/*; do
        [ -L "$fd" ] || continue
        fd_count=$((fd_count+1))
        [ "$fd_count" -le 256 ] || break
        target=$(readlink "$fd" 2>/dev/null || :)
        case "$target" in
            /dev/snd/*|/dev/dri/*|/dev/input/*|/dev/fb*|\
            /dev/tty0|/dev/tty1|/dev/hidraw*|/dev/gpiochip*)
                if [ "$reported" -eq 0 ]; then
                    printf '\nPID=%s ' "$(basename "$proc")"
                    cat "$proc/comm" 2>/dev/null || :
                    reported=1
                fi
                printf '  FD=%s -> %s\n' "$(basename "$fd")" "$target"
                ;;
        esac
    done
done

printf '\n=== Factory service state recheck ===\n'
if command -v systemctl >/dev/null 2>&1; then
    systemctl show acvs.service --no-pager -p ActiveState -p SubState -p MainPID \
        2>/dev/null || :
fi
printf 'END MPFREE FACTORY PROCESS SERVICE MAP\n'
printf 'No factory service/process or device was modified.\n'
