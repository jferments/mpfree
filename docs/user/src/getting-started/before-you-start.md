<!--
status: changing
covers: rust/mpfree-app/src/main.rs tools/mpc-one/offline-kexec-stick.py tools/mpc-one/hook-install.sh
checked: 2026-11-15 b9400e4
-->

# Before you start

MPFree is a new operating system for making music on the Akai MPC One. It replaces the software you see on the MPC's screen with its own: a drum machine, sampler, synths, looper and sequencer, built for playing live and making beats.

## What you need

- An **Akai MPC One** (the original model).
- **Headphones** or **speakers**. Headphones go in the PHONES socket on the front; speakers or a mixer go in the two OUTPUTS on the back.
- Optionally, something to record from, such as a synth, a turntable or a microphone preamp, plugged into the INPUTS on the back.

## How MPFree runs

MPFree starts from a **USB stick**. While it runs, the MPC's own storage, and any memory card in its SD slot, is switched off and never written to, so when you switch off and take the stick out, your MPC starts its factory software again exactly as before. (At MPFree's very first start it switches the MPC's storage on for a few minutes, read-only, to make a backup of it on the stick: see [Installing MPFree](installing.md).) Your projects, samples and settings in the factory software are untouched. What you save in MPFree is kept on the stick.

Once a small start-up helper is on the MPC, switching on with the stick in starts MPFree by itself, with nothing to tap: the MPC logo shows for a few seconds, then MPFree's startup screen, and the pads are ready about 16 seconds after you switch on. The factory software never appears first. Switch on without the stick and the factory software starts as usual.

- **To go back to the factory software,** switch off, take the stick out, and switch on again.
- **If the factory software starts even though the stick is in,** the MPC didn't accept the stick. That happens after an update of the MPC's own software, until the stick is made again for the new version. Nothing has been changed; the factory software works as normal.
- **If the MPC switches itself off while starting MPFree,** it is a safety stop: MPFree refuses to run unless the MPC's own storage is safely switched off first. Take the stick out and switch on to use the factory software.
- **Never switch the MPC off while it shows an update screen,** with or without the stick. MPFree never interrupts an update: if one is running when you switch on, the factory software starts instead.

> [!WARNING]
> MPFree is early test software. The stick is made with the MPFree USB creator from your own copy of Akai's update file: [Installing MPFree](installing.md) shows how. There are no downloads for musicians yet; that page will say where to get the creator when there is a release.

## What works today

MPFree is being built quickly and changes every week. Pages in this manual carry a banner when what they describe is still changing or not built yet:

- **Changing:** it works, but the screen or the controls may look different in a newer version.
- **Planned:** it isn't built yet. The page says what is coming.

> [!MPC]
> MPFree is not an update to the MPC software, and it can't open MPC projects (`.xpj` files) or expansions yet. It is a different instrument that runs on the same hardware, so some buttons do new jobs. The boxes like this one point out each difference; the [Coming from an MPC](../reference/coming-from-an-mpc.md) appendix lists them all in one place.
