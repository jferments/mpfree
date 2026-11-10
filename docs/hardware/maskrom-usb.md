# RK3288 MaskROM over USB: what we know for MPFree installers

Research note, 2026-11-10. **Nothing in this file has been run on the MPC One.**
It collects what is public about the RK3288 boot ROM's USB recovery mode
("MaskROM"), what our own read-only surveys already show about this unit, and
what that means for future MPFree installers and for the Tier 3 recovery gate
in the [recovery strategy](../research/recovery-and-os-strategy.md).

Every statement is tagged:

- **Measured**: from our own read-only runs on the reference unit (see the
  [boot map](boot-map.md) and
  [offline updater notes](akai-updater-offline.md)).
- **Public**: from Rockchip, U-Boot or community sources, linked at the end.
  These describe the RK3288 or other Akai units, not necessarily this board.
- **Inference**: our reasoning. Treat as unverified until checked.

The project's device safety rule (see [developer access](../guides/developer-access.md)) applies to everything here: no writes
to the bootloader, its environment or the system partition until MaskROM
recovery has been proven on this unit.

## Summary

1. **MaskROM lives in the RK3288's silicon, not on the eMMC** (Public). No
   firmware update can remove it. It is the only way to repair the 64 KiB
   idbloader (`p1`) or the 384 KiB U-Boot (`p3`), which are this unit's real
   brick surface (Measured).
2. **The boot ROM only drops into MaskROM when it finds no valid loader** on any
   boot medium (Public). On this unit the eMMC loader is valid, so we need a way
   to make the ROM skip it. Three kinds exist, in order of risk: a software flag
   left by a reboot (needs Akai's loader to cooperate; unknown here), a
   recovery/MaskROM button or pads (none known on the MPC One), or
   temporarily shorting the eMMC clock to ground at power-on (works on every
   RK3288 board, but needs the case open and the right pad).
3. **The port is almost certainly the USB-B socket on the back** (Inference, now
   strong): it is wired to the RK3288's OTG controller `ff580000`, which is
   the controller the ROM uses, and Akai's Update Mode (fastboot) uses it too.
4. **MaskROM work can be RAM-only.** The host loads a DDR-init blob and a
   small USB loader ("usbplug") into the SoC's RAM, and from then on can read
   the eMMC block by block. Nothing is written unless the host asks for it.
   A power-cycle returns to the normal eMMC boot. That makes a full eMMC
   **read-back over MaskROM**, compared against our SSH backup, a safe Tier 3
   qualification step once we can enter the mode.
5. **The one thing that could take MaskROM away is RK3288 secure boot.** Once a
   key hash is burned into the SoC's one-time eFuses, the ROM (including over
   USB) only runs loaders signed with that key (Public, 3mdeb on a Firefly
   RK3288). We don't know whether this unit is fused. Akai's 3.10.0 adds a
   tool, `az01-bootloader-sig`, that prints a bootloader signing-key hash
   (Measured from strings), so Akai is at least signing bootloaders on some
   product. This is the main open risk for the "guaranteed bootloader access"
   goal in the [roadmap](../project/roadmap.md).
6. **For a one-click installer, MaskROM is a recovery tool, not the install
   path.** It needs a host computer and, today, opening the case. The normal
   install should stay software-only (USB autorun, kexec), with MaskROM as the
   firmware-proof fallback.

## Three USB modes that are easy to confuse

| Mode | Where the code lives | How it is entered | USB protocol | Can it be broken by a bad update? |
| --- | --- | --- | --- | --- |
| **MaskROM** | RK3288 boot ROM (silicon) | No valid loader found at power-on (or a cooperating loader returns to the ROM) | Rockchip ROM protocol: vendor control transfers that load code to SRAM (`0x471`) and DRAM (`0x472`) (Public) | No, unless secure-boot eFuses are burned |
| **Rockusb "Loader" mode** | A Rockchip loader in RAM: Rockchip's miniloader, a usbplug sent over MaskROM, or U-Boot's `rockusb` command | Reboot flag, recovery key held at boot, or after a MaskROM `db` | Rockusb bulk commands: read/write/erase by LBA, read chip info | Only if the loader on eMMC is broken (then use MaskROM) |
| **Akai Update Mode** | Akai's U-Boot on `p1`/`p3` | FULL LEVEL + BROWSE + PAD BANK C/G at power-on, or reboot argument `loader` from Linux (Measured) | Android fastboot (`fastboot 0`) | Yes: needs `p1`/`p3` intact |

