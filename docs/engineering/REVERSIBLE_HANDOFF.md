# MPC One reversible factory/MPFree userspace handoff — safety design

**STATUS: READ-ONLY INVESTIGATION. No tested factory handoff, no verified independent boot recovery, and NO authorization to stop stock services in this phase.**

The factory user-facing unit `acvs.service` (launches `/usr/bin/az01-launch-MPC`) owns ACVA ALSA playback/capture, mixer and MIDI, DRM/KMS, touch and GPIO event device fds. Previous observations found both PCM subdevices unavailable while the factory app runs, and `Restart=always`. Network MIDI and script runner units also run. Read [factory resource map](../hardware/MPC_ONE_3_6_0_RESOURCES_20261109.md) and [44.1 kHz PCM survey](../hardware/MPC_ONE_3_6_0_AUDIO_RUNTIME_20261109.md).

## Read-only preflight (implemented)

On the developer workstation with the existing identity-checked root SSH lab setup:

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
sh -n tools/mpc-one/read-only-handoff-preflight.sh
bash tools/mpc-one/collect-over-ssh.sh --handoff-preflight
~~~

The script only reads systemd unit definitions/properties for `acvs.service`, `az01-network-midi.service` and `az01-script-runner.service`; current MainPID and selected `/proc` fd symlinks; ALSA PCM availability; device-node metadata; root/persistent overlay mounts; memory; boot arguments/partition names. It **never stops, starts, kills, reloads, masks, edits or disables services**, opens the PCM/MIDI/DRM devices, modifies partitions, remounts storage or binds USB gadgets.

Reports remain local and ignored at `research/mpc-one-handoff-preflight-*.txt`. **Raw reports can include kernel command lines, network addresses, service environment variables and device identifiers; review and redact before sharing. Never commit raw reports.** The owner **successfully ran this preflight on the actual MPC**; a sanitized findings note and raw report fingerprint are recorded in [2026-11-09 ABI/handoff survey](../hardware/MPC_ONE_3_6_0_ABI_HANDOFF_SURVEY_20261109.md). This confirms the *inspection*, not any stop/restore behavior.

## Measured service/resource details and resulting engineering constraints

The owner-run preflight confirms:

| Unit | Actual service behavior (snapshot) |
| --- | --- |
| `acvs.service` | `Type=simple`, `Restart=always` with **100 ms restart delay**, `KillMode=control-group`, **90-second stop timeout**, `Wants=az01-script-runner.service`, and `After=az01-network-midi.service az01-script-runner.service ...` |
| `az01-network-midi.service` | `Type=simple`, `Restart=always`, 90-second stop timeout, active; ordered before `acvs.service` |
| `az01-script-runner.service` | `Type=simple`, `Restart=no`, active; wanted and ordered before `acvs.service` |

**Do not stop the two helpers merely because they are ordered before `acvs`**: systemd ordering (`After=`) does not mean the unit requires or automatically shuts down those helpers. The factory GUI process itself owned `/dev/dri/card0`, the touchscreen/GPIO `event0/event1`, the ACVA playback/capture PCM endpoints, `/dev/snd/seq` and raw MIDI.

With **38 process threads, one observed leader affinity CPU 0, and `isolcpus=2-3`**, a thread-level survey is necessary before building the Rust realtime audio scheduler. Neither a single affinity line nor the isolation boot flag tells us which other threads receive SCHED_FIFO/RR or how the factory partitions the work.

A more serious scheduling/resource constraint is the **~1 GiB resident and locked factory process** on ~2 GiB RAM (no swap). A 64 MiB prepared-sample bank seems conservative, but does not prove MPFree will fit while stock software runs. We must measure worst-case working set with no unsupported assumptions about reclaiming mlocked pages.

The root is read-only, but `/etc` and `/var` are read-write overlays backed by `/data`. Any real handoff prototype must avoid those mounts and create *all* session state under volatile RAM-only locations.

### New per-thread passive survey

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
# Once this PR is merged, update your main checkout; until then test on a branch checkout.
bash tools/mpc-one/collect-over-ssh.sh --rtthreads
~~~

This reports the current factory process's per-thread names, CPU affinity, kernel scheduling policy/priority and limits. **It does not change CPU affinity, priorities or memory locks**. Any subsequent public writeup must redact machine/process IDs and retain a cryptographic hash of the owner's private raw report.

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

**Proposed (awaiting owner adoption):** [RECOVERY_AND_OS_STRATEGY.md](RECOVERY_AND_OS_STRATEGY.md) scopes gate 1 by tier. A session-only takeover with no eMMC writes (Tier 1) would need a full off-device backup and measured zero writes to `p1`–`p6`, not MaskROM-level recovery. Gates 2–6 are unchanged.

## A pure-Rust mock state machine now exists (SIMULATION ONLY)

The new `rust/mpfree-handoff-model` crate implements the following *simulated* guarded transitions:

`StockRunning → Qualified → Armed → StockQuiescing → ResourcesReleased → MpFreeActive → StockRestoring → StockVerified → StockRunning`.

Before `Qualified`, the mock requires **all six explicit flags**: firmware+board match, independently proven boot restore, factory project saved, owner consent, independent watchdog, and successful mock restoration. No single flag is optional. This is a **simulation of evidence**, not evidence that the real prerequisites hold; unit tests set flags true synthetically.

Once simulated resources are released, an owner cancel, an MPFree crash, transport failure, missed heartbeat, or a heartbeat delivered after deadline transitions to `StockRestoring`. Failure to verify factory unit/executable, PCM, input/display and user-visible stock interface results in terminal `RecoveryRequired`; it does not falsely claim `StockVerified`. A late heartbeat cannot revive an expired simulated session.

**This Rust crate has no filesystem, subprocess, systemd, network, ALSA or DRM API. It cannot stop, kill or launch anything.** CI checks this statically and runs state-machine unit tests on desktop. No production privileged systemd adapter exists.

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
cargo test --manifest-path rust/Cargo.toml --locked -p mpfree-handoff-model
~~~

The simulation moves us forward on failure semantics without changing the MPC's real device state.

## Proposed REAL hardware state machine (DESIGN; not implemented)

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
| Passive service dependency/ExecStop/KillMode/Restart and fd map | **Owner ran report; sanitized evidence documented** |
| Determine current kernel and runtime ABI compatibility | **Owner ran updated ABI survey; installed GNU libc release banner confirms version 2.39; per-binary dynamic linking remains unproven** |
| Mock service graph / watchdog timeout / restore test | **Implemented pure Rust simulator + unit tests; no real service manipulation** |
| Verify independent unbootable-device recovery | **Not done; hard blocker** |
| Consented manual session-only stock release/restore | **BLOCKED** |
| Use ACVA/DRM/MIDI with MPFree on actual MPC | **BLOCKED** |
| Persistent end-user installer or automatic USB takeover | **BLOCKED** |

**Next safe action:** owner runs the *revised* `--rtthreads` read-only survey for the missing scheduler policy/RT-priority fields and `--capacity` for actual CPU utilization and process memory proportions. The updated `--abi` has already explicitly confirmed libc 2.39. **Do not stop the stock service yet.** Independent recovery remains the critical blocker.
