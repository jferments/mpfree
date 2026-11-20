# MPFree integrated bootchain — fifth owner run and USB protocol families

**Owner-private evidence:** \`bootchain-wide-integrated-20261112T061323Z-3808968.zip\` (2026-11-12 UTC).
The private ZIP, Ghidra decompilations, firmware executables and full
device backups are not stored in the public source repository.

## Results

- ZIP CRC passed; **85/85** SHA-256 indexed report members verified.
- Wide Ghidra analysis passed (69.2 s). V11 directed updater/Ghidra passed
  (47.5 s). Entire run completed in 119.6 s with no stage errors.
- Ghidra P1 213 recognized functions; P3 1050 recognized functions.
- V11 decompiled 12 updater functions, 137 updater-library functions,
  25 directed P1 and 55 directed P3 functions.
- Generic ARM QEMU guest booted a clone of the 3.10 rootfs, matched
  both source-library binary hashes, completed all scripted inspection,
  and executed independently written static ARM test models from a separate
  read-only virtual disk.
- **13/13** interface-selection and **21/21** endpoint-discovery synthetic
  ARM tests passed inside the guest. Their per-case JSON and serial logs
  are contained in the private ZIP.
- No physical MPC, eMMC, OTP, USB passthrough, firmware update or
  original vendor application entry point was invoked.

## Two USB backend protocol families now have upstream source corroboration

V11 Ghidra demonstrated that the updater library's interface selector at
ELF vaddr \`0x1d9fc\` selects and installs three backend callback tables
using vendor interface class \`0xff\` plus subclass/protocol matching:

| USB interface class/subclass/protocol | Constructor | Backend table | Status |
| --- | --- | --- | --- |
| \`ff/42/03\` | \`0x16ff8\` | \`0x12a74\` | **Android Fastboot-compatible** |
| \`ff/61/7a\` | \`0x1b49c\` | \`0x12b48\` | Unknown vendor-specific |
| \`ff/06/05\` | \`0x1ca80\` | \`0x12bd8\` | **Rockchip RockUSB-compatible** |

Fastboot evidence: Android's fastboot USB matching code explicitly
requires \`ff/42/03\`; the Ghidra path contains Fastboot-style status
signatures (\`OKAY\`, \`FAIL\`) in its callback implementations.

- Google fastboot source: https://android.googlesource.com/platform/system/core/+/a00f91b4c15fcc5b8f9b9c70dde39fd97a8b2e89/fastboot/fastboot.cpp
- Google Fastboot interface doc: https://android.googlesource.com/platform/bootable/libbootloader/+/refs/heads/main/gbl/docs/GBL_EFI_FASTBOOT_USB_PROTOCOL.md

RockUSB evidence: Rockchip's U-Boot gadget implementation explicitly
defines interface \`ff/06/05\`. The corresponding updater-library backend
builds CBW/CSW-style Bulk-Only Transport frames with a \`USBC\` command
wrapper marker, consistent with the upstream RockUSB protocol.

- Rockchip U-Boot source: https://github.com/rockchip-linux/u-boot/blob/next-dev/drivers/usb/gadget/f_rockusb.c

**Crucial limitation:** A RockUSB-compatible interface is not proof of
immutable silicon MaskROM execution. Rockchip's ROM-mode download and
subsequent USB loader are distinct stages. Neither an interface descriptor,
host-side updater implementation nor this generic QEMU guest can prove
true BootROM access on the physical owner's MPC.

- Independent discussion of ROM vs RAM-loaded usbplug:
  https://bootlin.com/blog/recovery-boot-on-rockchip-socs/

## Next integrated experiment (committed, not yet run)

The existing single-run Ghidra/QEMU orchestrator is updated:

1. V11 Ghidra now pins additional underlying transport helpers from
   all three backend families, including FF/61/7A and RockUSB frame send
   and response processing paths. Existing SHA checks, read-only saved
   P1/P3 projects and disposable ELF import guarantees remain.
2. A pure offline synthesizer
   \`tools/mpc-one/synthesize_usb_backend_protocols.py\` transforms verified
   \`v11-ghidra-libaz01-prog.json\` into
   \`usb-protocol-evidence.json\` and
   \`usb-protocol-evidence.md\`.
   These report each interface's selected callback slots, Ghidra resolved
   function targets, call edges, signatures, and confidence boundaries.
   **No proprietary pseudocode is committed to GitHub**; generated reports
   are placed only in the owner's private SHA-indexed ZIP.
3. Host-side test \`test_usb_protocol_evidence.py\` exercises the synthesis
   logic with fabricated Ghidra fixtures. The orchestrator continues to
   run full Ghidra and both synthetic ARM models in QEMU.

The major outstanding challenge after this evidence map is understanding
FF/61/7A and unambiguously distinguishing device-side **ROM**, **loader**
and **U-Boot Fastboot** modes on actual hardware without writing any
boot-critical storage. Repeated synthetic parser success does not answer
these physical questions.

## Independent boot/recovery gates remain unresolved

- Genuine RK3288 silicon MaskROM entry on the actual MPC One.
- Owner-built first-stage loader acceptance and DDR/eMMC access.
- Complete owner-controlled eMMC readback and restoration.
- Safe RAM-only boot handoff and recoverable multi-boot design.
- U-Boot runtime Fastboot permission state and memory boundaries.

No physical bootloader writes or fuse programming should follow from this
analysis alone.
