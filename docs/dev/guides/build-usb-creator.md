# Building the USB creator

Until there is a release to download, you build the MPFree USB creator yourself, on a Linux computer, with one command. It has the MPFree of your clone inside it, so a stick it makes runs that MPFree. The creator works as the [Installing MPFree](../../user/src/getting-started/installing.md) page describes, and makes a new stick or updates an MPFree stick, keeping its settings, work and backups.

**Status (2026-11-21):** `tools/build-usb-creator.sh` builds what the [release workflow](../../../.github/workflows/release-creator.yml) builds for its Linux creator, from the same build lines. Rehearsed on 21 Nov word for word, as an ordinary user at the owner's paths, in a container standing in for the workstation (Ubuntu 24.04, 4 cores, cargo-zigbuild installed as block G1a installs it): a fresh clone built the creator in about 7 minutes; its window opened (on a virtual screen) with the programs inside it, and its own write step, run with the programs the window unpacked, updated a stick image and kept its partition 4. Not yet run on the workstation itself, or on a real stick.

## Build it

```bash
git clone https://github.com/jferments/mpfree.git ~/code/mpfree
cd ~/code/mpfree && tools/build-usb-creator.sh
```

It ends with:

```
DONE: the USB creator is ready, with MPFree 1a2b3c4d inside:
  /home/you/code/mpfree/rust/target/usb-creator/mpfree-creator
Open it with:
  rust/target/usb-creator/mpfree-creator
```

The first build takes about 10 minutes (7 on a 4-core machine, with the Rust crates and samples to download); later ones only rebuild what changed. To build a newer one later:

```bash
cd ~/code/mpfree && git pull && tools/build-usb-creator.sh
```

The creator is one file. Copy it anywhere, or open it where it is. Its first page looks for Akai's update file in your Downloads, Desktop and Documents folders. If yours is somewhere else, type its folder in step 1, or drop the file on the window.

## What you need

- **Linux** on a 64-bit PC. The creator itself also runs on Windows and macOS, but the programs for the MPC inside it are built with a Linux cross compiler, so this script runs on Linux only.
- **Rust**, from [rustup.rs](https://rustup.rs). The script adds the ARM target (`armv7-unknown-linux-gnueabihf`) itself when Rust came from rustup.
- **The ARM cross compiler.** On Debian or Ubuntu: `sudo apt install gcc-arm-linux-gnueabihf libc6-dev-armhf-cross`.
- **python3** and `readelf` (binutils), which most Linux systems have.
- **Internet, once,** for the free drum kits' samples (about 15 MB from GitHub, kept in `~/.cache/mpfree-free-kits`) and for the Rust crates.
- **Only if your system is newer than the MPC's:** cargo-zigbuild and zig. The download helper and the plugin process must not ask for a newer glibc than the MPC's 2.39, and a cross compiler from a newer Linux links against its own. The script checks, and when it is too new it stops with the line that installs both, into your home folder and without sudo (the device runbooks' block G1a). Once they are installed, the script uses them by itself.

When something is missing, the script stops with `STOP:` and says what to install.

The window needs your desktop's keyboard library for X11, `libxkbcommon-x11` (every desktop Linux has it; a bare server may not). If the creator stops at once with `Library libxkbcommon-x11.so could not be loaded`, install it (Debian or Ubuntu: `sudo apt install libxkbcommon-x11-0`).

## What it does

The script writes only under `rust/target` and the kits' cache, and needs no sudo. In order:

1. **MPFree's three programs for the MPC** (`mpfree`, `mpfree-startup`, `mpfree-screen-log`; [`stick::PROGRAM_FILES`](../../../rust/mpfree-stick/src/stick.rs)), linked statically for ARMv7 with the cross compiler, as the device rounds and releases build them.
2. **The download helper and the plugin process** (`mpfree-fetch` and `mpfree-plughost`), linked dynamically against the MPC's C library: with `cargo zigbuild` against glibc 2.34 when it is installed (on `PATH`, or in `~/.local/share/mpfree-zig` where block G1a puts zig), otherwise with the cross compiler. They build into `rust/target/helpers`, so the two builds' different flags don't rebuild each other. Each must be an ARM hard-float program that asks for glibc 2.39 at most ([plugins](../design/plugins.md#in-the-app), [the download helper](../design/mpc-plugin-store.md#the-download-helper-on-the-mpc)).
3. **The free drum kits** (`mpfree-free-kits`), from the samples `rust/mpfree-factory/free-kits.txt` pins, each checked against its SHA-256 and each library's CC0 licence. Its listing of every pad goes to `rust/target/usb-creator/free-kits.log`; only its last line is printed, or its last 20 when it fails.
4. **Collects them** in `rust/target/usb-creator/stick-programs`, with the names read from `stick.rs`, so a program added there can't be left out, and prints each one's SHA-256.
5. **The creator** ([`rust/mpfree-creator`](../../../rust/mpfree-creator)) with them inside it (`MPFREE_CREATOR_PROGRAMS`, [as a release carries them](../design/usb-creator.md#what-ships-in-the-app)), checks each is inside byte for byte, and copies it to `rust/target/usb-creator/mpfree-creator`.

It unsets `RUSTFLAGS`, `CARGO_TARGET_DIR` and `CARGO_BUILD_TARGET` for its own builds, so settings for other builds in your shell don't change what it makes or where.

## How it differs from a release

A release's creator is built on CI's runners, with a pinned Rust (`RUSTUP_TOOLCHAIN` in the workflow) and Ubuntu's cross compiler, and its Linux file must run on glibc 2.35 (Ubuntu 22.04). This one uses your Rust and your cross compiler, so it runs on your computer and on Linux systems as new as yours. It carries no version of its own beyond the MPFree commit the script prints, and no free kits' credits file.
