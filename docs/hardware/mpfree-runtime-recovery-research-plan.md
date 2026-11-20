# MPFree: runtime experiments, recoverability gates, and independent boot-chain roadmap

**Status:** Design and authorization document only. No live test is performed by any V6 offline investigation script. All steps below refer to the owner's own AKAI MPC One RK3288. Nothing here asserts that a tested, independent brick-recovery channel exists yet.

## 0. A literal 100% guarantee is not defensible

It is not possible to be **100% certain** that any experiment involving a real physical computer is recoverable from *every* failure. These risks include failed eMMC chips, electrical damage, power-system damage, inadvertent persistent firmware writes, incorrectly characterized boot fuses, and external USB equipment failures. The available evidence **does not yet provide** an independent way to recover the MPC One if its installed SRAM SPL or U-Boot is damaged.

A meaningful engineering guarantee must specify an exact *fault model*. We can aim for:

> For every tested software-level failure that does not physically damage components or permanently program security fuses, there exists an **independent, demonstrated** method to cold-boot recovery code, read the actual boot media, and restore verified backup bytes without relying on Akai Linux, kexec, the installed U-Boot, or a successful boot of MPFree.

Even that statement is a **target acceptance criterion**, not an achieved result. Before it is demonstrated, reboot and alternative-boot experiments remain conditional-risk research. We must not claim a bootloader-independent fallback by simply having a full backup file.

## 1. What the offline evidence actually establishes

| Observation | Supported by offline artifacts | What it does not prove |
| --- | --- | --- |
| P1 SRAM SPL and P3 U-Boot hashes are verified against owner backup | Original P1/P3 offline audit and Ghidra V3/V4 | That a newly built SPL is accepted by BootROM |
| U-Boot's own MMC controller aliases identify eMMC and SD | P3 FDT and 3.10 Linux DTBs | That SD will be tried before successful eMMC |
| `boot_targets=mmc0 mmc1`; valid saved/compiled env | P2 and P3 reports | That `mmc1` can be made primary without writes |
| Reboot modes normal/loader/recovery/bootloader | Board DTBs and P3 handlers | That loader means silicon MaskROM |
| 3.10 adds `mode-update` as synonym of `mode-loader` (`0x5242c301`) | 3.6/3.10 structural DTB diff | Any additional BootROM capability |
| U-Boot Fastboot can receive to RAM and invokes `bootm`; errors may cause CPU reset | P3 callbacks and V5 graph | Safe image size, authorization, or recovery from bad image |
| SPL and U-Boot DTBs share DDR initialization arrays | Embedded FDTs | Functional owner SPL without further testing |
| 3.10 `az01config` has a `program-fuses` command and partition-write symbols | V5 ELF/objdump | Which exact fuses, whether command is authorized, or whether safe to invoke |
| `az01-bootloader-sig` uses SHA256, RC4, compares hashes and can fatal-exit | V4/V5 ELF | Automatic invocation, ROM signature policy, boot refusal |
| 3.10 LoadPin helper accesses `/sys/kernel/security/loadpin/dm-verity` | V5 ELF strings/service | Silicon verified boot or restrictions on a separate owner-built Linux kernel |

**High-value external constraint:** The RK3288 technical reference manual's BootROM boot-flow diagram tries external media including eMMC and SD, then enters USB download when no valid ID block is found. USB download first loads DDR initialization code in SRAM and then a loader in DDR. This is *silicon ROM behavior*, not the same as Akai U-Boot Fastboot. Documentation for `CONFIG_SPL_ROCKCHIP_BACK_TO_BROM` describes handing control back to BootROM during a normal multistage boot; it is **not** proof that USB MaskROM mode is entered. See the sources below.

## 2. Define the failure model and recovery evidence gate

The preflight should distinguish:

