# MPFree owner-controlled RK3288 BootROM bring-up

**Status:** offline boot-image analysis only; no owner-built first stage has executed on the reference MPC One. Updated November 2026.

**Goal:** owner-controlled boot even when Akai changes or removes its SPL, U-Boot, Linux rootfs, Fastboot, kexec, SSH and USB autorun. True ROM entry is a stronger fallback than any vendor-specific software mechanism. The actual board still requires an electrically safe entry method, a free first-stage accepted by silicon, and proven eMMC recovery before boot-critical writes.

## New verified offline p1 evidence

The reference device's p1 occupies 64 KiB beginning at eMMC sector 64. SHA-256 from the project's checked backup: **aa36e0e4033ce7386a210ad3e41f405a0d952a5b2a2c2711328450017414f1ec**.

Its first 16 on-disk bytes (recorded in the 2026-11-09 boot map) are:

~~~text
3b 8c dc fc be 9f 9d 51 eb 30 34 ce 24 51 1f 98
~~~

Decoding those bytes using the *public Rockchip RC4 header key* described in upstream U-Boot produces:

~~~text
55 aa f0 0f 00 00 00 00 01 00 00 00 04 00 00 00
~~~

The decoded Rockchip image signature is **0x0ff0aa55**, the flag **disable_rc4=1** means the SPL payload is stored unencrypted, and the initial loader begins **4 x 512 = 2048 bytes** after the start of this ID block. The remaining init size fields require reading the entire header from the owner's private backup. These are format observations, not evidence of secure-boot acceptance or functional execution.

## Two independent first-stage image formats

### ROM USB / SRAM stage

Upstream U-Boot's older Rockchip documentation describes **rkimage**: a four-byte chip-family identifier (RK32 on RK3288), followed by an ARMv7 SPL image. The ROM USB upload process may apply RC4 to the transfer. The initial code is loaded into **SRAM at 0xff704000**, *before external DDR initialization*. A valid USB upload does **not** itself prove the CPU executed the payload.

**Proof goal:** when non-destructive MaskROM entry is available, upload a minimal first stage **built entirely from open source**, configured for internal SRAM and verified with a safe external observation (e.g. a board-confirmed, voltage-qualified UART). The first probe should contain no eMMC, PMIC, OTP, GPIO or DDR writes; after proof, add one capability at a time. The 32-KiB historical ROM/SPL envelope is a constraint to verify with the exact U-Boot and ROM versions.

This is separate from Rockchip's closed-source DDR-init + usbplug recovery loader. Uploading an SRAM SPL does not automatically supply the Rockusb USB/eMMC read protocol.

### ROM SD / removable-media boot

The Rockchip **rksd** disk image has an RC4-obfuscated header0 located at media sector 64, followed by an RK3288 first stage in a format selected by the upstream tooling. A valid eMMC loader normally takes priority over SD; stock Akai U-Boot also tries eMMC before the SD slot. An owner-serviceable SD boot override therefore requires a **board-specific boot-source isolation design** before it can work without vendor software.

PHYTEC implemented a dedicated eMMC boot-disable switch on a different RK3288 carrier board. That is a useful design precedent, **not an MPC One pinout**.

## Secure-boot and future manufacturer lockout

On this MPC One, live secure eFuse bank **rockchip,rk3288-secure-efuse at ffb10000** declares 128 bytes. The exposed enable word, 32-byte trusted-key-hash region and write-lock word were each entirely zero when read through Linux NVMEM. This strongly supports an unprovisioned RSA secure-boot state but does not settle the actual ROM signature policy until an independently compiled first stage executes.

Rockchip documents a **separate 1.5-V eFuse programming supply on RK3288 BGA ball P19**. We still need to determine whether the MPC One PCB can power that ball through software-controlled circuitry; no regulator visible in Linux is **not** proof that it is physically unavailable. Do not attempt any fuse programming or probe an energized unknown pad.

## Gate sequence

| Gate | Evidence required | Status |
| --- | --- | --- |
| R0 | Private complete eMMC user-area + boot0/1 backups on separate media | Passed |
| R1 | Offline decode p1 Rockchip header and inspect candidate owner SPL image | Header first bytes confirmed, full p1 report pending |
| R2 | Exact motherboard revision, eMMC and RAM part numbers, USB-B route, UART pin voltage, P19 eFuse rail, safe boot-source override | Not established |
| R3 | Independent genuine silicon MaskROM USB enumeration, not vendor Fastboot/Loader mode | Reported on another MPC One Gen1, not tested here |
| R4 | MaskROM executes a source-built **owner-controlled** SRAM-first stage | Not demonstrated |
| R5 | Own DDR bring-up, eMMC enumeration and full readback matching the verified backup | Not demonstrated |
| R6 | Owner-controlled SPL/U-Boot can boot MPFree Linux from removable SD or A/B, with stock fallback | Not demonstrated |
| R7 | Proven physical protection from irreversible vendor signing-key fuse provisioning | Not established |

**No p1/p2/p3 writes, no flash erase, no eFuse writes, no powered signal shorting, and no unsigned/unknown USB loader transmission are authorized.**

## Current workstation-only next command

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main

python3 -m unittest discover -s tools/mpc-one/tests \
  -p 'test_offline_rk3288_rom.py' -v

python3 tools/mpc-one/offline-rk3288-rom.py idblock \
  "$HOME/mpfree-backups/mpc-one-emmc-20261109T233604Z-datprr67/mmcblk0.img" \
  | tee "$HOME/mpfree-work/mpc-one-idblock-offline.txt"
~~~

This inspector reads **only 64 KiB from an ordinary backup file on the host**, validates the known SHA256 and decodes the Rockchip 512-byte header. It will not contact the MPC, access a block device, transmit a USB packet or alter files. Keep the original eMMC image and decrypted payload private.

## Safe board documentation

With power and all USB disconnected, obtain high-resolution photos of both PCB sides if board access is acceptable. Include PCB revision/markings, eMMC and RAM part numbers, RK3288 module, USB-B and SD connector areas, and identifiable unpopulated pads marked DEBUG/UART/RECOVERY. Redact serial stickers before publicly sharing. **Do not desolder anything, remove heatsinks to expose the BGA, use a continuity tester on powered hardware, short an eMMC clock/data line or inject an eFuse supply.**

Without board photographs or exact-revision schematics, an electrical switch specification would be guesswork. A serviceable override should be electrically transparent during normal operation and isolated to reset/ROM boot selection.

## Public technical references

- U-Boot Rockchip image code (RC4 header, format fields, signature and sector layout): https://android.googlesource.com/platform/external/u-boot/+/refs/heads/android10-c2f2-s2-release/tools/rkcommon.c
- U-Boot historical direct ROM USB SRAM upload and SD image documentation: https://github.com/qemu/u-boot/blob/master/doc/README.rockchip
- Upstream maintainers' caution about modern rkimage upload support: https://www.mail-archive.com/u-boot@lists.u-boot-project.org/msg02528.html
- PHYTEC RK3288 eMMC-disable switch and ROM boot-order documentation (different board): https://www.phytec.eu/fileadmin/legacy/downloads/Manuals/L-826e_1.pdf
- Rockchip secure eFuse layout and external P19 programming supply: https://github.com/mfkiwl/rk-open-docs/blob/master/NVM/Rockchip_Developer_Guide_Secure_Boot_Application_Note_EN.md
- First-person MPC One Gen1 MaskROM-after-eMMC-failure report: https://github.com/TheKikGen/MPC-LiveXplore/issues/80
