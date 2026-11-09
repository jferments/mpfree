# MPC One reversible factory/MPFree userspace handoff — safety design

**STATUS: READ-ONLY INVESTIGATION. No tested factory handoff, no verified independent boot recovery, and NO authorization to stop stock services in this phase.**

The factory user-facing unit `acvs.service` (launches `/usr/bin/az01-launch-MPC`) owns ACVA ALSA playback/capture, mixer and MIDI, DRM/KMS, touch and GPIO event device fds. Previous observations found both PCM subdevices unavailable while the factory app runs, and `Restart=always`. Network MIDI and script runner units also run. Read [factory resource map](../hardware/MPC_ONE_3_6_0_RESOURCES_20261109.md) and [44.1 kHz PCM survey](../hardware/MPC_ONE_3_6_0_AUDIO_RUNTIME_20261109.md).

## Read-only preflight (implemented)

On the developer workstation with the existing identity-checked root SSH lab setup:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin bootstrap/architecture-and-hardware-survey
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
sh -n tools/mpc-one/read-only-handoff-preflight.sh
bash tools/mpc-one/collect-over-ssh.sh --handoff-preflight
~~~

The script only reads systemd unit definitions/properties for `acvs.service`, `az01-network-midi.service` and `az01-script-runner.service`; current MainPID and selected `/proc` fd symlinks; ALSA PCM availability; device-node metadata; root/persistent overlay mounts; memory; boot arguments/partition names. It **never stops, starts, kills, reloads, masks, edits or disables services**, opens the PCM/MIDI/DRM devices, modifies partitions, remounts storage or binds USB gadgets.

Reports remain local and ignored at `research/mpc-one-handoff-preflight-*.txt`. **Raw reports can include kernel command lines, network addresses, service environment variables and device identifiers; review and redact before sharing. Never commit raw reports.** This mode will be tested on the owner's MPC only after they run it; CI syntax/safety tests are not an on-device execution.

## Important systemd semantic correction

For `Restart=always`, crashing or unexpectedly killing the service can cause an automatic restart. **A service explicitly stopped by systemd with `systemctl stop` is not automatically restarted because of `Restart=always`**; systemd documents this exception. This does *not* mean stopping the factory MPC is safe: dependencies, `ExecStop` cleanup, audio state, active saves, and recovery remain unverified. See https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html .

There are deliberately **no executable stop/start commands here**. Handoff remains a design gated behind owner-verified restoration.

## Hard prerequisites before ANY live takeover

1. **Independent recovery proof**: externally stored, checksum-verified eMMC/boot data and a tested procedure to restore even when factory Linux cannot boot. A successful stock SSH login or untested image file is not enough.
2. **Exact-device and firmware allowlist**: check board ID, firmware, kernel, service unit content, boot model and storage map; fail closed if anything is unknown or changed.
3. **Owner intent and factory quiescence**: confirm project saved, no current recording, exports or file writes; require explicit per-session consent *after* an actionable rollback plan is displayed.
4. **Separately supervised volatile test session**: independently running watchdog in RAM (`/run` or equivalent), not inside MPFree's render process and not dependent on the laptop/SSH bridge. All changes must be volatile. A local owner cancellation method is mandatory.
5. **Demonstrated fake/test restoration first**: model service dependency graph and fd ownership; test simulated MPFree crash, timeout, frozen audio thread, missing packets, loss of network/USB bridge, failed stock launch and watchdog failure. None of these tests can prove survival of a kernel crash or unbootable eMMC.
6. **Verified restorable factory resources**: explicit evidence that the factory releases ACVA PCM/DRM/input/MIDI safely, and that restoration reliably reopens them and returns working touch, pads, UI and audio. Do this only in a later, separately authorized real-device test.

**As of today gates 1, 4, 5 and 6 are NOT proven. Any device takeover is blocked.**

## Proposed state machine (DESIGN; not implemented)

~~~text
STOCK_RUNNING
  -> STOCK_QUALIFIED (all recovery, model, version, state and owner gates)
  -> SESSION_ARMED (watchdog ready and restore plan verified)
  -> STOCK_QUIESCING (only after a separate approved real-device experiment)
  -> RESOURCES_RELEASED (kernel confirms ACVA, MIDI, DRM/input ownership)
  -> MPFREE_SESSION (bounded health monitor; no persistent services)
  -> MPFREE_TERMINATING (cancel, crash, timeout, health loss)
  -> STOCK_RESTORING (close MPFree fds; restore saved original unit states)
  -> STOCK_VERIFIED (stock UI+audio+input functional; user confirms)
  -> STOCK_RUNNING
~~~

Every transition needs an explicit guard and finite timeout. On an error after releasing stock resources, an independent supervisor attempts stock restoration. **Do not promise that software watchdogs can guarantee recovery from power loss, unbootable storage or a kernel failure.** An independently tested flash/boot recovery mechanism remains essential.

### Planned implementation boundaries

- `HandoffSupervisor` should run out-of-band from the real-time audio thread. A Rust state machine and a mock service/device adapter should be developed on desktop first; the first real backend must fail closed unless a signed and explicit owner-gated test is enabled.
- At first all MPFree binaries, logs, watchdog and temporary units must remain in volatile memory; **never edit the factory `/etc` or `/var` overlays**, which persist even though `/` mounts read-only.
- If MPFree fails to start or misses a heartbeat, an independent supervisor would close MPFree's device handles, attempt restoration of **only services that were active before takeover**, and verify ownership and UI/PCM behavior. The watchdog must not live in the same failure domain as the MPFree process.
- Direct external DSP/laptop packets must never trigger service-management transitions. Treat all remote peers and plugins as untrusted.
- Preserve user projects and licensing state. No proprietary binary replacement, bootloader change, eMMC write or factory service masking is part of the initial tests.

## Research milestones

| Experiment | Current state |
| --- | --- |
| Passive service dependency/ExecStop/KillMode/Restart and fd map | **Script committed, awaiting owner run** |
| Determine current kernel and runtime ABI compatibility | **ABI survey and desktop checker committed, awaiting owner report** |
| Mock service graph / watchdog timeout / restore test | Planned; no real service manipulation |
| Verify independent unbootable-device recovery | **Not done; hard blocker** |
| Consented manual session-only stock release/restore | **BLOCKED** |
| Use ACVA/DRM/MIDI with MPFree on actual MPC | **BLOCKED** |
| Persistent end-user installer or automatic USB takeover | **BLOCKED** |

**Next action:** have the owner run `--handoff-preflight` and attach the output for redacted documentation. Do not stop the stock service yet.