- **F0 — no persistent changes:** Host-only logs, SSH `/proc` and `/sys` text read, non-intrusive UART RX capture. This eliminates *intentional firmware modification*, not spontaneous failure or electrical mistakes.
- **F1 — reboot or change temporary runtime state:** Reboot to stock Linux, physical recovery-button investigation, USB enumeration and runtime serial capture. Risks include failure to return from loader/update/recovery state, device stuck waiting for cable, or hardware reset anomalies.
- **F2 — execute new code only in RAM:** Temporary U-Boot environment, `bootm`/`bootz`/Fastboot RAM download, or kexec into an entirely RAM-resident minimal OS. This may still initialize peripherals or write eMMC through software bugs. Hardware memory use and no-write design must be validated first.
- **F3 — alter persistent boot state:** Modify P2 environment, eMMC boot partition, P1/P3 SPL/U-Boot, GPT, secure boot fuses or OTP. **PROHIBITED** in this plan until independent restore has been demonstrated and the owner explicitly authorizes a narrowly scoped step.

The **independent recovery gate** has five parts, each requiring evidence from this particular MPC One, not another RK3288 board:

1. **Media geometry proven:** Confirm entire eMMC physical layout, eMMC user area, any eMMC boot0/boot1 partitions, Boot Partition Configuration/EXT_CSD, active slots, read-only GPT and reserved vendor regions. The existing ~7.28 GiB `mmcblk0.img` backup is a user-area backup; it may **not** include boot0/boot1, RPMB, or hardware configuration, and it cannot restore blown eFuses.
2. **Independent entry verified:** Demonstrate an external way to enter ROM-controlled recovery that does *not* depend on currently valid P1/P3, factory Linux or USB Fastboot. A labelled U-Boot recovery menu does not satisfy this requirement.
3. **Interface verified:** Verify genuine Rockchip ROM USB identity/protocol on *this* physical connector, with explicit distinction from fastboot/Loader. Read-only enumeration is the first activity. No automatic `rkdeveloptool db/wl/ef` commands.
4. **Owner-controlled RAM code or compatible rescue loader:** If ROM requires code download, prove that the required, validated loader runs **in RAM** and can read the actual eMMC without permanently altering anything. Secure boot/fuse policy may prevent this. The embedded DDR timings are necessary but insufficient for a new loader.
5. **Restoration rehearsal:** On sacrificial storage or an otherwise isolated spare test fixture, demonstrate complete independent raw read-back and write/restore, with SHA checks of each region, a verified reboot to original firmware and repeatability across power cycles. A successful read from the *live* MPC does not by itself prove a write-based recovery pathway. A mere backup file is insufficient.

Without all five pieces, we must state that a bad F2 experiment **might** force device disassembly or specialized hardware repair. No intentional live corruption of the owner's sole bootloader is part of the rehearsal.

## 3. Runtime experiments in safe order

### Experiment R01 — Host-only USB identities and passive link observation

**Prerequisite:** No special device mode changes. Identify exact MPC USB physical connector and host-facing interface from the board diagram or existing known wiring. Avoid applying power on unfamiliar pins.

**Action:** Inspect already-enumerated USB devices on the Linux workstation via read-only `/sys/bus/usb/devices` metadata or ordinary `lsusb`, and optionally record a USB protocol trace *while normal boot occurs*. Do not download any bootloader or probe unknown control-transfer commands. Do not change firmware or force MaskROM.

**Expected information:** Vendor/product IDs, physical bus topology, whether the port is functioning as USB-device or USB-host, whether a recovery state enumerates as USB Fastboot or Rockchip, and whether a device disconnect/reset transitions modes.

**Value:** Eliminates unsupported assumptions about which port can reach the immutable ROM. Gives us detection code for a future rescue utility. **Limit:** The ordinary running system cannot prove ROM behavior simply because a USB controller is visible in its device tree.

### Experiment R02 — Read-only stock/Linux runtime inventory over established SSH

**Prerequisite:** Existing working Ethernet/SSH session, existing trusted host key, no need for escalated root permissions. Confirm that the commands are strictly reads.

**Action:** Collect `/proc/cmdline`, `/proc/version`, `/proc/iomem`, `/proc/meminfo`, `/proc/partitions`, `/proc/mounts`, selected `/sys/firmware/devicetree/base` nodes, `/sys/bus/mmc/devices` metadata, systemd unit statuses (query only), and kernel ring-buffer excerpts where access allows. Collect read-only USB enumeration on host separately. **Do not execute** `fw_setenv`, `az01config`, `az01-update`, `reboot`, `kexec`, `dd` to a block device, `mmc extcsd write`, `flash`, `erase`, or any fuse-programming interface.

