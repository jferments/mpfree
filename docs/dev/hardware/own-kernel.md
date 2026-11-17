# MPFree's own kernel on the MPC One (plan)

**As of 2026-11-17** · **Status:** step 1's read-only survey ran on 2026-11-17 ([results](#step-1-results-2026-11-17)) and our kernel's settings were corrected from it; step 2's build (script, pinned compiler, CI workflow) is built, and CI built the first kernel on 2026-11-17: `6.18.57-rt9-mpfree` with gcc 14.4.0, all pinned. The owner chose this as the next step on the chain route on 2026-11-15 ("merge #374, then our own kernel"), and made the [three choices](#the-owners-decisions-2026-11-16) on 2026-11-16: 6.18 long-term with the RT patch, no request to inMusic for now, builds on the CI runners in a pinned container. · **Builds on:** the chain to the stick's own device tree ([step 7](stick-autoboot.md#the-chain-to-the-sticks-own-device-tree-step-7-opt-in), #374) and its one-try guard ([step 7b](stick-autoboot.md#step-7b-round-2026-11-15-zip-215433z-the-guard-passed), #381), both merged and opt-in.

## Goal

The chain's second kernel becomes one MPFree builds, so MPFree picks its own drivers, modules, kernel settings and kernel version. Everything else about the chain stays.

- **Today:** a `--chain` stick restarts from Akai's kernel into `/boot/mpfree-kexec/zImage` with the stick's own device tree. That `zImage` is still a byte-for-byte copy of Akai's kernel.
- **This plan:** put our own kernel in that one place. The hook, the chain, the device tree, the one-try guard and the soft-reboot fallback are unchanged.
- **Not in this plan:** making the chain the default. The owner sees the chain as the long-term default (isolation, our own kernel settings and modules; about 25 s to the pad player instead of 16 s is acceptable), with the soft-reboot kept as the fallback if Akai changes something. The switch waits for more power-ons and is a later, small PR of its own.

## What we know

Most of the MPC One runs on drivers that are in mainline Linux. The parts that need care are the sound card, whose driver is Akai's own, and possibly the display panel.

| Part | What Akai's kernel uses | Our kernel |
| --- | --- | --- |
| Sound card (DAC and ADC on Rockchip I2S `ff890000`) | Akai's own machine driver `jp07-audio` (`inmusic,acva-audio`) and codec `inmusic,acva-audio-codec`, set by pins only (enable, mute, reset, MCLK), no I2C | **Needs work:** a small replacement (mainline `simple-audio-card` plus a pin-driven codec driver), or Akai's source |
| Display (800×1280 DSI panel `urt,umo-p076md-t`, `mipi-backlight`) | Rockchip VOP and DSI (mainline); the panel's driver is Akai's own (`panel-urt-umo-p076md-t`, step 1) | **Needs work (step 4):** not in mainline, and its start-up sequence is in the driver, not the device tree. No picture on our kernel until then |
| GPU (Mali-T760) | Panfrost | Mainline |
| Touchscreen (ILI2117 at I2C 4-0026) | Akai's own driver `ili2117` (`CONFIG_TOUCHSCREEN_ILI2117`) | Mainline `ili210x` supports `ilitek,ili2117`, built in; axes to check with the screen (step 4) |
| Power chips (step 1) | ACT8846 (`act8865`: most supplies and the power-off), Silergy SYR827 and SYR828 (`fan53555`: the CPU's and the GPU's supply), HYM8563 clock; restart by the `gpio-restart` line and the PMU's reboot mode | Mainline, built in (our first build had a different PMIC's driver instead) |
| Pads, buttons, lights, controller MIDI | Internal USB (DWC2 at `ff540000`), USB MIDI | Mainline |
| Ethernet (RTL8211E PHY) | `stmmac` / `rk_gmac` | Mainline; already works on the stick's device tree (step 7) |
| USB stick | DWC2 and EHCI, `usb-storage` | Mainline |
| eMMC and SD card | `dw_mmc` | Mainline, **built as modules that nothing loads at start**; MPFree switches the SD card, or the eMMC read-only, on when asked ([plan](sd-card-and-internal-drive.md)) |
| Real-time | Linux 6.18.44 with the PREEMPT_RT patch set (`6.18.44-az01-2026-08-10-rt6`) | 6.18 needs the same patch set; mainline Linux supports PREEMPT_RT on 32-bit ARM from 7.1 ([Phoronix](https://phoronix.com/news/linux-7.1-arm-rt)) |

Other facts that shape the plan:

- Akai's full kernel configuration is built into its `zImage` (IKCONFIG). [`offline-kernel-config.py`](../../../tools/mpc-one/offline-kernel-config.py) reads it on the workstation, so our first kernel can start from Akai's exact settings.
- Cloud sessions can't reach kernel.org (the proxy refuses it) and have no ARM cross compiler or `dtc`. By the owner's choice, kernels are built by CI on the self-hosted mpfree-ci runners with a pinned toolchain, so agents can build and fix them without the owner; the owner fetches the hash-checked result for the stick. The first build run (2026-11-17) showed the runners reach kernel.org but have neither Docker nor Podman, so the build runs on the runner itself, as the runner's user, with the compiler pinned by hash (the fallback planned for that case): kernel.org's crosstool gcc 14, the same one inside the container where a container engine works.
- Modules are not required to be signed: 3.10.1's kernel file and its running kernel have no `CONFIG_MODULE_SIG` (step 1, 2026-11-17; an offline read of 3.10.0's found the same on 2026-11-10). It matters only for adding modules to Akai's kernel (the kexec-file route); our own kernel builds its own modules.
- On the chain's second kernel LoadPin is enforcing and pins the stick's root, and modules load from it (step 7). Our modules therefore go on the stick's root.
- The first kernel loads the second with the old `kexec_load` call, with `loadpin.enforce=0` on its command line (step 7). That call has no signature check, so our `zImage` needs no signature.
- Sources for the table: [audio and MIDI survey](audio-midi-survey.md), [touchscreen and display](touchscreen-display.md), and the device trees read with [`offline-dtb.py`](../../../tools/mpc-one/offline-dtb.py).

## Safety

Our kernel runs from RAM only and never writes the MPC's own storage. Pulling the stick and switching off always brings back MPC OS as it was.

- **The eMMC and SD card stay off unless MPFree asks.** The stick's device tree disables both controllers today. Our kernel has the MMC drivers (corrected by the owner on 2026-11-17: MPFree is to use the SD card and read the eMMC when asked), but as modules that nothing loads at start, with both controllers held so loading the driver binds neither. Our kernel also makes every eMMC disk read-only when it is created. How they are switched on and off: [the SD card and the internal drive](sd-card-and-internal-drive.md). Until that plan's step 4, our kernel's sticks keep both controllers out of the device tree, as today.
- **RAM only.** The kernel file sits on the stick and the chain loads it into memory. The [recovery strategy](../research/recovery-and-os-strategy.md) treats a kernel in RAM as research that leaves the eMMC untouched. Nothing here is persistent, so the Tier 3 gate (for persistent changes) does not apply.
- **One try per failed power-on.** The guard (#381) marks the try before the jump. If our kernel panics or hangs before it has run 30 s, the next power-on skips the chain once and MPFree starts on Akai's kernel instead.
- **The soft-reboot stays.** The default route, and the fallback if Akai changes something, is not touched by this plan.
- **Same checks every round.** Each round records the boot-area hashes before and after, as steps 6, 7 and 7b did, and passes only if they are unchanged with 0 sectors written.
- **Undo:** switch off and pull the stick, or rebuild the stick without the new option (or without `--chain`).

## Steps

Each step is its own small PR with owner blocks, verified before the owner gets them. Each keeps Akai's kernel copy as the default for `--chain`; our kernel is used only when the stick is built with a new opt-in option, so reverting is one rebuild.

1. **Read-only survey (no change to the MPC).** Tools built 2026-11-16; [blocks for the owner](#step-1-blocks-for-the-owner) below.
   - [`offline-own-kernel-survey.py`](../../../tools/mpc-one/offline-own-kernel-survey.py) (workstation only, read-only) reads Akai's kernel file, board file and module lists out of the root filesystem image the stick is built from. It prints the kernel's build settings (the symbols that matter for our kernel first, then the whole config, our starting point), whether modules must be signed, every symbol, module and device-tree node with a vendor name, and for every node of the board file its status and which of Akai's drivers claims it (from the device-tree aliases in the module lists). The nodes for the sound, screen, backlight, touch, GPU, USB, Ethernet and storage follow in full.
   - [`collect-over-ssh.sh --drivers`](../guides/survey-tools.md) (read-only, Akai's system on port 22) lists every device on every bus with the driver bound to it, the driver's module, and the device-tree node it comes from; the loaded modules, sound cards, screen, backlight, inputs and regulators; the kernel's messages about firmware and failed probes; and the running kernel's build settings from `/proc/config.gz`, with a hash that matches the offline survey's for the same kernel. `--stick-drivers` runs the same script on the stick's system (port 2222), so a `--chain` stick's second kernel (the stick's own device tree) can be compared with Akai's system line by line.
   - Together they answer the open question in [boot routes](boot-routes.md#what-the-next-runs-answer): whether `CONFIG_MODULE_SIG_FORCE` is set.
   - Result: this page gets the full parts list and the config we start from.
2. **First own kernel, no sound yet.**
   - Linux 6.18.y with the matching PREEMPT_RT patch, Akai's config as the starting point, minus Akai's own drivers, with the MMC drivers as modules (not loaded; the device tree still disables both controllers in this step), and `vfat` and `exfat`.
   - Built by CI on the mpfree-ci runners by [`tools/kernel/build.sh`](../../../tools/kernel/build.sh) ([how it runs](../guides/testing-and-ci.md#mpfrees-kernel-build)), with kernel.org's crosstool gcc 14 as the compiler; in the pinned container where Docker or Podman works, otherwise on the runner itself (the runners have neither). It checks the sources and the compiler (kernel.org's PGP signatures the first time, then the sha256 pinned in `tools/kernel/versions.env`), builds from the kernel's `multi_v7_defconfig` plus MPFree's own settings ([`tools/kernel/mpfree.config`](../../../tools/kernel/mpfree.config)), and stops if a setting did not take or the MMC drivers are built in. The owner fetches the result and checks its hash before the stick builder takes it. **Built 2026-11-17** (the script, the pinned compiler, the container and the workflow). The first kernel, the same day: Linux 6.18.57 with patch-6.18.57-rt9 (signed by Greg Kroah-Hartman and Sebastian Andrzej Siewior), built with kernel.org's gcc 14.4.0 (signed by Arnd Bergmann), all three now pinned by sha256; release `6.18.57-rt9-mpfree`, a 12.5 MB `zImage` and 914 modules, about 2.5 minutes on a runner. The settings start from mainline's ARM defaults rather than Akai's config, and were compared with Akai's on 2026-11-17 ([step 1's results](#step-1-results-2026-11-17)): every mainline driver Akai's kernel binds on the MPC One is now built in, and the job log lists every built setting ([`config-digest.py`](../../../tools/kernel/config-digest.py), a hash on each line so a copy can be checked), so the next comparisons need no owner block.
   - The stick builder takes it with an opt-in option, **built 2026-11-17**: `offline-kexec-stick.py --kexec-file --loadpin-off --chain --own-kernel DIR`, where DIR is a kernel folder from the CI VM's store (`/srv/mpfree-store/kernel/main`, copied with `scp -r`). It checks the folder against its `SHA256SUMS` and refuses a kernel that is not ours (release `<x.y.z>[-rtN]-mpfree`, pinned, `CONFIG-OK`), has kexec on or LoadPin not enforcing, or has the eMMC and SD card drivers built in, and a modules tarball with anything but plain files and folders under `lib/modules/<release>/`. Then `/boot/mpfree-kexec/zImage` is our zImage (a file in place of the link to Akai's, with its hash in that folder's `SHA256SUMS`, which `chain.sh` checks before the load), the modules go to `/usr/lib/modules/<release>/` on the stick's root (where LoadPin pins), `/boot/mpfree-kexec/build-info.txt` says which kernel it is, and `/etc/modprobe.d/mpfree-own-kernel.conf` keeps udev from loading the eMMC and SD card modules by alias (the two power-sequence nodes stay in the device tree; MPFree can still load the modules by name). The root filesystem grows by what the modules need. The hook, `chain.sh`, the guard, `kexec.dtb` and `cmdline.txt` are unchanged. The USB creator does not have the option yet.
   - Passes when it boots on two power-ons with Ethernet and SSH, LoadPin enforcing, no `mmcblk`, the `--app-user` privileges and unchanged boot hashes. No picture is expected (the panel's driver is Akai's own, step 4) and no sound (step 3), so the round checks over SSH and the stick's logs.
3. **Sound.** A replacement for Akai's sound driver (`simple-audio-card` plus a small codec driver for the enable, mute and reset pins), with care over pops at start and stop. Passes with the same latency and underrun measurements as step 7 (8.70 ms pad to sound, 0 underruns) and no audible pop.
4. **Display, if needed.** If the panel is not mainline, port its start-up sequence from Akai's device tree or source.
5. **Tuning.** Trim modules, add any firmware the hardware needs, and set the real-time options (timer, CPU frequency, IRQ threads) for MPFree.
6. **More power-ons, then the owner's decision on the default.** Repeated power-ons with the guard, a comparison with Akai's kernel on the same tests, then a separate small PR if the owner chooses to make the chain the default.

## The owner's decisions (2026-11-16)

| Decision | Options that were offered | The owner chose |
| --- | --- | --- |
| Which kernel version to start from | **6.18.y with the PREEMPT_RT patch**, the series Akai runs, or **mainline 7.1 or later**, where PREEMPT_RT is built in for 32-bit ARM (since 7.1, June 2026) | **6.18 long-term with the RT patch**, so only one thing changes at a time (our config and drivers, not the kernel version). Mainline can be a later step, as a rebuild. |
| Ask inMusic for their kernel source | Akai ships a GPL kernel, so the source is owed on request; it would include their sound driver and probably the panel driver | **Not now.** The sound driver (and the panel's start-up, if it needs one) is written from the device tree and our own measurements. |
| Where builds run | A pinned container on the self-hosted CI runners; a pinned container on the owner's workstation; or a cross compiler installed there | **A pinned container on the CI runners**, so agents can build and fix kernels without the owner, and every build is the same and checkable by hash. The runners turned out to have no container engine (first run, 2026-11-17), so they build without one, with the compiler pinned by hash, the fallback planned for that case. |

## Step 1: blocks for the owner

Both blocks are **read-only**: the first runs on the workstation only, the second reads the MPC over SSH and changes nothing on it. They need no stick. The MPC runs Akai's 3.10.1 with developer mode on, on the direct Ethernet cable, with developer mode's host key recorded (block Q1). Neither asks for a password. Until this pull request is merged, they fetch its branch; after that, `main` works the same.

**Block K1.1: update, check, the offline survey** (workstation only; nothing contacts the MPC). It reads the image the stick is built from (the newest `~/mpfree-work/akai-unpacked-*` folder) with `debugfs`, read-only.

```bash
cd /home/jesse/code/mpfree-hardware-survey && git fetch origin claude/project-thread-5ua3l6:refs/remotes/origin/claude/project-thread-5ua3l6 && git switch --no-track -C own-kernel origin/claude/project-thread-5ua3l6 && git log --oneline -1 && python3 tools/mpc-one/results.py run local-checks -- bash -c 'MPFREE_REQUIRE_OFFLINE_TOOLS=1 python3 tools/mpc-one/tests/test_offline_own_kernel_survey.py && MPFREE_REQUIRE_BUSYBOX=1 python3 tools/mpc-one/tests/test_read_only_drivers.py && echo LOCAL-CHECKS-OK' && python3 tools/mpc-one/results.py run own-kernel-survey -- python3 tools/mpc-one/offline-own-kernel-survey.py
```

It must say `LOCAL-CHECKS-OK`, then `kernel: /boot/zImage-6.18.44-az01-2026-08-10-rt6, …` and end with `END OF OWN KERNEL SURVEY`. On `STOP: …`, send the zip (below) anyway.

**Block K1.2: the drivers of Akai's system, then the zip** (read-only on the MPC, port 22; switch the MPC on without the stick first).

```bash
cd /home/jesse/code/mpfree-hardware-survey && python3 tools/mpc-one/results.py run drivers-akai -- env MPC_PORT=22 bash tools/mpc-one/collect-over-ssh.sh --drivers ; python3 tools/mpc-one/results.py zip
```

It must end with `SUCCESS: …mpc-one-drivers-….txt`. Send the zip. The second kernel's side (`--stick-drivers`) is read in the next chain round ([step 7c](stick-autoboot.md#blocks-for-the-owner-step-7c-more-power-ons)), because it needs a `--chain` power-on.

**Passes when** both reports end with their `END OF …` line, and the `sha256 of the whole config` lines of the two are the same (the running kernel is the image's). The `signed modules:` line answers the open question.

## Step 1 results (2026-11-17)

The owner ran blocks K1.1 and K1.2 at cf0941d2 (zip `mpfree-results-20261117T083639Z`, kept off the repository: the drivers report quotes this unit's device tree). Both passed: `LOCAL-CHECKS-OK`, both reports end with their `END OF …` line, and the `sha256 of the whole config` is the same in both (`997550953e16…`), so the kernel file in the image is the one the MPC runs.

- **Signed modules:** none required. There is no `CONFIG_MODULE_SIG` (the open question in [boot routes](boot-routes.md#what-the-next-runs-answer)).
- **Akai's kernel:** 6.18.44 with PREEMPT_RT, gcc 13.4.0 (Yocto's), `HZ=1000`, `NO_HZ_IDLE`, LoadPin enforcing (the only LSM it starts besides capabilities), kexec and the file-based kexec built in, the CPU governor `performance`, an oops is a panic, and `isolcpus=2-3` built into its command line. 1211 settings built in, 78 as modules.
- **Akai's own drivers:** the sound card (`snd_soc_rockchip_inmusic_jp07` and the codec `snd_soc_inmusic_jp07`, modules), the panel (`panel-urt-umo-p076md-t`), the touch driver (`ili2117`), the AC power input (`az01-power`), a boot counter (`syscon-bootcount`) and the USB OTG switch (`virtual-mux`). Our kernel has none of them: sound is step 3, the panel step 4, touch uses mainline's `ili210x`; the other three only report or keep a state the boot loader set, and the OTG switch's line stays where it is.
- **Everything else Akai binds is mainline**, and our kernel now builds it in ([`mpfree.config`](../../../tools/kernel/mpfree.config), each line checked after the build): the power chips (above), the USB PHY, power domains, the screen's IOMMU, I/O voltage domains, PL330 DMA, PWM (the backlight), fuses, ADC, the Rockchip timer, the UARTs, the LEDs and the power key. Without the power chips' and the PHY's drivers, every device that names one of their supplies, the USB controllers among them, would have waited for ever, and the stick's root would never have mounted.
- **Kept as Akai's kernel left it:** our kernel's command line adds `regulator_ignore_unused clk_ignore_unused` for the first power-ons, so the supplies and clocks of the parts whose drivers are Akai's own (the panel, the sound card) are not switched off under them. Step 5 takes the two words away.
- **Different on purpose:** kexec off, the eMMC and SD card drivers and exFAT as modules (the [plan](sd-card-and-internal-drive.md)), the I2S driver built in, `-mpfree` in the release.
- **Seen on the MPC (Akai's system, no stick):** the sound card's analog power, mute and reset lines are driven by Akai's codec driver. Our kernel has no driver for them, so they stay as they were at the chain's restart; whether Akai's codec driver had loaded on the first kernel by then is not known yet, so the first power-on with our kernel starts at low volume. Ethernet's PHY is the RTL8211E, and the controller's MIDI is on the internal DWC2 USB, as in the table above.

## Risks

| Risk | What it would look like | How we handle it |
| --- | --- | --- |
| Sound without Akai's driver | Pops at start or stop, or wrong levels | Test at low volume first; switch the mute pin before the clocks start and after they stop; measure latency and underruns against step 7. |
| A kernel that fails early | A dark screen and no log, because the log partition is not mounted yet | The owner switches off; the guard skips the chain on the next power-on. Keeping the failed boot's messages in RAM (ramoops) would let the next boot save them to the stick. |
| Panel driver not in mainline | No picture, while SSH and sound still work | Port the panel's start-up sequence in step 4; MPFree stays usable over SSH meanwhile. |
| Ethernet link timing | One link retry on some power-ons (`Failed to reset the dma`), as in steps 7 and 7b and earlier kexec boots | Since 2026-11-16 the stick starts eth0 again 2 s after that failure instead of at 20 s. Watch it each round; our kernel can change it either way (a PHY reset just before the first start is a candidate fix). |
| Keeping the kernel up to date | Security and stable fixes land upstream | The build script makes a rebuild one command; follow the 6.18 stable and RT releases. |
| Akai changes MPC OS | A new first kernel that refuses the chain | The chain stops at the first kernel and MPFree starts there, as today; the soft-reboot stays the fallback. |

## Related

- [Boot routes on MPC OS 3.10.1](boot-routes.md): why the chain is the only route to our own kernel without touching U-Boot.
- [Starting MPFree from the stick at power-on](stick-autoboot.md): steps 6, 7 and 7b, their runbooks and results.
- [How MPFree boots from the stick](soft-reboot-boot.md): the default route.
