# Building the USB creator

Until there is a release to download, you build the MPFree USB creator yourself, with one command, on **Linux, macOS or Windows**. It has the MPFree of your clone inside it, so a stick it makes runs that MPFree. The creator works as the [Installing MPFree](../../user/src/getting-started/installing.md) page describes, and makes a new stick or updates an MPFree stick, keeping its settings, work and backups.

**Status (2026-11-22):** `cargo usb-creator` ([`rust/mpfree-creator-build`](../../../rust/mpfree-creator-build)) replaces the Linux-only script of 21 Nov, which now just runs it. It builds the same programs from the same sources as the [release workflow](../../../.github/workflows/release-creator.yml), linked against the same Ubuntu 24.04 C library, but by Rust's own linker instead of Ubuntu's cross compiler, so it needs nothing beyond Rust on any of the three systems. Built end to end from a fresh start in a Linux container; under an ARM emulator, its pad player draws every sample screen, and its startup screen every frame, byte for byte as the cross compiler's build does. [`usb-creator-build.yml`](../../../.github/workflows/usb-creator-build.yml) runs it on GitHub's Linux, macOS (Apple silicon) and Windows machines on every change to it; all three built the creator from a fresh clone on 22 Nov, in 12, 5 and 7 minutes (Linux's including the qemu comparison). The free kits came out byte for byte the same on all three; the programs differ slightly from one system to another, so on macOS and Windows they are checked from their headers, and the stick test on the MPC is what shows they run. Not yet run on a real Mac or Windows PC, and no stick it made has been started on the MPC yet: [Testing the USB creator build on macOS and Windows](test-usb-creator-macos-windows.md) has the steps.

## Three steps

**Once, install Git and Rust:**

- **Linux** (Debian or Ubuntu): `sudo apt install git curl build-essential`, then Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`.
- **macOS:** `xcode-select --install` (Apple's command line tools: Git and the linker), then Rust with the same `curl ... | sh` line.
- **Windows:** [Git for Windows](https://git-scm.com/download/win), then Rust's installer, `rustup-init.exe`, from [rustup.rs](https://rustup.rs). When it says Visual Studio's C++ build tools are missing, choose option 1, and it installs them.

Accept Rust's defaults, then open a new terminal (on Windows, PowerShell). Rust must come from rustup, which brings the standard library for the MPC and Rust's own linker; a system package of Rust has neither.

**1. Get the code:**

```
git clone https://github.com/jferments/mpfree.git
cd mpfree
```

**2. Build the creator:**

```
cargo usb-creator
```

It ends with:

```
DONE: the USB creator is ready, with MPFree 1a2b3c4d inside:
  /home/you/mpfree/rust/target/usb-creator/mpfree-creator
Open it with:
  rust/target/usb-creator/mpfree-creator
```

**3. Open it:** on Linux and macOS, `rust/target/usb-creator/mpfree-creator`; on Windows, `rust\target\usb-creator\mpfree-creator.exe`. It asks for your password (on Windows, "Do you want to allow this app to make changes?") when it writes the stick.

To build a newer one later, in the same folder:

```
git pull
cargo usb-creator
```

The first build takes 10 to 20 minutes (7 on a 4-core Linux machine), with the Rust crates, the MPC's libraries and the free kits' samples to download; later ones only rebuild what changed. Everything it makes stays in the clone (`rust/target` and `rust/mpfree-creator/target`, about 1 GB); it installs nothing and needs no administrator rights. The creator itself is one file: copy it anywhere, or open it where it is. Its first page looks for Akai's update file in your Downloads, Desktop and Documents folders; if yours is somewhere else, type its folder in step 1, or drop the file on the window.

`cargo usb-creator` must be run in the clone's top folder, where `.cargo/config.toml` defines it. On Linux, `tools/build-usb-creator.sh` does the same from anywhere. When something is missing, it stops with `STOP:` and says what to do.

On Linux, the window needs your desktop's keyboard library for X11, `libxkbcommon-x11` (every desktop Linux has it; a bare server may not). If the creator stops at once with `Library libxkbcommon-x11.so could not be loaded`, install it (Debian or Ubuntu: `sudo apt install libxkbcommon-x11-0`).

## What it does

In order, printing a `==` line for each:

1. **The tools.** Rust, with the standard library for the MPC (`armv7-unknown-linux-gnueabihf`, which it adds with rustup when it's missing) and Rust's own linker, `rust-lld`, which comes with every Rust from rustup; and `curl` (Windows 10 and 11, macOS and most Linux systems have it).
2. **The MPC's C library.** The four Ubuntu 24.04 packages the release's cross compiler installs (glibc 2.39 and GCC's support library for 32-bit ARM, about 3.5 MB), each checked against the SHA-256 pinned in [`libs.rs`](../../../rust/mpfree-creator-build/src/libs.rs), the one Ubuntu's signed package index lists. They come from Ubuntu's archive, or Launchpad, which keeps every version after the archive drops it. Only the 24 files the links use are taken out, into `rust/target/usb-creator/mpc-libs`, with symbolic links made into copies (so it works on Windows) and the C library's linker script pointed at its neighbours. Done once; downloads stay in `rust/target/usb-creator/downloads`.
3. **MPFree's three programs for the MPC** (`mpfree`, `mpfree-startup`, `mpfree-screen-log`; [`stick::PROGRAM_FILES`](../../../rust/mpfree-stick/src/stick.rs)), compiled for ARMv7 hard-float and linked statically, as the device rounds and releases build them. Rust's linker gets what `arm-linux-gnueabihf-gcc` would give it: the C runtime's start files (`crt1.o`, `crti.o`, `crtbeginT.o` before, `crtend.o`, `crtn.o` after) and the static libraries.
4. **The download helper and the plugin process** (`mpfree-fetch` and `mpfree-plughost`), linked dynamically against the MPC's C library, with `/lib/ld-linux-armhf.so.3` as their interpreter and the position-independent start files. Against Ubuntu 24.04's glibc 2.39 they ask for 2.34 at most, so neither cargo-zigbuild nor zig is needed any more. They build in their own target folder, so the two builds' different flags don't rebuild each other.
5. **The free drum kits** (`mpfree-free-kits`), from the samples `rust/mpfree-factory/free-kits.txt` pins, each checked against its SHA-256 and each library's CC0 licence. Its listing of every pad goes to `rust/target/usb-creator/free-kits.log`; only its last line is printed, or its last 20 when it fails.
6. **Collects them** in `rust/target/usb-creator/stick-programs`, with the names read from `stick.rs`, so a program added there can't be left out. Each program is checked from its ELF headers: 32-bit ARM for the hard-float EABI 5; the three static (no interpreter); the two helpers asking for `/lib/ld-linux-armhf.so.3` and glibc 2.39 at most. It prints each file's SHA-256.
7. **The creator** ([`rust/mpfree-creator`](../../../rust/mpfree-creator)) with them inside it (`MPFREE_CREATOR_PROGRAMS`, [as a release carries them](../design/usb-creator.md#what-ships-in-the-app)), checks each is inside byte for byte, and copies it to `rust/target/usb-creator/mpfree-creator` (`.exe` on Windows). On Windows the C runtime is linked in (`+crt-static`), as in a release, so the file runs on another PC without the Visual C++ package.

Settings in your shell for other builds (`RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_TARGET_DIR`, `CARGO_BUILD_TARGET`, an ARM linker) reach none of its builds: each sets its own, through the environment, which also wins over a linker or flags in a cargo config.

## Why no cross compiler is needed

MPFree's programs for the MPC are plain Rust (their only outside crate with C in it is `libc`, bindings only). Rust compiles them for 32-bit ARM on any computer; only linking needs anything from Linux, and only files: the C library's static archive and start files, and for the helpers its shared library to link against. Rust's own linker (`rust-lld`, LLVM's lld) reads those on Linux, macOS and Windows alike. Taking the files from the very Ubuntu packages the release's cross compiler uses keeps the C library inside the programs the same; what differs is the linker, which lays the program out a little differently (lld's own segment layout and alignment). Under qemu both builds draw the same screens, byte for byte, and CI checks that on every change (`usb-creator-build.yml`'s Linux job).

Other ways were weighed: a container (Docker Desktop on macOS and Windows: a large install, and on Windows it needs WSL); a ready-made cross compiler for each system (a different download for each, each bringing its own C library, so the three would build different programs); zig (it links against glibc only dynamically, and MPFree's programs are static); and downloading programs CI built (simplest, but then nobody builds what they run, and it works only for commits CI has built).

## Windows and line endings

Git for Windows converts line endings to CRLF on checkout by default. The repository's [`.gitattributes`](../../../.gitattributes) (`* -text`) turns that off for every file, so a clone on Windows has the same bytes as one on Linux or macOS. Without it, text files MPFree embeds in its programs (themes, setup text, the free kits' list) would carry CRLF into the MPC. A clone made before `.gitattributes` arrived keeps its converted files until they are checked out again: `git rm -r --cached . && git reset --hard` in the clone (it discards changes you made there).

## How it differs from a release

A release's creator is built on CI's runners with a pinned Rust (`RUSTUP_TOOLCHAIN` in the workflow), links MPFree's programs with Ubuntu's cross compiler (the same C library, a different linker), and its Linux file must run on glibc 2.35 (Ubuntu 22.04). This one uses your Rust, so its Linux file runs on your computer and on Linux systems as new as yours, and on macOS it is a plain program rather than an app bundle. It carries no version of its own beyond the MPFree commit it prints, and no free kits' credits file. Moving the release's `programs` job to `cargo usb-creator`'s way of linking, so every creator carries programs linked the same way, is a separate choice for the owner.