**Expected information:** Whether boot media are on device MMC0/1 in Linux, exact kernel/FDT/memory reservations, active boot arguments, whether 3.10 verification services really ran, filesystem mounts and whether updater processes are active.

**Value:** Translates static descriptions into runtime facts. There should be no intentional persistent device change. The command collector remains a **separate, opt-in** artifact, not part of the offline V6 script.

### Experiment R03 — RX-only UART observation during ordinary cold boot

**Prerequisite:** Independently verified UART pinout and I/O voltage, checked with high-impedance measurement or board schematic; proper common ground; no TX wire from host. **Never connect 5 V TTL to the SoC.** Use a sufficiently isolated, correctly rated USB serial adapter.

**Action:** Record UART output from power-on through SPL, U-Boot, kernel and early systemd, without injecting keystrokes or electrical reset signals. Capture voltage and baud-rate assumptions in test metadata. No console interrupts, no command submission.

**Expected information:** Real boot-source order, U-Boot relocation and build, DDR capacity, which MMC aliases are used, recovery-trigger conditions, boot script names, console availability, and any signature/verification failures.

**Value:** Early-stage dynamic ground truth without changing boot storage. **Limit:** Hardware attachment introduces an electrical risk, and output availability depends on whether Akai enabled UART for these stages.

### Experiment R04 — Ordinary controlled reboot and loader-vs-ROM mode identification

**Prerequisite:** Satisfactory R01–R03 and a reliable existing way to return to normal operation by removing power. It is still F1, not F0.

**Action:** After explicit owner approval, observe the previously documented ordinary restart and recovery-button paths, without issuing any raw PMU writes or unknown vendor commands. Capture external USB identity and UART output for each state. Prefer physical power-cycle over unverified software reboot-mode values.

**Expected information:** Which mode states are actually distinct, which can be exited using physical power cycling, whether `mode-update` and `mode-loader` behave identically as their DTB values predict, and whether true silicon MaskROM is ever observed.

**Value:** Separates Akai U-Boot Fastboot from immutable ROM recovery. **Stop condition:** Any state that fails to exit by a proven non-destructive action ends the campaign; no escalating to raw writes or shorting test points.

### Experiment R05 — Optional *informational* recovery-protocol interrogation

**Prerequisite:** Verified protocol identity from R04, documented command allowlist, and an explicitly authorized read-only session. Treat USB host API calls as capable of side effects; do not assume every command labelled `getvar` is harmless.

**Action:** Request only documented informational attributes that the decompiled dispatcher confirms do not modify storage or state. Start with vendor/product/protocol discovery; enable individual `getvar` queries only after reviewing exact U-Boot dispatch and authorization logic. No downloads, flashing, erase, OEM unlock, `reboot-bootloader`, RAM boot, raw block reads of unknown semantics, or arbitrary vendor requests.

**Expected information:** Build/version/partition metadata, permission state (if exposed), difference between Fastboot and ROM/Loader mode, and whether an accessible protocol can later support a RAM-only MPFree handoff.

**Value:** Direct evidence about actual software-visible recovery interfaces, without yet executing owner code.

### Experiment R06 — Discover U-Boot serial interaction WITHOUT changing boot policy

**Prerequisite:** Working R03 RX-only UART and independent rescue channel if attempting anything beyond receive-only logging. With `bootdelay=0`, automatic console interruption is not assured.

**Action:** Initially only observe autoboot. If a documented input mechanism is confirmed, a later owner-approved test may send a single non-storage-affecting query (such as display-only help or memory size) on a live U-Boot console; do not call `saveenv`, reset-mode commands, `mmc write`, `usb write`, flash/erase, or `source` of unreviewed scripts. A non-persistent `setenv` experiment would be a separate F2 approval after R07.

