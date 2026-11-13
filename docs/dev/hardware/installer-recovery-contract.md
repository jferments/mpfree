# MPFree Installer and Recovery Contract

**Target:** Original MPC One Gen 1. Read the [full architecture](mpc-one-boot-architecture.md) before implementing any boot operation.

## Proven foundation

The owner holds verified read-only copies of the eMMC user area and both 4-MiB hardware boot areas. A controlled `kexec` environment booted firmware 3.10.0 from USB **three times** with internal MMC controllers disabled. Checks of boot-critical partition hashes and write counters found no changes, and a power cycle resumed factory 3.6.0. U-Boot pre-Linux Fastboot, MMC commands, scripts and `bootz` exist, but alternative-image direct boot and independent ROM recovery are **not yet physically validated**.

## Permitted stages

| Gate | Allowed | Required additional evidence |
|---|---|---|
| G0 — inspect | Read-only board, kernel, GPT, backup and USB surveys | Existing device identity |
| G1 — reversible Linux | Isolated USB-root kexec, RAM data, internal MMC disabled, measured fallback | Already tested with 3.10.0; repeat for actual MPFree image |
| G2 — persistent app | Explicitly approved isolated files on SD/USB or scoped `/data/mpfree` | Backup and service rollback rehearsal |
| G3 — direct boot | **Nonpersistent** U-Boot RAM/removable boot only after hardware qualification | Proven image acceptance and factory fallback |
| G4 — installed boot changes | **Currently prohibited** | ROM or equivalent independent recovery, full read-back and restore rehearsal |

Never infer G4 permission merely because SSH is root, Fastboot is present, p6 is backed up, or simulated boot code executes correctly.

## Agent state machine

```text
IDENTIFY_BOARD → VERIFY_FIRMWARE_AND_GPT → VERIFY_BACKUP_AND_POWER
→ VERIFY_RELEASE_AND_HASHES → STAGE_REMOVABLE_OR_VOLATILE_IMAGE
→ OWNER_APPROVAL → ACTIVATE_ONLY_QUALIFIED_PATH
→ HEALTHCHECK → COMMIT_OR_PRESERVE_STOCK_FALLBACK
```

Refuse unknown boards, changed GPT ranges, invalid images, unsupported boot formats and unexpected destination media. Do not auto-format storage. Never write p1/p2/p3, GPT, eMMC boot0/boot1, RPMB, OTP or factory p6 until independent recovery is proven. A power-loss-interrupted G1/G2 test must leave factory boot intact.

## Recovery classification

- Temporary runtime: **power-cycle return to factory boot demonstrated**.
- Application handoff: orderly stop/restart of `acvs.service` plus watchdog still requires qualifying each scenario.
- Factory p6 failure: U-Boot Fastboot path is present, but full restore has **not** been rehearsed.
- Early loader/p3 failure: silicon-ROM first-stage access and full storage restoration remain **unproven**.

Next gates: map actual U-Boot MMC numbering and image-selection order; test RAM-only direct boot; validate actual Update Mode recovery; demonstrate genuine ROM entry, independently built SRAM execution and verified eMMC read-back; only then design persistent factory/MPFree selection with rollback.

Related: [3.10.1 compatibility](firmware-3.10.1-compatibility.md), [USB installer design](../design/usb-installer.md), [existing recovery strategy](../research/recovery-and-os-strategy.md).