Rockchip's own tools call both MaskROM and Rockusb "download" modes, and the
Windows tool shows a MaskROM device as "Found One MASKROM Device" and a
Rockusb device as "Found One LOADER Device" (Public). `rkdeveloptool ld`
prints `Maskrom` or `Loader` the same way.

**Akai's fastboot is locked** on the MPC Live: some fastboot commands return
"permission denied" until an `oem inmusic-unlock-magic-…` command is sent,
and the unlock string is hard-coded in U-Boot proper (Public, Nisbeth's MPC
Live notes). We have not checked the MPC One's U-Boot, but it is the same
`az01` family, so expect the same (Inference). This matters for the
"fastboot RAM boot" idea in the roadmap: `fastboot boot` may be refused.

## How MaskROM is entered on the RK3288

**The ROM's boot order.** At power-on the boot ROM looks for a Rockchip
ID block (the RC4-scrambled header we found at eMMC sector 64, Measured) on its
boot media in a fixed order: SPI flash, NAND, eMMC, then the SD card. If none
is valid it starts USB download on the OTG port (Public, RK3288 TRM as
summarised in U-Boot and community docs). Two consequences:

- With a valid eMMC loader the ROM never reaches the SD card or USB. This is
  why our U-Boot-level "SD is only tried if eMMC fails" finding has a ROM-level
  twin.
- If the eMMC is made invisible at power-on (clock shorted), **the ROM tries the
  SD card before USB**. A stray Rockchip loader on an inserted SD card would
  boot instead of MaskROM (Inference from the order). Remove SD cards before
  any MaskROM attempt.

**Ways to make the ROM skip the eMMC loader**, from safest to riskiest:

1. **A software "back to boot ROM" flag** (Public, U-Boot). Rockchip loaders can
   check a boot-mode register at start-up and, if it holds the magic value
   `0xEF08A53C` (`BOOT_BROM_DOWNLOAD`), hand control back to the ROM with a
   request to enter download mode. The kernel or U-Boot writes the value and
   warm-resets. **This only works if Akai's loader on `p1` has that check.**
   The mainline U-Boot patch that added it enabled it for the RK3399, and we
   don't know whether Akai's RK3288 loader does (Unknown). The Linux
   `reboot-mode` node in the device tree decides which reboot strings set
   which value; Akai's `loader` string already selects Update Mode (Measured).
   Nothing is written to flash by this route; the flag sits in an always-on
   register and is cleared by the loader or by removing power (Inference).
   Finding out is an offline check of `p1`, `p3` and the DTB from our backup
   (see the first check below), with no device contact.
2. **A MaskROM or recovery button or pads.** Many RK3288 boards have one. On the
   Radxa Rock 2 Square, the "recovery" button is an ADC key read by the
   loader (Public), so it gives Rockusb/Update Mode, not true MaskROM. On the
   MPC Live, the "recovery" button on the mainboard enters the same Update
   Mode as the software route (Public). No public photos or notes show a
   MaskROM button or pads on the MPC One's board (searched; nothing found).