**Expected information:** Whether U-Boot can be controlled without kexec or the Akai Linux kernel, availability of `mmc`, `bootz`, `extlinux`, `load`, and command transport; ability to enter an owner-controlled selection path.

**Value:** A reachable stock U-Boot console could provide an immediate direct-SD path even if later Akai firmware disables kexec. **Limit:** Console input alone is not an independent emergency recovery pathway.

### Experiment R07 — RAM memory-map and image-format validation

**Prerequisite:** R02/R03 memory data, verified UART/recovery state, complete ROM recovery gate if planning actual code execution, and offline review of target `bootm`/`bootz` image classification and allocation.

**Action:** First calculate the memory map **offline**, including actual DRAM capacity, relocated U-Boot, stack, heap, BSS, DDR reserved areas, FDT, kernel, ramdisk, framebuffer and Fastboot buffer at `0x00800800`. Prove non-overlap for a small, fixed-size image. If and only if the recovery gate is satisfied, a separately authorized session can attempt RAM-only execution of a minimal heartbeat kernel that has **no MMC write path**, no update functionality, no persistence and no automatic rootfs mount.

**Expected information:** Correct ARM Linux entry, DTB compatibility, reachable SD/USB/UART drivers, and whether owner code can boot independent of Akai's Linux userspace.

**Value:** Direct proof of an alternative kernel boot path. **Critical failure mode:** The P3 Fastboot `boot` continuation can call CPU reset if `bootm` returns; hardware may hang or fail before reset. A theoretically RAM-only boot can still corrupt persistent media if it mounts or drives the wrong storage device.

### Experiment R08 — Select SD boot without changing eMMC

**Prerequisite:** Positive R06+R07 results and independent ROM recovery; otherwise observe only. Note that the factory U-Boot environment is `mmc0 mmc1`, meaning eMMC first.

**Action:** Build an owner SD card *on the Linux workstation* containing an MPFree kernel, DTB, initramfs and a read-only root image, avoiding on-device flash writes. Test selection using only a verified temporary U-Boot input path or externally controlled hardware boot source. Do not change P2 environment, P1/P3, or disable eMMC to force fallback. Cold power-cycle and verify factory boot still works unchanged.

**Expected information:** Whether MPFree can boot fully from SD, with all factory partitions untouched, under stock U-Boot; whether future 3.10+ userspace changes can interfere.

**Value:** First practical boot path **independent of Akai Linux/kexec**. It remains dependent on the factory SPL/U-Boot and how future firmware updates treat them.

### Experiment R09 — Temporary owner loader under truly independent ROM recovery

**Prerequisite:** Full verified independent recovery gate, independent serial observation, exact SPL DDR timings, board-compatible Rockchip image format and ability to execute owner code in SRAM/DRAM without writing flash.

**Action:** Test an extremely small owner-controlled SRAM/DRAM loader in RAM only. It should produce a UART/USB heartbeat, expose read-only memory/storage telemetry, and return/reset. It must not include eMMC write, RPMB or fuse code. Stop if runtime ROM authentication rejects it. Do not bypass or alter hardware security fuses.

**Expected information:** Owner ROM/SPL code acceptance, DDR training, physical OTG transport, storage enumeration and independent recovery capability.

**Value:** Crucial step toward truly Akai-independent MPFree first-stage boot. The test itself is not guaranteed reversible merely because its binary is loaded into RAM. Strict hardware recovery requirements apply.

### Experiment R10 — Upgrade resilience and fault-injection *on spare hardware*

**Prerequisite:** Spare RK3288 board or equivalent sacrificial eMMC fixture; full reflash capability and an accepted test harness. No intentionally bricking the user's sole MPC One.

**Action:** Simulate missing bootloader, invalid environment CRC, boot script missing, malformed kernel, crashed update service, 3.10-style LoadPin failure, USB disconnect and power failure. Prove the MPFree recovery boot process survives **each** scenario and restores expected bytes, measuring recovery time and integrity.

**Expected information:** Actual—not assumed—failure-domain separation and rollback behavior. Tests can reveal that parts of the boot chain are more coupled to vendor components than static analysis suggested.

**Value:** Repeatable evidence to justify any eventual P1/P3 changes on the owner device, with a sharply defined supported fault model.

