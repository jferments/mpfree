# MPFree's own kernel on the MPC One (plan)

**As of 2026-11-16** · **Status:** plan, nothing built yet. The owner chose this as the next step on the chain route on 2026-11-15 ("merge #374, then our own kernel"); three choices below are still the owner's. · **Builds on:** the chain to the stick's own device tree ([step 7](stick-autoboot.md#the-chain-to-the-sticks-own-device-tree-step-7-opt-in), #374) and its one-try guard ([step 7b](stick-autoboot.md#step-7b-round-2026-11-15-zip-215433z-the-guard-passed), #381), both merged and opt-in.

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
| Display (800×1280 DSI panel, `mipi-backlight`) | Rockchip VOP and DSI (mainline); panel driver not yet identified | **To check:** mainline if the panel uses a standard driver, else its start-up sequence must be ported |
| GPU (Mali-T760) | Panfrost | Mainline |
| Touchscreen (ILI2117) | Driver named `ili2117` | Mainline `ili210x` supports `ilitek,ili2117`; axes to check |
| Pads, buttons, lights, controller MIDI | Internal USB (DWC2 at `ff540000`), USB MIDI | Mainline |
| Ethernet (RTL8211E PHY) | `stmmac` / `rk_gmac` | Mainline; already works on the stick's device tree (step 7) |
| USB stick | DWC2 and EHCI, `usb-storage` | Mainline |
| eMMC and SD card | `dw_mmc` | **Left out on purpose** (see Safety) |
| Real-time | Linux 6.18.44 with the PREEMPT_RT patch set (`6.18.44-az01-2026-08-10-rt6`) | 6.18 needs the same patch set; mainline Linux supports PREEMPT_RT on 32-bit ARM from 7.1 ([Phoronix](https://phoronix.com/news/linux-7.1-arm-rt)) |

Other facts that shape the plan:

- Akai's full kernel configuration is built into its `zImage` (IKCONFIG). [`offline-kernel-config.py`](../../../tools/mpc-one/offline-kernel-config.py) reads it on the workstation, so our first kernel can start from Akai's exact settings.
- Cloud sessions can't reach kernel.org (the proxy refuses it) and have no ARM cross compiler or `dtc`, so kernels are built on the owner's workstation, which already cross-compiles MPFree for the MPC.
- On the chain's second kernel LoadPin is enforcing and pins the stick's root, and modules load from it (step 7). Our modules therefore go on the stick's root.
- The first kernel loads the second with the old `kexec_load` call, with `loadpin.enforce=0` on its command line (step 7). That call has no signature check, so our `zImage` needs no signature.
- Sources for the table: [audio and MIDI survey](audio-midi-survey.md), [touchscreen and display](touchscreen-display.md), and the device trees read with [`offline-dtb.py`](../../../tools/mpc-one/offline-dtb.py).

## Safety

Our kernel runs from RAM only and never touches the MPC's own storage. Pulling the stick and switching off always brings back MPC OS as it was.

- **No eMMC or SD driver at all.** The stick's device tree already disables the eMMC and SD controllers. Our kernel goes further: it is built without the MMC drivers (`CONFIG_MMC` off), so even a wrong device tree cannot make the internal storage appear.
- **RAM only.** The kernel file sits on the stick and the chain loads it into memory. The [recovery strategy](../research/recovery-and-os-strategy.md) treats a kernel in RAM as research that leaves the eMMC untouched. Nothing here is persistent, so the Tier 3 gate (for persistent changes) does not apply.
- **One try per failed power-on.** The guard (#381) marks the try before the jump. If our kernel panics or hangs before it has run 30 s, the next power-on skips the chain once and MPFree starts on Akai's kernel instead.
- **The soft-reboot stays.** The default route, and the fallback if Akai changes something, is not touched by this plan.
- **Same checks every round.** Each round records the boot-area hashes before and after, as steps 6, 7 and 7b did, and passes only if they are unchanged with 0 sectors written.
- **Undo:** switch off and pull the stick, or rebuild the stick without the new option (or without `--chain`).

## Steps

Each step is its own small PR with owner blocks, verified before the owner gets them. Each keeps Akai's kernel copy as the default for `--chain`; our kernel is used only when the stick is built with a new opt-in option, so reverting is one rebuild.

1. **Read-only survey (no change to the MPC).**
   - Akai's kernel config, read offline from the `zImage` with `offline-kernel-config.py`.
   - The device tree nodes for the display, sound, touch and backlight, with `offline-dtb.py show`.
   - A new read-only script over SSH that lists every device and the driver bound to it (`/sys/bus/*/devices/*/driver`), so nothing Akai's kernel relies on is missed. The existing read-only scripts list modules and USB drivers only.
   - Also answer the open question in [boot routes](boot-routes.md#what-the-next-runs-answer): whether `CONFIG_MODULE_SIG_FORCE` is set.
   - Result: this page gets the full parts list and the config we start from.
2. **First own kernel, no sound yet.**
   - Linux 6.18.y with the matching PREEMPT_RT patch, Akai's config as the starting point, minus the MMC drivers and Akai's own drivers.
   - Built on the workstation by a pinned script in the repository that checks the source hashes.
   - The stick builder gets an opt-in option for our kernel and its modules (modules on the stick's root).
   - Passes when it boots on two power-ons with Ethernet and SSH, LoadPin enforcing, no `mmcblk`, the `--app-user` privileges and unchanged boot hashes. The screen works if the panel is mainline. No sound is expected, and the pad player says so.
3. **Sound.** A replacement for Akai's sound driver (`simple-audio-card` plus a small codec driver for the enable, mute and reset pins), with care over pops at start and stop. Passes with the same latency and underrun measurements as step 7 (8.70 ms pad to sound, 0 underruns) and no audible pop.
4. **Display, if needed.** If the panel is not mainline, port its start-up sequence from Akai's device tree or source.
5. **Tuning.** Trim modules, add any firmware the hardware needs, and set the real-time options (timer, CPU frequency, IRQ threads) for MPFree.
6. **More power-ons, then the owner's decision on the default.** Repeated power-ons with the guard, a comparison with Akai's kernel on the same tests, then a separate small PR if the owner chooses to make the chain the default.

## Decisions for the owner

None blocks the read-only survey in step 1.

| Decision | Options | Recommendation |
| --- | --- | --- |
| Which kernel version to start from | **6.18.y with the PREEMPT_RT patch**, the series Akai runs, or **mainline 7.1 or later**, where PREEMPT_RT is built in for 32-bit ARM | 6.18.y-rt first, so only one thing changes at a time (our config and drivers, not the kernel version). Mainline can be a later step. |
| Ask inMusic for their kernel source | Akai ships a GPL kernel, so the source is owed on request. It would include their sound driver and probably the panel driver. | Ask, but don't wait for it: step 3 can go ahead with our own replacement. It is a message outside the project, so the owner sends it. |
| How builds run on the workstation | A pinned container image (Docker or Podman), or a cross compiler installed directly | The container, so every build uses the same tools and the result can be checked against a hash. |

## Risks

| Risk | What it would look like | How we handle it |
| --- | --- | --- |
| Sound without Akai's driver | Pops at start or stop, or wrong levels | Test at low volume first; switch the mute pin before the clocks start and after they stop; measure latency and underruns against step 7. |
| A kernel that fails early | A dark screen and no log, because the log partition is not mounted yet | The owner switches off; the guard skips the chain on the next power-on. Keeping the failed boot's messages in RAM (ramoops) would let the next boot save them to the stick. |
| Panel driver not in mainline | No picture, while SSH and sound still work | Port the panel's start-up sequence in step 4; MPFree stays usable over SSH meanwhile. |
| Ethernet link timing | One link retry on some power-ons (`Failed to reset the dma`), as in steps 7 and 7b and earlier kexec boots | Watch it each round; our kernel can change it either way. |
| Keeping the kernel up to date | Security and stable fixes land upstream | The build script makes a rebuild one command; follow the 6.18 stable and RT releases. |
| Akai changes MPC OS | A new first kernel that refuses the chain | The chain stops at the first kernel and MPFree starts there, as today; the soft-reboot stays the fallback. |

## Related

- [Boot routes on MPC OS 3.10.1](boot-routes.md): why the chain is the only route to our own kernel without touching U-Boot.
- [Starting MPFree from the stick at power-on](stick-autoboot.md): steps 6, 7 and 7b, their runbooks and results.
- [How MPFree boots from the stick](soft-reboot-boot.md): the default route.