3. **Shorting the eMMC clock (or a data line) to ground at power-on.** This is
   the standard method on boards without a button (Public, many RK3288 board
   wikis). The ROM's eMMC read fails, so it falls through to SD and then USB;
   the short is released once the device enumerates. **Risks:** the case must
   be opened (owner's decision, may affect warranty); the right test point must
   be identified from the board, ideally with a datasheet-backed trace from
   the eMMC package; shorting the wrong pad (for example a supply rail) can
   damage the board; holding the short too long can corrupt a transfer.
   Grounding a clock line through a small resistor (for example 100 Ω) rather
   than a bare tweezer is gentler (Inference, common practice). **Not to be
   attempted without board photos, an identified pad and Jay's explicit go.**
4. **Erasing the eMMC loader** (what the Rockchip and U-Boot docs suggest for
   dev boards: zero the first megabytes). **Never on this unit.** It is exactly
   the brick we are protecting against, and it is only "recoverable" if
   MaskROM is already proven.

## The physical port

- **Measured:** `usb@ff580000` (`rockchip,rk3288-usb`, `snps,dwc2`) is in
  `dr_mode = peripheral`; the other DWC2 at `ff540000` is a host. On RK3288
  the OTG controller at `ff580000` is the one the ROM uses.
- **Measured:** Akai's desktop updater talks fastboot to the MPC through the
  USB-B socket, and fastboot needs a device-mode controller.
- **Inference:** the USB-B socket is wired to `ff580000`, so it is the MaskROM
  port. Confirm without opening the case once we can enter any device mode:
  in Update Mode the host's `lsusb` shows Akai's fastboot device; in MaskROM
  the same socket should show Rockchip's vendor ID `2207` with the RK3288
  product ID `320a` (Public, rkflashtool and rkdeveloptool device tables).
- A standard USB-A-to-B cable from the workstation is all the host side needs.

## Host tools

| Tool | What it is | Licence | Notes |
| --- | --- | --- | --- |
| `rkdeveloptool` | Rockchip's open Linux tool, packaged in Debian/Fedora | GPL-2.0 | `ld` list, `db` download loader to RAM, `rid` read flash ID, `rci` chip info, `rl` read LBA, `wl`/`ef`/`ul` **write/erase/upgrade** |
| `rkflashtool` | Community tool, older | BSD-2-Clause | Can send a raw SPL to the ROM (`l`), plus read/write by LBA |
| `upgrade_tool` | Rockchip's closed Linux binary (rkbin repo) | Proprietary | Same functions; also firmware-package upgrades |
| `RKDevTool` / `RKBatchTool` | Rockchip's closed Windows tools | Proprietary | Driver install needed on Windows |
| U-Boot `rockusb` command | Rockusb device mode inside U-Boot | GPL-2.0 | Only once U-Boot runs; not available if `p1`/`p3` are broken |

**For MPFree** we would use `rkdeveloptool` (open, scriptable). For an
installer that ordinary owners run, we would wrap it in our own guarded tool
that only issues an allowlist of read commands (`ld`, `db`, `rid`, `rci`,
`rl`) until a restore is deliberately requested, the same pattern as our
`--backup` tool. A Windows/macOS story needs a libusb build; that is later
work.

**The loader blob.** MaskROM only accepts commands once DDR is initialised
(Public, 3mdeb), so `db` needs a Rockchip-format loader file holding a
DDR-init blob and a usbplug. The usual source is Rockchip's `rkbin`
repository (`rk3288` DDR and usbplug binaries, packed with `boot_merger`).
These are closed binaries; check the rkbin licence before shipping one in an
installer (Unknown). Open options: mainline U-Boot can build an RK3288 SPL as
an `rkimage` that is sent straight to the ROM over USB (Public, U-Boot
README.rockchip), but its USB support stops at SPL; a full open usbplug
equivalent for RK3288 would be our own work.

## What a host can do in MaskROM, and whether it is RAM-only

1. `rkdeveloptool ld`: list the device. **Read-only.**
2. `rkdeveloptool db <loader.bin>`: load DDR init, then the usbplug, into RAM
   and run it. **RAM only:** nothing touches the eMMC. The SoC now speaks
   Rockusb.
3. `rkdeveloptool rid`, `rci`, `rfi`: read flash ID, chip info, flash info.
   **Read-only.**
4. `rkdeveloptool rl <start> <count> <file>`: read eMMC sectors to the host.
   **Read-only** on the eMMC. With this we can read back the whole eMMC and
   compare it with the SSH backup hashes. That is the Tier 3 condition "full
   eMMC read-back over MaskROM matches the SSH backup".
5. `wl`, `ef`, `ul`, `wlx`, `gpt`, `ppt`-style commands **write or erase**.
   Tier 3 only, and the tool must refuse them unless a restore is requested.
6. A power-cycle (pull the power, not a soft reset) leaves MaskROM and boots
   the eMMC normally, as long as nothing was written and no SD card holds a
   loader (Public behaviour; Inference for this unit).

The usbplug can also load and run our own code from RAM, which is the
"MaskROM RAM boot" route to testing another OS with zero eMMC writes listed in
the recovery strategy. Getting from usbplug to a running kernel needs a
RAM-loaded U-Boot or a custom usbplug, which is research.

## Secure boot and eFuses

- **Public (3mdeb, Firefly RK3288):** RK3288 secure boot uses a 2048-bit RSA
  key. A 32-byte value (the key hash, Inference) is burned into the eFuse/OTP
  with Rockchip's eFUSE tool. Burning is one-time and cannot be undone. After
  fusing, "loading of unsigned [loaders] no longer works", including over
  MaskROM: `rkdeveloptool db` hangs on an unsigned loader, and signed ones
  still load.
- **Public:** on that board, burning only succeeded after powering the eFuse
  programming supply (`EFUSE_VQPS`) from a bench supply at a test point.
  **Inference:** on many RK3288 boards the eFuses can't be burned by software
  alone, because the programming supply isn't powered. Whether the MPC One
  powers VQPS (for example through a GPIO-controlled regulator in its device
  tree) is unknown and worth checking offline in the DTB.
- **Public (U-Boot list, Radxa Rock 2):** Rockchip's miniloader prints
  `SecureBootEn` and `SecureBootLock` on the serial console. **Inference:** if
  Akai's loader is Rockchip's miniloader, the UART2 console would show the
  secure-boot state passively. If it is U-Boot SPL (which our `p1` size
  suggests), it may print nothing.
- **Unknown:** whether Linux can read the secure-boot flag. The RK3288's
  non-secure eFuse (32 bytes, `rockchip,rk3288-efuse`) is readable from
  Linux through nvmem, but it holds chip ID and calibration data; the
  secure-boot bits are believed to be in the secure eFuse, readable only from
  the secure world (Inference). The chip ID is a unique identifier, so any
  survey that reads it must redact it.
- **What 3.10.0's `az01-bootloader-sig` suggests** (Measured from strings only):
  it compares a "primary" and a "fallback" bootloader in partitions 1 and 2 and
  prints the signing-key hash. That is not the MPC One's 3.6.0 layout (here
  `p2` is the U-Boot environment), and nothing in 3.10.0 calls it. Readings,
  all Inference: Akai signs bootloaders on at least one product in the `az0x`
  family; a future update could move this board to an A/B signed-loader
  layout; and if Akai ever also burns the key hash into the eFuses, MaskROM
  would accept only Akai-signed loaders. The tool's name and output say
  "verify", not "burn", and we found no eFuse-writing tool in 3.10.0.