## 4. How to avoid dependence on Akai upgrades and kexec

Think in boot trust boundaries:

1. **Current path:** Rockchip BootROM → Akai SPL → Akai U-Boot → Akai Linux → Akai userspace → kexec → MPFree. Akai can remove/disable kexec or interfere with handoff in later Linux, kernel config or updater services.
2. **Intermediate path:** Rockchip BootROM → Akai SPL → Akai U-Boot → **MPFree kernel/DTB/initramfs from external SD or RAM**. This does not depend on Akai Linux or kexec, but U-Boot may remain a vendor component that upgrades can change. Require a verified, repeatable way to *choose* this path when the stock eMMC script succeeds.
3. **Independent stage-two path:** Rockchip BootROM → compatible DDR/SPL initialization → **MPFree U-Boot or owner boot manager** → MPFree OS. This removes dependency on Akai U-Boot but still needs an accepted first-stage loader, stable recovery and board-compatible DDR/USB/MMC drivers.
4. **Full owner chain:** Immutable Rockchip BootROM → **MPFree SPL** → **MPFree boot manager** → MPFree OS, with immutable/independent hardware recovery or an externally documented ROM rescue mechanism. This is the intended design, *not an achieved result*.

A future-proof installer must **never depend on upgrading Akai's image or on kexec remaining available**. It should run primarily from a recoverable owner-selected medium, preserve a complete audited factory image, and avoid eMMC boot-sector changes until recovery is proven. Avoid auto-replacing factory P1/P3 on ordinary OS upgrades.

## 5. Acceptance criteria, stop conditions and evidence capture

**Every test** gets an explicit test ID, firmware hashes, device/PCB revision, exact physical ports, initial power/reset state, permitted operations, excluded operations, USB vendor/product ids, timestamps, UART transcript (if available), kernel boot log and expected cold-power-cycle exit. Backups remain under local operator control. Do not send private device identifiers or proprietary binaries to public GitHub.

**Stop immediately if** the observed USB mode is not the expected protocol, power supply is unstable, the selected memory range overlaps another allocation, device identities change unexpectedly, a supposed read-only command requests storage writes, a previously reliable boot path no longer works, or any recovery prerequisite fails. Never escalate by executing unknown vendor CLI, corrupting the ID block, programming fuses, touching eMMC CLK pins, issuing raw PMU writes, or attempting flash/erase.

**Evidence gate to move beyond observation:** We must be able to demonstrate from power-off that recovery is possible *without the installed Akai bootloader*. We must then demonstrate read-back and, on separate sacrificial media/fixture, full restore verification. Without these gates, we should restrict the owner device to passive observation and stock normal operation.

**Sources for chip-level context (not evidence of this MPC One's wiring or fuse state):**

- RK3288 TRM, Chapter 2 boot procedure: https://rockchip.fr/Rockchip%20RK3288%20TRM%20V1.0%20Part%201-System%20and%20System%20Control.pdf
- U-Boot Rockchip documentation on SRAM SPL and back-to-BROM: https://github.com/qemu/u-boot/blob/master/doc/README.rockchip
- Firefly RK3288 description distinguishing Loader and MaskROM: https://wiki.t-firefly.com/en/Firefly-RK3288/bootmode.html
- Rockchip Linux historical U-Boot user manual: https://github.com/linux-rockchip/u-boot-rockchip/blob/u-boot-rk3288/UserManual

## 6. Concrete development deliverables

- V6 owner ZIP with one integrity index and no firmware executables.
- Offline analysis that distinguishes recognized Ghidra functions, import stubs, indirect call sites, and missing stripped-ELF main functions.
- A verified eMMC/partition coverage inventory distinguishing user area from eMMC boot0/boot1/RPMB and permanent fuses.
- A host/device *observation-only* collector, separately opt-in; never part of scheduled or automatic boot-chain audit.
- A direct-SD boot proof experiment conditioned on accessible transient U-Boot control and independent recovery.
- A full cold-boot independent restore demonstration on a safe, sacrificial test fixture before persistent bootloader modifications.