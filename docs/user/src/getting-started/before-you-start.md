<!--
status: changing
covers: rust/mpfree-app/src/main.rs tools/mpc-one/offline-kexec-stick.py
checked: 2026-11-13 f8cc558
-->

# Before you start

MPFree is a new operating system for making music on the Akai MPC One. It replaces the software you see on the MPC's screen with its own: a drum machine, sampler, synths, looper and sequencer, built for playing live and making beats.

## What you need

- An **Akai MPC One** (the original model).
- **Headphones** or **speakers**. Headphones go in the PHONES socket on the front; speakers or a mixer go in the two OUTPUTS on the back.
- Optionally, something to record from, such as a synth, a turntable or a microphone preamp, plugged into the INPUTS on the back.

## How MPFree runs

MPFree starts from a **USB stick**. While it runs, the MPC's own storage is hidden from it and never written to, so when you switch off and take the stick out, your MPC starts its factory software again exactly as before. Your projects, samples and settings in the factory software are untouched.

> [!WARNING]
> MPFree is early test software, and there is no installer or download for musicians yet. Today the test stick is made by MPFree's developers. When there is a way for anyone to try it, this page will say how.

## What works today

MPFree is being built quickly and changes every week. Pages in this manual carry a banner when what they describe is still changing or not built yet:

- **Changing:** it works, but the screen or the controls may look different in a newer version.
- **Planned:** it isn't built yet. The page says what is coming.

> [!MPC]
> MPFree is not an update to the MPC software, and it can't open MPC projects (`.xpj` files) or expansions yet. It is a different instrument that runs on the same hardware, so some buttons do new jobs. The boxes like this one point out each difference; the [Coming from an MPC](../reference/coming-from-an-mpc.md) appendix lists them all in one place.