**Consequences for the roadmap's "guaranteed bootloader access"**
(Inference): MaskROM gives access no firmware can remove **unless** a
firmware burns secure-boot fuses. So the guarantee becomes "MPFree units never
receive an update that can burn fuses", enforced by keeping updates off and by
the firmware analyzer flagging any eFuse/OTP writer, `nvmem` write, VQPS
regulator control or new signed-loader layout in a new Akai image. Even on a
fused unit, MaskROM still runs Akai-signed loaders, so read-back through an
Akai-signed usbplug might remain possible; that is speculative.

## Public information about Akai's RK3288 boards

- **MPC Live (Nisbeth's notes):** RK3288 with 2 GB RAM, apparently on a Radxa
  Rock 2 system-on-module (from FCC filings and the device tree; not confirmed
  under the heatsink). A 3.3 V UART on the base board (GND, RX, TX, 3.3 V; GND
  nearest the internal SD reader) at 115200 baud gave a root shell. A
  "recovery" button on the mainboard enters Update Mode. Fastboot is locked
  by an `oem inmusic-unlock-magic-…` command. U-Boot didn't see the internal
  SD reader. The author did not try MaskROM, because forcing it meant erasing
  the eMMC.
- **TheKikGen's MPC-LiveXplore:** rootfs modding, image tools and DTB names
  for MPC One/One MK2 (`inmusic,acva`, `inmusic,acva2`), Live II and Force
  (all RK3288). Nothing on MaskROM. (Its licence is ambiguous, so we use it
  only as pointers.)
- **MPC One board photos, UART pads, MaskROM pads:** nothing public found. The
  MPC One board is not known to use the Rock 2 module (Unknown).

## What root access and our own test boots can teach us

We now have persistent root SSH on stock 3.6.0 and can kexec into a kernel and
device tree of our choosing with the eMMC hidden ([kexec test
boot](kexec-test-boot.md)). That opens several ways to learn about MaskROM
without opening the case. Each is labelled.

**Read-only, on stock 3.6.0 over SSH** (candidates for one new
`--maskrom-survey` mode in the survey tool, BusyBox-safe, one output folder and
one zip):

1. **The live device tree**, which can differ from the file in `/boot` because
   U-Boot applies overlays and fixes before Linux starts: the `reboot-mode`
   node (register offset and every `mode-*` value, which tells us exactly what
   `reboot loader` and any other reboot string write), `adc-keys` and
   `gpio-keys` (a hidden recovery key?), the eFuse node, any regulator that
   could feed the eFuse programming supply, and the OTG node.
2. **What the loader told Linux:** `/proc/cmdline` and `/proc/device-tree/chosen`
   (Rockchip loaders often pass boot-medium and boot-mode hints there), plus
   the first lines of `dmesg`. A miniloader-style loader may also reveal the
   secure-boot state here (Inference).
3. **Secure-world presence:** whether OP-TEE or another TEE is running
   (`/proc/device-tree/firmware`, `/dev/tee*`, PSCI/SMC lines in `dmesg`).
   A secure world is what would hold the secure eFuse, so its absence is
   weak evidence that secure boot isn't in use (Inference).
4. **The non-secure eFuse** through nvmem (`/sys/bus/nvmem/devices/*`): that it
   exists, its size, whether the kernel marks it read-only. The Linux RK3288
   eFuse driver only reads (Inference from upstream; confirm in the kernel
   config and driver name). Contents include a unique chip ID, so the survey
   would print only a hash of it.
5. **Kernel config** from `/proc/config.gz`: `SYSCON_REBOOT_MODE`,
   `NVMEM_ROCKCHIP_EFUSE`, `DEVMEM`/`STRICT_DEVMEM`, `USB_DWC2` gadget support,
   `USB_CONFIGFS`. This decides which of the later experiments are possible
   with Akai's kernel and which need our own kexec'd kernel.
6. **Physical port check, paced from the workstation** (read-only if debugfs is
   already mounted): read the OTG controller's session-valid state, ask Jay to
   plug a cable from the workstation into the USB-B socket, read it again,
   unplug, read again. If the `ff580000` controller sees the cable come and go,
   the USB-B socket is the OTG port and therefore the MaskROM port. If debugfs
   isn't mounted, mounting it is a RAM-only state change and needs Jay's go.

**Offline, from the private backup** (no device contact): decode the `p1`
ID-block and search `p1`/`p3` for back-to-boot-ROM, Rockusb, fastboot `oem`
and secure-boot strings; see the next section.

**State-changing but RAM or register only** (each needs its own write-up and
Jay's go):

7. **Expose a USB gadget on the OTG port** (configfs, RAM only) and check that the
   workstation sees it on the USB-B socket. A stronger version of check 6.
   Undo: remove the gadget or reboot.
8. **Test the software route into MaskROM.** If the offline survey shows Akai's
   loader honours the `0xEF08A53C` boot-ROM flag, write it to the boot-mode
   register and reset. If it works, the workstation sees `2207:320a`; a
   power-cycle returns to the stock boot. Do it **from inside our kexec test
   environment**, where the eMMC is hidden and `/data` is in RAM, so the
   forced reset can't damage a mounted filesystem. A plain `reboot` from stock
   Linux would likely overwrite the register with the "normal" value on the
   way down (Inference from the upstream reboot-mode driver), which is another
   reason to use our own environment. This is the experiment that decides
   whether MaskROM can be reached without opening the case.

**Why this matters for the firmware-proof goal.** If check 8 works, a future
installer or recovery tool can put any MPC One into MaskROM from software for
as long as we have root on some firmware, and the eMMC-clock method stays the
physical fallback for when we don't. Either way, the pieces to keep forever
are: the per-unit eMMC backup (already done), a MaskROM recovery kit (host tool,
loader blob, our guarded read/restore wrapper), and the firmware analyzer's
eFuse/secure-boot checks before any update is allowed near an MPFree unit.

## Proposed first check (no device contact)

**Offline loader survey from the existing private eMMC backup, on ML-tower.**
Read-only, touches only copies of files, the MPC isn't involved.

It answers most of the open questions above:

1. Decode `p1`'s ID-block header (RC4 key `7c4e0304550509072d2c7b38170d1711`,
   Public) and report: Rockchip miniloader or U-Boot SPL/TPL, the sizes of its
   stages, and whether a signature or RSA key block is present.
2. Search `p1` and `p3` for the strings and constants that matter:
   `0xEF08A53C` / `BOOT_BROM_DOWNLOAD`, `back_to_bootrom`, `rockusb`, `brom`,
   `reset_bootloader`, `SecureBoot`, fastboot `oem` commands, and the
   boot-mode register address. This tells us whether a software MaskROM entry
   exists, and whether Akai's U-Boot can enter Rockusb.
3. Decompile `rk3288-az01-acva.dtb` from the backed-up `p6:/boot` and print the
   `reboot-mode` node, `adc-keys` / `gpio-keys` (a recovery key?), the eFuse
   node and any VQPS regulator, and the OTG node.

Output would follow the house pattern: one output folder, one zip, private
Akai content kept out of the repo, only our findings written up. **This needs a
new small tool**, written and tested off-device first. The step after it,
**reading the loader header and the `reboot-mode` node on the live device**,
adds nothing the backup doesn't already contain, so it is not needed.

**Later steps, each needing Jay's go and its own write-up:**

- (State-changing, low risk) Rehearse Akai Update Mode from the button combo,
  check `lsusb` on the workstation shows the fastboot device on the USB-B port,
  and run read-only `fastboot getvar all`. This confirms the port. Undo: power
  off and on. Already listed in the recovery plan.
- (State-changing, RAM only) If the offline survey finds a working
  "back to boot ROM" path: reboot into MaskROM, `rkdeveloptool ld`, then
  power-cycle. Partial failure: the unit sits in MaskROM until power is
  removed; the eMMC is untouched.
- (State-changing, RAM only) `rkdeveloptool db` + full `rl` read-back,
  compared with the SSH backup hashes. This is the Tier 3 read-back
  condition.
- (Physical, owner's decision) Board photos, then identify UART2 pads and a
  MaskROM/eMMC-clock test point. A passive UART connection (RX only, no TX)
  would show the loader's boot messages, possibly including secure-boot
  state, with no risk to the eMMC.

## Sources

- U-Boot, [README.rockchip](https://qemu.googlesource.com/u-boot/+/refs/heads/master/doc/README.rockchip): ROM boot mode, `rkimage`, the RC4 key, SRAM load address, back-to-BROM boot sequence on RK3288.
- U-Boot list, [rockchip: add support for enter to bootrom download mode (v3)](https://lists.denx.de/pipermail/u-boot/2017-October/309088.html): `BOOT_BROM_DOWNLOAD` = `0xEF08A53C`, enabled for RK3399.
- 3mdeb, [Enabling Secure Boot on RockChip SoCs](https://blog.3mdeb.com/2021/2021-12-03-rockchip-secure-boot) (Firefly RK3288).
- U-Boot list, [Secure Boot Mode on rk3288 Radxa Rock2](https://lists.denx.de/pipermail/u-boot/2015-December/237092.html).
- linux-arm-kernel, [add recovery button for Rock2 Square](https://lists.infradead.org/pipermail/linux-arm-kernel/2018-February/560322.html): recovery key on ADC1.
- ArmDesigner, [EM3288 MaskRom mode](https://armdesigner.com/wiki/EM3288/MaskRom_mode.html): eMMC_CLK-to-GND method on an RK3288 board.
- Niklas Nisbeth, [MPC Live Internals](https://niklasnisbeth.gitlab.io/mpc-internals/).
- TheKikGen, [MPC-LiveXplore](https://github.com/TheKikGen/MPC-LiveXplore).
- [rkdeveloptool in Debian](https://packages.debian.org/forky/rkdeveloptool).
