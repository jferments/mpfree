# Testing the USB creator build on macOS and Windows

**Status (2026-11-22):** steps for the first runs of [`cargo usb-creator`](build-usb-creator.md) on a real Mac and a real Windows PC, and of a stick from each on the MPC One. Not yet run. GitHub's macOS and Windows machines already build the creator this way and run its disk listing ([`usb-creator-build.yml`](../../../.github/workflows/usb-creator-build.yml)); what only a real computer can show is the window, the password prompt, writing a real stick, and the MPC starting MPFree from it.

The goal is three steps on every computer: get the code, build the creator, open it. Each test below follows exactly those, and the check after each step says what should happen. Note anything that differs, however small.

**What changes what:** building changes nothing outside the clone's folder. Making a stick **erases the USB stick you choose** (the creator lists only USB sticks, never the computer's own disks). Nothing here changes the MPC: switched on without the stick, it starts the factory software as before. To undo everything, delete the `mpfree` folder and reformat the stick.

## What you need

- A **Mac** with macOS 11 or newer (Apple silicon or Intel), or a **Windows 10 or 11** PC (64-bit), with an internet connection and about 10 GB free.
- A **USB stick** of 16 GB or more whose contents you don't need, or the MPFree stick you already use (updating keeps its settings, work and backups).
- Akai's **USB update file for MPC 3.10.1** (the `.img`), in Downloads, Desktop or Documents.
- The **MPC One (Gen 1)**, already set up for MPFree once (its start-up helper added), or the [first-time steps](../../user/src/getting-started/installing.md#3-let-the-mpc-start-mpfree-once) to hand.

## On a Mac

Use the **Terminal** app (Applications > Utilities).

**Once: the tools.**

```
xcode-select --install
```

Click **Install** in the box that opens, and wait for it to finish (Git and Apple's linker; skip this if `git --version` already prints a version). Then Rust:

```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Press Enter to accept the defaults, then **quit Terminal and open it again**.

- **Check M0:** `cargo --version` prints a version.

**1. Get the code.**

```
git clone https://github.com/jferments/mpfree.git
cd mpfree
```

**2. Build the creator.**

```
cargo usb-creator
```

- **Check M1:** it ends with `DONE: the USB creator is ready` and the line to open it. Note how long it took (the first build, 10 to 20 minutes). If it stops with `STOP:`, copy the last 30 lines.
- **Check M2:** the `== Collecting them for the creator` part lists six files with their SHA-256: `mpfree`, `mpfree-screen-log`, `mpfree-startup`, `mpfree-fetch`, `mpfree-plughost`, `free-kits`. Copy those six lines.

**3. Open it.**

```
rust/target/usb-creator/mpfree-creator
```

- **Check M3:** the creator's window opens (macOS shouldn't ask about an unidentified developer: you built it on this Mac). Step 1 shows Akai's update file with a green tick and **MPC 3.10.1 (MPC One, Gen 1)**.

**Make the stick** (erases the stick). Put the stick in, choose it in step 2, then **Make the stick**, tick **Yes, erase this stick**, **Erase and make the stick** (or, on your MPFree stick, **Update MPFree** with **Keep my backup, settings and files** chosen).

- **Check M4:** macOS asks for your password, saying the MPFree USB creator needs it to use the USB stick. After a few minutes the creator says **The stick is ready**. Eject the stick in the Finder.

**On the MPC.** Switch it off, put the stick in, switch it on.

- **Check M5:** MPFree's startup screen plays, then MPFree starts (a new stick's first start backs up the MPC and brings in its kits, 20 to 30 minutes, once). The pads play sound.
- **Check M6** (only with a network cable from the MPC's **Link** port to your router): open **Plugins**; MPFree fetches the plugin list. This runs the download helper, `mpfree-fetch`, one of the two programs linked against the MPC's own C library. Without a network, skip it.
- Switch off, take the stick out, switch on: the factory software starts.

**Later: build a newer one.**

```
git pull
cargo usb-creator
```

- **Check M7:** it ends with `DONE` again, much faster (only what changed is rebuilt).

## On a Windows PC

Use **PowerShell** (Start menu, type PowerShell).

**Once: the tools.**

1. Install **Git for Windows** from [git-scm.com/download/win](https://git-scm.com/download/win), with its default choices.
2. Download **rustup-init.exe** from [rustup.rs](https://rustup.rs) (the 64-bit one) and open it. If it says the Visual Studio C++ build tools are missing, choose **1** (*Quick install via the Visual Studio Community installer*) and let it finish; then press Enter to accept Rust's defaults.
3. **Close PowerShell and open it again.**

- **Check W0:** `cargo --version` and `git --version` each print a version.

**1. Get the code.**

```
git clone https://github.com/jferments/mpfree.git
cd mpfree
```

**2. Build the creator.**

```
cargo usb-creator
```

- **Check W1:** it ends with `DONE: the USB creator is ready` and the line to open it. Note how long it took. If it stops with `STOP:`, copy the last 30 lines.
- **Check W2:** copy the six SHA-256 lines under `== Collecting them for the creator`. The `free-kits` line is the same on every computer. The five programs' lines differ from one system to another, even for the same commit (GitHub's Linux, Mac and Windows machines show this too), so they are for the record, not a check.

**3. Open it.**

```
rust\target\usb-creator\mpfree-creator.exe
```

- **Check W3:** the creator's window opens (no "Windows protected your PC" box is expected for a program built on this PC). Step 1 shows Akai's update file with a green tick.

**Make the stick** (erases the stick), as on the Mac.

- **Check W4:** Windows asks "Do you want to allow this app to make changes to your device?"; click **Yes**. The creator says **The stick is ready**. Use **Safely Remove Hardware**, then take the stick out.

**On the MPC:** checks **W5** and **W6**, the same as M5 and M6. **Later:** `git pull` then `cargo usb-creator`: check **W7**, as M7.

## If something goes wrong

- **`STOP:` while building:** it says what is missing or what failed. The usual ones: Rust from a system package rather than rustup (install it from rustup.rs), no internet the first time, or `curl` missing (not on macOS or Windows 10 and 11).
- **Windows: "Smart App Control blocked an app"** while building or opening: Smart App Control blocks programs that aren't signed, and everything Rust builds on the PC is unsigned. It can't be allowed per program, only switched off (Windows Security > App & browser control). Not yet seen on a test PC; note it if it happens.
- **Windows: errors about `link.exe`:** the Visual Studio build tools are missing: open `rustup-init.exe` again and choose option 1.
- **The stick doesn't start MPFree:** take it out and switch the MPC on: the factory software starts. Run the creator again on the same stick (it erases and rewrites it), and send the creator's last message and the steps that differed.

## What to send back

For each computer: the computer and system (for example "MacBook Air M2, macOS 15.1" or "Windows 11 24H2"), the commit `cargo usb-creator` printed under `== What it builds`, each check passed or what happened instead, the build times (M1/W1, M7/W7), the six SHA-256 lines (M2/W2), and a photo or screenshot of anything unexpected.
